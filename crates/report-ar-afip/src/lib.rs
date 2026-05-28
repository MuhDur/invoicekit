// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Argentina **AFIP** e-invoicing (WSFE / WSFEX / WSMTXCA).
//!
//! Argentina's Administración Federal de Ingresos Públicos
//! (AFIP) operates the **CAE** (Código de Autorización
//! Electrónico) clearance regime. Issuers request a CAE per
//! invoice from one of three SOAP web services:
//!
//! - **WSFE** — domestic factura (sin detalle de items).
//! - **WSFEX** — facturación de exportación.
//! - **WSMTXCA** — factura con detalle de items (per-line
//!   breakdown).
//!
//! The CAE + its expiry date go on the printed invoice; the
//! buyer can validate via AFIP's public lookup.
//!
//! Letter classes (A/B/C/E/M) distinguish whether the issuer
//! is "responsable inscripto" / "monotributista" / "exento"
//! / "exportación", which determines IVA discrimination
//! requirements on the printed invoice.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockAfipProvider`]. The live AFIP SOAP integration
//! lands in a follow-up `report-ar-afip-http` crate behind a
//! feature flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the AFIP transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfipEnvironment {
    /// `wswhomo.afip.gov.ar` / homologación (sandbox).
    Homologacion,
    /// `servicios1.afip.gov.ar` / production.
    Produccion,
}

/// Which AFIP web service handles this invoice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfipService {
    /// WSFE — Factura Electrónica (sin detalle).
    Wsfe,
    /// WSFEX — Factura de Exportación.
    Wsfex,
    /// WSMTXCA — Factura con detalle de items.
    Wsmtxca,
}

/// AFIP letter class.
///
/// A = responsable inscripto; B = consumidor final /
/// monotributista; C = monotributista emisor; E =
/// exportación; M = factura de operación presunta. The
/// letter determines IVA discrimination requirements on
/// the printed invoice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfipLetter {
    /// Letter A.
    A,
    /// Letter B.
    B,
    /// Letter C.
    C,
    /// Letter E (exportación).
    E,
    /// Letter M.
    M,
}

/// What the operator passes in to
/// [`AfipProvider::request_cae`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AfipCaeRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: AfipEnvironment,
    /// Which web service to call.
    pub service: AfipService,
    /// Letter class.
    pub letter: AfipLetter,
    /// Issuer CUIT (Clave Única de Identificación Tributaria,
    /// 11 ASCII digits).
    pub issuer_cuit: String,
    /// Punto de venta (5-digit ASCII).
    pub punto_venta: String,
    /// Canonical request payload (XML or JSON depending on
    /// the service — opaque from the engine's perspective).
    pub request_payload: Vec<u8>,
}

/// AFIP per-CAE verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfipStatus {
    /// CAE granted; issuer can print + send.
    Aprobado,
    /// AFIP rejected the request (typed observación).
    Rechazado,
    /// AFIP rejected with observations (CAE granted but
    /// engine should surface the warnings).
    AprobadoConObservaciones,
}

/// What [`AfipProvider::request_cae`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AfipCaeEnvelope {
    /// 14-digit AFIP-issued CAE.
    pub cae: String,
    /// CAE expiry date (`YYYYMMDD`).
    pub cae_expiry_yyyymmdd: String,
    /// Latest observed status.
    pub status: AfipStatus,
    /// RFC-3339 UTC timestamp AFIP recorded.
    pub authorized_at: String,
    /// Free-form observaciones text when status is
    /// `Rechazado` or `AprobadoConObservaciones`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observaciones: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum AfipError {
    /// Request payload failed shape validation before the
    /// wire.
    #[error("request payload rejected: {0}")]
    BadPayload(String),
    /// CUIT didn't match the 11-digit shape.
    #[error("invalid CUIT: {0}")]
    BadCuit(String),
    /// Punto de venta didn't match the 5-digit shape.
    #[error("invalid punto de venta: {0}")]
    BadPuntoVenta(String),
    /// HTTP / TLS / DNS / SOAP failure talking to AFIP.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The AFIP integration surface.
pub trait AfipProvider: Send + Sync {
    /// Request a CAE for one invoice. The provider:
    ///
    /// 1. validates `issuer_cuit` shape,
    /// 2. validates `punto_venta` shape,
    /// 3. POSTs the request to the chosen `service`
    ///    (WSFE/WSFEX/WSMTXCA),
    /// 4. returns the CAE envelope.
    ///
    /// # Errors
    ///
    /// Returns [`AfipError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// AFIP-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `AfipStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn request_cae(&self, request: &AfipCaeRequest) -> Result<AfipCaeEnvelope, AfipError>;
}

/// Deterministic mock provider.
pub struct MockAfipProvider {
    fixed_authorized_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockAfipProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_authorized_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_authorized_at(authorized_at: impl Into<String>) -> Self {
        Self {
            fixed_authorized_at: authorized_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockAfipProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AfipProvider for MockAfipProvider {
    fn request_cae(&self, request: &AfipCaeRequest) -> Result<AfipCaeEnvelope, AfipError> {
        validate_cuit(&request.issuer_cuit)?;
        validate_punto_venta(&request.punto_venta)?;
        if request.request_payload.is_empty() {
            return Err(AfipError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(AfipCaeEnvelope {
            cae: format!("{serial:0>14}"),
            cae_expiry_yyyymmdd: "20260131".to_owned(),
            status: AfipStatus::Aprobado,
            authorized_at: self.fixed_authorized_at.clone(),
            observaciones: None,
        })
    }
}

/// Validate an Argentine CUIT — exactly 11 ASCII digits.
///
/// # Errors
///
/// Returns [`AfipError::BadCuit`] on shape failure.
pub fn validate_cuit(cuit: &str) -> Result<(), AfipError> {
    if cuit.len() == 11 && cuit.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(AfipError::BadCuit(format!(
            "CUIT must be 11 ASCII digits, got {cuit:?}"
        )))
    }
}

/// Validate an AFIP punto de venta — exactly 5 ASCII digits.
///
/// # Errors
///
/// Returns [`AfipError::BadPuntoVenta`] on shape failure.
pub fn validate_punto_venta(pv: &str) -> Result<(), AfipError> {
    if pv.len() == 5 && pv.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(AfipError::BadPuntoVenta(format!(
            "punto de venta must be 5 ASCII digits, got {pv:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ar_afip::crate_name(),
///     "invoicekit-report-ar-afip"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ar-afip"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> AfipCaeRequest {
        AfipCaeRequest {
            tenant_id: "tenant-ar-test".to_owned(),
            environment: AfipEnvironment::Homologacion,
            service: AfipService::Wsfe,
            letter: AfipLetter::A,
            issuer_cuit: "20123456789".to_owned(),
            punto_venta: "00001".to_owned(),
            request_payload: b"<FECAESolicitar/>".to_vec(),
        }
    }

    #[test]
    fn request_cae_returns_aprobado_with_cae() {
        let p = MockAfipProvider::default();
        let env = p.request_cae(&sample_request()).unwrap();
        assert_eq!(env.status, AfipStatus::Aprobado);
        assert_eq!(env.cae.len(), 14);
        assert_eq!(env.cae_expiry_yyyymmdd, "20260131");
    }

    #[test]
    fn request_cae_serial_increments() {
        let p = MockAfipProvider::default();
        let env1 = p.request_cae(&sample_request()).unwrap();
        let env2 = p.request_cae(&sample_request()).unwrap();
        assert_ne!(env1.cae, env2.cae);
    }

    #[test]
    fn request_cae_rejects_empty_payload() {
        let p = MockAfipProvider::default();
        let mut req = sample_request();
        req.request_payload.clear();
        let err = p.request_cae(&req).unwrap_err();
        assert!(matches!(err, AfipError::BadPayload(_)));
    }

    #[test]
    fn request_cae_rejects_bad_cuit() {
        let p = MockAfipProvider::default();
        let mut req = sample_request();
        req.issuer_cuit = "BAD".to_owned();
        let err = p.request_cae(&req).unwrap_err();
        assert!(matches!(err, AfipError::BadCuit(_)));
    }

    #[test]
    fn request_cae_rejects_bad_punto_venta() {
        let p = MockAfipProvider::default();
        let mut req = sample_request();
        req.punto_venta = "001".to_owned();
        let err = p.request_cae(&req).unwrap_err();
        assert!(matches!(err, AfipError::BadPuntoVenta(_)));
    }

    #[test]
    fn validate_cuit_round_trip() {
        assert!(validate_cuit("20123456789").is_ok());
        assert!(validate_cuit("2012345678").is_err());
        assert!(validate_cuit("201234567890").is_err());
        assert!(validate_cuit("20123A56789").is_err());
    }

    #[test]
    fn validate_punto_venta_round_trip() {
        assert!(validate_punto_venta("00001").is_ok());
        assert!(validate_punto_venta("12345").is_ok());
        assert!(validate_punto_venta("0001").is_err());
        assert!(validate_punto_venta("000001").is_err());
        assert!(validate_punto_venta("0001A").is_err());
    }

    #[test]
    fn service_serde_round_trips_all_three_variants() {
        for s in [AfipService::Wsfe, AfipService::Wsfex, AfipService::Wsmtxca] {
            let json = serde_json::to_string(&s).unwrap();
            let parsed: AfipService = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn letter_serde_round_trips_all_five_variants() {
        for l in [
            AfipLetter::A,
            AfipLetter::B,
            AfipLetter::C,
            AfipLetter::E,
            AfipLetter::M,
        ] {
            let json = serde_json::to_string(&l).unwrap();
            let parsed: AfipLetter = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, l);
        }
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = AfipCaeEnvelope {
            cae: "70000000000007".to_owned(),
            cae_expiry_yyyymmdd: "20260131".to_owned(),
            status: AfipStatus::Rechazado,
            authorized_at: "2026-01-01T00:00:00Z".to_owned(),
            observaciones: Some("CUIT del receptor inválido".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: AfipCaeEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
