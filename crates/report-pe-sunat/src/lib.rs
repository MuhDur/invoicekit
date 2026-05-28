// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Peru **SUNAT** e-invoicing (SEE — Sistema de Emisión Electrónica).
//!
//! Peru's Superintendencia Nacional de Aduanas y de
//! Administración Tributaria (SUNAT) runs the SEE clearance
//! regime. Every Peruvian B2B issuer signs a typed UBL 2.1
//! invoice, submits to SUNAT via SOAP, and receives a CDR
//! (Constancia de Recepción) ZIP carrying the per-invoice
//! `responseCode` (0 accepted, 2000–3999 rejected,
//! 4000–4999 warnings).
//!
//! Document codes mirror SUNAT's catálogo `06`:
//! - 01 Factura
//! - 03 Boleta de Venta
//! - 07 Nota de Crédito
//! - 08 Nota de Débito
//! - 09 Guía de Remisión Remitente
//!
//! This crate ships the typed surface and a deterministic
//! [`MockSunatProvider`]. The live SUNAT SOAP integration
//! lands in a follow-up `report-pe-sunat-http` crate behind a
//! feature flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the SUNAT transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SunatEnvironment {
    /// `e-beta.sunat.gob.pe` / SUNAT beta (sandbox).
    Beta,
    /// `e-factura.sunat.gob.pe` / production.
    Produccion,
}

/// SUNAT document class (catálogo 06).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SunatDocumentKind {
    /// 01 Factura.
    Factura,
    /// 03 Boleta de Venta.
    Boleta,
    /// 07 Nota de Crédito.
    NotaCredito,
    /// 08 Nota de Débito.
    NotaDebito,
    /// 09 Guía de Remisión Remitente.
    GuiaRemision,
}

impl SunatDocumentKind {
    /// SUNAT catálogo 06 code for this class.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Factura => "01",
            Self::Boleta => "03",
            Self::NotaCredito => "07",
            Self::NotaDebito => "08",
            Self::GuiaRemision => "09",
        }
    }
}

/// What the operator passes in to
/// [`SunatProvider::submit_document`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SunatSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SunatEnvironment,
    /// Document class.
    pub kind: SunatDocumentKind,
    /// Issuer RUC (Registro Único de Contribuyentes,
    /// 11 ASCII digits).
    pub issuer_ruc: String,
    /// Document series + correlative (e.g. `F001-00012345`).
    pub document_id: String,
    /// Canonical signed UBL 2.1 XML payload.
    pub invoice_xml: Vec<u8>,
}

/// SUNAT per-invoice verdict, mapped from CDR `responseCode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SunatStatus {
    /// 0 — Aceptado.
    Aceptado,
    /// 4000-4999 — Aceptado con observaciones.
    AceptadoConObservaciones,
    /// 2000-3999 — Rechazado.
    Rechazado,
    /// SUNAT didn't return a CDR (transport error /
    /// timeout). Engine retries.
    SinCdr,
}

/// What [`SunatProvider::submit_document`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SunatSubmitEnvelope {
    /// CDR ticket / response code (string for forward-compat).
    pub response_code: String,
    /// Latest observed status.
    pub status: SunatStatus,
    /// RFC-3339 UTC timestamp SUNAT recorded.
    pub submitted_at: String,
    /// CDR `description` text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum SunatError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// RUC didn't match the 11-digit shape.
    #[error("invalid RUC: {0}")]
    BadRuc(String),
    /// Document id didn't match `SSSS-NNNNNNNN` (series +
    /// correlative).
    #[error("invalid document id: {0}")]
    BadDocumentId(String),
    /// HTTP / TLS / DNS / SOAP failure talking to SUNAT.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The SUNAT integration surface.
pub trait SunatProvider: Send + Sync {
    /// Submit one document to SUNAT. The provider:
    ///
    /// 1. validates `issuer_ruc` shape,
    /// 2. validates `document_id` shape,
    /// 3. POSTs the signed UBL XML,
    /// 4. returns the CDR envelope.
    ///
    /// # Errors
    ///
    /// Returns [`SunatError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// SUNAT-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `SunatStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_document(
        &self,
        request: &SunatSubmitRequest,
    ) -> Result<SunatSubmitEnvelope, SunatError>;
}

/// Deterministic mock provider.
pub struct MockSunatProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockSunatProvider {
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
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockSunatProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SunatProvider for MockSunatProvider {
    fn submit_document(
        &self,
        request: &SunatSubmitRequest,
    ) -> Result<SunatSubmitEnvelope, SunatError> {
        validate_ruc(&request.issuer_ruc)?;
        validate_document_id(&request.document_id)?;
        if request.invoice_xml.is_empty() {
            return Err(SunatError::BadXml("payload is empty".to_owned()));
        }
        let _serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(SunatSubmitEnvelope {
            response_code: "0".to_owned(),
            status: SunatStatus::Aceptado,
            submitted_at: self.fixed_submitted_at.clone(),
            description: None,
        })
    }
}

/// Validate a Peruvian RUC — exactly 11 ASCII digits.
///
/// # Errors
///
/// Returns [`SunatError::BadRuc`] on shape failure.
pub fn validate_ruc(ruc: &str) -> Result<(), SunatError> {
    if ruc.len() == 11 && ruc.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SunatError::BadRuc(format!(
            "RUC must be 11 ASCII digits, got {ruc:?}"
        )))
    }
}

/// Validate a SUNAT document id — `SSSS-NNNNNNNN` shape
/// (4-char series, dash, 1-8 digit correlative).
///
/// # Errors
///
/// Returns [`SunatError::BadDocumentId`] on shape failure.
pub fn validate_document_id(doc_id: &str) -> Result<(), SunatError> {
    if let Some((series, correlative)) = doc_id.split_once('-') {
        let series_ok = series.len() == 4 && series.bytes().all(|b| b.is_ascii_alphanumeric());
        let corr_ok =
            (1..=8).contains(&correlative.len()) && correlative.bytes().all(|b| b.is_ascii_digit());
        if series_ok && corr_ok {
            return Ok(());
        }
    }
    Err(SunatError::BadDocumentId(format!(
        "document id must be SSSS-NNNNNNNN, got {doc_id:?}"
    )))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_pe_sunat::crate_name(),
///     "invoicekit-report-pe-sunat"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-pe-sunat"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SunatSubmitRequest {
        SunatSubmitRequest {
            tenant_id: "tenant-pe-test".to_owned(),
            environment: SunatEnvironment::Beta,
            kind: SunatDocumentKind::Factura,
            issuer_ruc: "20123456789".to_owned(),
            document_id: "F001-00012345".to_owned(),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_document_returns_aceptado_with_response_code_zero() {
        let p = MockSunatProvider::default();
        let env = p.submit_document(&sample_request()).unwrap();
        assert_eq!(env.status, SunatStatus::Aceptado);
        assert_eq!(env.response_code, "0");
    }

    #[test]
    fn submit_document_rejects_empty_payload() {
        let p = MockSunatProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit_document(&req).unwrap_err();
        assert!(matches!(err, SunatError::BadXml(_)));
    }

    #[test]
    fn submit_document_rejects_bad_ruc() {
        let p = MockSunatProvider::default();
        let mut req = sample_request();
        req.issuer_ruc = "BAD".to_owned();
        let err = p.submit_document(&req).unwrap_err();
        assert!(matches!(err, SunatError::BadRuc(_)));
    }

    #[test]
    fn submit_document_rejects_bad_document_id() {
        let p = MockSunatProvider::default();
        let mut req = sample_request();
        req.document_id = "NO-PREFIX".to_owned();
        let err = p.submit_document(&req).unwrap_err();
        assert!(matches!(err, SunatError::BadDocumentId(_)));
    }

    #[test]
    fn document_kind_codes_match_sunat_taxonomy() {
        assert_eq!(SunatDocumentKind::Factura.code(), "01");
        assert_eq!(SunatDocumentKind::Boleta.code(), "03");
        assert_eq!(SunatDocumentKind::NotaCredito.code(), "07");
        assert_eq!(SunatDocumentKind::NotaDebito.code(), "08");
        assert_eq!(SunatDocumentKind::GuiaRemision.code(), "09");
    }

    #[test]
    fn validate_ruc_round_trip() {
        assert!(validate_ruc("20123456789").is_ok());
        assert!(validate_ruc("2012345678").is_err());
        assert!(validate_ruc("201234567890").is_err());
        assert!(validate_ruc("2012345678A").is_err());
    }

    #[test]
    fn validate_document_id_round_trip() {
        assert!(validate_document_id("F001-00012345").is_ok());
        assert!(validate_document_id("B001-1").is_ok());
        assert!(validate_document_id("F001-12345678").is_ok());
        assert!(validate_document_id("F001-123456789").is_err()); // 9-digit correlative
        assert!(validate_document_id("F01-00012345").is_err()); // 3-char series
        assert!(validate_document_id("NOSEP00012345").is_err()); // no dash
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = SunatSubmitEnvelope {
            response_code: "2335".to_owned(),
            status: SunatStatus::Rechazado,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            description: Some("RUC del receptor no existe".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SunatSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
