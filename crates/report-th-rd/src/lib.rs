// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Thailand **Revenue Department** e-Tax Invoice & e-Receipt adapter.
//!
//! The Thai Revenue Department (กรมสรรพากร) operates the
//! e-Tax Invoice & e-Receipt regime. Issuers sign typed XML
//! with a Revenue Department-registered digital
//! certificate, submit to the RD portal, and receive an
//! acknowledgement carrying the RD-assigned reference.
//!
//! Two flavours: full e-Tax Invoice (signed XML over SMTP +
//! SOAP) and e-Tax Invoice by Email (lightweight PDF/A-3
//! with embedded XML for small operators).
//!
//! Ships typed surface + [`MockRdProvider`]; the live RD
//! REST integration lands in a follow-up
//! `report-th-rd-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the RD transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RdEnvironment {
    /// `etax-uat.rd.go.th` / RD UAT sandbox.
    Uat,
    /// `etax.rd.go.th` / production.
    Production,
}

/// Which Thai e-Tax flavour the engine is using.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RdFlavour {
    /// Full e-Tax Invoice (signed XML, SOAP).
    ETaxInvoice,
    /// e-Tax Invoice by Email (signed PDF/A-3, SMTP).
    EmailFlavour,
}

/// What the operator passes in to
/// [`RdProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RdSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: RdEnvironment,
    /// Flavour selector.
    pub flavour: RdFlavour,
    /// Issuer tax id (13 ASCII digits).
    pub issuer_tax_id: String,
    /// Canonical signed payload (XML for ETaxInvoice,
    /// PDF/A-3 for EmailFlavour).
    pub payload: Vec<u8>,
}

/// RD per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RdStatus {
    /// Acknowledged by RD.
    Acknowledged,
    /// Rejected by RD.
    Rejected,
}

/// What [`RdProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RdSubmitEnvelope {
    /// RD-assigned reference number.
    pub rd_ref: String,
    /// Latest observed status.
    pub status: RdStatus,
    /// RFC-3339 UTC timestamp RD recorded.
    pub acknowledged_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum RdError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// Tax id didn't match the 13-digit shape.
    #[error("invalid tax id: {0}")]
    BadTaxId(String),
    /// HTTP / TLS / DNS / SOAP / SMTP failure talking to RD.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The RD integration surface.
pub trait RdProvider: Send + Sync {
    /// Submit one invoice to RD.
    ///
    /// # Errors
    ///
    /// Returns [`RdError`] when local validation fails before
    /// the wire or transport fails on the wire. The
    /// RD-returned `Rejected` verdict is NOT an `Err` — it's
    /// surfaced via `RdStatus::Rejected` inside the envelope
    /// so the engine persists the rejection alongside its
    /// audit trail.
    fn submit_invoice(&self, request: &RdSubmitRequest) -> Result<RdSubmitEnvelope, RdError>;
}

/// Deterministic mock provider.
pub struct MockRdProvider {
    fixed_acknowledged_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockRdProvider {
    /// Build a mock with deterministic timestamps + serial
    /// references.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_acknowledged_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_acknowledged_at(acknowledged_at: impl Into<String>) -> Self {
        Self {
            fixed_acknowledged_at: acknowledged_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockRdProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl RdProvider for MockRdProvider {
    fn submit_invoice(&self, request: &RdSubmitRequest) -> Result<RdSubmitEnvelope, RdError> {
        validate_tax_id(&request.issuer_tax_id)?;
        if request.payload.is_empty() {
            return Err(RdError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(RdSubmitEnvelope {
            rd_ref: format!("TH-{serial:012}"),
            status: RdStatus::Acknowledged,
            acknowledged_at: self.fixed_acknowledged_at.clone(),
            reason: None,
        })
    }
}

/// Validate a Thai tax id — exactly 13 ASCII digits.
///
/// # Errors
///
/// Returns [`RdError::BadTaxId`] on shape failure.
pub fn validate_tax_id(tax_id: &str) -> Result<(), RdError> {
    if tax_id.len() == 13 && tax_id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(RdError::BadTaxId(format!(
            "tax id must be 13 ASCII digits, got {tax_id:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_th_rd::crate_name(),
///     "invoicekit-report-th-rd"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-th-rd"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> RdSubmitRequest {
        RdSubmitRequest {
            tenant_id: "tenant-th-test".to_owned(),
            environment: RdEnvironment::Uat,
            flavour: RdFlavour::ETaxInvoice,
            issuer_tax_id: "1234567890123".to_owned(),
            payload: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_acknowledged() {
        let p = MockRdProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, RdStatus::Acknowledged);
        assert!(env.rd_ref.starts_with("TH-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockRdProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.rd_ref, env2.rd_ref);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockRdProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, RdError::BadPayload(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_tax_id() {
        let p = MockRdProvider::default();
        let mut req = sample_request();
        req.issuer_tax_id = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, RdError::BadTaxId(_)));
    }

    #[test]
    fn validate_tax_id_round_trip() {
        assert!(validate_tax_id("1234567890123").is_ok());
        assert!(validate_tax_id("123456789012").is_err());
        assert!(validate_tax_id("12345678901234").is_err());
        assert!(validate_tax_id("123456789012A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = RdSubmitEnvelope {
            rd_ref: "TH-000000000007".to_owned(),
            status: RdStatus::Rejected,
            acknowledged_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("Invalid issuer tax id".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: RdSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
