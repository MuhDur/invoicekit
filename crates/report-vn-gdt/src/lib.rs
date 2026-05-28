// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Vietnam **GDT** e-Invoice clearance adapter.
//!
//! The General Department of Taxation (GDT) runs Vietnam's
//! e-Invoice clearance regime through the official portal at
//! `hoadondientu.gdt.gov.vn`. Issuers sign a typed XML
//! payload with a GDT-registered digital certificate, submit
//! to the GDT, and receive a `mã CQT` (tax authority code)
//! confirming clearance, which they print on the invoice.
//!
//! Ships typed surface + [`MockGdtProvider`]; the live GDT
//! REST integration lands in a follow-up
//! `report-vn-gdt-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the GDT transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GdtEnvironment {
    /// `hoadondientu-test.gdt.gov.vn` / sandbox.
    Sandbox,
    /// `hoadondientu.gdt.gov.vn` / production.
    Production,
}

/// What the operator passes in to
/// [`GdtProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GdtSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: GdtEnvironment,
    /// Issuer mã số thuế (MST — 10 or 13 ASCII digits,
    /// 13 when including the branch suffix).
    pub issuer_mst: String,
    /// Canonical signed XML payload.
    pub invoice_xml: Vec<u8>,
}

/// GDT per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GdtStatus {
    /// `Đã cấp mã` — cleared with code.
    Cleared,
    /// `Bị từ chối` — rejected.
    Rejected,
}

/// What [`GdtProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GdtSubmitEnvelope {
    /// GDT-assigned `mã CQT` (tax authority code).
    pub ma_cqt: String,
    /// Latest observed status.
    pub status: GdtStatus,
    /// RFC-3339 UTC timestamp GDT recorded.
    pub recorded_at: String,
    /// Free-form reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum GdtError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// MST didn't match the 10 / 13-digit shape.
    #[error("invalid MST: {0}")]
    BadMst(String),
    /// HTTP / TLS / DNS failure talking to GDT.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The GDT integration surface.
pub trait GdtProvider: Send + Sync {
    /// Submit one invoice to GDT.
    ///
    /// # Errors
    ///
    /// Returns [`GdtError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// GDT-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `GdtStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(&self, request: &GdtSubmitRequest) -> Result<GdtSubmitEnvelope, GdtError>;
}

/// Deterministic mock provider.
pub struct MockGdtProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockGdtProvider {
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

impl Default for MockGdtProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GdtProvider for MockGdtProvider {
    fn submit_invoice(&self, request: &GdtSubmitRequest) -> Result<GdtSubmitEnvelope, GdtError> {
        validate_mst(&request.issuer_mst)?;
        if request.invoice_xml.is_empty() {
            return Err(GdtError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(GdtSubmitEnvelope {
            ma_cqt: format!("VN-{serial:012}"),
            status: GdtStatus::Cleared,
            recorded_at: self.fixed_recorded_at.clone(),
            message: None,
        })
    }
}

/// Validate a Vietnamese MST — 10 or 13 ASCII digits
/// (13 when including the 3-digit branch suffix).
///
/// # Errors
///
/// Returns [`GdtError::BadMst`] on shape failure.
pub fn validate_mst(mst: &str) -> Result<(), GdtError> {
    if matches!(mst.len(), 10 | 13) && mst.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(GdtError::BadMst(format!(
            "MST must be 10 or 13 ASCII digits, got {mst:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_vn_gdt::crate_name(),
///     "invoicekit-report-vn-gdt"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-vn-gdt"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> GdtSubmitRequest {
        GdtSubmitRequest {
            tenant_id: "tenant-vn-test".to_owned(),
            environment: GdtEnvironment::Sandbox,
            issuer_mst: "0123456789".to_owned(),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_cleared() {
        let p = MockGdtProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, GdtStatus::Cleared);
        assert!(env.ma_cqt.starts_with("VN-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockGdtProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.ma_cqt, env2.ma_cqt);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockGdtProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, GdtError::BadXml(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_mst() {
        let p = MockGdtProvider::default();
        let mut req = sample_request();
        req.issuer_mst = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, GdtError::BadMst(_)));
    }

    #[test]
    fn validate_mst_round_trip() {
        assert!(validate_mst("0123456789").is_ok());
        assert!(validate_mst("0123456789001").is_ok());
        assert!(validate_mst("012345").is_err());
        assert!(validate_mst("0123456789A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = GdtSubmitEnvelope {
            ma_cqt: "VN-000000000007".to_owned(),
            status: GdtStatus::Rejected,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            message: Some("MST không hợp lệ".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: GdtSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
