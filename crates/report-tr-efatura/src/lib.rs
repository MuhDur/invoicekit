// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Turkey **e-Fatura / e-Arşiv** reporting adapter (GİB clearance).
//!
//! Turkey's Gelir İdaresi Başkanlığı (GİB, the Revenue
//! Administration) operates two parallel mandates:
//!
//! - **e-Fatura** — registered B2B issuers/receivers
//!   exchange UBL-TR invoices via the GİB clearance portal.
//! - **e-Arşiv** — issuers send to non-registered receivers
//!   (B2C / unregistered B2B) through e-Arşiv with the same
//!   wire format, reporting summaries back to GİB.
//!
//! Both flow through `efatura.gib.gov.tr` and assign a
//! 16-char alphanumeric ETTN (Evrensel Tekil Tanımlama
//! Numarası) the issuer prints on the invoice.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockEFaturaProvider`]. The live GİB SOAP integration
//! lands in a follow-up `report-tr-efatura-http` crate behind
//! a feature flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the GİB transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFaturaEnvironment {
    /// `efaturatest.izibiz.com.tr` / GİB sandbox.
    Sandbox,
    /// `efatura.gib.gov.tr` / production.
    Production,
}

/// Which Turkish mandate covers a given invoice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFaturaMandate {
    /// e-Fatura — B2B between registered issuers/receivers.
    EFatura,
    /// e-Arşiv — for non-registered receivers (B2C / B2B
    /// outside the e-Fatura mukellef list).
    EArsiv,
}

/// What the operator passes in to
/// [`EFaturaProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EFaturaSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: EFaturaEnvironment,
    /// Mandate (e-Fatura vs e-Arşiv).
    pub mandate: EFaturaMandate,
    /// Issuer's VKN (Vergi Kimlik Numarası, 10 digits) for
    /// legal entities.
    pub issuer_vkn: String,
    /// Buyer's VKN (legal entity) or TCKN (Türkiye Cumhuriyeti
    /// Kimlik Numarası, 11 digits for individuals). `None`
    /// for some e-Arşiv B2C exports.
    pub buyer_tax_id: Option<String>,
    /// Canonical UBL-TR XML payload.
    pub invoice_xml: Vec<u8>,
}

/// GİB per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFaturaStatus {
    /// Submitted; awaiting GİB clearance.
    Submitted,
    /// Cleared by GİB.
    Cleared,
    /// Receiver rejected the invoice (Red Yanıtı).
    Rejected,
    /// Cancelled (İptal) within the legal window.
    Cancelled,
}

/// What [`EFaturaProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EFaturaSubmitEnvelope {
    /// 16-char alphanumeric ETTN (Evrensel Tekil Tanımlama
    /// Numarası).
    pub ettn: String,
    /// Latest observed status.
    pub status: EFaturaStatus,
    /// RFC-3339 UTC timestamp GİB recorded.
    pub submitted_at: String,
    /// Free-form error / cancellation text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum EFaturaError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// VKN or TCKN had a wrong shape.
    #[error("invalid tax id: {0}")]
    BadTaxId(String),
    /// HTTP / TLS / DNS failure talking to GİB.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The GİB integration surface.
pub trait EFaturaProvider: Send + Sync {
    /// Submit one invoice to GİB. The provider:
    ///
    /// 1. validates the issuer VKN (+ buyer tax id when
    ///    supplied),
    /// 2. POSTs the canonical UBL-TR XML,
    /// 3. returns the GİB-issued envelope.
    ///
    /// # Errors
    ///
    /// Returns [`EFaturaError`] when validation fails before
    /// the wire or transport fails on the wire. The
    /// GİB-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `EFaturaStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(
        &self,
        request: &EFaturaSubmitRequest,
    ) -> Result<EFaturaSubmitEnvelope, EFaturaError>;

    /// Cancel a previously-submitted invoice within the
    /// legal window.
    ///
    /// # Errors
    ///
    /// Returns [`EFaturaError::Transport`] when the ETTN is
    /// unknown.
    fn cancel_invoice(
        &self,
        environment: EFaturaEnvironment,
        ettn: &str,
        reason: &str,
    ) -> Result<EFaturaSubmitEnvelope, EFaturaError>;
}

/// Deterministic mock provider.
///
/// Emits a `Cleared` envelope per `submit_invoice` call and
/// `Cancelled` per `cancel_invoice`.
pub struct MockEFaturaProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockEFaturaProvider {
    /// Build a mock with deterministic timestamps + serial
    /// ETTNs.
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

impl Default for MockEFaturaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EFaturaProvider for MockEFaturaProvider {
    fn submit_invoice(
        &self,
        request: &EFaturaSubmitRequest,
    ) -> Result<EFaturaSubmitEnvelope, EFaturaError> {
        validate_vkn(&request.issuer_vkn)?;
        if let Some(buyer) = &request.buyer_tax_id {
            validate_tax_id(buyer)?;
        }
        if request.invoice_xml.is_empty() {
            return Err(EFaturaError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        // Render a 16-char alphanumeric-looking ETTN derived
        // from the serial.
        let ettn = format!("MOCK-{serial:011x}");
        Ok(EFaturaSubmitEnvelope {
            ettn,
            status: EFaturaStatus::Cleared,
            submitted_at: self.fixed_submitted_at.clone(),
            message: None,
        })
    }

    fn cancel_invoice(
        &self,
        _environment: EFaturaEnvironment,
        ettn: &str,
        reason: &str,
    ) -> Result<EFaturaSubmitEnvelope, EFaturaError> {
        if ettn.is_empty() {
            return Err(EFaturaError::Transport("empty ETTN".to_owned()));
        }
        Ok(EFaturaSubmitEnvelope {
            ettn: ettn.to_owned(),
            status: EFaturaStatus::Cancelled,
            submitted_at: self.fixed_submitted_at.clone(),
            message: Some(reason.to_owned()),
        })
    }
}

/// Validate a Turkish VKN — exactly 10 ASCII digits.
///
/// # Errors
///
/// Returns [`EFaturaError::BadTaxId`] on shape failure.
pub fn validate_vkn(vkn: &str) -> Result<(), EFaturaError> {
    if vkn.len() == 10 && vkn.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(EFaturaError::BadTaxId(format!(
            "VKN must be 10 ASCII digits, got {vkn:?}"
        )))
    }
}

/// Validate a Turkish tax id — VKN (10 digits) or TCKN (11
/// digits).
///
/// # Errors
///
/// Returns [`EFaturaError::BadTaxId`] on shape failure.
pub fn validate_tax_id(tax_id: &str) -> Result<(), EFaturaError> {
    if matches!(tax_id.len(), 10 | 11) && tax_id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(EFaturaError::BadTaxId(format!(
            "tax id must be 10-digit VKN or 11-digit TCKN, got {tax_id:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_tr_efatura::crate_name(),
///     "invoicekit-report-tr-efatura"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-tr-efatura"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> EFaturaSubmitRequest {
        EFaturaSubmitRequest {
            tenant_id: "tenant-tr-test".to_owned(),
            environment: EFaturaEnvironment::Sandbox,
            mandate: EFaturaMandate::EFatura,
            issuer_vkn: "1234567890".to_owned(),
            buyer_tax_id: Some("0987654321".to_owned()),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_cleared_with_ettn() {
        let p = MockEFaturaProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, EFaturaStatus::Cleared);
        assert!(env.ettn.starts_with("MOCK-"));
    }

    #[test]
    fn submit_invoice_accepts_buyer_tckn() {
        let p = MockEFaturaProvider::default();
        let mut req = sample_request();
        req.buyer_tax_id = Some("12345678901".to_owned());
        let env = p.submit_invoice(&req).unwrap();
        assert_eq!(env.status, EFaturaStatus::Cleared);
    }

    #[test]
    fn submit_invoice_serial_increments_per_provider() {
        let p = MockEFaturaProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.ettn, env2.ettn);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockEFaturaProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EFaturaError::BadXml(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_vkn() {
        let p = MockEFaturaProvider::default();
        let mut req = sample_request();
        req.issuer_vkn = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EFaturaError::BadTaxId(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_buyer_tax_id() {
        let p = MockEFaturaProvider::default();
        let mut req = sample_request();
        req.buyer_tax_id = Some("not-digits".to_owned());
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, EFaturaError::BadTaxId(_)));
    }

    #[test]
    fn submit_invoice_accepts_b2c_without_buyer_tax_id() {
        let p = MockEFaturaProvider::default();
        let mut req = sample_request();
        req.mandate = EFaturaMandate::EArsiv;
        req.buyer_tax_id = None;
        let env = p.submit_invoice(&req).unwrap();
        assert_eq!(env.status, EFaturaStatus::Cleared);
    }

    #[test]
    fn cancel_invoice_returns_cancelled() {
        let p = MockEFaturaProvider::default();
        let env = p
            .cancel_invoice(
                EFaturaEnvironment::Sandbox,
                "MOCK-00000000007",
                "buyer dispute",
            )
            .unwrap();
        assert_eq!(env.status, EFaturaStatus::Cancelled);
        assert_eq!(env.message.as_deref(), Some("buyer dispute"));
    }

    #[test]
    fn cancel_invoice_rejects_empty_ettn() {
        let p = MockEFaturaProvider::default();
        let err = p
            .cancel_invoice(EFaturaEnvironment::Sandbox, "", "x")
            .unwrap_err();
        assert!(matches!(err, EFaturaError::Transport(_)));
    }

    #[test]
    fn validate_vkn_round_trip() {
        assert!(validate_vkn("1234567890").is_ok());
        assert!(validate_vkn("123456789").is_err());
        assert!(validate_vkn("12345678901").is_err());
        assert!(validate_vkn("12345A7890").is_err());
    }

    #[test]
    fn validate_tax_id_accepts_vkn_or_tckn() {
        assert!(validate_tax_id("1234567890").is_ok());
        assert!(validate_tax_id("12345678901").is_ok());
        assert!(validate_tax_id("123456789").is_err());
        assert!(validate_tax_id("123456789012").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = EFaturaSubmitEnvelope {
            ettn: "MOCK-00000000007".to_owned(),
            status: EFaturaStatus::Rejected,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            message: Some("buyer dispute".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EFaturaSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
