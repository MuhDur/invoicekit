// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Costa Rica **Ministerio de Hacienda** e-invoicing clearance adapter.
//!
//! Costa Rica's Ministerio de Hacienda runs the ATV
//! (Administración Tributaria Virtual) clearance regime
//! through `api.comprobanteselectronicos.go.cr`. Issuers sign
//! XML with a Banco Central CR (BCCR) certificate, compute a
//! 50-character **Clave Numérica** (numeric key concatenating
//! country, date, cédula, tipo, situación, consecutivo, and
//! the código de seguridad), and submit to Hacienda; the
//! authority returns an `aceptado` / `rechazado` envelope.
//!
//! Document codes mirror Hacienda's `tipoDocumento`
//! taxonomy:
//!
//! * 01 Factura Electrónica
//! * 02 Nota de Débito Electrónica
//! * 03 Nota de Crédito Electrónica
//! * 04 Tiquete Electrónico (B2C)
//! * 08 Factura Electrónica de Compra
//! * 09 Factura Electrónica de Exportación
//!
//! Ships typed surface + [`MockHaciendaProvider`]; the live
//! ATV REST integration lands in a follow-up
//! `report-cr-hacienda-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the Hacienda transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HaciendaEnvironment {
    /// `api.comprobanteselectronicos.go.cr/recepcion-sandbox`.
    Sandbox,
    /// `api.comprobanteselectronicos.go.cr/recepcion` /
    /// production.
    Produccion,
}

/// Hacienda document class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HaciendaDocumentKind {
    /// 01 Factura Electrónica.
    Factura,
    /// 02 Nota de Débito Electrónica.
    NotaDebito,
    /// 03 Nota de Crédito Electrónica.
    NotaCredito,
    /// 04 Tiquete Electrónico (B2C).
    Tiquete,
    /// 08 Factura Electrónica de Compra.
    FacturaCompra,
    /// 09 Factura Electrónica de Exportación.
    FacturaExportacion,
}

impl HaciendaDocumentKind {
    /// Hacienda `tipoDocumento` code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Factura => "01",
            Self::NotaDebito => "02",
            Self::NotaCredito => "03",
            Self::Tiquete => "04",
            Self::FacturaCompra => "08",
            Self::FacturaExportacion => "09",
        }
    }
}

/// What the operator passes in to
/// [`HaciendaProvider::submit_comprobante`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HaciendaSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: HaciendaEnvironment,
    /// Document class.
    pub kind: HaciendaDocumentKind,
    /// Issuer cédula (9-12 ASCII digits depending on physical
    /// / juridical / DIMEX / NITE shape).
    pub issuer_cedula: String,
    /// 50-character Clave Numérica computed by the engine.
    pub clave_numerica: String,
    /// Consecutivo (20-digit ASCII sequence number).
    pub consecutivo: String,
    /// Canonical signed XML payload.
    pub comprobante_xml: Vec<u8>,
}

/// Hacienda per-document verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HaciendaStatus {
    /// Recibido — Hacienda received the upload; awaiting
    /// validation.
    Recibido,
    /// Aceptado — Hacienda validation passed.
    Aceptado,
    /// Rechazado — Hacienda validation rejected the payload.
    Rechazado,
}

/// What [`HaciendaProvider::submit_comprobante`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HaciendaSubmitEnvelope {
    /// Clave Numérica echoed by Hacienda.
    pub clave_numerica: String,
    /// Latest observed status.
    pub status: HaciendaStatus,
    /// RFC-3339 UTC timestamp Hacienda recorded.
    pub received_at: String,
    /// Mensaje text from Hacienda when status is
    /// `Rechazado`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mensaje: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum HaciendaError {
    /// Comprobante XML failed shape validation before the
    /// wire.
    #[error("comprobante xml rejected: {0}")]
    BadXml(String),
    /// Cédula didn't match a recognised shape.
    #[error("invalid cedula: {0}")]
    BadCedula(String),
    /// Clave numérica didn't match the 50-digit shape.
    #[error("invalid clave numerica: {0}")]
    BadClave(String),
    /// Consecutivo didn't match the 20-digit shape.
    #[error("invalid consecutivo: {0}")]
    BadConsecutivo(String),
    /// HTTP / TLS / DNS failure talking to Hacienda.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The Hacienda integration surface.
pub trait HaciendaProvider: Send + Sync {
    /// Submit one comprobante to Hacienda.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// Hacienda-returned `Rechazado` verdict is NOT an
    /// `Err` — it's surfaced via `HaciendaStatus::Rechazado`
    /// inside the envelope so the engine persists the
    /// rejection alongside its audit trail.
    fn submit_comprobante(
        &self,
        request: &HaciendaSubmitRequest,
    ) -> Result<HaciendaSubmitEnvelope, HaciendaError>;
}

/// Deterministic mock provider.
pub struct MockHaciendaProvider {
    fixed_received_at: String,
    /// When set, the mock forces this terminal verdict (with an optional
    /// `mensaje`) on every otherwise shape-valid submission. This is how the
    /// caller exercises Hacienda's `Rechazado` envelope path — the authority
    /// rejection is a `HaciendaStatus`, not an `Err` — without standing up the
    /// live ATV REST backend.
    forced_status: Option<(HaciendaStatus, Option<String>)>,
}

impl MockHaciendaProvider {
    /// Build a mock with a deterministic received_at.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_received_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_received_at(received_at: impl Into<String>) -> Self {
        Self {
            fixed_received_at: received_at.into(),
            forced_status: None,
        }
    }

    /// Force the terminal Hacienda verdict on shape-valid submissions.
    ///
    /// Use this to drive the authority `Rechazado` / `Recibido` envelope
    /// branches. The supplied `mensaje` is echoed verbatim into the envelope
    /// (Hacienda only populates `mensaje` on `Rechazado`, mirroring the
    /// `MensajeHacienda` response document).
    #[must_use]
    pub fn with_forced_status(
        mut self,
        status: HaciendaStatus,
        mensaje: Option<String>,
    ) -> Self {
        self.forced_status = Some((status, mensaje));
        self
    }
}

impl Default for MockHaciendaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HaciendaProvider for MockHaciendaProvider {
    fn submit_comprobante(
        &self,
        request: &HaciendaSubmitRequest,
    ) -> Result<HaciendaSubmitEnvelope, HaciendaError> {
        validate_cedula(&request.issuer_cedula)?;
        validate_clave_numerica(&request.clave_numerica)?;
        validate_consecutivo(&request.consecutivo)?;
        if request.comprobante_xml.is_empty() {
            return Err(HaciendaError::BadXml("payload is empty".to_owned()));
        }
        let (status, mensaje) = match &self.forced_status {
            Some((status, mensaje)) => (*status, mensaje.clone()),
            None => (HaciendaStatus::Aceptado, None),
        };
        Ok(HaciendaSubmitEnvelope {
            clave_numerica: request.clave_numerica.clone(),
            status,
            received_at: self.fixed_received_at.clone(),
            mensaje,
        })
    }
}

/// Validate a Costa Rican cédula — 9-12 ASCII digits
/// (physical 9, juridical 10, DIMEX 11-12, NITE 10).
///
/// # Errors
///
/// Returns [`HaciendaError::BadCedula`] on shape failure.
pub fn validate_cedula(cedula: &str) -> Result<(), HaciendaError> {
    if (9..=12).contains(&cedula.len()) && cedula.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(HaciendaError::BadCedula(format!(
            "cedula must be 9-12 ASCII digits, got {cedula:?}"
        )))
    }
}

/// Validate a Clave Numérica — exactly 50 ASCII digits.
///
/// # Errors
///
/// Returns [`HaciendaError::BadClave`] on shape failure.
pub fn validate_clave_numerica(clave: &str) -> Result<(), HaciendaError> {
    if clave.len() == 50 && clave.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(HaciendaError::BadClave(format!(
            "clave numerica must be 50 ASCII digits, got len={}",
            clave.len()
        )))
    }
}

/// Validate a consecutivo — exactly 20 ASCII digits.
///
/// # Errors
///
/// Returns [`HaciendaError::BadConsecutivo`] on shape
/// failure.
pub fn validate_consecutivo(consecutivo: &str) -> Result<(), HaciendaError> {
    if consecutivo.len() == 20 && consecutivo.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(HaciendaError::BadConsecutivo(format!(
            "consecutivo must be 20 ASCII digits, got len={}",
            consecutivo.len()
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_cr_hacienda::crate_name(),
///     "invoicekit-report-cr-hacienda"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-cr-hacienda"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> HaciendaSubmitRequest {
        HaciendaSubmitRequest {
            tenant_id: "tenant-cr-test".to_owned(),
            environment: HaciendaEnvironment::Sandbox,
            kind: HaciendaDocumentKind::Factura,
            issuer_cedula: "3101123456".to_owned(),
            clave_numerica: "5".repeat(50),
            consecutivo: "0".repeat(20),
            comprobante_xml: b"<FacturaElectronica/>".to_vec(),
        }
    }

    #[test]
    fn submit_comprobante_returns_aceptado() {
        let p = MockHaciendaProvider::default();
        let env = p.submit_comprobante(&sample_request()).unwrap();
        assert_eq!(env.status, HaciendaStatus::Aceptado);
        assert_eq!(env.clave_numerica, "5".repeat(50));
    }

    #[test]
    fn submit_comprobante_rejects_empty_payload() {
        let p = MockHaciendaProvider::default();
        let mut req = sample_request();
        req.comprobante_xml.clear();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, HaciendaError::BadXml(_)));
    }

    #[test]
    fn submit_comprobante_rejects_bad_cedula() {
        let p = MockHaciendaProvider::default();
        let mut req = sample_request();
        req.issuer_cedula = "BAD".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, HaciendaError::BadCedula(_)));
    }

    #[test]
    fn submit_comprobante_rejects_bad_clave() {
        let p = MockHaciendaProvider::default();
        let mut req = sample_request();
        req.clave_numerica = "1".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, HaciendaError::BadClave(_)));
    }

    #[test]
    fn submit_comprobante_rejects_bad_consecutivo() {
        let p = MockHaciendaProvider::default();
        let mut req = sample_request();
        req.consecutivo = "1".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, HaciendaError::BadConsecutivo(_)));
    }

    #[test]
    fn document_kind_codes_match_hacienda_taxonomy() {
        assert_eq!(HaciendaDocumentKind::Factura.code(), "01");
        assert_eq!(HaciendaDocumentKind::NotaDebito.code(), "02");
        assert_eq!(HaciendaDocumentKind::NotaCredito.code(), "03");
        assert_eq!(HaciendaDocumentKind::Tiquete.code(), "04");
        assert_eq!(HaciendaDocumentKind::FacturaCompra.code(), "08");
        assert_eq!(HaciendaDocumentKind::FacturaExportacion.code(), "09");
    }

    #[test]
    fn validators_round_trip() {
        assert!(validate_cedula("123456789").is_ok());
        assert!(validate_cedula("310112345678").is_ok());
        assert!(validate_cedula("12345").is_err());
        assert!(validate_cedula("1234567890A").is_err());
        assert!(validate_clave_numerica(&"5".repeat(50)).is_ok());
        assert!(validate_clave_numerica(&"5".repeat(49)).is_err());
        assert!(validate_consecutivo(&"0".repeat(20)).is_ok());
        assert!(validate_consecutivo(&"0".repeat(19)).is_err());
    }

    #[test]
    fn forced_rechazado_surfaces_as_status_not_error() {
        // Per the provider contract, a Hacienda refusal is a `Rechazado`
        // verdict carried *inside* the Ok envelope — never an `Err`.
        let p = MockHaciendaProvider::new().with_forced_status(
            HaciendaStatus::Rechazado,
            Some("clave numerica ya registrada".to_owned()),
        );
        let env = p.submit_comprobante(&sample_request()).unwrap();
        assert_eq!(env.status, HaciendaStatus::Rechazado);
        assert_eq!(env.mensaje.as_deref(), Some("clave numerica ya registrada"));
        // The clave is still echoed even on rejection, so the audit trail can
        // correlate the refusal with the submitted comprobante.
        assert_eq!(env.clave_numerica, "5".repeat(50));
    }

    #[test]
    fn forced_recibido_models_async_pending_verdict() {
        // Hacienda's recepcion endpoint can return `recibido` (queued) before
        // the asynchronous `MensajeHacienda` resolves to aceptado/rechazado.
        let p = MockHaciendaProvider::new()
            .with_forced_status(HaciendaStatus::Recibido, None);
        let env = p.submit_comprobante(&sample_request()).unwrap();
        assert_eq!(env.status, HaciendaStatus::Recibido);
        assert!(env.mensaje.is_none());
    }

    #[test]
    fn forced_status_still_runs_prewire_shape_validation() {
        // A forced verdict must not bypass local shape checks: a malformed
        // cédula is refused pre-wire regardless of the configured verdict.
        let p = MockHaciendaProvider::new()
            .with_forced_status(HaciendaStatus::Rechazado, None);
        let mut req = sample_request();
        req.issuer_cedula = "BAD".to_owned();
        let err = p.submit_comprobante(&req).unwrap_err();
        assert!(matches!(err, HaciendaError::BadCedula(_)));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = HaciendaSubmitEnvelope {
            clave_numerica: "5".repeat(50),
            status: HaciendaStatus::Rechazado,
            received_at: "2026-01-01T00:00:00Z".to_owned(),
            mensaje: Some("XML mal formado".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: HaciendaSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
