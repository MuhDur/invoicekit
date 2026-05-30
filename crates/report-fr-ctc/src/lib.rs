// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// CTC / PPF / PDP / DGFiP / SIREN / SIRET / Chorus / AIFE / eIDAS / XAdES
// acronyms trip the doc-markdown lint; none are rust items.
#![allow(clippy::doc_markdown)]

//! France **CTC** (Continuous Transaction Control) report adapter for the
//! Chorus Pro / **PPF** (Portail Public de Facturation) reform.
//!
//! Unlike Italy, France has **no unique national XML**. The 2026+ e-invoicing
//! and e-reporting mandate (DGFiP "Spécifications Externes Facture
//! Électronique B2B") rides the European **EN 16931** semantic model, carried
//! on the wire as **Factur-X** (the hybrid CII syntax) or UBL. This crate
//! therefore *reuses* the existing Factur-X projection rather than minting a
//! new serializer, then drives the CTC lifecycle:
//!
//! 1. **serialize** — [`to_factur_x_xml`] projects an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] to deterministic Factur-X (EN 16931
//!    CII) XML by delegating to
//!    [`invoicekit_profile_factur_x::to_factur_x_cii_xml`]; there is no
//!    bespoke `report-fr-ctc` XML format.
//! 2. **validate (local)** — [`validate_siren`], [`validate_siret`], and
//!    [`validate_french_vat`] enforce the real French identifier shapes;
//!    reference-grade EN 16931 / CIUS-FR Schematron stays an external (JVM)
//!    backend and is labelled as such in the capability matrix.
//! 3. **sign + transmit** — [`MockFrCtcReportProvider`] composes the
//!    already-built [`invoicekit_signer_france_ctc::MockFrCtcProvider`] so the
//!    CTC routing / submission-id synthesis is exercised, never re-faked, and
//!    composes [`invoicekit_signer::Signer`] for the detached signature over
//!    the Factur-X bytes (the qualified-certificate signing leg).
//! 4. **evidence** — the caller bundles the canonical document, Factur-X XML,
//!    signed artifact, and receipt into a signed `.ikb` evidence bundle.
//!
//! Live PPF/PDP transmission (Chorus Pro web-service or an accredited PDP API)
//! is bring-your-own-credentials and lands in a follow-up `report-fr-ctc-http`
//! crate; this crate's `Mock*` providers are deterministic and offline.
//!
//! **Rejection is not an error.** When a platform or the receiver refuses an
//! invoice the CTC "cycle de vie" records a `Rejeté` (rejected) lifecycle
//! status — surfaced here as [`FrCtcLifecycle::Rejected`] inside an `Ok(_)`
//! envelope, never as `Err`. `Err` is reserved for pre-wire shape failures and
//! transport/TLS/DNS faults.

use std::sync::Arc;

use invoicekit_ir::CommercialDocument;
use invoicekit_profile_factur_x::{to_factur_x_cii_xml, FacturXProfile};
use invoicekit_signer::{KeyRef, SignRequest, Signer};
use invoicekit_signer_france_ctc::{
    FrCtcProvider, FrCtcStatus, FrCtcSubmitRequest, MockFrCtcProvider,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the signing + routing substrate types this crate's public API
// surfaces, so downstream callers need not depend on the signer crates
// directly. `FrCtcPlatform` / `FrCtcReceiver` are reused verbatim from the
// signer crate (they carry the real PPF/PDP routing shape).
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_eidas::{QualifiedCertificate, QualifiedCertificateId};
pub use invoicekit_signer_france_ctc::{FrCtcPlatform, FrCtcReceiver};

// ---------------------------------------------------------------------------
// Factur-X serialization (IR -> EN 16931 CII, the French wire format)
// ---------------------------------------------------------------------------

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic Factur-X
/// (EN 16931 profile, CII syntax) XML — the French CTC wire format.
///
/// France has no national XML schema: the CTC mandate carries the European
/// EN 16931 semantic model, so this delegates to
/// [`invoicekit_profile_factur_x::to_factur_x_cii_xml`] at the
/// [`FacturXProfile::En16931`] profile rather than inventing a new format.
/// Output is byte-stable because the underlying CII serializer canonicalizes.
///
/// # Errors
///
/// Returns [`invoicekit_profile_factur_x::FacturXError`] when the projection
/// or the CII serializer rejects the document (e.g. an unsupported document
/// type for the CII syntax).
pub fn to_factur_x_xml(
    document: &CommercialDocument,
) -> Result<String, invoicekit_profile_factur_x::FacturXError> {
    to_factur_x_cii_xml(document, FacturXProfile::En16931)
}

// ---------------------------------------------------------------------------
// French identifier validators (load-bearing country-specific content)
// ---------------------------------------------------------------------------

/// Validate a **SIREN**: exactly 9 ASCII digits (the legal-entity registration
/// number assigned by INSEE).
///
/// # Errors
///
/// Returns [`FrCtcReportError::BadIdentifier`] when the input is not 9 ASCII
/// digits.
pub fn validate_siren(siren: &str) -> Result<(), FrCtcReportError> {
    if siren.len() == 9 && siren.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(FrCtcReportError::BadIdentifier(format!(
            "SIREN must be 9 ASCII digits, got {siren:?}"
        )))
    }
}

/// Validate a **SIRET**: exactly 14 ASCII digits, whose first 9 are a valid
/// SIREN (a SIRET is a SIREN plus a 5-digit establishment suffix, the NIC).
///
/// # Errors
///
/// Returns [`FrCtcReportError::BadIdentifier`] when the input is not 14 ASCII
/// digits with a SIREN-shaped prefix.
pub fn validate_siret(siret: &str) -> Result<(), FrCtcReportError> {
    if siret.len() != 14 || !siret.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FrCtcReportError::BadIdentifier(format!(
            "SIRET must be 14 ASCII digits, got {siret:?}"
        )));
    }
    // The first 9 digits must themselves form a valid SIREN.
    validate_siren(&siret[..9]).map_err(|_| {
        FrCtcReportError::BadIdentifier(format!(
            "SIRET prefix is not a valid SIREN, got {siret:?}"
        ))
    })
}

/// Validate a **French VAT** number (TVA intracommunautaire): `FR`, then a
/// 2-character control key (digits and/or uppercase letters), then a 9-digit
/// SIREN — 13 characters total.
///
/// # Errors
///
/// Returns [`FrCtcReportError::BadIdentifier`] when the input does not match
/// the `FR` + 2-key + 9-digit-SIREN shape.
pub fn validate_french_vat(vat: &str) -> Result<(), FrCtcReportError> {
    let bad = |reason: &str| {
        Err(FrCtcReportError::BadIdentifier(format!(
            "French VAT must be FR + 2 key chars + 9-digit SIREN ({reason}), got {vat:?}"
        )))
    };
    if vat.len() != 13 {
        return bad("wrong length");
    }
    let bytes = vat.as_bytes();
    if !vat.starts_with("FR") {
        return bad("missing FR prefix");
    }
    // The 2-char control key: ASCII digits or uppercase letters (the DGFiP
    // "clé" can be alphanumeric).
    if !bytes[2..4]
        .iter()
        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    {
        return bad("bad control key");
    }
    if validate_siren(&vat[4..]).is_err() {
        return bad("trailing SIREN invalid");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CTC report adapter (validate -> sign -> transmit -> typed receipt)
// ---------------------------------------------------------------------------

/// The CTC lifecycle verdict recorded on a report receipt.
///
/// Mirrors the DGFiP "cycle de vie" the platform observes. This is the
/// audit-relevant projection of [`invoicekit_signer_france_ctc::FrCtcStatus`]
/// onto the subset a *report* surfaces, with [`FrCtcLifecycle::Rejected`] as a
/// first-class verdict (a refusal is a status, **not** an `Err`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcLifecycle {
    /// Platform accepted the deposit; awaiting routing to the receiver
    /// (`Déposée`).
    Deposited,
    /// Receiver platform / inbox confirmed receipt (`Reçue`).
    Received,
    /// Receiver accepted the invoice — legal validation done (`Approuvée`).
    Approved,
    /// Receiver or platform refused the invoice with a typed motif de rejet
    /// (`Rejetée`). A refusal is a verdict, never an error.
    Rejected,
}

impl FrCtcLifecycle {
    /// True when the invoice reached an accepted terminal state (`Approved`).
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Approved)
    }

    /// True when the platform/receiver refused the invoice (`Rejected`).
    #[must_use]
    pub const fn is_rejected(self) -> bool {
        matches!(self, Self::Rejected)
    }

    /// Project a signer-layer [`FrCtcStatus`] onto the report lifecycle.
    fn from_signer_status(status: FrCtcStatus) -> Self {
        match status {
            // `Submitted`/`Deposited` are both pre-routing intake states; a
            // report only ever surfaces the deposited verdict for them.
            FrCtcStatus::Submitted | FrCtcStatus::Deposited | FrCtcStatus::Suspended => {
                Self::Deposited
            }
            FrCtcStatus::Received => Self::Received,
            FrCtcStatus::Approved => Self::Approved,
            FrCtcStatus::Rejected => Self::Rejected,
        }
    }
}

/// CTC runtime environment selector (mirrors the signer crate's tiers).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcEnvironment {
    /// PISTE / sandbox tier (`piste.gouv.fr`).
    Piste,
    /// Production tier.
    Production,
}

impl FrCtcEnvironment {
    /// Convert to the signer-layer environment selector.
    const fn to_signer(self) -> invoicekit_signer_france_ctc::FrCtcEnvironment {
        match self {
            Self::Piste => invoicekit_signer_france_ctc::FrCtcEnvironment::Piste,
            Self::Production => invoicekit_signer_france_ctc::FrCtcEnvironment::Production,
        }
    }
}

/// Operator-facing CTC report request. The Factur-X XML is produced upstream
/// by [`to_factur_x_xml`]; this request carries it plus the French identity
/// fields the CTC mandate needs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrCtcReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: FrCtcEnvironment,
    /// Routing platform — the public PPF or a private accredited PDP.
    pub platform: FrCtcPlatform,
    /// Receiver lookup key (SIRET / SIREN / Annuaire id).
    pub receiver: FrCtcReceiver,
    /// Issuer SIREN (9 digits) — the legal entity emitting the invoice.
    pub issuer_siren: String,
    /// Qualified (eIDAS) certificate used to sign the Factur-X payload.
    pub certificate: QualifiedCertificate,
    /// Canonical Factur-X (EN 16931 CII) XML bytes.
    pub factur_x_xml: Vec<u8>,
}

/// Typed CTC receipt (the audit-relevant verdict and signature metadata).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrCtcReportEnvelope {
    /// Platform-assigned submission id (Chorus Pro / PPF / PDP routing id).
    pub submission_id: String,
    /// CTC lifecycle verdict the platform recorded.
    pub lifecycle: FrCtcLifecycle,
    /// RFC-3339 UTC timestamp the platform recorded.
    pub recorded_at: String,
    /// Detached signature over the Factur-X bytes (the qualified-certificate
    /// signing leg). Kept in the receipt as audit metadata.
    pub signature: Signature,
    /// Motif de rejet text when `lifecycle` is [`FrCtcLifecycle::Rejected`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The full result of a report: the receipt plus the signed Factur-X artifact
/// (the latter is an evidence-bundle artefact, kept out of the receipt JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrCtcReport {
    /// Audit receipt.
    pub envelope: FrCtcReportEnvelope,
    /// The Factur-X XML bytes that were signed and transmitted.
    pub transmitted_factur_x_xml: Vec<u8>,
}

/// Typed CTC report errors. Three buckets: payload shape, country-id shape,
/// and transport. A rejection verdict is **not** here — it is an `Ok`
/// envelope with [`FrCtcLifecycle::Rejected`].
#[derive(Debug, Error)]
pub enum FrCtcReportError {
    /// The Factur-X payload failed shape validation before the wire.
    #[error("factur-x xml rejected: {0}")]
    BadXml(String),
    /// A French identifier (SIREN / SIRET / VAT) did not match its shape.
    #[error("invalid french identifier: {0}")]
    BadIdentifier(String),
    /// The qualified-certificate signing leg failed.
    #[error("signing failure: {0}")]
    Signing(String),
    /// The CTC platform signer/transport failed on the wire.
    #[error("ctc signer/transport failure: {0}")]
    Transport(String),
}

/// The CTC report surface every integration (PPF, an accredited PDP, ...)
/// implements.
pub trait FrCtcReportProvider: Send + Sync {
    /// Validate the issuer identity, sign the Factur-X, transmit to the CTC
    /// platform, and return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`FrCtcReportError`] on pre-wire shape failures (bad identifier,
    /// empty payload, signing failure) or transport faults. A platform/receiver
    /// refusal is surfaced as an `Ok` envelope with
    /// [`FrCtcLifecycle::Rejected`], not an error.
    fn report(&self, request: &FrCtcReportRequest) -> Result<FrCtcReport, FrCtcReportError>;
}

/// Deterministic offline CTC report provider.
///
/// Composes [`invoicekit_signer_france_ctc::MockFrCtcProvider`] so the real CTC
/// routing + submission-id synthesis is exercised rather than re-implemented,
/// and an [`invoicekit_signer::Signer`] for the detached signature over the
/// Factur-X bytes (keyed by the certificate serial).
pub struct MockFrCtcReportProvider {
    signer: Arc<dyn Signer>,
    forced_lifecycle: Option<FrCtcLifecycle>,
    forced_reason: Option<String>,
    fixed_recorded_at: String,
}

impl MockFrCtcReportProvider {
    /// Build a mock report provider over the given signer (key it by the
    /// certificate serial number, e.g.
    /// `SoftwareSigner::new().with_key(serial, [2u8; 32])`).
    ///
    /// By default the happy path resolves to [`FrCtcLifecycle::Approved`]
    /// (the platform deposited and the receiver approved).
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self {
            signer,
            forced_lifecycle: None,
            forced_reason: None,
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every report to resolve to a specific lifecycle verdict (e.g.
    /// [`FrCtcLifecycle::Rejected`] to exercise the refusal path). When the
    /// forced verdict is `Rejected` a default motif de rejet is attached
    /// unless overridden with [`Self::with_rejection_reason`].
    #[must_use]
    pub fn with_forced_lifecycle(mut self, lifecycle: FrCtcLifecycle) -> Self {
        self.forced_lifecycle = Some(lifecycle);
        self
    }

    /// Attach an explicit motif de rejet (used when the forced lifecycle is a
    /// rejection).
    #[must_use]
    pub fn with_rejection_reason(mut self, reason: impl Into<String>) -> Self {
        self.forced_reason = Some(reason.into());
        self
    }
}

impl FrCtcReportProvider for MockFrCtcReportProvider {
    fn report(&self, request: &FrCtcReportRequest) -> Result<FrCtcReport, FrCtcReportError> {
        // 1. Country-specific pre-wire validation (anti-slop): the issuer SIREN
        //    and (where present) the routing SIRET must be shape-valid.
        validate_siren(&request.issuer_siren)?;
        if let FrCtcReceiver::Siret(siret) = &request.receiver {
            validate_siret(siret)?;
        }
        if let FrCtcReceiver::Siren(siren) = &request.receiver {
            validate_siren(siren)?;
        }
        if request.factur_x_xml.is_empty() {
            return Err(FrCtcReportError::BadXml("payload is empty".to_owned()));
        }

        // 2. Sign the Factur-X bytes (the qualified-certificate signing leg).
        //    The signer is keyed by the certificate serial, mirroring the SDI
        //    pattern.
        let signature = self
            .signer
            .sign(&SignRequest {
                payload: request.factur_x_xml.clone(),
                key_ref: KeyRef::new(request.certificate.serial.clone()),
            })
            .map_err(|e| FrCtcReportError::Signing(e.to_string()))?;

        // 3. Compose the signer crate's CTC provider to route the submission
        //    and synthesize the platform submission id.
        let inner = MockFrCtcProvider::with_fixed_stamped_at(self.fixed_recorded_at.clone());
        let stamp = inner
            .submit(
                &request.certificate,
                &FrCtcSubmitRequest {
                    tenant_id: request.tenant_id.clone(),
                    environment: request.environment.to_signer(),
                    platform: request.platform.clone(),
                    receiver: request.receiver.clone(),
                    xml: request.factur_x_xml.clone(),
                },
            )
            .map_err(|e| FrCtcReportError::Transport(e.to_string()))?;

        // 4. Resolve the lifecycle verdict. The signer mock deposits on
        //    submit; the report layer advances the deposited intake to its
        //    terminal verdict (default Approved) or applies a forced verdict
        //    (e.g. Rejected) — a refusal is a verdict, NOT an Err.
        let lifecycle = self.forced_lifecycle.unwrap_or_else(|| {
            // A fresh deposit advances to Approved on the happy path; map any
            // unexpected signer status faithfully.
            match FrCtcLifecycle::from_signer_status(stamp.status) {
                FrCtcLifecycle::Deposited => FrCtcLifecycle::Approved,
                other => other,
            }
        });
        let reason = if lifecycle.is_rejected() {
            Some(self.forced_reason.clone().unwrap_or_else(|| {
                "platform rejected the invoice (motif de rejet)".to_owned()
            }))
        } else {
            None
        };

        Ok(FrCtcReport {
            envelope: FrCtcReportEnvelope {
                submission_id: stamp.submission_id,
                lifecycle,
                recorded_at: self.fixed_recorded_at.clone(),
                signature,
                reason,
            },
            transmitted_factur_x_xml: request.factur_x_xml.clone(),
        })
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_report_fr_ctc::crate_name(), "invoicekit-report-fr-ctc");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-fr-ctc"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
    };
    use invoicekit_signer::SoftwareSigner;
    use rust_decimal::Decimal;

    const CERT_SERIAL: &str = "FR-CERT-0001";

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn french_party(name: &str, vat: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: vat.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["Rue de Rivoli 1".to_owned()],
                city: city.to_owned(),
                subdivision: None,
                postal_code: "75001".to_owned(),
                country: CountryCode::new("FR").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-fr-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("INV-2026-FR-0001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: french_party("Acme SAS", "FR40391838042", "Paris"),
            customer: french_party("Beta SARL", "FR32552081317", "Lyon"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Conseil & développement".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                // CII / Factur-X uses UN/ECE Rec 20 unit codes (C62, not EA).
                unit_code: Some("C62".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2000),
                tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(10000),
                tax_exclusive_amount: amt(10000),
                tax_inclusive_amount: amt(12000),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(12000),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant_fr".to_owned(),
                trace_id: "trace_fr".to_owned(),
                source_system: Some("unit".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_cert() -> QualifiedCertificate {
        QualifiedCertificate {
            id: QualifiedCertificateId::new("fr-mock-cert"),
            subject_dn: "CN=Acme SAS, C=FR".to_owned(),
            issuer_dn: "CN=Test QTSP, C=FR".to_owned(),
            serial: CERT_SERIAL.to_owned(),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-01-01T00:00:00Z".to_owned(),
            qualified: true,
        }
    }

    fn provider() -> MockFrCtcReportProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [2_u8; 32]));
        MockFrCtcReportProvider::new(signer)
    }

    fn sample_request(factur_x_xml: Vec<u8>) -> FrCtcReportRequest {
        FrCtcReportRequest {
            tenant_id: "tenant_fr".to_owned(),
            environment: FrCtcEnvironment::Piste,
            platform: FrCtcPlatform::Ppf,
            receiver: FrCtcReceiver::Siret("39183804200017".to_owned()),
            issuer_siren: "391838042".to_owned(),
            certificate: sample_cert(),
            factur_x_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-fr-ctc");
    }

    #[test]
    fn factur_x_is_en16931_cii_and_deterministic() {
        let doc = sample_invoice();
        let xml = to_factur_x_xml(&doc).unwrap();
        // France rides EN 16931 via Factur-X (CII), not a national format.
        assert!(xml.contains("<rsm:CrossIndustryInvoice"), "not CII:\n{xml}");
        assert!(xml.contains("urn:cen.eu:en16931:2017"), "not EN 16931:\n{xml}");
        assert!(xml.contains("<ram:GrandTotalAmount>120.00</ram:GrandTotalAmount>"));
        assert_eq!(xml, to_factur_x_xml(&doc).unwrap(), "must be byte-stable");
    }

    #[test]
    fn report_happy_path_is_approved() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let report = provider().report(&sample_request(xml)).unwrap();
        assert!(report.envelope.lifecycle.is_accepted());
        assert!(!report.envelope.lifecycle.is_rejected());
        assert!(report.envelope.submission_id.starts_with("PISTE-PPF-"));
        assert!(report.envelope.reason.is_none());
        assert_eq!(report.envelope.signature.algorithm, "blake3-keyed-256");
    }

    #[test]
    fn report_rejection_is_ok_not_err() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let provider = provider()
            .with_forced_lifecycle(FrCtcLifecycle::Rejected)
            .with_rejection_reason("motif:NOMENCLATURE invalide");
        let report = provider.report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.lifecycle, FrCtcLifecycle::Rejected);
        assert!(report.envelope.lifecycle.is_rejected());
        assert!(report.envelope.reason.is_some());
    }

    #[test]
    fn report_routes_through_pdp_with_partner_siret() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let mut req = sample_request(xml);
        req.platform = FrCtcPlatform::Pdp {
            siret: "73282932000074".to_owned(),
        };
        req.environment = FrCtcEnvironment::Production;
        let report = provider().report(&req).unwrap();
        assert!(report.envelope.submission_id.starts_with("PDP-"));
    }

    #[test]
    fn report_rejects_bad_issuer_siren() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let mut req = sample_request(xml);
        req.issuer_siren = "12345".to_owned();
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            FrCtcReportError::BadIdentifier(_)
        ));
    }

    #[test]
    fn report_rejects_bad_receiver_siret() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let mut req = sample_request(xml);
        req.receiver = FrCtcReceiver::Siret("not-a-siret".to_owned());
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            FrCtcReportError::BadIdentifier(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new());
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            FrCtcReportError::BadXml(_)
        ));
    }

    #[test]
    fn siren_validator_shapes() {
        assert!(validate_siren("391838042").is_ok());
        assert!(validate_siren("12345678").is_err()); // 8 digits
        assert!(validate_siren("1234567890").is_err()); // 10 digits
        assert!(validate_siren("12345678A").is_err()); // non-digit
    }

    #[test]
    fn siret_validator_shapes() {
        assert!(validate_siret("39183804200017").is_ok());
        assert!(validate_siret("3918380420001").is_err()); // 13 digits
        assert!(validate_siret("391838042000170").is_err()); // 15 digits
        assert!(validate_siret("3918380420001A").is_err()); // non-digit
    }

    #[test]
    fn french_vat_validator_shapes() {
        assert!(validate_french_vat("FR40391838042").is_ok()); // numeric key
        assert!(validate_french_vat("FRXX391838042").is_ok()); // alpha key
        assert!(validate_french_vat("DE40391838042").is_err()); // wrong country
        assert!(validate_french_vat("FR4039183804").is_err()); // short SIREN
        assert!(validate_french_vat("FR40x91838042").is_err()); // bad SIREN char
        assert!(validate_french_vat("FRxx391838042").is_err()); // lowercase key
    }

    #[test]
    fn lifecycle_round_trips_through_serde() {
        for lc in [
            FrCtcLifecycle::Deposited,
            FrCtcLifecycle::Received,
            FrCtcLifecycle::Approved,
            FrCtcLifecycle::Rejected,
        ] {
            let json = serde_json::to_string(&lc).unwrap();
            let back: FrCtcLifecycle = serde_json::from_str(&json).unwrap();
            assert_eq!(back, lc);
        }
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_factur_x_xml(&sample_invoice()).unwrap().into_bytes();
        let env = provider().report(&sample_request(xml)).unwrap().envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: FrCtcReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }
}
