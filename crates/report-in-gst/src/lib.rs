// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! India **GST e-invoicing** via the Invoice Registration Portal (IRP).
//!
//! Under the Goods and Services Tax mandate every notified
//! Indian taxpayer (turnover above the current threshold —
//! ₹5 crore at time of writing) issues B2B invoices through
//! an **IRP** (Invoice Registration Portal). The IRP
//! validates the invoice payload, assigns an **IRN** (Invoice
//! Reference Number — a 64-char SHA-256 hex of the invoice's
//! canonical fields), signs a JWS over the invoice, and
//! returns a base-64 PNG / TLV string for the **signed QR**
//! the issuer prints on the invoice.
//!
//! Multiple IRPs exist (NIC IRP1, NIC IRP2, IRIS IRP, EY
//! GSP, Cygnet GSP, etc.). The shape of every request +
//! response is identical — this crate captures it as a single
//! [`IrpProvider`] trait so operator code never re-derives
//! the IRP wire shape.
//!
//! Mock `MockIrpProvider` ships for tests + cassette-replay.
//! Real backends land in feature-flagged
//! `report-in-gst-http` / `report-in-gst-nic` follow-ups.

#![allow(clippy::doc_markdown)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which IRP backend the engine talks to. Strings stay
/// opaque so new IRPs can plug in without bumping this enum.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpBackend {
    /// Government IRP1 (`einvoice1.gst.gov.in`).
    Nic1,
    /// Government IRP2 (`einvoice2.gst.gov.in`).
    Nic2,
    /// Any private GSP / IRP. The string is the operator-side
    /// vendor label so cassettes can pin a recording to one
    /// specific IRP.
    Gsp(String),
}

/// Environment selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpEnvironment {
    /// `einv-apisandbox.nic.in` / GSP sandbox tier.
    Sandbox,
    /// Production.
    Production,
}

/// IRP per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpStatus {
    /// Successfully registered; IRN + signed QR returned.
    Accepted,
    /// Duplicate IRN — IRP returns the existing IRN; engine
    /// should reconcile against it instead of issuing fresh.
    Duplicate,
    /// IRP refused the payload. Fix + resubmit.
    Rejected,
}

/// What the operator passes in to
/// [`IrpProvider::register_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IrpRegisterRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: IrpEnvironment,
    /// Which IRP backend handles this request.
    pub backend: IrpBackend,
    /// Issuer's 15-character GSTIN (Goods and Services Tax
    /// Identification Number).
    pub issuer_gstin: String,
    /// Buyer's GSTIN; `None` for export / B2C transactions
    /// that don't carry a buyer GSTIN.
    pub buyer_gstin: Option<String>,
    /// Canonical IRP JSON payload (Schema-1.1 at time of
    /// writing). The provider does NOT pre-sign — the IRP
    /// signs on its side.
    pub invoice_json: Vec<u8>,
}

/// What [`IrpProvider::register_invoice`] returns when the
/// IRP has registered the invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IrpRegisterEnvelope {
    /// IRP verdict.
    pub status: IrpStatus,
    /// 64-char IRN (Invoice Reference Number). `None` only
    /// for `Rejected` status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irn: Option<String>,
    /// IRP acknowledgement number (numeric string). Engines
    /// quote this in support tickets with the IRP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_no: Option<String>,
    /// RFC-3339 UTC timestamp the IRP recorded.
    pub ack_dt: String,
    /// Base-64 PNG of the signed QR (the engine writes this
    /// straight into the printed invoice).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_qr_code: Option<String>,
    /// JWS the IRP signed the invoice with. Engines persist
    /// this for offline verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_invoice_jws: Option<String>,
    /// Free-form error text when `status == Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum IrpError {
    /// Invoice JSON failed shape validation before the wire.
    #[error("invoice json rejected: {0}")]
    BadJson(String),
    /// Issuer / buyer GSTIN didn't match the 15-char pattern.
    #[error("invalid GSTIN: {0}")]
    BadGstin(String),
    /// HTTP / TLS / DNS failure talking to the IRP.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The IRP integration surface. Real backends satisfy this
/// trait; the mock below is what tests + cassette-replay use.
pub trait IrpProvider: Send + Sync {
    /// Register one invoice with the IRP. The provider:
    ///
    /// 1. validates `issuer_gstin` (+ `buyer_gstin` when
    ///    supplied),
    /// 2. POSTs the invoice JSON to the backend endpoint
    ///    selected by `backend` + `environment`,
    /// 3. returns the IRP-issued envelope.
    ///
    /// The IRP-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `IrpStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    ///
    /// # Errors
    ///
    /// Returns [`IrpError`] when local validation fails
    /// before the wire or transport fails on the wire.
    fn register_invoice(
        &self,
        request: &IrpRegisterRequest,
    ) -> Result<IrpRegisterEnvelope, IrpError>;
}

/// Deterministic mock provider.
///
/// Emits a synthesised 64-hex-char IRN derived from the
/// payload length + first 24 bytes so cassette-replay tests
/// stay byte-identical across runs. Returns `Duplicate` when
/// the same IRN would be produced twice — i.e. when the same
/// payload is registered twice with the same provider.
pub struct MockIrpProvider {
    fixed_ack_dt: String,
    seen_irns: std::sync::Mutex<std::collections::BTreeSet<String>>,
    next_ack: std::sync::Mutex<u64>,
}

impl MockIrpProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_ack_dt("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp (the mock
    /// emits this value verbatim in every `IrpRegisterEnvelope`).
    #[must_use]
    pub fn with_fixed_ack_dt(ack_dt: impl Into<String>) -> Self {
        Self {
            fixed_ack_dt: ack_dt.into(),
            seen_irns: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            next_ack: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockIrpProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl IrpProvider for MockIrpProvider {
    fn register_invoice(
        &self,
        request: &IrpRegisterRequest,
    ) -> Result<IrpRegisterEnvelope, IrpError> {
        validate_gstin(&request.issuer_gstin)?;
        if let Some(buyer) = &request.buyer_gstin {
            validate_gstin(buyer)?;
        }
        if request.invoice_json.is_empty() {
            return Err(IrpError::BadJson("payload is empty".to_owned()));
        }

        // Synthesise a 64-hex "IRN" so callers can dedup.
        let mut irn = String::with_capacity(64);
        let _ = write!(irn, "{:0>16x}", request.invoice_json.len() as u64);
        for byte in request.invoice_json.iter().take(24) {
            let _ = write!(irn, "{byte:02x}");
        }
        while irn.len() < 64 {
            irn.push('0');
        }
        irn.truncate(64);

        let seen = {
            let mut guard = self.seen_irns.lock().expect("seen IRN mutex poisoned");
            let already = guard.contains(&irn);
            guard.insert(irn.clone());
            already
        };
        let ack_serial = {
            let mut g = self.next_ack.lock().expect("ack mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        let ack_no = format!("ACK-{ack_serial:014}");
        Ok(IrpRegisterEnvelope {
            status: if seen {
                IrpStatus::Duplicate
            } else {
                IrpStatus::Accepted
            },
            irn: Some(irn.clone()),
            ack_no: Some(ack_no),
            ack_dt: self.fixed_ack_dt.clone(),
            signed_qr_code: Some(mock_qr_base64(&irn)),
            signed_invoice_jws: Some(mock_jws(&irn)),
            error_message: None,
        })
    }
}

fn mock_qr_base64(irn: &str) -> String {
    // The real IRP returns a base-64 PNG; the mock returns a
    // deterministic placeholder so cassettes stay
    // byte-identical.
    format!("MOCK-QR-{}", &irn[..16])
}

fn mock_jws(irn: &str) -> String {
    format!("eyJhbGciOiJSUzI1NiJ9.{}.MOCK_SIG", &irn[..32])
}

/// Validate that a GSTIN is exactly 15 ASCII alphanumeric chars.
///
/// Real shape: state code + PAN + entity number + check
/// digit. The full IRP modulo-checksum is a separate concern;
/// this helper only catches obviously-wrong shapes before the
/// wire.
///
/// # Errors
///
/// Returns [`IrpError::BadGstin`] when the input isn't 15
/// ASCII alphanumeric characters.
pub fn validate_gstin(gstin: &str) -> Result<(), IrpError> {
    if gstin.len() == 15 && gstin.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(IrpError::BadGstin(format!(
            "GSTIN must be 15 ASCII alphanumeric chars, got {gstin:?}"
        )))
    }
}

/// Validate that an HSN (Harmonised System of Nomenclature)
/// or SAC (Services Accounting Code) is 4–8 ASCII digits.
///
/// # Errors
///
/// Returns [`IrpError::BadJson`] when the shape is wrong.
pub fn validate_hsn_sac(code: &str) -> Result<(), IrpError> {
    if (4..=8).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(IrpError::BadJson(format!(
            "HSN/SAC must be 4–8 ASCII digits, got {code:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_in_gst::crate_name(),
///     "invoicekit-report-in-gst"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-in-gst"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> IrpRegisterRequest {
        IrpRegisterRequest {
            tenant_id: "tenant-in-test".to_owned(),
            environment: IrpEnvironment::Sandbox,
            backend: IrpBackend::Nic1,
            issuer_gstin: "29AAAPL2356Q1ZS".to_owned(),
            buyer_gstin: Some("27AAAPL2356Q1ZT".to_owned()),
            invoice_json: br#"{"version":"1.1"}"#.to_vec(),
        }
    }

    #[test]
    fn register_invoice_returns_accepted_with_irn() {
        let p = MockIrpProvider::default();
        let env = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, IrpStatus::Accepted);
        assert!(env.irn.as_ref().is_some_and(|s| s.len() == 64));
        assert!(env.ack_no.as_ref().is_some_and(|s| s.starts_with("ACK-")));
        assert_eq!(env.ack_dt, "2026-01-01T00:00:00Z");
        assert!(env.signed_qr_code.is_some());
        assert!(env.signed_invoice_jws.is_some());
        assert!(env.error_message.is_none());
    }

    #[test]
    fn register_invoice_detects_duplicate_on_resubmit() {
        let p = MockIrpProvider::default();
        let env1 = p.register_invoice(&sample_request()).unwrap();
        let env2 = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env1.status, IrpStatus::Accepted);
        assert_eq!(env2.status, IrpStatus::Duplicate);
        // Same IRN returned both times.
        assert_eq!(env1.irn, env2.irn);
    }

    #[test]
    fn register_invoice_rejects_empty_payload() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.invoice_json.clear();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadJson(_)));
    }

    #[test]
    fn register_invoice_rejects_bad_issuer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.issuer_gstin = "TOO-SHORT".to_owned();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadGstin(_)));
    }

    #[test]
    fn register_invoice_rejects_bad_buyer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.buyer_gstin = Some("TOO-SHORT".to_owned());
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadGstin(_)));
    }

    #[test]
    fn register_invoice_accepts_export_without_buyer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.buyer_gstin = None;
        let env = p.register_invoice(&req).unwrap();
        assert_eq!(env.status, IrpStatus::Accepted);
    }

    #[test]
    fn validate_gstin_accepts_well_formed_15_char_string() {
        assert!(validate_gstin("29AAAPL2356Q1ZS").is_ok());
    }

    #[test]
    fn validate_gstin_rejects_wrong_length() {
        assert!(validate_gstin("29AAAPL2356Q1Z").is_err());
        assert!(validate_gstin("29AAAPL2356Q1ZSS").is_err());
    }

    #[test]
    fn validate_gstin_rejects_non_alphanumeric() {
        assert!(validate_gstin("29-AAPL2356Q1ZS").is_err());
        assert!(validate_gstin("29 AAPL2356Q1ZS").is_err());
    }

    #[test]
    fn validate_hsn_sac_accepts_4_to_8_digits() {
        assert!(validate_hsn_sac("8471").is_ok());
        assert!(validate_hsn_sac("84713010").is_ok());
    }

    #[test]
    fn validate_hsn_sac_rejects_wrong_length() {
        assert!(validate_hsn_sac("847").is_err());
        assert!(validate_hsn_sac("847130100").is_err());
    }

    #[test]
    fn validate_hsn_sac_rejects_non_digits() {
        assert!(validate_hsn_sac("84A1").is_err());
    }

    #[test]
    fn backend_serde_round_trips_all_three_variants() {
        for backend in [
            IrpBackend::Nic1,
            IrpBackend::Nic2,
            IrpBackend::Gsp("iris".to_owned()),
        ] {
            let json = serde_json::to_string(&backend).unwrap();
            let parsed: IrpBackend = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, backend);
        }
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = IrpRegisterEnvelope {
            status: IrpStatus::Accepted,
            irn: Some("ab".repeat(32)),
            ack_no: Some("ACK-00000000000007".to_owned()),
            ack_dt: "2026-01-01T00:00:00Z".to_owned(),
            signed_qr_code: Some("MOCK-QR-abababab".to_owned()),
            signed_invoice_jws: Some("eyJhbGciOiJSUzI1NiJ9.x.MOCK_SIG".to_owned()),
            error_message: None,
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: IrpRegisterEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
