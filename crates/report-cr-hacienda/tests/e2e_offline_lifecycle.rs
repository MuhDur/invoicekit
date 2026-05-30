// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Costa Rica Hacienda offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Costa Rica and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a `CR` country code
//!    and CRC (Costa Rican colón) currency
//! 2. serialize -> UBL 2.1 XML bytes via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path)
//! 3. submit those bytes to the crate's existing `MockHaciendaProvider`
//!    and assert the authority receipt's Costa Rica-specific fields
//!    (Clave Numérica echo, `Aceptado` status, recorded timestamp)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock only refuses pre-wire (shape validation -> `Err`);
//!    Hacienda never forces a `Rechazado` envelope, so the rejection test
//!    drives the pre-wire refusal path (empty / malformed payload).
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would
//! mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_cr_hacienda::{
    HaciendaDocumentKind, HaciendaEnvironment, HaciendaProvider, HaciendaStatus,
    HaciendaSubmitRequest, MockHaciendaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_RECEIVED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_cr_e2e";
const TRACE: &str = "trace_cr_e2e";
const ISSUER_CEDULA: &str = "3101123456";
const CONSECUTIVO: &str = "00100001010000000001";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn costa_rican_party(name: &str, cedula: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "cedula".to_owned(),
            value: cedula.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Avenida Central 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("San José".to_owned()),
            postal_code: "10101".to_owned(),
            country: CountryCode::new("CR").unwrap(),
        },
        contact: None,
    }
}

fn costa_rican_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FE-2026-CR-0001").unwrap(),
        currency: Iso4217Code::new("CRC").unwrap(),
        supplier: costa_rican_party("Acme CR S.A.", ISSUER_CEDULA, "San José"),
        customer: costa_rican_party("Beta CR S.A.", "3102654321", "Heredia"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoría y desarrollo de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1300),
            tax_rate: Some(DecimalValue::new(Decimal::new(1300, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11300),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11300),
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

/// Build the 50-digit Clave Numérica for the canonical [`CONSECUTIVO`]. Real
/// Hacienda layout: country (506) + date (DDMMYY) + cédula (12, left-padded) +
/// consecutivo (20) + situación (1) + código de seguridad (8). Here we
/// synthesize a shape-valid 50-digit key.
fn clave_numerica() -> String {
    clave_numerica_with(CONSECUTIVO)
}

fn submit_request(comprobante_xml: Vec<u8>) -> HaciendaSubmitRequest {
    HaciendaSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: HaciendaEnvironment::Sandbox,
        kind: HaciendaDocumentKind::Factura,
        issuer_cedula: ISSUER_CEDULA.to_owned(),
        clave_numerica: clave_numerica(),
        consecutivo: CONSECUTIVO.to_owned(),
        comprobante_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit (Hacienda mock) ->
/// assemble + pack the evidence bundle. This is the canonical happy-path
/// Factura wiring; the submit/bundle/pack chain is shared with the
/// parameterized [`run_lifecycle_for`].
fn run_lifecycle() -> (
    Vec<u8>,
    invoicekit_report_cr_hacienda::HaciendaSubmitEnvelope,
) {
    // 1. build
    let doc = costa_rican_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (EN 16931 / UBL family path).
    // Structural sanity: the UBL spine carries the document we built.
    // The serializer canonicalizes (XML C14N 1.1): namespace declarations are
    // inlined onto every element, so match on the tag + value separately rather
    // than a literal `<cbc:Tag>value</cbc:Tag>` slice.
    let ubl_str =
        String::from_utf8(invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes()).unwrap();
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cbc:DocumentCurrencyCode",
        ">CRC</cbc:DocumentCurrencyCode>",
        ">FE-2026-CR-0001</cbc:ID>",
        ">CR</cbc:IdentificationCode>",
    ] {
        assert!(ubl_str.contains(needle), "UBL missing {needle}");
    }

    // 3-4. submit (offline mock) + assemble/pack the evidence bundle. Identical
    // to the parameterized path with the canonical Factura kind/consecutivo and
    // no forced verdict (default `Aceptado`).
    let (ikb, envelope, _) =
        run_lifecycle_for(&doc, HaciendaDocumentKind::Factura, CONSECUTIVO, None);
    (ikb, envelope)
}

#[test]
fn costa_rica_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: Hacienda accepted the comprobante.
    assert_eq!(envelope.status, HaciendaStatus::Aceptado);
    // The Clave Numérica is echoed verbatim (the country-specific 50-digit key).
    assert_eq!(envelope.clave_numerica, clave_numerica());
    assert_eq!(envelope.clave_numerica.len(), 50);
    assert_eq!(envelope.received_at, PINNED_RECEIVED_AT);
    assert!(envelope.mensaje.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn costa_rica_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn costa_rica_prewire_refusal_is_an_error_not_an_envelope() {
    // The MockHaciendaProvider has no "force a Rechazado envelope" knob: the
    // happy path is always `Aceptado`. Its only refusal path is pre-wire shape
    // validation, which returns `Err(HaciendaError::*)` BEFORE any receipt is
    // synthesized. That is the supported rejection behaviour, so we drive it.
    let provider = MockHaciendaProvider::with_fixed_received_at(PINNED_RECEIVED_AT);

    // Empty payload -> BadXml.
    let err = provider
        .submit_comprobante(&submit_request(Vec::new()))
        .unwrap_err();
    assert!(
        matches!(err, invoicekit_report_cr_hacienda::HaciendaError::BadXml(_)),
        "empty comprobante must be refused pre-wire, got {err:?}"
    );

    // Malformed cédula -> BadCedula (country-id shape refusal).
    let mut bad_cedula = submit_request(b"<Invoice/>".to_vec());
    bad_cedula.issuer_cedula = "NOPE".to_owned();
    let err = provider.submit_comprobante(&bad_cedula).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_cr_hacienda::HaciendaError::BadCedula(_)
        ),
        "malformed cédula must be refused pre-wire, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Country-specific deepening scenarios.
//
// Grounded in the Costa Rican regulator's published format:
//   - Ministerio de Hacienda / Dirección General de Tributación (DGT),
//     "Anexos y Estructuras" v4.4, Resolución DGT-R-0027-2024.
//     https://www.hacienda.go.cr/contenido/14160-factura-electronica
//   - tipoDocumento taxonomy (01 Factura, 03 Nota de Crédito, 09 Factura de
//     Exportación) and the IVA tariff catalog "CodigoTarifaIVA"
//     (08 = 13% general rate; 01 = 0% / exento) from the same Anexos.
//   - The 50-digit Clave Numérica layout (país 506 + fecha DDMMYY + cédula(12)
//     + consecutivo(20) + situación(1) + código de seguridad(8)) from the
//     "Estructura de la clave numérica" section of the Anexos.
// All fixtures below are hand-built / synthetic; no regulator file is vendored.
// ---------------------------------------------------------------------------

/// A multi-line, mixed-IVA-tariff Costa Rican export invoice (tipoDocumento 09,
/// Factura Electrónica de Exportación). Exercises the multi-line path plus a
/// taxed line (13% general IVA, `CodigoTarifaIVA` 08) alongside a zero-rated
/// export line (0%, treated as the IVA-exempt/tasa-cero "S"/"Z" categories).
fn costa_rican_export_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cr-e2e-exp-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FEE-2026-CR-0009").unwrap(),
        currency: Iso4217Code::new("CRC").unwrap(),
        supplier: costa_rican_party("Acme CR S.A.", ISSUER_CEDULA, "San José"),
        customer: costa_rican_party("Gamma CR S.A.", "3103789012", "Alajuela"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Line 1: domestic consultancy at the 13% general IVA rate.
            DocumentLine {
                id: "1".to_owned(),
                description: "Servicios de consultoría (gravado 13%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            // Line 2: exported goods, zero-rated (tasa cero / exportación).
            DocumentLine {
                id: "2".to_owned(),
                description: "Bienes para exportación (tasa cero)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(4)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(10000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(1300),
                tax_rate: Some(DecimalValue::new(Decimal::new(1300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            tax_inclusive_amount: amt(21300),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(21300),
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

/// A Costa Rican Nota de Crédito Electrónica (tipoDocumento 03) that corrects
/// the original Factura Electrónica. The DGT requires a corrective document to
/// carry the original document reference (`InformacionReferencia`); here it
/// rides the IR `references` vec and is serialized as a UBL `CreditNote`
/// (`CreditNoteTypeCode` 381 / `CreditNoteLine`).
fn costa_rican_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cr-e2e-nc-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote has no top-level cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("NC-2026-CR-0003").unwrap(),
        currency: Iso4217Code::new("CRC").unwrap(),
        supplier: costa_rican_party("Acme CR S.A.", ISSUER_CEDULA, "San José"),
        customer: costa_rican_party("Beta CR S.A.", "3102654321", "Heredia"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Anulación parcial de consultoría".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(650),
            tax_rate: Some(DecimalValue::new(Decimal::new(1300, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5650),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5650),
        },
        attachments: Vec::new(),
        // InformacionReferencia: this NC references the original FE.
        references: vec![DocumentReference {
            kind: "credit-note-original".to_owned(),
            id: "FE-2026-CR-0001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
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

/// Generic offline lifecycle for any CR comprobante: serialize -> submit ->
/// bundle -> verify, with a configurable Hacienda document kind and an optional
/// forced authority verdict. Returns the packed `.ikb`, the receipt envelope,
/// and the national UBL XML string for downstream assertions.
fn run_lifecycle_for(
    doc: &CommercialDocument,
    kind: HaciendaDocumentKind,
    consecutivo: &str,
    forced: Option<(HaciendaStatus, Option<String>)>,
) -> (
    Vec<u8>,
    invoicekit_report_cr_hacienda::HaciendaSubmitEnvelope,
    String,
) {
    let ubl_xml: Vec<u8> = invoicekit_format_ubl::to_xml(doc).unwrap().into_bytes();
    let ubl_str = String::from_utf8(ubl_xml.clone()).unwrap();

    let mut request = submit_request(ubl_xml.clone());
    request.kind = kind;
    consecutivo.clone_into(&mut request.consecutivo);
    // Keep the clave consistent with the consecutivo it embeds.
    request.clave_numerica = clave_numerica_with(consecutivo);

    let provider = match forced {
        Some((status, mensaje)) => MockHaciendaProvider::with_fixed_received_at(PINNED_RECEIVED_AT)
            .with_forced_status(status, mensaje),
        None => MockHaciendaProvider::with_fixed_received_at(PINNED_RECEIVED_AT),
    };
    let envelope = provider.submit_comprobante(&request).unwrap();

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
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    let ikb = pack(&bundle).unwrap();
    (ikb, envelope, ubl_str)
}

/// Build a 50-digit Clave Numérica around an arbitrary 20-digit consecutivo.
fn clave_numerica_with(consecutivo: &str) -> String {
    let pais = "506";
    let fecha = "260526"; // DDMMYY
    let cedula12 = format!("{ISSUER_CEDULA:0>12}");
    let situacion = "1";
    let codigo_seguridad = "12345678";
    let clave = format!("{pais}{fecha}{cedula12}{consecutivo}{situacion}{codigo_seguridad}");
    assert_eq!(clave.len(), 50, "clave numérica must be 50 digits");
    assert!(clave.bytes().all(|b| b.is_ascii_digit()));
    clave
}

#[test]
fn costa_rica_multiline_export_invoice_mixed_iva_tariffs() {
    let doc = costa_rican_export_invoice();
    // tipoDocumento 09 = Factura Electrónica de Exportación.
    assert_eq!(HaciendaDocumentKind::FacturaExportacion.code(), "09");

    let (ikb, envelope, ubl) = run_lifecycle_for(
        &doc,
        HaciendaDocumentKind::FacturaExportacion,
        "00100001090000000009",
        None,
    );

    // Two distinct invoice lines reach the national UBL artifact. The
    // serializer canonicalizes (XML C14N 1.1): the `xmlns:cac` declaration is
    // inlined onto every `cac:` element, so match the tag-open prefix.
    assert_eq!(ubl.matches("<cac:InvoiceLine ").count(), 2);
    // The 13%-taxed line and the zero-rated export line both serialize their
    // classified tax category code (S = standard-rated, Z = zero-rated).
    assert!(
        ubl.contains(">S</cbc:ID>"),
        "13% IVA category S must appear"
    );
    assert!(
        ubl.contains(">Z</cbc:ID>"),
        "zero-rated export category Z must appear"
    );
    // The taxed line carries a 13.00% Percent; the export line carries 0%.
    assert!(ubl.contains(">13.00</cbc:Percent>"), "13% general IVA rate");
    assert!(ubl.contains(">0</cbc:Percent>"), "zero-rated export rate");
    // Mixed-rate totals: 13% IVA on one 100.00 base only, the export base is 0%,
    // so 200.00 net + 13.00 tax = 213.00 gross.
    assert!(ubl.contains(">213.00</cbc:TaxInclusiveAmount>"));
    assert!(ubl.contains(">200.00</cbc:LineExtensionAmount>"));
    assert!(ubl.contains(">13.00</cbc:TaxAmount>"), "13% IVA = 13.00");
    assert!(
        ubl.contains(">0.00</cbc:TaxAmount>"),
        "zero-rated subtotal = 0.00"
    );

    assert_eq!(envelope.status, HaciendaStatus::Aceptado);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "export-invoice evidence bundle must verify");
}

#[test]
fn costa_rica_credit_note_corrects_original_factura() {
    let doc = costa_rican_credit_note();
    // tipoDocumento 03 = Nota de Crédito Electrónica.
    assert_eq!(HaciendaDocumentKind::NotaCredito.code(), "03");

    let (ikb, envelope, ubl) = run_lifecycle_for(
        &doc,
        HaciendaDocumentKind::NotaCredito,
        "00100001030000000003",
        None,
    );

    // The corrective document serializes as a UBL CreditNote, not an Invoice:
    // UBL code 381 (Credit note) and the CreditNoteLine container. C14N inlines
    // the namespace onto each element, so match the tag-open prefix.
    assert!(ubl.contains(
        "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\">"
    ));
    assert!(ubl.contains(">381</cbc:CreditNoteTypeCode>"));
    assert!(ubl.contains("<cac:CreditNoteLine "));
    assert!(ubl.contains("<cbc:CreditedQuantity "));
    // A CreditNote must NOT carry an Invoice spine.
    assert!(!ubl.contains("<cbc:InvoiceTypeCode"));
    assert!(!ubl.contains("<cac:InvoiceLine "));
    // The credited amount (half of the original 100.00 line) flows through.
    assert!(ubl.contains(">50.00</cbc:LineExtensionAmount>"));
    assert!(ubl.contains(">56.50</cbc:PayableAmount>"));
    // The original FE reference rides the canonical IR.
    assert_eq!(doc.references.len(), 1);
    assert_eq!(doc.references[0].id, "FE-2026-CR-0001");

    assert_eq!(envelope.status, HaciendaStatus::Aceptado);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

#[test]
fn costa_rica_credit_note_lifecycle_is_byte_deterministic() {
    let doc = costa_rican_credit_note();
    let (a, _, _) = run_lifecycle_for(
        &doc,
        HaciendaDocumentKind::NotaCredito,
        "00100001030000000003",
        None,
    );
    let (b, _, _) = run_lifecycle_for(
        &doc,
        HaciendaDocumentKind::NotaCredito,
        "00100001030000000003",
        None,
    );
    assert_eq!(a, b, "the credit-note lifecycle must be byte-stable");
}

#[test]
fn costa_rica_authority_rechazado_is_a_receipt_status_not_an_error() {
    // Hacienda's asynchronous `MensajeHacienda` can reject an otherwise
    // shape-valid comprobante (e.g. duplicate clave, signature mismatch). That
    // rejection is a `Rechazado` verdict carried inside the Ok envelope — the
    // engine must persist it in the audit trail, so the evidence bundle still
    // verifies. (DGT "Mensaje de respuesta de Hacienda", Anexos v4.4.)
    let doc = costa_rican_invoice();
    let (ikb, envelope, _) = run_lifecycle_for(
        &doc,
        HaciendaDocumentKind::Factura,
        CONSECUTIVO,
        Some((
            HaciendaStatus::Rechazado,
            Some("Clave numérica duplicada".to_owned()),
        )),
    );

    assert_eq!(envelope.status, HaciendaStatus::Rechazado);
    assert_eq!(
        envelope.mensaje.as_deref(),
        Some("Clave numérica duplicada")
    );
    // The clave is still echoed so the rejection correlates with the upload.
    assert_eq!(envelope.clave_numerica.len(), 50);

    // Rejection path still produces a verifiable evidence bundle.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn costa_rica_invalid_identifiers_are_refused_pre_wire() {
    let provider = MockHaciendaProvider::with_fixed_received_at(PINNED_RECEIVED_AT);
    let xml = b"<FacturaElectronica/>".to_vec();

    // 49-digit clave numérica (must be exactly 50) -> BadClave.
    let mut bad_clave = submit_request(xml.clone());
    bad_clave.clave_numerica = "5".repeat(49);
    let err = provider.submit_comprobante(&bad_clave).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_cr_hacienda::HaciendaError::BadClave(_)
        ),
        "49-digit clave must be refused, got {err:?}"
    );

    // Non-numeric clave of the right length -> BadClave (digits-only rule).
    let mut alpha_clave = submit_request(xml.clone());
    alpha_clave.clave_numerica = format!("{}A", "5".repeat(49));
    let err = provider.submit_comprobante(&alpha_clave).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_cr_hacienda::HaciendaError::BadClave(_)
        ),
        "non-numeric clave must be refused, got {err:?}"
    );

    // 19-digit consecutivo (must be exactly 20) -> BadConsecutivo.
    let mut bad_consec = submit_request(xml.clone());
    bad_consec.consecutivo = "0".repeat(19);
    let err = provider.submit_comprobante(&bad_consec).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_cr_hacienda::HaciendaError::BadConsecutivo(_)
        ),
        "19-digit consecutivo must be refused, got {err:?}"
    );

    // 8-digit cédula (below the physical-person minimum of 9) -> BadCedula.
    let mut short_cedula = submit_request(xml);
    short_cedula.issuer_cedula = "10101010".to_owned();
    let err = provider.submit_comprobante(&short_cedula).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_cr_hacienda::HaciendaError::BadCedula(_)
        ),
        "8-digit cédula must be refused, got {err:?}"
    );
}
