// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Dominican Republic DGII e-CF — deepened offline coverage.
//!
//! This file *adds* genuinely country-specific scenarios on top of the basic
//! `e2e_offline_lifecycle.rs` honest-bar test; it does not weaken it. Each
//! scenario asserts real, Dominican-Republic-specific values grounded in the
//! DGII (Dirección General de Impuestos Internos) Facturación Electrónica
//! program.
//!
//! External authority references (all are the same sources pinned in
//! `data/country-manifests/dominican-republic.toml`):
//!
//! - DGII — Facturación Electrónica portal:
//!   <https://dgii.gov.do/cicloContribuyente/facturacion/facturacionElectronica/Paginas/default.aspx>
//! - Ley 32-23 sobre Facturación Electrónica (primary legislation):
//!   <https://www.dgii.gov.do/legislacion/leyesTributarias/Documents/2023/Ley%2032-23.pdf>
//! - Norma General 06-2018 — e-CF technical specification (defines the
//!   comprobante catálogo, the e-NCF and RNC shapes, the XAdES signature, and
//!   the BR-DO business-rule layer):
//!   <https://dgii.gov.do/legislacion/normasGenerales/Documents/2018/06-2018.pdf>
//!
//! Scenarios covered here:
//!
//! 1. Nota de Crédito Electrónica (catálogo type 34) — a corrective document
//!    that references the original Factura de Crédito Fiscal it amends; proves
//!    the CreditNote UBL spine + DGII type-34 submission.
//! 2. Multi-line Factura de Consumo (type 32, B2C) with the 18% ITBIS standard
//!    rate applied per line.
//! 3. Exportación Electrónica (type 46) — Dominican exports are ITBIS-exempt
//!    (0%, tax category `E`); proves the zero-rated path carries no tax amount.
//! 4. Authority `Rechazado` verdict — surfaced as a receipt status carrying a
//!    DGII `mensaje`, NOT an `Err` (the audit trail persists the rejection).
//! 5. Authority `AceptadoCondicional` and `EnProceso` verdicts — the two other
//!    real DGII async states, each round-tripping through serde with mensaje.
//! 6. Invalid-identifier rejections grounded in Norma 06-2018 shapes: a
//!    too-short RNC, a wrong e-NCF type-length, and a non-`E`-prefixed e-NCF.
//! 7. Receipt serialization determinism — the DGII envelope serializes to a
//!    byte-stable canonical JSON.
//!
//! The authority-verdict scenarios (4, 5) cannot be forced through the crate's
//! shipped `MockDgiiProvider` (it always returns `Aceptado` and exposes no
//! forced-receipt knob). They are driven instead by a small test-local provider
//! that implements the crate's public `DgiiProvider` trait and runs the *same*
//! pre-wire validators (`validate_rnc` / `validate_e_ncf`) a real DGII client
//! would, then emits the chosen authority verdict. This mirrors the Italy SDI
//! reference's `with_forced_receipt` pattern without modifying crate source.

// Bare type/identifier names (e-CF, e-NCF, RNC, TrackId, ITBIS) read fine in
// prose here; mirror the crate's own `src/lib.rs` allow.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_do_dgii::{
    validate_e_ncf, validate_rnc, DgiiDocumentKind, DgiiEnvironment, DgiiError, DgiiProvider,
    DgiiStatus, DgiiSubmitEnvelope, DgiiSubmitRequest, MockDgiiProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_do_scenarios";
const TRACE: &str = "trace_do_scenarios";
const ISSUER_RNC: &str = "131234567"; // 9-digit Dominican RNC
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

/// Submit one request through any provider and return the envelope, asserting
/// the bundle that wraps the receipt verifies. Returns the authority envelope.
fn submit_and_bundle(
    provider: &dyn DgiiProvider,
    doc: &CommercialDocument,
    ubl_xml: Vec<u8>,
    request: &DgiiSubmitRequest,
) -> DgiiSubmitEnvelope {
    let envelope = provider.submit_ecf(request).unwrap();
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_xml);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
    envelope
}

// ---------------------------------------------------------------------------
// Scenario 1: Nota de Crédito Electrónica (catálogo type 34, corrective).
// ---------------------------------------------------------------------------

/// A Nota de Crédito (DGII catálogo type 34) is a *corrective* e-CF: it amends
/// an already-issued Factura de Crédito Fiscal, carrying a reference to the
/// original document. Per Norma General 06-2018 the credit note is its own
/// comprobante type with its own e-NCF series (`E34…`).
///
/// Authority ref: Norma General 06-2018, catálogo de tipos de e-CF (33 Nota de
/// Débito, 34 Nota de Crédito).
/// <https://dgii.gov.do/legislacion/normasGenerales/Documents/2018/06-2018.pdf>
fn dominican_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-do-nc-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote has no top-level cbc:DueDate; omit it.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("NC-2026-DO-0001").unwrap(),
        currency: Iso4217Code::new("DOP").unwrap(),
        supplier: dominican_party("Empresa Dominicana SRL", ISSUER_RNC, "Santo Domingo"),
        customer: dominican_party("Cliente Caribe SAS", "401234567", "Santiago"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Devolución parcial — servicios de consultoría".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(900), // 18% ITBIS on 50.00 == 9.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5900),
        },
        attachments: Vec::new(),
        // The corrective link to the original Factura de Crédito Fiscal.
        references: vec![DocumentReference {
            kind: "corrected-ecf".to_owned(),
            id: "E310000000001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
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

#[test]
fn nota_credito_type_34_corrective_submits_and_bundles() {
    let doc = dominican_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // The CreditNote UBL spine — root element and the UBL credit-note type code
    // 381 — must be present, distinguishing it from a plain Invoice (which would
    // carry InvoiceTypeCode 380). The canonicalizer injects per-element xmlns
    // declarations onto open tags, so match the close tag / bounded value the
    // way `format-ubl`'s own corpus test does (`">381<"`).
    assert!(ubl.contains("<CreditNote"), "must serialize a UBL CreditNote root");
    assert!(
        ubl.contains("CreditNoteTypeCode") && ubl.contains(">381<"),
        "UBL CreditNote carries type code 381 (not the Invoice 380)"
    );
    assert!(!ubl.contains("InvoiceTypeCode"), "a CreditNote must not carry InvoiceTypeCode");
    assert!(ubl.contains(">DOP</cbc:DocumentCurrencyCode>"), "denominated in DOP");

    // DGII catálogo: a Nota de Crédito is comprobante type 34, e-NCF series E34…
    assert_eq!(DgiiDocumentKind::NotaCredito.code(), 34);
    let e_ncf = "E340000000001";
    assert!(e_ncf.starts_with("E34"), "Nota de Crédito e-NCF series is E34");

    let request = DgiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DgiiEnvironment::Sandbox,
        kind: DgiiDocumentKind::NotaCredito,
        issuer_rnc: ISSUER_RNC.to_owned(),
        e_ncf: e_ncf.to_owned(),
        ecf_xml: ubl.into_bytes(),
    };
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);
    let envelope = submit_and_bundle(&provider, &doc, request.ecf_xml.clone(), &request);

    assert_eq!(envelope.status, DgiiStatus::Aceptado);
    assert_eq!(
        envelope.e_ncf, "E340000000001",
        "DGII echoes the Nota de Crédito e-NCF"
    );
    // The corrective link survives into the canonical JSON the bundle carries.
    let canonical = String::from_utf8(
        canonicalize_value(&doc.to_value().unwrap())
            .unwrap()
            .into_bytes(),
    )
    .unwrap();
    assert!(
        canonical.contains("E310000000001"),
        "credit note must reference the original Factura de Crédito Fiscal"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: multi-line Factura de Consumo (type 32, B2C), 18% ITBIS.
// ---------------------------------------------------------------------------

/// A Factura de Consumo Electrónica (catálogo type 32) is the Dominican B2C
/// consumer invoice. This builds a genuine multi-line document and applies the
/// 18% ITBIS standard rate across the aggregate taxable base.
///
/// Authority ref: ITBIS standard rate 18% (Norma General; manifest validator
/// note "18% standard, 16% reduced for selected goods"). Catálogo type 32.
#[test]
fn factura_consumo_type_32_multiline_18pct_itbis() {
    let lines = vec![
        DocumentLine {
            id: "1".to_owned(),
            description: "Arroz selecto (saco)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(3)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(2000), // 20.00
            line_extension_amount: amt(6000), // 60.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        },
        DocumentLine {
            id: "2".to_owned(),
            description: "Aceite vegetal (litro)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(1500), // 15.00
            line_extension_amount: amt(3000), // 30.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        },
        DocumentLine {
            id: "3".to_owned(),
            description: "Detergente en polvo (kg)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(1000), // 10.00
            line_extension_amount: amt(1000), // 10.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        },
    ];
    // Base = 60 + 30 + 10 = 100.00; ITBIS 18% = 18.00; total = 118.00.
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-do-fc-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FC-2026-DO-0042").unwrap(),
        currency: Iso4217Code::new("DOP").unwrap(),
        supplier: dominican_party("Supermercado Nacional SRL", ISSUER_RNC, "Santo Domingo"),
        customer: dominican_party("Consumidor Final", "001000000", "Santo Domingo"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines,
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000), // 100.00
            tax_amount: amt(1800),      // 18.00
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
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap();

    let ubl = to_xml(&doc).unwrap();
    // All three consumer lines made it into the UBL artifact.
    let line_count = ubl.matches("<cac:InvoiceLine").count();
    assert_eq!(line_count, 3, "all three consumer lines must serialize");
    assert!(ubl.contains("Arroz selecto"), "first line present");
    assert!(ubl.contains("Detergente en polvo"), "third line present");

    assert_eq!(DgiiDocumentKind::FacturaConsumo.code(), 32);
    let request = DgiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DgiiEnvironment::Sandbox,
        kind: DgiiDocumentKind::FacturaConsumo,
        issuer_rnc: ISSUER_RNC.to_owned(),
        e_ncf: "E320000000042".to_owned(), // E32 series for Factura de Consumo
        ecf_xml: ubl.into_bytes(),
    };
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);
    let envelope = submit_and_bundle(&provider, &doc, request.ecf_xml.clone(), &request);
    assert_eq!(envelope.status, DgiiStatus::Aceptado);
    assert_eq!(envelope.e_ncf, "E320000000042");
}

// ---------------------------------------------------------------------------
// Scenario 3: Exportación Electrónica (type 46) — ITBIS-exempt (zero-rated).
// ---------------------------------------------------------------------------

/// An Exportación Electrónica (catálogo type 46) covers Dominican exports.
/// Exports are ITBIS-exempt (effectively zero-rated), so the e-CF carries no
/// ITBIS amount and the tax category is `E` (exempt) rather than `S`.
///
/// Authority ref: Norma General 06-2018 catálogo type 46 (Exportaciones);
/// ITBIS exemption for exports under the Código Tributario / Ley 32-23 regime.
#[test]
fn exportacion_type_46_is_itbis_exempt_zero_rated() {
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-do-exp-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-07-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("EXP-2026-DO-0007").unwrap(),
        // Exports are commonly invoiced in USD; DGII accepts foreign currency.
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: dominican_party("Exportadora del Caribe SRL", ISSUER_RNC, "Santo Domingo"),
        customer: Party {
            id: Some("buyer-us".to_owned()),
            name: "Caribbean Imports LLC".to_owned(),
            tax_ids: Vec::new(), // foreign buyer; no RNC
            address: PostalAddress {
                lines: vec!["100 Biscayne Blvd".to_owned()],
                city: "Miami".to_owned(),
                subdivision: Some("FL".to_owned()),
                postal_code: "33132".to_owned(),
                country: CountryCode::new("US").unwrap(),
            },
            contact: None,
        },
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cacao orgánico (export, FOB)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(100)),
            unit_code: Some("KGM".to_owned()),
            unit_price: amt(500),               // 5.00/kg
            line_extension_amount: amt(50000),  // 500.00
            tax_category: Some("E".to_owned()), // exempt / zero-rated
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(50000),
            tax_amount: amt(0), // ITBIS-exempt: no tax
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(50000), // == exclusive: no ITBIS added
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(50000),
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
    .unwrap();

    let ubl = to_xml(&doc).unwrap();
    assert!(ubl.contains(">USD</cbc:DocumentCurrencyCode>"), "export priced in USD");
    // Exempt category surfaces in the UBL tax classification as cbc:ID == "E"
    // (the canonicalizer injects xmlns onto open tags, so match the bounded
    // value `>E<`). It must NOT carry the standard-rate "S".
    assert!(ubl.contains(">E<"), "exempt tax category id `E` must appear in the UBL artifact");
    assert!(!ubl.contains(">S<"), "an export must not carry the standard-rate category `S`");
    // ITBIS-exempt: the tax total is zero (0.00), never the 18% an `S` line bears.
    assert!(ubl.contains(">0.00<"), "ITBIS-exempt export carries a 0.00 tax amount");
    assert!(!ubl.contains(">18<"), "an exempt export must not carry an 18% ITBIS rate");

    assert_eq!(DgiiDocumentKind::Exportaciones.code(), 46);
    let request = DgiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DgiiEnvironment::Sandbox,
        kind: DgiiDocumentKind::Exportaciones,
        issuer_rnc: ISSUER_RNC.to_owned(),
        e_ncf: "E460000000007".to_owned(), // E46 series for Exportaciones
        ecf_xml: ubl.into_bytes(),
    };
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);
    let envelope = submit_and_bundle(&provider, &doc, request.ecf_xml.clone(), &request);
    assert_eq!(envelope.status, DgiiStatus::Aceptado);
    assert_eq!(envelope.e_ncf, "E460000000007");
}

// ---------------------------------------------------------------------------
// A test-local provider that can emit any DGII authority verdict.
//
// The crate's shipped `MockDgiiProvider` only ever returns `Aceptado` and has
// no forced-receipt knob. To exercise the real `Rechazado` /
// `AceptadoCondicional` / `EnProceso` async states (and the `mensaje` field),
// we implement the crate's *public* `DgiiProvider` trait here in the test. It
// runs the same pre-wire validators a real DGII client runs, then surfaces the
// chosen verdict as a receipt status — NOT as an `Err`, matching the contract
// documented on `DgiiProvider::submit_ecf`.
// ---------------------------------------------------------------------------

struct ForcedVerdictProvider {
    status: DgiiStatus,
    mensaje: Option<String>,
}

impl DgiiProvider for ForcedVerdictProvider {
    fn submit_ecf(&self, request: &DgiiSubmitRequest) -> Result<DgiiSubmitEnvelope, DgiiError> {
        // Same pre-wire validation a real implementation performs.
        validate_rnc(&request.issuer_rnc)?;
        validate_e_ncf(&request.e_ncf)?;
        if request.ecf_xml.is_empty() {
            return Err(DgiiError::BadXml("payload is empty".to_owned()));
        }
        Ok(DgiiSubmitEnvelope {
            track_id: "DGII-000000000099".to_owned(),
            e_ncf: request.e_ncf.clone(),
            status: self.status,
            received_at: FIXED_RECEIVED_AT.to_owned(),
            mensaje: self.mensaje.clone(),
        })
    }
}

fn minimal_b2b_request() -> DgiiSubmitRequest {
    // A well-formed request so validation passes and the forced verdict is the
    // only thing under test.
    DgiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DgiiEnvironment::Produccion,
        kind: DgiiDocumentKind::FacturaCreditoFiscal,
        issuer_rnc: ISSUER_RNC.to_owned(),
        e_ncf: "E310000000123".to_owned(),
        ecf_xml: b"<ECF/>".to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Scenario 4: authority `Rechazado` — a receipt status, not an `Err`.
// ---------------------------------------------------------------------------

/// When DGII *rejects* a well-formed submission (e.g. a business-rule failure
/// such as an unauthorised e-NCF series or an RNC del receptor inválido), the
/// verdict comes back as `DgiiStatus::Rechazado` with a `mensaje`, carried
/// inside the envelope — it is NOT an `Err`. The engine persists the rejection
/// in the audit trail. This contract is documented on `DgiiProvider::submit_ecf`.
///
/// Authority ref: Norma General 06-2018 BR-DO business-rule layer (rejection
/// reasons surfaced via the recepción API verdict). DGII async states:
/// EnProceso / Aceptado / Rechazado / Aceptado Condicional.
#[test]
fn authority_rechazado_is_a_receipt_status_not_an_error() {
    let provider = ForcedVerdictProvider {
        status: DgiiStatus::Rechazado,
        mensaje: Some("RNC del receptor no registrado en DGII".to_owned()),
    };
    let result = provider.submit_ecf(&minimal_b2b_request());

    // Crucial: a DGII rejection is Ok(...) carrying Rechazado, never Err(...).
    let envelope = result.expect("DGII rejection must be Ok, surfaced as a status");
    assert_eq!(envelope.status, DgiiStatus::Rechazado);
    assert_eq!(
        envelope.mensaje.as_deref(),
        Some("RNC del receptor no registrado en DGII"),
        "Rechazado verdict must carry the DGII mensaje for the audit trail"
    );
    assert_eq!(envelope.e_ncf, "E310000000123", "DGII echoes the e-NCF even on rejection");

    // The rejection still produces a verifiable evidence bundle.
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must still verify");
}

// ---------------------------------------------------------------------------
// Scenario 5: `AceptadoCondicional` and `EnProceso` round-trip with mensaje.
// ---------------------------------------------------------------------------

/// DGII's recepción flow is asynchronous: a TrackId comes back immediately,
/// and the verdict transitions across `EnProceso` → terminal. Besides the two
/// terminal states (`Aceptado` / `Rechazado`), DGII can return
/// `AceptadoCondicional` (accepted with observations the engine must surface).
/// All four states must round-trip through serde so they persist in evidence.
///
/// Authority ref: DGII e-CF recepción API verdict states (Norma 06-2018).
#[test]
fn aceptado_condicional_and_en_proceso_round_trip_through_serde() {
    // AceptadoCondicional carries observations.
    let cond = ForcedVerdictProvider {
        status: DgiiStatus::AceptadoCondicional,
        mensaje: Some("Aceptado con observaciones: monto ITBIS difiere del calculado".to_owned()),
    }
    .submit_ecf(&minimal_b2b_request())
    .unwrap();
    assert_eq!(cond.status, DgiiStatus::AceptadoCondicional);
    assert!(cond.mensaje.is_some(), "conditional acceptance carries observations");

    let json = serde_json::to_string(&cond).unwrap();
    // serde rename_all = "kebab-case" maps the variant to "aceptado-condicional".
    assert!(
        json.contains("\"aceptado-condicional\""),
        "AceptadoCondicional serializes kebab-case, got {json}"
    );
    let parsed: DgiiSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, cond, "AceptadoCondicional must round-trip through serde");

    // EnProceso is the async-pending state, returned before a terminal verdict.
    let pending = ForcedVerdictProvider {
        status: DgiiStatus::EnProceso,
        mensaje: None,
    }
    .submit_ecf(&minimal_b2b_request())
    .unwrap();
    assert_eq!(pending.status, DgiiStatus::EnProceso);
    let pending_json = serde_json::to_string(&pending).unwrap();
    assert!(
        pending_json.contains("\"en-proceso\""),
        "EnProceso serializes kebab-case, got {pending_json}"
    );
    // No mensaje => the field is skipped entirely (serde skip_serializing_if).
    assert!(
        !pending_json.contains("mensaje"),
        "an absent mensaje must be omitted from the wire JSON, got {pending_json}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 6: invalid-identifier rejections grounded in Norma 06-2018 shapes.
// ---------------------------------------------------------------------------

/// The RNC (Registro Nacional del Contribuyente) is 9 or 11 ASCII digits and
/// the e-NCF is `E` + a 2-digit comprobante type + a 10-digit sequential
/// (13 chars total). Malformed identifiers are refused *before* the wire as a
/// typed `Err`, distinct from an authority `Rechazado`.
///
/// Authority ref: Norma General 06-2018 — e-NCF structure (`E` + tipo + NCF
/// secuencial) and RNC format. <https://dgii.gov.do/legislacion/normasGenerales/Documents/2018/06-2018.pdf>
#[test]
fn malformed_dominican_identifiers_are_refused_before_the_wire() {
    let provider = MockDgiiProvider::with_fixed_received_at(FIXED_RECEIVED_AT);

    // (a) RNC too short (8 digits) — must be 9 or 11.
    let mut short_rnc = minimal_b2b_request();
    short_rnc.issuer_rnc = "13123456".to_owned();
    assert!(
        matches!(provider.submit_ecf(&short_rnc), Err(DgiiError::BadRnc(_))),
        "an 8-digit RNC must be refused (RNC is 9 or 11 digits)"
    );

    // (b) RNC of the right length but containing a non-digit.
    let mut alpha_rnc = minimal_b2b_request();
    alpha_rnc.issuer_rnc = "13123456X".to_owned();
    assert!(matches!(provider.submit_ecf(&alpha_rnc), Err(DgiiError::BadRnc(_))));

    // (c) e-NCF missing the mandatory `E` prefix.
    let mut no_e_prefix = minimal_b2b_request();
    no_e_prefix.e_ncf = "3100000000012".to_owned(); // 13 chars but starts with '3'
    assert!(
        matches!(provider.submit_ecf(&no_e_prefix), Err(DgiiError::BadENcf(_))),
        "an e-NCF without the leading `E` must be refused"
    );

    // (d) e-NCF of the wrong length (12 chars instead of 13).
    let mut wrong_len = minimal_b2b_request();
    wrong_len.e_ncf = "E31000000001".to_owned(); // only 11 digits after E
    assert!(
        matches!(provider.submit_ecf(&wrong_len), Err(DgiiError::BadENcf(_))),
        "an e-NCF that is not E + 12 digits must be refused"
    );

    // (e) the free `validate_*` helpers agree with the provider's pre-wire path.
    assert!(validate_rnc("13123456").is_err());
    assert!(validate_rnc("131234567").is_ok()); // 9-digit valid
    assert!(validate_rnc("13123456789").is_ok()); // 11-digit valid
    assert!(validate_e_ncf("3100000000012").is_err());
    assert!(validate_e_ncf("E460000000007").is_ok()); // E46 export series valid
}

// ---------------------------------------------------------------------------
// Scenario 7: receipt serialization determinism.
// ---------------------------------------------------------------------------

/// The DGII receipt envelope must serialize to byte-stable JSON so the evidence
/// bundle is reproducible. Two independent submissions of the same well-formed
/// request through a fixed-timestamp provider must yield byte-identical
/// receipt JSON (the only varying field, the serial TrackId, is held fixed by
/// the forced-verdict provider here).
#[test]
fn dgii_receipt_serialization_is_byte_deterministic() {
    let provider = ForcedVerdictProvider {
        status: DgiiStatus::Aceptado,
        mensaje: None,
    };
    let a = serde_json::to_vec(&provider.submit_ecf(&minimal_b2b_request()).unwrap()).unwrap();
    let b = serde_json::to_vec(&provider.submit_ecf(&minimal_b2b_request()).unwrap()).unwrap();
    assert_eq!(a, b, "the DGII receipt JSON must be byte-stable");

    // And the canonicalizer agrees: canonical bytes are stable too.
    let env = provider.submit_ecf(&minimal_b2b_request()).unwrap();
    let canon_a = canonicalize_value(&serde_json::to_value(&env).unwrap()).unwrap();
    let canon_b = canonicalize_value(&serde_json::to_value(&env).unwrap()).unwrap();
    assert_eq!(canon_a, canon_b, "canonical receipt bytes must be byte-stable");
}
