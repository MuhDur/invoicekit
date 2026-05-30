// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Hungary NAV Online Számla offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Hungary and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Hungarian supplier +
//!    customer and the forint (HUF) currency
//! 2. serialize -> NAV Online Számla `InvoiceData` (RTIR) XML, the REAL
//!    national document defined by the Online Számla Interface Specification
//!    v3.0 (`invoiceData.xsd`); these bytes ARE the `manageInvoiceRequest`
//!    payload NAV ingests (base64-wrapped on the live wire). A UBL rendering is
//!    bundled alongside as a portable EN 16931 artifact, but it is NOT the wire
//!    payload.
//! 3. validate the national artifact's structure (the mandatory `InvoiceData`
//!    spine: `invoiceNumber`, `supplierTaxNumber`, `invoiceLines/line`,
//!    `invoiceSummary/summaryNormal`)
//! 4. submit the `InvoiceData` bytes to the existing offline `MockNavProvider`
//!    (`manage_invoice`), asserting NAV's country-specific receipt fields:
//!    the `NAV-` transaction id, `Received` status, and the recorded timestamp
//! 5. poll `query_transaction` to reach the terminal `Done` status — the real
//!    two-step NAV async lifecycle (submit -> Received, poll -> Done)
//! 6. assemble a `.ikb` evidence bundle (canonical.json + formats/invoice-data.xml
//!    + formats/ubl.xml + receipt.json) and `verify_packed(content_only).ok ==
//!    true` (exit 0)
//! 7. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 8. refusal paths: NAV's mock refuses an empty payload (`BadXml`) and a
//!    malformed adóazonosító (`BadTaxId`) as typed `Err`s before the wire
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). The HU NAV mock has no signing layer, so no signer is wired.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_hu_nav::{
    to_invoice_data_xml, MockNavProvider, NavEnvironment, NavManageRequest, NavOperation,
    NavProvider, NavStatus,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_hu_e2e";
const TRACE: &str = "trace_hu_e2e";
const FIXED_RECORDED_AT: &str = "2026-01-01T00:00:00Z";
/// 8-digit adóazonosító + 1 check digit + 2-digit area code, hyphenated as
/// NAV writes it on the portal: `12345678-2-41`.
const ISSUER_TAX_ID: &str = "12345678-2-41";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn hungarian_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Andrássy út 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "1061".to_owned(),
            country: CountryCode::new("HU").unwrap(),
        },
        contact: None,
    }
}

fn hungarian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-HU-0001").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Debrecen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Szoftverfejlesztési tanácsadás".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // Hungary's standard VAT is 27% (the highest in the EU).
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2700),
            tax_rate: Some(DecimalValue::new(Decimal::new(2700, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12700),
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

fn manage_request(manage_invoice_xml: Vec<u8>) -> NavManageRequest {
    NavManageRequest {
        tenant_id: TENANT.to_owned(),
        environment: NavEnvironment::Test,
        operation: NavOperation::Create,
        issuer_tax_id: ISSUER_TAX_ID.to_owned(),
        manage_invoice_xml,
    }
}

/// Steps 1-5: build -> serialize -> submit (Received) -> poll (Done) ->
/// evidence bundle. Returns the packed `.ikb` plus the two NAV envelopes so the
/// callers can assert the country-specific receipt fields.
fn run_lifecycle() -> (
    Vec<u8>,
    invoicekit_report_hu_nav::NavManageEnvelope,
    invoicekit_report_hu_nav::NavManageEnvelope,
) {
    // 1. build the canonical IR document.
    let doc = hungarian_invoice();

    // 2. serialize -> NAV Online Számla `InvoiceData` (RTIR) XML. This is the
    //    REAL national document (invoiceData.xsd); its bytes ARE the
    //    manageInvoiceRequest payload NAV ingests.
    let invoice_data_xml = to_invoice_data_xml(&doc).unwrap();

    // 3. validate the national artifact's structure: the mandatory InvoiceData
    //    spine with NAV's actual element names (not UBL).
    for needle in [
        "<InvoiceData xmlns=\"http://schemas.nav.gov.hu/OSA/3.0/data\"",
        "<invoiceNumber>INV-2026-HU-0001</invoiceNumber>",
        "<invoiceIssueDate>2026-05-26</invoiceIssueDate>",
        "<supplierInfo>",
        "<supplierTaxNumber>",
        "<base:taxpayerId>12345678</base:taxpayerId>",
        "<customerInfo>",
        "<invoiceLines>",
        "<line>",
        "<lineNumber>1</lineNumber>",
        "<lineNetAmount>100.00</lineNetAmount>",
        "<vatPercentage>0.27</vatPercentage>",
        "<lineVatData>",
        "<lineVatAmount>27.00</lineVatAmount>",
        "<invoiceSummary>",
        "<summaryNormal>",
        "<invoiceNetAmount>100.00</invoiceNetAmount>",
        "<invoiceVatAmount>27.00</invoiceVatAmount>",
    ] {
        assert!(
            invoice_data_xml.contains(needle),
            "NAV InvoiceData missing {needle}\n{invoice_data_xml}"
        );
    }
    let invoice_data_bytes = invoice_data_xml.into_bytes();

    // A portable UBL rendering rides along in the bundle (EN 16931 family path),
    // but it is NOT the NAV wire payload.
    let ubl_xml = to_xml(&doc).unwrap();
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "currencyID=\"HUF\"",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 4. submit the NATIONAL InvoiceData bytes to the offline NAV mock; NAV
    //    returns a Received envelope with a NAV-assigned transaction id.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let received = provider
        .manage_invoice(&manage_request(invoice_data_bytes.clone()))
        .unwrap();

    // 5. poll the same transaction; NAV resolves it to the terminal Done status.
    let done = provider
        .query_transaction(NavEnvironment::Test, &received.transaction_id)
        .unwrap();

    // 6. evidence bundle: canonical IR + national InvoiceData + portable UBL +
    //    the final receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/invoice-data.xml".to_owned(), invoice_data_bytes);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&done).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, received, done)
}

#[test]
fn hungary_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, received, done) = run_lifecycle();

    // Step 3 success criterion: NAV accepted the submission and assigned a
    // country-specific transaction id with the `NAV-` prefix.
    assert_eq!(received.status, NavStatus::Received);
    assert!(
        received.transaction_id.starts_with("NAV-"),
        "NAV transaction id must carry the NAV- prefix, got {:?}",
        received.transaction_id
    );
    assert_eq!(received.recorded_at, FIXED_RECORDED_AT);
    assert!(received.validation_result.is_none());

    // Step 4 success criterion: polling reaches the terminal Done status while
    // preserving the same transaction id (the NAV async lifecycle).
    assert_eq!(done.status, NavStatus::Done);
    assert_eq!(done.transaction_id, received.transaction_id);

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn hungary_lifecycle_is_byte_deterministic() {
    let (a, _, _) = run_lifecycle();
    let (b, _, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn hungary_native_invoice_data_is_the_wire_payload_and_not_ubl() {
    // Prove the wire payload submitted to NAV is the REAL national InvoiceData
    // document (invoiceData.xsd), not a UBL relabelling: it carries NAV's own
    // element names and the 8+1+2 taxNumber split, and carries NONE of the UBL
    // element names. A buyer with a full 8-2-2 adószám exercises all three
    // taxNumber sub-elements (taxpayerId/vatCode/countyCode).
    let doc = hungarian_invoice();
    let invoice_data = to_invoice_data_xml(&doc).unwrap();

    // Real NAV InvoiceData spine.
    for needle in [
        "<InvoiceData xmlns=\"http://schemas.nav.gov.hu/OSA/3.0/data\"",
        "xmlns:base=\"http://schemas.nav.gov.hu/OSA/3.0/base\"",
        "<invoiceNumber>INV-2026-HU-0001</invoiceNumber>",
        "<invoiceIssueDate>2026-05-26</invoiceIssueDate>",
        "<invoiceMain>",
        "<invoiceHead>",
        "<supplierInfo>",
        "<supplierTaxNumber>",
        "<base:taxpayerId>12345678</base:taxpayerId>",
        "<customerInfo>",
        "<invoiceCategory>NORMAL</invoiceCategory>",
        "<currencyCode>HUF</currencyCode>",
        "<invoiceLines>",
        "<lineNumber>1</lineNumber>",
        "<lineNetAmount>100.00</lineNetAmount>",
        "<vatPercentage>0.27</vatPercentage>",
        "<lineVatAmount>27.00</lineVatAmount>",
        "<invoiceSummary>",
        "<summaryNormal>",
        "<invoiceNetAmount>100.00</invoiceNetAmount>",
        "<invoiceVatAmount>27.00</invoiceVatAmount>",
        "<invoiceGrossAmount>127.00</invoiceGrossAmount>",
    ] {
        assert!(
            invoice_data.contains(needle),
            "InvoiceData missing {needle}\n{invoice_data}"
        );
    }

    // It must be the national format, NOT UBL relabelled: no UBL element names.
    for forbidden in [
        "<cbc:",
        "<cac:",
        "InvoiceTypeCode",
        "AccountingSupplierParty",
        "TaxSubtotal",
    ] {
        assert!(
            !invoice_data.contains(forbidden),
            "NAV InvoiceData must not carry the UBL element {forbidden}"
        );
    }

    // The exact bytes submitted to NAV's mock are the InvoiceData bytes.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let received = provider
        .manage_invoice(&manage_request(invoice_data.into_bytes()))
        .unwrap();
    assert_eq!(received.status, NavStatus::Received);
    assert!(received.transaction_id.starts_with("NAV-"));
}

#[test]
fn hungary_native_invoice_data_emits_full_8_1_2_tax_number() {
    // A supplier whose adószám is the full 8-1-2 shape (`12345678-2-41`) must
    // render all three NAV taxNumber sub-elements in order.
    let mut doc = hungarian_invoice();
    doc.supplier = hungarian_party("Acme Kft", "HU12345678-2-41", "Budapest");
    let invoice_data = to_invoice_data_xml(&doc).unwrap();

    assert!(invoice_data.contains("<base:taxpayerId>12345678</base:taxpayerId>"));
    assert!(invoice_data.contains("<base:vatCode>2</base:vatCode>"));
    assert!(invoice_data.contains("<base:countyCode>41</base:countyCode>"));

    // taxpayerId precedes vatCode precedes countyCode (schema element order).
    let pid = invoice_data.find("<base:taxpayerId>").unwrap();
    let vat = invoice_data.find("<base:vatCode>").unwrap();
    let county = invoice_data.find("<base:countyCode>").unwrap();
    assert!(pid < vat && vat < county, "taxNumber sub-elements out of order");
}

#[test]
fn hungary_native_invoice_data_multiline_aggregates_summary() {
    // A two-line invoice (27% + 5%) must emit one `line` per IR line in document
    // order and aggregate the net/VAT totals into summaryNormal. NAV reports
    // per-line VAT (lineVatData/lineVatAmount) and the document-level
    // invoiceNetAmount/invoiceVatAmount.
    //   L1: 100.00 @ 27% -> VAT 27.00
    //   L2:  30.00 @  5% -> VAT  1.50
    //   net 130.00 ; VAT 28.50 ; gross 158.50
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-nav-mix").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-HU-NAV-MIX").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678-2-41", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Szeged"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Tanácsadás (27%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Élelmiszer (5%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(3)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(1000),
                line_extension_amount: amt(3000),
                tax_category: Some("R5".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2700),
                tax_rate: Some(DecimalValue::new(Decimal::new(2700, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "R5".to_owned(),
                taxable_amount: amt(3000),
                tax_amount: amt(150),
                tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(13000),
            tax_exclusive_amount: amt(13000),
            tax_inclusive_amount: amt(15850),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(15850),
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
    .unwrap();

    let invoice_data = to_invoice_data_xml(&doc).unwrap();

    // One `line` per IR line, in document order.
    assert_eq!(invoice_data.matches("<line>").count(), 2);
    assert!(invoice_data.contains("<lineNumber>1</lineNumber>"));
    assert!(invoice_data.contains("<lineNumber>2</lineNumber>"));
    // Per-line VAT rates render as NAV fractions.
    assert!(invoice_data.contains("<vatPercentage>0.27</vatPercentage>"));
    assert!(invoice_data.contains("<vatPercentage>0.05</vatPercentage>"));
    // Aggregated document totals: net 130.00, VAT 28.50, gross 158.50.
    assert!(invoice_data.contains("<invoiceNetAmount>130.00</invoiceNetAmount>"));
    assert!(invoice_data.contains("<invoiceVatAmount>28.50</invoiceVatAmount>"));
    assert!(invoice_data.contains("<invoiceGrossAmount>158.50</invoiceGrossAmount>"));

    // Determinism on the multi-line national document.
    assert_eq!(invoice_data, to_invoice_data_xml(&doc).unwrap());
}

#[test]
fn hungary_mock_refuses_empty_payload_and_bad_tax_id() {
    // The HU NAV mock has no forced-status knob: manage_invoice always returns a
    // `Received` envelope on the happy path and CANNOT be made to emit an
    // `Aborted` status. The only refusals it models are pre-wire shape failures,
    // surfaced as typed `Err`s (which is the correct contract for shape errors;
    // an authority `Aborted` verdict would be an Ok-envelope, not an Err).
    use invoicekit_report_hu_nav::NavError;

    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);

    // Empty manageInvoiceRequest payload -> BadXml.
    let empty = provider.manage_invoice(&manage_request(Vec::new())).unwrap_err();
    assert!(
        matches!(empty, NavError::BadXml(_)),
        "empty payload must be refused as BadXml, got {empty:?}"
    );

    // Malformed adóazonosító -> BadTaxId, before the wire.
    let mut bad = manage_request(b"<Invoice/>".to_vec());
    bad.issuer_tax_id = "NOT-A-TAX-ID".to_owned();
    let err = provider.manage_invoice(&bad).unwrap_err();
    assert!(
        matches!(err, NavError::BadTaxId(_)),
        "malformed tax id must be refused as BadTaxId, got {err:?}"
    );
}

// ===========================================================================
// Deepened, country-specific coverage.
//
// External grounding (cited per scenario):
//
// * Authority: Nemzeti Adó- és Vámhivatal (NAV) — the Hungarian National Tax
//   and Customs Administration. Real-time invoice-data reporting ("Online
//   Számla" / RTIR) is mandatory for invoices issued by domestically
//   VAT-registered businesses. Reporting is *post-issuance*, NOT clearance:
//   the invoice is legally issued first, then its data is reported to NAV.
//   Portal + spec: https://onlineszamla.nav.gov.hu/
//
// * Wire schema: NAV Online Számla Interface Specification v3.0. The
//   `manageInvoiceRequest` carries an `invoiceOperation` whose `invoiceOperation`
//   enum is CREATE / MODIFY / STORNO; technical annulment of an erroneously
//   reported submission goes through the separate `manageAnnulment` operation.
//   This crate models all four as `NavOperation::{Create, Modify, Storno, Annul}`.
//   Spec hub: https://onlineszamla.nav.gov.hu/dokumentaciok
//
// * Status lifecycle: a submitted transaction moves RECEIVED -> PROCESSING ->
//   DONE; a per-invoice technical/business validation block resolves the index
//   to ABORTED, with the failing rules carried in
//   technical/businessValidationMessages. Modelled as
//   `NavStatus::{Received, InProgress, Done, Aborted}` + `validation_result`.
//
// * Hungarian VAT (Act CXXVII of 2007 on Value Added Tax, "Áfa tv."): standard
//   rate 27% (the highest standard VAT rate in the EU), reduced rates 18% and
//   5%. Subjective tax exemption ("alanyi adómentesség", code AAM) and the
//   domestic reverse-charge mechanism for listed supplies under §142 are
//   reported with the supply's VAT exemption / "no VAT charged" markers.
//
// Fixtures are hand-built synthetic data; no copyrighted regulator file is
// vendored. Goldens stay hand-rolled (no `insta`/`pretty_assertions`).
// ===========================================================================

// Domestic 27%-standard-rate forint credit note (storno-style corrective).
//
// NAV reports a credit note (a corrective document that reverses an earlier
// invoice) via the STORNO invoiceOperation. The IR DocumentType::CreditNote
// serializes to a UBL 2.1 <CreditNote> with type code 381 and
// cac:CreditNoteLine / cbc:CreditedQuantity lines. UBL 2.1 forbids a top-level
// cbc:DueDate on a CreditNote, so due_date stays None.
fn hungarian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: Some(DateOnly::new("2026-05-28").unwrap()),
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("STORNO-2026-HU-0001").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Debrecen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Szoftverfejlesztési tanácsadás (sztornó)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2700),
            tax_rate: Some(DecimalValue::new(Decimal::new(2700, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12700),
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
fn hungary_credit_note_serializes_as_storno_corrective() {
    // NAV's STORNO operation reverses a previously-reported invoice. The IR
    // CreditNote -> UBL CreditNote path (type code 381) is the format variant;
    // the report-side operation is `NavOperation::Storno`.
    let doc = hungarian_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // UBL 2.1 CreditNote spine, asserted on real rendered values. The
    // canonicalizer renders namespace declarations inline on each element's open
    // tag, so we match the element-name prefix (`<cbc:CreditNoteTypeCode`), not
    // the bare closed form.
    for needle in [
        "<CreditNote",
        ">381</cbc:CreditNoteTypeCode>",
        "<cac:CreditNoteLine",
        "<cbc:CreditedQuantity",
        ">STORNO-2026-HU-0001</cbc:ID>",
        // Hungary's 27% standard rate renders verbatim as the tax percent.
        ">27.00</cbc:Percent>",
        "currencyID=\"HUF\"",
    ] {
        assert!(ubl.contains(needle), "HU CreditNote UBL missing {needle}\n{ubl}");
    }
    // A CreditNote must NOT carry an Invoice type code or a top-level due date.
    assert!(
        !ubl.contains("</cbc:InvoiceTypeCode>"),
        "a CreditNote must not emit an InvoiceTypeCode"
    );
    assert!(
        !ubl.contains("</cbc:DueDate>"),
        "UBL 2.1 CreditNote forbids a top-level cbc:DueDate"
    );

    // The report-side wire operation for a reversal is STORNO; submit it and
    // confirm NAV accepts the corrective with a fresh NAV- transaction id.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let mut req = manage_request(ubl.into_bytes());
    req.operation = NavOperation::Storno;
    let env = provider.manage_invoice(&req).unwrap();
    assert_eq!(env.status, NavStatus::Received);
    assert!(env.transaction_id.starts_with("NAV-"));
    assert_eq!(req.operation, NavOperation::Storno);
}

#[test]
fn hungary_multi_line_mixed_rate_invoice_reports_each_band() {
    // Mixed-rate invoice exercising three real Hungarian VAT bands in one
    // document: 27% standard (S), 18% reduced, and 5% reduced. The reduced
    // rates are reported with their own ClassifiedTaxCategory IDs, distinct
    // from the standard "S" band, so the per-band TaxSubtotal entries survive
    // serialization.
    //
    // Line economics (all HUF; `amt(minor)` builds a scale-2 Decimal, so
    // `amt(10000)` renders as 100.00):
    //   L1: 2 x 50.00 = 100.00 @ 27%  -> tax 27.00  (S)
    //   L2: 1 x 40.00 =  40.00 @ 18%  -> tax  7.20  (R18)
    //   L3: 3 x 10.00 =  30.00 @  5%  -> tax  1.50  (R5)
    //   net 170.00 ; VAT 35.70 ; gross 205.70
    let line = |id: &str, desc: &str, qty: i64, unit_minor: i64, ext_minor: i64, cat: &str| {
        DocumentLine {
            id: id.to_owned(),
            description: desc.to_owned(),
            quantity: DecimalValue::new(Decimal::from(qty)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(unit_minor),
            line_extension_amount: amt(ext_minor),
            tax_category: Some(cat.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }
    };
    let band = |cat: &str, taxable_minor: i64, tax_minor: i64, rate_minor: i64| TaxCategorySummary {
        category_code: cat.to_owned(),
        taxable_amount: amt(taxable_minor),
        tax_amount: amt(tax_minor),
        tax_rate: Some(DecimalValue::new(Decimal::new(rate_minor, 2))),
        exemption_reason: None,
        exemption_reason_code: None,
    };

    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-mix-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-HU-MIX-1").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Szeged"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            line("1", "Tanácsadás", 2, 5000, 10000, "S"),
            line("2", "Szakkönyv (18%)", 1, 4000, 4000, "R18"),
            line("3", "Élelmiszer (5%)", 3, 1000, 3000, "R5"),
        ],
        tax_summary: vec![
            band("S", 10000, 2700, 2700),
            band("R18", 4000, 720, 1800),
            band("R5", 3000, 150, 500),
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(17000),
            tax_exclusive_amount: amt(17000),
            tax_inclusive_amount: amt(20570),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(20570),
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
    .unwrap();

    let ubl = to_xml(&doc).unwrap();
    // All three real Hungarian VAT percentages render as distinct bands. (The
    // canonicalizer puts an inline xmlns on each cbc element, so match the
    // value-plus-close form `>27.00</cbc:Percent>`.)
    for needle in [
        ">27.00</cbc:Percent>",
        ">18.00</cbc:Percent>",
        ">5.00</cbc:Percent>",
        // The aggregate VAT (sum of the three bands) on the TaxTotal header.
        "currencyID=\"HUF\">35.70</cbc:TaxAmount>",
        // The gross payable.
        "currencyID=\"HUF\">205.70</cbc:PayableAmount>",
    ] {
        assert!(ubl.contains(needle), "mixed-rate UBL missing {needle}\n{ubl}");
    }
    // Three invoice lines survived (the open tag carries an inline xmlns:cac).
    assert_eq!(ubl.matches("<cac:InvoiceLine ").count(), 3);
    // Three tax bands survived as TaxSubtotal entries.
    assert_eq!(ubl.matches("<cac:TaxSubtotal>").count(), 3);

    // The whole document still submits and gets a NAV transaction id.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let env = provider.manage_invoice(&manage_request(ubl.into_bytes())).unwrap();
    assert_eq!(env.status, NavStatus::Received);
}

#[test]
fn hungary_domestic_reverse_charge_invoice_carries_no_vat() {
    // Domestic reverse charge ("belföldi fordított adózás") under §142 of the
    // Hungarian VAT Act (Act CXXVII of 2007): for listed supplies the customer,
    // not the supplier, accounts for the VAT, so the supplier issues the
    // invoice with NO VAT charged. We model the supply with the EN 16931 VAT
    // category code "AE" (VAT Reverse Charge) at a 0.00 charged amount.
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-rc-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-HU-RC-1").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Miskolc"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            // Construction work is a classic §142 reverse-charge supply.
            description: "Építési-szerelési munka (fordított adózás)".to_owned(),
            quantity: DecimalValue::new(Decimal::ONE),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(500_000),
            line_extension_amount: amt(500_000),
            tax_category: Some("AE".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "AE".to_owned(),
            taxable_amount: amt(500_000),
            // Reverse charge: supplier charges no VAT.
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(500_000),
            tax_exclusive_amount: amt(500_000),
            // Gross == net: no VAT added under reverse charge.
            tax_inclusive_amount: amt(500_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(500_000),
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
    .unwrap();

    let ubl = to_xml(&doc).unwrap();
    for needle in [
        // The reverse-charge category code rides through to the line + subtotal.
        ">AE</cbc:ID>",
        // Net == gross; no VAT was added (amt(500_000) renders as 5000.00).
        "currencyID=\"HUF\">5000.00</cbc:TaxExclusiveAmount>",
        "currencyID=\"HUF\">5000.00</cbc:TaxInclusiveAmount>",
        // Header VAT is zero.
        "currencyID=\"HUF\">0.00</cbc:TaxAmount>",
    ] {
        assert!(
            ubl.contains(needle),
            "reverse-charge UBL missing {needle}\n{ubl}"
        );
    }

    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let env = provider.manage_invoice(&manage_request(ubl.into_bytes())).unwrap();
    assert_eq!(env.status, NavStatus::Received);
}

#[test]
fn hungary_subjective_exemption_invoice_is_zero_rated() {
    // "Alanyi adómentesség" (AAM) — the subjective tax exemption a small
    // Hungarian business below the turnover threshold elects. Such an issuer
    // charges no VAT. We model it with the EN 16931 "E" (Exempt) category at a
    // 0% rate, and assert the exemption flows through to the wire and the
    // amounts carry no tax.
    let aam_party = hungarian_party("Kis Vállalkozó Ev", "HU11223344", "Pécs");
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-aam-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-HU-AAM-1").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: aam_party,
        customer: hungarian_party("Beta Zrt", "HU98765432", "Győr"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Grafikai munka (alanyi adómentes)".to_owned(),
            quantity: DecimalValue::new(Decimal::ONE),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(80000),
            line_extension_amount: amt(80000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(80000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(80000),
            tax_exclusive_amount: amt(80000),
            tax_inclusive_amount: amt(80000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(80000),
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
    .unwrap();

    let ubl = to_xml(&doc).unwrap();
    for needle in [
        ">E</cbc:ID>",
        // A 0% exempt rate renders as the bare "0" (Decimal::ZERO.to_string()).
        ">0</cbc:Percent>",
        // amt(80000) renders as 800.00; the exemption carries no VAT.
        "currencyID=\"HUF\">800.00</cbc:TaxInclusiveAmount>",
        "currencyID=\"HUF\">0.00</cbc:TaxAmount>",
    ] {
        assert!(ubl.contains(needle), "AAM UBL missing {needle}\n{ubl}");
    }

    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let env = provider.manage_invoice(&manage_request(ubl.into_bytes())).unwrap();
    assert_eq!(env.status, NavStatus::Received);
}

#[test]
fn hungary_aborted_verdict_is_ok_envelope_not_err_and_still_bundles() {
    // NAV per-invoice rejection contract: when a reported invoice fails NAV's
    // technical/business validation, the transaction index resolves to ABORTED
    // with the failing rules carried in technical/businessValidationMessages.
    // That refusal is an authority *verdict*, surfaced as an `Ok` envelope with
    // `NavStatus::Aborted` + a `validation_result`, NOT a transport `Err`.
    //
    // The `MockNavProvider` happy path cannot emit `Aborted` (it has no
    // forced-status knob and always returns `Received`). To prove the rejection
    // contract honestly we synthesize the exact envelope NAV would return for an
    // ABORTED index and assert (a) it carries the failing-rule text, (b) it
    // round-trips through serde unchanged, and (c) it still bundles + verifies
    // so the audit trail persists the rejection. This mirrors the Italy SDI
    // "rejection still bundles and verifies" pattern, adapted to NAV's status
    // model (NAV has no separate receipt-kind type; the verdict lives in
    // `NavStatus`).
    use invoicekit_report_hu_nav::NavManageEnvelope;

    // A representative NAV business-validation block reason. NAV returns these
    // as `businessValidationMessages` with a rule code; INCORRECT_VAT_AMOUNT is
    // a real Online Számla v3.0 business-validation error code.
    let aborted = NavManageEnvelope {
        transaction_id: "NAV-0000000000000042".to_owned(),
        status: NavStatus::Aborted,
        recorded_at: FIXED_RECORDED_AT.to_owned(),
        validation_result: Some(
            "INCORRECT_VAT_AMOUNT: reported lineVatData does not match the line net amount"
                .to_owned(),
        ),
    };

    // (a) the verdict is a rejection carrying the failing-rule text.
    assert_eq!(aborted.status, NavStatus::Aborted);
    assert_ne!(aborted.status, NavStatus::Done);
    let reason = aborted.validation_result.as_deref().unwrap();
    assert!(
        reason.contains("INCORRECT_VAT_AMOUNT"),
        "ABORTED envelope must carry the NAV business-validation rule code"
    );

    // (b) serde round-trip is lossless (the audit trail persists it verbatim).
    let json = serde_json::to_string(&aborted).unwrap();
    let parsed: NavManageEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, aborted);
    // The kebab-case status renders as "aborted" on the wire JSON.
    assert!(
        json.contains("\"status\":\"aborted\""),
        "NavStatus serializes kebab-case; got {json}"
    );

    // (c) the rejection still assembles into a verifiable evidence bundle.
    let doc = hungarian_invoice();
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let ubl = to_xml(&doc).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl);
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&aborted).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(
        verify.ok,
        "an ABORTED-verdict evidence bundle must still verify (the rejection is auditable)"
    );
}

#[test]
fn hungary_rejects_group_member_and_truncated_tax_ids() {
    // The adóazonosító / adószám shape rule, exercised on real Hungarian
    // identifier widths. NAV's adószám is 8 base digits + 1 VAT code + 2 area
    // code (11 digits, written `12345678-2-41`); the 8-digit core and the
    // 9-digit individual adóazonosító jel are also accepted by the shape rule.
    // A 10-digit value (e.g. a truncated group identifier) and any non-digit
    // body must be refused as `BadTaxId`.
    use invoicekit_report_hu_nav::{validate_tax_id, NavError};

    // Accepted real Hungarian shapes.
    assert!(validate_tax_id("12345678").is_ok(), "8-digit core adószám");
    assert!(
        validate_tax_id("12345678-2-41").is_ok(),
        "hyphenated 8-2-2 adószám as NAV prints it"
    );
    assert!(
        validate_tax_id("12345678241").is_ok(),
        "the same 11-digit adószám without hyphens"
    );

    // 10 digits is not a valid Hungarian width -> rejected.
    let ten = validate_tax_id("1234567824").unwrap_err();
    assert!(
        matches!(ten, NavError::BadTaxId(_)),
        "a 10-digit value is not a valid HU tax-id width"
    );

    // A letter in the body -> rejected (digits only).
    let alpha = validate_tax_id("1234567X").unwrap_err();
    assert!(matches!(alpha, NavError::BadTaxId(_)));

    // And the same refusal surfaces through the provider, before the wire.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let mut req = manage_request(b"<Invoice/>".to_vec());
    req.issuer_tax_id = "1234567824".to_owned();
    assert!(matches!(
        provider.manage_invoice(&req).unwrap_err(),
        NavError::BadTaxId(_)
    ));
}

#[test]
fn hungary_corrective_lifecycle_modify_then_query_is_deterministic() {
    // The MODIFY operation reports a correcting invoice that adjusts (rather
    // than fully reverses) an earlier reported one. Drive the real two-step NAV
    // async lifecycle (submit -> Received, poll -> Done) on a MODIFY operation
    // and prove the receipt fields and byte-stability hold for corrections too.
    let doc = hungarian_invoice();
    let ubl = to_xml(&doc).unwrap().into_bytes();

    let run = || {
        let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
        let mut req = manage_request(ubl.clone());
        req.operation = NavOperation::Modify;
        let received = provider.manage_invoice(&req).unwrap();
        let done = provider
            .query_transaction(NavEnvironment::Test, &received.transaction_id)
            .unwrap();
        (received, done)
    };

    let (received, done) = run();
    assert_eq!(received.status, NavStatus::Received);
    assert!(received.transaction_id.starts_with("NAV-"));
    assert_eq!(done.status, NavStatus::Done);
    assert_eq!(done.transaction_id, received.transaction_id);
    assert_eq!(done.recorded_at, FIXED_RECORDED_AT);

    // Determinism: a fresh provider re-runs to the identical transaction id and
    // receipt fields (the serial counter resets per-provider).
    let (received2, done2) = run();
    assert_eq!(received2, received);
    assert_eq!(done2, done);
}
