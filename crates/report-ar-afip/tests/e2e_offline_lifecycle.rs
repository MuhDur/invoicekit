// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Argentina AFIP offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Argentina and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) for an AR->AR invoice in ARS
//! 2. serialize -> UBL XML (the EN 16931 / UBL family path; AFIP has no
//!    bespoke serializer in this crate)
//! 3. submit the serialized bytes to the existing `MockAfipProvider` and assert
//!    the CAE envelope's Argentina-specific fields (14-digit CAE, expiry,
//!    `Aprobado` status, recorded timestamp)
//! 4. assemble a `.ikb` evidence bundle ({canonical.json, formats/ubl.xml,
//!    receipt.json}) and `verify_packed(content_only).ok == true` (exit 0)
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the mock runs the same CUIT / punto-de-venta / empty-payload
//!    validators the real adapter runs, surfaced as typed `Err` (pre-wire
//!    shape refusal)
//!
//! Note on the rejection path: `MockAfipProvider` does NOT expose a
//! forced-`Rechazado` knob — it always grants a CAE for a shape-valid request.
//! AFIP's authority-level `Rechazado`/`AprobadoConObservaciones` verdicts are
//! modelled in `AfipStatus` and round-tripped by the crate's own serde tests,
//! but cannot be forced through the offline mock here. The refusal we CAN drive
//! end-to-end is the pre-wire shape rejection (`AfipError::BadPayload` /
//! `BadCuit` / `BadPuntoVenta`), exercised in
//! `argentina_refuses_invalid_cuit_punto_venta_and_payload`.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ar_afip::{
    AfipCaeEnvelope, AfipCaeRequest, AfipEnvironment, AfipLetter, AfipProvider, AfipService,
    AfipStatus, MockAfipProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const FIXED_AUTHORIZED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_ar_e2e";
const TRACE: &str = "trace_ar_e2e";
const ISSUER_CUIT: &str = "20123456789";
const PUNTO_VENTA: &str = "00001";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn argentine_party(name: &str, cuit: &str, city: &str, province: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "cuit".to_owned(),
            value: cuit.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Av. Corrientes 1234".to_owned()],
            city: city.to_owned(),
            subdivision: Some(province.to_owned()),
            postal_code: "C1043".to_owned(),
            country: CountryCode::new("AR").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: build a valid AR->AR invoice in ARS (Argentine peso).
fn argentine_invoice() -> CommercialDocument {
    // IVA 21% domestic rate: 100.00 net -> 21.00 tax -> 121.00 gross.
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ar-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-01-01").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-01-31").unwrap()),
        document_number: DocumentNumber::new("0001-00000001").unwrap(),
        currency: Iso4217Code::new("ARS").unwrap(),
        supplier: argentine_party("Acme SRL", "20123456789", "Buenos Aires", "C"),
        customer: argentine_party("Beta SA", "27987654321", "Cordoba", "X"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoria de software".to_owned(),
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

fn cae_request(payload: Vec<u8>) -> AfipCaeRequest {
    AfipCaeRequest {
        tenant_id: TENANT.to_owned(),
        environment: AfipEnvironment::Homologacion,
        service: AfipService::Wsfe,
        letter: AfipLetter::A,
        issuer_cuit: ISSUER_CUIT.to_owned(),
        punto_venta: PUNTO_VENTA.to_owned(),
        request_payload: payload,
    }
}

/// Steps 1-4: build -> serialize -> request CAE -> assemble evidence bundle.
///
/// Returns the packed `.ikb` bytes and the AFIP CAE envelope so each test can
/// assert on the country-specific receipt and then on bundle verification.
fn run_lifecycle() -> (Vec<u8>, AfipCaeEnvelope) {
    // 1. build the IR document.
    let doc = argentine_invoice();

    // 2. serialize -> UBL XML bytes (EN 16931 / UBL family path).
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // 3. request a CAE from the offline mock, feeding it the serialized bytes
    //    as the canonical request payload.
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let envelope = provider.request_cae(&cae_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR JSON + national-family UBL + AFIP receipt.
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
fn argentina_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: AFIP granted a CAE. Assert the Argentina-specific receipt
    // fields the printed invoice and buyer lookup depend on.
    assert_eq!(envelope.status, AfipStatus::Aprobado);
    assert_eq!(envelope.cae.len(), 14, "CAE is a 14-digit AFIP code");
    assert!(
        envelope.cae.bytes().all(|b| b.is_ascii_digit()),
        "CAE must be all ASCII digits, got {:?}",
        envelope.cae
    );
    assert_eq!(envelope.cae_expiry_yyyymmdd, "20260131");
    assert_eq!(envelope.authorized_at, FIXED_AUTHORIZED_AT);
    assert!(
        envelope.observaciones.is_none(),
        "a clean Aprobado carries no observaciones"
    );

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn argentina_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn argentina_refuses_invalid_cuit_punto_venta_and_payload() {
    // The mock runs the SAME validators the real AFIP adapter runs. These are
    // pre-wire shape refusals surfaced as typed `Err` (not an AfipStatus). The
    // mock has no forced-`Rechazado` knob, so this is the refusal path we can
    // drive end-to-end. See the module doc for why.
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);

    // A shape-valid request grants a CAE...
    let valid = to_xml(&argentine_invoice()).unwrap().into_bytes();
    assert!(provider.request_cae(&cae_request(valid.clone())).is_ok());

    // ...empty payload is refused before the wire.
    let empty = provider.request_cae(&cae_request(Vec::new()));
    assert!(
        matches!(empty, Err(invoicekit_report_ar_afip::AfipError::BadPayload(_))),
        "empty payload must be refused as BadPayload, got {empty:?}"
    );

    // ...a malformed CUIT (not 11 ASCII digits) is refused.
    let mut bad_cuit = cae_request(valid.clone());
    bad_cuit.issuer_cuit = "NOT-A-CUIT".to_owned();
    let bad_cuit_res = provider.request_cae(&bad_cuit);
    assert!(
        matches!(
            bad_cuit_res,
            Err(invoicekit_report_ar_afip::AfipError::BadCuit(_))
        ),
        "malformed CUIT must be refused as BadCuit, got {bad_cuit_res:?}"
    );

    // ...a malformed punto de venta (not 5 ASCII digits) is refused.
    let mut bad_pv = cae_request(valid);
    bad_pv.punto_venta = "001".to_owned();
    let bad_pv_res = provider.request_cae(&bad_pv);
    assert!(
        matches!(
            bad_pv_res,
            Err(invoicekit_report_ar_afip::AfipError::BadPuntoVenta(_))
        ),
        "malformed punto de venta must be refused as BadPuntoVenta, got {bad_pv_res:?}"
    );
}
