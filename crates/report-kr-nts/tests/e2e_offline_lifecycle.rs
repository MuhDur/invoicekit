// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! South Korea NTS offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Korea and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a `KR` country code and
//!    KRW currency;
//! 2. serialize -> EN16931/UBL XML bytes via `invoicekit_format_ubl::to_xml`
//!    (Korea's e-Tax Invoice payload is national typed XML on the wire, but the
//!    canonical IR projection used for evidence is the UBL family path);
//! 3. submit those bytes to the crate's existing `MockNtsProvider` and assert
//!    the NTS-specific receipt fields (approval number / status / timestamp);
//! 4. assemble an `.ikb` evidence bundle ({canonical.json, formats/ubl.xml,
//!    receipt.json}) and `verify_packed(content_only).ok == true`;
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`;
//! 6. refusal: pre-wire shape failures (bad BRN / empty payload) surface as
//!    `Err`, while the distinct authority-side `NtsStatus::Rejected` verdict
//!    surfaces as a recorded receipt (never an `Err`) that still verifies as
//!    evidence — both contracts are covered below.
//!
//! Country-specific variations exercised in addition to the happy path:
//! `면세` (tax-exempt, zero-VAT) invoices, `수정세금계산서` (correction /
//! UBL CreditNote) documents, multi-line invoices, the real National Tax
//! Service `사업자등록번호` (BRN) check-digit rule, and authority refusal.
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`).

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_kr_nts::{
    validate_brn_checksum, MockNtsProvider, NtsEnvironment, NtsError, NtsInvoiceKind, NtsProvider,
    NtsStatus, NtsSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const FIXED_ISSUED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_kr_e2e";
const TRACE: &str = "trace_kr_e2e";
// Issuer 사업자등록번호 (Business Registration Number), 10 digits, hyphenated.
const ISSUER_BRN: &str = "123-45-67890";

/// KRW is a zero-decimal currency; build amounts at scale 0 (whole won).
fn won(amount: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(amount, 0))
}

fn korean_party(name: &str, brn: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "brn".to_owned(),
            value: brn.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Teheran-ro 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("Seoul".to_owned()),
            postal_code: "06234".to_owned(),
            country: CountryCode::new("KR").unwrap(),
        },
        contact: None,
    }
}

fn korean_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-kr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-KR-0001").unwrap(),
        currency: Iso4217Code::new("KRW").unwrap(),
        supplier: korean_party("Hangukkit Co Ltd", "1234567890", "Seoul"),
        customer: korean_party("Dongbang Trading", "9876543210", "Busan"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting (소프트웨어 컨설팅)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: won(500_000),
            line_extension_amount: won(1_000_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: won(1_000_000),
            // Korean VAT is a flat 10%.
            tax_amount: won(100_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: won(1_000_000),
            tax_exclusive_amount: won(1_000_000),
            tax_inclusive_amount: won(1_100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: won(1_100_000),
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

fn provider() -> MockNtsProvider {
    MockNtsProvider::with_fixed_issued_at(FIXED_ISSUED_AT)
}

fn submit_request(invoice_xml: Vec<u8>) -> NtsSubmitRequest {
    NtsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: NtsEnvironment::Test,
        kind: NtsInvoiceKind::Standard,
        issuer_brn: ISSUER_BRN.to_owned(),
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit (NTS mock) -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_kr_nts::NtsSubmitEnvelope) {
    // 1. build the IR document.
    let doc = korean_invoice();

    // 2. serialize -> EN16931/UBL bytes (the family path).
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    // 3. submit to the existing NTS mock provider; assert NTS receipt fields.
    let envelope = provider().submit(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR + national/UBL XML + NTS receipt.
    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    (ikb, envelope)
}

/// Pack {canonical.json, formats/ubl.xml, receipt.json} into a verifiable
/// `.ikb`. Shared so every scenario below ends at the same step-4 bar
/// (`verify_packed(content_only).ok == true`) as the happy path.
fn bundle_for(
    doc: &CommercialDocument,
    ubl_bytes: &[u8],
    envelope: &invoicekit_report_kr_nts::NtsSubmitEnvelope,
) -> Vec<u8> {
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

/// 면세 (tax-exempt) e-Tax Invoice: a VAT-exempt supply such as financial,
/// educational, or medical services under Korean VAT law. Tax category "E",
/// zero tax, and the issuer files it as [`NtsInvoiceKind::Exempt`] (면세
/// 계산서) rather than a 일반 taxable invoice.
fn korean_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-kr-exempt-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-KR-EX01").unwrap(),
        currency: Iso4217Code::new("KRW").unwrap(),
        supplier: korean_party("Hanguk Academy", "1234567890", "Seoul"),
        customer: korean_party("Dongbang Trading", "9876543210", "Busan"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Vocational training (직업 교육)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: won(800_000),
            line_extension_amount: won(800_000),
            // E = exempt from output VAT.
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: won(800_000),
            // Exempt supply: no output VAT charged.
            tax_amount: won(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: won(800_000),
            tax_exclusive_amount: won(800_000),
            // No VAT added: inclusive == exclusive.
            tax_inclusive_amount: won(800_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: won(800_000),
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

/// 수정세금계산서 (corrective / credit note): NTS requires a separate
/// correction document referencing the original when an issued e-Tax Invoice
/// must be reduced or cancelled. Modelled as a UBL `CreditNote` (TypeCode 381)
/// and filed as [`NtsInvoiceKind::Correction`]. Per the UBL serializer rule a
/// CreditNote MUST NOT carry a top-level DueDate, so `due_date` is `None`.
fn korean_correction_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-kr-corr-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote cannot carry cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CRN-2026-KR-0001").unwrap(),
        currency: Iso4217Code::new("KRW").unwrap(),
        supplier: korean_party("Hangukkit Co Ltd", "1234567890", "Seoul"),
        customer: korean_party("Dongbang Trading", "9876543210", "Busan"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Correction: returned consulting hours (반품)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: won(500_000),
            line_extension_amount: won(500_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: won(500_000),
            // 10% Korean VAT on the corrected (reduced) amount.
            tax_amount: won(50_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: won(500_000),
            tax_exclusive_amount: won(500_000),
            tax_inclusive_amount: won(550_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: won(550_000),
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

/// Multi-line 일반 (standard) taxable invoice: two lines, each at the flat
/// 10% Korean VAT, summed into one "S" tax-category total.
fn korean_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-kr-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-KR-ML01").unwrap(),
        currency: Iso4217Code::new("KRW").unwrap(),
        supplier: korean_party("Hangukkit Co Ltd", "1234567890", "Seoul"),
        customer: korean_party("Dongbang Trading", "9876543210", "Busan"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Software consulting (소프트웨어 컨설팅)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: won(500_000),
                line_extension_amount: won(1_000_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Annual support (연간 유지보수)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: won(2_000_000),
                line_extension_amount: won(2_000_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: won(3_000_000),
            // 10% of 3,000,000 won.
            tax_amount: won(300_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: won(3_000_000),
            tax_exclusive_amount: won(3_000_000),
            tax_inclusive_amount: won(3_300_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: won(3_300_000),
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
fn korea_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: NTS recorded an approval (전송완료).
    assert_eq!(envelope.status, NtsStatus::Approved);
    // NTS approval number is country-tagged (KR-) and 24 ASCII chars wide.
    assert!(
        envelope.approval_no.starts_with("KR-"),
        "approval number must carry the KR authority prefix, got {:?}",
        envelope.approval_no
    );
    assert_eq!(envelope.approval_no.len(), 24);
    // Pinned, deterministic NTS issuance timestamp.
    assert_eq!(envelope.issued_at, FIXED_ISSUED_AT);
    // Approved envelopes carry no rejection reason.
    assert!(envelope.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn korea_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn korea_refusal_is_an_error_not_a_receipt() {
    // Pre-wire shape failures must surface as Err (NOT as a receipt) and so
    // never reach the evidence bundle. (The distinct authority-side
    // NtsStatus::Rejected verdict — a recorded receipt, not an Err — is
    // covered separately by korea_authority_rejection_is_a_receipt below.)

    // (a) An invalid BRN (wrong digit shape) is refused before the wire.
    let mut bad_brn = submit_request(b"<Invoice/>".to_vec());
    bad_brn.issuer_brn = "12-34-5".to_owned();
    let err = provider().submit(&bad_brn).unwrap_err();
    assert!(
        matches!(err, NtsError::BadBrn(_)),
        "bad BRN must refuse with BadBrn, got {err:?}"
    );

    // (b) An empty payload is refused before the wire.
    let empty = submit_request(Vec::new());
    let err = provider().submit(&empty).unwrap_err();
    assert!(
        matches!(err, NtsError::BadXml(_)),
        "empty payload must refuse with BadXml, got {err:?}"
    );
}

/// Authority-side refusal as a RECEIPT (not an Err).
///
/// The clearance contract: when Hometax accepts transmission but the NTS
/// engine refuses the filing (전송오류 / transmission error — e.g. the
/// counterparty BRN is not on the NTS registry), the verdict is a recorded
/// receipt carrying [`NtsStatus::Rejected`] plus a reason, and that receipt
/// still belongs in the signed evidence bundle. Inverting this (returning
/// `Err`) would lose the audit trail.
///
/// Authority: National Tax Service (국세청, NTS), e-Tax Invoice
/// (전자세금계산서) clearance regime — <https://www.hometax.go.kr/>.
#[test]
fn korea_authority_rejection_is_a_receipt() {
    let doc = korean_invoice();
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    let reason = "공급받는자 사업자등록번호가 국세청에 등록되어 있지 않습니다";
    let provider = MockNtsProvider::with_fixed_issued_at(FIXED_ISSUED_AT)
        .with_forced_rejection(reason);
    let envelope = provider.submit(&submit_request(ubl_bytes.clone())).unwrap();

    // The refusal is a receipt, not an Err.
    assert_eq!(envelope.status, NtsStatus::Rejected);
    assert_eq!(envelope.reason.as_deref(), Some(reason));
    // Even a refused filing is recorded with a country-tagged approval number
    // and the pinned NTS issuance timestamp.
    assert!(envelope.approval_no.starts_with("KR-"));
    assert_eq!(envelope.approval_no.len(), 24);
    assert_eq!(envelope.issued_at, FIXED_ISSUED_AT);

    // The rejection receipt still packs into a verifiable evidence bundle.
    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "a rejection receipt must still verify as evidence");
}

/// 면세 (tax-exempt) e-Tax Invoice lifecycle.
///
/// A VAT-exempt supply carries zero output VAT, so tax-inclusive equals
/// tax-exclusive, and the issuer files it as [`NtsInvoiceKind::Exempt`]
/// (면세계산서). Authority: National Tax Service VAT Act exempt-supply rules
/// surfaced through the e-Tax Invoice regime — <https://www.hometax.go.kr/>.
#[test]
fn korea_tax_exempt_invoice_carries_zero_vat() {
    let doc = korean_exempt_invoice();

    // Zero-rated/exempt: no VAT added on top of the taxable base.
    assert_eq!(
        doc.tax_summary[0].tax_amount,
        won(0),
        "an exempt supply must carry zero output VAT"
    );
    assert_eq!(doc.tax_summary[0].category_code, "E");
    assert_eq!(
        doc.monetary_total.tax_inclusive_amount, doc.monetary_total.tax_exclusive_amount,
        "no VAT means inclusive == exclusive"
    );
    assert_eq!(doc.monetary_total.payable_amount, won(800_000));

    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    // Filed under the 면세 document kind.
    let mut req = submit_request(ubl_bytes.clone());
    req.kind = NtsInvoiceKind::Exempt;
    let envelope = provider().submit(&req).unwrap();
    assert_eq!(envelope.status, NtsStatus::Approved);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

/// 수정세금계산서 (correction / credit note) lifecycle.
///
/// NTS handles a reduction or cancellation of an issued e-Tax Invoice as a
/// separate correction document, modelled here as a UBL `CreditNote`
/// (CreditNoteTypeCode 381) and filed as [`NtsInvoiceKind::Correction`]. The
/// serializer emits the `<CreditNote>` root, not `<Invoice>`.
/// Authority: National Tax Service e-Tax Invoice correction rules —
/// <https://www.hometax.go.kr/>.
#[test]
fn korea_correction_note_lifecycle() {
    let doc = korean_correction_note();
    assert_eq!(doc.document_type, DocumentType::CreditNote);

    let xml = invoicekit_format_ubl::to_xml(&doc).unwrap();
    // The credit-note root is emitted, distinguishing a correction from a
    // standard invoice on the wire.
    assert!(
        xml.contains("<CreditNote"),
        "a correction must serialize to a UBL CreditNote root"
    );
    assert!(
        !xml.contains("<Invoice "),
        "a correction must not serialize to an Invoice root"
    );
    // UBL CreditNote carries CreditNoteTypeCode 381, never an InvoiceTypeCode.
    assert!(xml.contains("381"), "credit note must carry type code 381");

    let ubl_bytes = xml.into_bytes();
    let mut req = submit_request(ubl_bytes.clone());
    req.kind = NtsInvoiceKind::Correction;
    let envelope = provider().submit(&req).unwrap();
    assert_eq!(envelope.status, NtsStatus::Approved);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

/// Multi-line standard invoice: two taxable lines summing to one 10% VAT
/// total. Asserts the per-line and aggregate Korean-won arithmetic and that
/// both lines reach the serialized UBL.
#[test]
fn korea_multiline_invoice_lifecycle() {
    let doc = korean_multiline_invoice();
    assert_eq!(doc.lines.len(), 2);

    // Line extensions sum to the document total (whole won, scale 0).
    let sum = won(1_000_000).inner() + won(2_000_000).inner();
    assert_eq!(
        DecimalValue::new(sum),
        doc.monetary_total.line_extension_amount
    );
    // 10% Korean VAT on the 3,000,000-won base.
    assert_eq!(doc.tax_summary[0].tax_amount, won(300_000));
    assert_eq!(doc.monetary_total.tax_inclusive_amount, won(3_300_000));

    let xml = invoicekit_format_ubl::to_xml(&doc).unwrap();
    // Both line descriptions reach the wire.
    assert!(xml.contains("소프트웨어 컨설팅"));
    assert!(xml.contains("연간 유지보수"));

    let ubl_bytes = xml.into_bytes();
    let envelope = provider().submit(&submit_request(ubl_bytes.clone())).unwrap();
    assert_eq!(envelope.status, NtsStatus::Approved);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

/// Invalid issuer identifier rejected by the real NTS check-digit rule.
///
/// `validate_brn` only checks shape; the National Tax Service additionally
/// enforces a modulus-10 check digit on the tenth digit of the
/// 사업자등록번호. A BRN can be ten well-formed digits yet still be refused
/// because the check digit does not validate. Authority: NTS 사업자등록번호
/// check-digit algorithm — <https://www.hometax.go.kr/>.
#[test]
fn korea_brn_check_digit_rejects_well_formed_but_invalid_id() {
    // Real, publicly-listed company BRNs pass the check-digit rule:
    // Samsung Electronics 124-81-00998, NAVER 220-81-62517, Kakao 120-81-47521.
    assert!(validate_brn_checksum("124-81-00998").is_ok());
    assert!(validate_brn_checksum("220-81-62517").is_ok());
    assert!(validate_brn_checksum("120-81-47521").is_ok());

    // Well-formed (ten digits) but the check digit does not validate: the
    // shape gate passes, the checksum gate refuses.
    let bogus = "124-81-00999";
    assert!(
        invoicekit_report_kr_nts::validate_brn(bogus).is_ok(),
        "the bogus BRN must still pass the shape-only gate"
    );
    let err = validate_brn_checksum(bogus).unwrap_err();
    assert!(
        matches!(err, NtsError::BadBrn(_)),
        "an invalid check digit must refuse with BadBrn, got {err:?}"
    );
}

/// Determinism for the correction (credit-note) path.
///
/// The existing `korea_lifecycle_is_byte_deterministic` covers only the
/// standard invoice. Re-running the full correction lifecycle twice must
/// yield byte-identical `.ikb` bundles.
#[test]
fn korea_correction_lifecycle_is_byte_deterministic() {
    let run = || {
        let doc = korean_correction_note();
        let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();
        let mut req = submit_request(ubl_bytes.clone());
        req.kind = NtsInvoiceKind::Correction;
        let envelope = provider().submit(&req).unwrap();
        bundle_for(&doc, &ubl_bytes, &envelope)
    };
    assert_eq!(run(), run(), "the correction lifecycle must be byte-stable");
}
