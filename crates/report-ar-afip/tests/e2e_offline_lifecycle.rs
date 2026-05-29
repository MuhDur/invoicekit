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

// AFIP/Spanish terminology (CUIT, WSFE, FECAESolicitar, observaciones, AFIP
// spec URLs in citations) trips `doc_markdown`; the crate's own `lib.rs` allows
// it for the same reason, so the test module follows suit to keep the
// regulator-grounded citations readable rather than backtick-littered.
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

// ---------------------------------------------------------------------------
// Deepened, Argentina-specific coverage (added without weakening the above).
//
// All AFIP wire-fact values asserted below — comprobante (CbteTipo) codes, the
// IVA aliquot table, the WSFEX export class, and CUIT shape — are grounded in
// AFIP's own developer documentation, cited per-scenario:
//
//   - AFIP-SDG SIT Facturación Electrónica, "Manual para el desarrollador"
//     WSFEv1 (RG 4291), comprobante-type table FEParamGetTiposCbte and IVA
//     aliquot table FEParamGetTiposIva:
//     https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
//   - AFIP WSFEX (Factura de Exportación) "Manual para el desarrollador" v2.1.0:
//     https://afip.gob.ar/fe/documentos/WSFEX-Manualparaeldesarrollador_V2.1.0.pdf
//   - AFIP CUIT structure / módulo-11 verifier digit (Clave Única de
//     Identificación Tributaria): https://www.afip.gob.ar/
//
// AFIP comprobante (CbteTipo) codes used here (from FEParamGetTiposCbte):
//   1 = Factura A,  2 = Nota de Débito A,  3 = Nota de Crédito A,
//   6 = Factura B,  8 = Nota de Crédito B, 11 = Factura C, 19 = Factura E.
// AFIP IVA aliquot (Id) codes (from FEParamGetTiposIva):
//   3 = 0%,  4 = 10.5%,  5 = 21%,  6 = 27%.

/// One-line export customer party (foreign buyer carries no Argentine CUIT;
/// WSFEX class-E invoices identify the buyer by name/country, not by CUIT).
fn foreign_party(name: &str, city: &str, country: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: Vec::new(),
        address: PostalAddress {
            lines: vec!["1 Market Street".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "94105".to_owned(),
            country: CountryCode::new(country).unwrap(),
        },
        contact: None,
    }
}

/// Generic builder so each scenario shares one validated spine and only varies
/// the country-specific axis under test (document type, currency, line set,
/// IVA aliquot). Totals are passed in already balanced.
struct DocSpec {
    id: &'static str,
    number: &'static str,
    doc_type: DocumentType,
    currency: &'static str,
    due_date: Option<&'static str>,
    customer: Party,
    lines: Vec<DocumentLine>,
    tax_summary: Vec<TaxCategorySummary>,
    total: MonetaryTotal,
}

fn build_doc(spec: DocSpec) -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(spec.id).unwrap(),
        document_type: spec.doc_type,
        issue_date: DateOnly::new("2026-01-01").unwrap(),
        tax_point_date: None,
        due_date: spec.due_date.map(|d| DateOnly::new(d).unwrap()),
        document_number: DocumentNumber::new(spec.number).unwrap(),
        currency: Iso4217Code::new(spec.currency).unwrap(),
        supplier: argentine_party("Acme SRL", "20123456789", "Buenos Aires", "C"),
        customer: spec.customer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: spec.lines,
        tax_summary: spec.tax_summary,
        monetary_total: spec.total,
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

fn line(id: &str, desc: &str, qty: i64, unit_minor: i64, ext_minor: i64) -> DocumentLine {
    DocumentLine {
        id: id.to_owned(),
        description: desc.to_owned(),
        quantity: DecimalValue::new(Decimal::from(qty)),
        unit_code: Some("EA".to_owned()),
        unit_price: amt(unit_minor),
        line_extension_amount: amt(ext_minor),
        tax_category: Some("S".to_owned()),
        extensions: Vec::new(),
    }
}

/// Build the full request from explicit Argentina-specific axes (service +
/// letter), feeding the serialized bytes as the canonical payload.
fn cae_request_full(
    service: AfipService,
    letter: AfipLetter,
    cuit: &str,
    pv: &str,
    payload: Vec<u8>,
) -> AfipCaeRequest {
    AfipCaeRequest {
        tenant_id: TENANT.to_owned(),
        environment: AfipEnvironment::Homologacion,
        service,
        letter,
        issuer_cuit: cuit.to_owned(),
        punto_venta: pv.to_owned(),
        request_payload: payload,
    }
}

/// Pack {canonical.json, formats/ubl.xml, receipt.json} into a `.ikb`. Mirrors
/// `run_lifecycle`'s bundle step but takes an arbitrary doc + envelope so the
/// authority-rejection scenarios (which the offline mock cannot force) can be
/// driven through the same evidence path the happy path uses.
fn bundle_for(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &AfipCaeEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Step 4 success criterion shared across scenarios: a packed `.ikb` verifies
/// (`verify_packed(content_only).ok == true`, i.e. exit 0). `msg` is the
/// scenario-specific failure message so a regression still names the path.
fn assert_bundle_verifies(ikb: &[u8], msg: &str) {
    let report = verify_packed(ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "{msg}");
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
    assert_bundle_verifies(&ikb, "evidence bundle must verify");
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

/// Scenario: domestic **Nota de Crédito A** (corrective document).
///
/// AFIP comprobante type 3 = "Nota de Crédito A" in the WSFEv1
/// FEParamGetTiposCbte table (vs. type 1 = "Factura A"). In the UBL family the
/// IR `DocumentType::CreditNote` projects to `<cbc:CreditNoteTypeCode>381` and
/// MUST NOT carry a top-level `cbc:DueDate` (UBL 2.1 CreditNote has no such
/// element) — the inverse of an invoice's `<cbc:InvoiceTypeCode>380`. We assert
/// the credit-note projection, then clear the CAE through WSFE and bundle it.
///
/// Spec: AFIP WSFEv1 FEParamGetTiposCbte, "Manual para el desarrollador"
/// COMPG v3.3, https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
#[test]
fn argentina_nota_de_credito_a_corrective_document() {
    // Credit note for 100.00 net + 21% IVA = 121.00 reversed against an invoice.
    let doc = build_doc(DocSpec {
        id: "doc-ar-nc-1",
        number: "0001-00000002",
        doc_type: DocumentType::CreditNote,
        currency: "ARS",
        due_date: None, // UBL CreditNote forbids a top-level DueDate.
        customer: argentine_party("Beta SA", "27987654321", "Cordoba", "X"),
        lines: vec![line(
            "1",
            "Ajuste por nota de credito - servicios de consultoria",
            1,
            10000,
            10000,
        )],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100),
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12100),
        },
    });

    let ubl = to_xml(&doc).unwrap();
    // A credit note projects to UBL CreditNoteTypeCode 381, NOT InvoiceTypeCode.
    // (The canonicalizer inlines the cbc namespace decl as an attribute, so we
    // match the element by its open-tag prefix + the closing tag.)
    assert!(
        ubl.contains("<cbc:CreditNoteTypeCode")
            && ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "Nota de Crédito must serialize as UBL CreditNoteTypeCode 381"
    );
    assert!(
        !ubl.contains("InvoiceTypeCode"),
        "a credit note must not emit an InvoiceTypeCode"
    );
    assert!(
        !ubl.contains("DueDate"),
        "UBL 2.1 CreditNote carries no top-level cbc:DueDate"
    );

    let ubl_bytes = ubl.into_bytes();
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let env = provider
        .request_cae(&cae_request_full(
            AfipService::Wsfe,
            AfipLetter::A,
            ISSUER_CUIT,
            PUNTO_VENTA,
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(env.status, AfipStatus::Aprobado);
    assert_eq!(env.cae.len(), 14, "AFIP CAE is a 14-digit code");

    let ikb = bundle_for(&doc, &ubl_bytes, &env);
    assert_bundle_verifies(&ikb, "credit-note evidence bundle must verify");
}

/// Scenario: multi-line domestic **Factura A** at the 21% IVA aliquot.
///
/// Exercises a three-line breakdown that sums to a single 21% IVA aliquot
/// summary. AFIP IVA aliquot code 5 = 21% in the WSFEv1 FEParamGetTiposIva
/// table; 300.00 net * 21% = 63.00 IVA -> 363.00 gross. The three lines must
/// each appear in the UBL and the totals must balance.
///
/// Spec: AFIP WSFEv1 FEParamGetTiposIva (aliquot code 5 = 21%),
/// https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
#[test]
fn argentina_multiline_factura_a_21pct() {
    let doc = build_doc(DocSpec {
        id: "doc-ar-ml-1",
        number: "0001-00000003",
        doc_type: DocumentType::Invoice,
        currency: "ARS",
        due_date: Some("2026-01-31"),
        customer: argentine_party("Beta SA", "27987654321", "Cordoba", "X"),
        lines: vec![
            line("1", "Licencia software anual", 1, 15000, 15000),
            line("2", "Horas de soporte", 2, 5000, 10000),
            line("3", "Capacitacion in-company", 1, 5000, 5000),
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(30000), // 300.00 net
            tax_amount: amt(6300),      // 63.00 IVA @ 21%
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            tax_inclusive_amount: amt(36300), // 363.00 gross
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(36300),
        },
    });

    let ubl = to_xml(&doc).unwrap();
    assert!(
        ubl.contains("<cbc:InvoiceTypeCode") && ubl.contains(">380</cbc:InvoiceTypeCode>"),
        "Factura A projects to UBL InvoiceTypeCode 380"
    );
    // All three line descriptions survive into the national-family artifact.
    for needle in ["Licencia software anual", "Horas de soporte", "Capacitacion in-company"] {
        assert!(ubl.contains(needle), "multi-line UBL missing line {needle:?}");
    }
    // The 63.00 IVA total and 363.00 payable balance the three lines.
    assert!(ubl.contains("363.00"), "gross payable 363.00 must appear in UBL");

    let ubl_bytes = ubl.into_bytes();
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let env = provider
        .request_cae(&cae_request_full(
            AfipService::Wsfe,
            AfipLetter::A,
            ISSUER_CUIT,
            PUNTO_VENTA,
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(env.status, AfipStatus::Aprobado);

    let ikb = bundle_for(&doc, &ubl_bytes, &env);
    assert_bundle_verifies(&ikb, "multi-line invoice evidence bundle must verify");
}

/// Scenario: **Factura E (exportación)** routed through WSFEX, zero-rated IVA.
///
/// Exports are class "E" (AFIP comprobante type 19) and clear through the
/// dedicated WSFEX service, not WSFEv1. Exports are not subject to domestic IVA
/// (zero-rated / aliquot 0%, AFIP IVA code 3); the buyer is foreign and carries
/// no Argentine CUIT, so the invoice is billed in USD. We assert the request
/// carries the export-specific service + letter and that the document is
/// genuinely zero-tax.
///
/// Spec: AFIP WSFEX "Manual para el desarrollador" v2.1.0 (export class E),
/// https://afip.gob.ar/fe/documentos/WSFEX-Manualparaeldesarrollador_V2.1.0.pdf
#[test]
fn argentina_factura_e_export_wsfex_zero_rated() {
    let doc = build_doc(DocSpec {
        id: "doc-ar-exp-1",
        number: "0002-00000001",
        doc_type: DocumentType::Invoice,
        currency: "USD", // export billed in foreign currency
        due_date: Some("2026-02-28"),
        customer: foreign_party("Globex Inc", "San Francisco", "US"),
        lines: vec![line("1", "Software development services (export)", 1, 50000, 50000)],
        // Zero-rated export: taxable base present, zero IVA at 0% aliquot.
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(), // zero-rated
            taxable_amount: amt(50000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(50000), // gross == net: no IVA on exports
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(50000),
        },
    });

    let ubl = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();

    let req = cae_request_full(
        AfipService::Wsfex, // export service, not WSFE
        AfipLetter::E,      // letter E = exportación
        ISSUER_CUIT,
        "00002",
        ubl_bytes.clone(),
    );
    assert_eq!(req.service, AfipService::Wsfex, "exports route through WSFEX");
    assert_eq!(req.letter, AfipLetter::E, "export invoices are letter class E");

    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let env = provider.request_cae(&req).unwrap();
    assert_eq!(env.status, AfipStatus::Aprobado);
    assert_eq!(env.cae_expiry_yyyymmdd, "20260131");

    let ikb = bundle_for(&doc, &ubl_bytes, &env);
    assert_bundle_verifies(&ikb, "export (WSFEX) evidence bundle must verify");
}

/// Scenario: detailed **Factura A** with line breakdown via WSMTXCA at the
/// reduced 10.5% IVA aliquot.
///
/// WSMTXCA is the AFIP service for "factura con detalle de items" (per-line
/// breakdown), distinct from WSFE (sin detalle). The reduced aliquot 10.5%
/// (AFIP IVA code 4) applies to specific goods/services; 200.00 net * 10.5% =
/// 21.00 IVA -> 221.00 gross. We assert the service axis and the reduced-rate
/// totals.
///
/// Spec: AFIP WSMTXCA developer manual + WSFEv1 FEParamGetTiposIva (aliquot
/// code 4 = 10.5%),
/// https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
#[test]
fn argentina_wsmtxca_detailed_reduced_rate() {
    let doc = build_doc(DocSpec {
        id: "doc-ar-mtx-1",
        number: "0003-00000001",
        doc_type: DocumentType::Invoice,
        currency: "ARS",
        due_date: Some("2026-01-31"),
        customer: argentine_party("Beta SA", "27987654321", "Cordoba", "X"),
        lines: vec![
            line("1", "Servicio gravado al 10,5%", 1, 12000, 12000),
            line("2", "Insumos gravados al 10,5%", 1, 8000, 8000),
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(20000), // 200.00 net
            tax_amount: amt(2100),      // 21.00 IVA @ 10.5%
            tax_rate: Some(DecimalValue::new(Decimal::new(1050, 2))), // 10.50
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            tax_inclusive_amount: amt(22100), // 221.00 gross
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(22100),
        },
    });

    let ubl = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();
    assert!(
        ubl_bytes.windows(5).any(|w| w == b"10.50"),
        "reduced 10.50% aliquot must appear in the serialized totals"
    );

    let req = cae_request_full(
        AfipService::Wsmtxca, // detalle de items service
        AfipLetter::A,
        ISSUER_CUIT,
        PUNTO_VENTA,
        ubl_bytes.clone(),
    );
    assert_eq!(req.service, AfipService::Wsmtxca);

    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let env = provider.request_cae(&req).unwrap();
    assert_eq!(env.status, AfipStatus::Aprobado);

    let ikb = bundle_for(&doc, &ubl_bytes, &env);
    assert_bundle_verifies(&ikb, "WSMTXCA detailed invoice evidence bundle must verify");
}

/// Scenario: AFIP authority **RECHAZADO** (refusal) is a receipt STATUS, not an
/// `Err` — and the rejection still bundles and verifies.
///
/// This is the AFIP analogue of Italy's "NS (Notifica di Scarto) is a receipt
/// kind, not an Err" rule. The `MockAfipProvider` always grants a CAE for a
/// shape-valid request and exposes no forced-`Rechazado` knob, so we construct
/// the rejected envelope directly (mirroring the crate's own serde round-trip
/// test) — with NO CAE issued, a typed observación, and `Rechazado` status —
/// then drive it through the SAME evidence path the happy path uses. The audit
/// trail must persist the rejection and the bundle must still verify (exit 0).
///
/// Spec: AFIP WSFEv1 returns observaciones + a Resultado of "R" (Rechazado) in
/// FECAESolicitar; rejection is an authority verdict carried on the receipt,
/// https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
#[test]
fn argentina_rechazado_is_a_status_not_an_error_and_still_bundles() {
    // Build the same valid corrective document; the rejection is at the
    // authority layer, not a local shape failure.
    let doc = argentine_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // AFIP refused: no CAE granted, empty CAE, typed observación, Rechazado.
    let rejected = AfipCaeEnvelope {
        cae: String::new(),
        cae_expiry_yyyymmdd: String::new(),
        status: AfipStatus::Rechazado,
        authorized_at: FIXED_AUTHORIZED_AT.to_owned(),
        observaciones: Some("10016: El comprobante ya fue autorizado (CbteNro duplicado)".to_owned()),
    };
    assert_eq!(rejected.status, AfipStatus::Rechazado);
    assert!(
        rejected.cae.is_empty(),
        "a Rechazado verdict grants no CAE"
    );
    assert!(
        rejected.observaciones.is_some(),
        "AFIP rejection carries an observación"
    );

    // Rejection persists in the audit trail and the bundle still verifies.
    let ikb = bundle_for(&doc, &ubl_bytes, &rejected);
    assert_bundle_verifies(&ikb, "rejection-path evidence bundle must still verify (exit 0)");

    // The receipt round-trips through serde with its rejection intact.
    let raw = serde_json::to_vec(&rejected).unwrap();
    let parsed: AfipCaeEnvelope = serde_json::from_slice(&raw).unwrap();
    assert_eq!(parsed, rejected);
}

/// Scenario: AFIP **APROBADO_CON_OBSERVACIONES** — a CAE IS granted but the
/// authority attaches warnings the engine must surface.
///
/// AFIP's WSFEv1 can return Resultado "A" with non-fatal observaciones (the CAE
/// is valid for printing, but the issuer should heed the warning). We model a
/// granted-with-observación verdict, prove the CAE is real (14 digits) yet the
/// observación is preserved, and bundle + verify it.
///
/// Spec: AFIP WSFEv1 FECAESolicitar Resultado "A" + Observaciones array,
/// https://www.afip.gob.ar/fe/ayuda/documentos/manual_desarrollador_COMPG_v3_3.pdf
#[test]
fn argentina_aprobado_con_observaciones_grants_cae_with_warning() {
    let doc = argentine_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let warned = AfipCaeEnvelope {
        cae: "70000000000007".to_owned(),
        cae_expiry_yyyymmdd: "20260131".to_owned(),
        status: AfipStatus::AprobadoConObservaciones,
        authorized_at: FIXED_AUTHORIZED_AT.to_owned(),
        observaciones: Some(
            "10018: El campo FchVtoPago no corresponde para el tipo de comprobante".to_owned(),
        ),
    };
    assert_eq!(warned.status, AfipStatus::AprobadoConObservaciones);
    assert_eq!(warned.cae.len(), 14, "a granted CAE is still 14 digits");
    assert!(
        warned.observaciones.is_some(),
        "AprobadoConObservaciones surfaces the warning text"
    );

    let ikb = bundle_for(&doc, &ubl_bytes, &warned);
    assert_bundle_verifies(&ikb, "approved-with-observations evidence bundle must verify");
}

/// Scenario: invalid-identifier refusals grounded in the real CUIT structure.
///
/// The Argentine CUIT (Clave Única de Identificación Tributaria) is 11 digits:
/// a 2-digit type prefix + 8-digit DNI/company number + a 1-digit módulo-11
/// check digit (weights 5 4 3 2 7 6 5 4 3 2). This crate's adapter enforces the
/// 11-ASCII-digit shape at the pre-wire layer (the check-digit arithmetic is a
/// downstream rule); we assert the adapter accepts a structurally-shaped CUIT
/// and refuses every malformed shape as a typed `BadCuit`/`BadPuntoVenta` `Err`
/// — distinct from an authority `Rechazado`.
///
/// Worked example from AFIP CUIT docs: 20-17254359-7 has check digit 7
/// (sum 136, 136 mod 11 = 4, 11-4 = 7). That well-formed CUIT must be accepted.
///
/// Spec: AFIP CUIT structure / módulo-11 verifier, https://www.afip.gob.ar/
#[test]
fn argentina_cuit_and_punto_venta_shape_refusals() {
    use invoicekit_report_ar_afip::{validate_cuit, validate_punto_venta, AfipError};

    // A structurally valid, módulo-11-consistent CUIT is accepted.
    assert!(validate_cuit("20172543597").is_ok());

    // Too short / too long / non-digit are all refused with the typed error.
    for bad in ["2017254359", "201725435970", "2017254359X", "20-17254359-7", ""] {
        assert!(
            matches!(validate_cuit(bad), Err(AfipError::BadCuit(_))),
            "malformed CUIT {bad:?} must be refused as BadCuit"
        );
    }

    // Punto de venta must be exactly 5 ASCII digits.
    assert!(validate_punto_venta("00002").is_ok());
    for bad in ["0002", "000020", "0002A", ""] {
        assert!(
            matches!(validate_punto_venta(bad), Err(AfipError::BadPuntoVenta(_))),
            "malformed punto de venta {bad:?} must be refused as BadPuntoVenta"
        );
    }

    // The full request path surfaces the same refusal end-to-end with a real,
    // shape-valid payload (so the CUIT is the sole failing axis).
    let payload = to_xml(&argentine_invoice()).unwrap().into_bytes();
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let res = provider.request_cae(&cae_request_full(
        AfipService::Wsfe,
        AfipLetter::A,
        "20-17254359-7", // formatted, contains dashes -> not 11 digits
        PUNTO_VENTA,
        payload,
    ));
    assert!(
        matches!(res, Err(AfipError::BadCuit(_))),
        "a dash-formatted CUIT must be refused as BadCuit, got {res:?}"
    );
}

/// Scenario: serialization + clearance determinism across the credit-note and
/// export paths (not just the original happy-path invoice).
///
/// Re-running each lifecycle must produce byte-identical UBL and byte-identical
/// `.ikb`. This guards the canonical XML projection (C14N) and the deterministic
/// pack for the document-type and currency variants the scenarios above add.
#[test]
fn argentina_credit_note_and_export_paths_are_byte_deterministic() {
    // Credit note: same input -> identical UBL bytes.
    let nc = build_doc(DocSpec {
        id: "doc-ar-nc-det",
        number: "0001-00000099",
        doc_type: DocumentType::CreditNote,
        currency: "ARS",
        due_date: None,
        customer: argentine_party("Beta SA", "27987654321", "Cordoba", "X"),
        lines: vec![line("1", "Ajuste", 1, 10000, 10000)],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100),
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12100),
        },
    });
    let a = to_xml(&nc).unwrap();
    let b = to_xml(&nc).unwrap();
    assert_eq!(a, b, "credit-note UBL serialization must be byte-stable");

    // Export path: same input -> identical .ikb bundle bytes.
    let exp = build_doc(DocSpec {
        id: "doc-ar-exp-det",
        number: "0002-00000099",
        doc_type: DocumentType::Invoice,
        currency: "USD",
        due_date: None,
        customer: foreign_party("Globex Inc", "San Francisco", "US"),
        lines: vec![line("1", "Export services", 1, 50000, 50000)],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(50000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        total: MonetaryTotal {
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(50000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(50000),
        },
    });
    let ubl_bytes = to_xml(&exp).unwrap().into_bytes();
    let provider = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    // Two providers from the same fixed seed -> same first serial -> same CAE.
    let env1 = provider
        .request_cae(&cae_request_full(
            AfipService::Wsfex,
            AfipLetter::E,
            ISSUER_CUIT,
            "00002",
            ubl_bytes.clone(),
        ))
        .unwrap();
    let provider2 = MockAfipProvider::with_fixed_authorized_at(FIXED_AUTHORIZED_AT);
    let env2 = provider2
        .request_cae(&cae_request_full(
            AfipService::Wsfex,
            AfipLetter::E,
            ISSUER_CUIT,
            "00002",
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(env1, env2, "deterministic mock yields identical CAE envelope");

    let ikb1 = bundle_for(&exp, &ubl_bytes, &env1);
    let ikb2 = bundle_for(&exp, &ubl_bytes, &env2);
    assert_eq!(ikb1, ikb2, "export .ikb bundle must be byte-stable");
}
