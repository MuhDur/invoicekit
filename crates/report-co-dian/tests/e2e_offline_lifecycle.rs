// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Colombia DIAN offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Colombia and proves it
//! deterministically, mirroring the proven `report-it-sdi` pattern:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("CO")`
//!    and the Colombian peso (`COP`)
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml` (DIAN's
//!    payload is the UBL 2.1 + DIAN CIUS family; this crate exposes no
//!    serializer of its own, so the EN 16931 / UBL path is the honest source)
//! 3. submit those bytes to the existing offline `MockDianProvider`, asserting
//!    the DIAN authority artefacts (96-char CUFE, `DIAN-` track id, status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the mock has no forced-`Rechazado` knob (its happy path always
//!    returns `Procesando`), so the only authority-refusal route is the
//!    pre-wire `Err(DianError::BadNit)` / `Err(DianError::BadXml)` validation
//!    the real adapter runs. That genuine refusal path is exercised below.
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would
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
use invoicekit_report_co_dian::{
    DianDocumentKind, DianEnvironment, DianError, DianProvider, DianStatus, DianSubmitRequest,
    MockDianProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const FIXED_SUBMITTED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_co_e2e";
const TRACE: &str = "trace_co_e2e";
const ISSUER_NIT: &str = "900123456-7";
const BUYER_NIT: &str = "800987654";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn colombian_party(name: &str, nit: &str, city: &str, subdivision: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "nit".to_owned(),
            value: nit.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Carrera 7 # 1-00".to_owned()],
            city: city.to_owned(),
            subdivision: Some(subdivision.to_owned()),
            postal_code: "110111".to_owned(),
            country: CountryCode::new("CO").unwrap(),
        },
        contact: None,
    }
}

fn colombian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-co-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("SETP-2026-CO-0001").unwrap(),
        // Colombian peso. 19% IVA on a 100.00 line -> 119.00 payable.
        currency: Iso4217Code::new("COP").unwrap(),
        supplier: colombian_party("Acme SAS", "900123456-7", "Bogota", "DC"),
        customer: colombian_party("Beta Ltda", "800987654", "Medellin", "ANT"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoria y desarrollo de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11900),
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

fn submit_request(invoice_xml: Vec<u8>) -> DianSubmitRequest {
    DianSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DianEnvironment::Habilitacion,
        kind: DianDocumentKind::FacturaVenta,
        issuer_nit: ISSUER_NIT.to_owned(),
        buyer_nit: Some(BUYER_NIT.to_owned()),
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize -> submit (mock DIAN) -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the DIAN envelope so callers can
/// assert both the authority artefacts and the verifiability of the bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_co_dian::DianSubmitEnvelope) {
    // 1. build
    let doc = colombian_invoice();

    // 2. serialize -> UBL 2.1 (the DIAN CIUS payload family).
    let ubl = to_xml(&doc).unwrap();
    // Structural spot-check: the canonical UBL spine is present. The
    // canonicalizer inlines the namespace declarations onto each element, so
    // assert on the prefixed element-name openers (no trailing `>`) plus the
    // Colombian peso currency code in the body.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">COP</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL payload missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the offline mock DIAN provider.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + DIAN receipt.
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
fn colombia_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // DIAN authority artefacts: 96-char CUFE, DIAN- track id, Procesando verdict.
    assert_eq!(envelope.cufe.len(), 96, "CUFE must be 96 hex chars");
    assert!(
        envelope.cufe.bytes().all(|b| b.is_ascii_hexdigit()),
        "CUFE must be lowercase hex"
    );
    assert!(
        envelope.track_id.starts_with("DIAN-"),
        "track id must carry the DIAN- prefix"
    );
    assert_eq!(envelope.status, DianStatus::Procesando);
    assert_eq!(envelope.submitted_at, FIXED_SUBMITTED_AT);
    assert!(envelope.message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn colombia_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn colombia_refusal_is_surfaced_as_err_before_the_wire() {
    // DIAN's mock has NO forced-`Rechazado` knob: the happy path always returns
    // `Procesando`. The only authority-refusal route the adapter models is the
    // pre-wire validation that runs BEFORE the payload reaches DIAN, surfaced as
    // an `Err` (per the project's "rejection-is-not-an-error" contract, a true
    // DIAN `Rechazado` verdict would be an Ok-envelope, but the mock does not
    // synthesize one). Exercise both genuine refusal shapes the mock can force.
    let provider = MockDianProvider::with_fixed_submitted_at(FIXED_SUBMITTED_AT);
    let ubl = to_xml(&colombian_invoice()).unwrap().into_bytes();

    // Bad issuer NIT -> BadNit, before the wire.
    let mut bad_nit = submit_request(ubl);
    bad_nit.issuer_nit = "NOT-A-NIT".to_owned();
    let err = provider.submit_invoice(&bad_nit).unwrap_err();
    assert!(matches!(err, DianError::BadNit(_)), "got {err:?}");

    // Empty payload -> BadXml, before the wire.
    let mut empty = submit_request(Vec::new());
    empty.buyer_nit = None;
    let err = provider.submit_invoice(&empty).unwrap_err();
    assert!(matches!(err, DianError::BadXml(_)), "got {err:?}");
}
