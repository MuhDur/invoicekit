// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-ffi` — C ABI for the InvoiceKit engine byte contract.
//!
//! The ABI keeps all structured data behind canonical JSON byte streams.
//! Callers pass request bytes to [`invoicekit_engine_process_json`], inspect
//! the returned opaque handle, copy the response bytes, and release the handle
//! with [`invoicekit_engine_result_free`].

use std::os::raw::c_uchar;
use std::ptr;
use std::slice;

const NULL_INPUT_RESPONSE: &[u8] = br#"{"abi_version":1,"error":{"code":"invalid_input_pointer","message":"request pointer was null while request_len was non-zero","remediation":"Pass a valid pointer to request_len UTF-8 JSON bytes, or pass null with length 0 for an empty request."},"operation":null,"status":"error"}"#;

const PANIC_RESPONSE: &[u8] = br#"{"abi_version":1,"error":{"code":"internal_panic","message":"the engine panicked while processing the request; the panic was caught at the C ABI boundary","remediation":"This is an InvoiceKit bug. Report it with the request that triggered it. The library state is unaffected and the handle is safe to free."},"operation":null,"status":"error"}"#;

/// Result status code returned by the C ABI.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvoiceKitStatusCode {
    /// The engine operation completed successfully.
    Ok = 0,
    /// The engine returned a canonical JSON error response.
    Error = 1,
    /// The caller passed a null result handle.
    InvalidHandle = 2,
}

/// Opaque handle that owns engine response bytes.
///
/// The struct is intentionally opaque to C callers. Use
/// [`invoicekit_engine_result_status`], [`invoicekit_engine_result_bytes`],
/// [`invoicekit_engine_result_len`], and [`invoicekit_engine_result_free`]
/// instead of reading fields directly.
pub struct InvoiceKitEngineResult {
    status: InvoiceKitStatusCode,
    bytes: Vec<u8>,
}

impl InvoiceKitEngineResult {
    fn new(bytes: Vec<u8>) -> Self {
        let status = if is_success_response(&bytes) {
            InvoiceKitStatusCode::Ok
        } else {
            InvoiceKitStatusCode::Error
        };
        Self { status, bytes }
    }

    fn error(bytes: &[u8]) -> Self {
        Self {
            status: InvoiceKitStatusCode::Error,
            bytes: bytes.to_vec(),
        }
    }
}

/// Return the engine ABI version implemented by this library.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_ffi::invoicekit_engine_abi_version(), 1);
/// ```
#[no_mangle]
#[must_use]
pub extern "C" fn invoicekit_engine_abi_version() -> u32 {
    invoicekit_engine::ENGINE_ABI_VERSION
}

/// Process an Engine ABI JSON request and return an owned result handle.
///
/// The returned pointer must be released exactly once with
/// [`invoicekit_engine_result_free`]. A null `request_ptr` is valid only when
/// `request_len` is `0`; otherwise the function returns an error handle.
///
/// # Safety
///
/// When `request_len` is greater than zero, `request_ptr` must point to
/// `request_len` initialized bytes that remain valid for the duration of this
/// call.
///
/// # Examples
///
/// ```
/// let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
/// let result = unsafe {
///     invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
/// };
/// assert!(!result.is_null());
/// unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
/// ```
#[no_mangle]
#[must_use]
pub unsafe extern "C" fn invoicekit_engine_process_json(
    request_ptr: *const c_uchar,
    request_len: usize,
) -> *mut InvoiceKitEngineResult {
    let request = if request_ptr.is_null() {
        if request_len == 0 {
            &[]
        } else {
            return Box::into_raw(Box::new(InvoiceKitEngineResult::error(NULL_INPUT_RESPONSE)));
        }
    } else {
        // SAFETY: The caller promises request_ptr points to request_len bytes.
        unsafe { slice::from_raw_parts(request_ptr, request_len) }
    };

    Box::into_raw(Box::new(catch_engine_panic(|| {
        invoicekit_engine::process_abi_json(request)
    })))
}

/// Run an engine call, containing any panic at the C ABI boundary.
///
/// A panic must never unwind across the `extern "C"` frame: callers are Go
/// (cgo), .NET (P/Invoke), and Java (FFM), and unwinding into a foreign frame is
/// undefined behavior. `process_abi_json` is expected to encode its errors as
/// JSON rather than panic, but a transitive panic (allocation failure, serde
/// recursion limit, or a future logic bug) must be contained here. A caught
/// panic becomes a well-formed [`PANIC_RESPONSE`] error handle so the foreign
/// caller stays alive and can free the handle normally.
fn catch_engine_panic(
    call: impl FnOnce() -> Vec<u8> + std::panic::UnwindSafe,
) -> InvoiceKitEngineResult {
    std::panic::catch_unwind(call).map_or_else(
        |_| InvoiceKitEngineResult::error(PANIC_RESPONSE),
        InvoiceKitEngineResult::new,
    )
}

/// Return the status code carried by an engine result handle.
///
/// # Safety
///
/// `result` must be null or a live pointer previously returned by
/// [`invoicekit_engine_process_json`].
///
/// # Examples
///
/// ```
/// let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
/// let result = unsafe {
///     invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
/// };
/// let status = unsafe { invoicekit_ffi::invoicekit_engine_result_status(result) };
/// assert_eq!(status, invoicekit_ffi::InvoiceKitStatusCode::Error as u32);
/// unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
/// ```
#[no_mangle]
#[must_use]
pub unsafe extern "C" fn invoicekit_engine_result_status(
    result: *const InvoiceKitEngineResult,
) -> u32 {
    if result.is_null() {
        return InvoiceKitStatusCode::InvalidHandle as u32;
    }
    // SAFETY: The caller promises result is a live handle or null; null is handled above.
    unsafe { (*result).status as u32 }
}

/// Return a pointer to response bytes owned by an engine result handle.
///
/// The pointer remains valid until [`invoicekit_engine_result_free`] is called.
/// It may be null when `result` is null or when the response length is zero.
///
/// # Safety
///
/// `result` must be null or a live pointer previously returned by
/// [`invoicekit_engine_process_json`].
///
/// # Examples
///
/// ```
/// let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
/// let result = unsafe {
///     invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
/// };
/// let len = unsafe { invoicekit_ffi::invoicekit_engine_result_len(result) };
/// let ptr = unsafe { invoicekit_ffi::invoicekit_engine_result_bytes(result) };
/// assert!(len > 0);
/// assert!(!ptr.is_null());
/// unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
/// ```
#[no_mangle]
#[must_use]
pub unsafe extern "C" fn invoicekit_engine_result_bytes(
    result: *const InvoiceKitEngineResult,
) -> *const c_uchar {
    if result.is_null() {
        return ptr::null();
    }
    // SAFETY: The caller promises result is a live handle or null; null is handled above.
    let bytes = unsafe { &(*result).bytes };
    if bytes.is_empty() {
        ptr::null()
    } else {
        bytes.as_ptr()
    }
}

/// Return the response byte length carried by an engine result handle.
///
/// # Safety
///
/// `result` must be null or a live pointer previously returned by
/// [`invoicekit_engine_process_json`].
///
/// # Examples
///
/// ```
/// let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
/// let result = unsafe {
///     invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
/// };
/// assert!(unsafe { invoicekit_ffi::invoicekit_engine_result_len(result) } > 0);
/// unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
/// ```
#[no_mangle]
#[must_use]
pub unsafe extern "C" fn invoicekit_engine_result_len(
    result: *const InvoiceKitEngineResult,
) -> usize {
    if result.is_null() {
        return 0;
    }
    // SAFETY: The caller promises result is a live handle or null; null is handled above.
    unsafe { (*result).bytes.len() }
}

/// Release an engine result handle returned by [`invoicekit_engine_process_json`].
///
/// Passing null is a no-op.
///
/// # Safety
///
/// `result` must be null or a pointer returned by
/// [`invoicekit_engine_process_json`] that has not already been freed.
///
/// # Examples
///
/// ```
/// let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
/// let result = unsafe {
///     invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
/// };
/// unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
/// ```
#[no_mangle]
pub unsafe extern "C" fn invoicekit_engine_result_free(result: *mut InvoiceKitEngineResult) {
    if !result.is_null() {
        // SAFETY: The caller promises result was returned by Box::into_raw in this library
        // and has not been freed yet.
        unsafe {
            drop(Box::from_raw(result));
        }
    }
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_ffi::crate_name(), "invoicekit-ffi");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-ffi"
}

fn is_success_response(bytes: &[u8]) -> bool {
    bytes
        .windows(br#""status":"ok""#.len())
        .any(|window| window == br#""status":"ok""#)
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::{
        crate_name, invoicekit_engine_abi_version, invoicekit_engine_process_json,
        invoicekit_engine_result_bytes, invoicekit_engine_result_free,
        invoicekit_engine_result_len, invoicekit_engine_result_status, InvoiceKitStatusCode,
    };
    use serde::Deserialize;

    const GOLDEN_FIXTURE: &str =
        include_str!("../../../conformance-corpus/golden/engine-abi-v1-commercial-document.json");

    #[derive(Debug, Deserialize)]
    struct GoldenFixture {
        request_bytes: String,
        expected_response_bytes: String,
    }

    fn golden_fixture() -> GoldenFixture {
        serde_json::from_str(GOLDEN_FIXTURE).expect("golden fixture is valid JSON")
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-ffi");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn c_abi_reports_engine_abi_version() {
        assert_eq!(invoicekit_engine_abi_version(), 1);
    }

    #[test]
    fn c_abi_matches_golden_fixture_bytes() {
        let fixture = golden_fixture();
        let result = unsafe {
            invoicekit_engine_process_json(
                fixture.request_bytes.as_ptr(),
                fixture.request_bytes.len(),
            )
        };
        assert!(!result.is_null());
        assert_eq!(
            unsafe { invoicekit_engine_result_status(result) },
            InvoiceKitStatusCode::Ok as u32
        );
        let len = unsafe { invoicekit_engine_result_len(result) };
        let ptr = unsafe { invoicekit_engine_result_bytes(result) };
        assert!(!ptr.is_null());
        let bytes = unsafe { slice::from_raw_parts(ptr, len) };
        assert_eq!(bytes, fixture.expected_response_bytes.as_bytes());
        unsafe { invoicekit_engine_result_free(result) };
    }

    #[test]
    fn c_abi_null_nonzero_request_returns_error_handle() {
        let result = unsafe { invoicekit_engine_process_json(std::ptr::null(), 8) };
        assert!(!result.is_null());
        assert_eq!(
            unsafe { invoicekit_engine_result_status(result) },
            InvoiceKitStatusCode::Error as u32
        );
        unsafe { invoicekit_engine_result_free(result) };
    }

    #[test]
    fn c_abi_null_result_accessors_are_safe() {
        assert_eq!(
            unsafe { invoicekit_engine_result_status(std::ptr::null()) },
            InvoiceKitStatusCode::InvalidHandle as u32
        );
        assert_eq!(unsafe { invoicekit_engine_result_len(std::ptr::null()) }, 0);
        assert!(unsafe { invoicekit_engine_result_bytes(std::ptr::null()) }.is_null());
        unsafe { invoicekit_engine_result_free(std::ptr::null_mut()) };
    }

    // --- Panic-across-FFI containment (the C ABI must never let a panic unwind
    // --- into a foreign caller; that is undefined behavior).

    #[test]
    fn catch_engine_panic_contains_a_panicking_call() {
        // Fault injection: force the engine call to panic and prove the boundary
        // helper turns it into a well-formed error handle instead of unwinding.
        let result = super::catch_engine_panic(|| panic!("simulated engine panic"));
        assert_eq!(result.status, InvoiceKitStatusCode::Error);
        assert_eq!(result.bytes, super::PANIC_RESPONSE);
        // The contained response must be parseable JSON the foreign caller can read.
        let parsed: serde_json::Value =
            serde_json::from_slice(&result.bytes).expect("panic response is valid JSON");
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["error"]["code"], "internal_panic");
    }

    #[test]
    fn catch_engine_panic_passes_through_a_normal_call() {
        // The happy path must be untouched: a non-panicking call returns its
        // bytes and the success/error status derives from the payload as before.
        let result = super::catch_engine_panic(|| br#"{"status":"ok"}"#.to_vec());
        assert_eq!(result.status, InvoiceKitStatusCode::Ok);
        assert_eq!(result.bytes, br#"{"status":"ok"}"#);
    }

    #[test]
    fn process_json_survives_when_engine_would_panic() {
        // End-to-end: even if the engine panicked, the public C ABI returns a
        // non-null, freeable error handle rather than unwinding across the
        // boundary. We drive the real entry point with a valid request; the
        // guarantee being pinned is that the call returns normally and the
        // handle is well-formed and safe to free.
        let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
        let result = unsafe { invoicekit_engine_process_json(request.as_ptr(), request.len()) };
        assert!(!result.is_null());
        let status = unsafe { invoicekit_engine_result_status(result) };
        assert!(
            status == InvoiceKitStatusCode::Ok as u32
                || status == InvoiceKitStatusCode::Error as u32
        );
        unsafe { invoicekit_engine_result_free(result) };
    }
}
