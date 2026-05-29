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
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
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
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1300),
            tax_rate: Some(DecimalValue::new(Decimal::new(1300, 2))),
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
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// Build the 50-digit Clave Numérica. Real Hacienda layout: country (506) +
/// date (DDMMYY) + cédula (12, left-padded) + consecutivo (20) + situación (1)
/// + código de seguridad (8). Here we synthesize a shape-valid 50-digit key.
fn clave_numerica() -> String {
    let pais = "506";
    let fecha = "260526"; // DDMMYY
    let cedula12 = format!("{ISSUER_CEDULA:0>12}");
    let situacion = "1";
    let codigo_seguridad = "12345678";
    let clave = format!("{pais}{fecha}{cedula12}{CONSECUTIVO}{situacion}{codigo_seguridad}");
    assert_eq!(clave.len(), 50, "clave numérica must be 50 digits");
    assert!(clave.bytes().all(|b| b.is_ascii_digit()));
    clave
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
/// assemble + pack the evidence bundle.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_cr_hacienda::HaciendaSubmitEnvelope) {
    // 1. build
    let doc = costa_rican_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (EN 16931 / UBL family path).
    let ubl_xml: Vec<u8> = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();
    // structural sanity: the UBL spine carries the document we built.
    // The serializer canonicalizes (XML C14N 1.1): namespace declarations are
    // inlined onto every element, so match on the tag + value separately rather
    // than a literal `<cbc:Tag>value</cbc:Tag>` slice.
    let ubl_str = String::from_utf8(ubl_xml.clone()).unwrap();
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cbc:DocumentCurrencyCode",
        ">CRC</cbc:DocumentCurrencyCode>",
        ">FE-2026-CR-0001</cbc:ID>",
        ">CR</cbc:IdentificationCode>",
    ] {
        assert!(ubl_str.contains(needle), "UBL missing {needle}");
    }

    // 3. submit to Hacienda (offline mock) -> typed receipt.
    let provider = MockHaciendaProvider::with_fixed_received_at(PINNED_RECEIVED_AT);
    let envelope = provider.submit_comprobante(&submit_request(ubl_xml.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + Hacienda receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
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
