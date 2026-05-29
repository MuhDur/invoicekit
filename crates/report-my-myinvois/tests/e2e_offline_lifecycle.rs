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
            extensions: Vec::new(),
        }],
        // Malaysia's standard SST rate is 6%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(600),
            tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
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
