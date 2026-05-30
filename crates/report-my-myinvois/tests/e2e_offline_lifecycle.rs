// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Malaysia **`MyInvois`** (LHDNM) offline end-to-end lifecycle.
//!
//! Drives the full local-only chain for Malaysia and proves it deterministically,
//! mirroring the reference Italy SDI pattern in `report-it-sdi`:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("MY")` + MYR.
//! 2. serialize to UBL 2.1 bytes via `invoicekit_format_ubl::to_xml` (the
//!    EN16931/UBL family path — LHDNM's schema is PEPPOL-derived UBL).
//! 3. submit those bytes to the existing `MockMyInvoisProvider` and assert the
//!    LHDNM-issued authority fields: UUID, 64-char content hash, Long ID, status.
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), pack, and `verify_packed(content_only).ok == true`.
//! 5. determinism: pack twice -> byte-identical.
//! 6. refusal: the mock validates TIN/BRN/empty-payload shape before the wire
//!    and returns `Err`; it does NOT expose a way to force an authority-side
//!    `MyInvoisStatus::Rejected` envelope, so this file exercises the
//!    pre-wire refusal path (the only refusal the mock supports) and documents
//!    that distinction.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_my_myinvois::{
    MockMyInvoisProvider, MyInvoisDocumentKind, MyInvoisEnvironment, MyInvoisError,
    MyInvoisProvider, MyInvoisStatus, MyInvoisSubmitEnvelope, MyInvoisSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_my_e2e";
const TRACE: &str = "trace_my_e2e";
const ISSUER_TIN: &str = "C1234567890";
const ISSUER_BRN: &str = "202301234567";
const BUYER_TIN: &str = "C9876543210";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn malaysian_party(name: &str, vat: &str, city: &str, state: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Jalan Ampang 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(state.to_owned()),
            postal_code: "50450".to_owned(),
            country: CountryCode::new("MY").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Malaysian B2B invoice in IR, denominated in MYR.
fn malaysian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-my-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-MY-0001").unwrap(),
        currency: Iso4217Code::new("MYR").unwrap(),
        supplier: malaysian_party("Acme Sdn Bhd", "MY1234567890", "Kuala Lumpur", "14"),
        customer: malaysian_party("Beta Sdn Bhd", "MY9876543210", "George Town", "07"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cloud platform subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Malaysia's standard SST rate is 6%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(600),
            tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(10600),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(10600),
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

fn submit_request(invoice_xml: Vec<u8>) -> MyInvoisSubmitRequest {
    MyInvoisSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: MyInvoisEnvironment::Sandbox,
        kind: MyInvoisDocumentKind::Invoice,
        issuer_tin: ISSUER_TIN.to_owned(),
        issuer_brn: ISSUER_BRN.to_owned(),
        buyer_tin: Some(BUYER_TIN.to_owned()),
        invoice_xml,
    }
}

/// Assemble + pack an `.ikb` evidence bundle from the canonical IR document, its
/// UBL serialization, and the LHDNM receipt envelope.
///
/// Bundle layout (matching the reference Italy SDI pattern): `canonical.json`
/// (the canonicalized IR) + `formats/ubl.xml` (the national serialization) +
/// `receipt.json` (the authority envelope).
fn pack_evidence(doc: &CommercialDocument, ubl_bytes: Vec<u8>, envelope: &MyInvoisSubmitEnvelope) -> Vec<u8> {
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
    pack(&EvidenceBundle { manifest, artefacts }).unwrap()
}

/// Steps 1-4: build -> serialize -> submit -> assemble + pack evidence bundle.
///
/// Returns the packed `.ikb` bytes and the LHDNM envelope so callers can assert
/// either the authority artefacts or the byte-level determinism.
fn run_lifecycle() -> (Vec<u8>, MyInvoisSubmitEnvelope) {
    // 1. build IR
    let doc = malaysian_invoice();

    // 2. serialize -> UBL 2.1 bytes (LHDNM's schema is PEPPOL/UBL-derived)
    let ubl = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();

    // 3. submit the UBL bytes to the existing offline mock provider
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national UBL + authority receipt
    let ikb = pack_evidence(&doc, ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn malaysia_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: LHDNM accepted and stamped the invoice.
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);
    assert!(
        envelope.uuid.starts_with("MOCK-UUID-"),
        "LHDNM UUID must be present"
    );
    assert!(
        envelope.long_id.starts_with("MOCK-LONG-ID-"),
        "buyer-facing Long ID must be present"
    );
    assert_eq!(
        envelope.content_hash_hex.len(),
        64,
        "LHDNM content hash is a 64-char hex digest"
    );
    assert!(
        envelope.content_hash_hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "content hash must be hex"
    );
    assert_eq!(envelope.submitted_at, PINNED_CREATED_AT);
    assert!(envelope.rejection_reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn malaysia_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn malaysia_cancellation_within_grace_window_records_cancelled() {
    // LHDNM allows the buyer to cancel within the 72-hour grace window; the
    // mock's `cancel_invoice` always succeeds and returns `Cancelled`.
    let (_, envelope) = run_lifecycle();
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let cancelled = provider
        .cancel_invoice(
            MyInvoisEnvironment::Sandbox,
            &envelope.uuid,
            "buyer requested cancellation",
        )
        .unwrap();
    assert_eq!(cancelled.status, MyInvoisStatus::Cancelled);
    assert_eq!(cancelled.uuid, envelope.uuid);
    assert_eq!(
        cancelled.rejection_reason.as_deref(),
        Some("buyer requested cancellation")
    );
}

#[test]
fn malaysia_refusal_is_a_prewire_error_not_a_rejected_envelope() {
    // NOTE: the `MockMyInvoisProvider` does NOT expose a way to force an
    // authority-side `MyInvoisStatus::Rejected` envelope (its happy path always
    // returns `Submitted`). The only refusal it supports is pre-wire shape
    // validation, which surfaces as `Err(MyInvoisError)`. We exercise all three
    // pre-wire refusal buckets here.
    let doc = malaysian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);

    // (a) bad issuer TIN (must be `C` + 10 digits).
    let mut bad_tin = submit_request(ubl_bytes.clone());
    bad_tin.issuer_tin = "X999".to_owned();
    assert!(matches!(
        provider.submit_invoice(&bad_tin).unwrap_err(),
        MyInvoisError::BadTin(_)
    ));

    // (b) bad issuer BRN (must be 12 digits).
    let mut bad_brn = submit_request(ubl_bytes);
    bad_brn.issuer_brn = "BAD".to_owned();
    assert!(matches!(
        provider.submit_invoice(&bad_brn).unwrap_err(),
        MyInvoisError::BadBrn(_)
    ));

    // (c) empty payload — the wire would never accept it.
    let empty = submit_request(Vec::new());
    assert!(matches!(
        provider.submit_invoice(&empty).unwrap_err(),
        MyInvoisError::BadXml(_)
    ));
}

// ---------------------------------------------------------------------------
// Deepened, country-specific scenarios.
//
// Grounded in the Lembaga Hasil Dalam Negeri Malaysia (LHDNM / Inland Revenue
// Board of Malaysia) MyInvois Software Development Kit, the authoritative spec
// the crate's `code()` / TIN / hash surface mirrors:
//   - e-Invoice type codes:  https://sdk.myinvois.hasil.gov.my/codes/e-invoice-types/
//   - Tax type codes:        https://sdk.myinvois.hasil.gov.my/codes/tax-types/
//   - Credit Note v1.0:      https://sdk.myinvois.hasil.gov.my/documents/credit-v1-0/
//   - Self-Billed Invoice:   https://sdk.myinvois.hasil.gov.my/documents/self-billed-invoice-v1-0/
//   - SDK FAQ (TIN shape, SHA-256 documentHash):
//                            https://sdk.myinvois.hasil.gov.my/faq/
// All fixtures below are hand-built synthetic payloads. No copyrighted LHDNM
// sample file is vendored.
// ---------------------------------------------------------------------------

/// A Malaysian credit note in IR, correcting an earlier invoice.
///
/// LHDNM models a credit note (e-Invoice type code `02`) as a document the
/// supplier issues to reduce the value of a previously cleared e-Invoice
/// (returns / discounts / error correction) where no money is returned to the
/// buyer. See the SDK "Credit Note v1.0" document type. In UBL 2.1 a credit
/// note serializes under a `<CreditNote>` root carrying `cbc:CreditNoteTypeCode`
/// `381`, and — unlike an invoice — it MUST NOT carry a top-level
/// `cbc:DueDate`, so we leave `due_date: None`.
fn malaysian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-my-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote forbids a top-level cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-MY-0001").unwrap(),
        currency: Iso4217Code::new("MYR").unwrap(),
        supplier: malaysian_party("Acme Sdn Bhd", "MY1234567890", "Kuala Lumpur", "14"),
        customer: malaysian_party("Beta Sdn Bhd", "MY9876543210", "George Town", "07"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit: returned platform seats".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Malaysia's standard service-tax rate is 6%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(300),
            tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5300),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5300),
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
fn malaysia_credit_note_clears_under_type_code_02() {
    // The MyInvois e-Invoice type code for a credit note is `02`
    // (https://sdk.myinvois.hasil.gov.my/codes/e-invoice-types/). The IR/UBL
    // layer additionally tags it with the UBL CreditNote type code `381`.
    assert_eq!(MyInvoisDocumentKind::CreditNote.code(), "02");

    let doc = malaysian_credit_note();
    let xml = to_xml(&doc).unwrap();

    // UBL CreditNote spine: a `<CreditNote>` root carrying CreditNoteTypeCode
    // 381 and a CreditNoteLine, NOT an Invoice/InvoiceLine. (Canonicalization
    // pins the `cbc:`/`cac:` prefixes but repeats the namespace declaration on
    // each element, so we match element name + value, not a bare open tag.)
    assert!(
        xml.contains("<CreditNote xmlns="),
        "credit note must serialize under the UBL CreditNote root"
    );
    assert!(
        xml.contains("cbc:CreditNoteTypeCode") && xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL credit note must carry CreditNoteTypeCode 381, got {xml}"
    );
    assert!(
        xml.contains("cac:CreditNoteLine"),
        "credit note lines map to cac:CreditNoteLine, got {xml}"
    );
    assert!(
        !xml.contains("cac:InvoiceLine"),
        "a credit note must not emit cac:InvoiceLine"
    );
    // A UBL credit note never carries a top-level due date.
    assert!(
        !xml.contains("cbc:DueDate"),
        "UBL CreditNote must not emit a top-level cbc:DueDate"
    );

    // Submit the credit note under the `02` document class and assert LHDNM
    // stamps it just like any other cleared document.
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let mut req = submit_request(xml.into_bytes());
    req.kind = MyInvoisDocumentKind::CreditNote;
    let envelope = provider.submit_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);
    assert_eq!(req.kind.code(), "02");
    assert!(envelope.uuid.starts_with("MOCK-UUID-"));
    assert_eq!(envelope.content_hash_hex.len(), 64);
}

/// A multi-line Malaysian service invoice carrying 6% service tax.
///
/// Malaysia levies Service Tax at the standard 6% rate (LHDNM tax type code
/// `02`, <https://sdk.myinvois.hasil.gov.my/codes/tax-types/>). Three taxed
/// lines sum to RM 300.00 net + RM 18.00 tax = RM 318.00 payable.
fn malaysian_multiline_invoice() -> CommercialDocument {
    let line = |id: &str, desc: &str, qty: i64, unit_minor: i64| DocumentLine {
        id: id.to_owned(),
        description: desc.to_owned(),
        quantity: DecimalValue::new(Decimal::from(qty)),
        unit_code: Some("EA".to_owned()),
        unit_price: amt(unit_minor),
        line_extension_amount: amt(unit_minor * qty),
        tax_category: Some("S".to_owned()),
        classifications: Vec::new(),
        extensions: Vec::new(),
    };
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-my-multiline-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-MY-0002").unwrap(),
        currency: Iso4217Code::new("MYR").unwrap(),
        supplier: malaysian_party("Acme Sdn Bhd", "MY1234567890", "Kuala Lumpur", "14"),
        customer: malaysian_party("Beta Sdn Bhd", "MY9876543210", "George Town", "07"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            line("1", "Consulting day rate", 1, 10000),
            line("2", "Cloud platform seats", 2, 5000),
            line("3", "Premium support", 1, 10000),
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(30000),
            tax_amount: amt(1800),
            tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            tax_inclusive_amount: amt(31800),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(31800),
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
fn malaysia_multiline_service_tax_invoice_clears_and_bundles() {
    let doc = malaysian_multiline_invoice();
    let xml = to_xml(&doc).unwrap();
    let ubl_bytes = xml.into_bytes();

    // The UBL artefact must carry all three priced lines.
    let xml_text = String::from_utf8(ubl_bytes.clone()).unwrap();
    assert_eq!(
        xml_text.matches("cac:InvoiceLine").count(),
        // open + close tag per line.
        6,
        "three invoice lines must serialize"
    );
    assert!(
        xml_text.contains("Consulting day rate")
            && xml_text.contains("Cloud platform seats")
            && xml_text.contains("Premium support"),
        "all three line descriptions must survive serialization"
    );
    // RM 300.00 net + RM 18.00 service tax = RM 318.00 payable.
    assert!(
        xml_text.contains("318.00"),
        "payable total RM 318.00 must appear in the UBL artefact"
    );

    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(ubl_bytes.clone()))
        .unwrap();
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);

    // The multi-line document still produces a verifiable evidence bundle.
    let ikb = pack_evidence(&doc, ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

#[test]
fn malaysia_tax_exempt_invoice_carries_zero_tax_and_exempt_category() {
    // LHDNM tax type code `E` marks an exempted supply
    // (https://sdk.myinvois.hasil.gov.my/codes/tax-types/). A tax-exempt
    // invoice has a tax amount of RM 0.00 and the taxable amount equals the
    // payable amount. We tag the IR line/summary with EN-16931 tax category
    // `E` (Exempt) so the UBL artefact carries an exempt classification.
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-my-exempt-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-MY-0003").unwrap(),
        currency: Iso4217Code::new("MYR").unwrap(),
        supplier: malaysian_party("Acme Sdn Bhd", "MY1234567890", "Kuala Lumpur", "14"),
        customer: malaysian_party("Beta Sdn Bhd", "MY9876543210", "George Town", "07"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exempt financial service".to_owned(),
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
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            // Exempt: no tax added, so inclusive == exclusive.
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
    .unwrap();

    let xml = to_xml(&doc).unwrap();
    // The exempt classification must surface in the UBL tax category. After
    // canonicalization every element carries an `xmlns:cbc=...` declaration, so
    // we match the closing tag carrying the value rather than a bare open tag.
    assert!(
        xml.contains(">E</cbc:ID>"),
        "exempt tax category `E` must appear in the UBL TaxCategory, got {xml}"
    );
    // The tax total is zero for an exempt supply.
    assert!(
        xml.contains("currencyID=\"MYR\">0.00</cbc:TaxAmount>"),
        "exempt invoice must carry a RM 0.00 tax amount, got {xml}"
    );
    // Tax-exclusive and tax-inclusive amounts are equal for an exempt supply.
    assert!(
        xml.contains("currencyID=\"MYR\">200.00</cbc:TaxExclusiveAmount>")
            && xml.contains("currencyID=\"MYR\">200.00</cbc:TaxInclusiveAmount>"),
        "exempt invoice: tax-exclusive must equal tax-inclusive (RM 200.00), got {xml}"
    );

    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(xml.into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);
}

#[test]
fn malaysia_self_billed_b2c_import_clears_under_type_code_11() {
    // A self-billed e-Invoice (e-Invoice type code `11`,
    // https://sdk.myinvois.hasil.gov.my/codes/e-invoice-types/ and
    // https://sdk.myinvois.hasil.gov.my/documents/self-billed-invoice-v1-0/) is
    // issued by the BUYER for an acquisition from a supplier who cannot issue an
    // e-Invoice (e.g. a foreign supplier / import). The counterparty supplier
    // often has no Malaysian TIN, so the request omits `buyer_tin` — the crate
    // accepts the B2C / self-billed shape with `buyer_tin: None`.
    assert_eq!(MyInvoisDocumentKind::SelfBilledInvoice.code(), "11");

    let doc = malaysian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);

    let request = MyInvoisSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: MyInvoisEnvironment::Production,
        kind: MyInvoisDocumentKind::SelfBilledInvoice,
        issuer_tin: ISSUER_TIN.to_owned(),
        issuer_brn: ISSUER_BRN.to_owned(),
        // Self-billed import: foreign supplier has no Malaysian TIN.
        buyer_tin: None,
        invoice_xml: ubl_bytes,
    };
    let envelope = provider.submit_invoice(&request).unwrap();
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);
    assert_eq!(request.kind.code(), "11");
    assert!(envelope.long_id.starts_with("MOCK-LONG-ID-"));
}

#[test]
fn malaysia_refund_note_uses_type_code_04() {
    // A refund note (e-Invoice type code `04`,
    // https://sdk.myinvois.hasil.gov.my/codes/e-invoice-types/) is issued by the
    // supplier to confirm a refund of monies to the buyer — distinct from a
    // credit note (`02`), which reduces value without returning money.
    assert_eq!(MyInvoisDocumentKind::RefundNote.code(), "04");
    assert_eq!(MyInvoisDocumentKind::SelfBilledRefundNote.code(), "14");

    // It still clears like any other document class.
    let doc = malaysian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
    let mut req = submit_request(ubl_bytes);
    req.kind = MyInvoisDocumentKind::RefundNote;
    let envelope = provider.submit_invoice(&req).unwrap();
    assert_eq!(envelope.status, MyInvoisStatus::Submitted);
}

#[test]
fn malaysia_rejects_tin_without_mandatory_c_prefix() {
    // The MyInvois SDK FAQ (https://sdk.myinvois.hasil.gov.my/faq/) documents
    // that a non-individual (business) TIN carries a letter prefix such as `C`.
    // The crate enforces the `C` + 10-digit shape: a 10-digit-only string (no
    // prefix) and a wrong-prefix string must both be refused PRE-WIRE as
    // `MyInvoisError::BadTin` — never silently accepted, and never an
    // authority-side `Rejected` envelope (the mock has no force-reject hook).
    let doc = malaysian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);

    // (a) digits only, no `C` prefix.
    let mut no_prefix = submit_request(ubl_bytes.clone());
    no_prefix.issuer_tin = "1234567890".to_owned();
    assert!(matches!(
        provider.submit_invoice(&no_prefix).unwrap_err(),
        MyInvoisError::BadTin(_)
    ));

    // (b) an individual-style prefix `J` is not the `C` non-individual shape
    // this crate validates for a B2B issuer.
    let mut wrong_prefix = submit_request(ubl_bytes.clone());
    wrong_prefix.issuer_tin = "J123456789".to_owned();
    assert!(matches!(
        provider.submit_invoice(&wrong_prefix).unwrap_err(),
        MyInvoisError::BadTin(_)
    ));

    // (c) a buyer TIN with a trailing non-digit is rejected too.
    let mut bad_buyer = submit_request(ubl_bytes);
    bad_buyer.buyer_tin = Some("C123456789X".to_owned());
    assert!(matches!(
        provider.submit_invoice(&bad_buyer).unwrap_err(),
        MyInvoisError::BadTin(_)
    ));
}

#[test]
fn malaysia_document_hash_is_sha256_shaped_and_payload_bound() {
    // The MyInvois SDK FAQ (https://sdk.myinvois.hasil.gov.my/faq/) states the
    // submitted document is hashed with SHA-256 to produce the `documentHash`
    // value. A SHA-256 digest is 32 bytes == 64 lowercase hex characters. The
    // mock stands in for the real digest but must keep that shape and the
    // determinism property: identical payload -> identical hash; different
    // payload -> different hash.
    let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);

    let invoice = to_xml(&malaysian_invoice()).unwrap().into_bytes();
    let credit = to_xml(&malaysian_credit_note()).unwrap().into_bytes();

    let a = provider
        .submit_invoice(&submit_request(invoice.clone()))
        .unwrap();
    let b = provider
        .submit_invoice(&submit_request(invoice))
        .unwrap();
    let c = provider.submit_invoice(&submit_request(credit)).unwrap();

    // SHA-256 shape: exactly 64 lowercase hex chars.
    for env in [&a, &b, &c] {
        assert_eq!(
            env.content_hash_hex.len(),
            64,
            "documentHash must be 64 hex chars (SHA-256)"
        );
        assert!(
            env.content_hash_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "documentHash must be lowercase hex"
        );
    }
    // Same payload -> same content hash (the hash is bound to the bytes, not
    // the per-submission serial).
    assert_eq!(
        a.content_hash_hex, b.content_hash_hex,
        "identical payloads must hash identically"
    );
    // Different payload -> different content hash.
    assert_ne!(
        a.content_hash_hex, c.content_hash_hex,
        "an invoice and a credit note must not share a documentHash"
    );
}

#[test]
fn malaysia_credit_note_lifecycle_is_byte_deterministic() {
    // Determinism for the credit-note class too: serialize + submit + pack
    // twice and assert byte-identical `.ikb` output, mirroring the invoice
    // determinism guarantee.
    let pack_once = || {
        let doc = malaysian_credit_note();
        let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
        let provider = MockMyInvoisProvider::with_fixed_submitted_at(PINNED_CREATED_AT);
        let mut req = submit_request(ubl_bytes.clone());
        req.kind = MyInvoisDocumentKind::CreditNote;
        let envelope = provider.submit_invoice(&req).unwrap();
        pack_evidence(&doc, ubl_bytes, &envelope)
    };
    assert_eq!(
        pack_once(),
        pack_once(),
        "the credit-note offline lifecycle must be byte-stable"
    );
}
