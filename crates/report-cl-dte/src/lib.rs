// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Chile **SII DTE** (Documento Tributario Electrónico) reporting adapter.
//!
//! Chile's Servicio de Impuestos Internos (SII) operates the
//! gold-standard LATAM clearance regime, in production since
//! 2003. Every Chilean B2B issuer signs a typed XML DTE,
//! consumes a **folio** from a CAF (Código de Autorización
//! de Folios) bundle the SII issued in advance, and submits
//! to the SII; the SII returns a TrackId for reconciliation
//! and within minutes a typed Aceptado / Rechazado state.
//!
//! Key DTE kinds with SII tipo codes:
//! - 33 Factura Electrónica
//! - 34 Factura No Afecta o Exenta
//! - 39 Boleta Electrónica
//! - 41 Boleta No Afecta o Exenta
//! - 46 Factura de Compra
//! - 52 Guía de Despacho
//! - 56 Nota de Débito
//! - 61 Nota de Crédito
//!
//! This crate ships the typed surface and a deterministic
//! [`MockSiiProvider`]. The live SII SOAP integration lands
//! in a follow-up `report-cl-dte-http` crate behind a feature
//! flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the SII transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiiEnvironment {
    /// `maullin.sii.cl` / SII certification (sandbox).
    Certification,
    /// `palena.sii.cl` / production.
    Production,
}

/// DTE class (subset of the most common SII tipo codes).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DteKind {
    /// 33 Factura Electrónica (B2B affected by IVA).
    FacturaElectronica,
    /// 34 Factura No Afecta o Exenta.
    FacturaExenta,
    /// 39 Boleta Electrónica (B2C).
    BoletaElectronica,
    /// 41 Boleta No Afecta o Exenta.
    BoletaExenta,
    /// 46 Factura de Compra (self-billed purchase).
    FacturaCompra,
    /// 52 Guía de Despacho (delivery / movement note).
    GuiaDespacho,
    /// 56 Nota de Débito.
    NotaDebito,
    /// 61 Nota de Crédito.
    NotaCredito,
}

impl DteKind {
    /// SII tipo code (`tipo DTE`) for this class.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::FacturaElectronica => 33,
            Self::FacturaExenta => 34,
            Self::BoletaElectronica => 39,
            Self::BoletaExenta => 41,
            Self::FacturaCompra => 46,
            Self::GuiaDespacho => 52,
            Self::NotaDebito => 56,
            Self::NotaCredito => 61,
        }
    }
}

/// What the operator passes in to [`SiiProvider::submit_dte`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SiiSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SiiEnvironment,
    /// DTE class.
    pub kind: DteKind,
    /// Issuer RUT (`NNNNNNNN-X`, where `X` is digit or `K`).
    pub issuer_rut: String,
    /// Folio consumed from the issuer's CAF bundle.
    pub folio: u64,
    /// Canonical signed DTE XML payload.
    pub dte_xml: Vec<u8>,
}

/// SII per-DTE verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiiStatus {
    /// Recibido — SII received the upload; awaiting
    /// validation.
    Recibido,
    /// Aceptado — SII validation passed; the DTE is final.
    Aceptado,
    /// Aceptado con Reparos — validation passed with
    /// warnings.
    AceptadoConReparos,
    /// Rechazado — SII validation rejected the DTE.
    Rechazado,
}

/// What [`SiiProvider::submit_dte`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SiiSubmitEnvelope {
    /// SII-assigned TrackId.
    pub track_id: String,
    /// Latest observed status.
    pub status: SiiStatus,
    /// RFC-3339 UTC timestamp SII recorded.
    pub submitted_at: String,
    /// Glosa from SII when `status == Rechazado` or
    /// `AceptadoConReparos`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glosa: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum SiiError {
    /// DTE XML failed shape validation before the wire.
    #[error("dte xml rejected: {0}")]
    BadXml(String),
    /// RUT didn't match SII's `NNNNNNNN-X` shape.
    #[error("invalid RUT: {0}")]
    BadRut(String),
    /// Folio out of CAF range.
    #[error("invalid folio: {0}")]
    BadFolio(String),
    /// HTTP / TLS / DNS failure talking to SII.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The SII integration surface.
pub trait SiiProvider: Send + Sync {
    /// Submit one DTE to SII. The provider:
    ///
    /// 1. validates `issuer_rut` shape,
    /// 2. validates `folio` is non-zero,
    /// 3. POSTs the signed DTE XML,
    /// 4. returns the SII-assigned TrackId envelope.
    ///
    /// # Errors
    ///
    /// Returns [`SiiError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// SII-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `SiiStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_dte(&self, request: &SiiSubmitRequest) -> Result<SiiSubmitEnvelope, SiiError>;

    /// Poll SII for the latest status of a previously
    /// submitted DTE.
    ///
    /// # Errors
    ///
    /// Returns [`SiiError::Transport`] when the TrackId is
    /// unknown.
    fn query_track_id(
        &self,
        environment: SiiEnvironment,
        track_id: &str,
    ) -> Result<SiiSubmitEnvelope, SiiError>;
}

/// Deterministic mock provider.
pub struct MockSiiProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockSiiProvider {
    /// Build a mock with deterministic timestamps + serial
    /// TrackIds.
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

impl Default for MockSiiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SiiProvider for MockSiiProvider {
    fn submit_dte(&self, request: &SiiSubmitRequest) -> Result<SiiSubmitEnvelope, SiiError> {
        validate_rut(&request.issuer_rut)?;
        if request.folio == 0 {
            return Err(SiiError::BadFolio("folio must be > 0".to_owned()));
        }
        if request.dte_xml.is_empty() {
            return Err(SiiError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(SiiSubmitEnvelope {
            track_id: format!("SII-{serial:012}"),
            status: SiiStatus::Recibido,
            submitted_at: self.fixed_submitted_at.clone(),
            glosa: None,
        })
    }

    fn query_track_id(
        &self,
        _environment: SiiEnvironment,
        track_id: &str,
    ) -> Result<SiiSubmitEnvelope, SiiError> {
        if track_id.is_empty() {
            return Err(SiiError::Transport("empty TrackId".to_owned()));
        }
        Ok(SiiSubmitEnvelope {
            track_id: track_id.to_owned(),
            status: SiiStatus::Aceptado,
            submitted_at: self.fixed_submitted_at.clone(),
            glosa: None,
        })
    }
}

/// Validate a Chilean RUT — `NNNNNNNN-X` where `X` is a
/// digit or `K`. The Chilean modulo-11 check digit is a
/// separate concern; this helper only catches obviously-wrong
/// shapes before the wire.
///
/// # Errors
///
/// Returns [`SiiError::BadRut`] on shape failure.
pub fn validate_rut(rut: &str) -> Result<(), SiiError> {
    if let Some((head, tail)) = rut.rsplit_once('-') {
        let head_ok = (1..=8).contains(&head.len()) && head.bytes().all(|b| b.is_ascii_digit());
        let tail_ok = tail.len() == 1
            && tail
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == 'K' || c == 'k');
        if head_ok && tail_ok {
            return Ok(());
        }
    }
    Err(SiiError::BadRut(format!(
        "RUT must be `NNNNNNNN-X` (1-8 digits, dash, digit/K), got {rut:?}"
    )))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_cl_dte::crate_name(),
///     "invoicekit-report-cl-dte"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-cl-dte"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SiiSubmitRequest {
        SiiSubmitRequest {
            tenant_id: "tenant-cl-test".to_owned(),
            environment: SiiEnvironment::Certification,
            kind: DteKind::FacturaElectronica,
            issuer_rut: "12345678-9".to_owned(),
            folio: 4242,
            dte_xml: b"<DTE/>".to_vec(),
        }
    }

    #[test]
    fn submit_dte_returns_recibido_with_track_id() {
        let p = MockSiiProvider::default();
        let env = p.submit_dte(&sample_request()).unwrap();
        assert_eq!(env.status, SiiStatus::Recibido);
        assert!(env.track_id.starts_with("SII-"));
    }

    #[test]
    fn submit_dte_serial_increments_per_provider() {
        let p = MockSiiProvider::default();
        let env1 = p.submit_dte(&sample_request()).unwrap();
        let env2 = p.submit_dte(&sample_request()).unwrap();
        assert_ne!(env1.track_id, env2.track_id);
    }

    #[test]
    fn submit_dte_rejects_empty_payload() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.dte_xml.clear();
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadXml(_)));
    }

    #[test]
    fn submit_dte_rejects_zero_folio() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.folio = 0;
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadFolio(_)));
    }

    #[test]
    fn submit_dte_rejects_bad_rut() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.issuer_rut = "BAD".to_owned();
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadRut(_)));
    }

    #[test]
    fn query_track_id_returns_aceptado() {
        let p = MockSiiProvider::default();
        let env = p
            .query_track_id(SiiEnvironment::Certification, "SII-000000000001")
            .unwrap();
        assert_eq!(env.status, SiiStatus::Aceptado);
    }

    #[test]
    fn query_track_id_rejects_empty() {
        let p = MockSiiProvider::default();
        let err = p
            .query_track_id(SiiEnvironment::Certification, "")
            .unwrap_err();
        assert!(matches!(err, SiiError::Transport(_)));
    }

    #[test]
    fn dte_kind_codes_match_sii_taxonomy() {
        assert_eq!(DteKind::FacturaElectronica.code(), 33);
        assert_eq!(DteKind::FacturaExenta.code(), 34);
        assert_eq!(DteKind::BoletaElectronica.code(), 39);
        assert_eq!(DteKind::BoletaExenta.code(), 41);
        assert_eq!(DteKind::FacturaCompra.code(), 46);
        assert_eq!(DteKind::GuiaDespacho.code(), 52);
        assert_eq!(DteKind::NotaDebito.code(), 56);
        assert_eq!(DteKind::NotaCredito.code(), 61);
    }

    #[test]
    fn validate_rut_accepts_well_formed_strings() {
        assert!(validate_rut("12345678-9").is_ok());
        assert!(validate_rut("12345678-K").is_ok());
        assert!(validate_rut("12345678-k").is_ok());
        assert!(validate_rut("1-9").is_ok());
        assert!(validate_rut("12345678-0").is_ok());
    }

    #[test]
    fn validate_rut_rejects_bad_shapes() {
        assert!(validate_rut("123456789").is_err()); // no dash
        assert!(validate_rut("12345678-XY").is_err()); // 2-char tail
        assert!(validate_rut("ABCDEFGH-9").is_err()); // non-digit head
        assert!(validate_rut("123456789-9").is_err()); // 9-digit head
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = SiiSubmitEnvelope {
            track_id: "SII-000000000007".to_owned(),
            status: SiiStatus::Rechazado,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            glosa: Some("RUT no existe".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SiiSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
