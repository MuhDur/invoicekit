// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Colombia **DIAN** e-invoicing clearance adapter.
//!
//! Colombia's Dirección de Impuestos y Aduanas Nacionales
//! (DIAN) operates the country's e-invoicing clearance. Every
//! Colombian B2B / B2G issuer signs a typed UBL 2.1 + DIAN
//! CIUS payload, computes a **CUFE** (Código Único de
//! Facturación Electrónica — a 96-char SHA-384 hex over the
//! invoice's canonical fields), and submits to DIAN; the
//! authority validates and returns an Aceptado / Rechazado
//! verdict plus a track id the engine reconciles against.
//!
//! Document classes mirror DIAN's `tipo de operación`
//! taxonomy: standard invoices, credit notes, debit notes,
//! support documents (documento soporte), payroll
//! (nómina electrónica), etc.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockDianProvider`]. The live DIAN SOAP integration
//! lands in a follow-up `report-co-dian-http` crate behind a
//! feature flag.

#![allow(clippy::doc_markdown)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the DIAN transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DianEnvironment {
    /// `vpfe-hab.dian.gov.co` / habilitación (sandbox).
    Habilitacion,
    /// `vpfe.dian.gov.co` / production.
    Produccion,
}

/// Which DIAN document class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DianDocumentKind {
    /// Factura electrónica de venta (standard sales invoice).
    FacturaVenta,
    /// Factura electrónica de exportación.
    FacturaExportacion,
    /// Nota crédito.
    NotaCredito,
    /// Nota débito.
    NotaDebito,
    /// Documento soporte (support document for
    /// non-obligated suppliers).
    DocumentoSoporte,
    /// Nómina electrónica (payroll).
    NominaElectronica,
}

/// What the operator passes in to
/// [`DianProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DianSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: DianEnvironment,
    /// Document class.
    pub kind: DianDocumentKind,
    /// Issuer NIT (Número de Identificación Tributaria,
    /// 9-10 ASCII digits, optionally with `-X` check digit).
    pub issuer_nit: String,
    /// Buyer NIT; `None` for B2C transactions that omit it.
    pub buyer_nit: Option<String>,
    /// Canonical signed UBL 2.1 + DIAN CIUS XML payload.
    pub invoice_xml: Vec<u8>,
}

/// DIAN per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DianStatus {
    /// Procesando — submission accepted; awaiting validation.
    Procesando,
    /// Aceptado — DIAN validation passed.
    Aceptado,
    /// Rechazado — DIAN validation rejected the payload.
    Rechazado,
    /// AceptadoConObservaciones — passed with warnings.
    AceptadoConObservaciones,
}

/// What [`DianProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DianSubmitEnvelope {
    /// 96-char hex CUFE (Código Único de Facturación
    /// Electrónica). Derived from a SHA-384 over the
    /// invoice's canonical fields.
    pub cufe: String,
    /// DIAN-assigned track id for async reconciliation.
    pub track_id: String,
    /// Latest observed status.
    pub status: DianStatus,
    /// RFC-3339 UTC timestamp DIAN recorded.
    pub submitted_at: String,
    /// Free-form error text when `status` is `Rechazado` or
    /// `AceptadoConObservaciones`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum DianError {
    /// Invoice XML failed shape validation before the wire.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// NIT didn't match the 9-10 digit shape.
    #[error("invalid NIT: {0}")]
    BadNit(String),
    /// HTTP / TLS / DNS failure talking to DIAN.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The DIAN integration surface.
pub trait DianProvider: Send + Sync {
    /// Submit one invoice to DIAN. The provider:
    ///
    /// 1. validates `issuer_nit` (+ `buyer_nit` when supplied),
    /// 2. POSTs the signed UBL XML,
    /// 3. returns the DIAN-issued envelope.
    ///
    /// # Errors
    ///
    /// Returns [`DianError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// DIAN-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `DianStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(&self, request: &DianSubmitRequest) -> Result<DianSubmitEnvelope, DianError>;

    /// Poll DIAN for the latest status of a previously
    /// submitted invoice.
    ///
    /// # Errors
    ///
    /// Returns [`DianError::Transport`] when the track id is
    /// unknown.
    fn query_track_id(
        &self,
        environment: DianEnvironment,
        track_id: &str,
    ) -> Result<DianSubmitEnvelope, DianError>;
}

/// Deterministic mock provider.
pub struct MockDianProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockDianProvider {
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

impl Default for MockDianProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DianProvider for MockDianProvider {
    fn submit_invoice(&self, request: &DianSubmitRequest) -> Result<DianSubmitEnvelope, DianError> {
        validate_nit(&request.issuer_nit)?;
        if let Some(buyer) = &request.buyer_nit {
            validate_nit(buyer)?;
        }
        if request.invoice_xml.is_empty() {
            return Err(DianError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        // Mock 96-char CUFE.
        let mut cufe = String::with_capacity(96);
        let prefix = format!("{serial:0>16x}");
        cufe.push_str(&prefix);
        for byte in request.invoice_xml.iter().take(24) {
            let _ = write!(cufe, "{byte:02x}");
        }
        while cufe.len() < 96 {
            cufe.push('0');
        }
        cufe.truncate(96);
        Ok(DianSubmitEnvelope {
            cufe,
            track_id: format!("DIAN-{serial:012}"),
            status: DianStatus::Procesando,
            submitted_at: self.fixed_submitted_at.clone(),
            message: None,
        })
    }

    fn query_track_id(
        &self,
        _environment: DianEnvironment,
        track_id: &str,
    ) -> Result<DianSubmitEnvelope, DianError> {
        if track_id.is_empty() {
            return Err(DianError::Transport("empty track id".to_owned()));
        }
        Ok(DianSubmitEnvelope {
            cufe: "0".repeat(96),
            track_id: track_id.to_owned(),
            status: DianStatus::Aceptado,
            submitted_at: self.fixed_submitted_at.clone(),
            message: None,
        })
    }
}

/// Validate a Colombian NIT — 9-10 ASCII digits, optionally
/// hyphenated with a 1-digit check (`123456789-0`).
///
/// # Errors
///
/// Returns [`DianError::BadNit`] on shape failure.
pub fn validate_nit(nit: &str) -> Result<(), DianError> {
    let collapsed: String = nit.chars().filter(|c| *c != '-').collect();
    let len_ok = (9..=11).contains(&collapsed.len());
    let digits_ok = collapsed.bytes().all(|b| b.is_ascii_digit());
    if len_ok && digits_ok {
        Ok(())
    } else {
        Err(DianError::BadNit(format!(
            "NIT must be 9-11 digits (optionally hyphenated with check digit), got {nit:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_co_dian::crate_name(),
///     "invoicekit-report-co-dian"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-co-dian"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DianSubmitRequest {
        DianSubmitRequest {
            tenant_id: "tenant-co-test".to_owned(),
            environment: DianEnvironment::Habilitacion,
            kind: DianDocumentKind::FacturaVenta,
            issuer_nit: "900123456-7".to_owned(),
            buyer_nit: Some("800987654".to_owned()),
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_procesando_with_cufe() {
        let p = MockDianProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, DianStatus::Procesando);
        assert_eq!(env.cufe.len(), 96);
        assert!(env.track_id.starts_with("DIAN-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockDianProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.track_id, env2.track_id);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockDianProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, DianError::BadXml(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_issuer_nit() {
        let p = MockDianProvider::default();
        let mut req = sample_request();
        req.issuer_nit = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, DianError::BadNit(_)));
    }

    #[test]
    fn submit_invoice_accepts_b2c_without_buyer_nit() {
        let p = MockDianProvider::default();
        let mut req = sample_request();
        req.buyer_nit = None;
        let env = p.submit_invoice(&req).unwrap();
        assert_eq!(env.status, DianStatus::Procesando);
    }

    #[test]
    fn query_track_id_returns_aceptado() {
        let p = MockDianProvider::default();
        let env = p
            .query_track_id(DianEnvironment::Habilitacion, "DIAN-000000000001")
            .unwrap();
        assert_eq!(env.status, DianStatus::Aceptado);
    }

    #[test]
    fn query_track_id_rejects_empty() {
        let p = MockDianProvider::default();
        let err = p
            .query_track_id(DianEnvironment::Habilitacion, "")
            .unwrap_err();
        assert!(matches!(err, DianError::Transport(_)));
    }

    #[test]
    fn validate_nit_round_trip() {
        assert!(validate_nit("900123456").is_ok());
        assert!(validate_nit("900123456-7").is_ok());
        assert!(validate_nit("900123456789").is_err());
        assert!(validate_nit("12345").is_err());
        assert!(validate_nit("90012345A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = DianSubmitEnvelope {
            cufe: "a".repeat(96),
            track_id: "DIAN-000000000007".to_owned(),
            status: DianStatus::Rechazado,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            message: Some("RES_89_001".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: DianSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
