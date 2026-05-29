// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Thailand Revenue Department (RD) offline end-to-end lifecycle.
//!
//! Drives the full local-only chain for Thailand and proves it
//! deterministically, mirroring the proven `report-it-sdi` pattern:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("TH")`
//!    and `Iso4217Code("THB")` (Thai baht)
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path; this crate exposes no national
//!    serializer of its own, so the e-Tax payload rides the UBL syntax)
//! 3. submit those bytes to the crate's existing `MockRdProvider` and
//!    assert the RD-specific receipt fields (`rd_ref` prefix `TH-`, the
//!    `Acknowledged` status, the pinned acknowledgement timestamp)
//! 4. assemble an `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml`
//!    + `receipt.json`) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock rejects an empty payload and a malformed Thai tax id
//!    with `Err` before the wire (see the note in `th_rejection_is_a_refusal`)
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would
//! mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_th_rd::{
    MockRdProvider, RdEnvironment, RdFlavour, RdProvider, RdStatus, RdSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ACKNOWLEDGED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_th_e2e";
const TRACE: &str = "trace_th_e2e";
// 13 ASCII digits — the exact Thai tax-id shape `validate_tax_id` enforces.
const ISSUER_TAX_ID: &str = "1234567890123";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn thai_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 Sukhumvit Road".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "10110".to_owned(),
            country: CountryCode::new("TH").unwrap(),
        },
        contact: None,
    }
}

fn thai_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-th-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-TH-0001").unwrap(),
        // Thai baht — a sensible ISO 4217 currency for a domestic RD invoice.
        currency: Iso4217Code::new("THB").unwrap(),
        supplier: thai_party("Acme (Thailand) Co Ltd", "TH0105551234567", "Bangkok"),
        customer: thai_party("Beta Trading Co Ltd", "TH0105559876543", "Nonthaburi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL uses EA
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Thai standard VAT is 7%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(7_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(700, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(107_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(107_000),
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

fn submit_request(ubl_xml: Vec<u8>) -> RdSubmitRequest {
    RdSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: RdEnvironment::Uat,
        flavour: RdFlavour::ETaxInvoice,
        issuer_tax_id: ISSUER_TAX_ID.to_owned(),
        payload: ubl_xml,
    }
}

fn provider() -> MockRdProvider {
    // Pin the acknowledgement timestamp so the receipt artefact is byte-stable.
    MockRdProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT)
}

/// Steps 1-4: build -> serialize -> submit -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the RD receipt so callers can assert
/// both the bundle and the country-specific receipt fields.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_th_rd::RdSubmitEnvelope) {
    // 1. build the IR document.
    let doc = thai_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path).
    let ubl_xml = to_xml(&doc).unwrap();
    // Structural smoke check on the national-family artefact spine. The C14N
    // canonicalizer attaches per-element namespace declarations, so each
    // prefixed element opens as `<cac:Foo xmlns:cac="...">` — match the
    // prefix-plus-space form, and match the currency by its element close so
    // the leading-attribute on the open tag is irrelevant.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
        "<cac:AccountingSupplierParty ",
        "<cac:AccountingCustomerParty ",
        ">THB</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL XML missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit to the existing offline MockRdProvider.
    let receipt = provider().submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR + national-family XML + RD receipt.
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
fn thailand_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Country-specific receipt assertions: RD reference, status, timestamp.
    assert_eq!(receipt.status, RdStatus::Acknowledged);
    assert!(
        receipt.rd_ref.starts_with("TH-"),
        "RD reference must carry the country-tagged TH- prefix, got {:?}",
        receipt.rd_ref
    );
    assert_eq!(receipt.acknowledged_at, PINNED_ACKNOWLEDGED_AT);
    assert!(receipt.reason.is_none(), "happy path carries no rejection reason");

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn thailand_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn thailand_rejection_is_a_refusal() {
    // The MockRdProvider always returns RdStatus::Acknowledged on a valid
    // submission — it exposes NO knob to force an RD-side `Rejected` verdict
    // envelope (unlike report-it-sdi's `with_forced_receipt`). The only
    // refusal paths it supports are the pre-wire `Err` validators, which we
    // exercise here: an empty payload and a malformed 13-digit Thai tax id.
    use invoicekit_report_th_rd::RdError;

    let p = provider();

    // Empty payload is refused before the wire.
    let mut empty = submit_request(Vec::new());
    empty.payload.clear();
    let err = p.submit_invoice(&empty).unwrap_err();
    assert!(matches!(err, RdError::BadPayload(_)), "empty payload must be a BadPayload Err");

    // A malformed Thai tax id is refused before the wire.
    let mut bad_id = submit_request(b"<Invoice/>".to_vec());
    bad_id.issuer_tax_id = "NOT-13-DIGITS".to_owned();
    let err = p.submit_invoice(&bad_id).unwrap_err();
    assert!(matches!(err, RdError::BadTaxId(_)), "malformed tax id must be a BadTaxId Err");
}
