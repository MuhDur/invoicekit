// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! China **Fapiao** clearance adapter (全面数字化的电子发票).
//!
//! The State Taxation Administration (国家税务总局, STA)
//! operates China's Golden Tax (金税) e-Fapiao clearance —
//! the fully digital electronic invoice regime that
//! supersedes the older Aisino tax-control hardware
//! mandate. Issuers register on the STA platform, request
//! invoice codes from a pre-allocated track (发票字轨), sign
//! typed XML, and submit; STA returns a 20-character invoice
//! number plus a QR-encoded fapiao for printing.
//!
//! Ships typed surface + [`MockFapiaoProvider`]; the live
//! STA REST integration lands in a follow-up
//! `report-cn-fapiao-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the STA transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FapiaoEnvironment {
    /// STA sandbox.
    Sandbox,
    /// Production.
    Production,
}

/// Fapiao classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FapiaoKind {
    /// 增值税专用发票 — VAT special invoice (B2B,
    /// input-VAT credit).
    SpecialVat,
    /// 增值税普通发票 — VAT general invoice (B2C, no
    /// input-VAT credit).
    GeneralVat,
    /// 电子发票（普通） — electronic general fapiao.
    ElectronicGeneral,
    /// 电子发票（专用） — electronic VAT special fapiao.
    ElectronicSpecial,
}

/// What the operator passes in to
/// [`FapiaoProvider::issue_fapiao`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FapiaoIssueRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: FapiaoEnvironment,
    /// Fapiao class.
    pub kind: FapiaoKind,
    /// Issuer 统一社会信用代码 (USCC — Unified Social
    /// Credit Code, 18 ASCII alphanumeric chars).
    pub issuer_uscc: String,
    /// Canonical signed XML payload.
    pub payload: Vec<u8>,
}

/// STA per-fapiao verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FapiaoStatus {
    /// 开票成功 — issuance succeeded.
    Issued,
    /// 开票失败 — issuance failed.
    Rejected,
    /// 已作废 — voided.
    Voided,
}

/// What [`FapiaoProvider::issue_fapiao`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FapiaoIssueEnvelope {
    /// 20-character fapiao number assigned by STA.
    pub fapiao_number: String,
    /// 12-digit fapiao code (发票代码).
    pub fapiao_code: String,
    /// Latest observed status.
    pub status: FapiaoStatus,
    /// RFC-3339 UTC timestamp STA recorded.
    pub issued_at: String,
    /// Reason text when status is `Rejected` or `Voided`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum FapiaoError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// USCC didn't match the 18-char alphanumeric shape.
    #[error("invalid USCC: {0}")]
    BadUscc(String),
    /// HTTP / TLS / DNS failure talking to STA.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The STA Fapiao integration surface.
pub trait FapiaoProvider: Send + Sync {
    /// Issue one fapiao.
    ///
    /// # Errors
    ///
    /// Returns [`FapiaoError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// STA-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `FapiaoStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn issue_fapiao(
        &self,
        request: &FapiaoIssueRequest,
    ) -> Result<FapiaoIssueEnvelope, FapiaoError>;

    /// Void a previously-issued fapiao.
    ///
    /// # Errors
    ///
    /// Returns [`FapiaoError::Transport`] when the fapiao
    /// number is unknown.
    fn void_fapiao(
        &self,
        environment: FapiaoEnvironment,
        fapiao_number: &str,
        reason: &str,
    ) -> Result<FapiaoIssueEnvelope, FapiaoError>;
}

/// Deterministic mock provider.
pub struct MockFapiaoProvider {
    fixed_issued_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockFapiaoProvider {
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

impl Default for MockFapiaoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FapiaoProvider for MockFapiaoProvider {
    fn issue_fapiao(
        &self,
        request: &FapiaoIssueRequest,
    ) -> Result<FapiaoIssueEnvelope, FapiaoError> {
        validate_uscc(&request.issuer_uscc)?;
        if request.payload.is_empty() {
            return Err(FapiaoError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(FapiaoIssueEnvelope {
            fapiao_number: format!("{serial:0>20}"),
            fapiao_code: format!("{:0>12}", serial.wrapping_mul(31)),
            status: FapiaoStatus::Issued,
            issued_at: self.fixed_issued_at.clone(),
            reason: None,
        })
    }

    fn void_fapiao(
        &self,
        _environment: FapiaoEnvironment,
        fapiao_number: &str,
        reason: &str,
    ) -> Result<FapiaoIssueEnvelope, FapiaoError> {
        if fapiao_number.is_empty() {
            return Err(FapiaoError::Transport("empty fapiao number".to_owned()));
        }
        Ok(FapiaoIssueEnvelope {
            fapiao_number: fapiao_number.to_owned(),
            fapiao_code: "0".repeat(12),
            status: FapiaoStatus::Voided,
            issued_at: self.fixed_issued_at.clone(),
            reason: Some(reason.to_owned()),
        })
    }
}

/// Validate a Chinese USCC — 18 ASCII alphanumeric chars.
///
/// # Errors
///
/// Returns [`FapiaoError::BadUscc`] on shape failure.
pub fn validate_uscc(uscc: &str) -> Result<(), FapiaoError> {
    if uscc.len() == 18 && uscc.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(FapiaoError::BadUscc(format!(
            "USCC must be 18 ASCII alphanumeric chars, got {uscc:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_cn_fapiao::crate_name(),
///     "invoicekit-report-cn-fapiao"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-cn-fapiao"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> FapiaoIssueRequest {
        FapiaoIssueRequest {
            tenant_id: "tenant-cn-test".to_owned(),
            environment: FapiaoEnvironment::Sandbox,
            kind: FapiaoKind::ElectronicSpecial,
            issuer_uscc: "91110108MA01234567".to_owned(),
            payload: b"<Fapiao/>".to_vec(),
        }
    }

    #[test]
    fn issue_returns_issued() {
        let p = MockFapiaoProvider::default();
        let env = p.issue_fapiao(&sample_request()).unwrap();
        assert_eq!(env.status, FapiaoStatus::Issued);
        assert_eq!(env.fapiao_number.len(), 20);
        assert_eq!(env.fapiao_code.len(), 12);
    }

    #[test]
    fn issue_serial_increments() {
        let p = MockFapiaoProvider::default();
        let env1 = p.issue_fapiao(&sample_request()).unwrap();
        let env2 = p.issue_fapiao(&sample_request()).unwrap();
        assert_ne!(env1.fapiao_number, env2.fapiao_number);
    }

    #[test]
    fn issue_rejects_empty_payload() {
        let p = MockFapiaoProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.issue_fapiao(&req).unwrap_err();
        assert!(matches!(err, FapiaoError::BadPayload(_)));
    }

    #[test]
    fn issue_rejects_bad_uscc() {
        let p = MockFapiaoProvider::default();
        let mut req = sample_request();
        req.issuer_uscc = "BAD".to_owned();
        let err = p.issue_fapiao(&req).unwrap_err();
        assert!(matches!(err, FapiaoError::BadUscc(_)));
    }

    #[test]
    fn void_returns_voided() {
        let p = MockFapiaoProvider::default();
        let env = p
            .void_fapiao(
                FapiaoEnvironment::Sandbox,
                "00000000000000000001",
                "buyer dispute",
            )
            .unwrap();
        assert_eq!(env.status, FapiaoStatus::Voided);
        assert_eq!(env.reason.as_deref(), Some("buyer dispute"));
    }

    #[test]
    fn void_rejects_empty_fapiao_number() {
        let p = MockFapiaoProvider::default();
        let err = p
            .void_fapiao(FapiaoEnvironment::Sandbox, "", "x")
            .unwrap_err();
        assert!(matches!(err, FapiaoError::Transport(_)));
    }

    #[test]
    fn validate_uscc_round_trip() {
        assert!(validate_uscc("91110108MA01234567").is_ok());
        assert!(validate_uscc("12345").is_err());
        assert!(validate_uscc("91110108MA012345678").is_err());
        assert!(validate_uscc("91110108MA0123456-").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = FapiaoIssueEnvelope {
            fapiao_number: "00000000000000000007".to_owned(),
            fapiao_code: "000000000007".to_owned(),
            status: FapiaoStatus::Rejected,
            issued_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("USCC not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: FapiaoIssueEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
