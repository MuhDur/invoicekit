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
    /// When set, the mock synthesizes an authority-side
    /// 전송오류 (transmission error) receipt instead of an
    /// approval — i.e. [`NtsStatus::Rejected`] surfaced as a
    /// receipt, never as `Err`.
    forced_rejection: Option<String>,
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
            forced_rejection: None,
        }
    }

    /// Force the mock to return an authority-side
    /// 전송오류 (transmission error) verdict carrying `reason`.
    ///
    /// The NTS clearance regime refuses malformed or
    /// non-conforming filings *after* they reach Hometax; that
    /// refusal is a recorded receipt with
    /// [`NtsStatus::Rejected`], not a transport `Err`. This
    /// knob exercises that branch deterministically. Pre-wire
    /// shape validation (BRN / empty payload) still runs first
    /// and still returns `Err`.
    #[must_use]
    pub fn with_forced_rejection(mut self, reason: impl Into<String>) -> Self {
        self.forced_rejection = Some(reason.into());
        self
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
        // Authority-side refusal (전송오류) is a *receipt*, not an
        // `Err`: NTS still records the filing and assigns an
        // approval number, but stamps the verdict Rejected with a
        // reason. Surfacing it inside the envelope lets the engine
        // persist the refusal in its audit trail.
        let (status, reason) = self
            .forced_rejection
            .as_ref()
            .map_or((NtsStatus::Approved, None), |reason| {
                (NtsStatus::Rejected, Some(reason.clone()))
            });
        Ok(NtsSubmitEnvelope {
            approval_no: format!("KR-{serial:0>21}"),
            status,
            issued_at: self.fixed_issued_at.clone(),
            reason,
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

/// Per-digit weights the NTS check-digit algorithm applies to
/// the first nine digits of a 사업자등록번호.
const BRN_CHECKSUM_WEIGHTS: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];

/// Validate a Korean BRN's **check digit** (검증번호), not just
/// its shape.
///
/// The National Tax Service assigns the tenth digit of every
/// 사업자등록번호 as a modulus-10 checksum: weight the first
/// nine digits by `[1,3,7,1,3,7,1,3,5]`, add `floor(d9 * 5 /
/// 10)`, and the valid tenth digit is `(10 - (sum mod 10)) mod
/// 10`. This is the same rule Hometax enforces before it will
/// accept an e-Tax Invoice, so a BRN that passes
/// [`validate_brn`] (shape) can still be refused here.
///
/// # Errors
///
/// Returns [`NtsError::BadBrn`] when the shape is wrong or the
/// computed check digit does not match the tenth digit.
pub fn validate_brn_checksum(brn: &str) -> Result<(), NtsError> {
    validate_brn(brn)?;
    let digits: Vec<u32> = brn.chars().filter_map(|c| c.to_digit(10)).collect();
    let mut sum: u32 = BRN_CHECKSUM_WEIGHTS
        .iter()
        .zip(&digits)
        .map(|(w, d)| w * d)
        .sum();
    sum += (digits[8] * 5) / 10;
    let expected = (10 - (sum % 10)) % 10;
    if expected == digits[9] {
        Ok(())
    } else {
        Err(NtsError::BadBrn(format!(
            "BRN check digit failed: expected tenth digit {expected}, got {} in {brn:?}",
            digits[9]
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
    fn forced_rejection_is_a_receipt_not_an_error() {
        // The authority-side 전송오류 verdict must surface inside the
        // envelope (status Rejected + reason), never as Err.
        let p = MockNtsProvider::default().with_forced_rejection("공급받는자 사업자등록번호 오류");
        let env = p.submit(&sample_request()).unwrap();
        assert_eq!(env.status, NtsStatus::Rejected);
        assert_eq!(
            env.reason.as_deref(),
            Some("공급받는자 사업자등록번호 오류")
        );
        // Even a rejected filing is recorded with an approval number.
        assert!(env.approval_no.starts_with("KR-"));
    }

    #[test]
    fn forced_rejection_still_runs_pre_wire_validation_first() {
        // Pre-wire shape failures outrank the forced authority verdict:
        // a bad BRN is still an Err, never a Rejected receipt.
        let p = MockNtsProvider::default().with_forced_rejection("late");
        let mut req = sample_request();
        req.issuer_brn = "NOPE".to_owned();
        assert!(matches!(p.submit(&req).unwrap_err(), NtsError::BadBrn(_)));
    }

    #[test]
    fn brn_checksum_accepts_real_korean_brns() {
        // Real, publicly-listed company 사업자등록번호 values whose
        // tenth digit satisfies the NTS check-digit rule.
        // Samsung Electronics 124-81-00998, NAVER 220-81-62517,
        // Kakao 120-81-47521.
        assert!(validate_brn_checksum("124-81-00998").is_ok());
        assert!(validate_brn_checksum("220-81-62517").is_ok());
        assert!(validate_brn_checksum("1208147521").is_ok());
    }

    #[test]
    fn brn_checksum_rejects_wrong_check_digit() {
        // Right shape, wrong tenth digit (the placeholder used by the
        // shape-only tests fails the real checksum).
        assert!(validate_brn("123-45-67890").is_ok());
        assert!(validate_brn_checksum("123-45-67890").is_err());
        // Flip the last digit of a real BRN.
        assert!(validate_brn_checksum("124-81-00999").is_err());
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
