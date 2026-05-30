// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// CTC / DGFiP / PPF / PDP / SIREN / SIRET / UNCL / TVA / CGI acronyms in the
// scenario doc-comments trip the doc-markdown lint; none are Rust items. This
// mirrors the same allow on the crate's `src/lib.rs`.
#![allow(clippy::doc_markdown)]

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
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2000),
            // France's standard TVA rate is 20%.
            tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
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

    // 2-5. serialize -> Factur-X, sign + transmit, and assemble the `.ikb`
    //      bundle. The artefact wiring is shared with every other scenario via
    //      `bundle_document` (France's "national" format is Factur-X, written
    //      to formats/factur-x.xml).
    let (ikb, report, factur_x) = bundle_document(&doc, forced);

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

// ---------------------------------------------------------------------------
// Deepened, country-specific coverage.
//
// Grounding: France's CTC mandate carries the European EN 16931 semantic
// model on the wire as Factur-X (hybrid CII). The values asserted below are
// fixed by:
//
//   * Direction Générale des Finances Publiques (DGFiP) — "Spécifications
//     Externes B2B pour la facturation électronique", v3.0 (the document type
//     "facture / avoir", the "cycle de vie" lifecycle, and the "motif de
//     rejet" vocabulary). Hub: https://www.impots.gouv.fr/facturation-electronique
//   * EN 16931-1:2017 — semantic data model. BT-3 document type code
//     (UNCL1001: 380 = invoice, 381 = credit note), BT-118/BT-151 VAT category
//     code (UNCL5305: S, AA, Z, E, AE), BT-117/BT-116 tax breakdown amounts.
//   * Factur-X 1.0.07 / EN 16931 (CII) syntax binding — the CrossIndustryInvoice
//     element names (ram:TypeCode, ram:CategoryCode, ram:CalculatedAmount,
//     ram:GrandTotalAmount, ram:DuePayableAmount) the values below assert on.
//     Spec hub: https://fnfe-mpe.org/factur-x/
//
// Fixtures are hand-built and synthetic (no vendored regulator files).
// ---------------------------------------------------------------------------

/// Bundle steps 4-5 for an *arbitrary* already-built French CTC document:
/// serialize -> Factur-X, sign + transmit (offline), assemble + verify a
/// `.ikb` evidence bundle. Returns the packed bundle and the report.
fn bundle_document(
    doc: &CommercialDocument,
    forced: Option<FrCtcLifecycle>,
) -> (Vec<u8>, FrCtcReport, String) {
    let factur_x = to_factur_x_xml(doc).unwrap();
    let report = provider(forced)
        .report(&report_request(factur_x.clone().into_bytes()))
        .unwrap();

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/factur-x.xml".to_owned(), factur_x.clone().into_bytes());
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
    (ikb, report, factur_x)
}

/// A French B2B **avoir** (credit note). EN 16931 BT-3 carries UNCL1001 code
/// `381`, which the Factur-X CII binding emits as `<ram:TypeCode>381`. The
/// DGFiP spec treats an avoir as a first-class lifecycle document, distinct
/// from a facture (`380`).
fn french_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-fr-avoir-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-06-02").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-07-02").unwrap()),
        document_number: DocumentNumber::new("AV-2026-FR-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: french_party("Acme SAS", "FR40391838042", "Paris", "75001"),
        customer: french_party("Beta SARL", "FR32552081317", "Lyon", "69002"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Remboursement prestation de conseil".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(1000),
            // France's standard TVA rate is 20%.
            tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(6000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(6000),
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

/// A multi-line French invoice mixing the **20% standard** rate (TVA normale,
/// UNCL5305 category `S`) and the **5.5% reduced** rate (taux réduit, also
/// category `S` at a different rate per the French TVA schedule, CGI art. 278-0
/// bis). Two lines, two tax-breakdown groups.
fn french_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-fr-multiline-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-FR-ML-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: french_party("Acme SAS", "FR40391838042", "Paris", "75001"),
        customer: french_party("Beta SARL", "FR32552081317", "Lyon", "69002"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Prestation de conseil (TVA 20%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Livre technique (TVA 5,5%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(4)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(10000),
                tax_category: Some("AA".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2000),
                tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "AA".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(550),
                tax_rate: Some(DecimalValue::new(Decimal::new(550, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            // 100.00 + 100.00 lines
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            // 200.00 + 20.00 + 5.50 tax = 225.50
            tax_inclusive_amount: amt(22550),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(22550),
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

/// A French B2B **autoliquidation** (reverse charge) invoice. Per EN 16931 the
/// VAT category is UNCL5305 `AE` with a 0% applicable rate and no VAT amount;
/// the tax liability shifts to the buyer (CGI art. 283-2). The grand total then
/// equals the net (no VAT added).
fn french_reverse_charge_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-fr-autoliq-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-FR-RC-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: french_party("Acme SAS", "FR40391838042", "Paris", "75001"),
        customer: french_party("Beta SARL", "FR32552081317", "Lyon", "69002"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Sous-traitance bâtiment (autoliquidation)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            tax_category: Some("AE".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "AE".to_owned(),
            taxable_amount: amt(10000),
            // Reverse charge: no VAT charged by the supplier.
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            // No VAT added: grand total == net.
            tax_inclusive_amount: amt(10000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(10000),
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

#[test]
fn france_credit_note_emits_type_code_381_and_bundles() {
    // EN 16931 BT-3 / UNCL1001: a credit note (avoir) is code 381, distinct
    // from an invoice (380). The Factur-X CII binding writes it as
    // <ram:TypeCode>381</ram:TypeCode> in the ExchangedDocument.
    let doc = french_credit_note();
    let (ikb, report, factur_x) = bundle_document(&doc, None);

    // The ExchangedDocument TypeCode element carries an inline xmlns:ram, so we
    // match on the value + close tag (UNCL1001 code 381 = credit note / avoir).
    assert!(
        factur_x.contains(">381</ram:TypeCode>"),
        "avoir must carry UNCL1001 code 381, not 380:\n{factur_x}"
    );
    // It must NOT also be tagged as a plain invoice (380).
    assert!(
        !factur_x.contains(">380</ram:TypeCode>"),
        "credit note must not also be a 380 invoice"
    );
    // Net 50.00 + 20% TVA = 60.00 payable.
    assert!(factur_x.contains("<ram:GrandTotalAmount>60.00</ram:GrandTotalAmount>"));
    assert!(factur_x.contains("<ram:DuePayableAmount>60.00</ram:DuePayableAmount>"));

    // The CTC lifecycle and evidence bundle work identically for an avoir.
    assert_eq!(report.envelope.lifecycle, FrCtcLifecycle::Approved);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "credit-note evidence bundle must verify");
}

#[test]
fn france_multiline_invoice_carries_both_tax_breakdowns() {
    // Two lines at two French TVA rates (20% standard `S`, 5.5% reduced `AA`).
    // The CII binding emits one ApplicableTradeTax breakdown per category with
    // ram:CalculatedAmount + ram:BasisAmount (EN 16931 BG-23 / BT-117 / BT-116).
    let doc = french_multiline_invoice();
    let (ikb, report, factur_x) = bundle_document(&doc, None);

    // Both line items are present (EN 16931 BG-25, one per IncludedSupplyChain
    // trade line). The opening tag carries an inline xmlns:ram, so match the
    // tag prefix rather than the bare `>`-terminated form.
    assert_eq!(
        factur_x
            .matches("<ram:IncludedSupplyChainTradeLineItem")
            .count(),
        2,
        "multi-line invoice must emit two CII trade line items:\n{factur_x}"
    );
    // Two header-level tax breakdowns (one per category). Each header
    // ApplicableTradeTax block opens with a ram:CalculatedAmount (EN 16931
    // BT-117); line-level tax blocks do not. Count those to isolate the
    // header breakdowns from the per-line tax tags.
    assert_eq!(
        factur_x.matches("<ram:CalculatedAmount>").count(),
        2,
        "two distinct VAT categories must yield two header tax groups"
    );
    // The reduced-rate group: category AA, 5.50 VAT on a 100.00 basis.
    assert!(factur_x.contains("<ram:CategoryCode>AA</ram:CategoryCode>"));
    assert!(
        factur_x.contains("<ram:CalculatedAmount>5.50</ram:CalculatedAmount>"),
        "reduced-rate VAT amount 5.50 missing:\n{factur_x}"
    );
    // The standard-rate group: category S, 20.00 VAT.
    assert!(factur_x.contains("<ram:CategoryCode>S</ram:CategoryCode>"));
    assert!(factur_x.contains("<ram:CalculatedAmount>20.00</ram:CalculatedAmount>"));
    // Grand total = 200.00 net + 25.50 VAT = 225.50.
    assert!(factur_x.contains("<ram:GrandTotalAmount>225.50</ram:GrandTotalAmount>"));

    assert!(report.envelope.lifecycle.is_accepted());
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "multi-line evidence bundle must verify");
}

#[test]
fn france_reverse_charge_emits_category_ae_with_zero_vat() {
    // Autoliquidation: UNCL5305 category `AE`, 0% rate, no VAT amount; the
    // grand total equals the net. The buyer self-assesses the VAT (CGI 283-2).
    let doc = french_reverse_charge_invoice();
    let (ikb, report, factur_x) = bundle_document(&doc, None);

    assert!(
        factur_x.contains("<ram:CategoryCode>AE</ram:CategoryCode>"),
        "reverse charge must use UNCL5305 category AE:\n{factur_x}"
    );
    // No VAT charged: the header tax breakdown CalculatedAmount is 0.00.
    assert!(factur_x.contains("<ram:CalculatedAmount>0.00</ram:CalculatedAmount>"));
    // Grand total == net basis (100.00); no VAT added to the payable amount.
    assert!(factur_x.contains("<ram:GrandTotalAmount>100.00</ram:GrandTotalAmount>"));
    assert!(factur_x.contains("<ram:DuePayableAmount>100.00</ram:DuePayableAmount>"));
    // The standard-rate category must not appear on a pure reverse-charge line.
    assert!(!factur_x.contains("<ram:CategoryCode>S</ram:CategoryCode>"));

    assert!(report.envelope.lifecycle.is_accepted());
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "reverse-charge evidence bundle must verify");
}

#[test]
fn france_authority_rejection_carries_dgfip_motif_and_bundles() {
    // The DGFiP "cycle de vie" records a refusal as the status `Rejeté`, with a
    // typed "motif de rejet" — NOT a transport error. Here we exercise motif
    // R10 ("Données de facture non conformes") from the DGFiP rejection
    // vocabulary, and prove the refusal still produces a verifiable audit trail.
    let doc = french_multiline_invoice();
    let factur_x = to_factur_x_xml(&doc).unwrap();
    let provider = provider(None)
        .with_forced_lifecycle(FrCtcLifecycle::Rejected)
        .with_rejection_reason("motif:R10 Données de facture non conformes");
    let report = provider
        .report(&report_request(factur_x.clone().into_bytes()))
        .unwrap();

    // A refusal is a verdict inside Ok(_), never an Err.
    assert_eq!(report.envelope.lifecycle, FrCtcLifecycle::Rejected);
    assert!(report.envelope.lifecycle.is_rejected());
    assert!(!report.envelope.lifecycle.is_accepted());
    let reason = report.envelope.reason.as_deref().unwrap();
    assert!(
        reason.contains("R10"),
        "the platform motif de rejet must be surfaced verbatim, got {reason:?}"
    );

    // Even a rejected submission still bundles into verifiable evidence.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/factur-x.xml".to_owned(), factur_x.into_bytes());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let ikb = pack(&EvidenceBundle { manifest, artefacts }).unwrap();
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejected-with-motif bundle must verify");
}

#[test]
fn france_rejects_malformed_pdp_partner_siret() {
    // The PDP routing partner is identified by a 14-digit SIRET (DGFiP spec:
    // an accredited Plateforme de Dématérialisation Partenaire). A malformed
    // *receiver* SIRET must be refused at the shape gate (a pre-wire Err), not
    // silently transmitted. (The partner-platform SIRET on FrCtcPlatform::Pdp
    // is opaque routing metadata; the receiver SIRET is what the report layer
    // validates.)
    let factur_x = to_factur_x_xml(&french_invoice()).unwrap().into_bytes();
    let mut req = report_request(factur_x);
    // Route to a private PDP, but hand it a non-numeric receiver SIRET.
    req.platform = FrCtcPlatform::Pdp {
        siret: "73282932000074".to_owned(),
    };
    req.environment = FrCtcEnvironment::Production;
    req.receiver = FrCtcReceiver::Siret("7328293200007X".to_owned());
    let err = provider(None).report(&req).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_fr_ctc::FrCtcReportError::BadIdentifier(_)
        ),
        "a malformed receiver SIRET must be a BadIdentifier Err, got {err:?}"
    );
}

#[test]
fn france_rejects_receiver_siren_with_wrong_length() {
    // The SIREN receiver key is exactly 9 digits (INSEE legal-entity number).
    // A 10-digit value is a shape violation, refused before the wire.
    let factur_x = to_factur_x_xml(&french_invoice()).unwrap().into_bytes();
    let mut req = report_request(factur_x);
    req.receiver = FrCtcReceiver::Siren("3918380420".to_owned()); // 10 digits
    let err = provider(None).report(&req).unwrap_err();
    assert!(matches!(
        err,
        invoicekit_report_fr_ctc::FrCtcReportError::BadIdentifier(_)
    ));
}

#[test]
fn france_credit_note_lifecycle_is_byte_deterministic() {
    // The avoir path must be just as byte-stable as the facture path: the CII
    // serializer canonicalizes and the mock providers are deterministic.
    let doc = french_credit_note();
    let (a, _, xa) = bundle_document(&doc, None);
    let (b, _, xb) = bundle_document(&doc, None);
    assert_eq!(xa, xb, "credit-note Factur-X must be byte-stable");
    assert_eq!(a, b, "credit-note evidence bundle must be byte-stable");
}
