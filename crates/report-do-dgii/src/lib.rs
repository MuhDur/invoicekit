// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Dominican Republic **DGII** e-CF (e-Comprobante Fiscal) adapter.
//!
//! The Dirección General de Impuestos Internos (DGII) runs
//! the Dominican Republic's e-invoicing clearance. Issuers
//! sign XML with a DGII-issued certificate, compute a 13-19
//! character **e-NCF** (Número de Comprobante Fiscal
//! electrónico), and submit to DGII via REST. DGII returns
//! `Aceptado` / `Rechazado` plus a TrackId for async
//! reconciliation.
//!
//! e-CF types mirror DGII's catálogo: 31 Factura de Crédito
//! Fiscal Electrónica, 32 Factura de Consumo Electrónica,
//! 33 Nota de Débito Electrónica, 34 Nota de Crédito
//! Electrónica, 41 Compras Electrónica, 43 Gastos Menores
//! Electrónica, 44 Regímenes Especiales Electrónica, 45
//! Gubernamental Electrónica, 46 Exportaciones Electrónica,
//! 47 Pagos al Exterior Electrónica.
//!
//! Ships typed surface + [`MockDgiiProvider`]; the live REST
//! integration lands in a follow-up `report-do-dgii-http`
//! crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the DGII transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DgiiEnvironment {
    /// `ecf.dgii.gov.do/testecf` / TestECF sandbox.
    Sandbox,
    /// `ecf.dgii.gov.do/ecf` / production.
    Produccion,
}

/// e-CF document class (catálogo DGII).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DgiiDocumentKind {
    /// 31 Factura de Crédito Fiscal Electrónica.
    FacturaCreditoFiscal,
    /// 32 Factura de Consumo Electrónica.
    FacturaConsumo,
    /// 33 Nota de Débito Electrónica.
    NotaDebito,
    /// 34 Nota de Crédito Electrónica.
    NotaCredito,
    /// 41 Compras Electrónica.
    Compras,
    /// 43 Gastos Menores Electrónica.
    GastosMenores,
    /// 44 Regímenes Especiales Electrónica.
    RegimenesEspeciales,
    /// 45 Gubernamental Electrónica.
    Gubernamental,
    /// 46 Exportaciones Electrónica.
    Exportaciones,
    /// 47 Pagos al Exterior Electrónica.
    PagosExterior,
}

impl DgiiDocumentKind {
    /// DGII catálogo code for this class.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::FacturaCreditoFiscal => 31,
            Self::FacturaConsumo => 32,
            Self::NotaDebito => 33,
            Self::NotaCredito => 34,
            Self::Compras => 41,
            Self::GastosMenores => 43,
            Self::RegimenesEspeciales => 44,
            Self::Gubernamental => 45,
            Self::Exportaciones => 46,
            Self::PagosExterior => 47,
        }
    }
}

/// What the operator passes in to
/// [`DgiiProvider::submit_ecf`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DgiiSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: DgiiEnvironment,
    /// e-CF class.
    pub kind: DgiiDocumentKind,
    /// Issuer RNC (Registro Nacional del Contribuyente,
    /// 9 or 11 ASCII digits).
    pub issuer_rnc: String,
    /// e-NCF (e-Comprobante Fiscal number, 13 chars: `E` +
    /// 2-digit type + 10-digit sequential).
    pub e_ncf: String,
    /// Canonical signed XML payload.
    pub ecf_xml: Vec<u8>,
}

/// DGII per-e-CF verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DgiiStatus {
    /// EnProceso — DGII received the upload; awaiting
    /// validation.
    EnProceso,
    /// Aceptado — DGII validation passed.
    Aceptado,
    /// Rechazado — DGII validation rejected the payload.
    Rechazado,
    /// AceptadoCondicional — accepted with observations
    /// the engine should surface.
    AceptadoCondicional,
}

/// What [`DgiiProvider::submit_ecf`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DgiiSubmitEnvelope {
    /// DGII-assigned TrackId for async reconciliation.
    pub track_id: String,
    /// e-NCF echoed by DGII.
    pub e_ncf: String,
    /// Latest observed status.
    pub status: DgiiStatus,
    /// RFC-3339 UTC timestamp DGII recorded.
    pub received_at: String,
    /// Mensaje text from DGII when status is `Rechazado`
    /// or `AceptadoCondicional`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mensaje: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum DgiiError {
    /// e-CF XML failed shape validation before the wire.
    #[error("ecf xml rejected: {0}")]
    BadXml(String),
    /// RNC didn't match the 9-or-11-digit shape.
    #[error("invalid RNC: {0}")]
    BadRnc(String),
    /// e-NCF didn't match the `E` + 2-digit type + 10-digit
    /// sequential shape.
    #[error("invalid e-NCF: {0}")]
    BadENcf(String),
    /// HTTP / TLS / DNS failure talking to DGII.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The DGII integration surface.
pub trait DgiiProvider: Send + Sync {
    /// Submit one e-CF to DGII.
    ///
    /// # Errors
    ///
    /// Returns [`DgiiError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// DGII-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `DgiiStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_ecf(&self, request: &DgiiSubmitRequest) -> Result<DgiiSubmitEnvelope, DgiiError>;
}

/// Deterministic mock provider.
pub struct MockDgiiProvider {
    fixed_received_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockDgiiProvider {
    /// Build a mock with deterministic timestamps + serial
    /// TrackIds.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_received_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_received_at(received_at: impl Into<String>) -> Self {
        Self {
            fixed_received_at: received_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockDgiiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DgiiProvider for MockDgiiProvider {
    fn submit_ecf(&self, request: &DgiiSubmitRequest) -> Result<DgiiSubmitEnvelope, DgiiError> {
        validate_rnc(&request.issuer_rnc)?;
        validate_e_ncf(&request.e_ncf)?;
        if request.ecf_xml.is_empty() {
            return Err(DgiiError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(DgiiSubmitEnvelope {
            track_id: format!("DGII-{serial:012}"),
            e_ncf: request.e_ncf.clone(),
            status: DgiiStatus::Aceptado,
            received_at: self.fixed_received_at.clone(),
            mensaje: None,
        })
    }
}

/// Validate a Dominican RNC — 9 or 11 ASCII digits.
///
/// # Errors
///
/// Returns [`DgiiError::BadRnc`] on shape failure.
pub fn validate_rnc(rnc: &str) -> Result<(), DgiiError> {
    if matches!(rnc.len(), 9 | 11) && rnc.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(DgiiError::BadRnc(format!(
            "RNC must be 9 or 11 ASCII digits, got {rnc:?}"
        )))
    }
}

/// Validate an e-NCF — `E` + 2-digit type + 10-digit
/// sequential.
///
/// # Errors
///
/// Returns [`DgiiError::BadENcf`] on shape failure.
pub fn validate_e_ncf(e_ncf: &str) -> Result<(), DgiiError> {
    if e_ncf.len() == 13
        && e_ncf.starts_with('E')
        && e_ncf.bytes().skip(1).all(|b| b.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(DgiiError::BadENcf(format!(
            "e-NCF must be `E` + 12 ASCII digits, got {e_ncf:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_do_dgii::crate_name(),
///     "invoicekit-report-do-dgii"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-do-dgii"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DgiiSubmitRequest {
        DgiiSubmitRequest {
            tenant_id: "tenant-do-test".to_owned(),
            environment: DgiiEnvironment::Sandbox,
            kind: DgiiDocumentKind::FacturaCreditoFiscal,
            issuer_rnc: "131234567".to_owned(),
            e_ncf: "E310000000001".to_owned(),
            ecf_xml: b"<ECF/>".to_vec(),
        }
    }

    #[test]
    fn submit_ecf_returns_aceptado() {
        let p = MockDgiiProvider::default();
        let env = p.submit_ecf(&sample_request()).unwrap();
        assert_eq!(env.status, DgiiStatus::Aceptado);
        assert!(env.track_id.starts_with("DGII-"));
        assert_eq!(env.e_ncf, "E310000000001");
    }

    #[test]
    fn submit_ecf_serial_increments() {
        let p = MockDgiiProvider::default();
        let env1 = p.submit_ecf(&sample_request()).unwrap();
        let env2 = p.submit_ecf(&sample_request()).unwrap();
        assert_ne!(env1.track_id, env2.track_id);
    }

    #[test]
    fn submit_ecf_rejects_empty_payload() {
        let p = MockDgiiProvider::default();
        let mut req = sample_request();
        req.ecf_xml.clear();
        let err = p.submit_ecf(&req).unwrap_err();
        assert!(matches!(err, DgiiError::BadXml(_)));
    }

    #[test]
    fn submit_ecf_rejects_bad_rnc() {
        let p = MockDgiiProvider::default();
        let mut req = sample_request();
        req.issuer_rnc = "BAD".to_owned();
        let err = p.submit_ecf(&req).unwrap_err();
        assert!(matches!(err, DgiiError::BadRnc(_)));
    }

    #[test]
    fn submit_ecf_rejects_bad_e_ncf() {
        let p = MockDgiiProvider::default();
        let mut req = sample_request();
        req.e_ncf = "BADENCF".to_owned();
        let err = p.submit_ecf(&req).unwrap_err();
        assert!(matches!(err, DgiiError::BadENcf(_)));
    }

    #[test]
    fn document_kind_codes_match_dgii_catalog() {
        assert_eq!(DgiiDocumentKind::FacturaCreditoFiscal.code(), 31);
        assert_eq!(DgiiDocumentKind::FacturaConsumo.code(), 32);
        assert_eq!(DgiiDocumentKind::NotaDebito.code(), 33);
        assert_eq!(DgiiDocumentKind::NotaCredito.code(), 34);
        assert_eq!(DgiiDocumentKind::Compras.code(), 41);
        assert_eq!(DgiiDocumentKind::GastosMenores.code(), 43);
        assert_eq!(DgiiDocumentKind::RegimenesEspeciales.code(), 44);
        assert_eq!(DgiiDocumentKind::Gubernamental.code(), 45);
        assert_eq!(DgiiDocumentKind::Exportaciones.code(), 46);
        assert_eq!(DgiiDocumentKind::PagosExterior.code(), 47);
    }

    #[test]
    fn validate_rnc_round_trip() {
        assert!(validate_rnc("131234567").is_ok());
        assert!(validate_rnc("13123456789").is_ok());
        assert!(validate_rnc("1312345").is_err());
        assert!(validate_rnc("13123456A").is_err());
    }

    #[test]
    fn validate_e_ncf_round_trip() {
        assert!(validate_e_ncf("E310000000001").is_ok());
        assert!(validate_e_ncf("E321234567890").is_ok());
        assert!(validate_e_ncf("X310000000001").is_err());
        assert!(validate_e_ncf("E31000000001A").is_err());
        assert!(validate_e_ncf("E3100000001").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = DgiiSubmitEnvelope {
            track_id: "DGII-000000000007".to_owned(),
            e_ncf: "E310000000007".to_owned(),
            status: DgiiStatus::Rechazado,
            received_at: "2026-01-01T00:00:00Z".to_owned(),
            mensaje: Some("RNC del receptor inválido".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: DgiiSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
