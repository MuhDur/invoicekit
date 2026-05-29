// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Kenya KRA eTIMS offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Kenya and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("KE")` + KES.
//! 2. serialize -> EN 16931 / UBL 2.1 bytes via `invoicekit_format_ubl::to_xml`
//!    (eTIMS itself accepts JSON over REST; the crate ships no own serializer, so
//!    the UBL family path is the canonical wire artefact we bundle as evidence).
//! 3. submit those bytes to the existing `MockEtimsProvider` and assert the
//!    KRA-specific receipt fields: CU Invoice Number, KRA signature, status.
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), pin `created_at`, `pack`, then `verify_packed` -> `ok == true`.
//! 5. determinism: pack twice -> byte-identical.
//! 6. refusal: the mock has no forced-`Rejected`-status knob (it always returns
//!    `Accepted`), so we exercise the genuine refusal surface it DOES expose —
//!    `Err(EtimsError::BadPin)` on a malformed KRA PIN and `Err(BadPayload)` on an
//!    empty payload — running the same `validate_pin` validator the real impl runs.
//!
//! No `insta`/`pretty_assertions` (they would mutate `Cargo.lock`); goldens are
//! the typed assertions below.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ke_etims::{
    EtimsEnvironment, EtimsError, EtimsProvider, EtimsStatus, EtimsSubmitRequest, MockEtimsProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_ke_e2e";
const TRACE: &str = "trace_ke_e2e";
const ISSUER_PIN: &str = "A123456789Z";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn kenyan_party(name: &str, kra_pin: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "kra-pin".to_owned(),
            value: kra_pin.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Kenyatta Avenue 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "00100".to_owned(),
            country: CountryCode::new("KE").unwrap(),
        },
        contact: None,
    }
}

/// Build a valid Kenyan B2B invoice in the IR. KES, 16% standard VAT.
fn kenyan_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ke-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-KE-0001").unwrap(),
        currency: Iso4217Code::new("KES").unwrap(),
        supplier: kenyan_party("Acme Kenya Ltd", "A123456789Z", "Nairobi"),
        customer: kenyan_party("Beta Traders Ltd", "P051234567M", "Mombasa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cloud hosting subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // UBL family uses UN/ECE Rec 20 code "EA" for "each".
            unit_code: Some("EA".to_owned()),
            unit_price: amt(500_000),
            line_extension_amount: amt(1_000_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000),
            tax_amount: amt(160_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000),
            tax_exclusive_amount: amt(1_000_000),
            tax_inclusive_amount: amt(1_160_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_160_000),
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

fn submit_request(payload: Vec<u8>) -> EtimsSubmitRequest {
    EtimsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EtimsEnvironment::Sandbox,
        issuer_pin: ISSUER_PIN.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize -> submit (mock) -> evidence bundle.
///
/// Returns the packed `.ikb` plus the KRA receipt so callers can assert on both.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_ke_etims::EtimsSubmitEnvelope) {
    // 1. build
    let doc = kenyan_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (the canonical wire artefact)
    let ubl_xml = to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();
    // Structural sanity: the UBL spine carrying KE identity + KES is present.
    // Canonicalization normalizes namespace prefixes and pins `xmlns:` decls
    // inline, so we match on the stable closing tags / open tags that survive.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">KES</cbc:DocumentCurrencyCode>",
        "<cac:Country>",
        ">KE</cbc:IdentificationCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    // 3. submit to the existing offline MockEtimsProvider
    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national wire XML + KRA receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&receipt).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, receipt)
}

#[test]
fn kenya_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Happy path: KRA accepted, with the country-specific authority artefacts.
    assert_eq!(receipt.status, EtimsStatus::Accepted);
    assert!(
        receipt.cu_invoice_number.starts_with("KE-"),
        "CU Invoice Number must carry the KE prefix, got {:?}",
        receipt.cu_invoice_number
    );
    assert!(
        receipt.kra_signature.starts_with("MOCK-SIG-"),
        "KRA signature must be present, got {:?}",
        receipt.kra_signature
    );
    assert_eq!(receipt.recorded_at, "2026-01-01T00:00:00Z");
    assert!(receipt.reason.is_none());

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn kenya_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn kenya_refuses_malformed_pin_before_the_wire() {
    // The MockEtimsProvider exposes no forced-`Rejected`-status knob (it always
    // returns Accepted), so the genuine refusal surface is the pre-wire
    // validator: a malformed KRA PIN is an Err, never a silent accept.
    let provider = MockEtimsProvider::default();
    let doc = kenyan_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let mut bad = submit_request(ubl_bytes);
    bad.issuer_pin = "NOT-A-PIN".to_owned();
    let err = provider.submit_invoice(&bad).unwrap_err();
    assert!(
        matches!(err, EtimsError::BadPin(_)),
        "malformed PIN must refuse with BadPin, got {err:?}"
    );
}

#[test]
fn kenya_refuses_empty_payload_before_the_wire() {
    // The second pre-wire refusal: an empty payload never reaches KRA.
    let provider = MockEtimsProvider::default();
    let err = provider.submit_invoice(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, EtimsError::BadPayload(_)),
        "empty payload must refuse with BadPayload, got {err:?}"
    );
}
