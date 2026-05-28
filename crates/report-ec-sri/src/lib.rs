// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Ecuador **SRI** e-invoicing clearance adapter.
//!
//! Ecuador's Servicio de Rentas Internas (SRI) operates the
//! country's clearance regime. Issuers sign XML using a
//! security certificate from a Banco Central de Ecuador
//! (BCE) or Security Data partner, compute a 49-digit
//! **Clave de Acceso** (access key), and submit to SRI via
//! SOAP. SRI returns an autorización envelope with a
//! `numeroAutorizacion` (the access key, post-authorization)
//! and `fechaAutorizacion`.
//!
//! Document codes mirror SRI's `tipoComprobante` taxonomy:
//! - 01 Factura
//! - 04 Nota de Crédito
//! - 05 Nota de Débito
//! - 06 Guía de Remisión
//! - 07 Comprobante de Retención
//! - 03 Liquidación de Compra
//!
//! This crate ships the typed surface and a deterministic
//! [`MockSriProvider`]. The live SRI SOAP integration lands
//! in a follow-up `report-ec-sri-http` crate behind a feature
//! flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the SRI transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SriEnvironment {
    /// `celcer.sri.gob.ec` / certificación (sandbox).
    Certificacion,
    /// `cel.sri.gob.ec` / production.
    Produccion,
}

/// SRI document class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SriDocumentKind {
    /// 01 Factura.
    Factura,
    /// 03 Liquidación de Compra.
    LiquidacionCompra,
    /// 04 Nota de Crédito.
    NotaCredito,
    /// 05 Nota de Débito.
    NotaDebito,
    /// 06 Guía de Remisión.
    GuiaRemision,
    /// 07 Comprobante de Retención.
    Retencion,
}

impl SriDocumentKind {
    /// SRI `tipoComprobante` code for this class.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Factura => "01",
            Self::LiquidacionCompra => "03",
            Self::NotaCredito => "04",
            Self::NotaDebito => "05",
            Self::GuiaRemision => "06",
            Self::Retencion => "07",
        }
    }
}

/// What the operator passes in to
/// [`SriProvider::submit_comprobante`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SriSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SriEnvironment,
    /// Document class.
    pub kind: SriDocumentKind,
    /// Issuer RUC (Registro Único de Contribuyentes,
    /// 13 ASCII digits).
    pub issuer_ruc: String,
    /// 49-digit Clave de Acceso computed by the engine.
    pub clave_acceso: String,
    /// Canonical signed XML payload.
    pub comprobante_xml: Vec<u8>,
}

/// SRI per-document verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SriStatus {
    /// Recibido — SRI received the upload; awaiting
    /// authorization.
    Recibido,
    /// Autorizado — SRI authorized the document.
    Autorizado,
    /// Devuelto — SRI rejected on initial validation.
    Devuelto,
    /// NoAutorizado — SRI processed but refused
    /// authorization.
    NoAutorizado,
}

/// What [`SriProvider::submit_comprobante`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SriSubmitEnvelope {
    /// SRI `numeroAutorizacion` (== Clave de Acceso once
    /// authorized).
    pub numero_autorizacion: String,
    /// Latest observed status.
    pub status: SriStatus,
    /// RFC-3339 UTC `fechaAutorizacion` SRI recorded.
    pub fecha_autorizacion: String,
    /// Mensaje text when status is `Devuelto` or
    /// `NoAutorizado`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mensaje: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum SriError {
    /// Comprobante XML failed shape validation before the
    /// wire.
    #[error("comprobante xml rejected: {0}")]
    BadXml(String),
    /// RUC didn't match the 13-digit shape.
    #[error("invalid RUC: {0}")]
    BadRuc(String),
    /// Clave de Acceso didn't match the 49-digit shape.
    #[error("invalid Clave de Acceso: {0}")]
    BadClaveAcceso(String),
    /// HTTP / TLS / DNS / SOAP failure talking to SRI.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The SRI integration surface.
pub trait SriProvider: Send + Sync {
    /// Submit one comprobante to SRI. The provider:
    ///
    /// 1. validates `issuer_ruc` shape,
    /// 2. validates `clave_acceso` shape,
    /// 3. POSTs the signed XML,
    /// 4. returns the SRI-issued autorización envelope.
    ///
    /// # Errors
    ///
    /// Returns [`SriError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// SRI-returned `Devuelto` / `NoAutorizado` verdicts are
    /// NOT `Err`s — they're surfaced via the typed
    /// `SriStatus` inside the envelope so the engine
    /// persists the rejection alongside its audit trail.
    fn submit_comprobante(&self, request: &SriSubmitRequest)
        -> Result<SriSubmitEnvelope, SriError>;
}

/// Deterministic mock provider.
pub struct MockSriProvider {
    fixed_fecha_autorizacion: String,
}

impl MockSriProvider {
    /// Build a mock with a deterministic fecha_autorizacion.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_fecha_autorizacion("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_fecha_autorizacion(fecha: impl Into<String>) -> Self {
        Self {
            fixed_fecha_autorizacion: fecha.into(),
        }
    }
}

impl Default for MockSriProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SriProvider for MockSriProvider {
    fn submit_comprobante(
        &self,
        request: &SriSubmitRequest,
    ) -> Result<SriSubmitEnvelope, SriError> {
        validate_ruc(&request.issuer_ruc)?;
        validate_clave_acceso(&request.clave_acceso)?;
        if request.comprobante_xml.is_empty() {
            return Err(SriError::BadXml("payload is empty".to_owned()));
        }
        Ok(SriSubmitEnvelope {
            numero_autorizacion: request.clave_acceso.clone(),
            status: SriStatus::Autorizado,
            fecha_autorizacion: self.fixed_fecha_autorizacion.clone(),
            mensaje: None,
        })
    }
}

/// Validate an Ecuadorian RUC — exactly 13 ASCII digits.
///
/// # Errors
///
/// Returns [`SriError::BadRuc`] on shape failure.
pub fn validate_ruc(ruc: &str) -> Result<(), SriError> {
    if ruc.len() == 13 && ruc.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SriError::BadRuc(format!(
            "RUC must be 13 ASCII digits, got {ruc:?}"
        )))
    }
}

/// Validate a Clave de Acceso — exactly 49 ASCII digits.
///
/// # Errors
///
/// Returns [`SriError::BadClaveAcceso`] on shape failure.
pub fn validate_clave_acceso(clave: &str) -> Result<(), SriError> {
    if clave.len() == 49 && clave.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(SriError::BadClaveAcceso(format!(
            "Clave de Acceso must be 49 ASCII digits, got len={}",
            clave.len()
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ec_sri::crate_name(),
///     "invoicekit-report-ec-sri"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ec-sri"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SriSubmitRequest {
        SriSubmitRequest {
            tenant_id: "tenant-ec-test".to_owned(),
            environment: SriEnvironment::Certificacion,
            kind: SriDocumentKind::Factura,
            issuer_ruc: "1791234567001".to_owned(),
            clave_acceso: "1".repeat(49),
            comprobante_xml: b"<factura/>".to_vec(),
        }
    }

    #[test]
    fn submit_comprobante_returns_autorizado() {
        let p = MockSriProvider::default();
        let env = p.submit_comprobante(&sample_request()).unwrap();
        assert_eq!(env.status, SriStatus::Autorizado);
        assert_eq!(env.numero_autorizacion, "1".repeat(49));
    }

    #[test]
    fn submit_comprobante_rejects_empty_payload() {
        let p = MockSriProvider::default();
        let mut req = sample_request();
        req.comprobante_xml.clear();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, SriError::BadXml(_)));
    }

    #[test]
    fn submit_comprobante_rejects_bad_ruc() {
        let p = MockSriProvider::default();
        let mut req = sample_request();
        req.issuer_ruc = "BAD".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, SriError::BadRuc(_)));
    }

    #[test]
    fn submit_comprobante_rejects_bad_clave_acceso() {
        let p = MockSriProvider::default();
        let mut req = sample_request();
        req.clave_acceso = "TOO-SHORT".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, SriError::BadClaveAcceso(_)));
    }

    #[test]
    fn document_kind_codes_match_sri_taxonomy() {
        assert_eq!(SriDocumentKind::Factura.code(), "01");
        assert_eq!(SriDocumentKind::LiquidacionCompra.code(), "03");
        assert_eq!(SriDocumentKind::NotaCredito.code(), "04");
        assert_eq!(SriDocumentKind::NotaDebito.code(), "05");
        assert_eq!(SriDocumentKind::GuiaRemision.code(), "06");
        assert_eq!(SriDocumentKind::Retencion.code(), "07");
    }

    #[test]
    fn validate_ruc_round_trip() {
        assert!(validate_ruc("1791234567001").is_ok());
        assert!(validate_ruc("179123456700").is_err());
        assert!(validate_ruc("17912345670011").is_err());
        assert!(validate_ruc("179123456700A").is_err());
    }

    #[test]
    fn validate_clave_acceso_round_trip() {
        assert!(validate_clave_acceso(&"1".repeat(49)).is_ok());
        assert!(validate_clave_acceso(&"1".repeat(48)).is_err());
        assert!(validate_clave_acceso(&"1".repeat(50)).is_err());
        let mut bad = "1".repeat(48);
        bad.push('A');
        assert!(validate_clave_acceso(&bad).is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = SriSubmitEnvelope {
            numero_autorizacion: "1".repeat(49),
            status: SriStatus::Devuelto,
            fecha_autorizacion: "2026-01-01T00:00:00Z".to_owned(),
            mensaje: Some("clave de acceso ya existe".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SriSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
