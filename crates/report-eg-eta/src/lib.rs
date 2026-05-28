// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Egypt **ETA** e-Invoicing / e-Receipt adapter.
//!
//! The Egyptian Tax Authority (ETA — مصلحة الضرائب المصرية)
//! operates the country's e-Invoicing and e-Receipt
//! clearance through `api.invoicing.eta.gov.eg`. Issuers
//! sign typed JSON, submit, and receive a UUID + Long ID
//! plus a per-document hash.
//!
//! Ships typed surface + [`MockEtaProvider`]; the live ETA
//! REST integration lands in a follow-up
//! `report-eg-eta-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the ETA transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EtaEnvironment {
    /// `api.preprod.invoicing.eta.gov.eg` / preprod.
    Preprod,
    /// `api.invoicing.eta.gov.eg` / production.
    Production,
}

/// Which kind of document the issuer is producing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EtaDocumentKind {
    /// Standard e-Invoice (`I`).
    Invoice,
    /// Credit note (`C`).
    CreditNote,
    /// Debit note (`D`).
    DebitNote,
    /// e-Receipt (`R`, B2C).
    Receipt,
}

/// What the operator passes in to [`EtaProvider::submit`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EtaSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: EtaEnvironment,
    /// Document class.
    pub kind: EtaDocumentKind,
    /// Issuer tax registration number (9 ASCII digits) or
    /// national id (14 ASCII digits) for B2C receipts.
    pub issuer_tax_or_national_id: String,
    /// Canonical signed JSON payload.
    pub payload: Vec<u8>,
}

/// ETA per-document verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EtaStatus {
    /// Submitted; awaiting validation.
    Submitted,
    /// Valid — ETA cleared.
    Valid,
    /// Invalid — ETA rejected.
    Invalid,
}

/// What [`EtaProvider::submit`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EtaSubmitEnvelope {
    /// ETA-assigned UUID.
    pub uuid: String,
    /// Long ID the buyer uses to validate on the ETA public
    /// portal.
    pub long_id: String,
    /// 64-char canonical content hash (SHA-256 hex).
    pub content_hash_hex: String,
    /// Latest observed status.
    pub status: EtaStatus,
    /// RFC-3339 UTC timestamp ETA recorded.
    pub submitted_at: String,
    /// Reason text when status is `Invalid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum EtaError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// Tax / national id didn't match the expected shape.
    #[error("invalid tax/national id: {0}")]
    BadId(String),
    /// HTTP / TLS / DNS failure talking to ETA.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The ETA integration surface.
pub trait EtaProvider: Send + Sync {
    /// Submit one document to ETA.
    ///
    /// # Errors
    ///
    /// Returns [`EtaError`] when local validation fails before
    /// the wire or transport fails on the wire. The
    /// ETA-returned `Invalid` verdict is NOT an `Err` — it's
    /// surfaced via `EtaStatus::Invalid` inside the envelope
    /// so the engine persists the rejection alongside its
    /// audit trail.
    fn submit(&self, request: &EtaSubmitRequest) -> Result<EtaSubmitEnvelope, EtaError>;
}

/// Deterministic mock provider.
pub struct MockEtaProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockEtaProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_submitted_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_submitted_at(submitted_at: impl Into<String>) -> Self {
        Self {
            fixed_submitted_at: submitted_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockEtaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EtaProvider for MockEtaProvider {
    fn submit(&self, request: &EtaSubmitRequest) -> Result<EtaSubmitEnvelope, EtaError> {
        validate_tax_or_national_id(&request.issuer_tax_or_national_id)?;
        if request.payload.is_empty() {
            return Err(EtaError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        let uuid = format!("EG-{serial:0>8x}-{:0>8x}", serial.wrapping_mul(7));
        Ok(EtaSubmitEnvelope {
            uuid: uuid.clone(),
            long_id: format!("ETA-LONG-{serial:012}"),
            content_hash_hex: "0".repeat(64),
            status: EtaStatus::Submitted,
            submitted_at: self.fixed_submitted_at.clone(),
            reason: None,
        })
    }
}

/// Validate an Egyptian tax / national id — 9 ASCII digits
/// (tax registration) or 14 ASCII digits (national id).
///
/// # Errors
///
/// Returns [`EtaError::BadId`] on shape failure.
pub fn validate_tax_or_national_id(value: &str) -> Result<(), EtaError> {
    if matches!(value.len(), 9 | 14) && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(EtaError::BadId(format!(
            "tax/national id must be 9 or 14 ASCII digits, got {value:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_eg_eta::crate_name(),
///     "invoicekit-report-eg-eta"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-eg-eta"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> EtaSubmitRequest {
        EtaSubmitRequest {
            tenant_id: "tenant-eg-test".to_owned(),
            environment: EtaEnvironment::Preprod,
            kind: EtaDocumentKind::Invoice,
            issuer_tax_or_national_id: "123456789".to_owned(),
            payload: br#"{"invoice":"v1"}"#.to_vec(),
        }
    }

    #[test]
    fn submit_returns_submitted() {
        let p = MockEtaProvider::default();
        let env = p.submit(&sample_request()).unwrap();
        assert_eq!(env.status, EtaStatus::Submitted);
        assert!(env.uuid.starts_with("EG-"));
        assert!(env.long_id.starts_with("ETA-LONG-"));
        assert_eq!(env.content_hash_hex.len(), 64);
    }

    #[test]
    fn submit_serial_increments() {
        let p = MockEtaProvider::default();
        let env1 = p.submit(&sample_request()).unwrap();
        let env2 = p.submit(&sample_request()).unwrap();
        assert_ne!(env1.uuid, env2.uuid);
    }

    #[test]
    fn submit_rejects_empty_payload() {
        let p = MockEtaProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, EtaError::BadPayload(_)));
    }

    #[test]
    fn submit_rejects_bad_id() {
        let p = MockEtaProvider::default();
        let mut req = sample_request();
        req.issuer_tax_or_national_id = "BAD".to_owned();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, EtaError::BadId(_)));
    }

    #[test]
    fn validate_tax_or_national_id_round_trip() {
        assert!(validate_tax_or_national_id("123456789").is_ok());
        assert!(validate_tax_or_national_id("12345678901234").is_ok());
        assert!(validate_tax_or_national_id("12345").is_err());
        assert!(validate_tax_or_national_id("12345678901").is_err());
        assert!(validate_tax_or_national_id("12345678A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = EtaSubmitEnvelope {
            uuid: "EG-00000007-00000031".to_owned(),
            long_id: "ETA-LONG-000000000007".to_owned(),
            content_hash_hex: "a".repeat(64),
            status: EtaStatus::Invalid,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("tax id not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EtaSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
