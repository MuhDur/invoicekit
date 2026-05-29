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
//! 6. refusal: the mock surfaces pre-wire shape failures (bad BRN / empty
//!    payload) as `Err`. It does NOT expose a knob to force an authority-side
//!    `NtsStatus::Rejected` envelope, so that path is covered as a refusal-Err
//!    test and noted below.
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_kr_nts::{
    MockNtsProvider, NtsEnvironment, NtsError, NtsInvoiceKind, NtsProvider, NtsStatus,
    NtsSubmitRequest,
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
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: won(1_000_000),
            // Korean VAT is a flat 10%.
            tax_amount: won(100_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
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
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, envelope)
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
    // The NTS mock has no knob to force an authority-side NtsStatus::Rejected
    // envelope, so the refusal contract is exercised through the two pre-wire
    // shape failures the mock genuinely enforces: both surface as Err, NOT as
    // an Approved receipt, and so never reach the evidence bundle.

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
