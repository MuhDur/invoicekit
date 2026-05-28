// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// CFDI / SAT acronyms (CFDI, SAT, PAC, RFC, UUID, CSD, FIEL,
// ISR, IVA, IEPS) trip doc-markdown; suppress it crate-wide.
#![allow(clippy::doc_markdown)]

//! `invoicekit-signer-cfdi` — Mexico CFDI 4.0 PAC-signing
//! adapter.
//!
//! Layers the Mexico SAT CFDI 4.0 contract on top of
//! [`invoicekit_signer`]. CFDI requires every invoice to be
//! signed by a Proveedor Autorizado de Certificación (PAC) —
//! a third-party intermediary authorised by SAT to issue the
//! `sello digital` (digital seal) + the `UUID` (Folio Fiscal)
//! that closes the document.
//!
//! Provider surface:
//!
//! * [`CfdiPacProvider`] — provider trait every PAC
//!   integration implements. Bundles the underlying
//!   [`Signer`] with the CFDI-specific operations.
//! * [`CertificadoSelloDigital`] — typed CSD (the taxpayer's
//!   per-RFC certificate used to compute the pre-stamp seal).
//! * [`CfdiStampEnvelope`] — typed envelope: UUID + sello +
//!   cadena_original + the PAC's own certificate number.
//! * [`MockCfdiPacProvider`] — deterministic test provider.
//!
//! # Strict-gate scope
//!
//! Real CFDI signing needs RSA-SHA256 + XSLT for the
//! cadena_original transformation + a PAC sandbox account.
//! The substrate ships the surface today; the real provider
//! lands behind a future `cfdi-rsa` feature flag.

use std::collections::BTreeMap;
use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// CFDI 4.0 invoice type.
///
/// SAT distinguishes invoice / nota de crédito / nota de
/// débito / nómina / pago / traslado / retención; the bridge
/// consumer surfaces the chosen kind so the PAC can validate
/// the right CFDI sub-schema.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CfdiKind {
    /// Ingreso (regular invoice).
    Ingreso,
    /// Egreso (credit note / refund).
    Egreso,
    /// Traslado (transport / movement of goods).
    Traslado,
    /// Nómina (payroll).
    Nomina,
    /// Pago (payment receipt).
    Pago,
    /// Retención (withholding-tax).
    Retencion,
}

impl CfdiKind {
    /// Lowercase wire name (matches the kebab JSON encoding).
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Ingreso => "ingreso",
            Self::Egreso => "egreso",
            Self::Traslado => "traslado",
            Self::Nomina => "nomina",
            Self::Pago => "pago",
            Self::Retencion => "retencion",
        }
    }
}

/// PAC environment (sandbox vs production). SAT requires
/// every PAC to expose both; the operator selects per-tenant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PacEnvironment {
    /// Sandbox.
    Sandbox,
    /// Production.
    Production,
}

impl PacEnvironment {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

/// Taxpayer's Certificado de Sello Digital (CSD) reference.
/// SAT issues one CSD per RFC; the taxpayer uses it to
/// compute the pre-stamp signature that the PAC then wraps in
/// its own seal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CertificadoSelloDigital {
    /// SAT-issued certificate serial number (20-digit string).
    pub serial_number: String,
    /// RFC (Mexican tax id) the CSD is bound to.
    pub rfc: String,
    /// `notBefore` (RFC 3339 UTC).
    pub not_before: String,
    /// `notAfter` (RFC 3339 UTC).
    pub not_after: String,
    /// PEM-encoded X.509 certificate bytes (kept opaque on
    /// the substrate — the real provider parses them).
    pub certificate_pem: Vec<u8>,
}

/// Typed CFDI stamp envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CfdiStampEnvelope {
    /// Underlying [`Signer`] receipt — the raw `sello`
    /// produced over the cadena_original.
    pub signature: Signature,
    /// CFDI kind that was stamped.
    pub kind: CfdiKind,
    /// `UUID` (Folio Fiscal) the PAC assigned to the invoice.
    pub uuid: String,
    /// `cadena_original` — the canonical pre-stamp string
    /// derived from the CFDI XML via the SAT XSLT.
    pub cadena_original: String,
    /// `selloCFDI` (taxpayer seal) as base64 string.
    pub sello_cfdi: String,
    /// `selloSAT` (PAC's wrapping seal) as base64 string.
    pub sello_sat: String,
    /// PAC's certificate serial number.
    pub pac_certificate_serial: String,
    /// Stamping timestamp (`FechaTimbrado`, RFC 3339 UTC).
    pub fecha_timbrado: String,
}

/// Sign-request shape for a CFDI 4.0 stamp.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CfdiSignRequest {
    /// Canonical CFDI 4.0 XML bytes.
    pub cfdi_xml: Vec<u8>,
    /// Taxpayer's CSD.
    pub csd: CertificadoSelloDigital,
    /// CFDI kind.
    pub kind: CfdiKind,
    /// Operator-side metadata (sucursal id, branch, etc.)
    /// forwarded to the PAC for audit. Not part of the signed
    /// envelope; surfaced in the PAC's audit log only.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Errors raised by [`CfdiPacProvider`] implementations.
#[derive(Debug, Error)]
pub enum CfdiError {
    /// Underlying signer refused.
    #[error("cfdi provider's signer refused: {0}")]
    Signer(SigningError),
    /// CSD is expired or not yet valid.
    #[error("cfdi CSD is outside its validity window: serial={0}")]
    CsdInvalid(String),
    /// PAC rejected the cadena / CFDI XML (validation error).
    #[error("PAC rejected the CFDI: {0}")]
    PacRejected(String),
    /// PAC environment mismatch (sandbox CSD against prod
    /// stamp request or vice versa).
    #[error("PAC environment mismatch: csd={csd:?}, request={request:?}")]
    EnvironmentMismatch {
        /// Environment the CSD targets.
        csd: PacEnvironment,
        /// Environment the request targets.
        request: PacEnvironment,
    },
    /// PAC is unreachable.
    #[error("PAC unavailable: {0}")]
    Unavailable(String),
}

/// CFDI PAC provider surface.
pub trait CfdiPacProvider: Send + Sync {
    /// PAC display name (e.g. `solucion-factible`, `edicom`).
    fn provider_name(&self) -> &str;

    /// Environment this provider is configured against.
    fn environment(&self) -> PacEnvironment;

    /// Stamp a CFDI 4.0 invoice.
    ///
    /// # Errors
    ///
    /// Returns [`CfdiError`] when the CSD is invalid, the
    /// PAC rejects the cadena, the environment doesn't
    /// match, or the PAC is unreachable.
    fn stamp(
        &self,
        request: &CfdiSignRequest,
        target_environment: PacEnvironment,
    ) -> Result<CfdiStampEnvelope, CfdiError>;
}

/// Mock CFDI PAC provider — deterministic test outputs.
pub struct MockCfdiPacProvider {
    name: String,
    environment: PacEnvironment,
    signer: std::sync::Arc<dyn Signer>,
    pac_certificate_serial: String,
    fixed_uuid_prefix: String,
    fixed_fecha_timbrado: String,
    stamps: Mutex<Vec<CfdiSignRequest>>,
    next_uuid: Mutex<u64>,
}

impl MockCfdiPacProvider {
    /// Build a mock provider.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        environment: PacEnvironment,
        signer: std::sync::Arc<dyn Signer>,
        pac_certificate_serial: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            environment,
            signer,
            pac_certificate_serial: pac_certificate_serial.into(),
            fixed_uuid_prefix: "00000000-0000-4000-8000-".to_owned(),
            fixed_fecha_timbrado: "2026-01-01T00:00:00Z".to_owned(),
            stamps: Mutex::new(Vec::new()),
            next_uuid: Mutex::new(1),
        }
    }

    /// Override the fixed `fecha_timbrado` returned by the
    /// mock (useful for cassette-replay sandboxes that pin to
    /// a recorded timestamp).
    #[must_use]
    pub fn with_fixed_fecha_timbrado(mut self, fecha: impl Into<String>) -> Self {
        self.fixed_fecha_timbrado = fecha.into();
        self
    }

    /// Snapshot of every recorded stamp request.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn stamps(&self) -> Vec<CfdiSignRequest> {
        self.stamps.lock().unwrap().clone()
    }

    fn next_uuid_value(&self) -> String {
        let n = {
            let mut guard = self.next_uuid.lock().expect("uuid mutex poisoned");
            let n = *guard;
            *guard += 1;
            n
        };
        format!("{prefix}{n:012}", prefix = self.fixed_uuid_prefix)
    }
}

impl CfdiPacProvider for MockCfdiPacProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn environment(&self) -> PacEnvironment {
        self.environment
    }

    fn stamp(
        &self,
        request: &CfdiSignRequest,
        target_environment: PacEnvironment,
    ) -> Result<CfdiStampEnvelope, CfdiError> {
        if self.environment != target_environment {
            return Err(CfdiError::EnvironmentMismatch {
                csd: self.environment,
                request: target_environment,
            });
        }
        if request.csd.rfc.is_empty() {
            return Err(CfdiError::CsdInvalid(request.csd.serial_number.clone()));
        }
        let cadena = compute_cadena_original(&request.cfdi_xml, &request.csd.rfc);
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(&request.csd.serial_number),
                payload: cadena.as_bytes().to_vec(),
            })
            .map_err(CfdiError::Signer)?;
        let sello_cfdi = signature.signature_b64.clone();
        let sello_sat = wrap_pac_seal(&sello_cfdi, &self.pac_certificate_serial);
        let uuid = self.next_uuid_value();
        self.stamps.lock().unwrap().push(request.clone());
        Ok(CfdiStampEnvelope {
            signature,
            kind: request.kind,
            uuid,
            cadena_original: cadena,
            sello_cfdi,
            sello_sat,
            pac_certificate_serial: self.pac_certificate_serial.clone(),
            fecha_timbrado: self.fixed_fecha_timbrado.clone(),
        })
    }
}

/// Compute the cadena original string the CFDI seal is
/// produced over.
///
/// The real cadena is an XSLT transform of the canonical XML
/// the SAT publishes (`cadenaoriginal_TFD_1_1.xslt`); the
/// substrate uses a deterministic stand-in (`|RFC|SHA-stub|`)
/// so callers can exercise the surface without an XSLT
/// engine. The real provider swaps this out.
#[must_use]
pub fn compute_cadena_original(cfdi_xml: &[u8], rfc: &str) -> String {
    // Substrate placeholder: collapse to a deterministic
    // string the seal can sign. The real cadena starts with
    // `||1.0|` and walks the CFDI XML in document order.
    // Constants are FNV-1a offset basis + prime.
    let mut digest: u64 = 1_469_598_103_934_665_603;
    for byte in cfdi_xml {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(1_099_511_628_211);
    }
    format!("||4.0|{rfc}|{digest:016x}||")
}

/// Wrap the taxpayer's `selloCFDI` in the PAC's outer `selloSAT`.
///
/// Real PACs sign the inner sello with their own SAT-issued
/// certificate; the substrate produces a deterministic
/// concatenation so callers can verify the envelope shape.
#[must_use]
pub fn wrap_pac_seal(sello_cfdi: &str, pac_certificate_serial: &str) -> String {
    format!("pac:{pac_certificate_serial}:{sello_cfdi}")
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_cfdi::crate_name(),
///     "invoicekit-signer-cfdi"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-cfdi"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use std::sync::Arc;

    fn sample_csd() -> CertificadoSelloDigital {
        CertificadoSelloDigital {
            serial_number: "30001000000400002434".to_owned(),
            rfc: "ACME010101AAA".to_owned(),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-12-31T23:59:59Z".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn build_provider(env: PacEnvironment) -> MockCfdiPacProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key("30001000000400002434", [4_u8; 32]));
        MockCfdiPacProvider::new("test-pac", env, signer, "PAC-SERIAL-1234")
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-cfdi");
    }

    #[test]
    fn cfdi_kind_round_trips_kebab_json() {
        for kind in [
            CfdiKind::Ingreso,
            CfdiKind::Egreso,
            CfdiKind::Traslado,
            CfdiKind::Nomina,
            CfdiKind::Pago,
            CfdiKind::Retencion,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: CfdiKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
            assert!(!kind.slug().is_empty());
        }
    }

    #[test]
    fn compute_cadena_original_is_deterministic() {
        let a = compute_cadena_original(b"<cfdi:Comprobante/>", "ACME010101AAA");
        let b = compute_cadena_original(b"<cfdi:Comprobante/>", "ACME010101AAA");
        assert_eq!(a, b);
        assert!(a.starts_with("||4.0|ACME010101AAA|"));
    }

    #[test]
    fn compute_cadena_changes_with_input() {
        let a = compute_cadena_original(b"<a/>", "ACME010101AAA");
        let b = compute_cadena_original(b"<b/>", "ACME010101AAA");
        assert_ne!(a, b);
    }

    #[test]
    fn stamp_rejects_environment_mismatch() {
        let provider = build_provider(PacEnvironment::Sandbox);
        let err = provider
            .stamp(
                &CfdiSignRequest {
                    cfdi_xml: b"<cfdi:Comprobante/>".to_vec(),
                    csd: sample_csd(),
                    kind: CfdiKind::Ingreso,
                    metadata: BTreeMap::new(),
                },
                PacEnvironment::Production,
            )
            .unwrap_err();
        assert!(matches!(err, CfdiError::EnvironmentMismatch { .. }));
    }

    #[test]
    fn stamp_rejects_empty_rfc() {
        let provider = build_provider(PacEnvironment::Sandbox);
        let mut csd = sample_csd();
        csd.rfc = String::new();
        let err = provider
            .stamp(
                &CfdiSignRequest {
                    cfdi_xml: b"<cfdi:Comprobante/>".to_vec(),
                    csd,
                    kind: CfdiKind::Ingreso,
                    metadata: BTreeMap::new(),
                },
                PacEnvironment::Sandbox,
            )
            .unwrap_err();
        assert!(matches!(err, CfdiError::CsdInvalid(_)));
    }

    #[test]
    fn stamp_produces_envelope_with_pac_wrapped_sello() {
        let provider = build_provider(PacEnvironment::Sandbox)
            .with_fixed_fecha_timbrado("2026-05-28T03:30:00Z");
        let envelope = provider
            .stamp(
                &CfdiSignRequest {
                    cfdi_xml: b"<cfdi:Comprobante/>".to_vec(),
                    csd: sample_csd(),
                    kind: CfdiKind::Ingreso,
                    metadata: BTreeMap::new(),
                },
                PacEnvironment::Sandbox,
            )
            .unwrap();
        assert_eq!(envelope.kind, CfdiKind::Ingreso);
        assert!(envelope.uuid.starts_with("00000000-0000-4000-8000-"));
        assert!(envelope.cadena_original.starts_with("||4.0|ACME010101AAA|"));
        assert_eq!(envelope.pac_certificate_serial, "PAC-SERIAL-1234");
        assert!(envelope.sello_sat.starts_with("pac:PAC-SERIAL-1234:"));
        assert!(envelope.sello_sat.ends_with(&envelope.sello_cfdi));
        assert_eq!(envelope.fecha_timbrado, "2026-05-28T03:30:00Z");
        assert_eq!(provider.stamps().len(), 1);
    }

    #[test]
    fn stamp_increments_uuid_serial_per_provider() {
        let provider = build_provider(PacEnvironment::Sandbox);
        let req = CfdiSignRequest {
            cfdi_xml: b"<x/>".to_vec(),
            csd: sample_csd(),
            kind: CfdiKind::Ingreso,
            metadata: BTreeMap::new(),
        };
        let a = provider.stamp(&req, PacEnvironment::Sandbox).unwrap();
        let b = provider.stamp(&req, PacEnvironment::Sandbox).unwrap();
        assert_ne!(a.uuid, b.uuid);
    }

    #[test]
    fn wrap_pac_seal_round_trips_envelope() {
        let sealed = wrap_pac_seal("ABC123", "SERIAL-42");
        assert_eq!(sealed, "pac:SERIAL-42:ABC123");
    }

    #[test]
    fn pac_environment_slug_and_round_trip() {
        assert_eq!(PacEnvironment::Sandbox.slug(), "sandbox");
        assert_eq!(PacEnvironment::Production.slug(), "production");
        let json = serde_json::to_string(&PacEnvironment::Production).unwrap();
        assert_eq!(json, "\"production\"");
    }
}
