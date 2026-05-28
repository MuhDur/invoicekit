// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Taiwan **MOF** e-Invoice (電子發票) adapter.
//!
//! Taiwan's Ministry of Finance (財政部) operates the
//! electronic uniform invoice (電子統一發票) regime through
//! the e-Invoice platform at `einvoice.nat.gov.tw`. Issuers
//! submit B2B and B2C invoices to the platform; the MOF
//! returns an invoice number from a pre-allocated invoice
//! number book (發票字軌) and a random number used in the
//! periodic uniform invoice lottery (統一發票兌獎).
//!
//! Ships typed surface + [`MockMofProvider`]; the live MOF
//! integration lands in a follow-up `report-tw-mof-http`
//! crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the MOF transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MofEnvironment {
    /// `wwwtest.einvoice.nat.gov.tw` / MOF test.
    Test,
    /// `einvoice.nat.gov.tw` / production.
    Production,
}

/// Which kind of MOF e-Invoice the issuer is producing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MofInvoiceKind {
    /// B2B (三聯式 — triplicate).
    B2b,
    /// B2C (二聯式 — duplicate).
    B2c,
    /// Allowance / credit note (折讓單).
    Allowance,
    /// Void / cancellation (作廢).
    Void,
}

/// What the operator passes in to [`MofProvider::submit`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MofSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: MofEnvironment,
    /// Invoice kind.
    pub kind: MofInvoiceKind,
    /// Issuer 統一編號 (uniform number / VAT id,
    /// 8 ASCII digits).
    pub issuer_uniform_number: String,
    /// Canonical signed payload (MIG 3.2 XML / X.501 JSON).
    pub payload: Vec<u8>,
}

/// MOF per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MofStatus {
    /// 上傳成功 (upload succeeded).
    Accepted,
    /// 上傳失敗 (upload failed).
    Rejected,
}

/// What [`MofProvider::submit`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MofSubmitEnvelope {
    /// MOF invoice number (`AA-12345678` — two-letter track +
    /// 8-digit serial).
    pub invoice_number: String,
    /// 4-digit random number used for the uniform invoice
    /// lottery.
    pub random_number: String,
    /// Latest observed status.
    pub status: MofStatus,
    /// RFC-3339 UTC issuance timestamp MOF recorded.
    pub issued_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum MofError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// 統一編號 didn't match the 8-digit shape.
    #[error("invalid uniform number: {0}")]
    BadUniformNumber(String),
    /// HTTP / TLS / DNS failure talking to MOF.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The MOF integration surface.
pub trait MofProvider: Send + Sync {
    /// Submit one e-Invoice to MOF.
    ///
    /// # Errors
    ///
    /// Returns [`MofError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// MOF-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `MofStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit(&self, request: &MofSubmitRequest) -> Result<MofSubmitEnvelope, MofError>;
}

/// Deterministic mock provider.
pub struct MockMofProvider {
    fixed_issued_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockMofProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_issued_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_issued_at(issued_at: impl Into<String>) -> Self {
        Self {
            fixed_issued_at: issued_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockMofProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MofProvider for MockMofProvider {
    fn submit(&self, request: &MofSubmitRequest) -> Result<MofSubmitEnvelope, MofError> {
        validate_uniform_number(&request.issuer_uniform_number)?;
        if request.payload.is_empty() {
            return Err(MofError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(MofSubmitEnvelope {
            invoice_number: format!("AA-{serial:08}"),
            random_number: format!("{:04}", serial % 10_000),
            status: MofStatus::Accepted,
            issued_at: self.fixed_issued_at.clone(),
            reason: None,
        })
    }
}

/// Validate a Taiwanese 統一編號 — 8 ASCII digits.
///
/// # Errors
///
/// Returns [`MofError::BadUniformNumber`] on shape failure.
pub fn validate_uniform_number(value: &str) -> Result<(), MofError> {
    if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(MofError::BadUniformNumber(format!(
            "uniform number must be 8 ASCII digits, got {value:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_tw_mof::crate_name(),
///     "invoicekit-report-tw-mof"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-tw-mof"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> MofSubmitRequest {
        MofSubmitRequest {
            tenant_id: "tenant-tw-test".to_owned(),
            environment: MofEnvironment::Test,
            kind: MofInvoiceKind::B2b,
            issuer_uniform_number: "12345678".to_owned(),
            payload: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_returns_accepted() {
        let p = MockMofProvider::default();
        let env = p.submit(&sample_request()).unwrap();
        assert_eq!(env.status, MofStatus::Accepted);
        assert!(env.invoice_number.starts_with("AA-"));
        assert_eq!(env.random_number.len(), 4);
    }

    #[test]
    fn submit_serial_increments() {
        let p = MockMofProvider::default();
        let env1 = p.submit(&sample_request()).unwrap();
        let env2 = p.submit(&sample_request()).unwrap();
        assert_ne!(env1.invoice_number, env2.invoice_number);
    }

    #[test]
    fn submit_rejects_empty_payload() {
        let p = MockMofProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, MofError::BadPayload(_)));
    }

    #[test]
    fn submit_rejects_bad_uniform_number() {
        let p = MockMofProvider::default();
        let mut req = sample_request();
        req.issuer_uniform_number = "BAD".to_owned();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, MofError::BadUniformNumber(_)));
    }

    #[test]
    fn validate_uniform_number_round_trip() {
        assert!(validate_uniform_number("12345678").is_ok());
        assert!(validate_uniform_number("1234567").is_err());
        assert!(validate_uniform_number("123456789").is_err());
        assert!(validate_uniform_number("1234567A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = MofSubmitEnvelope {
            invoice_number: "AA-00000007".to_owned(),
            random_number: "1234".to_owned(),
            status: MofStatus::Rejected,
            issued_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("uniform number not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: MofSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
