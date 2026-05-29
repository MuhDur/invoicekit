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
//! 7. refusal has two distinct surfaces. Pre-wire refusals (bad RUT / zero
//!    folio / empty payload) surface as `Err`, NOT as a receipt status — see
//!    `cl_lifecycle_refuses_malformed_input`. An authority-side
//!    `SiiStatus::Rechazado` verdict surfaces as a *receipt status* (`Ok`
//!    envelope), NOT an `Err` — the audit-trail contract — see
//!    `cl_authority_rechazado_is_a_receipt_not_an_error`.
//!
//! The deepened scenarios below exercise more of Chile's real DTE taxonomy:
//! - **Nota de Crédito (tipo 61)** corrective document, serialized as a UBL
//!   `CreditNote` (TypeCode 381) referencing the corrected factura.
//! - **Factura Exenta (tipo 34)** — a tax-exempt (No Afecta o Exenta) document
//!   with a zero IVA line, the IVA-exempt analogue of the affected tipo 33.
//! - **Multi-line affected Factura (tipo 33)** with two IVA-bearing lines.
//!
//! Citations (regulator + spec):
//! - Servicio de Impuestos Internos (SII), "Formato de Documentos Tributarios
//!   Electrónicos", DTE tipo codes 33/34/61 and the Aceptado/Rechazado verdict
//!   model: <https://www.sii.cl/factura_electronica/formato_dte.pdf> and the
//!   developer portal <https://www.sii.cl/servicios_online/1039-.html>.
//! - SII RUT (Rol Único Tributario) identifier shape `NNNNNNNN-DV` with a
//!   modulo-11 verifier digit (0-9 or K):
//!   <https://www.sii.cl/preguntas_frecuentes/catastro/001_012_0586.htm>.
//!
//! Fixtures are license-safe hand-built synthetic DTEs; no copyrighted
//! regulator sample files are vendored. Goldens are hand-rolled (no
//! `insta`/`pretty_assertions`, which would mutate `Cargo.lock`).

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, LocalizedString,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_cl_dte::{
    to_dte_xml, DteContext, DteKind, MockSiiProvider, SiiEnvironment, SiiError, SiiProvider,
    SiiStatus, SiiSubmitEnvelope, SiiSubmitRequest,
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
    // The default happy-path request is the kind-explicit constructor pinned to
    // a standard-rated Factura Electrónica (tipo 33) and the canonical FOLIO.
    submit_request_kind(dte_xml, DteKind::FacturaElectronica, FOLIO)
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

    // 5. evidence bundle: canonical doc + UBL XML + SII receipt. Reuses the
    //    same packing machinery as the deepened scenarios so the happy path and
    //    the credit-note / exempt paths assemble bundles byte-identically.
    let ikb = pack_bundle(&doc, &ubl_bytes, &envelope);
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

// ---------------------------------------------------------------------------
// Deepened, country-specific scenarios.
// ---------------------------------------------------------------------------

/// Submit-request helper carrying an explicit DTE kind (tipo code).
fn submit_request_kind(dte_xml: Vec<u8>, kind: DteKind, folio: u64) -> SiiSubmitRequest {
    SiiSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SiiEnvironment::Certification,
        kind,
        issuer_rut: ISSUER_RUT.to_owned(),
        folio,
        dte_xml,
    }
}

/// A Chilean **Nota de Crédito Electrónica (tipo 61)** correcting the factura
/// above. Modeled as a UBL `CreditNote` (TypeCode 381). UBL 2.1 `CreditNote`
/// carries no top-level `cbc:DueDate`, so `due_date` is `None`; the corrected
/// document is recorded in `references`.
///
/// SII tipo 61 (Nota de Crédito) per the SII DTE format spec:
/// <https://www.sii.cl/factura_electronica/formato_dte.pdf>.
fn chilean_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cl-e2e-nc-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote rejects a top-level DueDate; keep it None.
        due_date: None,
        document_number: DocumentNumber::new("DTE-61-0001").unwrap(),
        currency: Iso4217Code::new("CLP").unwrap(),
        supplier: chilean_party("Proveedor SpA", ISSUER_RUT, "Santiago", "RM"),
        customer: chilean_party("Cliente Limitada", "77654321-0", "Valparaíso", "VS"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Anulación parcial factura DTE-33-0001".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Half the original IVA line is reversed (CLP 50.00 net, 19% IVA).
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(950),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5950),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5950),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: vec![LocalizedString {
            language: "es".to_owned(),
            text: "Referencia: corrige DTE tipo 33 folio 4242".to_owned(),
        }],
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A Chilean **Factura No Afecta o Exenta (tipo 34)** — a tax-exempt document
/// (e.g. exports / exempt services). The single line is IVA-exempt: the tax
/// summary carries category `E` with a zero tax amount and a 0% rate, so the
/// payable amount equals the net (no 19% IVA added).
///
/// SII tipo 34 (Factura No Afecta o Exenta) per the SII DTE format spec:
/// <https://www.sii.cl/factura_electronica/formato_dte.pdf>.
fn chilean_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cl-e2e-exenta-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("DTE-34-0001").unwrap(),
        currency: Iso4217Code::new("CLP").unwrap(),
        supplier: chilean_party("Proveedor SpA", ISSUER_RUT, "Santiago", "RM"),
        customer: chilean_party("Cliente Limitada", "77654321-0", "Valparaíso", "VS"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicio exento de IVA (exportación)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            // Exempt category, not the standard-rated `S`.
            tax_category: Some("E".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        monetary_total: MonetaryTotal {
            // No IVA: tax-inclusive == tax-exclusive == net.
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(10000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(10000),
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

/// A two-line affected **Factura Electrónica (tipo 33)**: CLP 100.00 + 250.00
/// net = 350.00, 19% IVA = 66.50, payable 416.50.
fn chilean_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-cl-e2e-multiline-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("DTE-33-0002").unwrap(),
        currency: Iso4217Code::new("CLP").unwrap(),
        supplier: chilean_party("Proveedor SpA", ISSUER_RUT, "Santiago", "RM"),
        customer: chilean_party("Cliente Limitada", "77654321-0", "Valparaíso", "VS"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Servicios de consultoría".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(10000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Licencia de software anual".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(25000),
                line_extension_amount: amt(25000),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(35000),
            tax_amount: amt(6650),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(35000),
            tax_exclusive_amount: amt(35000),
            tax_inclusive_amount: amt(41650),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(41650),
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

/// Pack a freshly-built bundle from a document + a submit envelope and return
/// the `.ikb` bytes, so the credit-note / exempt scenarios reuse the same
/// evidence machinery as the happy path without weakening `run_lifecycle`.
fn pack_bundle(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &SiiSubmitEnvelope) -> Vec<u8> {
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

/// Pack a bundle whose national artifact is the **SII DTE** XML (not UBL). The
/// `formats/dte.xml` member carries Chile's real `DTE`/`Documento` tree, and the
/// receipt + canonical document round out the audit trail.
fn pack_bundle_dte(
    doc: &CommercialDocument,
    dte_bytes: &[u8],
    envelope: &SiiSubmitEnvelope,
) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/dte.xml".to_owned(), dte_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Native-format end-to-end lifecycle: serialize the IR document to the **real**
/// SII DTE (`Documento Tributario Electrónico`) XML via [`to_dte_xml`] (NOT UBL),
/// structurally validate the national spine, submit those bytes through the
/// existing `MockSiiProvider` (real RUT + folio validators), then bundle and
/// verify.
///
/// This mirrors Italy's `to_fattura_pa_xml` -> validate -> mock-transmit ->
/// evidence -> verify chain, but emits Chile's national tree with its actual
/// SII element names (`Encabezado`, `IdDoc`, `TipoDTE`, `Emisor`, `RUTEmisor`,
/// `Totales`, `Detalle`, ...), per the SII "Formato de Documentos Tributarios
/// Electrónicos": <https://www.sii.cl/factura_electronica/formato_dte.pdf>.
#[test]
fn cl_native_dte_lifecycle_produces_verifiable_evidence() {
    // 1. build
    let doc = chilean_invoice();

    // 2. serialize -> SII DTE national XML (the real Chilean format).
    let ctx = DteContext {
        folio: FOLIO,
        giro_emisor: "Servicios de consultoría e ingeniería".to_owned(),
    };
    let dte_xml = to_dte_xml(&doc, &ctx).unwrap();
    let dte_bytes = dte_xml.clone().into_bytes();

    // 3. validate (local, structural): the national artifact must carry the
    //    mandatory SII DTE spine with its actual element names — proving this is
    //    the real DTE tree, not UBL relabeled. (`amt(10000)` is CLP 100.00; the
    //    SII serializer renders pesos as integers, so 10000 minor -> "100".)
    for needle in [
        "<DTE xmlns=\"http://www.sii.cl/SiiDte\" version=\"1.0\">",
        "<Documento ID=\"T33F4242\">",
        "<Encabezado>",
        "<TipoDTE>33</TipoDTE>",
        "<Folio>4242</Folio>",
        "<RUTEmisor>76192083-9</RUTEmisor>",
        "<RUTRecep>77654321-0</RUTRecep>",
        "<Totales>",
        "<MntNeto>100</MntNeto>",
        "<TasaIVA>19</TasaIVA>",
        "<IVA>19</IVA>",
        "<MntTotal>119</MntTotal>",
        "<Detalle>",
        "<NroLinDet>1</NroLinDet>",
        "<MontoItem>100</MontoItem>",
    ] {
        assert!(dte_xml.contains(needle), "SII DTE missing {needle}");
    }
    // It must NOT be the UBL surface.
    assert!(
        !dte_xml.contains("cac:AccountingSupplierParty") && !dte_xml.contains("<Invoice"),
        "the native DTE artifact must not be UBL relabeled"
    );

    // 4. transmit via the existing mock (real RUT + folio validators).
    let provider = MockSiiProvider::new();
    let envelope = provider
        .submit_dte(&submit_request(dte_bytes.clone()))
        .unwrap();
    assert_eq!(envelope.status, SiiStatus::Recibido);
    assert!(envelope.track_id.starts_with("SII-"));

    // 5. evidence bundle (DTE XML as the national artifact) -> verify.
    let ikb = pack_bundle_dte(&doc, &dte_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native-DTE evidence bundle must verify");

    // 6. determinism: the whole native chain is byte-stable.
    let dte_xml_again = to_dte_xml(&doc, &ctx).unwrap();
    assert_eq!(dte_xml, dte_xml_again, "DTE serialization must be stable");
    let ikb_again = pack_bundle_dte(&doc, &dte_xml_again.into_bytes(), &envelope);
    assert_eq!(ikb, ikb_again, "the native-DTE lifecycle must be byte-stable");
}

/// Native-format Nota de Crédito (tipo 61): the IR credit note serializes to SII
/// DTE XML with `TipoDTE` 61 (NOT 33), submits under the credit-note tipo code,
/// and the bundle verifies. SII tipo 61 per the DTE format spec:
/// <https://www.sii.cl/factura_electronica/formato_dte.pdf>.
#[test]
fn cl_native_dte_credit_note_maps_to_tipo_61() {
    let doc = chilean_credit_note();
    let ctx = DteContext {
        folio: 7001,
        giro_emisor: "Servicios de consultoría".to_owned(),
    };
    let dte_xml = to_dte_xml(&doc, &ctx).unwrap();

    assert!(
        dte_xml.contains("<TipoDTE>61</TipoDTE>"),
        "a Nota de Crédito must serialize as SII TipoDTE 61, got:\n{dte_xml}"
    );
    assert!(
        !dte_xml.contains("<TipoDTE>33</TipoDTE>"),
        "a credit note must not carry the factura tipo 33"
    );
    assert!(dte_xml.contains("<Documento ID=\"T61F7001\">"));
    // Half the original IVA line reversed: CLP 50.00 net, 19% -> IVA 9.50 -> 10
    // pesos (rounded), MntTotal 59.50 -> 60. (amt() carries scale-2 minor units.)
    assert!(dte_xml.contains("<MntNeto>50</MntNeto>"));

    let provider = MockSiiProvider::new();
    let envelope = provider
        .submit_dte(&submit_request_kind(
            dte_xml.clone().into_bytes(),
            DteKind::NotaCredito,
            7001,
        ))
        .unwrap();
    assert_eq!(envelope.status, SiiStatus::Recibido);
    assert_eq!(DteKind::NotaCredito.code(), 61);

    let ikb = pack_bundle_dte(&doc, &dte_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "native-DTE credit-note bundle must verify");
}

/// Nota de Crédito (tipo 61): the corrective document serializes as a UBL
/// `CreditNote` (TypeCode 381 — NOT 380), submits with the credit-note tipo
/// code, and the whole chain still produces a verifiable evidence bundle.
#[test]
fn cl_credit_note_tipo_61_serializes_and_bundles() {
    let doc = chilean_credit_note();
    let ubl_xml = invoicekit_format_ubl::to_xml(&doc).unwrap();

    // UBL CreditNote spine: root <CreditNote>, the 381 type code, and the
    // credited-quantity element — none of which an Invoice (tipo 33) emits.
    assert!(ubl_xml.contains("<CreditNote"), "root must be CreditNote");
    assert!(
        ubl_xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "Nota de Crédito must carry UBL CreditNoteTypeCode 381"
    );
    assert!(
        ubl_xml.contains("<cbc:CreditedQuantity"),
        "CreditNote lines use CreditedQuantity, not InvoicedQuantity"
    );
    assert!(
        !ubl_xml.contains("InvoiceTypeCode"),
        "a CreditNote must not emit InvoiceTypeCode 380"
    );
    assert!(ubl_xml.contains(">CLP</cbc:DocumentCurrencyCode>"));

    let provider = MockSiiProvider::new();
    let envelope = provider
        .submit_dte(&submit_request_kind(
            ubl_xml.clone().into_bytes(),
            DteKind::NotaCredito,
            7001,
        ))
        .unwrap();
    assert_eq!(envelope.status, SiiStatus::Recibido);
    assert_eq!(DteKind::NotaCredito.code(), 61);

    let ikb = pack_bundle(&doc, &ubl_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// Factura Exenta (tipo 34): tax-exempt document carries a zero IVA amount and
/// the `E` (exempt) tax category, so tax-inclusive equals the net. The payload
/// submits under the exempt tipo code and bundles cleanly.
#[test]
fn cl_exempt_factura_tipo_34_has_zero_iva() {
    let doc = chilean_exempt_invoice();

    // The IR-level invariant that makes this a genuinely exempt document:
    // a single `E`-category summary with no IVA, and payable == net.
    let summary = &doc.tax_summary[0];
    assert_eq!(summary.category_code, "E");
    assert_eq!(summary.tax_amount, amt(0));
    assert_eq!(doc.monetary_total.payable_amount, amt(10000));
    assert_eq!(
        doc.monetary_total.tax_inclusive_amount,
        doc.monetary_total.tax_exclusive_amount,
        "an exempt factura adds no IVA: inclusive == exclusive"
    );

    let ubl_xml = invoicekit_format_ubl::to_xml(&doc).unwrap();
    // Standard-rated tipo 33 emits InvoiceTypeCode 380 too, so the
    // distinguishing exempt evidence is the zero tax in the totals block.
    assert!(ubl_xml.contains(">380</cbc:InvoiceTypeCode>"));
    assert!(ubl_xml.contains(">CLP</cbc:DocumentCurrencyCode>"));

    let provider = MockSiiProvider::new();
    let envelope = provider
        .submit_dte(&submit_request_kind(
            ubl_xml.clone().into_bytes(),
            DteKind::FacturaExenta,
            8001,
        ))
        .unwrap();
    assert_eq!(envelope.status, SiiStatus::Recibido);
    assert_eq!(DteKind::FacturaExenta.code(), 34);

    let ikb = pack_bundle(&doc, &ubl_xml.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "exempt-factura evidence bundle must verify");
}

/// Multi-line affected Factura (tipo 33): the UBL spine carries two
/// `cac:InvoiceLine` blocks and the 19% IVA totals foot to CLP 416.50.
#[test]
fn cl_multiline_factura_carries_two_lines() {
    let doc = chilean_multiline_invoice();
    assert_eq!(doc.lines.len(), 2);
    assert_eq!(doc.monetary_total.tax_inclusive_amount, amt(41650));

    let ubl_xml = invoicekit_format_ubl::to_xml(&doc).unwrap();
    let line_blocks = ubl_xml.matches("<cac:InvoiceLine").count();
    assert_eq!(
        line_blocks, 2,
        "a two-line factura must emit two cac:InvoiceLine blocks, got {line_blocks}"
    );
    // Both line nets must be present in the serialized form.
    assert!(ubl_xml.contains(">100.00<") && ubl_xml.contains(">250.00<"));

    let provider = MockSiiProvider::new();
    let envelope = provider
        .submit_dte(&submit_request_kind(
            ubl_xml.into_bytes(),
            DteKind::FacturaElectronica,
            9001,
        ))
        .unwrap();
    assert_eq!(envelope.status, SiiStatus::Recibido);
}

/// Authority REJECTION path: when the SII refuses the *content* of an
/// otherwise well-formed DTE, the verdict is `SiiStatus::Rechazado` carried
/// in an `Ok` receipt envelope (with a glosa) — NOT an `Err`. The rejection
/// is part of the audit trail and the evidence bundle still verifies.
///
/// This is the SII Aceptado/Rechazado verdict model:
/// <https://www.sii.cl/factura_electronica/formato_dte.pdf>.
#[test]
fn cl_authority_rechazado_is_a_receipt_not_an_error() {
    let doc = chilean_invoice();
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();

    let provider = MockSiiProvider::new().with_forced_status(SiiStatus::Rechazado);
    // submit_dte returns Ok(...) even though the authority rejected the DTE.
    let envelope = provider
        .submit_dte(&submit_request(ubl_bytes.clone()))
        .expect("authority rejection must NOT be an Err");
    assert_eq!(envelope.status, SiiStatus::Rechazado);
    assert!(
        envelope
            .glosa
            .as_deref()
            .is_some_and(|g| g.contains("RECHAZADO")),
        "a Rechazado verdict must carry a SII glosa, got {:?}",
        envelope.glosa
    );
    assert!(envelope.track_id.starts_with("SII-"));

    // The rejection persists in a verifiable evidence bundle.
    let ikb = pack_bundle(&doc, &ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

/// Invalid-identifier rejection: a RUT whose verifier-digit slot is malformed
/// (the `K` placed inside the numeric body rather than the check-digit slot)
/// fails the SII `NNNNNNNN-DV` shape and surfaces as `Err(BadRut)` BEFORE the
/// wire — never as a receipt.
///
/// SII RUT shape: <https://www.sii.cl/preguntas_frecuentes/catastro/001_012_0586.htm>.
#[test]
fn cl_rejects_rut_with_misplaced_verifier_digit() {
    let doc = chilean_invoice();
    let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();
    let provider = MockSiiProvider::new();

    // `K` is only legal in the single check-digit slot, not in the body.
    let mut bad = submit_request(ubl_bytes);
    bad.issuer_rut = "7619K083-9".to_owned();
    assert!(matches!(
        provider.submit_dte(&bad).unwrap_err(),
        SiiError::BadRut(_)
    ));
}

/// Determinism beyond the happy path: the credit-note lifecycle is also
/// byte-stable across repeated runs (same pinned timestamp + serial reset per
/// provider).
#[test]
fn cl_credit_note_lifecycle_is_byte_deterministic() {
    let build = || {
        let doc = chilean_credit_note();
        let ubl_bytes = invoicekit_format_ubl::to_xml(&doc).unwrap().into_bytes();
        let provider = MockSiiProvider::new();
        let envelope = provider
            .submit_dte(&submit_request_kind(
                ubl_bytes.clone(),
                DteKind::NotaCredito,
                7001,
            ))
            .unwrap();
        pack_bundle(&doc, &ubl_bytes, &envelope)
    };
    assert_eq!(build(), build(), "credit-note lifecycle must be byte-stable");
}
