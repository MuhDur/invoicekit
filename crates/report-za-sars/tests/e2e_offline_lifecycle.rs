// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! South Africa SARS offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for South Africa and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a `ZA` country code and
//!    the `ZAR` (South African rand) ISO-4217 currency;
//! 2. serialize -> EN 16931 / UBL 2.1 XML bytes (the family path; SARS has no
//!    bespoke national serializer in-tree yet);
//! 3. submit those bytes to the crate's existing `MockSarsProvider` and assert
//!    the SARS-specific receipt fields (`sars_ref` prefix + `Accepted` status);
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`;
//! 5. determinism: pack twice -> byte-identical;
//! 6. refusal path: bad VAT and empty payload are surfaced as `Err`.
//!
//! Note on the rejection contract: the SARS mock always returns
//! `SarsStatus::Accepted` and exposes no knob to force a
//! `SarsStatus::Rejected` envelope. The authority-`Rejected` verdict is
//! therefore NOT exercised here; instead the refusal test covers the two
//! pre-wire `Err` buckets the mock does support (`BadVat`, `BadPayload`).
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
use invoicekit_report_za_sars::{
    MockSarsProvider, SarsEnvironment, SarsError, SarsProvider, SarsStatus, SarsSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_za_e2e";
const TRACE: &str = "trace_za_e2e";
/// Issuer SARS VAT registration: 10 ASCII digits, always starting with `4`.
const ISSUER_VAT: &str = "4123456789";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn za_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 Adderley Street".to_owned()],
            city: city.to_owned(),
            subdivision: Some("Western Cape".to_owned()),
            postal_code: "8001".to_owned(),
            country: CountryCode::new("ZA").unwrap(),
        },
        contact: None,
    }
}

fn za_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ZA-0001").unwrap(),
        // South African rand.
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Beta Holdings", "4987654321", "Johannesburg"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL uses EA.
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // South African standard-rated VAT is 15%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(15000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
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
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

fn submit_request(payload: Vec<u8>) -> SarsSubmitRequest {
    SarsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SarsEnvironment::Sandbox,
        issuer_vat: ISSUER_VAT.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to SARS mock -> evidence
/// bundle. Returns the packed `.ikb` plus the SARS receipt.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_za_sars::SarsSubmitEnvelope) {
    // 1. build the IR document.
    let doc = za_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 XML bytes (the family path).
    let ubl_xml = to_xml(&doc).unwrap();
    // Cheap structural sanity: the canonical UBL spine is present and the
    // South African currency surfaced on the wire. The canonicalizer emits
    // namespace declarations inline on the first use of each prefix, so match
    // on the element-name prefix rather than a bare closing angle bracket.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        "ZAR",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit the serialized bytes to the existing SARS mock provider.
    let provider = MockSarsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national-family XML + SARS receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
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
fn za_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // SARS authority artifacts: an accepted verdict carrying a ZA-prefixed
    // reference and the pinned recorded-at timestamp from the deterministic
    // mock.
    assert_eq!(receipt.status, SarsStatus::Accepted);
    assert!(
        receipt.sars_ref.starts_with("ZA-"),
        "SARS reference must carry the ZA country prefix, got {:?}",
        receipt.sars_ref
    );
    assert_eq!(receipt.recorded_at, "2026-01-01T00:00:00Z");
    assert!(receipt.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn za_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn za_refusal_paths_are_errors_not_envelopes() {
    // The SARS mock cannot be forced to return SarsStatus::Rejected. The
    // two refusal buckets it DOES support are pre-wire shape failures, both
    // surfaced as Err (never an Accepted/Rejected envelope).
    let provider = MockSarsProvider::default();

    // Bad VAT registration (does not start with `4`).
    let mut bad_vat = submit_request(b"<Invoice/>".to_vec());
    bad_vat.issuer_vat = "5123456789".to_owned();
    let err = provider.submit_invoice(&bad_vat).unwrap_err();
    assert!(
        matches!(err, SarsError::BadVat(_)),
        "expected BadVat, got {err:?}"
    );

    // Empty payload.
    let empty = submit_request(Vec::new());
    let err = provider.submit_invoice(&empty).unwrap_err();
    assert!(
        matches!(err, SarsError::BadPayload(_)),
        "expected BadPayload, got {err:?}"
    );
}
