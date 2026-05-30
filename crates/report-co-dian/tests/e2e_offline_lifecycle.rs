// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Colombia DIAN offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Colombia and proves it
//! deterministically, mirroring the proven `report-it-sdi` pattern:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("CO")`
//!    and the Colombian peso (`COP`)
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml` (DIAN's
//!    payload is the UBL 2.1 + DIAN CIUS family; this crate exposes no
//!    serializer of its own, so the EN 16931 / UBL path is the honest source)
//! 3. submit those bytes to the existing offline `MockDianProvider`, asserting
//!    the DIAN authority artefacts (96-char CUFE, `DIAN-` track id, status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the mock has no forced-`Rechazado` knob (its happy path always
//!    returns `Procesando`), so the only authority-refusal route is the
//!    pre-wire `Err(DianError::BadNit)` / `Err(DianError::BadXml)` validation
//!    the real adapter runs. That genuine refusal path is exercised below.
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would
//! mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_co_dian::{
    DianDocumentKind, DianEnvironment, DianError, DianProvider, DianStatus, DianSubmitEnvelope,
    DianSubmitRequest, MockDianProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const FIXED_SUBMITTED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_co_e2e";
const TRACE: &str = "trace_co_e2e";
const ISSUER_NIT: &str = "900123456-7";
const BUYER_NIT: &str = "800987654";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn colombian_party(name: &str, nit: &str, city: &str, subdivision: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "nit".to_owned(),
            value: nit.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Carrera 7 # 1-00".to_owned()],
            city: city.to_owned(),
            subdivision: Some(subdivision.to_owned()),
            postal_code: "110111".to_owned(),
            country: CountryCode::new("CO").unwrap(),
        },
        contact: None,
    }
}

fn colombian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-co-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("SETP-2026-CO-0001").unwrap(),
        // Colombian peso. 19% IVA on a 100.00 line -> 119.00 payable.
        currency: Iso4217Code::new("COP").unwrap(),
        supplier: colombian_party("Acme SAS", "900123456-7", "Bogota", "DC"),
        customer: colombian_party("Beta Ltda", "800987654", "Medellin", "ANT"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoria y desarrollo de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11900),
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

fn submit_request(invoice_xml: Vec<u8>) -> DianSubmitRequest {
    DianSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DianEnvironment::Habilitacion,
        kind: DianDocumentKind::FacturaVenta,
        issuer_nit: ISSUER_NIT.to_owned(),
        buyer_nit: Some(BUYER_NIT.to_owned()),
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize -> submit (mock DIAN) -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the DIAN envelope so callers can
/// assert both the authority artefacts and the verifiability of the bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_co_dian::DianSubmitEnvelope) {
    // 1. build
    let doc = colombian_invoice();

    // 2. serialize -> UBL 2.1 (the DIAN CIUS payload family).
    let ubl = to_xml(&doc).unwrap();
    // Structural spot-check: the canonical UBL spine is present. The
    // canonicalizer inlines the namespace declarations onto each element, so
    // assert on the prefixed element-name openers (no trailing `>`) plus the
    // Colombian peso currency code in the body.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">COP</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL payload missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the offline mock DIAN provider.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + DIAN receipt.
    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn colombia_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // DIAN authority artefacts: 96-char CUFE, DIAN- track id, Procesando verdict.
    assert_eq!(envelope.cufe.len(), 96, "CUFE must be 96 hex chars");
    assert!(
        envelope.cufe.bytes().all(|b| b.is_ascii_hexdigit()),
        "CUFE must be lowercase hex"
    );
    assert!(
        envelope.track_id.starts_with("DIAN-"),
        "track id must carry the DIAN- prefix"
    );
    assert_eq!(envelope.status, DianStatus::Procesando);
    assert_eq!(envelope.submitted_at, FIXED_SUBMITTED_AT);
    assert!(envelope.message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn colombia_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn colombia_refusal_is_surfaced_as_err_before_the_wire() {
    // DIAN's mock has NO forced-`Rechazado` knob: the happy path always returns
    // `Procesando`. The only authority-refusal route the adapter models is the
    // pre-wire validation that runs BEFORE the payload reaches DIAN, surfaced as
    // an `Err` (per the project's "rejection-is-not-an-error" contract, a true
    // DIAN `Rechazado` verdict would be an Ok-envelope, but the mock does not
    // synthesize one). Exercise both genuine refusal shapes the mock can force.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let ubl = to_xml(&colombian_invoice()).unwrap().into_bytes();

    // Bad issuer NIT -> BadNit, before the wire.
    let mut bad_nit = submit_request(ubl);
    bad_nit.issuer_nit = "NOT-A-NIT".to_owned();
    let err = provider.submit_invoice(&bad_nit).unwrap_err();
    assert!(matches!(err, DianError::BadNit(_)), "got {err:?}");

    // Empty payload -> BadXml, before the wire.
    let mut empty = submit_request(Vec::new());
    empty.buyer_nit = None;
    let err = provider.submit_invoice(&empty).unwrap_err();
    assert!(matches!(err, DianError::BadXml(_)), "got {err:?}");
}

// ---------------------------------------------------------------------------
// Country-specific deepening scenarios (added; do not weaken the above).
//
// Grounded in the DIAN "Anexo Técnico de la Factura Electrónica de Venta"
// (current v1.9, Resolución DIAN 000165 de 2023; prior baseline Resolución
// 000012 de 09-02-2021):
//   https://www.dian.gov.co/impuestos/factura-electronica/Documents/Anexo-Tecnico-Factura-Electronica-de-Venta-vr-1-9.pdf
// The Anexo defines five UBL 2.1 document classes (Invoice, CreditNote /
// Nota Crédito, DebitNote / Nota Débito, ApplicationResponse, AttachedDocument),
// the CUFE (Código Único de Factura Electrónica, SHA-384, 96 hex chars) for
// sales invoices and the CUDE (Código Único de Documento Electrónico) for the
// other classes, the standard IVA rate of 19%, and the tax treatments
// "exenta" (exempt, 0% with right to credit) vs "excluida" (excluded).
// ---------------------------------------------------------------------------

/// Build the offline evidence bundle for an arbitrary Colombian document +
/// DIAN envelope. Mirrors `run_lifecycle`'s step 4 so every scenario produces a
/// verifiable `.ikb` from the same canonical artefact layout.
fn bundle_for(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &DianSubmitEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    pack(&EvidenceBundle { manifest, artefacts }).unwrap()
}

/// A Nota Crédito (UBL `CreditNote`, DIAN doc class `NotaCredito`) that fully
/// reverses an earlier factura de venta. DIAN's Anexo requires a corrective
/// document to carry the reference to the invoice it amends; we model that with
/// an IR `DocumentReference` (UBL `cac:BillingReference`). Per the UBL serializer
/// a `CreditNote` carries no top-level `cbc:DueDate`, so `due_date` stays `None`.
fn colombian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-co-nc-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        // DIAN Nota Crédito numbering keeps its own authorized prefix range.
        document_number: DocumentNumber::new("NC-2026-CO-0007").unwrap(),
        currency: Iso4217Code::new("COP").unwrap(),
        supplier: colombian_party("Acme SAS", "900123456-7", "Bogota", "DC"),
        customer: colombian_party("Beta Ltda", "800987654", "Medellin", "ANT"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Reverso total factura SETP-2026-CO-0001".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11900),
        },
        attachments: Vec::new(),
        // DIAN-required pointer back to the corrected invoice.
        references: vec![DocumentReference {
            kind: "factura".to_owned(),
            id: "SETP-2026-CO-0001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
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

/// A multi-line factura de venta mixing the standard 19% IVA (tax category `S`)
/// with an IVA-exempt line (tax category `E` / "exenta", 0%). DIAN's Anexo
/// requires every distinct tax treatment to be summarised in its own
/// `cac:TaxSubtotal`, so this exercises the serializer's per-category emission.
fn colombian_mixed_tax_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-co-mixed-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("SETP-2026-CO-0042").unwrap(),
        currency: Iso4217Code::new("COP").unwrap(),
        supplier: colombian_party("Acme SAS", "900123456-7", "Bogota", "DC"),
        customer: colombian_party("Beta Ltda", "800987654", "Medellin", "ANT"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Taxed line: 200.00 @ 19% IVA.
            DocumentLine {
                id: "1".to_owned(),
                description: "Licencia de software (gravado 19%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            // Exempt line: 50.00 @ 0% (exenta).
            DocumentLine {
                id: "2".to_owned(),
                description: "Servicio educativo (exento de IVA)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(5000),
                tax_category: Some("E".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(20000),
                tax_amount: amt(3800),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            },
            TaxCategorySummary {
                category_code: "E".to_owned(),
                taxable_amount: amt(5000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            },
        ],
        monetary_total: MonetaryTotal {
            // 200.00 + 50.00 = 250.00 base; 38.00 IVA; 288.00 payable.
            line_extension_amount: amt(25000),
            tax_exclusive_amount: amt(25000),
            tax_inclusive_amount: amt(28800),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(28800),
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

/// A factura de exportación (DIAN doc class `FacturaExportacion`). Exports are
/// IVA-exempt and the foreign buyer has no Colombian NIT, so `buyer_nit` is
/// `None` (the adapter only shape-validates a NIT when one is supplied). The
/// currency is USD, which DIAN allows on export invoices.
fn colombian_export_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-co-exp-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-07-28").unwrap()),
        document_number: DocumentNumber::new("EXP-2026-CO-0003").unwrap(),
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: colombian_party("Acme SAS", "900123456-7", "Bogota", "DC"),
        // Foreign buyer: a US corporation with no Colombian NIT.
        customer: Party {
            id: Some("gamma-inc".to_owned()),
            name: "Gamma Inc".to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "foreign".to_owned(),
                value: "US-EIN-123456".to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["1 Market St".to_owned()],
                city: "San Francisco".to_owned(),
                subdivision: Some("CA".to_owned()),
                postal_code: "94105".to_owned(),
                country: CountryCode::new("US").unwrap(),
            },
            contact: None,
        },
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exported software services (0% IVA)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(50000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(50000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        monetary_total: MonetaryTotal {
            // No IVA on an export: payable == net.
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(50000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(50000),
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
fn colombia_nota_credito_lifecycle_bundles_and_verifies() {
    // DIAN Anexo Técnico v1.9 §"Nota Crédito": a corrective document is a UBL
    // 2.1 CreditNote (cbc:CreditNoteTypeCode 381 at the syntax layer) that must
    // reference the factura it amends. We submit it under doc class NotaCredito.
    let doc = colombian_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // CreditNote syntax spine. (The canonicalizer inlines each element's
    // namespace declaration as an attribute, so assert on the prefixed
    // element-name opener — no trailing `>` — and on the exact closing tags.)
    assert!(ubl.contains("<CreditNote"), "root must be a UBL CreditNote");
    assert!(
        ubl.contains("<cbc:CreditNoteTypeCode")
            && ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "Nota Crédito must carry UBL CreditNoteTypeCode 381"
    );
    assert!(
        !ubl.contains("<cbc:InvoiceTypeCode"),
        "a CreditNote must NOT emit an InvoiceTypeCode"
    );
    // The corrected invoice id survives in the corrective line description.
    assert!(
        ubl.contains("Reverso total factura SETP-2026-CO-0001"),
        "the corrected invoice id must be carried on the Nota Crédito"
    );
    assert!(
        ubl.contains(">COP</cbc:DocumentCurrencyCode>"),
        "Nota Crédito must keep the Colombian peso"
    );
    // The IR `references` pointer back to the original factura is preserved in
    // the canonical evidence artefact (it has no top-level UBL home today).
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap();
    assert!(
        canonical.contains("SETP-2026-CO-0001"),
        "the DIAN-required reference to the corrected invoice must persist in canonical.json"
    );
    let ubl_bytes = ubl.into_bytes();

    // Submit under the NotaCredito document class. DIAN issues a CUDE (not a
    // CUFE) for this class; the mock still returns a 96-char hex code.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let mut req = submit_request(ubl_bytes.clone());
    req.kind = DianDocumentKind::NotaCredito;
    let envelope = provider.submit_invoice(&req).unwrap();
    assert_eq!(envelope.cufe.len(), 96, "CUDE/CUFE code is 96 hex chars");
    assert!(envelope.cufe.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(envelope.status, DianStatus::Procesando);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "Nota Crédito evidence bundle must verify");
}

#[test]
fn colombia_multi_line_mixed_iva_serializes_both_tax_subtotals() {
    // DIAN Anexo Técnico v1.9 §"Impuestos": each tax treatment is summarised in
    // its own cac:TaxSubtotal. Standard IVA in Colombia is 19%; an "exenta" line
    // sits at 0%. Assert both subtotals + the 19% percent render, and the bundle
    // verifies.
    let doc = colombian_mixed_tax_invoice();
    let ubl = to_xml(&doc).unwrap();

    // Two distinct lines.
    assert!(
        ubl.contains("Licencia de software (gravado 19%)"),
        "taxed line description must be present"
    );
    assert!(
        ubl.contains("Servicio educativo (exento de IVA)"),
        "exempt line description must be present"
    );
    // Per-category tax subtotals: the standard 19% IVA and the 0% exempt basis.
    // (Element openers carry an inlined xmlns attribute; match the exact value +
    // closing tag instead.)
    assert!(
        ubl.contains(">19.00</cbc:Percent>"),
        "the standard Colombian IVA rate (19%) must render as a TaxCategory Percent"
    );
    assert!(
        ubl.contains(">0</cbc:Percent>"),
        "the exempt (exenta) basis must render a 0% TaxCategory Percent"
    );
    // Aggregate IVA across the document is 38.00 (only the taxed line carries it).
    assert!(
        ubl.contains(">38.00</cbc:TaxAmount>"),
        "aggregate IVA across the two lines must be 38.00"
    );
    // Both category ids appear (S = gravado, E = exenta).
    assert!(ubl.contains(">S</cbc:ID>"), "standard category id S");
    assert!(ubl.contains(">E</cbc:ID>"), "exempt category id E");
    let ubl_bytes = ubl.into_bytes();

    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();
    assert_eq!(envelope.status, DianStatus::Procesando);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok,
        "mixed-IVA evidence bundle must verify"
    );
}

#[test]
fn colombia_export_invoice_is_zero_rated_and_b2c_omits_buyer_nit() {
    // DIAN Anexo Técnico v1.9 §"Factura de exportación": exports are IVA-exempt
    // and the foreign buyer has no Colombian NIT. The adapter only validates a
    // NIT when one is supplied, so buyer_nit stays None.
    let doc = colombian_export_invoice();
    let ubl = to_xml(&doc).unwrap();

    assert!(
        ubl.contains(">USD</cbc:DocumentCurrencyCode>"),
        "export invoice is denominated in USD"
    );
    assert!(
        ubl.contains(">US</cbc:IdentificationCode>"),
        "the foreign buyer's country must be US"
    );
    // Zero-rated export: no IVA in the totals; payable equals the net 500.00,
    // and the only TaxAmount values present are 0.00.
    assert!(
        ubl.contains(">500.00</cbc:PayableAmount>"),
        "export payable equals the net (no IVA)"
    );
    assert!(
        ubl.contains(">0.00</cbc:TaxAmount>") && !ubl.contains(">38.00</cbc:TaxAmount>"),
        "an export invoice carries no positive IVA"
    );
    let ubl_bytes = ubl.into_bytes();

    // Submit a FacturaExportacion with NO buyer NIT.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let req = DianSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DianEnvironment::Produccion,
        kind: DianDocumentKind::FacturaExportacion,
        issuer_nit: ISSUER_NIT.to_owned(),
        buyer_nit: None,
        invoice_xml: ubl_bytes.clone(),
    };
    let envelope = provider.submit_invoice(&req).unwrap();
    assert_eq!(envelope.status, DianStatus::Procesando);
    assert!(envelope.track_id.starts_with("DIAN-"));

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok,
        "export evidence bundle must verify"
    );
}

#[test]
fn colombia_rechazado_verdict_is_a_status_not_an_err_and_still_bundles() {
    // The two load-bearing anti-slop rules (Builder's Manual §4): a DIAN refusal
    // is surfaced as DianStatus::Rechazado inside an Ok envelope, NOT as an Err,
    // so the audit trail persists the rejection. The mock has no forced-Rechazado
    // knob, so we synthesise the verdict the live SOAP adapter would return for a
    // payload DIAN rejects (e.g. validation rule "FAE/CAE-fixed" failures), then
    // prove it serialises into a verifiable bundle exactly like the happy path.
    let doc = colombian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let rejected = DianSubmitEnvelope {
        cufe: "f".repeat(96),
        track_id: "DIAN-000000000099".to_owned(),
        status: DianStatus::Rechazado,
        submitted_at: FIXED_SUBMITTED_AT.to_owned(),
        // DIAN surfaces a rule code + Spanish message on rejection.
        message: Some("Regla: DIAN090, Rechazo: CUFE no corresponde".to_owned()),
    };
    assert_eq!(rejected.status, DianStatus::Rechazado);
    assert!(rejected.message.is_some(), "a rejection carries a reason");

    // The rejection verdict round-trips through serde with the kebab-case tag
    // DIAN's JSON contract uses.
    let json = serde_json::to_string(&rejected).unwrap();
    assert!(
        json.contains("\"status\":\"rechazado\""),
        "Rechazado must serialise kebab-case, got {json}"
    );
    let parsed: DianSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, rejected);

    // A rejected verdict bundles + verifies just like a delivered one.
    let ikb = bundle_for(&doc, &ubl_bytes, &rejected);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok,
        "rejection-path evidence bundle must verify"
    );
}

#[test]
fn colombia_aceptado_con_observaciones_serialises_kebab_case() {
    // DIAN can accept a document with warnings (AceptadoConObservaciones). That
    // verdict is also an Ok envelope and must round-trip with its kebab-case
    // serde tag.
    let envelope = DianSubmitEnvelope {
        cufe: "0".repeat(96),
        track_id: "DIAN-000000000100".to_owned(),
        status: DianStatus::AceptadoConObservaciones,
        submitted_at: FIXED_SUBMITTED_AT.to_owned(),
        message: Some("Observacion: DIAN tolerancia de redondeo".to_owned()),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(
        json.contains("\"status\":\"aceptado-con-observaciones\""),
        "AceptadoConObservaciones must serialise kebab-case, got {json}"
    );
    let parsed: DianSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, envelope);
}

#[test]
fn colombia_query_track_id_round_trips_and_rejects_blank() {
    // The async reconciliation verb: query_track_id returns the latest verdict
    // (Aceptado in the mock) for a known DIAN- track id, and surfaces an unknown
    // (empty) track id as a transport Err — never a panic.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let env = provider
        .query_track_id(DianEnvironment::Produccion, "DIAN-000000000001")
        .unwrap();
    assert_eq!(env.status, DianStatus::Aceptado);
    assert_eq!(env.track_id, "DIAN-000000000001");
    assert_eq!(env.cufe.len(), 96);

    let err = provider
        .query_track_id(DianEnvironment::Produccion, "")
        .unwrap_err();
    assert!(matches!(err, DianError::Transport(_)), "got {err:?}");
}

#[test]
fn colombia_nit_check_digit_boundaries_match_dian_shape() {
    // DIAN NIT shape: 9-10 digit base, optionally a hyphenated check digit
    // (dígito de verificación), so the collapsed length lands in 9..=11. The
    // adapter rejects out-of-range and non-numeric ids before the wire as
    // BadNit. These are the genuine boundary cases the validator enforces.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let ubl = to_xml(&colombian_invoice()).unwrap().into_bytes();

    // 12 collapsed digits (900123456789) is too long -> BadNit.
    let mut too_long = submit_request(ubl.clone());
    too_long.issuer_nit = "900123456789".to_owned();
    assert!(
        matches!(
            provider.submit_invoice(&too_long).unwrap_err(),
            DianError::BadNit(_)
        ),
        "a 12-digit NIT must be rejected"
    );

    // A bad buyer NIT (letters) is also rejected, even when the issuer is valid.
    let mut bad_buyer = submit_request(ubl.clone());
    bad_buyer.buyer_nit = Some("80098765X".to_owned());
    assert!(
        matches!(
            provider.submit_invoice(&bad_buyer).unwrap_err(),
            DianError::BadNit(_)
        ),
        "a non-numeric buyer NIT must be rejected"
    );

    // A valid 9-digit base WITH hyphenated check digit (10 base + 1 check = 11
    // collapsed) is accepted.
    let mut hyphenated = submit_request(ubl);
    hyphenated.issuer_nit = "9001234560-1".to_owned();
    hyphenated.buyer_nit = Some("800987654-3".to_owned());
    let env = provider.submit_invoice(&hyphenated).unwrap();
    assert_eq!(env.status, DianStatus::Procesando);
}

#[test]
fn colombia_credit_note_lifecycle_is_byte_deterministic() {
    // Determinism must hold for the corrective (CreditNote) path too, not just
    // the standard invoice covered above.
    let run = || {
        let doc = colombian_credit_note();
        let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
        let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
        let mut req = submit_request(ubl_bytes.clone());
        req.kind = DianDocumentKind::NotaCredito;
        let envelope = provider.submit_invoice(&req).unwrap();
        bundle_for(&doc, &ubl_bytes, &envelope)
    };
    assert_eq!(run(), run(), "the Nota Crédito lifecycle must be byte-stable");
}
