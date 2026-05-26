// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-binding-wasm-browser` — browser wrapper over the engine ABI.
//!
//! The browser bundle bead will add the JavaScript package and wasm-bindgen
//! surface. This crate already runs the shared engine ABI golden fixture so the
//! browser binding track is pinned to the byte contract.

/// Process an Engine ABI JSON request through the browser binding wrapper.
///
/// # Examples
///
/// ```
/// let response = invoicekit_binding_wasm_browser::process_engine_abi_json(
///     br#"{"abi_version":1,"operation":"unknown","payload":{}}"#,
/// );
/// assert!(std::str::from_utf8(&response).unwrap().contains(r#""status":"error""#));
/// ```
#[must_use]
pub fn process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8> {
    invoicekit_engine::process_abi_json(request_bytes)
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
/// assert_eq!(invoicekit_binding_wasm_browser::crate_name(), "invoicekit-binding-wasm-browser");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-binding-wasm-browser"
}

#[cfg(test)]
mod tests {
    use super::{crate_name, process_engine_abi_json};
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
        assert_eq!(crate_name(), "invoicekit-binding-wasm-browser");
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
    fn browser_wrapper_matches_engine_abi_golden_fixture() {
        let fixture = golden_fixture();
        assert_eq!(
            process_engine_abi_json(fixture.request_bytes.as_bytes()),
            fixture.expected_response_bytes.as_bytes()
        );
    }
}
