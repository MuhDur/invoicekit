// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Greece **myDATA** offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Greece and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) for a Greek (`GR`) supplier +
//!    customer, currency `EUR`
//! 2. serialize -> EN 16931 / UBL XML via `invoicekit_format_ubl::to_xml`
//!    (myDATA's wire payload is the IAPR `InvoicesDoc`; the foundation UBL path
//!    is the family serializer this crate composes — this crate ships no
//!    serializer of its own)
//! 3. submit those bytes to the EXISTING `MockMyDataProvider` and assert the
//!    Greek authority artefacts: an `Accepted` verdict, the IAPR **MARK**
//!    (Μοναδικός Αριθμός Καταχώρησης), the **UID**, and the pinned
//!    `reported_at` timestamp; also assert the QR payload the printed invoice
//!    must carry embeds both MARK + UID
//! 4. assemble a `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml` +
//!    `receipt.json`) and `verify_packed(content_only).ok == true` (exit 0)
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the mock returns `Err` for the two pre-wire shape failures it
//!    validates (bad ΑΦΜ / AFM, empty payload). See the note on the test for
//!    why an authority-side `Rejected` verdict cannot be forced here.
//!
//! This mirrors `crates/report-it-sdi/tests/e2e_offline_lifecycle.rs`, the
//! proven offline-E2E reference pattern. Goldens are hand-rolled (no `insta` /
//! `pretty_assertions`, which would mutate `Cargo.lock`). The capability matrix
//! is intentionally NOT asserted here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_gr_mydata::{
    qr_payload, to_invoices_doc_xml, validate_afm, MockMyDataProvider, MyDataDocContext,
    MyDataEnvironment, MyDataError, MyDataInvoiceCategory, MyDataMark, MyDataProvider,
    MyDataReportEnvelope, MyDataReportRequest, MyDataStatus, MyDataUid,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_REPORTED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_gr_e2e";
const TRACE: &str = "trace_gr_e2e";
const ISSUER_AFM: &str = "123456789";
const BUYER_AFM: &str = "987654321";
const QR_BASE_URL: &str = "https://www.aade.gr/mydata";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn greek_party(name: &str, vat: &str, street: &str, city: &str, postal: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec![street.to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: postal.to_owned(),
            country: CountryCode::new("GR").unwrap(),
        },
        contact: None,
    }
}

fn greek_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-gr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-GR-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: greek_party(
            "Acme Hellas AE",
            "EL123456789",
            "Leoforos Kifisias 1",
            "Athina",
            "11523",
        ),
        customer: greek_party(
            "Beta EPE",
            "EL987654321",
            "Egnatia 100",
            "Thessaloniki",
            "54622",
        ),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Symvouleftikes ypiresies logismikou".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // UBL family uses EA (CII/Factur-X would use C62).
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Greek standard VAT rate is 24%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2400),
            tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12400),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12400),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

fn report_request(invoices_doc_xml: Vec<u8>) -> MyDataReportRequest {
    MyDataReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: MyDataEnvironment::Sandbox,
        issuer_afm: ISSUER_AFM.to_owned(),
        buyer_afm: Some(BUYER_AFM.to_owned()),
        category: MyDataInvoiceCategory::SalesGoods {
            code: "1.1".to_owned(),
        },
        invoices_doc_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> report (mock IAPR) -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the authority envelope so callers can
/// assert both the bundle verifies and the Greek artefacts are present.
fn run_lifecycle() -> (Vec<u8>, MyDataReportEnvelope) {
    // 1. build the canonical IR document.
    let doc = greek_invoice();

    // 2. serialize -> EN 16931 / UBL XML bytes (this crate ships no serializer
    //    of its own; it composes the UBL family path).
    let ubl_xml = to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();
    // Sanity: the UBL spine the IAPR mapping reads from must be present.
    // The canonicalizer pins namespace declarations inline on each element, so
    // we match the element-name prefix (open angle + name), not a bare `>`.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">EUR</cbc:DocumentCurrencyCode>",
        ">GR</cbc:IdentificationCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    // 3. report to the offline IAPR mock.
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let envelope = provider.report_invoice(&report_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national/family XML + receipt.
    let ikb = pack_bundle(&doc, ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn greece_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: the IAPR accepted and assigned a MARK + UID.
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    assert_eq!(envelope.reported_at, PINNED_REPORTED_AT);
    assert!(envelope.message.is_none());

    let mark = envelope.mark.as_ref().expect("accepted invoice carries a MARK");
    let uid = envelope.uid.as_ref().expect("accepted invoice carries a UID");
    // The mock derives a 16-digit IAPR-shaped MARK from its serial.
    assert!(mark.as_str().starts_with("4000"));
    assert!(uid.as_str().starts_with("MYDATA-MOCK-UID-"));

    // The printed-invoice QR payload must embed both MARK + UID.
    let qr = qr_payload(QR_BASE_URL, &envelope).unwrap();
    assert!(qr.contains(&format!("mark={}", mark.as_str())));
    assert!(qr.contains(&format!("uid={}", uid.as_str())));

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn greece_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn greece_refuses_invalid_afm_and_empty_payload_before_the_wire() {
    // Refusal note: `MockMyDataProvider` always synthesises an `Accepted`
    // verdict for a well-shaped request — it exposes no knob to force an
    // authority-side `MyDataStatus::Rejected` (unlike Italy's
    // `with_forced_receipt`). The genuine refusal surface it DOES implement is
    // pre-wire shape validation, which returns `Err`, not a `Rejected`
    // envelope. We exercise both shape refusals here.
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);

    // Well-shaped UBL bytes so only the AFM is wrong.
    let ubl_bytes = to_xml(&greek_invoice()).unwrap().into_bytes();

    // (a) bad issuer ΑΦΜ (AFM) — must be exactly 9 ASCII digits.
    let mut bad_afm = report_request(ubl_bytes.clone());
    bad_afm.issuer_afm = "12345".to_owned();
    let err = provider.report_invoice(&bad_afm).unwrap_err();
    assert!(
        matches!(err, MyDataError::BadAfm(_)),
        "short AFM must be refused as BadAfm, got {err:?}"
    );

    // (b) empty InvoicesDoc payload — refused before any synthesis.
    let mut empty_payload = report_request(ubl_bytes);
    empty_payload.invoices_doc_xml.clear();
    let err = provider.report_invoice(&empty_payload).unwrap_err();
    assert!(
        matches!(err, MyDataError::BadXml(_)),
        "empty payload must be refused as BadXml, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Deepened, Greece-specific scenarios.
//
// Each exercises a distinct myDATA capability or format variation grounded in
// the IAPR (Independent Authority for Public Revenue, ΑΑΔΕ) myDATA REST API
// specification. The authoritative `invoiceType` and `vatCategory` /
// `vatExemptionCategory` code lists referenced below are published by the IAPR:
//   - myDATA technical specifications / API documentation:
//     <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>
//   - myDATA documentation portal: <https://www.aade.gr/mydata>
// Fixtures are hand-built and synthetic; no copyrighted regulator files are
// vendored.
// ---------------------------------------------------------------------------

/// Pack a document + authority envelope into a `.ikb` evidence bundle the same
/// way `run_lifecycle` does, so the deepened scenarios reuse the proven path.
fn pack_bundle(doc: &CommercialDocument, ubl_bytes: Vec<u8>, envelope: &MyDataReportEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// A Greek **credit note** that corrects an earlier invoice.
///
/// myDATA classifies an *associated* credit note (one that points back to the
/// original transmitted invoice) as `invoiceType` **`5.1`** and a
/// non-associated credit note as `5.2` (IAPR myDATA API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
/// On the UBL family wire the InvoiceKit serializer emits a `<CreditNote>` root
/// carrying `cbc:CreditNoteTypeCode` `381` (UN/CEFACT 1001 code for a credit
/// note), which is the document the IAPR mapping reads.
fn greek_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-gr-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-06-02").unwrap(),
        // myDATA credit notes carry a tax point; serializer emits cbc:TaxPointDate.
        tax_point_date: Some(DateOnly::new("2026-05-26").unwrap()),
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-GR-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: greek_party(
            "Acme Hellas AE",
            "EL123456789",
            "Leoforos Kifisias 1",
            "Athina",
            "11523",
        ),
        customer: greek_party(
            "Beta EPE",
            "EL987654321",
            "Egnatia 100",
            "Thessaloniki",
            "54622",
        ),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Pistotiko gia akyrosi ypiresion".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(1200),
            tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(6200),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(6200),
        },
        attachments: Vec::new(),
        // Associated credit note: it references the corrected invoice.
        references: vec![DocumentReference {
            kind: "invoice".to_owned(),
            id: "INV-2026-GR-0001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn greece_associated_credit_note_5_1_reports_and_bundles() {
    // Build a credit note and serialize it on the UBL family path.
    let doc = greek_credit_note();
    let ubl_xml = to_xml(&doc).unwrap();

    // Country/format specifics: a CreditNote root with the 381 type code and
    // no InvoiceTypeCode. (UBL emits inline namespace declarations on each
    // element, so we match the value-bearing close tag, which is stable.)
    assert!(ubl_xml.contains("<CreditNote"), "must serialize a UBL CreditNote root");
    assert!(
        ubl_xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "credit note must carry CreditNoteTypeCode 381, got: {ubl_xml}"
    );
    assert!(
        !ubl_xml.contains("</cbc:InvoiceTypeCode>"),
        "a credit note must NOT emit an InvoiceTypeCode"
    );
    // myDATA correlates an associated (5.1) credit note to the original
    // invoice. The UBL family serializer does not carry IR `references` on the
    // wire, but the canonical projection (which lands in the evidence bundle's
    // canonical.json) preserves the corrected-invoice link verbatim.
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap();
    assert!(
        canonical.contains("INV-2026-GR-0001"),
        "associated credit note must preserve the corrected invoice reference in canonical form"
    );
    assert!(
        canonical.contains("\"document_type\":\"credit_note\""),
        "canonical projection must mark this document as a credit_note"
    );

    // Report under the IAPR credit-note classification 5.1.
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let mut req = report_request(ubl_xml.clone().into_bytes());
    req.category = MyDataInvoiceCategory::CreditNote {
        code: "5.1".to_owned(),
    };
    assert_eq!(req.category.code(), "5.1");
    let envelope = provider.report_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    let mark = envelope.mark.as_ref().expect("accepted credit note carries a MARK");
    assert!(mark.as_str().starts_with("4000"));

    // The whole credit-note lifecycle bundles into verifiable evidence.
    let ikb = pack_bundle(&doc, ubl_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// A **multi-line** invoice that mixes the Greek standard VAT rate (24%) with
/// the reduced rate (13%). Both appear in the myDATA `vatCategory` taxonomy
/// (`1` = 24% standard, `2` = 13% reduced — IAPR myDATA API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
/// This proves the serializer carries two distinct `cac:TaxSubtotal` blocks
/// with the right percentages and that the multi-rate document still earns a
/// single MARK and a verifiable bundle.
fn greek_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-gr-e2e-ml-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-GR-0010").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: greek_party(
            "Acme Hellas AE",
            "EL123456789",
            "Leoforos Kifisias 1",
            "Athina",
            "11523",
        ),
        customer: greek_party("Beta EPE", "EL987654321", "Egnatia 100", "Thessaloniki", "54622"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Line 1: standard-rate goods, 24%.
            DocumentLine {
                id: "1".to_owned(),
                description: "Eksoplismos grafeiou (24%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(10000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            // Line 2: reduced-rate item, 13% (e.g. certain foodstuffs/services).
            DocumentLine {
                id: "2".to_owned(),
                description: "Ypiresia meiomenou syntelesti (13%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(5000),
                tax_category: Some("AA".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2400),
                tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "AA".to_owned(),
                taxable_amount: amt(5000),
                tax_amount: amt(650),
                tax_rate: Some(DecimalValue::new(Decimal::new(1300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(15000),
            tax_exclusive_amount: amt(15000),
            // 150.00 + 24.00 + 6.50 = 180.50
            tax_inclusive_amount: amt(18050),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(18050),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn greece_multiline_mixed_vat_rates_serialize_and_bundle() {
    let doc = greek_multiline_invoice();
    let ubl_xml = to_xml(&doc).unwrap();

    // Two distinct VAT subtotals with the Greek standard (24%) and reduced (13%)
    // percentages must both reach the wire. (Elements carry inline namespace
    // declarations, so we match the value-bearing close tag, which is stable.)
    assert!(
        ubl_xml.contains(">24.00</cbc:Percent>"),
        "standard-rate 24% subtotal must be present, got: {ubl_xml}"
    );
    assert!(
        ubl_xml.contains(">13.00</cbc:Percent>"),
        "reduced-rate 13% subtotal must be present, got: {ubl_xml}"
    );
    // Aggregate tax = 24.00 + 6.50 = 30.50 at the TaxTotal head.
    assert!(
        ubl_xml.contains(">30.50</cbc:TaxAmount>"),
        "TaxTotal head must sum both subtotals to 30.50, got: {ubl_xml}"
    );
    // Two priced lines are carried.
    assert_eq!(
        ubl_xml.matches("<cac:InvoiceLine").count(),
        2,
        "multi-line invoice must serialize exactly two InvoiceLine blocks"
    );

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let envelope = provider
        .report_invoice(&report_request(ubl_xml.clone().into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);

    let ikb = pack_bundle(&doc, ubl_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

/// A **VAT-exempt / 0%** services invoice.
///
/// In the myDATA taxonomy a 0%/exempt line uses `vatCategory` **`7`** ("Excluding
/// VAT"), and when `vatCategory` is `7` the `vatExemptionCategory` element is
/// mandatory (IAPR myDATA API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
/// On the EN 16931 / UBL family wire this surfaces as the BT-118 tax category
/// code `E` (exempt) with a `0` percentage. Here we prove the serializer emits a
/// zero-rated exempt `cac:TaxCategory` and that the invoice is still accepted
/// (its tax-inclusive total equals its taxable amount — no VAT is added).
fn greek_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-gr-e2e-ex-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-GR-0020").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: greek_party(
            "Acme Hellas AE",
            "EL123456789",
            "Leoforos Kifisias 1",
            "Athina",
            "11523",
        ),
        // Domestic exempt supply (e.g. an Article-22 exempt service): a Greek
        // customer, so the wire keeps the GR country code consistent.
        customer: greek_party("Gamma EPE", "EL555555555", "Akti Miaouli 7", "Pireas", "18535"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Apallassomeni ypiresia (exempt)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(20000),
            line_extension_amount: amt(20000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(20000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            // No VAT added: inclusive == exclusive.
            tax_inclusive_amount: amt(20000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(20000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn greece_vat_exempt_zero_rated_invoice_serializes_and_bundles() {
    let doc = greek_exempt_invoice();
    let ubl_xml = to_xml(&doc).unwrap();

    // Exempt category `E` at 0% must reach the wire (BT-118/BT-119). Elements
    // carry inline namespace declarations, so match the value-bearing close
    // tags. `>E</cbc:ID>` appears in both the line ClassifiedTaxCategory and the
    // TaxSubtotal TaxCategory — assert it shows up at least twice.
    assert!(
        ubl_xml.matches(">E</cbc:ID>").count() >= 2,
        "exempt invoice must carry tax category code E on line + subtotal, got: {ubl_xml}"
    );
    assert!(
        ubl_xml.contains(">0.00</cbc:Percent>"),
        "exempt category must carry a 0% rate, got: {ubl_xml}"
    );
    // No VAT was charged: head TaxAmount is zero and inclusive == taxable.
    assert!(
        ubl_xml.contains(">0.00</cbc:TaxAmount>"),
        "exempt invoice TaxTotal head must be zero, got: {ubl_xml}"
    );
    assert!(
        ubl_xml.contains(">200.00</cbc:TaxInclusiveAmount>"),
        "exempt invoice inclusive total must equal its taxable amount, got: {ubl_xml}"
    );

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    // Exempt domestic service: myDATA services class 2.x.
    let mut req = report_request(ubl_xml.clone().into_bytes());
    req.category = MyDataInvoiceCategory::Services {
        code: "2.1".to_owned(),
    };
    let envelope = provider.report_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);

    let ikb = pack_bundle(&doc, ubl_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "exempt-invoice evidence bundle must verify");
}

/// An authority-side **rejection** persisted via `MyDataStatus::Rejected`
/// (status, NOT an `Err`).
///
/// The crate's contract — like the IAPR REST API, which returns a per-invoice
/// error object rather than an HTTP failure — is that a `Rejected` verdict is a
/// value, not a transport error: it is recorded in the audit trail with no MARK
/// and no UID, plus the IAPR error text (IAPR myDATA API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
/// This mirrors the Italy SDI `NotificaScarto` path. We assert that such an
/// envelope (a) refuses to produce a printed-invoice QR payload (there is no
/// MARK/UID to embed), (b) survives a serde round-trip verbatim, and (c) still
/// packs into a verifiable evidence bundle so the refusal is auditable.
#[test]
fn greece_authority_rejection_is_a_status_not_an_error_and_stays_auditable() {
    let doc = greek_invoice();
    let ubl_xml = to_xml(&doc).unwrap();

    // The IAPR refuses the submission: no MARK/UID, an error message, status
    // Rejected. Error code 102 in the myDATA error taxonomy is a validation
    // failure on the invoice payload.
    let rejected = MyDataReportEnvelope {
        status: MyDataStatus::Rejected,
        mark: None,
        uid: None,
        message: Some("102: invalid invoice — vatCategory/vatExemptionCategory mismatch".to_owned()),
        reported_at: PINNED_REPORTED_AT.to_owned(),
    };

    // (a) A rejected envelope cannot yield a printed-invoice QR payload.
    let qr_err = qr_payload(QR_BASE_URL, &rejected).unwrap_err();
    assert!(
        matches!(qr_err, MyDataError::BadXml(_)),
        "QR build must refuse a rejected envelope (no MARK/UID), got {qr_err:?}"
    );

    // (b) The rejection round-trips through serde verbatim (audit fidelity).
    let json = serde_json::to_string(&rejected).unwrap();
    let parsed: MyDataReportEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, rejected);
    // Rejected status serializes in kebab-case and omits the absent MARK/UID.
    assert!(json.contains("\"status\":\"rejected\""));
    assert!(!json.contains("\"mark\""), "rejected receipt must omit MARK");
    assert!(!json.contains("\"uid\""), "rejected receipt must omit UID");

    // (c) The rejection still bundles into verifiable evidence — the audit
    // trail must persist refusals exactly as it persists acceptances.
    let ikb = pack_bundle(&doc, ubl_xml.into_bytes(), &rejected);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

/// The myDATA **ΑΦΜ (AFM)** field is the bare 9-digit Greek tax number, NOT the
/// EU VAT identifier (which Greek parties prefix with `EL`, e.g. `EL123456789`).
///
/// This is a real, country-specific trap: a party's `cac:PartyTaxScheme`
/// `CompanyID` on the EN 16931 / UBL wire carries the `EL`-prefixed VAT id, but
/// the value handed to the myDATA `issuerAFM` field must be the 9 digits with
/// the prefix stripped (IAPR myDATA API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
/// We prove the validator rejects the prefixed form and accepts the stripped
/// form, and that a request built with the prefixed AFM is refused before the
/// wire as `BadAfm`.
#[test]
fn greece_rejects_el_prefixed_vat_as_afm_but_accepts_bare_nine_digits() {
    // The EU VAT identifier the party publishes.
    let vat_id = "EL123456789";
    // Directly handing the VAT id to the AFM field is wrong: 11 chars, leading
    // letters — not 9 ASCII digits.
    assert!(
        validate_afm(vat_id).is_err(),
        "EL-prefixed VAT id must not pass as an AFM"
    );
    // Stripping the `EL` country prefix yields a valid 9-digit AFM.
    let bare_afm = vat_id.strip_prefix("EL").unwrap();
    assert_eq!(bare_afm, "123456789");
    assert!(validate_afm(bare_afm).is_ok(), "bare 9-digit AFM must pass");

    // End to end: a report request that mistakenly uses the prefixed VAT id as
    // the issuer AFM is refused before the wire (Err, not a Rejected verdict).
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let ubl_bytes = to_xml(&greek_invoice()).unwrap().into_bytes();
    let mut req = report_request(ubl_bytes);
    req.issuer_afm = vat_id.to_owned();
    let err = provider.report_invoice(&req).unwrap_err();
    assert!(
        matches!(err, MyDataError::BadAfm(_)),
        "EL-prefixed AFM must be refused as BadAfm, got {err:?}"
    );
}

/// Determinism for the credit-note shape: the full corrective-document
/// lifecycle (serialize -> report -> pack) must be byte-identical across runs,
/// the same guarantee the baseline invoice lifecycle holds.
#[test]
fn greece_credit_note_lifecycle_is_byte_deterministic() {
    let run = || {
        let doc = greek_credit_note();
        let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::Accepted,
            mark: Some(MyDataMark::new("400000000000042")),
            uid: Some(MyDataUid::new("MYDATA-MOCK-UID-00000042")),
            message: None,
            reported_at: PINNED_REPORTED_AT.to_owned(),
        };
        pack_bundle(&doc, ubl_bytes, &envelope)
    };
    assert_eq!(run(), run(), "credit-note lifecycle must be byte-stable");
}

// ---------------------------------------------------------------------------
// Native myDATA InvoicesDoc lifecycle (the REAL Greek national format).
//
// The scenarios above serialize via the EN 16931 / UBL family path; the
// scenarios below drive the lifecycle over `to_invoices_doc_xml`, the native
// AADE myDATA `InvoicesDoc` serializer (namespace
// `http://www.aade.gr/myDATA/invoice/v1.0`). They serialize -> validate the
// real element/field names against the myDATA XSD shape -> mock transmit ->
// pack a `.ikb` evidence bundle (carrying `formats/invoices_doc.xml`) -> verify.
// Reference: AADE myDATA REST API / XSD,
// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>.
// ---------------------------------------------------------------------------

const SERIES: &str = "A";

/// Pack a document + native myDATA `InvoicesDoc` bytes + authority envelope into
/// a `.ikb` evidence bundle. Mirrors [`pack_bundle`] but lands the national
/// `formats/invoices_doc.xml` artefact instead of the UBL family one.
fn pack_native_bundle(
    doc: &CommercialDocument,
    invoices_doc_bytes: Vec<u8>,
    envelope: &MyDataReportEnvelope,
) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/invoices_doc.xml".to_owned(), invoices_doc_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Structural validation of a native myDATA `InvoicesDoc`: the mandatory spine
/// the AADE XSD requires (issuer/counterpart VAT, invoiceHeader, per-line
/// invoiceDetails, invoiceSummary totals). Reference Schematron/XSD validation
/// stays an external (JVM) backend; this is the local structural gate.
fn assert_invoices_doc_structure(xml: &str) {
    for needle in [
        "<InvoicesDoc xmlns=\"http://www.aade.gr/myDATA/invoice/v1.0\">",
        "<invoice>",
        "<issuer>",
        "<counterpart>",
        "<invoiceHeader>",
        "<series>",
        "<aa>",
        "<issueDate>",
        "<invoiceType>",
        "<invoiceDetails>",
        "<lineNumber>",
        "<netValue>",
        "<vatAmount>",
        "<vatCategory>",
        "<invoiceSummary>",
        "<totalNetValue>",
        "<totalVatAmount>",
        "<totalGrossValue>",
    ] {
        assert!(xml.contains(needle), "native InvoicesDoc missing {needle}");
    }
}

/// Steps 1-4 over the NATIVE myDATA `InvoicesDoc`: build -> serialize (native)
/// -> validate structure -> report (mock IAPR) -> evidence bundle.
fn run_native_lifecycle() -> (Vec<u8>, String, MyDataReportEnvelope) {
    let doc = greek_invoice();
    let ctx = MyDataDocContext {
        series: SERIES.to_owned(),
        issuer_branch: 0,
        counterpart_branch: 0,
    };
    let xml = to_invoices_doc_xml(&doc, &ctx).unwrap();
    assert_invoices_doc_structure(&xml);

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let envelope = provider
        .report_invoice(&report_request(xml.clone().into_bytes()))
        .unwrap();

    let ikb = pack_native_bundle(&doc, xml.clone().into_bytes(), &envelope);
    (ikb, xml, envelope)
}

#[test]
fn greece_native_invoices_doc_lifecycle_produces_verifiable_evidence() {
    let (ikb, xml, envelope) = run_native_lifecycle();

    // The native artefact carries the REAL myDATA element/field names with the
    // 24% standard rate mapped to vatCategory 1 and the per-line + summary
    // totals computed from the IR.
    assert!(xml.contains("<vatNumber>123456789</vatNumber>")); // EL prefix stripped
    assert!(xml.contains("<country>GR</country>"));
    assert!(xml.contains("<invoiceType>1.1</invoiceType>"));
    assert!(xml.contains("<vatCategory>1</vatCategory>"));
    assert!(xml.contains("<netValue>100.00</netValue>"));
    assert!(xml.contains("<vatAmount>24.00</vatAmount>"));
    assert!(xml.contains("<totalNetValue>100.00</totalNetValue>"));
    assert!(xml.contains("<totalVatAmount>24.00</totalVatAmount>"));
    assert!(xml.contains("<totalGrossValue>124.00</totalGrossValue>"));

    // The IAPR accepted and assigned a MARK + UID.
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    let mark = envelope.mark.as_ref().expect("accepted invoice carries a MARK");
    assert!(mark.as_str().starts_with("4000"));

    // The bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native InvoicesDoc evidence bundle must verify");
}

#[test]
fn greece_native_lifecycle_is_byte_deterministic() {
    let (a, xml_a, _) = run_native_lifecycle();
    let (b, xml_b, _) = run_native_lifecycle();
    assert_eq!(xml_a, xml_b, "native InvoicesDoc serialization must be stable");
    assert_eq!(a, b, "the whole native offline lifecycle must be byte-stable");
}

#[test]
fn greece_native_multiline_emits_one_invoice_details_per_line() {
    let doc = greek_multiline_invoice();
    let ctx = MyDataDocContext {
        series: SERIES.to_owned(),
        issuer_branch: 0,
        counterpart_branch: 0,
    };
    let xml = to_invoices_doc_xml(&doc, &ctx).unwrap();
    assert_invoices_doc_structure(&xml);

    // One invoiceDetails row per IR line, in document order.
    assert_eq!(
        xml.matches("<invoiceDetails>").count(),
        2,
        "a two-line invoice must emit two invoiceDetails rows, got:\n{xml}"
    );
    assert!(xml.contains("<lineNumber>1</lineNumber>"));
    assert!(xml.contains("<lineNumber>2</lineNumber>"));
    // Standard 24% (vatCategory 1) and reduced 13% (vatCategory 2) both reach
    // the wire on their respective rows.
    assert!(xml.contains("<vatCategory>1</vatCategory>"));
    assert!(xml.contains("<vatCategory>2</vatCategory>"));
    // Summary totals: net 150.00, vat 24.00 + 6.50 = 30.50, gross 180.50.
    assert!(xml.contains("<totalNetValue>150.00</totalNetValue>"));
    assert!(xml.contains("<totalVatAmount>30.50</totalVatAmount>"));
    assert!(xml.contains("<totalGrossValue>180.50</totalGrossValue>"));

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let envelope = provider
        .report_invoice(&report_request(xml.clone().into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    let ikb = pack_native_bundle(&doc, xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native multi-line evidence bundle must verify");
}

#[test]
fn greece_native_exempt_line_emits_vat_category_7_and_exemption() {
    let doc = greek_exempt_invoice();
    let ctx = MyDataDocContext {
        series: SERIES.to_owned(),
        issuer_branch: 0,
        counterpart_branch: 0,
    };
    let xml = to_invoices_doc_xml(&doc, &ctx).unwrap();
    assert_invoices_doc_structure(&xml);

    // An exempt line maps to myDATA vatCategory 7 (excluding VAT) and the XSD
    // makes vatExemptionCategory mandatory for that category.
    assert!(
        xml.contains("<vatCategory>7</vatCategory>"),
        "an exempt line must map to vatCategory 7, got:\n{xml}"
    );
    assert!(
        xml.contains("<vatExemptionCategory>"),
        "vatCategory 7 requires a vatExemptionCategory per the myDATA XSD"
    );
    assert!(xml.contains("<vatAmount>0.00</vatAmount>"));
    // No VAT charged: gross equals net.
    assert!(xml.contains("<totalNetValue>200.00</totalNetValue>"));
    assert!(xml.contains("<totalVatAmount>0.00</totalVatAmount>"));
    assert!(xml.contains("<totalGrossValue>200.00</totalGrossValue>"));

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let mut req = report_request(xml.clone().into_bytes());
    req.category = MyDataInvoiceCategory::Services {
        code: "2.1".to_owned(),
    };
    let envelope = provider.report_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    let ikb = pack_native_bundle(&doc, xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native exempt-invoice evidence bundle must verify");
}

#[test]
fn greece_native_credit_note_maps_to_invoice_type_5_1_and_bundles() {
    let doc = greek_credit_note();
    let ctx = MyDataDocContext {
        series: SERIES.to_owned(),
        issuer_branch: 0,
        counterpart_branch: 0,
    };
    let xml = to_invoices_doc_xml(&doc, &ctx).unwrap();
    assert_invoices_doc_structure(&xml);

    // A credit note maps to the myDATA associated-credit-note invoiceType 5.1.
    assert!(
        xml.contains("<invoiceType>5.1</invoiceType>"),
        "a credit note must map to myDATA invoiceType 5.1, got:\n{xml}"
    );
    assert!(xml.contains("<aa>CN-2026-GR-0001</aa>"));
    // The fixture references the corrected invoice (kind "invoice" ->
    // PrecedingInvoice). The associated-credit-note link must reach the wire as
    // the myDATA correlatedInvoices element, carrying the referenced id verbatim.
    assert!(
        xml.contains("<correlatedInvoices>INV-2026-GR-0001</correlatedInvoices>"),
        "an associated credit note must link the original invoice via \
         correlatedInvoices, got:\n{xml}"
    );

    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let mut req = report_request(xml.clone().into_bytes());
    req.category = MyDataInvoiceCategory::CreditNote {
        code: "5.1".to_owned(),
    };
    let envelope = provider.report_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    let ikb = pack_native_bundle(&doc, xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native credit-note evidence bundle must verify");
}
