// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Chile SII DTE offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Chile and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("CL")` +
//!    a sensible ISO currency (CLP — the Chilean peso)
//! 2. serialize -> EN 16931 / UBL 2.1 bytes via `invoicekit_format_ubl::to_xml`
//!    (this crate exposes no serializer of its own; the SII DTE national XML
//!    is produced upstream and the report adapter takes the bytes verbatim)
//! 3. submit those bytes to the EXISTING `MockSiiProvider`, exercising the
//!    real Chilean RUT shape validator + CAF folio check, and assert the
//!    SII-specific receipt fields (TrackId, status, timestamp)
//! 4. poll the TrackId and assert the authority advances Recibido -> Aceptado
//! 5. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true`
//! 6. determinism: pack twice -> byte-identical
//! 7. refusal: the mock supports pre-wire refusals (bad RUT / zero folio /
//!    empty payload) which surface as `Err`, NOT as a receipt status — see
//!    `cl_lifecycle_refuses_malformed_input`. NOTE: `MockSiiProvider` exposes
//!    no knob to force an authority-side `SiiStatus::Rechazado` verdict, so
//!    that branch is not driven here; the genuine refusal surface is the
//!    local shape validation.
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`).

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_cl_dte::{
    DteKind, MockSiiProvider, SiiEnvironment, SiiError, SiiProvider, SiiStatus, SiiSubmitEnvelope,
    SiiSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_cl_e2e";
const TRACE: &str = "trace_cl_e2e";
/// Issuer RUT in the SII `NNNNNNNN-X` shape the adapter validates.
const ISSUER_RUT: &str = "76192083-9";
/// A folio consumed from the issuer's CAF bundle (must be non-zero).
const FOLIO: u64 = 4242;

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn chilean_party(name: &str, rut: &str, city: &str, region: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            // Chile keys parties by RUT; the IR carries it as a tax id.
            scheme: "CL:RUT".to_owned(),
            value: rut.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Av. Libertador Bernardo O'Higgins 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(region.to_owned()),
            postal_code: "8320000".to_owned(),
            country: CountryCode::new("CL").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Chilean B2B invoice (Factura Electrónica, tipo 33),
/// priced in Chilean pesos (CLP).
fn chilean_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cl-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("DTE-33-0001").unwrap(),
        currency: Iso4217Code::new("CLP").unwrap(),
        supplier: chilean_party("Proveedor SpA", ISSUER_RUT, "Santiago", "RM"),
        customer: chilean_party("Cliente Limitada", "77654321-0", "Valparaíso", "VS"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoría e ingeniería".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // IVA (Chilean VAT) is 19%.
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

fn submit_request(dte_xml: Vec<u8>) -> SiiSubmitRequest {
    SiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SiiEnvironment::Certification,
        kind: DteKind::FacturaElectronica,
        issuer_rut: ISSUER_RUT.to_owned(),
        folio: FOLIO,
        dte_xml,
    }
}

/// Steps 1-5: build -> serialize -> submit (SII) -> evidence bundle.
///
/// Returns the packed `.ikb` bytes and the SII submit envelope so callers can
/// assert both the receipt fields and the bundle.
fn run_lifecycle() -> (Vec<u8>, SiiSubmitEnvelope) {
    // 1. build
    let doc = chilean_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (this crate has no serializer;
    //    the report adapter consumes already-serialized DTE payload bytes).
    let ubl_xml = invoicekit_format_ubl::to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();

    // structural sanity: the UBL spine the SII payload is derived from. The
    // canonicalizer hoists namespace declarations onto each element, so match
    // on the element + the data rather than a bare tag form.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">CLP</cbc:DocumentCurrencyCode>",
        ">CL</cbc:IdentificationCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    // 3. submit to the existing Chilean mock; this runs the real RUT + folio
    //    validators and synthesizes a SII TrackId receipt.
    let provider = MockSiiProvider::new();
    let envelope = provider.submit_dte(&submit_request(ubl_bytes.clone())).unwrap();

    // 5. evidence bundle: canonical doc + UBL XML + SII receipt.
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
fn cl_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Step 3 success criterion: the SII accepted the upload and assigned a
    // country-specific TrackId; the first verdict is Recibido (received,
    // pending validation).
    assert_eq!(envelope.status, SiiStatus::Recibido);
    assert!(
        envelope.track_id.starts_with("SII-"),
        "SII TrackId must carry the country-tagged prefix, got {:?}",
        envelope.track_id
    );
    assert_eq!(envelope.submitted_at, "2026-01-01T00:00:00Z");
    assert!(envelope.glosa.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn cl_lifecycle_advances_recibido_to_aceptado_on_poll() {
    let provider = MockSiiProvider::new();
    let doc = chilean_invoice();
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    // submit -> Recibido
    let submitted = provider.submit_dte(&submit_request(ubl_bytes)).unwrap();
    assert_eq!(submitted.status, SiiStatus::Recibido);

    // poll the TrackId -> Aceptado (SII validation passed; the DTE is final).
    let polled = provider
        .query_track_id(SiiEnvironment::Certification, &submitted.track_id)
        .unwrap();
    assert_eq!(polled.status, SiiStatus::Aceptado);
    assert_eq!(polled.track_id, submitted.track_id);
}

#[test]
fn cl_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn cl_lifecycle_refuses_malformed_input() {
    // The mock has no knob to force an authority-side `SiiStatus::Rechazado`,
    // so the genuine refusal surface is pre-wire shape validation, which is an
    // `Err` (not a receipt status). Exercise all three buckets.
    let provider = MockSiiProvider::new();
    let doc = chilean_invoice();
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    // Bad RUT shape.
    let mut bad_rut = submit_request(ubl_bytes.clone());
    bad_rut.issuer_rut = "NOT-A-RUT".to_owned();
    assert!(matches!(
        provider.submit_dte(&bad_rut).unwrap_err(),
        SiiError::BadRut(_)
    ));

    // Zero folio (outside any CAF range).
    let mut zero_folio = submit_request(ubl_bytes.clone());
    zero_folio.folio = 0;
    assert!(matches!(
        provider.submit_dte(&zero_folio).unwrap_err(),
        SiiError::BadFolio(_)
    ));

    // Empty DTE payload.
    let mut empty_payload = submit_request(ubl_bytes);
    empty_payload.dte_xml.clear();
    assert!(matches!(
        provider.submit_dte(&empty_payload).unwrap_err(),
        SiiError::BadXml(_)
    ));
}
