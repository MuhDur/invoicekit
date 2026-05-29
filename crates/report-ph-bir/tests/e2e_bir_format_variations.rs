// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Philippines **BIR EIS** country-specific format-variation coverage.
//!
//! This file *adds* to the baseline `e2e_offline_lifecycle.rs` (it does not
//! replace it). Where the baseline proves the happy-path build -> UBL ->
//! submit -> evidence chain once, this file exercises the document and tax
//! variations the Bureau of Internal Revenue's Electronic Invoicing /
//! Receipting & Sales reporting System (EIS) actually distinguishes.
//!
//! Regulatory grounding (cited per scenario in the test doc-comments):
//!
//! - Bureau of Internal Revenue (BIR), *Revenue Regulations No. 8-2022*
//!   (issued 30 June 2022), implementing Sections 237 and 237-A of the
//!   National Internal Revenue Code (NIRC) as amended by the TRAIN Law
//!   (Republic Act No. 10963). RR 8-2022 stands up the EIS and mandates
//!   issuance/transmission of electronic Sales Invoices, Official Receipts,
//!   Billing Invoices, and Credit/Debit Memos in a JSON data format.
//!   <https://www.bir.gov.ph/> (Revenue Issuances > Revenue Regulations 2022).
//! - BIR EIS production / sandbox portals: <https://eis.bir.gov.ph/> and
//!   <https://eis-cert.bir.gov.ph/> (certification environment).
//! - Standard Philippine VAT rate is 12% (NIRC Sec. 106 / 108, as amended by
//!   the TRAIN Law). Zero-rated sales (NIRC Sec. 106(A)(2) / 108(B)) carry a
//!   0% output VAT; VAT-exempt sales (NIRC Sec. 109) carry no VAT at all, and
//!   the invoice must be conspicuously marked "zero-rated sale" /
//!   "VAT-exempt sale" respectively (RR 16-2005 as amended; restated for
//!   e-invoices under RR 8-2022).
//!
//! Fixtures here are entirely hand-built / synthetic. No copyrighted BIR
//! schema or sample file is vendored. As in the baseline, no
//! `insta`/`pretty_assertions` (those would mutate `Cargo.lock`).
//!
//! Note on the rejection path: `MockEisProvider` exposes no forced-receipt
//! knob, so an `Ok(envelope)` carrying `EisStatus::Rejected` is unreachable
//! through `submit_invoice`. The BIR rejection *verdict* is nonetheless a
//! first-class persisted artefact, so the dedicated serde scenario below
//! constructs and round-trips exactly the envelope the live
//! `report-ph-bir-http` adapter will hand back, proving the audit-trail shape.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ph_bir::{
    validate_tin, EisDocumentKind, EisEnvironment, EisProvider, EisStatus, EisSubmitEnvelope,
    EisSubmitRequest, MockEisProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ACKNOWLEDGED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_ph_fmt";
const TRACE: &str = "trace_ph_fmt";
const ISSUER_TIN: &str = "123456789-001";
const ATP: &str = "ATP-2026-000042";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn ph_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["123 Ayala Avenue".to_owned()],
            city: city.to_owned(),
            subdivision: Some("NCR".to_owned()),
            postal_code: "1226".to_owned(),
            country: CountryCode::new("PH").unwrap(),
        },
        contact: None,
    }
}

fn meta() -> DocumentMeta {
    DocumentMeta {
        tenant_id: TENANT.to_owned(),
        trace_id: TRACE.to_owned(),
        source_system: Some("e2e-fmt".to_owned()),
    }
}

fn submit_request(kind: EisDocumentKind, invoice_json: Vec<u8>) -> EisSubmitRequest {
    EisSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EisEnvironment::Production,
        kind,
        issuer_tin: ISSUER_TIN.to_owned(),
        atp: ATP.to_owned(),
        invoice_json,
    }
}

/// Pack a single document + its UBL bytes + the BIR receipt into an `.ikb`
/// bundle and return the bytes, so each scenario can prove verifiability.
fn bundle_for(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &EisSubmitEnvelope) -> Vec<u8> {
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
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    pack(&bundle).unwrap()
}

// --------------------------------------------------------------------------
// Scenario 1 — VAT zero-rated sale (NIRC Sec. 106(A)(2); RR 8-2022).
// --------------------------------------------------------------------------

/// A VAT **zero-rated** sale (e.g. a sale to a registered export enterprise).
/// Output VAT is 0% but the supply must still be e-invoiced and transmitted to
/// EIS; the tax category and 0% rate must survive to the wire payload. Uses
/// the Philippine zero-rated tax-category code `Z`.
fn zero_rated_sale() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ph-zero-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("SI-2026-PH-ZR-0001").unwrap(),
        currency: Iso4217Code::new("PHP").unwrap(),
        supplier: ph_party("Subic Export Mfg Inc", "PH223456789", "Olongapo"),
        customer: ph_party("PEZA Registered Buyer", "PH998877665", "Lapu-Lapu"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Export-oriented assembly services (zero-rated sale)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(10)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(25_000),
            line_extension_amount: amt(250_000),
            tax_category: Some("Z".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(250_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(250_000),
            tax_exclusive_amount: amt(250_000),
            // Zero-rated: tax-inclusive == tax-exclusive, payable carries no VAT.
            tax_inclusive_amount: amt(250_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(250_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: meta(),
    })
    .unwrap()
}

#[test]
fn zero_rated_sale_carries_zero_percent_to_wire_and_acknowledges() {
    let doc = zero_rated_sale();
    let ubl = to_xml(&doc).unwrap();

    // The canonicalizer declares namespaces inline on each element, so we match
    // on the namespace-tolerant suffix shape (as the baseline lifecycle does).
    // The zero-rated tax category and explicit 0% rate must reach the EIS
    // payload, and no positive VAT may appear in the tax/monetary totals.
    assert!(
        ubl.contains(">Z</cbc:ID>"),
        "zero-rated category code Z must be serialized, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">0</cbc:Percent>"),
        "zero-rated supply must carry an explicit 0% output VAT rate"
    );
    // amt(250_000) == 2500.00; zero-rated keeps inclusive == exclusive == base.
    assert!(
        ubl.contains(">2500.00</cbc:TaxInclusiveAmount>"),
        "zero-rated invoice must keep tax-inclusive == tax-exclusive (no VAT added)"
    );
    assert!(
        ubl.contains(">2500.00</cbc:PayableAmount>"),
        "zero-rated payable must equal the taxable base"
    );
    assert!(
        ubl.contains("currencyID=\"PHP\">0.00</cbc:TaxAmount>"),
        "zero-rated TaxTotal must report 0.00 PHP output VAT"
    );

    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(
            EisDocumentKind::SalesInvoice,
            ubl.clone().into_bytes(),
        ))
        .unwrap();
    assert_eq!(envelope.status, EisStatus::Acknowledged);
    assert!(envelope.reference_number.starts_with("BIR-"));

    let ikb = bundle_for(&doc, ubl.as_bytes(), &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "zero-rated evidence bundle must verify"
    );
}

// --------------------------------------------------------------------------
// Scenario 2 — VAT-exempt sale (NIRC Sec. 109; RR 8-2022).
// --------------------------------------------------------------------------

/// A VAT-**exempt** sale (NIRC Sec. 109 enumerated transaction, e.g. sale of
/// prescription medicines / agricultural produce). No output VAT is computed;
/// the exempt tax-category code `E` and a zero VAT subtotal must persist.
fn vat_exempt_sale() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ph-exempt-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("SI-2026-PH-EX-0001").unwrap(),
        currency: Iso4217Code::new("PHP").unwrap(),
        supplier: ph_party("Botika ng Bayan Coop", "PH334455667", "Quezon City"),
        customer: ph_party("Senior Citizen Buyer", "PH112233445", "Pasig"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Prescription maintenance medicines (VAT-exempt sale)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(4)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(37_500),
            line_extension_amount: amt(150_000),
            tax_category: Some("E".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(150_000),
            tax_amount: amt(0),
            // VAT-exempt: no rate is asserted on the line at all.
            tax_rate: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(150_000),
            tax_exclusive_amount: amt(150_000),
            tax_inclusive_amount: amt(150_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(150_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: meta(),
    })
    .unwrap()
}

#[test]
fn vat_exempt_sale_emits_exempt_category_without_percent() {
    let doc = vat_exempt_sale();
    let ubl = to_xml(&doc).unwrap();

    // Exempt category `E` is serialized. Because no rate was supplied, the
    // serializer must emit NO cbc:Percent element at all — the distinguishing
    // trait of a VAT-exempt (vs a 0%-rate zero-rated) supply. Contrast with
    // `zero_rated_sale_carries_zero_percent_to_wire_*`, which DOES emit one.
    assert!(
        ubl.contains(">E</cbc:ID>"),
        "VAT-exempt category code E must be serialized, got:\n{ubl}"
    );
    assert!(
        !ubl.contains("<cbc:Percent"),
        "an exempt-only invoice must not carry any cbc:Percent output VAT rate"
    );
    // Whole-invoice TaxTotal still emits a zero TaxAmount header.
    assert!(
        ubl.contains("currencyID=\"PHP\">0.00</cbc:TaxAmount>"),
        "exempt invoice TaxTotal must report 0.00 PHP output VAT"
    );
    // amt(150_000) == 1500.00 base, no VAT, payable == base.
    assert!(
        ubl.contains(">1500.00</cbc:PayableAmount>"),
        "exempt payable must equal the taxable base (no VAT)"
    );

    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(
            EisDocumentKind::SalesInvoice,
            ubl.into_bytes(),
        ))
        .unwrap();
    assert_eq!(envelope.status, EisStatus::Acknowledged);
}

// --------------------------------------------------------------------------
// Scenario 3 — Credit Memo (BIR EisDocumentKind::CreditMemo; RR 8-2022).
// --------------------------------------------------------------------------

/// A BIR **Credit Memo** (CM) that reverses part of an earlier sales invoice.
/// EIS treats the CM as its own document class; the UBL family maps it to a
/// `<CreditNote>` root with `cbc:CreditNoteTypeCode` 381 (UBL 2.1 code list).
/// A UBL `CreditNote` carries no top-level `cbc:DueDate`, so `due_date` is None.
fn credit_memo() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ph-cm-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        document_number: DocumentNumber::new("CM-2026-PH-0001").unwrap(),
        currency: Iso4217Code::new("PHP").unwrap(),
        supplier: ph_party("Makati Trading Inc", "PH123456789", "Makati"),
        customer: ph_party("Cebu Logistics Corp", "PH987654321", "Cebu"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Reversal of over-billed consulting (credit memo)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(50_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Standard 12% Philippine VAT on the credited amount.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50_000),
            tax_amount: amt(6_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1200, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50_000),
            tax_exclusive_amount: amt(50_000),
            tax_inclusive_amount: amt(56_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(56_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: meta(),
    })
    .unwrap()
}

#[test]
fn credit_memo_serializes_as_ubl_credit_note_and_bundles() {
    let doc = credit_memo();
    let ubl = to_xml(&doc).unwrap();

    // CreditNote root + UBL 2.1 type code 381 + CreditNote-specific line shape.
    // The root carries the default-namespace declaration; child cbc/cac
    // elements carry inline namespaces, so child matches use the suffix shape.
    assert!(
        ubl.starts_with("<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\">"),
        "credit memo must serialize with a UBL CreditNote root, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "credit memo must carry UBL CreditNoteTypeCode 381"
    );
    assert!(
        ubl.contains("<cac:CreditNoteLine ") && ubl.contains("<cbc:CreditedQuantity"),
        "credit memo lines must use cac:CreditNoteLine / cbc:CreditedQuantity"
    );
    // No top-level DueDate is permitted on a UBL CreditNote.
    assert!(
        !ubl.contains("<cbc:DueDate"),
        "UBL CreditNote must not emit a top-level cbc:DueDate"
    );
    // 12% standard Philippine VAT survived to the wire (rate scale preserved).
    assert!(
        ubl.contains(">12.00</cbc:Percent>"),
        "credit memo must keep the 12% standard-rate category"
    );
    // amt(56_000) == 560.00 payable (500.00 base + 60.00 VAT).
    assert!(
        ubl.contains(">560.00</cbc:PayableAmount>"),
        "credit memo payable must be base + 12% VAT"
    );

    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(
            EisDocumentKind::CreditMemo,
            ubl.clone().into_bytes(),
        ))
        .unwrap();
    assert_eq!(envelope.status, EisStatus::Acknowledged);

    let ikb = bundle_for(&doc, ubl.as_bytes(), &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "credit-memo evidence bundle must verify"
    );
}

// --------------------------------------------------------------------------
// Scenario 4 — Official Receipt for services, multi-line, 12% VAT (RR 8-2022).
// --------------------------------------------------------------------------

/// A BIR **Official Receipt** (OR) covering a multi-line services engagement,
/// all at the 12% standard rate. Distinct from a Sales Invoice in the EIS
/// taxonomy (`EisDocumentKind::OfficialReceipt`); proves multi-line UBL
/// emission with multiple `cac:InvoiceLine` nodes and the aggregate `TaxTotal`.
fn official_receipt_multiline() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ph-or-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("OR-2026-PH-0007").unwrap(),
        currency: Iso4217Code::new("PHP").unwrap(),
        supplier: ph_party("Manila Pro Services Inc", "PH445566778", "Manila"),
        customer: ph_party("Davao Holdings Corp", "PH556677889", "Davao"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Professional fees - audit".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(80_000),
                line_extension_amount: amt(80_000),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Professional fees - tax advisory".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(35_000),
                line_extension_amount: amt(70_000),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            },
        ],
        // 150,000.00 base @ 12% = 18,000.00 output VAT.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(150_000),
            tax_amount: amt(18_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1200, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(150_000),
            tax_exclusive_amount: amt(150_000),
            tax_inclusive_amount: amt(168_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(168_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: meta(),
    })
    .unwrap()
}

#[test]
fn official_receipt_multiline_emits_each_line_and_acknowledges() {
    let doc = official_receipt_multiline();
    let ubl = to_xml(&doc).unwrap();

    // Two distinct invoice lines reach the wire (canonicalized elements carry an
    // inline namespace declaration, so the opening tag has a trailing space).
    let line_count = ubl.matches("<cac:InvoiceLine ").count();
    assert_eq!(
        line_count, 2,
        "multi-line official receipt must emit exactly two cac:InvoiceLine nodes, got:\n{ubl}"
    );
    assert!(
        ubl.contains("Professional fees - audit")
            && ubl.contains("Professional fees - tax advisory"),
        "both service descriptions must survive serialization"
    );
    // Aggregate 12% VAT header on the whole receipt: amt(18_000) == 180.00.
    assert!(
        ubl.contains("currencyID=\"PHP\">180.00</cbc:TaxAmount>"),
        "aggregate output VAT must be 180.00 PHP, got:\n{ubl}"
    );
    // amt(168_000) == 1680.00 payable (1500.00 base + 180.00 VAT).
    assert!(
        ubl.contains(">1680.00</cbc:PayableAmount>"),
        "payable must be base + 12% VAT"
    );

    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(
            EisDocumentKind::OfficialReceipt,
            ubl.clone().into_bytes(),
        ))
        .unwrap();
    assert_eq!(envelope.status, EisStatus::Acknowledged);

    let ikb = bundle_for(&doc, ubl.as_bytes(), &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "official-receipt evidence bundle must verify"
    );
}

// --------------------------------------------------------------------------
// Scenario 5 — Philippine TIN identifier matrix (BIR registration number).
// --------------------------------------------------------------------------

/// The BIR Taxpayer Identification Number is 9 base digits plus an optional
/// 3-digit branch code (head office is `000`, the first branch `001`, etc.).
/// This locks the country-specific identifier rules: head-office and branch
/// suffixes are accepted; common malformations are rejected. The pre-wire
/// `validate_tin` refusal is a typed `EisError::BadTin`, NOT a panic.
#[test]
fn philippine_tin_identifier_matrix() {
    // Valid: 9-digit base, no branch (TIN without explicit branch code).
    assert!(validate_tin("123456789").is_ok());
    // Valid: head-office branch suffix 000.
    assert!(validate_tin("123456789-000").is_ok());
    // Valid: a branch suffix.
    assert!(validate_tin("123456789-015").is_ok());

    // Invalid: 8-digit base (one digit short).
    assert!(validate_tin("12345678").is_err());
    // Invalid: 12 contiguous digits (branch must be hyphen-separated, not glued).
    assert!(validate_tin("123456789015").is_err());
    // Invalid: 2-digit branch code (must be exactly 3).
    assert!(validate_tin("123456789-15").is_err());
    // Invalid: alphabetic contamination in the base.
    assert!(validate_tin("12345678X").is_err());
    // Invalid: alphabetic contamination in the branch.
    assert!(validate_tin("123456789-0A1").is_err());
}

/// An out-of-shape issuer TIN is refused before the EIS wire as a typed
/// `EisError::BadTin`, and the document never reaches `Acknowledged`. This is
/// the invalid-identifier rejection path (a local refusal, not a BIR verdict).
#[test]
fn submission_with_malformed_issuer_tin_is_refused_pre_wire() {
    let doc = official_receipt_multiline();
    let ubl = to_xml(&doc).unwrap().into_bytes();
    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);

    let mut req = submit_request(EisDocumentKind::OfficialReceipt, ubl);
    req.issuer_tin = "123456789-15".to_owned(); // 2-digit branch == malformed.

    let err = provider.submit_invoice(&req).unwrap_err();
    assert!(
        matches!(err, invoicekit_report_ph_bir::EisError::BadTin(_)),
        "a malformed branch-code TIN must be refused as EisError::BadTin, got {err:?}"
    );
}

// --------------------------------------------------------------------------
// Scenario 6 — BIR rejection-verdict audit shape + determinism.
// --------------------------------------------------------------------------

/// The live EIS adapter persists a BIR *rejection verdict* (e.g. ATP not yet
/// accredited) inside the same `EisSubmitEnvelope` the audit trail stores.
/// `MockEisProvider` has no forced-receipt knob, so we construct the rejection
/// envelope exactly as the `report-ph-bir-http` adapter will and prove the
/// country-specific shape survives a JSON round-trip with its reason intact,
/// and that the reason field is omitted entirely on the acknowledged path.
#[test]
fn bir_rejection_verdict_round_trips_with_reason() {
    let rejected = EisSubmitEnvelope {
        reference_number: "BIR-000000000099".to_owned(),
        status: EisStatus::Rejected,
        acknowledged_at: PINNED_ACKNOWLEDGED_AT.to_owned(),
        reason: Some("ATP not registered for the submitting POS".to_owned()),
    };
    let json = serde_json::to_string(&rejected).unwrap();
    assert!(
        json.contains("\"status\":\"rejected\""),
        "EisStatus must serialize kebab-case as `rejected`, got {json}"
    );
    assert!(
        json.contains("ATP not registered"),
        "the BIR rejection reason must be persisted"
    );
    let parsed: EisSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, rejected);

    // The acknowledged path omits the reason field entirely (skip_serializing_if).
    let acknowledged = EisSubmitEnvelope {
        reference_number: "BIR-000000000100".to_owned(),
        status: EisStatus::Acknowledged,
        acknowledged_at: PINNED_ACKNOWLEDGED_AT.to_owned(),
        reason: None,
    };
    let ack_json = serde_json::to_string(&acknowledged).unwrap();
    assert!(
        !ack_json.contains("reason"),
        "acknowledged envelope must omit the reason key, got {ack_json}"
    );
    assert!(ack_json.contains("\"status\":\"acknowledged\""));
}

/// The whole zero-rated lifecycle (build -> UBL -> submit -> bundle) must be
/// byte-identical across two runs. Determinism is the trust-toolkit contract:
/// a regulator (or the public conformance corpus) must reproduce the bytes.
#[test]
fn zero_rated_lifecycle_is_byte_deterministic() {
    let run = || {
        let doc = zero_rated_sale();
        let ubl = to_xml(&doc).unwrap();
        let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
        let envelope = provider
            .submit_invoice(&submit_request(
                EisDocumentKind::SalesInvoice,
                ubl.clone().into_bytes(),
            ))
            .unwrap();
        bundle_for(&doc, ubl.as_bytes(), &envelope)
    };
    assert_eq!(run(), run(), "zero-rated offline lifecycle must be byte-stable");
}
