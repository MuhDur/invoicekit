// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! South Korea **NTS** e-Tax Invoice (전자세금계산서) adapter.
//!
//! Korea's National Tax Service (국세청, NTS) operates the
//! e-Tax Invoice (전자세금계산서) clearance regime. Issuers
//! sign typed XML with an Authorized Certification Authority
//! certificate, submit to the NTS portal (Hometax), and
//! receive an approval number plus issuance timestamp.
//!
//! Document kinds mirror NTS's classification: 일반
//! (standard) tax invoice, 면세 (tax-exempt) invoice, 수정
//! (correction) — typed here as a Rust enum.
//!
//! Ships typed surface + [`MockNtsProvider`]; the live NTS
//! REST integration lands in a follow-up
//! `report-kr-nts-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the NTS transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NtsEnvironment {
    /// Hometax test environment.
    Test,
    /// `hometax.go.kr` / production.
    Production,
}

/// e-Tax Invoice kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NtsInvoiceKind {
    /// 일반 (standard taxable invoice).
    Standard,
    /// 면세 (tax-exempt invoice).
    Exempt,
    /// 수정 (correction note).
    Correction,
}

/// What the operator passes in to [`NtsProvider::submit`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NtsSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: NtsEnvironment,
    /// Invoice kind.
    pub kind: NtsInvoiceKind,
    /// Issuer 사업자등록번호 (Business Registration Number,
    /// 10 ASCII digits — `NNN-NN-NNNNN` collapses to
    /// `NNNNNNNNNN`).
    pub issuer_brn: String,
    /// Canonical signed XML payload.
    pub invoice_xml: Vec<u8>,
}

/// NTS per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NtsStatus {
    /// 전송완료 (transmission complete).
    Approved,
    /// 전송오류 (transmission error / validation failed).
    Rejected,
}

/// What [`NtsProvider::submit`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NtsSubmitEnvelope {
    /// NTS-assigned 승인번호 (approval number, 24 ASCII chars).
    pub approval_no: String,
    /// Latest observed status.
    pub status: NtsStatus,
    /// RFC-3339 UTC issuance timestamp NTS recorded.
    pub issued_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum NtsError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// BRN didn't match the 10-digit shape.
    #[error("invalid BRN: {0}")]
    BadBrn(String),
    /// HTTP / TLS / DNS failure talking to NTS.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The NTS integration surface.
pub trait NtsProvider: Send + Sync {
    /// Submit one e-Tax Invoice to NTS.
    ///
    /// # Errors
    ///
    /// Returns [`NtsError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// NTS-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `NtsStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit(&self, request: &NtsSubmitRequest) -> Result<NtsSubmitEnvelope, NtsError>;
}

/// Deterministic mock provider.
pub struct MockNtsProvider {
    fixed_issued_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockNtsProvider {
    /// Build a mock with deterministic timestamps + serial
    /// approval numbers.
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

impl Default for MockNtsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl NtsProvider for MockNtsProvider {
    fn submit(&self, request: &NtsSubmitRequest) -> Result<NtsSubmitEnvelope, NtsError> {
        validate_brn(&request.issuer_brn)?;
        if request.invoice_xml.is_empty() {
            return Err(NtsError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(NtsSubmitEnvelope {
            approval_no: format!("KR-{serial:0>21}"),
            status: NtsStatus::Approved,
            issued_at: self.fixed_issued_at.clone(),
            reason: None,
        })
    }
}

/// Validate a Korean BRN — 10 ASCII digits (hyphens
/// stripped before checking).
///
/// # Errors
///
/// Returns [`NtsError::BadBrn`] on shape failure.
pub fn validate_brn(brn: &str) -> Result<(), NtsError> {
    let collapsed: String = brn.chars().filter(|c| *c != '-').collect();
    if collapsed.len() == 10 && collapsed.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(NtsError::BadBrn(format!(
            "BRN must be 10 ASCII digits (optionally hyphenated as NNN-NN-NNNNN), got {brn:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_kr_nts::crate_name(),
///     "invoicekit-report-kr-nts"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-kr-nts"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> NtsSubmitRequest {
        NtsSubmitRequest {
            tenant_id: "tenant-kr-test".to_owned(),
            environment: NtsEnvironment::Test,
            kind: NtsInvoiceKind::Standard,
            issuer_brn: "123-45-67890".to_owned(),
            invoice_xml: b"<TaxInvoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_returns_approved() {
        let p = MockNtsProvider::default();
        let env = p.submit(&sample_request()).unwrap();
        assert_eq!(env.status, NtsStatus::Approved);
        assert!(env.approval_no.starts_with("KR-"));
    }

    #[test]
    fn submit_serial_increments() {
        let p = MockNtsProvider::default();
        let env1 = p.submit(&sample_request()).unwrap();
        let env2 = p.submit(&sample_request()).unwrap();
        assert_ne!(env1.approval_no, env2.approval_no);
    }

    #[test]
    fn submit_rejects_empty_payload() {
        let p = MockNtsProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, NtsError::BadXml(_)));
    }

    #[test]
    fn submit_rejects_bad_brn() {
        let p = MockNtsProvider::default();
        let mut req = sample_request();
        req.issuer_brn = "BAD".to_owned();
        let err = p.submit(&req).unwrap_err();
        assert!(matches!(err, NtsError::BadBrn(_)));
    }

    #[test]
    fn validate_brn_round_trip() {
        assert!(validate_brn("1234567890").is_ok());
        assert!(validate_brn("123-45-67890").is_ok());
        assert!(validate_brn("12345").is_err());
        assert!(validate_brn("123-45-6789A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = NtsSubmitEnvelope {
            approval_no: "KR-000000000000000000007".to_owned(),
            status: NtsStatus::Rejected,
            issued_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("BRN not in NTS registry".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: NtsSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
