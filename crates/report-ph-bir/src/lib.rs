// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Philippines **BIR EIS** (Electronic Invoicing System) adapter.
//!
//! The Bureau of Internal Revenue (BIR) operates EIS, the
//! Philippine e-invoicing clearance regime. Issuers register
//! their POS / accounting system and receive an ATP
//! (Authority To Print). For each invoice the engine submits
//! a typed JSON envelope to EIS; BIR returns a JSON
//! acknowledgement with a reference number.
//!
//! Document kinds mirror BIR's EIS taxonomy: standard sales
//! invoice, official receipt (for services), credit memo,
//! debit memo, billing invoice.
//!
//! Ships typed surface + [`MockEisProvider`]; the live BIR
//! REST integration lands in a follow-up
//! `report-ph-bir-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the BIR EIS transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EisEnvironment {
    /// `eis-sandbox.bir.gov.ph` / EIS sandbox.
    Sandbox,
    /// `eis.bir.gov.ph` / production.
    Production,
}

/// EIS document class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EisDocumentKind {
    /// Sales Invoice (SI).
    SalesInvoice,
    /// Official Receipt (OR, for services).
    OfficialReceipt,
    /// Credit Memo (CM).
    CreditMemo,
    /// Debit Memo (DM).
    DebitMemo,
    /// Billing Invoice (BI).
    BillingInvoice,
}

/// What the operator passes in to
/// [`EisProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EisSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: EisEnvironment,
    /// Document class.
    pub kind: EisDocumentKind,
    /// Issuer TIN (Tax Identification Number, 9 base digits
    /// + optional branch code, allowing
    /// `NNNNNNNNN-BBB` shape).
    pub issuer_tin: String,
    /// BIR-issued ATP (Authority To Print) reference for the
    /// issuer's accredited POS.
    pub atp: String,
    /// Canonical JSON payload.
    pub invoice_json: Vec<u8>,
}

/// BIR EIS per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EisStatus {
    /// Received and validated.
    Acknowledged,
    /// Validation failed.
    Rejected,
}

/// What [`EisProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EisSubmitEnvelope {
    /// BIR-assigned reference number.
    pub reference_number: String,
    /// Latest observed status.
    pub status: EisStatus,
    /// RFC-3339 UTC timestamp BIR recorded.
    pub acknowledged_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum EisError {
    /// Invoice JSON failed shape validation before the wire.
    #[error("invoice json rejected: {0}")]
    BadJson(String),
    /// TIN didn't match the expected shape.
    #[error("invalid TIN: {0}")]
    BadTin(String),
    /// ATP missing / empty.
    #[error("missing ATP")]
    MissingAtp,
    /// HTTP / TLS / DNS failure talking to BIR EIS.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The BIR EIS integration surface.
pub trait EisProvider: Send + Sync {
    /// Submit one invoice to BIR EIS.
    ///
    /// # Errors
    ///
    /// Returns [`EisError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// BIR-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `EisStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(&self, request: &EisSubmitRequest) -> Result<EisSubmitEnvelope, EisError>;
}

/// Deterministic mock provider.
pub struct MockEisProvider {
    fixed_acknowledged_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockEisProvider {
    /// Build a mock with deterministic timestamps + serial
    /// reference numbers.
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

impl Default for MockEisProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EisProvider for MockEisProvider {
    fn submit_invoice(&self, request: &EisSubmitRequest) -> Result<EisSubmitEnvelope, EisError> {
        validate_tin(&request.issuer_tin)?;
        if request.atp.is_empty() {
            return Err(EisError::MissingAtp);
        }
        if request.invoice_json.is_empty() {
            return Err(EisError::BadJson("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(EisSubmitEnvelope {
            reference_number: format!("BIR-{serial:012}"),
            status: EisStatus::Acknowledged,
            acknowledged_at: self.fixed_acknowledged_at.clone(),
            reason: None,
        })
    }
}

/// Validate a Philippine TIN — 9 base digits with optional
/// `-BBB` branch suffix (3 digits).
///
/// # Errors
///
/// Returns [`EisError::BadTin`] on shape failure.
pub fn validate_tin(tin: &str) -> Result<(), EisError> {
    let (base, branch) = tin.split_once('-').map_or((tin, ""), |(a, b)| (a, b));
    let base_ok = base.len() == 9 && base.bytes().all(|b| b.is_ascii_digit());
    let branch_ok =
        branch.is_empty() || (branch.len() == 3 && branch.bytes().all(|b| b.is_ascii_digit()));
    if base_ok && branch_ok {
        Ok(())
    } else {
        Err(EisError::BadTin(format!(
            "TIN must be 9 digits with optional `-NNN` branch, got {tin:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ph_bir::crate_name(),
///     "invoicekit-report-ph-bir"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ph-bir"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> EisSubmitRequest {
        EisSubmitRequest {
            tenant_id: "tenant-ph-test".to_owned(),
            environment: EisEnvironment::Sandbox,
            kind: EisDocumentKind::SalesInvoice,
            issuer_tin: "123456789-001".to_owned(),
            atp: "ATP-2026-000001".to_owned(),
            invoice_json: br#"{"id":"INV-1"}"#.to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_acknowledged() {
        let p = MockEisProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, EisStatus::Acknowledged);
        assert!(env.reference_number.starts_with("BIR-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockEisProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.reference_number, env2.reference_number);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockEisProvider::default();
        let mut req = sample_request();
        req.invoice_json.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EisError::BadJson(_)));
    }

    #[test]
    fn submit_invoice_rejects_missing_atp() {
        let p = MockEisProvider::default();
        let mut req = sample_request();
        req.atp.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EisError::MissingAtp));
    }

    #[test]
    fn submit_invoice_rejects_bad_tin() {
        let p = MockEisProvider::default();
        let mut req = sample_request();
        req.issuer_tin = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EisError::BadTin(_)));
    }

    #[test]
    fn validate_tin_round_trip() {
        assert!(validate_tin("123456789").is_ok());
        assert!(validate_tin("123456789-001").is_ok());
        assert!(validate_tin("12345678").is_err());
        assert!(validate_tin("123456789-01").is_err()); // 2-digit branch
        assert!(validate_tin("12345678A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = EisSubmitEnvelope {
            reference_number: "BIR-000000000007".to_owned(),
            status: EisStatus::Rejected,
            acknowledged_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("ATP not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EisSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
