// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! South Africa **SARS** e-Invoicing adapter.
//!
//! The South African Revenue Service (SARS) operates the
//! country's evolving e-Invoicing regime. Issuers submit
//! typed JSON envelopes to SARS; SARS returns a Reference
//! plus acceptance status.
//!
//! Ships typed surface + [`MockSarsProvider`]; the live SARS
//! REST integration lands in a follow-up
//! `report-za-sars-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the SARS transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SarsEnvironment {
    /// SARS sandbox.
    Sandbox,
    /// Production.
    Production,
}

/// What the operator passes in to
/// [`SarsProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SarsSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SarsEnvironment,
    /// Issuer SARS VAT registration (10 ASCII digits,
    /// always starts with `4`).
    pub issuer_vat: String,
    /// Canonical signed JSON payload.
    pub payload: Vec<u8>,
}

/// SARS per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SarsStatus {
    /// Accepted by SARS.
    Accepted,
    /// Rejected by SARS.
    Rejected,
}

/// What [`SarsProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SarsSubmitEnvelope {
    /// SARS-assigned reference.
    pub sars_ref: String,
    /// Latest observed status.
    pub status: SarsStatus,
    /// RFC-3339 UTC timestamp SARS recorded.
    pub recorded_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum SarsError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// VAT registration didn't match the expected shape.
    #[error("invalid VAT registration: {0}")]
    BadVat(String),
    /// HTTP / TLS / DNS failure talking to SARS.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The SARS integration surface.
pub trait SarsProvider: Send + Sync {
    /// Submit one invoice to SARS.
    ///
    /// # Errors
    ///
    /// Returns [`SarsError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// SARS-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `SarsStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(&self, request: &SarsSubmitRequest) -> Result<SarsSubmitEnvelope, SarsError>;
}

/// Deterministic mock provider.
pub struct MockSarsProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockSarsProvider {
    /// Build a mock with deterministic timestamps + serials.
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

impl Default for MockSarsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SarsProvider for MockSarsProvider {
    fn submit_invoice(&self, request: &SarsSubmitRequest) -> Result<SarsSubmitEnvelope, SarsError> {
        validate_vat(&request.issuer_vat)?;
        if request.payload.is_empty() {
            return Err(SarsError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(SarsSubmitEnvelope {
            sars_ref: format!("ZA-{serial:0>12}"),
            status: SarsStatus::Accepted,
            recorded_at: self.fixed_recorded_at.clone(),
            reason: None,
        })
    }
}

/// Validate a SARS VAT registration — 10 ASCII digits
/// starting with `4`.
///
/// # Errors
///
/// Returns [`SarsError::BadVat`] on shape failure.
pub fn validate_vat(vat: &str) -> Result<(), SarsError> {
    if vat.len() == 10 && vat.starts_with('4') && vat.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SarsError::BadVat(format!(
            "VAT registration must be 10 ASCII digits starting with `4`, got {vat:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_za_sars::crate_name(),
///     "invoicekit-report-za-sars"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-za-sars"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SarsSubmitRequest {
        SarsSubmitRequest {
            tenant_id: "tenant-za-test".to_owned(),
            environment: SarsEnvironment::Sandbox,
            issuer_vat: "4123456789".to_owned(),
            payload: br#"{"invoice":"v1"}"#.to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_accepted() {
        let p = MockSarsProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, SarsStatus::Accepted);
        assert!(env.sars_ref.starts_with("ZA-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockSarsProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.sars_ref, env2.sars_ref);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockSarsProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, SarsError::BadPayload(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_vat() {
        let p = MockSarsProvider::default();
        let mut req = sample_request();
        req.issuer_vat = "5123456789".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, SarsError::BadVat(_)));
    }

    #[test]
    fn validate_vat_round_trip() {
        assert!(validate_vat("4123456789").is_ok());
        assert!(validate_vat("5123456789").is_err()); // doesn't start with 4
        assert!(validate_vat("412345678").is_err()); // 9 digits
        assert!(validate_vat("41234567890").is_err()); // 11 digits
        assert!(validate_vat("412345678A").is_err()); // non-digit
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = SarsSubmitEnvelope {
            sars_ref: "ZA-000000000007".to_owned(),
            status: SarsStatus::Rejected,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("VAT not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SarsSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
