// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! France CTC offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for France and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> Factur-X (EN 16931 CII) XML — France has no national XML,
//!    so it rides the European model rather than a bespoke `report-fr-ctc`
//!    format
//! 3. local validate (structural + French SIREN/SIRET/VAT identity shapes)
//! 4. sign + transmit via the offline `MockFrCtcReportProvider` (composes
//!    `invoicekit-signer-france-ctc` for routing + `invoicekit-signer` for the
//!    qualified-certificate signing leg)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. rejection path: a refusal is a lifecycle status, NOT an `Err`
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). The capability matrix is populated centrally, so this test
//! does NOT assert matrix presence.

use std::collections::BTreeMap;
use std::sync::Arc;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_fr_ctc::{
    to_factur_x_xml, FrCtcEnvironment, FrCtcLifecycle, FrCtcPlatform, FrCtcReceiver, FrCtcReport,
    FrCtcReportProvider, FrCtcReportRequest, MockFrCtcReportProvider, QualifiedCertificate,
    QualifiedCertificateId,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_fr_e2e";
const TRACE: &str = "trace_fr_e2e";
const CERT_SERIAL: &str = "FR-CERT-E2E-0001";
const ISSUER_SIREN: &str = "391838042";
const RECEIVER_SIRET: &str = "55208131700016";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn french_party(name: &str, vat: &str, city: &str, postal: &str) -> Party {
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
            postal_code: postal.to_owned(),
            country: CountryCode::new("FR").unwrap(),
        },
        contact: None,
    }
}

fn french_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-fr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-FR-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: french_party("Acme SAS", "FR40391838042", "Paris", "75001"),
        customer: french_party("Beta SARL", "FR32552081317", "Lyon", "69002"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Conseil & développement logiciel".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // CII / Factur-X uses UN/ECE Rec 20 unit codes (C62), not UBL "EA".
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2000),
            // France's standard TVA rate is 20%.
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
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

fn cert() -> QualifiedCertificate {
    QualifiedCertificate {
        id: QualifiedCertificateId::new("fr-e2e-cert"),
        subject_dn: "CN=Acme SAS,C=FR".to_owned(),
        issuer_dn: "CN=Test QTSP,C=FR".to_owned(),
        serial: CERT_SERIAL.to_owned(),
        not_before: "2026-01-01T00:00:00Z".to_owned(),
        not_after: "2027-01-01T00:00:00Z".to_owned(),
        qualified: true,
    }
}

fn provider(forced: Option<FrCtcLifecycle>) -> MockFrCtcReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [2_u8; 32]));
    let p = MockFrCtcReportProvider::new(signer);
    match forced {
        Some(FrCtcLifecycle::Rejected) => p
            .with_forced_lifecycle(FrCtcLifecycle::Rejected)
            .with_rejection_reason("motif:NOMENCLATURE invalide"),
        Some(other) => p.with_forced_lifecycle(other),
        None => p,
    }
}

fn report_request(factur_x_xml: Vec<u8>) -> FrCtcReportRequest {
    FrCtcReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: FrCtcEnvironment::Piste,
        platform: FrCtcPlatform::Ppf,
        receiver: FrCtcReceiver::Siret(RECEIVER_SIRET.to_owned()),
        issuer_siren: ISSUER_SIREN.to_owned(),
        certificate: cert(),
        factur_x_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> sign/transmit -> evidence bundle.
fn run_lifecycle(forced: Option<FrCtcLifecycle>) -> (Vec<u8>, FrCtcReport) {
    // 1. build
    let doc = french_invoice();

    // 2. serialize -> Factur-X (EN 16931 CII). No national XML for France.
    let factur_x = to_factur_x_xml(&doc).unwrap();

    // 3. local validate (structural): the artifact carries the EN 16931 CII
    //    spine and the EN 16931 guideline URN. Reference Schematron (CIUS-FR)
    //    stays external (JVM).
    for needle in [
        "<rsm:CrossIndustryInvoice",
        "urn:cen.eu:en16931:2017",
        "<ram:GrandTotalAmount>120.00</ram:GrandTotalAmount>",
    ] {
        assert!(factur_x.contains(needle), "Factur-X missing {needle}");
    }

    // 4. sign + transmit (offline mock composing the real CTC routing + signer)
    let report = provider(forced)
        .report(&report_request(factur_x.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national wire XML + signed artifact +
    //    receipt. France's "national" format is Factur-X (formats/factur-x.xml).
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert(
        "formats/factur-x.xml".to_owned(),
        factur_x.into_bytes(),
    );
    // The signed artifact: the transmitted Factur-X bytes plus a sidecar
    // carrying the detached signature receipt.
    artefacts.insert(
        "signed/factur-x.xml".to_owned(),
        report.transmitted_factur_x_xml.clone(),
    );
    artefacts.insert(
        "signed/signature.json".to_owned(),
        serde_json::to_vec(&report.envelope.signature).unwrap(),
    );
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, report)
}

#[test]
fn france_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(None);

    // Happy path: the CTC cycle de vie reached Approved.
    assert!(report.envelope.lifecycle.is_accepted());
    assert_eq!(report.envelope.lifecycle, FrCtcLifecycle::Approved);
    assert!(report.envelope.submission_id.starts_with("PISTE-PPF-"));
    assert!(report.envelope.reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn france_rejection_still_bundles_and_verifies() {
    // A platform/receiver refusal is a lifecycle status (Rejeté), NOT an Err —
    // the audit trail persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(Some(FrCtcLifecycle::Rejected));
    assert_eq!(report.envelope.lifecycle, FrCtcLifecycle::Rejected);
    assert!(report.envelope.lifecycle.is_rejected());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn france_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(None);
    let (b, _) = run_lifecycle(None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}
