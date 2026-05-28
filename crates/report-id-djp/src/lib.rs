// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Indonesia **DJP** e-Faktur (Pajak Pertambahan Nilai) adapter.
//!
//! Indonesia's Direktorat Jenderal Pajak (DJP, the
//! Directorate General of Taxes) operates e-Faktur — the
//! electronic VAT invoice clearance regime. Issuers consume
//! an **NSFP** (Nomor Seri Faktur Pajak — a tax-authority
//! pre-allocated invoice serial), build a typed Faktur XML,
//! sign with an e-Faktur certificate, and submit to DJP. The
//! authority returns an `Approved` envelope or a typed
//! rejection.
//!
//! Faktur type codes mirror DJP's `kode_jenis`:
//! 01 standard tax-payable, 02 government collector,
//! 03 collector other than government, 04 DPP custom basis,
//! 06 other (export of services, BKP tidak berwujud),
//! 07 export, 08 free / exempt, 09 retail.
//!
//! Ships typed surface + [`MockDjpProvider`]; the live DJP
//! REST integration lands in a follow-up
//! `report-id-djp-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the DJP transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DjpEnvironment {
    /// `efaktur-uat.pajak.go.id` / DJP UAT sandbox.
    Uat,
    /// `efaktur.pajak.go.id` / production.
    Production,
}

/// e-Faktur kode_jenis (transaction-type code).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FakturKodeJenis {
    /// 01 standard tax-payable.
    Standard,
    /// 02 government collector.
    GovCollector,
    /// 03 collector other than government.
    OtherCollector,
    /// 04 DPP custom basis.
    DppCustom,
    /// 06 other (export of services, BKP tidak berwujud).
    Other,
    /// 07 export.
    Export,
    /// 08 free / exempt.
    Exempt,
    /// 09 retail.
    Retail,
}

impl FakturKodeJenis {
    /// DJP kode_jenis code for this variant.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Standard => "01",
            Self::GovCollector => "02",
            Self::OtherCollector => "03",
            Self::DppCustom => "04",
            Self::Other => "06",
            Self::Export => "07",
            Self::Exempt => "08",
            Self::Retail => "09",
        }
    }
}

/// What the operator passes in to
/// [`DjpProvider::submit_faktur`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DjpSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: DjpEnvironment,
    /// Faktur kode_jenis.
    pub kode_jenis: FakturKodeJenis,
    /// Issuer NPWP (15 or 16 ASCII digits — 15 legacy, 16
    /// post-PMK 112/2022).
    pub issuer_npwp: String,
    /// NSFP (Nomor Seri Faktur Pajak, 16 digits formatted
    /// without separators).
    pub nsfp: String,
    /// Canonical signed Faktur XML payload.
    pub faktur_xml: Vec<u8>,
}

/// DJP per-Faktur verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DjpStatus {
    /// Submitted; awaiting DJP validation.
    Submitted,
    /// Approved by DJP.
    Approved,
    /// Rejected by DJP (typed reason).
    Rejected,
}

/// What [`DjpProvider::submit_faktur`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DjpSubmitEnvelope {
    /// DJP-issued nomor referensi (reference number).
    pub nomor_referensi: String,
    /// NSFP echoed by DJP.
    pub nsfp: String,
    /// Latest observed status.
    pub status: DjpStatus,
    /// RFC-3339 UTC timestamp DJP recorded.
    pub submitted_at: String,
    /// Free-form reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alasan: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum DjpError {
    /// Faktur XML failed shape validation before the wire.
    #[error("faktur xml rejected: {0}")]
    BadXml(String),
    /// NPWP didn't match the 15 / 16-digit shape.
    #[error("invalid NPWP: {0}")]
    BadNpwp(String),
    /// NSFP didn't match the 16-digit shape.
    #[error("invalid NSFP: {0}")]
    BadNsfp(String),
    /// HTTP / TLS / DNS failure talking to DJP.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The DJP integration surface.
pub trait DjpProvider: Send + Sync {
    /// Submit one Faktur to DJP.
    ///
    /// # Errors
    ///
    /// Returns [`DjpError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// DJP-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `DjpStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_faktur(&self, request: &DjpSubmitRequest) -> Result<DjpSubmitEnvelope, DjpError>;
}

/// Deterministic mock provider.
pub struct MockDjpProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockDjpProvider {
    /// Build a mock with deterministic timestamps + serial
    /// nomor referensi.
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

impl Default for MockDjpProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DjpProvider for MockDjpProvider {
    fn submit_faktur(&self, request: &DjpSubmitRequest) -> Result<DjpSubmitEnvelope, DjpError> {
        validate_npwp(&request.issuer_npwp)?;
        validate_nsfp(&request.nsfp)?;
        if request.faktur_xml.is_empty() {
            return Err(DjpError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(DjpSubmitEnvelope {
            nomor_referensi: format!("DJP-{serial:012}"),
            nsfp: request.nsfp.clone(),
            status: DjpStatus::Approved,
            submitted_at: self.fixed_submitted_at.clone(),
            alasan: None,
        })
    }
}

/// Validate an Indonesian NPWP — 15 (legacy) or 16
/// (PMK 112/2022) ASCII digits.
///
/// # Errors
///
/// Returns [`DjpError::BadNpwp`] on shape failure.
pub fn validate_npwp(npwp: &str) -> Result<(), DjpError> {
    if matches!(npwp.len(), 15 | 16) && npwp.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(DjpError::BadNpwp(format!(
            "NPWP must be 15 or 16 ASCII digits, got {npwp:?}"
        )))
    }
}

/// Validate an NSFP — exactly 16 ASCII digits (no
/// separators).
///
/// # Errors
///
/// Returns [`DjpError::BadNsfp`] on shape failure.
pub fn validate_nsfp(nsfp: &str) -> Result<(), DjpError> {
    if nsfp.len() == 16 && nsfp.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(DjpError::BadNsfp(format!(
            "NSFP must be 16 ASCII digits, got {nsfp:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_id_djp::crate_name(),
///     "invoicekit-report-id-djp"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-id-djp"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DjpSubmitRequest {
        DjpSubmitRequest {
            tenant_id: "tenant-id-test".to_owned(),
            environment: DjpEnvironment::Uat,
            kode_jenis: FakturKodeJenis::Standard,
            issuer_npwp: "1".repeat(16),
            nsfp: "2".repeat(16),
            faktur_xml: b"<Faktur/>".to_vec(),
        }
    }

    #[test]
    fn submit_faktur_returns_approved() {
        let p = MockDjpProvider::default();
        let env = p.submit_faktur(&sample_request()).unwrap();
        assert_eq!(env.status, DjpStatus::Approved);
        assert!(env.nomor_referensi.starts_with("DJP-"));
        assert_eq!(env.nsfp, "2".repeat(16));
    }

    #[test]
    fn submit_faktur_serial_increments() {
        let p = MockDjpProvider::default();
        let env1 = p.submit_faktur(&sample_request()).unwrap();
        let env2 = p.submit_faktur(&sample_request()).unwrap();
        assert_ne!(env1.nomor_referensi, env2.nomor_referensi);
    }

    #[test]
    fn submit_faktur_rejects_empty_payload() {
        let p = MockDjpProvider::default();
        let mut req = sample_request();
        req.faktur_xml.clear();
        let err = p.submit_faktur(&req).unwrap_err();
        assert!(matches!(err, DjpError::BadXml(_)));
    }

    #[test]
    fn submit_faktur_rejects_bad_npwp() {
        let p = MockDjpProvider::default();
        let mut req = sample_request();
        req.issuer_npwp = "BAD".to_owned();
        let err = p.submit_faktur(&req).unwrap_err();
        assert!(matches!(err, DjpError::BadNpwp(_)));
    }

    #[test]
    fn submit_faktur_rejects_bad_nsfp() {
        let p = MockDjpProvider::default();
        let mut req = sample_request();
        req.nsfp = "TOO-SHORT".to_owned();
        let err = p.submit_faktur(&req).unwrap_err();
        assert!(matches!(err, DjpError::BadNsfp(_)));
    }

    #[test]
    fn kode_jenis_codes_match_djp_taxonomy() {
        assert_eq!(FakturKodeJenis::Standard.code(), "01");
        assert_eq!(FakturKodeJenis::GovCollector.code(), "02");
        assert_eq!(FakturKodeJenis::OtherCollector.code(), "03");
        assert_eq!(FakturKodeJenis::DppCustom.code(), "04");
        assert_eq!(FakturKodeJenis::Other.code(), "06");
        assert_eq!(FakturKodeJenis::Export.code(), "07");
        assert_eq!(FakturKodeJenis::Exempt.code(), "08");
        assert_eq!(FakturKodeJenis::Retail.code(), "09");
    }

    #[test]
    fn validate_npwp_round_trip() {
        assert!(validate_npwp(&"1".repeat(15)).is_ok());
        assert!(validate_npwp(&"1".repeat(16)).is_ok());
        assert!(validate_npwp(&"1".repeat(14)).is_err());
        assert!(validate_npwp(&"1".repeat(17)).is_err());
    }

    #[test]
    fn validate_nsfp_round_trip() {
        assert!(validate_nsfp(&"1".repeat(16)).is_ok());
        assert!(validate_nsfp(&"1".repeat(15)).is_err());
        assert!(validate_nsfp(&"1".repeat(17)).is_err());
        let mut bad = "1".repeat(15);
        bad.push('A');
        assert!(validate_nsfp(&bad).is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = DjpSubmitEnvelope {
            nomor_referensi: "DJP-000000000007".to_owned(),
            nsfp: "2".repeat(16),
            status: DjpStatus::Rejected,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            alasan: Some("NSFP sudah digunakan".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: DjpSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
