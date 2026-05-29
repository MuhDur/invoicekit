// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Egypt ETA offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Egypt and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("EG")` and
//!    a sensible ISO currency (`EGP`, the Egyptian Pound)
//! 2. serialize -> EN 16931 / UBL 2.1 bytes via `invoicekit_format_ubl::to_xml`
//!    (the ETA crate exposes no own serializer; it consumes signed payload
//!    bytes, so the family UBL path is the honest upstream)
//! 3. submit those bytes to the crate's existing `MockEtaProvider` and assert
//!    the ETA-specific receipt fields (UUID prefix, Long ID, 64-char content
//!    hash, status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the local pre-wire validators reject a bad national id and an
//!    empty payload with `Err`
//!
//! Note on the authority-`Invalid` verdict: `MockEtaProvider` has no
//! forced-receipt knob — it always synthesizes `EtaStatus::Submitted`. The
//! ETA `Invalid` clearance verdict therefore cannot be exercised offline; the
//! refusal coverage here is the crate's real pre-wire shape validation
//! (`EtaError::BadId` / `EtaError::BadPayload`), which returns `Err` exactly
//! as the adapter contract specifies. Goldens are hand-rolled (no `insta` /
//! `pretty_assertions`, which would mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_eg_eta::{
    EtaDocumentKind, EtaEnvironment, EtaError, EtaStatus, EtaSubmitRequest, EtaProvider,
    MockEtaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_eg_e2e";
const TRACE: &str = "trace_eg_e2e";
/// Egyptian tax registration number — 9 ASCII digits (the ETA shape).
const ISSUER_TAX_ID: &str = "100200300";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn egyptian_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "eta-tin".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["12 Tahrir Square".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "11511".to_owned(),
            country: CountryCode::new("EG").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Egyptian B2B invoice in `EGP`.
fn egyptian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-EG-0001").unwrap(),
        currency: Iso4217Code::new("EGP").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", "100200300", "Cairo"),
        customer: egyptian_party("Delta Imports", "400500600", "Alexandria"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulting services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(14_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1400, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(114_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(114_000),
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

fn submit_request(payload: Vec<u8>) -> EtaSubmitRequest {
    EtaSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EtaEnvironment::Preprod,
        kind: EtaDocumentKind::Invoice,
        issuer_tax_or_national_id: ISSUER_TAX_ID.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize -> submit to the mock ETA -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_eg_eta::EtaSubmitEnvelope) {
    // 1. build
    let doc = egyptian_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (the ETA crate has no own
    //    serializer; it consumes signed payload bytes, so UBL is the upstream).
    let ubl: Vec<u8> = to_xml(&doc).unwrap().into_bytes();

    // 3. submit to the EXISTING MockEtaProvider (deterministic timestamp).
    let provider = MockEtaProvider::new();
    let envelope = provider.submit(&submit_request(ubl.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national-family UBL + ETA receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl);
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
fn egypt_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // ETA-specific receipt assertions: UUID prefix, Long ID prefix, 64-char
    // content hash, and the cleared status.
    assert_eq!(envelope.status, EtaStatus::Submitted);
    assert!(
        envelope.uuid.starts_with("EG-"),
        "ETA UUID must carry the EG- prefix, got {:?}",
        envelope.uuid
    );
    assert!(
        envelope.long_id.starts_with("ETA-LONG-"),
        "ETA Long ID must carry the ETA-LONG- prefix, got {:?}",
        envelope.long_id
    );
    assert_eq!(
        envelope.content_hash_hex.len(),
        64,
        "ETA content hash must be a 64-char SHA-256 hex string"
    );
    assert_eq!(envelope.submitted_at, "2026-01-01T00:00:00Z");
    assert!(envelope.reason.is_none());

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn egypt_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn egypt_refuses_bad_national_id_before_the_wire() {
    // The mock runs the SAME pre-wire validators the real impl runs. A bad
    // tax/national id is an Err (shape refusal), not a cleared receipt.
    let provider = MockEtaProvider::new();
    let mut req = submit_request(to_xml(&egyptian_invoice()).unwrap().into_bytes());
    req.issuer_tax_or_national_id = "NOT-DIGITS".to_owned();
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, EtaError::BadId(_)),
        "a malformed national id must refuse with EtaError::BadId, got {err:?}"
    );
}

#[test]
fn egypt_refuses_empty_payload_before_the_wire() {
    // An empty payload is a pre-wire refusal. (The ETA authority `Invalid`
    // clearance verdict cannot be forced offline — the mock only ever returns
    // `Submitted` — so this Err path is the honest offline refusal coverage.)
    let provider = MockEtaProvider::new();
    let err = provider.submit(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, EtaError::BadPayload(_)),
        "an empty payload must refuse with EtaError::BadPayload, got {err:?}"
    );
}
