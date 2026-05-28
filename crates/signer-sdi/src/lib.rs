// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// SDI / Aruba / FatturaPA / PEC / AdE / IVA acronyms trip
// doc-markdown; suppress crate-wide.
#![allow(clippy::doc_markdown)]
// Doc list-item continuations use 2-space indent throughout
// this crate; the rust-stable list-continuation check wants 4.
#![allow(clippy::doc_lazy_continuation)]

//! `invoicekit-signer-sdi` — Italy SDI (Sistema di
//! Interscambio) signing adapter.
//!
//! Layers the Italy Agenzia delle Entrate FatturaPA / SDI
//! contract on top of [`invoicekit_signer`]. Italian
//! e-invoices must be XAdES-BES-signed by the issuer, then
//! delivered to SDI, which routes the invoice to the buyer
//! and returns one of five receipt types.
//!
//! Public surface:
//!
//! * [`SdiProvider`] — provider trait every SDI integration
//!   implements (Aruba, Infocert, Namirial, ...).
//! * [`SdiTransport`] — `WebService` (REST/SOAP) vs `Pec`
//!   (certified email).
//! * [`SdiReceiptKind`] — RC / NS / MC / MT / NE outcome
//!   classes the AdE returns.
//! * [`SdiStampEnvelope`] — typed envelope: SdI identifier
//!   + receipt kind + transmission progressive + signed
//!   FatturaPA bytes.
//! * [`MockSdiProvider`] — deterministic test provider.

use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// SDI transport mechanism. WebService is the REST/SOAP API
/// AdE exposes; PEC is the certified-email channel still used
/// by some smaller integrators.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdiTransport {
    /// REST / SOAP web-service transport.
    WebService,
    /// PEC (Posta Elettronica Certificata) transport.
    Pec,
}

/// Receipt kinds SDI returns after invoice routing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdiReceiptKind {
    /// RC — Ricevuta di Consegna (invoice delivered to buyer).
    RicevutaConsegna,
    /// NS — Notifica di Scarto (rejected, validation failure).
    NotificaScarto,
    /// MC — Mancata Consegna (delivery to buyer failed).
    MancataConsegna,
    /// NE — Notifica Esito (buyer accept / reject).
    NotificaEsito,
    /// MT — Metadata only (informational).
    Metadata,
}

impl SdiReceiptKind {
    /// True when the receipt indicates successful delivery.
    #[must_use]
    pub const fn is_delivered(self) -> bool {
        matches!(self, Self::RicevutaConsegna)
    }
}

/// Aruba (or other QTSP) qualified-certificate reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArubaQualifiedCertificate {
    /// Certificate serial number.
    pub serial_number: String,
    /// Codice fiscale (Italian tax id) the cert is bound to.
    pub codice_fiscale: String,
    /// Subject distinguished name.
    pub subject_dn: String,
    /// PEM-encoded certificate bytes (opaque on substrate).
    pub certificate_pem: Vec<u8>,
}

/// Typed SDI stamp envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiStampEnvelope {
    /// Underlying [`Signer`] receipt — the XAdES-BES
    /// signature over the FatturaPA XML.
    pub signature: Signature,
    /// SDI identifier (`IdentificativoSdI`) — the routing id
    /// SDI assigns post-acceptance.
    pub identificativo_sdi: String,
    /// Receipt kind SDI returned.
    pub receipt_kind: SdiReceiptKind,
    /// Progressive transmission number (`ProgressivoInvio`),
    /// up to 5 alphanumeric chars.
    pub progressivo_invio: String,
    /// Signed FatturaPA XML bytes (XAdES wrapped).
    pub signed_fattura_xml: Vec<u8>,
    /// Transport that delivered the invoice.
    pub transport: SdiTransport,
}

/// Submission request shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiSubmitRequest {
    /// FatturaPA XML bytes (canonical).
    pub fattura_xml: Vec<u8>,
    /// Qualified certificate used to sign.
    pub certificate: ArubaQualifiedCertificate,
    /// Transport channel.
    pub transport: SdiTransport,
    /// Progressive transmission number assigned by the issuer
    /// (1..=99999).
    pub progressivo_invio: String,
}

/// Errors raised by [`SdiProvider`] implementations.
#[derive(Debug, Error)]
pub enum SdiError {
    /// Underlying signer refused.
    #[error("sdi provider's signer refused: {0}")]
    Signer(SigningError),
    /// Codice fiscale on the certificate did not match the
    /// invoice.
    #[error("SDI codice fiscale mismatch: certificate={cert}")]
    CodiceFiscaleMismatch {
        /// Codice fiscale on the certificate.
        cert: String,
    },
    /// SDI rejected the FatturaPA schema or business rules
    /// (NS receipt).
    #[error("SDI rejected the invoice: {0}")]
    InvoiceRejected(String),
    /// SDI portal is unreachable.
    #[error("SDI portal unavailable: {0}")]
    Unavailable(String),
}

/// SDI provider surface.
pub trait SdiProvider: Send + Sync {
    /// Provider display name (e.g. `aruba`, `infocert`).
    fn provider_name(&self) -> &str;

    /// Submit a FatturaPA invoice to SDI.
    ///
    /// # Errors
    ///
    /// Returns [`SdiError`] when the codice fiscale
    /// mismatches, SDI rejects the invoice, the signer
    /// refuses, or the portal is unreachable.
    fn submit(&self, request: &SdiSubmitRequest) -> Result<SdiStampEnvelope, SdiError>;
}

/// Mock SDI provider.
pub struct MockSdiProvider {
    name: String,
    signer: std::sync::Arc<dyn Signer>,
    forced_receipt: SdiReceiptKind,
    submissions: Mutex<Vec<SdiSubmitRequest>>,
    next_sdi_id: Mutex<u64>,
}

impl MockSdiProvider {
    /// Build a mock SDI provider.
    #[must_use]
    pub fn new(name: impl Into<String>, signer: std::sync::Arc<dyn Signer>) -> Self {
        Self {
            name: name.into(),
            signer,
            forced_receipt: SdiReceiptKind::RicevutaConsegna,
            submissions: Mutex::new(Vec::new()),
            next_sdi_id: Mutex::new(1),
        }
    }

    /// Force the provider to return a specific receipt kind
    /// on every submit.
    #[must_use]
    pub fn with_forced_receipt(mut self, receipt: SdiReceiptKind) -> Self {
        self.forced_receipt = receipt;
        self
    }

    /// Snapshot of recorded submissions.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn submissions(&self) -> Vec<SdiSubmitRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

impl SdiProvider for MockSdiProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn submit(&self, request: &SdiSubmitRequest) -> Result<SdiStampEnvelope, SdiError> {
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(&request.certificate.serial_number),
                payload: request.fattura_xml.clone(),
            })
            .map_err(SdiError::Signer)?;
        let id = {
            let mut g = self.next_sdi_id.lock().expect("mutex poisoned");
            let n = *g;
            *g += 1;
            n
        };
        let identificativo_sdi = format!("IT{id:013}");
        // Mock signed envelope: prepend a `<XAdES-stub>` tag
        // around the canonical FatturaPA bytes so callers can
        // verify the envelope shape.
        let mut signed_fattura_xml: Vec<u8> = b"<XAdES-stub>".to_vec();
        signed_fattura_xml.extend_from_slice(&request.fattura_xml);
        signed_fattura_xml.extend_from_slice(b"</XAdES-stub>");
        self.submissions.lock().unwrap().push(request.clone());
        Ok(SdiStampEnvelope {
            signature,
            identificativo_sdi,
            receipt_kind: self.forced_receipt,
            progressivo_invio: request.progressivo_invio.clone(),
            signed_fattura_xml,
            transport: request.transport,
        })
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_sdi::crate_name(),
///     "invoicekit-signer-sdi"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-sdi"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use std::sync::Arc;

    fn sample_cert() -> ArubaQualifiedCertificate {
        ArubaQualifiedCertificate {
            serial_number: "1234567890ABCDEF".to_owned(),
            codice_fiscale: "RSSMRA80A01H501U".to_owned(),
            subject_dn: "CN=Mario Rossi,O=Acme SRL,C=IT".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn build_provider() -> MockSdiProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key("1234567890ABCDEF", [2_u8; 32]));
        MockSdiProvider::new("aruba-test", signer)
    }

    fn sample_request() -> SdiSubmitRequest {
        SdiSubmitRequest {
            fattura_xml: b"<FatturaElettronica/>".to_vec(),
            certificate: sample_cert(),
            transport: SdiTransport::WebService,
            progressivo_invio: "ABCDE".to_owned(),
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-sdi");
    }

    #[test]
    fn receipt_kind_round_trips_kebab_json() {
        for kind in [
            SdiReceiptKind::RicevutaConsegna,
            SdiReceiptKind::NotificaScarto,
            SdiReceiptKind::MancataConsegna,
            SdiReceiptKind::NotificaEsito,
            SdiReceiptKind::Metadata,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: SdiReceiptKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn receipt_kind_delivered_predicate() {
        assert!(SdiReceiptKind::RicevutaConsegna.is_delivered());
        assert!(!SdiReceiptKind::NotificaScarto.is_delivered());
        assert!(!SdiReceiptKind::MancataConsegna.is_delivered());
        assert!(!SdiReceiptKind::NotificaEsito.is_delivered());
        assert!(!SdiReceiptKind::Metadata.is_delivered());
    }

    #[test]
    fn transport_round_trips_kebab_json() {
        let json = serde_json::to_string(&SdiTransport::WebService).unwrap();
        assert_eq!(json, "\"web-service\"");
        let back: SdiTransport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SdiTransport::WebService);
    }

    #[test]
    fn submit_produces_envelope_with_identificativo_sdi() {
        let provider = build_provider();
        let envelope = provider.submit(&sample_request()).unwrap();
        assert!(envelope.identificativo_sdi.starts_with("IT"));
        assert_eq!(envelope.receipt_kind, SdiReceiptKind::RicevutaConsegna);
        assert!(envelope.receipt_kind.is_delivered());
        assert_eq!(envelope.progressivo_invio, "ABCDE");
        assert_eq!(envelope.transport, SdiTransport::WebService);
        assert!(envelope.signed_fattura_xml.starts_with(b"<XAdES-stub>"));
        assert_eq!(provider.submissions().len(), 1);
    }

    #[test]
    fn submit_increments_identificativo_per_provider() {
        let provider = build_provider();
        let a = provider.submit(&sample_request()).unwrap();
        let b = provider.submit(&sample_request()).unwrap();
        assert_ne!(a.identificativo_sdi, b.identificativo_sdi);
    }

    #[test]
    fn submit_propagates_forced_receipt() {
        let provider = build_provider().with_forced_receipt(SdiReceiptKind::NotificaScarto);
        let envelope = provider.submit(&sample_request()).unwrap();
        assert_eq!(envelope.receipt_kind, SdiReceiptKind::NotificaScarto);
        assert!(!envelope.receipt_kind.is_delivered());
    }
}
