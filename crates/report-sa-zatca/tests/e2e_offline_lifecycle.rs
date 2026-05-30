// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Saudi Arabia ZATCA Phase 2 offline end-to-end lifecycle (coverage-loop §1
//! honest bar).
//!
//! Drives the full local-only chain for Saudi Arabia and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> ZATCA UBL 2.1 XML (UBL base + ZATCA extensions + ICV/PIH
//!    hash chain), reusing `invoicekit-format-ubl`
//! 3. local validate (structural + Saudi VAT / ICV / PIH identity shapes)
//! 4. stamp + clear/report via the offline `MockZatcaReportProvider` (composes
//!    `invoicekit-signer-zatca`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. byte-determinism: serialize twice and pack twice -> byte-identical
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). This mirrors the Italy SDI reference E2E. Capability-matrix
//! presence is asserted centrally, not here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_sa_zatca::{
    build_qr_fields, to_zatca_ubl_xml, validate_invoice_counter_value,
    validate_previous_invoice_hash, validate_saudi_vat_number, CsidRecord, InvoiceMode,
    MockZatcaReportProvider, ReportingStatus, ZatcaClearanceKind, ZatcaEnvironment, ZatcaQrField,
    ZatcaReport, ZatcaReportError, ZatcaReportProvider, ZatcaReportRequest, ZatcaUblContext,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;
use std::sync::Arc;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_sa_e2e";
const TRACE: &str = "trace_sa_e2e";
const CSID: &str = "csid-compliance-e2e";
// Valid Saudi VAT: 15 digits, starts and ends with 3, position 11 is 1.
const SELLER_VAT: &str = "300000000010003";
const INVOICE_UUID: &str = "uuid-sa-e2e-0001";
const QR_TIMESTAMP: &str = "2026-05-26T10:30:00Z";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn saudi_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["King Fahd Road 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("01".to_owned()),
            postal_code: "12345".to_owned(),
            country: CountryCode::new("SA").unwrap(),
        },
        contact: None,
    }
}

fn saudi_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-sa-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-SA-0001").unwrap(),
        currency: Iso4217Code::new("SAR").unwrap(),
        supplier: saudi_party("Acme KSA", SELLER_VAT, "Riyadh"),
        customer: saudi_party("Beta Trading", "311111111110003", "Jeddah"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulting & support services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(15000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(115_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(115_000),
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

fn csid() -> CsidRecord {
    CsidRecord {
        csid: CSID.to_owned(),
        environment: ZatcaEnvironment::Compliance,
        vat_number: SELLER_VAT.to_owned(),
        stamp_uuid: Some("stamp-uuid-e2e".to_owned()),
        not_before: "2026-01-01T00:00:00Z".to_owned(),
        not_after: "2027-12-31T23:59:59Z".to_owned(),
    }
}

fn provider(forced: Option<ReportingStatus>) -> MockZatcaReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CSID, [9_u8; 32]));
    let p = MockZatcaReportProvider::new(signer, csid());
    match forced {
        Some(status) => p.with_forced_status(status),
        None => p,
    }
}

fn report_request(ubl_xml: Vec<u8>, mode: InvoiceMode) -> ZatcaReportRequest {
    // The genesis lifecycle request is the general builder at chain position 1
    // (ICV 1, genesis PIH) over the primary `saudi_invoice()` fixture.
    report_request_for(
        &saudi_invoice(),
        INVOICE_UUID,
        ubl_xml,
        mode,
        SELLER_VAT,
        1,
        ZatcaUblContext::GENESIS_PIH,
    )
}

/// Step 5 of the lifecycle: assemble the canonical doc + ZATCA UBL + signed QR
/// TLV + receipt into a `.ikb` evidence bundle and pack it. The "signed
/// artifact" is the QR-code TLV envelope the stamp produced (ZATCA's printable
/// cryptographic proof).
fn pack_zatca_bundle(doc: &CommercialDocument, ubl: String, report: &ZatcaReport) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/zatca-ubl.xml".to_owned(), ubl.into_bytes());
    artefacts.insert("signed/qr.tlv".to_owned(), report.qr_tlv.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    pack(&bundle).unwrap()
}

/// Steps 1-5: build -> serialize -> validate -> stamp/clear -> evidence bundle.
fn run_lifecycle(mode: InvoiceMode, forced: Option<ReportingStatus>) -> (Vec<u8>, ZatcaReport) {
    // 1. build
    let doc = saudi_invoice();

    // 2. serialize -> ZATCA UBL (reusing format-ubl for the UBL 2.1 spine)
    let ctx = ZatcaUblContext::genesis(INVOICE_UUID, mode);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();

    // 3. local validate: structural ZATCA spine + Saudi identity shapes.
    for needle in [
        "<ext:UBLExtensions",
        "<cbc:ProfileID>reporting:1.0</cbc:ProfileID>",
        "<cbc:UUID>uuid-sa-e2e-0001</cbc:UUID>",
        "<cbc:ID>ICV</cbc:ID>",
        "<cbc:ID>PIH</cbc:ID>",
        "<cbc:CompanyID>300000000010003</cbc:CompanyID>",
    ] {
        assert!(ubl.contains(needle), "ZATCA UBL missing {needle}");
    }
    validate_saudi_vat_number(SELLER_VAT).unwrap();
    validate_invoice_counter_value(1).unwrap();
    validate_previous_invoice_hash(ZatcaUblContext::GENESIS_PIH).unwrap();

    // 4. stamp + clear/report (offline mock composing the real ZATCA signer)
    let report = provider(forced)
        .report(&report_request(ubl.clone().into_bytes(), mode))
        .unwrap();

    // 5. evidence bundle: canonical doc + ZATCA UBL + signed/stamped artifact +
    //    receipt.
    let ikb = pack_zatca_bundle(&doc, ubl, &report);
    (ikb, report)
}

#[test]
fn saudi_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(InvoiceMode::Standard, None);

    // Happy path: ZATCA cleared the standard (B2B) invoice.
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
    assert!(report.envelope.clearance_kind.is_accepted());
    assert_eq!(report.envelope.invoice_counter_value, 1);
    assert!(report.envelope.reason.is_none());
    assert!(!report.qr_tlv.is_empty());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn saudi_simplified_invoice_is_reported_b2c() {
    let (ikb, report) = run_lifecycle(InvoiceMode::Simplified, None);
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Reported);
    assert_eq!(report.envelope.mode, InvoiceMode::Simplified);

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "B2C reporting-path evidence bundle must verify");
}

#[test]
fn saudi_rejection_still_bundles_and_verifies() {
    // A portal rejection is a receipt kind, NOT an Err — the audit trail
    // persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(InvoiceMode::Standard, Some(ReportingStatus::Rejected));
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Rejected);
    assert!(!report.envelope.clearance_kind.is_accepted());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn saudi_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(InvoiceMode::Standard, None);
    let (b, _) = run_lifecycle(InvoiceMode::Standard, None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

// ===========================================================================
// Deepened, country-specific scenarios.
//
// Every scenario below is grounded in the ZATCA (Zakat, Tax and Customs
// Authority) Electronic Invoice (e-Invoice) XML Implementation Standard and the
// FATOORA Detailed Technical Guideline (the two normative ZATCA Phase 2
// references):
//
//   * ZATCA Electronic Invoice XML Implementation Standard, v1.2 (BR-KSA rules,
//     InvoiceTypeCode @name transaction-subtype code, VAT category codes):
//     https://zatca.gov.sa/en/E-Invoicing/SystemsDevelopers/Pages/E-Invoice-specifications.aspx
//   * ZATCA E-invoicing Detailed Technical Guideline (FATOORA), Nov 2022
//     (clearance vs reporting flow, PIH hash chain, ICV, QR-code TLV §V):
//     https://zatca.gov.sa/en/E-Invoicing/Introduction/Guidelines/Documents/E-invoicing-Detailed-Technical-Guideline.pdf
//
// Fixtures are hand-built and synthetic (no vendored regulator files).
// ===========================================================================

/// A second, distinct supplier VAT used by chained-document scenarios so the
/// assertions don't depend on the primary `SELLER_VAT` constant alone.
/// 15 digits, starts and ends with `3`, position 11 is `1` (the entity-type
/// marker ZATCA mandates). Per the Electronic Invoice XML Implementation
/// Standard, rule BR-KSA-39.
const SECOND_SELLER_VAT: &str = "300000000910003";

/// Build a credit note (corrective document) referencing a prior invoice.
///
/// ZATCA maps a credit note to UN/CEFACT 1001 `InvoiceTypeCode` `381` (see the
/// Electronic Invoice XML Implementation Standard, §"Invoice type code"). A UBL
/// 2.1 `CreditNote` must NOT carry a top-level due date, so `due_date` is left
/// `None`.
fn saudi_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-sa-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote cannot carry cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-SA-0001").unwrap(),
        currency: Iso4217Code::new("SAR").unwrap(),
        supplier: saudi_party("Acme KSA", SELLER_VAT, "Riyadh"),
        customer: saudi_party("Beta Trading", "311111111110003", "Jeddah"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Correction: returned consulting hours".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(50000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50000),
            tax_amount: amt(7500),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(57500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(57500),
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

/// Build a zero-rated export invoice (multi-line).
///
/// Per the Electronic Invoice XML Implementation Standard, an export of goods is
/// VAT category `Z` (zero-rated) at a 0% rate, carrying tax-exemption-reason
/// code `VATEX-SA-32`. Zero-rated supplies appear on the invoice with a tax
/// amount of zero, so QR Tag 5 (VAT total) is `0.00` and the invoice total
/// equals the taxable amount.
fn saudi_zero_rated_export_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-sa-e2e-zr-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-SA-EXP-0001").unwrap(),
        currency: Iso4217Code::new("SAR").unwrap(),
        supplier: saudi_party("Acme KSA", SELLER_VAT, "Riyadh"),
        // Foreign buyer (export): no Saudi VAT.
        customer: Party {
            id: Some("globex-llc".to_owned()),
            name: "Globex LLC".to_owned(),
            tax_ids: Vec::new(),
            address: PostalAddress {
                lines: vec!["1 Market Street".to_owned()],
                city: "Dubai".to_owned(),
                subdivision: None,
                postal_code: "00000".to_owned(),
                country: CountryCode::new("AE").unwrap(),
            },
            contact: None,
        },
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Exported dates (40kg)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(40)),
                unit_code: Some("KGM".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(100_000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Exported saffron (1kg)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("KGM".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(120_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(120_000),
            tax_exclusive_amount: amt(120_000),
            // Zero-rated: tax-inclusive == tax-exclusive.
            tax_inclusive_amount: amt(120_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(120_000),
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

/// Build a VAT-exempt invoice (financial services).
///
/// Per the Electronic Invoice XML Implementation Standard, exempt supplies use
/// VAT category `E` (tax-exemption-reason code `VATEX-SA-29`, financial
/// services, VAT Regulations Article 29). Like zero-rated, exempt supplies carry
/// a zero tax amount, but they are categorised distinctly from `Z`.
fn saudi_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-sa-e2e-ex-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-SA-EXM-0001").unwrap(),
        currency: Iso4217Code::new("SAR").unwrap(),
        supplier: saudi_party("Riyadh Finance Co", SELLER_VAT, "Riyadh"),
        customer: saudi_party("Beta Trading", "311111111110003", "Jeddah"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Margin-based financing fee".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(80000),
            line_extension_amount: amt(80000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
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
    .unwrap()
}

/// A report request driving an arbitrary document at an explicit ICV / PIH chain
/// position, so chained-document scenarios can advance the hash chain by hand.
fn report_request_for(
    doc: &CommercialDocument,
    uuid: &str,
    ubl_xml: Vec<u8>,
    mode: InvoiceMode,
    seller_vat: &str,
    icv: u64,
    pih: &str,
) -> ZatcaReportRequest {
    ZatcaReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: ZatcaEnvironment::Compliance,
        invoice_uuid: uuid.to_owned(),
        seller_vat_number: seller_vat.to_owned(),
        mode,
        invoice_counter_value: icv,
        previous_invoice_hash: pih.to_owned(),
        qr_fields: build_qr_fields(doc, QR_TIMESTAMP).unwrap(),
        ubl_xml,
    }
}

#[test]
fn saudi_credit_note_serializes_as_invoice_type_381() {
    // ZATCA InvoiceTypeCode 381 == credit note (Electronic Invoice XML
    // Implementation Standard). Standard (B2B) mode keeps the @name subtype
    // 0100000.
    let ctx = ZatcaUblContext::genesis("uuid-sa-e2e-cn-0001", InvoiceMode::Standard);
    let xml = to_zatca_ubl_xml(&saudi_credit_note(), &ctx).unwrap();
    assert!(
        xml.contains("<cbc:InvoiceTypeCode name=\"0100000\">381</cbc:InvoiceTypeCode>"),
        "credit note must serialize as ZATCA InvoiceTypeCode 381:\n{xml}"
    );
    // The UBL spine for a CreditNote uses the CreditNote root, not Invoice.
    assert!(
        xml.contains("CreditNote"),
        "credit note UBL spine must be a CreditNote document:\n{xml}"
    );
}

#[test]
fn saudi_credit_note_clears_and_bundles() {
    // A corrective document runs the same clearance lifecycle as an invoice.
    let doc = saudi_credit_note();
    let ctx = ZatcaUblContext::genesis("uuid-sa-e2e-cn-0001", InvoiceMode::Standard);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    let report = provider(None)
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            ubl.clone().into_bytes(),
            InvoiceMode::Standard,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
    assert_eq!(
        report.envelope.invoice_uuid, "uuid-sa-e2e-cn-0001",
        "the receipt must echo the real cbc:UUID, not a counter-synthesized one"
    );

    // The credit note's QR Tag 4 (invoice total, tax-inclusive) is 575.00 and
    // Tag 5 (VAT total) is 75.00 — half of the original invoice.
    let qr = build_qr_fields(&doc, QR_TIMESTAMP).unwrap();
    assert_eq!(qr.get(&ZatcaQrField::Total).unwrap(), "575.00");
    assert_eq!(qr.get(&ZatcaQrField::VatTotal).unwrap(), "75.00");

    // Bundle + verify the corrective-document evidence.
    let ikb = pack_zatca_bundle(&doc, ubl, &report);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "credit-note evidence bundle must verify");
}

#[test]
fn saudi_zero_rated_export_has_zero_vat_in_qr() {
    // Export of goods: VAT category Z (VATEX-SA-32), 0% rate. The five-field QR
    // (ZATCA Phase 2 §V) must carry a VAT total of 0.00 and a tax-inclusive
    // total equal to the taxable amount.
    let doc = saudi_zero_rated_export_invoice();
    let qr = build_qr_fields(&doc, QR_TIMESTAMP).unwrap();
    assert_eq!(
        qr.get(&ZatcaQrField::VatTotal).unwrap(),
        "0.00",
        "a zero-rated export must report 0.00 VAT in QR Tag 5"
    );
    assert_eq!(
        qr.get(&ZatcaQrField::Total).unwrap(),
        "1200.00",
        "zero-rated total == taxable amount (no VAT added)"
    );
    assert_eq!(qr.get(&ZatcaQrField::SellerName).unwrap(), "Acme KSA");

    // Multi-line: the UBL spine carries both export lines.
    let ctx = ZatcaUblContext::genesis("uuid-sa-e2e-zr-0001", InvoiceMode::Standard);
    let xml = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    assert!(
        xml.contains("Exported dates (40kg)"),
        "line 1 missing:\n{xml}"
    );
    assert!(
        xml.contains("Exported saffron (1kg)"),
        "line 2 missing:\n{xml}"
    );

    // It still clears through the standard flow.
    let report = provider(None)
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            xml.into_bytes(),
            InvoiceMode::Standard,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
}

#[test]
fn saudi_exempt_supply_has_zero_vat_and_clears() {
    // Exempt financial services: VAT category E (VATEX-SA-29). Zero VAT, total
    // equals the net.
    let doc = saudi_exempt_invoice();
    let qr = build_qr_fields(&doc, QR_TIMESTAMP).unwrap();
    assert_eq!(qr.get(&ZatcaQrField::VatTotal).unwrap(), "0.00");
    assert_eq!(qr.get(&ZatcaQrField::Total).unwrap(), "800.00");
    assert_eq!(
        qr.get(&ZatcaQrField::SellerName).unwrap(),
        "Riyadh Finance Co"
    );

    let ctx = ZatcaUblContext::genesis("uuid-sa-e2e-ex-0001", InvoiceMode::Standard);
    let xml = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    let report = provider(None)
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            xml.into_bytes(),
            InvoiceMode::Standard,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
}

#[test]
fn saudi_pih_chain_links_consecutive_invoices() {
    // ZATCA chains invoices: each invoice's hash becomes the Previous Invoice
    // Hash (PIH) of the next, and the Invoice Counter Value (ICV) increments by
    // one (FATOORA Detailed Technical Guideline — invoice hash chain). The first
    // invoice in a device chain uses the genesis PIH sentinel.
    let doc1 = saudi_invoice();
    let ctx1 = ZatcaUblContext::genesis("uuid-sa-chain-0001", InvoiceMode::Standard);
    let ubl1 = to_zatca_ubl_xml(&doc1, &ctx1).unwrap();
    let report1 = provider(None)
        .report(&report_request_for(
            &doc1,
            &ctx1.uuid,
            ubl1.into_bytes(),
            InvoiceMode::Standard,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(report1.envelope.invoice_counter_value, 1);
    assert_eq!(
        report1.envelope.invoice_uuid, "uuid-sa-chain-0001",
        "chain link 1 must echo its own real cbc:UUID"
    );
    let first_hash = report1.envelope.invoice_hash_hex;
    assert!(!first_hash.is_empty(), "invoice 1 must produce a hash");

    // Invoice 2 chains off invoice 1's hash at ICV 2. The PIH the second invoice
    // carries IS the first invoice's hash — that is the chain link.
    let doc2 = saudi_credit_note();
    let ctx2 = ZatcaUblContext {
        uuid: "uuid-sa-chain-0002".to_owned(),
        invoice_counter_value: 2,
        previous_invoice_hash: first_hash.clone(),
        mode: InvoiceMode::Standard,
    };
    let ubl2 = to_zatca_ubl_xml(&doc2, &ctx2).unwrap();
    // The serialized UBL embeds the previous invoice's hash as the PIH.
    assert!(
        ubl2.contains(&first_hash),
        "invoice 2 must embed invoice 1's hash as its PIH"
    );
    assert!(
        ubl2.contains("<cbc:UUID>2</cbc:UUID>"),
        "invoice 2 must carry ICV 2 in the AdditionalDocumentReference"
    );
}

#[test]
fn saudi_simplified_rejection_is_a_receipt_not_an_error() {
    // ZATCA's reporting (B2C / simplified) flow can refuse a report. A refusal
    // is a Rejected reporting status surfaced inside Ok(_), never an Err — the
    // audit-trail contract. Verify it on the simplified path specifically.
    let doc = saudi_invoice();
    let ctx = ZatcaUblContext::genesis(INVOICE_UUID, InvoiceMode::Simplified);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    let report = provider(Some(ReportingStatus::Rejected))
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            ubl.into_bytes(),
            InvoiceMode::Simplified,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Rejected);
    assert_eq!(report.envelope.mode, InvoiceMode::Simplified);
    assert!(!report.envelope.clearance_kind.is_accepted());
    assert!(
        report.envelope.reason.is_some(),
        "a rejection must carry a reason"
    );
}

#[test]
fn saudi_accepted_with_warnings_is_accepted() {
    // ZATCA can clear an invoice while attaching warnings (the stamp is valid;
    // the operator must fix the next invoice). This is an accepted verdict.
    let doc = saudi_invoice();
    let ctx = ZatcaUblContext::genesis(INVOICE_UUID, InvoiceMode::Standard);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    let report = provider(Some(ReportingStatus::AcceptedWithWarnings))
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            ubl.into_bytes(),
            InvoiceMode::Standard,
            SELLER_VAT,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap();
    assert_eq!(
        report.envelope.clearance_kind,
        ZatcaClearanceKind::AcceptedWithWarnings
    );
    assert!(report.envelope.clearance_kind.is_accepted());
    assert!(
        report.envelope.reason.is_none(),
        "accepted-with-warnings is not a rejection, so it carries no reason"
    );
}

#[test]
fn saudi_invalid_vat_identifier_is_a_pre_wire_error() {
    // An ill-shaped seller VAT is a pre-wire shape failure (Err), distinct from
    // a portal rejection (Ok). ZATCA BR-KSA: the VAT number is 15 digits,
    // starts and ends with 3, with a 1 at position 11. A number that ends in 4
    // violates that and must never reach the wire.
    let doc = saudi_invoice();
    let ctx = ZatcaUblContext::genesis(INVOICE_UUID, InvoiceMode::Standard);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();
    let bad_vat = "300000000010004"; // ends with 4, not 3
    let err = provider(None)
        .report(&report_request_for(
            &doc,
            &ctx.uuid,
            ubl.into_bytes(),
            InvoiceMode::Standard,
            bad_vat,
            1,
            ZatcaUblContext::GENESIS_PIH,
        ))
        .unwrap_err();
    assert!(
        matches!(err, ZatcaReportError::BadVatNumber(_)),
        "an ill-shaped Saudi VAT must be a BadVatNumber pre-wire error, got {err:?}"
    );
    // The standalone validator agrees, and a well-shaped second VAT passes.
    assert!(validate_saudi_vat_number(bad_vat).is_err());
    assert!(validate_saudi_vat_number(SECOND_SELLER_VAT).is_ok());
}

#[test]
fn saudi_serialization_distinguishes_invoice_from_credit_note() {
    // Determinism plus a country-specific discriminator: an Invoice and a
    // CreditNote built from the same supplier serialize to byte-distinct ZATCA
    // UBL carrying type codes 388 vs 381 respectively, and each is internally
    // stable across repeated serialization.
    let inv = saudi_invoice();
    let cn = saudi_credit_note();
    let inv_ctx = ZatcaUblContext::genesis("uuid-sa-disc-inv", InvoiceMode::Standard);
    let cn_ctx = ZatcaUblContext::genesis("uuid-sa-disc-cn", InvoiceMode::Standard);

    let inv_xml = to_zatca_ubl_xml(&inv, &inv_ctx).unwrap();
    let cn_xml = to_zatca_ubl_xml(&cn, &cn_ctx).unwrap();

    assert!(inv_xml.contains("<cbc:InvoiceTypeCode name=\"0100000\">388</cbc:InvoiceTypeCode>"));
    assert!(cn_xml.contains("<cbc:InvoiceTypeCode name=\"0100000\">381</cbc:InvoiceTypeCode>"));
    assert_ne!(
        inv_xml, cn_xml,
        "388 and 381 documents must differ on the wire"
    );

    // Each is deterministic across repeated serialization.
    assert_eq!(to_zatca_ubl_xml(&inv, &inv_ctx).unwrap(), inv_xml);
    assert_eq!(to_zatca_ubl_xml(&cn, &cn_ctx).unwrap(), cn_xml);
}

#[test]
fn saudi_qr_tlv_is_deterministic_and_well_formed() {
    // The ZATCA Phase 2 QR (FATOORA Technical Guideline §V) is a TLV envelope:
    // tag(1) | length(1) | value. The report's TLV bytes must be byte-stable
    // across two runs of the same lifecycle, and non-empty.
    let (_, report_a) = run_lifecycle(InvoiceMode::Standard, None);
    let (_, report_b) = run_lifecycle(InvoiceMode::Standard, None);
    assert_eq!(
        report_a.qr_tlv, report_b.qr_tlv,
        "the QR-code TLV must be byte-deterministic"
    );
    assert!(!report_a.qr_tlv.is_empty());

    // The TLV is self-describing: the first byte is a 1-byte tag, the second is
    // a 1-byte length, and tag+len+value never overruns the buffer.
    let tlv = &report_a.qr_tlv;
    let mut i = 0usize;
    let mut seen_tags = Vec::new();
    while i + 2 <= tlv.len() {
        let tag = tlv[i];
        let len = tlv[i + 1] as usize;
        assert!(
            i + 2 + len <= tlv.len(),
            "TLV field tag {tag} length {len} overruns the {}-byte buffer",
            tlv.len()
        );
        seen_tags.push(tag);
        i += 2 + len;
    }
    assert_eq!(i, tlv.len(), "TLV must consume the buffer exactly");
    // Tags are emitted in ascending order (ZATCA §V).
    let mut sorted = seen_tags.clone();
    sorted.sort_unstable();
    assert_eq!(seen_tags, sorted, "ZATCA QR TLV tags must ascend");
}
