// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-052 veraPDF adapter.
//!
//! The validator-verapdf sidecar wraps the JVM-only veraPDF library
//! and exposes a `validator.validate_pdf` JSON-RPC method that
//! takes a base64-encoded PDF body and returns a JSON `PdfAReport`.
//! This module is the Rust side of that contract: it parses the
//! sidecar's response into a typed [`PdfAReport`] and exposes a
//! `parse_response` helper that callers wrap in their own
//! HTTP/transport layer.
//!
//! The wire format is owned by this file. The sidecar's Java side
//! produces it via `services/validator-common/src/main/java/dev/
//! invoicekit/validator/PdfAReport.java`; any change to the JSON
//! shape MUST land in both places at the same time. The unit tests
//! pin a representative response so an accidental rename surfaces
//! as a test failure.

#![allow(
    clippy::option_if_let_else,
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    clippy::redundant_closure_for_method_calls
)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Bead identifier carried on emitted log records.
pub const VERAPDF_ADAPTER_BEAD_ID: &str = "invoices-t-052-verapdf-adapter-ksc";

/// JSON-RPC method name the validator-verapdf sidecar exposes.
pub const RPC_METHOD: &str = "validator.validate_pdf";

/// PDF/A conformance flavour the caller asks the sidecar to grade
/// against. `pdfa-3b` is the InvoiceKit default (matches the
/// Factur-X requirement); `pdfa-3a` is the stricter accessibility-
/// inclusive flavour; `pdfa-3u` is the Unicode-mapped variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfAFlavour {
    /// PDF/A-3a (accessibility + tagging).
    #[serde(rename = "pdfa-3a")]
    Pdfa3A,
    /// PDF/A-3b (basic; Factur-X default).
    #[serde(rename = "pdfa-3b")]
    Pdfa3B,
    /// PDF/A-3u (Unicode-mapped).
    #[serde(rename = "pdfa-3u")]
    Pdfa3U,
}

impl PdfAFlavour {
    /// Wire-format spelling (e.g. `"pdfa-3b"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pdfa3A => "pdfa-3a",
            Self::Pdfa3B => "pdfa-3b",
            Self::Pdfa3U => "pdfa-3u",
        }
    }
}

/// One PDF/A conformance check failure as reported by veraPDF.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PdfAFinding {
    /// veraPDF rule id (e.g. `6.1.13-1`, `6.3.4-1`).
    pub rule_id: String,
    /// Severity — `violation` for spec rule failures, `fatal` for
    /// library-level errors that prevented the check from running.
    pub severity: String,
    /// Human-readable explanation of the failure.
    pub message: String,
    /// Optional pointer into the PDF (object id, page index, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// Typed report parsed from the sidecar's `result.report` payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PdfAReport {
    /// Flavour the sidecar graded against.
    pub flavour: String,
    /// Trace id echoed from the request.
    pub trace_id: String,
    /// `true` when the document passes the flavour cleanly.
    pub conformant: bool,
    /// Failures + warnings. Empty when `conformant == true`.
    #[serde(default)]
    pub failures: Vec<PdfAFinding>,
    /// Populated when the sidecar's veraPDF library itself raised
    /// an exception (missing classpath, corrupt PDF that the
    /// library couldn't even open). When set, `conformant` is
    /// `false` and `failures` typically contains a single
    /// `*-LIBRARY-ERROR` finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    /// Operator-readable explanation of `error_class`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl PdfAReport {
    /// True when veraPDF graded the document as conformant AND no
    /// library-level error fired.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conformant && self.error_class.is_none()
    }

    /// Filter helper: the rule ids of every failure with the given
    /// severity. Useful for the "must not contain any fatal" gate
    /// callers wrap around the adapter response.
    #[must_use]
    pub fn rule_ids_with_severity(&self, severity: &str) -> Vec<&str> {
        self.failures
            .iter()
            .filter(|f| f.severity == severity)
            .map(|f| f.rule_id.as_str())
            .collect()
    }
}

/// Top-level shape of the sidecar's `result` object.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatePdfResult {
    /// Sidecar backend tag (`jvm:verapdf`).
    pub backend: String,
    /// Sidecar service name (`validator-verapdf`).
    pub service: String,
    /// Maven coordinate of the veraPDF dependency the sidecar built against.
    pub oracle_coordinate: String,
    /// Sidecar-startup class-check name.
    pub oracle_class: String,
    /// Flavour the caller asked the sidecar to grade.
    pub flavour: String,
    /// Trace id echoed from the request.
    pub trace_id: String,
    /// Wall-clock ms the sidecar spent inside the validator call.
    pub duration_ms: u64,
    /// PDF metadata stamped by the sidecar.
    pub document: DocumentMeta,
    /// The actual veraPDF report.
    pub report: PdfAReport,
}

/// PDF metadata the sidecar stamps on every response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentMeta {
    /// Always `application/pdf`.
    pub content_type: String,
    /// Body length in bytes.
    pub byte_length: u64,
    /// Hex SHA-256 of the body bytes.
    pub sha256: String,
}

/// Errors raised by [`parse_response`].
#[derive(Debug, Error)]
pub enum AdapterError {
    /// The response was not JSON-RPC 2.0 shaped.
    #[error("response is not JSON-RPC 2.0 shaped: {0}")]
    BadEnvelope(String),
    /// The response carried a JSON-RPC error object instead of a result.
    #[error("sidecar returned JSON-RPC error {code}: {message}")]
    SidecarError {
        /// JSON-RPC error code.
        code: i64,
        /// JSON-RPC error message.
        message: String,
    },
    /// The result payload didn't deserialize into [`ValidatePdfResult`].
    #[error("result payload did not match the documented PdfAReport shape: {0}")]
    BadResult(String),
}

/// Parse a sidecar response body into a typed [`ValidatePdfResult`].
///
/// # Errors
///
/// Returns [`AdapterError::BadEnvelope`] when the response isn't
/// JSON-RPC 2.0, [`AdapterError::SidecarError`] when the sidecar
/// returned a JSON-RPC error object, and [`AdapterError::BadResult`]
/// when the result payload doesn't match the documented shape.
pub fn parse_response(raw: &[u8]) -> Result<ValidatePdfResult, AdapterError> {
    let value: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| AdapterError::BadEnvelope(e.to_string()))?;
    if value.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Err(AdapterError::BadEnvelope(
            "missing or non-2.0 jsonrpc field".into(),
        ));
    }
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let message = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message)")
            .to_owned();
        return Err(AdapterError::SidecarError { code, message });
    }
    let result = value
        .get("result")
        .ok_or_else(|| AdapterError::BadEnvelope("response missing result".into()))?;
    serde_json::from_value(result.clone()).map_err(|e| AdapterError::BadResult(e.to_string()))
}

/// Construct the JSON-RPC request body the sidecar expects. The
/// future T-029 RPC client wraps this for transport; tests and
/// other adapters can use it standalone.
///
/// # Panics
///
/// Panics only via the internal `serde_json::to_vec` `expect`,
/// which would indicate that a `Vec<u8>` failed to allocate during
/// JSON serialization — impossible on a healthy host.
#[must_use]
pub fn build_request(
    rpc_id: &str,
    pdf_bytes: &[u8],
    flavour: PdfAFlavour,
    trace_id: &str,
) -> Vec<u8> {
    use serde_json::{json, to_vec};
    let body = json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "method": RPC_METHOD,
        "params": {
            "document": {
                "pdf_base64": base64_encode(pdf_bytes),
            },
            "flavour": flavour.as_str(),
            "trace_id": trace_id,
        },
    });
    to_vec(&body).expect("JSON-RPC request must serialize")
}

fn base64_encode(bytes: &[u8]) -> String {
    // Standard MIME base64 alphabet, hand-rolled so the adapter
    // doesn't take a `base64` crate dependency for one function.
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n =
            (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = u32::from(bytes[i]) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn passing_response() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "trace-001",
            "result": {
                "backend": "jvm:verapdf",
                "service": "validator-verapdf",
                "oracle_coordinate": "org.verapdf:verapdf-library:1.27.1",
                "oracle_class": "org.verapdf.pdfa.Foundries",
                "flavour": "pdfa-3b",
                "trace_id": "trace-001",
                "duration_ms": 42,
                "document": {
                    "content_type": "application/pdf",
                    "byte_length": 12345,
                    "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                },
                "report": {
                    "flavour": "pdfa-3b",
                    "trace_id": "trace-001",
                    "conformant": true,
                    "failures": []
                }
            }
        }))
        .unwrap()
    }

    fn failing_response() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "trace-002",
            "result": {
                "backend": "jvm:verapdf",
                "service": "validator-verapdf",
                "oracle_coordinate": "org.verapdf:verapdf-library:1.27.1",
                "oracle_class": "org.verapdf.pdfa.Foundries",
                "flavour": "pdfa-3b",
                "trace_id": "trace-002",
                "duration_ms": 87,
                "document": {
                    "content_type": "application/pdf",
                    "byte_length": 23456,
                    "sha256": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                },
                "report": {
                    "flavour": "pdfa-3b",
                    "trace_id": "trace-002",
                    "conformant": false,
                    "failures": [
                        {
                            "rule_id": "6.1.13-1",
                            "severity": "violation",
                            "message": "Document does not contain a CIDFont subset",
                            "location": "Page[0]/Font[F1]"
                        },
                        {
                            "rule_id": "6.3.4-1",
                            "severity": "violation",
                            "message": "Annotation requires an Appearance Stream"
                        }
                    ]
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn parse_passing_response_yields_clean_report() {
        let result = parse_response(&passing_response()).unwrap();
        assert_eq!(result.backend, "jvm:verapdf");
        assert_eq!(result.flavour, "pdfa-3b");
        assert!(result.report.conformant);
        assert!(result.report.is_clean());
        assert!(result.report.failures.is_empty());
    }

    #[test]
    fn parse_failing_response_yields_two_findings() {
        let result = parse_response(&failing_response()).unwrap();
        assert!(!result.report.conformant);
        assert!(!result.report.is_clean());
        assert_eq!(result.report.failures.len(), 2);
        let violation_ids = result.report.rule_ids_with_severity("violation");
        assert_eq!(violation_ids, vec!["6.1.13-1", "6.3.4-1"]);
    }

    #[test]
    fn parse_rpc_error_returns_sidecar_error_variant() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "trace-003",
            "error": {
                "code": -32602,
                "message": "params.document.pdf_base64 must be a non-empty base64 string"
            }
        }))
        .unwrap();
        let err = parse_response(&body).unwrap_err();
        match err {
            AdapterError::SidecarError { code, message } => {
                assert_eq!(code, -32602);
                assert!(message.contains("pdf_base64"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_non_json_body_is_bad_envelope() {
        let err = parse_response(b"not json").unwrap_err();
        assert!(matches!(err, AdapterError::BadEnvelope(_)));
    }

    #[test]
    fn parse_missing_jsonrpc_field_is_bad_envelope() {
        let body = serde_json::to_vec(&json!({ "id": "x", "result": {} })).unwrap();
        let err = parse_response(&body).unwrap_err();
        assert!(matches!(err, AdapterError::BadEnvelope(_)));
    }

    #[test]
    fn parse_missing_result_is_bad_envelope() {
        let body = serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": "x" })).unwrap();
        let err = parse_response(&body).unwrap_err();
        assert!(matches!(err, AdapterError::BadEnvelope(_)));
    }

    #[test]
    fn parse_malformed_result_is_bad_result() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "x",
            "result": {"backend": "jvm:verapdf"}
        }))
        .unwrap();
        let err = parse_response(&body).unwrap_err();
        assert!(matches!(err, AdapterError::BadResult(_)));
    }

    #[test]
    fn build_request_round_trips_through_serde_json() {
        let body = build_request("trace-007", b"%PDF-1.4\n", PdfAFlavour::Pdfa3B, "trace-007");
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
        assert_eq!(
            value.get("method").and_then(|v| v.as_str()),
            Some(RPC_METHOD)
        );
        assert_eq!(
            value["params"]["document"]["pdf_base64"].as_str(),
            Some("JVBERi0xLjQK"),
        );
        assert_eq!(value["params"]["flavour"].as_str(), Some("pdfa-3b"));
        assert_eq!(value["params"]["trace_id"].as_str(), Some("trace-007"));
    }

    #[test]
    fn base64_encode_handles_all_three_remainder_cases() {
        // No remainder, 1-byte remainder, 2-byte remainder.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn library_error_response_surfaces_error_class() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "trace-004",
            "result": {
                "backend": "jvm:verapdf",
                "service": "validator-verapdf",
                "oracle_coordinate": "org.verapdf:verapdf-library:1.27.1",
                "oracle_class": "org.verapdf.pdfa.Foundries",
                "flavour": "pdfa-3b",
                "trace_id": "trace-004",
                "duration_ms": 5,
                "document": {
                    "content_type": "application/pdf",
                    "byte_length": 100,
                    "sha256": "00"
                },
                "report": {
                    "flavour": "pdfa-3b",
                    "trace_id": "trace-004",
                    "conformant": false,
                    "error_class": "org.verapdf.parser.NotPdfBoundaryException",
                    "error_message": "PDF document boundary not found",
                    "failures": [{
                        "rule_id": "VERAPDF-LIBRARY-ERROR",
                        "severity": "fatal",
                        "message": "veraPDF library raised an exception before producing a report"
                    }]
                }
            }
        }))
        .unwrap();
        let result = parse_response(&body).unwrap();
        assert!(!result.report.conformant);
        assert!(!result.report.is_clean());
        assert_eq!(
            result.report.error_class.as_deref(),
            Some("org.verapdf.parser.NotPdfBoundaryException"),
        );
        let fatal_ids = result.report.rule_ids_with_severity("fatal");
        assert_eq!(fatal_ids, vec!["VERAPDF-LIBRARY-ERROR"]);
    }
}
