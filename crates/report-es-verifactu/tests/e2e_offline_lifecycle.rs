// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Spain VeriFactu offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Spain and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Spanish supplier +
//!    EUR currency
//! 2. serialize -> EN 16931 / UBL 2.1 XML (Spain rides the UBL family path;
//!    the live AEAT envelope wraps this)
//! 3. submit the UBL bytes to the in-crate `MockVeriFactuProvider` and assert
//!    the AEAT receipt's country-specific fields (CSV + recorded hash + status)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only)` it (exit 0 == report.ok)
//! 5. determinism: pack twice -> byte-identical
//! 6. hash-chain continuity: a second invoice that pins the first invoice's
//!    `recorded_hash_hex` as its `previous_hash_hex` is accepted
//!
//! Mirrors the proven `report-it-sdi` offline-E2E pattern. Goldens are
//! hand-rolled (no `insta`/`pretty_assertions`, which would mutate `Cargo.lock`).
//!
//! Note on the rejection path: `MockVeriFactuProvider` always returns
//! `VeriFactuStatus::Accepted` and exposes no `with_forced_*` knob, so a forced
//! AEAT *refusal* cannot be driven here. What the mock *does* support is
//! pre-wire *shape* refusal (`Err`): a malformed NIF, a malformed previous
//! hash, or an empty payload. Those are exercised in
//! `verifactu_rejects_bad_shapes_before_the_wire`.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_es_verifactu::{
    MockVeriFactuProvider, VeriFactuEnvironment, VeriFactuMode, VeriFactuProvider,
    VeriFactuRegisterEnvelope, VeriFactuRegisterRequest, VeriFactuStatus,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_es_e2e";
const TRACE: &str = "trace_es_e2e";
const ISSUER_NIF: &str = "A12345678";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn spanish_party(name: &str, vat: &str, city: &str, subdivision: &str, postal: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Calle Mayor 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(subdivision.to_owned()),
            postal_code: postal.to_owned(),
            country: CountryCode::new("ES").unwrap(),
        },
        contact: None,
    }
}

fn spanish_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-es-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("F2026/0007").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: spanish_party("Acme SL", "ESA12345678", "Madrid", "M", "28013"),
        customer: spanish_party("Beta SA", "ESB98765432", "Barcelona", "B", "08001"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoria y desarrollo de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100),
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12100),
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

fn register_request(
    invoice_xml: Vec<u8>,
    previous_hash_hex: Option<String>,
) -> VeriFactuRegisterRequest {
    VeriFactuRegisterRequest {
        tenant_id: TENANT.to_owned(),
        environment: VeriFactuEnvironment::Sandbox,
        mode: VeriFactuMode::VeriFactu,
        issuer_nif: ISSUER_NIF.to_owned(),
        invoice_number: "F2026/0007".to_owned(),
        issued_at: "2026-07-01T10:00:00Z".to_owned(),
        previous_hash_hex,
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> register with the AEAT mock ->
/// assemble + return the packed `.ikb` plus the AEAT receipt.
fn run_lifecycle() -> (Vec<u8>, VeriFactuRegisterEnvelope) {
    // 1. build
    let doc = spanish_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (Spain rides the UBL family).
    let ubl: String = to_xml(&doc).unwrap();
    // Spot-check the national-relevant UBL spine before it hits the wire.
    // Match local names without the closing `>` because canonicalization may
    // attach inline `xmlns:` declarations right after the element name.
    for needle in [
        "<Invoice",
        "cac:AccountingSupplierParty",
        "cac:AccountingCustomerParty",
        "cbc:DocumentCurrencyCode",
        ">EUR</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. register with the AEAT (offline deterministic mock).
    let provider = MockVeriFactuProvider::default();
    let envelope = provider
        .register_invoice(&register_request(ubl_bytes.clone(), None))
        .unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + AEAT receipt.
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
fn spain_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: AEAT recorded the invoice. Assert the Spain-specific artifacts.
    assert_eq!(envelope.status, VeriFactuStatus::Accepted);
    // CSV (Codigo Seguro de Verificacion) is what the printed-invoice QR carries.
    assert!(
        envelope.csv.starts_with("MOCK-CSV-"),
        "AEAT must assign a CSV, got {:?}",
        envelope.csv
    );
    // Recorded hash is the SHA-256-shaped chain link the next invoice pins.
    assert_eq!(
        envelope.recorded_hash_hex.len(),
        64,
        "recorded hash must be SHA-256 wire-shaped (64 hex chars)"
    );
    assert!(
        envelope
            .recorded_hash_hex
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "recorded hash must be lowercase hex"
    );
    assert_eq!(envelope.recorded_at, PINNED_CREATED_AT);
    assert!(envelope.message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn spain_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn spain_hash_chain_continuity_is_accepted() {
    // VeriFactu's anti-fraud spine is the hash chain: each invoice pins the
    // previous invoice's recorded hash. Prove the chain link the AEAT returned
    // for invoice #1 is a valid `previous_hash_hex` for invoice #2.
    let provider = MockVeriFactuProvider::default();
    let doc = spanish_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let first = provider
        .register_invoice(&register_request(ubl_bytes.clone(), None))
        .unwrap();
    assert_eq!(first.status, VeriFactuStatus::Accepted);

    let second = provider
        .register_invoice(&register_request(ubl_bytes, Some(first.recorded_hash_hex.clone())))
        .unwrap();
    assert_eq!(second.status, VeriFactuStatus::Accepted);
    // Distinct CSVs prove the AEAT serial advanced for the chained invoice.
    assert_ne!(first.csv, second.csv);
}

#[test]
fn verifactu_rejects_bad_shapes_before_the_wire() {
    // The mock has no forced-AEAT-refusal knob (it always returns Accepted), so
    // there is no `VeriFactuStatus::Rejected` path to drive offline. What it
    // does refuse, as `Err`, is pre-wire shape validation. Those refusals must
    // never reach the wire / a bundle.
    let provider = MockVeriFactuProvider::default();
    let ubl_bytes = to_xml(&spanish_invoice()).unwrap().into_bytes();

    // (a) malformed NIF (not 9 alphanumeric chars).
    let mut bad_nif = register_request(ubl_bytes.clone(), None);
    bad_nif.issuer_nif = "A12".to_owned();
    assert!(
        provider.register_invoice(&bad_nif).is_err(),
        "a malformed issuer NIF must be refused before the wire"
    );

    // (b) malformed previous hash (not 64 lowercase hex chars).
    let bad_hash = register_request(ubl_bytes, Some("not-a-sha256".to_owned()));
    assert!(
        provider.register_invoice(&bad_hash).is_err(),
        "a malformed previous hash must be refused before the wire"
    );

    // (c) empty payload.
    let empty = register_request(Vec::new(), None);
    assert!(
        provider.register_invoice(&empty).is_err(),
        "an empty payload must be refused before the wire"
    );
}
