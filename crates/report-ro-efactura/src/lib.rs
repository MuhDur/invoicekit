// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Romania **RO e-Factura** (ANAF clearance) reporting adapter.
//!
//! ANAF (Agenția Națională de Administrare Fiscală) operates
//! the Romanian RO e-Factura clearance portal at
//! `api.anaf.ro`. Every Romanian B2B and B2G issuer
//! transmits invoices to RO e-Factura; ANAF validates the
//! payload against EN 16931 + the Romanian CIUS, returns an
//! **indice de încărcare** (upload index, a numeric id), and
//! within minutes follows up with a signed **mesaj** XML
//! containing the cleared invoice + ANAF's countersignature.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockEFacturaProvider`]. The live ANAF REST integration
//! lands in a follow-up `report-ro-efactura-http` crate
//! behind a feature flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the ANAF transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFacturaEnvironment {
    /// `api.anaf.ro/test` — ANAF sandbox tier.
    Sandbox,
    /// `api.anaf.ro/prod` — production.
    Production,
}

/// Which kind of document is being submitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFacturaDocumentKind {
    /// B2B / B2G invoice in UBL 2.1 + RO CIUS.
    Invoice,
    /// Credit note.
    CreditNote,
    /// Self-billing invoice (autofactura).
    SelfBilling,
}

/// What the operator passes in to
/// [`EFacturaProvider::upload`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EFacturaUploadRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: EFacturaEnvironment,
    /// Document class.
    pub kind: EFacturaDocumentKind,
    /// Issuer's Romanian CUI (Codul Unic de Înregistrare,
    /// 2–10 ASCII digits, optionally prefixed with `RO`).
    pub issuer_cui: String,
    /// Buyer's CUI; `None` for some B2C transactions.
    pub buyer_cui: Option<String>,
    /// Canonical UBL 2.1 + RO CIUS XML payload.
    pub invoice_xml: Vec<u8>,
}

/// ANAF per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EFacturaStatus {
    /// Upload accepted; awaiting validation.
    Uploaded,
    /// Validation in progress (ANAF processes in batches).
    InProgress,
    /// Cleared by ANAF; signed mesaj XML available for
    /// download.
    Cleared,
    /// Rejected with typed motivare.
    Rejected,
}

/// What [`EFacturaProvider::upload`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EFacturaUploadEnvelope {
    /// ANAF-assigned indice de încărcare (upload index).
    pub indice_incarcare: String,
    /// Latest observed status.
    pub status: EFacturaStatus,
    /// RFC-3339 UTC timestamp ANAF recorded.
    pub uploaded_at: String,
    /// Free-form motivare text when `status == Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motivare: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum EFacturaError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// CUI didn't match ANAF's 2-to-10-digit pattern (with
    /// optional `RO` prefix).
    #[error("invalid CUI: {0}")]
    BadCui(String),
    /// HTTP / TLS / DNS failure talking to ANAF.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The ANAF integration surface.
pub trait EFacturaProvider: Send + Sync {
    /// Upload one invoice to ANAF. The provider:
    ///
    /// 1. validates `issuer_cui` (+ `buyer_cui` when supplied),
    /// 2. POSTs the canonical UBL XML,
    /// 3. returns the ANAF-issued upload envelope.
    ///
    /// # Errors
    ///
    /// Returns [`EFacturaError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// ANAF-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `EFacturaStatus::Rejected` inside
    /// the envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn upload(
        &self,
        request: &EFacturaUploadRequest,
    ) -> Result<EFacturaUploadEnvelope, EFacturaError>;

    /// Poll ANAF for the latest status of a previously
    /// uploaded invoice.
    ///
    /// # Errors
    ///
    /// Returns [`EFacturaError::Transport`] when the
    /// `indice_incarcare` is unknown.
    fn poll_status(
        &self,
        environment: EFacturaEnvironment,
        indice_incarcare: &str,
    ) -> Result<EFacturaUploadEnvelope, EFacturaError>;
}

/// Deterministic mock provider.
///
/// Emits an `Uploaded` envelope per `upload` call and
/// `Cleared` per subsequent `poll_status` so cassette-replay
/// tests can exercise the full lifecycle without spinning up
/// ANAF.
pub struct MockEFacturaProvider {
    fixed_uploaded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockEFacturaProvider {
    /// Build a mock with deterministic timestamps + serial
    /// indices.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_uploaded_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_uploaded_at(uploaded_at: impl Into<String>) -> Self {
        Self {
            fixed_uploaded_at: uploaded_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockEFacturaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl EFacturaProvider for MockEFacturaProvider {
    fn upload(
        &self,
        request: &EFacturaUploadRequest,
    ) -> Result<EFacturaUploadEnvelope, EFacturaError> {
        validate_cui(&request.issuer_cui)?;
        if let Some(buyer) = &request.buyer_cui {
            validate_cui(buyer)?;
        }
        if request.invoice_xml.is_empty() {
            return Err(EFacturaError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(EFacturaUploadEnvelope {
            indice_incarcare: format!("ANAF-{serial:012}"),
            status: EFacturaStatus::Uploaded,
            uploaded_at: self.fixed_uploaded_at.clone(),
            motivare: None,
        })
    }

    fn poll_status(
        &self,
        _environment: EFacturaEnvironment,
        indice_incarcare: &str,
    ) -> Result<EFacturaUploadEnvelope, EFacturaError> {
        if indice_incarcare.is_empty() {
            return Err(EFacturaError::Transport(
                "empty indice de incarcare".to_owned(),
            ));
        }
        Ok(EFacturaUploadEnvelope {
            indice_incarcare: indice_incarcare.to_owned(),
            status: EFacturaStatus::Cleared,
            uploaded_at: self.fixed_uploaded_at.clone(),
            motivare: None,
        })
    }
}

/// Validate a Romanian CUI — 2–10 ASCII digits, optionally
/// prefixed with `RO`.
///
/// # Errors
///
/// Returns [`EFacturaError::BadCui`] on shape failure.
pub fn validate_cui(cui: &str) -> Result<(), EFacturaError> {
    let digits = cui.strip_prefix("RO").unwrap_or(cui);
    if (2..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(EFacturaError::BadCui(format!(
            "CUI must be 2-10 ASCII digits (optionally `RO`-prefixed), got {cui:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ro_efactura::crate_name(),
///     "invoicekit-report-ro-efactura"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ro-efactura"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> EFacturaUploadRequest {
        EFacturaUploadRequest {
            tenant_id: "tenant-ro-test".to_owned(),
            environment: EFacturaEnvironment::Sandbox,
            kind: EFacturaDocumentKind::Invoice,
            issuer_cui: "RO12345678".to_owned(),
            buyer_cui: Some("87654321".to_owned()),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn upload_returns_uploaded_with_indice() {
        let p = MockEFacturaProvider::default();
        let env = p.upload(&sample_request()).unwrap();
        assert_eq!(env.status, EFacturaStatus::Uploaded);
        assert!(env.indice_incarcare.starts_with("ANAF-"));
        assert_eq!(env.uploaded_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn upload_serial_increments_per_provider() {
        let p = MockEFacturaProvider::default();
        let env1 = p.upload(&sample_request()).unwrap();
        let env2 = p.upload(&sample_request()).unwrap();
        assert_ne!(env1.indice_incarcare, env2.indice_incarcare);
    }

    #[test]
    fn upload_rejects_empty_payload() {
        let p = MockEFacturaProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.upload(&req).unwrap_err();
        assert!(matches!(err, EFacturaError::BadXml(_)));
    }

    #[test]
    fn upload_rejects_bad_issuer_cui() {
        let p = MockEFacturaProvider::default();
        let mut req = sample_request();
        req.issuer_cui = "BAD".to_owned();
        let err = p.upload(&req).unwrap_err();
        assert!(matches!(err, EFacturaError::BadCui(_)));
    }

    #[test]
    fn upload_accepts_b2c_without_buyer_cui() {
        let p = MockEFacturaProvider::default();
        let mut req = sample_request();
        req.buyer_cui = None;
        let env = p.upload(&req).unwrap();
        assert_eq!(env.status, EFacturaStatus::Uploaded);
    }

    #[test]
    fn poll_status_returns_cleared() {
        let p = MockEFacturaProvider::default();
        let env = p
            .poll_status(EFacturaEnvironment::Sandbox, "ANAF-000000000001")
            .unwrap();
        assert_eq!(env.status, EFacturaStatus::Cleared);
    }

    #[test]
    fn poll_status_rejects_empty_indice() {
        let p = MockEFacturaProvider::default();
        let err = p.poll_status(EFacturaEnvironment::Sandbox, "").unwrap_err();
        assert!(matches!(err, EFacturaError::Transport(_)));
    }

    #[test]
    fn validate_cui_accepts_2_to_10_digits_with_or_without_prefix() {
        assert!(validate_cui("12345678").is_ok());
        assert!(validate_cui("RO12345678").is_ok());
        assert!(validate_cui("12").is_ok());
    }

    #[test]
    fn validate_cui_rejects_wrong_lengths() {
        assert!(validate_cui("1").is_err());
        assert!(validate_cui("12345678901").is_err());
        assert!(validate_cui("RO12345678901").is_err());
    }

    #[test]
    fn validate_cui_rejects_non_digits() {
        assert!(validate_cui("12345A").is_err());
        assert!(validate_cui("RO12345A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = EFacturaUploadEnvelope {
            indice_incarcare: "ANAF-000000000007".to_owned(),
            status: EFacturaStatus::Rejected,
            uploaded_at: "2026-01-01T00:00:00Z".to_owned(),
            motivare: Some("BR-CO-15 violation".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: EFacturaUploadEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
