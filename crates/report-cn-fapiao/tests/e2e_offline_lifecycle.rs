// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! China **Fapiao** offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for China and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("CN")` +
//!    ISO currency `CNY`;
//! 2. serialize -> UBL 2.1 XML bytes via `invoicekit_format_ubl::to_xml` (the
//!    EN 16931 / UBL family path; the national serializer lands later);
//! 3. submit those bytes to the in-crate `MockFapiaoProvider` and assert the
//!    STA receipt's country-specific fields (20-char fapiao number, 12-digit
//!    fapiao code, `Issued` status, pinned `issued_at`);
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true` (exit 0 == report.ok);
//! 5. determinism: pack twice -> byte-identical;
//! 6. refusal: the mock surfaces STA refusal as a typed `Err` (bad USCC /
//!    empty payload), and `void_fapiao` flips status to `Voided`.
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_cn_fapiao::{
    FapiaoEnvironment, FapiaoIssueEnvelope, FapiaoIssueRequest, FapiaoKind, FapiaoProvider,
    FapiaoStatus, MockFapiaoProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ISSUED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_cn_e2e";
const TRACE: &str = "trace_cn_e2e";
// Issuer 统一社会信用代码 (USCC): 18 ASCII alphanumeric chars.
const ISSUER_USCC: &str = "91110108MA01234567";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn chinese_party(name: &str, uscc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "uscc".to_owned(),
            value: uscc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["朝阳路 1 号".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "100020".to_owned(),
            country: CountryCode::new("CN").unwrap(),
        },
        contact: None,
    }
}

fn chinese_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cn-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-CN-0001").unwrap(),
        // CNY is the ISO 4217 code for the Chinese renminbi (yuan).
        currency: Iso4217Code::new("CNY").unwrap(),
        supplier: chinese_party("Acme 科技有限公司", ISSUER_USCC, "Beijing"),
        customer: chinese_party("Beta 贸易有限公司", "91310115MA1K3X9876", "Shanghai"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "软件咨询与开发服务".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // UBL uses UN/ECE Rec 20 "EA" (not CII's "C62").
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // China VAT at 6% on modern services.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(6_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(106_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(106_000),
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

fn issue_request(payload: Vec<u8>) -> FapiaoIssueRequest {
    FapiaoIssueRequest {
        tenant_id: TENANT.to_owned(),
        environment: FapiaoEnvironment::Sandbox,
        kind: FapiaoKind::ElectronicSpecial,
        issuer_uscc: ISSUER_USCC.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to STA mock -> evidence bundle.
///
/// Returns the packed `.ikb` bytes and the STA receipt so callers can assert
/// both the bundle and the country-specific receipt fields.
fn run_lifecycle() -> (Vec<u8>, FapiaoIssueEnvelope) {
    // 1. build a valid CN invoice in the IR.
    let doc = chinese_invoice();

    // 2. serialize -> UBL 2.1 (EN 16931 family path) -> bytes.
    let ubl = to_xml(&doc).unwrap();
    // local structural sanity: the canonical UBL spine is present. The
    // canonicalizer pins stable prefixes and writes namespace declarations
    // inline on each element, so assert against the canonical forms.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">CNY</cbc:DocumentCurrencyCode>",
        // the Chinese issuer USCC survives the round-trip into the XML.
        ISSUER_USCC,
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit the serialized bytes to the STA clearance mock.
    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);
    let receipt = provider.issue_fapiao(&issue_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR + national-family XML + STA receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&receipt).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, receipt)
}

#[test]
fn china_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Country-specific STA receipt fields (the fapiao identity).
    assert_eq!(receipt.status, FapiaoStatus::Issued);
    assert_eq!(
        receipt.fapiao_number.len(),
        20,
        "STA assigns a 20-character fapiao number"
    );
    assert_eq!(
        receipt.fapiao_code.len(),
        12,
        "STA assigns a 12-digit fapiao code (发票代码)"
    );
    assert!(receipt.fapiao_number.bytes().all(|b| b.is_ascii_digit()));
    assert!(receipt.fapiao_code.bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(receipt.issued_at, PINNED_ISSUED_AT);
    assert!(receipt.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn china_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn china_refusal_is_a_typed_error_not_a_silent_pass() {
    // The MockFapiaoProvider does NOT support forcing an STA `Rejected`
    // *status* on the issue path (its happy path always returns `Issued`).
    // Its refusal surface is a typed `Err`, exercised here: an issuer USCC of
    // the wrong shape is rejected pre-wire, and an empty payload is refused.
    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);

    let mut bad_uscc = issue_request(b"<Invoice/>".to_vec());
    bad_uscc.issuer_uscc = "NOT-A-VALID-USCC".to_owned();
    assert!(
        provider.issue_fapiao(&bad_uscc).is_err(),
        "an 18-char-shape USCC failure must be a typed Err"
    );

    let empty_payload = issue_request(Vec::new());
    assert!(
        provider.issue_fapiao(&empty_payload).is_err(),
        "an empty payload must be a typed Err"
    );

    // The `Rejected` and `Voided` verdicts are reachable as receipt *statuses*
    // (not Errs): a void flips an issued fapiao to `Voided` with a reason, and
    // the bundle of that audit record still verifies.
    let voided = provider
        .void_fapiao(
            FapiaoEnvironment::Sandbox,
            "00000000000000000001",
            "buyer dispute",
        )
        .unwrap();
    assert_eq!(voided.status, FapiaoStatus::Voided);
    assert_eq!(voided.reason.as_deref(), Some("buyer dispute"));

    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&voided).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "void-path audit bundle must verify");
}
