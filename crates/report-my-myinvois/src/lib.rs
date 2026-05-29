// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Malaysia **MyInvois** (LHDNM e-invoicing) reporting adapter.
//!
//! Malaysia's Lembaga Hasil Dalam Negeri Malaysia (LHDNM,
//! the Inland Revenue Board) operates MyInvois, the
//! near-real-time clearance portal every Malaysian B2B issuer
//! transmits invoices to. The portal validates the payload,
//! assigns a **UUID** + a 64-char content hash, then returns
//! a signed acknowledgement carrying the **Long ID** the
//! buyer uses to validate the invoice on the LHDNM portal.
//!
//! Invoice classes: standard `Invoice`, `CreditNote`,
//! `DebitNote`, `RefundNote`, `SelfBilledInvoice` (for B2C
//! imports), `SelfBilledCreditNote`, `SelfBilledDebitNote`,
//! `SelfBilledRefundNote`.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockMyInvoisProvider`]. The live LHDNM REST integration
//! lands in a follow-up `report-my-myinvois-http` crate
//! behind a feature flag.

#![allow(clippy::doc_markdown)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the LHDNM portal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyInvoisEnvironment {
    /// `preprod-api.myinvois.hasil.gov.my` / sandbox.
    Sandbox,
    /// `api.myinvois.hasil.gov.my` / production.
    Production,
}

/// MyInvois invoice class.
///
/// Mirrors the LHDNM `eInvoiceTypeCode` taxonomy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyInvoisDocumentKind {
    /// Standard B2B invoice (code `01`).
    Invoice,
    /// Credit note (code `02`).
    CreditNote,
    /// Debit note (code `03`).
    DebitNote,
    /// Refund note (code `04`).
    RefundNote,
    /// Self-billed B2C / import invoice (code `11`).
    SelfBilledInvoice,
    /// Self-billed credit note (code `12`).
    SelfBilledCreditNote,
    /// Self-billed debit note (code `13`).
    SelfBilledDebitNote,
    /// Self-billed refund note (code `14`).
    SelfBilledRefundNote,
}

impl MyInvoisDocumentKind {
    /// LHDNM `eInvoiceTypeCode` for this class.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invoice => "01",
            Self::CreditNote => "02",
            Self::DebitNote => "03",
            Self::RefundNote => "04",
            Self::SelfBilledInvoice => "11",
            Self::SelfBilledCreditNote => "12",
            Self::SelfBilledDebitNote => "13",
            Self::SelfBilledRefundNote => "14",
        }
    }
}

/// What the operator passes in to
/// [`MyInvoisProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyInvoisSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: MyInvoisEnvironment,
    /// Document class.
    pub kind: MyInvoisDocumentKind,
    /// Issuer's TIN (`C` + 10 digits, e.g. `C1234567890`).
    pub issuer_tin: String,
    /// Issuer's BRN (Business Registration Number, 12 digits).
    pub issuer_brn: String,
    /// Buyer TIN. `None` for B2C transactions that omit it.
    pub buyer_tin: Option<String>,
    /// Canonical UBL XML payload conforming to LHDNM's
    /// PEPPOL-derived schema.
    pub invoice_xml: Vec<u8>,
}

/// MyInvois per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyInvoisStatus {
    /// Submitted and accepted by LHDNM; UUID + Long ID
    /// returned.
    Submitted,
    /// Validated successfully — invoice is final and visible
    /// on the LHDNM portal.
    Valid,
    /// Buyer rejected within the 72-hour grace window.
    Cancelled,
    /// LHDNM refused (schema / business rule failure).
    Rejected,
}

/// What [`MyInvoisProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyInvoisSubmitEnvelope {
    /// MyInvois UUID assigned by LHDNM.
    pub uuid: String,
    /// 64-char canonical content hash (BLAKE3/SHA-256 hex).
    pub content_hash_hex: String,
    /// Long ID the buyer uses to validate on the public
    /// portal.
    pub long_id: String,
    /// Latest observed status.
    pub status: MyInvoisStatus,
    /// RFC-3339 UTC timestamp LHDNM recorded.
    pub submitted_at: String,
    /// Free-form rejection / cancellation reason text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum MyInvoisError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// TIN didn't match LHDNM's `C` + 10 digits pattern.
    #[error("invalid TIN: {0}")]
    BadTin(String),
    /// BRN didn't match 12 ASCII digits pattern.
    #[error("invalid BRN: {0}")]
    BadBrn(String),
    /// HTTP / TLS / DNS failure talking to LHDNM.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The MyInvois integration surface.
pub trait MyInvoisProvider: Send + Sync {
    /// Submit one invoice to LHDNM. The provider:
    ///
    /// 1. validates TIN + BRN shape,
    /// 2. POSTs the canonical UBL XML,
    /// 3. returns the LHDNM-issued envelope.
    ///
    /// # Errors
    ///
    /// Returns [`MyInvoisError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// LHDNM-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `MyInvoisStatus::Rejected` inside
    /// the envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(
        &self,
        request: &MyInvoisSubmitRequest,
    ) -> Result<MyInvoisSubmitEnvelope, MyInvoisError>;

    /// Cancel a previously-submitted invoice within the
    /// 72-hour grace window.
    ///
    /// # Errors
    ///
    /// Returns [`MyInvoisError::Transport`] when the UUID is
    /// unknown or the cancellation window has closed.
    fn cancel_invoice(
        &self,
        environment: MyInvoisEnvironment,
        uuid: &str,
        reason: &str,
    ) -> Result<MyInvoisSubmitEnvelope, MyInvoisError>;
}

/// Deterministic mock provider.
///
/// Emits a synthesised UUID derived from the payload length +
/// first 16 bytes so cassette-replay tests stay byte-identical
/// across runs. `cancel_invoice` always succeeds and returns
/// `MyInvoisStatus::Cancelled`.
pub struct MockMyInvoisProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::atomic::AtomicU64,
}

impl MockMyInvoisProvider {
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
            next_serial: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl Default for MockMyInvoisProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MyInvoisProvider for MockMyInvoisProvider {
    fn submit_invoice(
        &self,
        request: &MyInvoisSubmitRequest,
    ) -> Result<MyInvoisSubmitEnvelope, MyInvoisError> {
        validate_tin(&request.issuer_tin)?;
        validate_brn(&request.issuer_brn)?;
        if let Some(buyer) = &request.buyer_tin {
            validate_tin(buyer)?;
        }
        if request.invoice_xml.is_empty() {
            return Err(MyInvoisError::BadXml("payload is empty".to_owned()));
        }

        let serial = self
            .next_serial
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut content_hash = String::with_capacity(64);
        let _ = write!(content_hash, "{:0>16x}", request.invoice_xml.len() as u64);
        for byte in request.invoice_xml.iter().take(24) {
            let _ = write!(content_hash, "{byte:02x}");
        }
        while content_hash.len() < 64 {
            content_hash.push('0');
        }
        content_hash.truncate(64);

        Ok(MyInvoisSubmitEnvelope {
            uuid: format!("MOCK-UUID-{:0>8x}-{}", serial, &content_hash[..16]),
            content_hash_hex: content_hash,
            long_id: format!("MOCK-LONG-ID-{serial:012}"),
            status: MyInvoisStatus::Submitted,
            submitted_at: self.fixed_submitted_at.clone(),
            rejection_reason: None,
        })
    }

    fn cancel_invoice(
        &self,
        _environment: MyInvoisEnvironment,
        uuid: &str,
        reason: &str,
    ) -> Result<MyInvoisSubmitEnvelope, MyInvoisError> {
        if uuid.is_empty() {
            return Err(MyInvoisError::Transport("empty UUID".to_owned()));
        }
        Ok(MyInvoisSubmitEnvelope {
            uuid: uuid.to_owned(),
            content_hash_hex: "0".repeat(64),
            long_id: format!("CANCELLED-{uuid}"),
            status: MyInvoisStatus::Cancelled,
            submitted_at: self.fixed_submitted_at.clone(),
            rejection_reason: Some(reason.to_owned()),
        })
    }
}

/// Validate a Malaysian TIN — `C` prefix + 10 ASCII digits.
///
/// # Errors
///
/// Returns [`MyInvoisError::BadTin`] on shape failure.
pub fn validate_tin(tin: &str) -> Result<(), MyInvoisError> {
    if tin.len() == 11 && tin.starts_with('C') && tin.bytes().skip(1).all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(MyInvoisError::BadTin(format!(
            "TIN must be `C` + 10 ASCII digits, got {tin:?}"
        )))
    }
}

/// Validate a Malaysian BRN — 12 ASCII digits.
///
/// # Errors
///
/// Returns [`MyInvoisError::BadBrn`] on shape failure.
pub fn validate_brn(brn: &str) -> Result<(), MyInvoisError> {
    if brn.len() == 12 && brn.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(MyInvoisError::BadBrn(format!(
            "BRN must be 12 ASCII digits, got {brn:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_my_myinvois::crate_name(),
///     "invoicekit-report-my-myinvois"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-my-myinvois"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> MyInvoisSubmitRequest {
        MyInvoisSubmitRequest {
            tenant_id: "tenant-my-test".to_owned(),
            environment: MyInvoisEnvironment::Sandbox,
            kind: MyInvoisDocumentKind::Invoice,
            issuer_tin: "C1234567890".to_owned(),
            issuer_brn: "202301234567".to_owned(),
            buyer_tin: Some("C9876543210".to_owned()),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_submitted_with_uuid_and_long_id() {
        let p = MockMyInvoisProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, MyInvoisStatus::Submitted);
        assert!(env.uuid.starts_with("MOCK-UUID-"));
        assert!(env.long_id.starts_with("MOCK-LONG-ID-"));
        assert_eq!(env.content_hash_hex.len(), 64);
        assert_eq!(env.submitted_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn submit_invoice_serial_increments_per_provider() {
        let p = MockMyInvoisProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.uuid, env2.uuid);
        assert_ne!(env1.long_id, env2.long_id);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockMyInvoisProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, MyInvoisError::BadXml(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_issuer_tin() {
        let p = MockMyInvoisProvider::default();
        let mut req = sample_request();
        req.issuer_tin = "BADTIN".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, MyInvoisError::BadTin(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_issuer_brn() {
        let p = MockMyInvoisProvider::default();
        let mut req = sample_request();
        req.issuer_brn = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, MyInvoisError::BadBrn(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_buyer_tin() {
        let p = MockMyInvoisProvider::default();
        let mut req = sample_request();
        req.buyer_tin = Some("ALSO-BAD".to_owned());
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, MyInvoisError::BadTin(_)));
    }

    #[test]
    fn submit_invoice_accepts_b2c_without_buyer_tin() {
        let p = MockMyInvoisProvider::default();
        let mut req = sample_request();
        req.buyer_tin = None;
        let env = p.submit_invoice(&req).unwrap();
        assert_eq!(env.status, MyInvoisStatus::Submitted);
    }

    #[test]
    fn cancel_invoice_returns_cancelled_status() {
        let p = MockMyInvoisProvider::default();
        let env = p
            .cancel_invoice(
                MyInvoisEnvironment::Sandbox,
                "MOCK-UUID-00000001-abcdef0123456789",
                "buyer requested cancellation",
            )
            .unwrap();
        assert_eq!(env.status, MyInvoisStatus::Cancelled);
        assert_eq!(
            env.rejection_reason.as_deref(),
            Some("buyer requested cancellation")
        );
    }

    #[test]
    fn cancel_invoice_rejects_empty_uuid() {
        let p = MockMyInvoisProvider::default();
        let err = p
            .cancel_invoice(MyInvoisEnvironment::Sandbox, "", "x")
            .unwrap_err();
        assert!(matches!(err, MyInvoisError::Transport(_)));
    }

    #[test]
    fn document_kind_codes_match_lhdnm_taxonomy() {
        assert_eq!(MyInvoisDocumentKind::Invoice.code(), "01");
        assert_eq!(MyInvoisDocumentKind::CreditNote.code(), "02");
        assert_eq!(MyInvoisDocumentKind::DebitNote.code(), "03");
        assert_eq!(MyInvoisDocumentKind::RefundNote.code(), "04");
        assert_eq!(MyInvoisDocumentKind::SelfBilledInvoice.code(), "11");
        assert_eq!(MyInvoisDocumentKind::SelfBilledCreditNote.code(), "12");
        assert_eq!(MyInvoisDocumentKind::SelfBilledDebitNote.code(), "13");
        assert_eq!(MyInvoisDocumentKind::SelfBilledRefundNote.code(), "14");
    }

    #[test]
    fn validate_tin_round_trip() {
        assert!(validate_tin("C1234567890").is_ok());
        assert!(validate_tin("D1234567890").is_err());
        assert!(validate_tin("C123456789").is_err());
        assert!(validate_tin("C12345678901").is_err());
    }

    #[test]
    fn validate_brn_round_trip() {
        assert!(validate_brn("202301234567").is_ok());
        assert!(validate_brn("20230123456").is_err());
        assert!(validate_brn("2023012345678").is_err());
        assert!(validate_brn("20230123456A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = MyInvoisSubmitEnvelope {
            uuid: "MOCK-UUID-00000001-abcdef0123456789".to_owned(),
            content_hash_hex: "a".repeat(64),
            long_id: "MOCK-LONG-ID-000000000007".to_owned(),
            status: MyInvoisStatus::Rejected,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            rejection_reason: Some("BR-CO-15 violation".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: MyInvoisSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
