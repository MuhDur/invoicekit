// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Dominican Republic DGII offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for the Dominican Republic and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("DO")` +
//!    the Dominican peso (`DOP`)
//! 2. serialize -> UBL 2.1 bytes via `invoicekit_format_ubl::to_xml` (the
//!    EN 16931 / UBL family path; DGII's wire format is national XML, but the
//!    offline honest-bar uses the UBL family artifact as the canonical payload)
//! 3. submit those bytes to the crate's existing `MockDgiiProvider` and assert
//!    the DGII authority receipt's country-specific fields (TrackId, echoed
//!    e-NCF, `Aceptado` status, recorded timestamp)
//! 4. assemble a `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml` +
//!    `receipt.json`), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock runs the same pre-wire validators as a real impl, so a
//!    bad RNC / bad e-NCF / empty payload is refused with `Err` before the wire
//!
//! Note on the authority `Rechazado` verdict: `MockDgiiProvider` always returns
//! `DgiiStatus::Aceptado` on a well-formed submission and exposes no
//! `with_forced_receipt`-style knob, so an authority-side `Rechazado` cannot be
//! forced here. The refusal path that the mock *does* support — pre-wire shape
//! refusal returned as `Err(DgiiError::*)` — is exercised instead (step 6).
//!
//! Goldens are hand-rolled where needed (no `insta` / `pretty_assertions`,
//! which would mutate `Cargo.lock`).

// Bare type/identifier names (TrackId, e-NCF, RNC, MockDgiiProvider) read fine
// in prose here; mirror the crate's own `src/lib.rs` allow.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_do_dgii::{
    DgiiDocumentKind, DgiiEnvironment, DgiiProvider, DgiiStatus, DgiiSubmitRequest,
    MockDgiiProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_do_e2e";
const TRACE: &str = "trace_do_e2e";
const ISSUER_RNC: &str = "131234567"; // 9-digit Dominican RNC
const E_NCF: &str = "E310000000001"; // E + type 31 + 10-digit sequential
const FIXED_RECEIVED_AT: &str = "2026-01-01T00:00:00Z";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

/// A Dominican party (`CountryCode("DO")`) carrying an RNC tax id.
fn dominican_party(name: &str, rnc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "rnc".to_owned(),
            value: rnc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Av. Winston Churchill 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "10101".to_owned(),
            country: CountryCode::new("DO").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Dominican B2B `CommercialDocument` denominated in DOP.
fn dominican_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-do-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-DO-0001").unwrap(),
        currency: Iso4217Code::new("DOP").unwrap(),
        supplier: dominican_party("Empresa Dominicana SRL", ISSUER_RNC, "Santo Domingo"),
        customer: dominican_party("Cliente Caribe SAS", "401234567", "Santiago"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoría".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        // ITBIS (the Dominican VAT) is 18%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1800),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11800),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11800),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

fn submit_request(ubl_xml: Vec<u8>) -> DgiiSubmitRequest {
    DgiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DgiiEnvironment::Sandbox,
        kind: DgiiDocumentKind::FacturaCreditoFiscal,
        issuer_rnc: ISSUER_RNC.to_owned(),
        e_ncf: E_NCF.to_owned(),
        ecf_xml: ubl_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to DGII mock -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_do_dgii::DgiiSubmitEnvelope) {
    // 1. build
    let doc = dominican_invoice();

    // 2. serialize -> UBL 2.1 bytes (EN 16931 / UBL family path)
    let ubl = to_xml(&doc).unwrap();
    // Local structural sanity: the canonical UBL spine is present. The
    // canonicalizer emits per-element namespace declarations, so elements
    // appear as `<cac:AccountingSupplierParty xmlns:cac="...">`; match the
    // open tag and the carried DOP currency value.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">DOP</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit those bytes to the existing MockDgiiProvider.
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);
    let envelope = provider.submit_ecf(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + UBL XML + DGII receipt.
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
fn dominican_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Step 3 success criterion: DGII accepted, with the country-specific
    // artifacts populated.
    assert_eq!(envelope.status, DgiiStatus::Aceptado);
    assert!(
        envelope.track_id.starts_with("DGII-"),
        "TrackId must carry the DGII country prefix, got {:?}",
        envelope.track_id
    );
    assert_eq!(envelope.e_ncf, E_NCF, "DGII must echo the submitted e-NCF");
    assert_eq!(envelope.received_at, FIXED_RECEIVED_AT);
    assert!(
        envelope.mensaje.is_none(),
        "Aceptado verdict carries no DGII mensaje"
    );

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn dominican_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn dominican_refusal_is_returned_as_err_before_the_wire() {
    // The mock runs the same pre-wire validators a real impl runs. A malformed
    // RNC, a malformed e-NCF, and an empty payload are all refused as typed
    // `Err`s *before* anything hits the wire. (An authority-side `Rechazado`
    // verdict cannot be forced through this mock — see the module doc.)
    let doc = dominican_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);

    // (a) bad RNC.
    let mut bad_rnc = submit_request(ubl_bytes.clone());
    bad_rnc.issuer_rnc = "NOTADIGIT".to_owned();
    assert!(
        matches!(
            provider.submit_ecf(&bad_rnc),
            Err(invoicekit_report_do_dgii::DgiiError::BadRnc(_))
        ),
        "a malformed RNC must be refused before the wire"
    );

    // (b) bad e-NCF.
    let mut bad_ncf = submit_request(ubl_bytes);
    bad_ncf.e_ncf = "X310000000001".to_owned();
    assert!(
        matches!(
            provider.submit_ecf(&bad_ncf),
            Err(invoicekit_report_do_dgii::DgiiError::BadENcf(_))
        ),
        "a malformed e-NCF must be refused before the wire"
    );

    // (c) empty payload.
    let mut empty = submit_request(Vec::new());
    empty.ecf_xml.clear();
    assert!(
        matches!(
            provider.submit_ecf(&empty),
            Err(invoicekit_report_do_dgii::DgiiError::BadXml(_))
        ),
        "an empty payload must be refused before the wire"
    );
}
