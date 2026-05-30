// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Philippines BIR EIS offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for the Philippines and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `PH` parties + `PHP`
//! 2. serialize -> UBL 2.1 XML bytes via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path — BIR EIS ships a typed JSON envelope
//!    around the wire payload rather than its own national XML schema)
//! 3. submit those bytes to the crate's existing `MockEisProvider` and assert
//!    the BIR authority receipt's country-specific fields (BIR reference
//!    number + `Acknowledged` status + pinned `acknowledged_at`)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock's pre-wire validators (`Err`) reject a bad TIN /
//!    missing ATP / empty payload
//!
//! No `insta`/`pretty_assertions` (which would mutate `Cargo.lock`).
//!
//! Note on the rejection path: `MockEisProvider` never returns an
//! `Ok(envelope)` carrying `EisStatus::Rejected` — there is no
//! `with_forced_receipt`-style knob. The only refusal surface is the typed
//! `EisError` raised by the pre-wire validators, which step 6 exercises.

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
    EisDocumentKind, EisEnvironment, EisError, EisProvider, EisStatus, EisSubmitRequest,
    MockEisProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ACKNOWLEDGED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_ph_e2e";
const TRACE: &str = "trace_ph_e2e";
const ISSUER_TIN: &str = "123456789-001";
const ATP: &str = "ATP-2026-000001";

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

fn ph_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ph-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-PH-0001").unwrap(),
        currency: Iso4217Code::new("PHP").unwrap(),
        supplier: ph_party("Makati Trading Inc", "PH123456789", "Makati"),
        customer: ph_party("Cebu Logistics Corp", "PH987654321", "Cebu"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Philippine VAT is 12%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(12_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1200, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(112_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(112_000),
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

fn submit_request(invoice_json: Vec<u8>) -> EisSubmitRequest {
    EisSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EisEnvironment::Sandbox,
        kind: EisDocumentKind::SalesInvoice,
        issuer_tin: ISSUER_TIN.to_owned(),
        atp: ATP.to_owned(),
        invoice_json,
    }
}

/// Steps 1-4: build -> serialize -> submit to BIR EIS -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the BIR receipt envelope so callers
/// can assert both the authority verdict and bundle verifiability.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_ph_bir::EisSubmitEnvelope) {
    // 1. build a valid IR document for the Philippines.
    let doc = ph_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path).
    //    The canonicalizer declares namespaces inline on each element, so the
    //    spot-check needles match the canonical (not pretty) prefix shape.
    let ubl_xml = to_xml(&doc).unwrap();
    // Structural spot-check on the wire payload BIR EIS forwards.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">PHP</cbc:DocumentCurrencyCode>",
        ">1120.00</cbc:PayableAmount>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL XML missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit those bytes to the crate's MockEisProvider (deterministic
    //    pinned acknowledgement timestamp + serial reference number).
    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR JSON + UBL XML + BIR receipt JSON.
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
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
fn philippines_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // BIR EIS acknowledged the submission: country-specific receipt fields.
    assert_eq!(envelope.status, EisStatus::Acknowledged);
    assert!(
        envelope.reference_number.starts_with("BIR-"),
        "BIR reference number must carry the country-tagged prefix, got {:?}",
        envelope.reference_number
    );
    assert_eq!(envelope.acknowledged_at, PINNED_ACKNOWLEDGED_AT);
    assert!(envelope.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn philippines_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn philippines_refusal_paths_are_typed_errors() {
    // The MockEisProvider has no forced-rejection knob: it never returns an
    // Ok(envelope) with EisStatus::Rejected. Its refusal surface is the typed
    // EisError raised by the same pre-wire validators the real adapter runs.
    let provider = MockEisProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT);
    let valid_payload = to_xml(&ph_invoice()).unwrap().into_bytes();

    // Bad TIN.
    let mut bad_tin = submit_request(valid_payload.clone());
    bad_tin.issuer_tin = "BAD".to_owned();
    assert!(matches!(
        provider.submit_invoice(&bad_tin).unwrap_err(),
        EisError::BadTin(_)
    ));

    // Missing ATP.
    let mut no_atp = submit_request(valid_payload.clone());
    no_atp.atp.clear();
    assert!(matches!(
        provider.submit_invoice(&no_atp).unwrap_err(),
        EisError::MissingAtp
    ));

    // Empty payload.
    let mut empty = submit_request(valid_payload);
    empty.invoice_json.clear();
    assert!(matches!(
        provider.submit_invoice(&empty).unwrap_err(),
        EisError::BadJson(_)
    ));
}
