// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-engine` — deterministic InvoiceKit engine entry points.
//!
//! This crate defines the byte-level ABI contract that the separate
//! native-binding (`invoicekit-ffi`), WebAssembly (`invoicekit-wasm`), and
//! service-shim (`services/*`) crates consume. The v1 contract accepts
//! canonicalizable JSON request bytes and returns canonical JSON response
//! bytes.

use std::str;

use invoicekit_canonical::{canonicalize, canonicalize_value, CanonicalizeError};
use invoicekit_ir::{CommercialDocument, IrError};
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::debug;

/// Stable engine ABI version implemented by this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_engine::ENGINE_ABI_VERSION, 1);
/// ```
pub const ENGINE_ABI_VERSION: u32 = 1;

/// ABI operation that validates and canonicalizes a commercial document.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_engine::COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION,
///     "commercial_document.canonicalize"
/// );
/// ```
pub const COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION: &str = "commercial_document.canonicalize";

const BEAD_ID: &str = "invoices-t-023-stable-engine-abi-ux1";
const INTERNAL_SERIALIZATION_ERROR_RESPONSE: &[u8] = br#"{"abi_version":1,"error":{"code":"internal_response_serialization","message":"engine could not serialize an ABI response","remediation":"Retry with the same request and report this deterministic engine defect if it repeats."},"operation":null,"status":"error"}"#;

/// Process an Engine ABI v1 JSON request and return canonical JSON response bytes.
///
/// The function never panics on user input. Invalid UTF-8, invalid JSON,
/// unsupported ABI versions, unsupported operations, and invalid IR payloads
/// are returned as canonical error responses.
///
/// # Examples
///
/// ```
/// let response = invoicekit_engine::process_abi_json(
///     br#"{"abi_version":1,"operation":"unknown","payload":{}}"#,
/// );
/// let text = std::str::from_utf8(&response).unwrap();
/// assert!(text.contains(r#""status":"error""#));
/// assert!(text.contains(r#""code":"unsupported_operation""#));
/// ```
#[must_use]
pub fn process_abi_json(request_bytes: &[u8]) -> Vec<u8> {
    let request_text = match str::from_utf8(request_bytes) {
        Ok(text) => text,
        Err(error) => {
            return canonical_response(&error_response(None, &EngineAbiError::InvalidUtf8(error)));
        }
    };

    let canonical_request = match canonicalize(request_text) {
        Ok(request) => request,
        Err(error) => {
            return canonical_response(&error_response(
                None,
                &EngineAbiError::InvalidRequestJson(error),
            ));
        }
    };

    let request = match serde_json::from_str::<EngineRequest>(&canonical_request) {
        Ok(request) => request,
        Err(error) => {
            return canonical_response(&error_response(
                None,
                &EngineAbiError::InvalidRequestEnvelope(error),
            ));
        }
    };

    let operation = request.operation.clone();
    match process_request(request) {
        Ok(value) => canonical_response(&value),
        Err(error) => canonical_response(&error_response(Some(&operation), &error)),
    }
}

/// Errors that can occur while processing an engine ABI request.
///
/// Public callers normally receive these errors as JSON response objects from
/// [`process_abi_json`]. The Rust enum is public so wrappers can make precise
/// assertions in conformance tests.
#[derive(Debug, Error)]
pub enum EngineAbiError {
    /// Request bytes were not valid UTF-8.
    #[error("request bytes were not valid UTF-8: {0}")]
    InvalidUtf8(#[from] str::Utf8Error),
    /// Request JSON could not be canonicalized.
    #[error("request JSON is not valid canonicalizable JSON: {0}")]
    InvalidRequestJson(#[source] CanonicalizeError),
    /// Request JSON did not match the ABI envelope.
    #[error("request JSON did not match the ABI envelope: {0}")]
    InvalidRequestEnvelope(#[source] serde_json::Error),
    /// The request ABI version is not implemented.
    #[error("unsupported engine ABI version `{0}`")]
    UnsupportedAbiVersion(u32),
    /// The request operation is not implemented.
    #[error("unsupported engine ABI operation `{0}`")]
    UnsupportedOperation(String),
    /// The operation payload was not a valid InvoiceKit IR document.
    #[error("invalid commercial document payload: {0}")]
    InvalidCommercialDocument(#[source] IrError),
    /// The validated document could not be serialized to JSON.
    #[error("validated commercial document could not be serialized: {0}")]
    SerializeCommercialDocument(#[source] IrError),
    /// A response JSON value could not be canonicalized.
    #[error("engine response could not be canonicalized: {0}")]
    CanonicalizeResponse(#[source] CanonicalizeError),
}

impl EngineAbiError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8(_) => "invalid_utf8",
            Self::InvalidRequestJson(_) => "invalid_request_json",
            Self::InvalidRequestEnvelope(_) => "invalid_request_envelope",
            Self::UnsupportedAbiVersion(_) => "unsupported_abi_version",
            Self::UnsupportedOperation(_) => "unsupported_operation",
            Self::InvalidCommercialDocument(_) => "invalid_commercial_document",
            Self::SerializeCommercialDocument(_) => "serialize_commercial_document",
            Self::CanonicalizeResponse(_) => "canonicalize_response",
        }
    }

    fn remediation(&self) -> &'static str {
        match self {
            Self::InvalidUtf8(_) => "Send UTF-8 encoded JSON bytes.",
            Self::InvalidRequestJson(_) => {
                "Send RFC 8259 JSON without duplicate object members or unsafe I-JSON numbers."
            }
            Self::InvalidRequestEnvelope(_) => {
                "Send an object with abi_version, operation, and payload fields."
            }
            Self::UnsupportedAbiVersion(_) => {
                "Use ABI version 1 or upgrade the linked InvoiceKit engine."
            }
            Self::UnsupportedOperation(_) => {
                "Use commercial_document.canonicalize for the Engine ABI v1 contract."
            }
            Self::InvalidCommercialDocument(_) => {
                "Fix the commercial document fields reported by the validation error."
            }
            Self::SerializeCommercialDocument(_) | Self::CanonicalizeResponse(_) => {
                "Retry with the same request and report this deterministic engine defect if it repeats."
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineRequest {
    abi_version: u32,
    operation: String,
    payload: Value,
}

fn process_request(request: EngineRequest) -> Result<Value, EngineAbiError> {
    if request.abi_version != ENGINE_ABI_VERSION {
        return Err(EngineAbiError::UnsupportedAbiVersion(request.abi_version));
    }
    if request.operation != COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION {
        return Err(EngineAbiError::UnsupportedOperation(request.operation));
    }

    let document = CommercialDocument::try_from_value(request.payload)
        .map_err(EngineAbiError::InvalidCommercialDocument)?;
    debug!(
        bead_id = BEAD_ID,
        operation = COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION,
        tenant_id = %document.meta.tenant_id,
        trace_id = %document.meta.trace_id,
        "processed engine ABI request"
    );

    let document_value = document
        .to_value()
        .map_err(EngineAbiError::SerializeCommercialDocument)?;
    let canonical_document_json =
        canonicalize_value(&document_value).map_err(EngineAbiError::CanonicalizeResponse)?;

    Ok(json!({
        "abi_version": ENGINE_ABI_VERSION,
        "operation": COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION,
        "payload": {
            "canonical_document_json": canonical_document_json,
            "document": document_value,
        },
        "status": "ok",
    }))
}

fn error_response(operation: Option<&str>, error: &EngineAbiError) -> Value {
    json!({
        "abi_version": ENGINE_ABI_VERSION,
        "error": {
            "code": error.code(),
            "message": error.to_string(),
            "remediation": error.remediation(),
        },
        "operation": operation,
        "status": "error",
    })
}

fn canonical_response(value: &Value) -> Vec<u8> {
    canonicalize_value(value).map_or_else(
        |_| INTERNAL_SERIALIZATION_ERROR_RESPONSE.to_vec(),
        String::into_bytes,
    )
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
/// assert_eq!(invoicekit_engine::crate_name(), "invoicekit-engine");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-engine"
}

#[cfg(test)]
mod tests {
    use super::{crate_name, process_abi_json, ENGINE_ABI_VERSION};
    use proptest::prelude::*;
    use serde::Deserialize;
    use serde_json::Value;

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
        assert_eq!(crate_name(), "invoicekit-engine");
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
    fn abi_version_is_v1() {
        assert_eq!(ENGINE_ABI_VERSION, 1);
    }

    #[test]
    fn golden_fixture_expected_response_matches_engine_bytes() {
        let fixture = golden_fixture();
        let actual = String::from_utf8(process_abi_json(fixture.request_bytes.as_bytes()))
            .expect("engine response is UTF-8 JSON");
        assert_eq!(
            actual, fixture.expected_response_bytes,
            "golden fixture expected_response_bytes must be regenerated from the engine contract"
        );
    }

    #[test]
    fn response_is_byte_identical_across_two_runs() {
        let fixture = golden_fixture();
        let first = process_abi_json(fixture.request_bytes.as_bytes());
        let second = process_abi_json(fixture.request_bytes.as_bytes());
        assert_eq!(first, second);
    }

    #[test]
    fn invalid_json_is_canonical_error_response() {
        let response = process_abi_json(b"{");
        let value: Value = serde_json::from_slice(&response).expect("response is JSON");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "invalid_request_json");
    }

    #[test]
    fn unsupported_abi_version_is_canonical_error_response() {
        let response = process_abi_json(
            br#"{"abi_version":2,"operation":"commercial_document.canonicalize","payload":{}}"#,
        );
        let value: Value = serde_json::from_slice(&response).expect("response is JSON");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "unsupported_abi_version");
    }

    #[test]
    fn invalid_document_payload_is_canonical_error_response() {
        let response = process_abi_json(
            br#"{"abi_version":1,"operation":"commercial_document.canonicalize","payload":{}}"#,
        );
        let value: Value = serde_json::from_slice(&response).expect("response is JSON");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "invalid_commercial_document");
    }

    #[test]
    fn unknown_request_envelope_field_is_rejected() {
        let response = process_abi_json(
            br#"{"abi_version":1,"operation":"commercial_document.canonicalize","payload":{},"unexpected":true}"#,
        );
        let value: Value = serde_json::from_slice(&response).expect("response is JSON");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "invalid_request_envelope");
    }

    #[test]
    fn duplicate_request_member_is_rejected_before_processing() {
        let response = process_abi_json(
            br#"{"abi_version":1,"abi_version":1,"operation":"commercial_document.canonicalize","payload":{}}"#,
        );
        let value: Value = serde_json::from_slice(&response).expect("response is JSON");
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "invalid_request_json");
    }

    proptest! {
        #[test]
        fn abi_processing_is_deterministic_for_arbitrary_bytes(input in proptest::collection::vec(any::<u8>(), 0..256)) {
            prop_assert_eq!(process_abi_json(&input), process_abi_json(&input));
        }
    }
}
