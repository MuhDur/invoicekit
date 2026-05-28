// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Kenya **KRA eTIMS** (electronic Tax Invoice Management System) adapter.
//!
//! The Kenya Revenue Authority (KRA) operates eTIMS, the
//! country's e-invoicing clearance regime that replaced the
//! older Tax Invoice Management System (TIMS) hardware
//! requirement. Issuers transmit invoices via REST and
//! receive a CU Invoice Number + KRA-issued signature.
//!
//! Ships typed surface + [`MockEtimsProvider`]; the live KRA
//! REST integration lands in a follow-up
//! `report-ke-etims-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the KRA transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EtimsEnvironment {
    /// `etims-api-sbx.kra.go.ke` / KRA sandbox.
    Sandbox,
    /// `etims-api.kra.go.ke` / production.
    Production,
}

/// What the operator passes in to
/// [`EtimsProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EtimsSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: EtimsEnvironment,
    /// Issuer KRA PIN (11-character alphanumeric, format
    /// `A123456789Z`).
    pub issuer_pin: String,
    /// Canonical signed JSON payload.
    pub payload: Vec<u8>,
}

/// eTIMS per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EtimsStatus {
    /// Accepted by KRA eTIMS.
    Accepted,
    /// Rejected by KRA eTIMS.
    Rejected,
}

/// What [`EtimsProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EtimsSubmitEnvelope {
    /// CU Invoice Number (sequence the issuer prints on the
    /// receipt).
    pub cu_invoice_number: String,
    /// KRA-issued signature (opaque base64).
    pub kra_signature: String,
    /// Latest observed status.
    pub status: EtimsStatus,
    /// RFC-3339 UTC timestamp KRA recorded.
    pub recorded_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum EtimsError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// PIN didn't match the expected shape.
    #[error("invalid PIN: {0}")]
    BadPin(String),
    /// HTTP / TLS / DNS failure talking to KRA.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The KRA eTIMS integration surface.
pub trait EtimsProvider: Send + Sync {
    /// Submit one invoice to KRA eTIMS.
    ///
    /// # Errors
    ///
    /// Returns [`EtimsError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// KRA-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `EtimsStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(
        &self,
        request: &EtimsSubmitRequest,
    ) -> Result<EtimsSubmitEnvelope, EtimsError>;
}

/// Deterministic mock provider.
pub struct MockEtimsProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockEtimsProvider {
    /// Build a mock with deterministic timestamps + serial
    /// CU numbers.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_recorded_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_recorded_at(recorded_at: impl Into<String>) -> Self {
        Self {
            fixed_recorded_at: recorded_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockEtimsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EtimsProvider for MockEtimsProvider {
    fn submit_invoice(
        &self,
        request: &EtimsSubmitRequest,
    ) -> Result<EtimsSubmitEnvelope, EtimsError> {
        validate_pin(&request.issuer_pin)?;
        if request.payload.is_empty() {
            return Err(EtimsError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(EtimsSubmitEnvelope {
            cu_invoice_number: format!("KE-{serial:012}"),
            kra_signature: format!("MOCK-SIG-{serial:0>16x}"),
            status: EtimsStatus::Accepted,
            recorded_at: self.fixed_recorded_at.clone(),
            reason: None,
        })
    }
}

/// Validate a KRA PIN — 11 ASCII alphanumeric chars, first
/// + last are letters, middle 9 are digits.
///
/// # Errors
///
/// Returns [`EtimsError::BadPin`] on shape failure.
pub fn validate_pin(pin: &str) -> Result<(), EtimsError> {
    let bytes = pin.as_bytes();
    let shape_ok = bytes.len() == 11
        && bytes[0].is_ascii_alphabetic()
        && bytes[10].is_ascii_alphabetic()
        && bytes[1..10].iter().all(u8::is_ascii_digit);
    if shape_ok {
        Ok(())
    } else {
        Err(EtimsError::BadPin(format!(
            "PIN must be `A123456789Z` shape (letter + 9 digits + letter), got {pin:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ke_etims::crate_name(),
///     "invoicekit-report-ke-etims"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ke-etims"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> EtimsSubmitRequest {
        EtimsSubmitRequest {
            tenant_id: "tenant-ke-test".to_owned(),
            environment: EtimsEnvironment::Sandbox,
            issuer_pin: "A123456789Z".to_owned(),
            payload: br#"{"invoice":"v1"}"#.to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_accepted() {
        let p = MockEtimsProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, EtimsStatus::Accepted);
        assert!(env.cu_invoice_number.starts_with("KE-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockEtimsProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.cu_invoice_number, env2.cu_invoice_number);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockEtimsProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EtimsError::BadPayload(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_pin() {
        let p = MockEtimsProvider::default();
        let mut req = sample_request();
        req.issuer_pin = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EtimsError::BadPin(_)));
    }

    #[test]
    fn validate_pin_round_trip() {
        assert!(validate_pin("A123456789Z").is_ok());
        assert!(validate_pin("P012345678N").is_ok());
        assert!(validate_pin("A12345678").is_err()); // too short
        assert!(validate_pin("A123456789ZA").is_err()); // too long
        assert!(validate_pin("1123456789Z").is_err()); // first not letter
        assert!(validate_pin("A123456789-").is_err()); // last not letter
        assert!(validate_pin("A12345678AZ").is_err()); // middle not digits
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = EtimsSubmitEnvelope {
            cu_invoice_number: "KE-000000000007".to_owned(),
            kra_signature: "MOCK-SIG-0000000000000007".to_owned(),
            status: EtimsStatus::Rejected,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("PIN not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EtimsSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
