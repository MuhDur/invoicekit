<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-ffi

A C ABI over the InvoiceKit engine byte contract. All structured data crosses the boundary as canonical JSON byte streams; nothing of the invoice model is exposed as C structs.

## What it does

This crate is a thin `extern "C"` shell around `invoicekit_engine::process_abi_json`. A foreign caller hands in request bytes (UTF-8 JSON shaped per the Engine ABI, version 1), gets back an opaque handle that owns the response bytes, reads the status code, copies the response bytes out, and frees the handle. There is no other surface: no parsing helpers, no struct layout to keep in sync across languages, no shared state. The library is built as `rlib`, `cdylib`, and `staticlib`, so it can be linked from Go (cgo), .NET (P/Invoke), Java (Foreign Function and Memory), or any other runtime that speaks the C ABI.

The one behavior this crate adds on top of the engine is panic containment. A panic must never unwind across an `extern "C"` frame into a foreign caller — that is undefined behavior. Every engine call runs inside `std::panic::catch_unwind`; a caught panic is converted into a well-formed JSON error handle (`error.code` = `internal_panic`) that the foreign caller can read and free normally, instead of crashing the process.

This crate does not implement any invoice logic, validation, canonicalization, or cryptography. It forwards bytes to `invoicekit-engine` and owns the lifecycle of the response buffer. The ABI version it reports is whatever the engine defines (`ENGINE_ABI_VERSION`, currently 1).

## Capabilities

- `invoicekit_engine_abi_version() -> u32` — the Engine ABI version this library implements. Returns the engine's `ENGINE_ABI_VERSION` (currently `1`).
- `invoicekit_engine_process_json(request_ptr, request_len) -> *mut InvoiceKitEngineResult` — process one Engine ABI JSON request and return an owned result handle. A null `request_ptr` is valid only when `request_len` is `0` (empty request); a null pointer with a non-zero length returns an `invalid_input_pointer` error handle. The returned pointer is never null and must be freed exactly once. `unsafe`: when `request_len > 0`, `request_ptr` must point to `request_len` initialized bytes valid for the call.
- `invoicekit_engine_result_status(result) -> u32` — the status code carried by a handle: `0` Ok, `1` Error, `2` InvalidHandle (returned when `result` is null). Ok versus Error is derived by scanning the response bytes for `"status":"ok"`.
- `invoicekit_engine_result_bytes(result) -> *const c_uchar` — pointer to the response bytes, valid until the handle is freed. Null when `result` is null or the response is empty.
- `invoicekit_engine_result_len(result) -> usize` — response byte length; `0` when `result` is null.
- `invoicekit_engine_result_free(result)` — release a handle. Null is a no-op. Must be called exactly once per non-null handle.
- `InvoiceKitStatusCode` — `#[repr(u32)]` enum: `Ok = 0`, `Error = 1`, `InvalidHandle = 2`.
- `InvoiceKitEngineResult` — opaque handle owning response bytes; not field-readable from C.
- `crate_name() -> &'static str` — returns `"invoicekit-ffi"`, used by release tooling and bead-correlation reports.

## Calling contract

The ownership rule is fixed: `invoicekit_engine_process_json` returns a handle, the caller reads status and copies bytes, then calls `invoicekit_engine_result_free` exactly once. The bytes pointer is borrowed from the handle and is invalid after the free. The accessors (`status`, `bytes`, `len`, `free`) all tolerate a null handle so a foreign caller cannot fault by passing null.

Errors are in-band: a malformed request, an unsupported ABI version, or an engine-side failure comes back as a result handle with status `Error` and a canonical JSON error body. The functions do not signal failure through a sentinel return value other than the status code and the null-pointer rules above.

## Mode / Residuals

- This is a transport shell, not a logic crate. It performs no invoice processing of its own; correctness of the response is entirely the engine's responsibility.
- Status classification is a byte-substring scan for `"status":"ok"` in the response, not a JSON parse. It assumes the engine emits canonical responses where that token appears only as the top-level status. It is a fast path, not a structural check.
- `unsafe_code` is allowed in this crate (it is an FFI boundary). The pointer-validity preconditions are documented per-function as `# Safety` and are the caller's responsibility.
- No cryptography, no I/O, no global state. Each call is independent.

## Where it sits

```
foreign runtime (Go / .NET / Java / C) -> invoicekit-ffi -> invoicekit-engine
```

`invoicekit-ffi` is the only language-binding edge for the engine byte contract. Its sole dependency is `invoicekit-engine`. Everything structured stays behind the JSON byte stream, so adding or changing an engine operation does not change this crate's ABI surface — only the JSON shapes the engine accepts and returns.

## Usage

From Rust (the same calls a C caller makes through the generated header):

```rust
let request = br#"{"abi_version":1,"operation":"unknown","payload":{}}"#;
let result = unsafe {
    invoicekit_ffi::invoicekit_engine_process_json(request.as_ptr(), request.len())
};
assert!(!result.is_null());

let status = unsafe { invoicekit_ffi::invoicekit_engine_result_status(result) };
let len = unsafe { invoicekit_ffi::invoicekit_engine_result_len(result) };
let ptr = unsafe { invoicekit_ffi::invoicekit_engine_result_bytes(result) };
// copy `len` bytes from `ptr` before freeing.

unsafe { invoicekit_ffi::invoicekit_engine_result_free(result) };
```

## Testing

Unit tests drive the public C ABI: they replay an engine-ABI golden fixture (`conformance-corpus/golden/engine-abi-v1-commercial-document.json`) through `process_json` and assert the response bytes match exactly, check the null-pointer accessor paths, confirm a null pointer with non-zero length yields an error handle, and inject a forced panic to prove `catch_engine_panic` converts it into a freeable `internal_panic` error handle rather than unwinding across the boundary.

## References

- Engine ABI version 1 — defined by `invoicekit-engine` (`ENGINE_ABI_VERSION`); this crate forwards to its `process_abi_json` entry point.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
