// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Taiwan MOF e-Invoice offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Taiwan and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("TW")` and
//!    the New Taiwan dollar (`TWD`).
//! 2. serialize -> UBL 2.1 XML (the EN 16931 / UBL family path) via
//!    `invoicekit_format_ubl::to_xml`.
//! 3. submit those bytes to the crate's existing `MockMofProvider` and assert
//!    the MOF authority receipt's Taiwan-specific fields: invoice number
//!    (發票字軌 `AA-nnnnnnnn`), the 4-digit lottery random number (統一發票
//!    兌獎隨機碼), and the `MofStatus::Accepted` (上傳成功) verdict.
//! 4. assemble an `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml` +
//!    `receipt.json`), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true` (exit 0 == report.ok).
//! 5. determinism: pack twice -> byte-identical.
//! 6. refusal: the mock refuses an invalid 統一編號 / empty payload with an
//!    `Err` (pre-wire shape validation). The mock does NOT forge an authority
//!    `MofStatus::Rejected`, so that wire-verdict branch cannot be forced here
//!    (see the note on `tw_mof_rejection_status_is_not_forceable`).
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
use invoicekit_report_tw_mof::{
    MockMofProvider, MofEnvironment, MofError, MofInvoiceKind, MofProvider, MofStatus,
    MofSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_tw_e2e";
const TRACE: &str = "trace_tw_e2e";
const ISSUER_UNIFORM_NUMBER: &str = "12345678";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn taiwanese_party(name: &str, uniform_number: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            // Taiwan's 統一編號 (uniform number / business id) carried as the
            // party tax id; MOF treats it as the issuer/buyer VAT-equivalent.
            scheme: "tw:ubn".to_owned(),
            value: uniform_number.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["No. 1, Section 1, Zhongxiao W. Rd.".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "100".to_owned(),
            country: CountryCode::new("TW").unwrap(),
        },
        contact: None,
    }
}

/// A minimal, valid B2B invoice routed inside Taiwan (TWD, 5% business tax).
fn taiwanese_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-tw-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-TW-0001").unwrap(),
        currency: Iso4217Code::new("TWD").unwrap(),
        supplier: taiwanese_party("Acme Co Ltd", ISSUER_UNIFORM_NUMBER, "Taipei"),
        customer: taiwanese_party("Beta Co Ltd", "87654321", "Kaohsiung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cloud platform subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Taiwan's standard business tax (營業稅) is 5%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(5_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(105_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(105_000),
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

fn submit_request(payload: Vec<u8>) -> MofSubmitRequest {
    MofSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: MofEnvironment::Test,
        kind: MofInvoiceKind::B2b,
        issuer_uniform_number: ISSUER_UNIFORM_NUMBER.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to MOF -> assemble `.ikb`.
///
/// Returns the packed bundle bytes plus the MOF receipt so each test can assert
/// the country-specific fields and re-verify.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_tw_mof::MofSubmitEnvelope) {
    // 1. build the canonical IR document.
    let doc = taiwanese_invoice();

    // 2. serialize -> UBL 2.1 (EN 16931 / UBL family). MOF MIG is XML; the UBL
    //    family path is the workspace-resolved serializer this crate reuses.
    let ubl = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();

    // 3. submit the serialized bytes to the existing offline mock provider.
    let provider = MockMofProvider::with_fixed_issued_at("2026-05-26T08:30:00Z");
    let receipt = provider.submit(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR + national UBL XML + MOF receipt.
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
fn taiwan_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Country-specific MOF authority artifacts:
    // - 發票字軌 invoice number: two-letter track + 8-digit serial.
    assert!(
        receipt.invoice_number.starts_with("AA-"),
        "MOF invoice number must carry the AA track prefix, got {:?}",
        receipt.invoice_number
    );
    assert_eq!(
        receipt.invoice_number.len(),
        11,
        "AA-nnnnnnnn is 11 chars, got {:?}",
        receipt.invoice_number
    );
    // - 統一發票兌獎 lottery random number: exactly 4 ASCII digits.
    assert_eq!(receipt.random_number.len(), 4);
    assert!(receipt.random_number.bytes().all(|b| b.is_ascii_digit()));
    // - 上傳成功 verdict + the timestamp MOF recorded.
    assert_eq!(receipt.status, MofStatus::Accepted);
    assert_eq!(receipt.issued_at, "2026-05-26T08:30:00Z");
    assert!(receipt.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn taiwan_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn taiwan_mof_refuses_invalid_uniform_number() {
    // Refusal path (anti-slop): a malformed 統一編號 is rejected pre-wire as an
    // `Err`, before any receipt is synthesized.
    let provider = MockMofProvider::default();
    let ubl_bytes = to_xml(&taiwanese_invoice()).unwrap().into_bytes();
    let mut req = submit_request(ubl_bytes);
    req.issuer_uniform_number = "BAD".to_owned();
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, MofError::BadUniformNumber(_)),
        "expected BadUniformNumber, got {err:?}"
    );
}

#[test]
fn taiwan_mof_refuses_empty_payload() {
    // The serializer always yields non-empty bytes; this guards the wire
    // contract that an empty MOF payload is refused pre-wire as an `Err`.
    let provider = MockMofProvider::default();
    let err = provider.submit(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, MofError::BadPayload(_)),
        "expected BadPayload, got {err:?}"
    );
}

#[test]
fn tw_mof_rejection_status_is_not_forceable() {
    // NOTE: the existing `MockMofProvider` has no `with_forced_*` hook. For
    // valid input it always returns `MofStatus::Accepted`; the authority
    // `MofStatus::Rejected` (上傳失敗) wire-verdict branch cannot be forced
    // through the mock today. The mock's only refusal surface is the pre-wire
    // `Err` path exercised by the two `refuses_*` tests above. This test pins
    // that behavior so a future forced-rejection hook is a deliberate change.
    let provider = MockMofProvider::default();
    let ubl_bytes = to_xml(&taiwanese_invoice()).unwrap().into_bytes();
    let receipt = provider.submit(&submit_request(ubl_bytes)).unwrap();
    assert_eq!(receipt.status, MofStatus::Accepted);
}
