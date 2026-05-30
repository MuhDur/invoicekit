// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Mexico CFDI 4.0 offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Mexico and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national CFDI 4.0 XML (`cfdi:Comprobante`)
//! 3. local validate (structural + Mexican identity shapes: RFC, Folio Fiscal)
//! 4. seal + timbrar via the offline `MockCfdiReportProvider` (composes
//!    `invoicekit-signer-cfdi`, the PAC timbrado path)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. rejection path: a PAC *rechazo* is a receipt status, not an `Err`
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! This mirrors the Italy SDI reference E2E. Goldens are hand-rolled (no
//! `insta`/`pretty_assertions`, which would mutate `Cargo.lock`), and the
//! capability matrix is populated centrally — this test does not assert it.

use std::collections::BTreeMap;
use std::sync::Arc;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_mx_cfdi::{
    report_request_for, to_cfdi_xml, validate_folio_fiscal, validate_rfc, CfdiComprobanteKind,
    CfdiContext, CfdiEnvironment, CfdiReport, CfdiReportError, CfdiReportProvider,
    CfdiReportRequest, MockCfdiReportProvider, TimbradoStatus,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_cfdi::CertificadoSelloDigital;
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_mx_e2e";
const TRACE: &str = "trace_mx_e2e";
const ISSUER_RFC: &str = "CAZ010101AAA";
const CSD_SERIAL: &str = "30001000000400002434";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn mexican_party(name: &str, rfc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "rfc".to_owned(),
            value: rfc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Avenida Reforma 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("CMX".to_owned()),
            postal_code: "06600".to_owned(),
            country: CountryCode::new("MX").unwrap(),
        },
        contact: None,
    }
}

fn mexican_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-mx-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("FAC-2026-MX-0001").unwrap(),
        currency: Iso4217Code::new("MXN").unwrap(),
        supplier: mexican_party("Comercializadora Azteca SA de CV", ISSUER_RFC, "Ciudad de Mexico"),
        customer: mexican_party("Distribuidora Maya SA de CV", "DMA020202BBB", "Monterrey"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoria & desarrollo".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("E48".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(16000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(116_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(116_000),
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

fn csd() -> CertificadoSelloDigital {
    CertificadoSelloDigital {
        serial_number: CSD_SERIAL.to_owned(),
        rfc: ISSUER_RFC.to_owned(),
        not_before: "2026-01-01T00:00:00Z".to_owned(),
        not_after: "2027-12-31T23:59:59Z".to_owned(),
        certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
    }
}

fn provider(reject: bool) -> MockCfdiReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CSD_SERIAL, [4_u8; 32]));
    let p = MockCfdiReportProvider::new(signer);
    if reject {
        p.with_rejection()
    } else {
        p
    }
}

fn report_request(cfdi_xml: Vec<u8>) -> CfdiReportRequest {
    CfdiReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: CfdiEnvironment::Sandbox,
        issuer_rfc: ISSUER_RFC.to_owned(),
        kind: CfdiComprobanteKind::Ingreso,
        csd: csd(),
        cfdi_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> seal/timbrar -> evidence bundle.
fn run_lifecycle(reject: bool) -> (Vec<u8>, CfdiReport) {
    // 1. build
    let doc = mexican_invoice();

    // 2. serialize -> CFDI 4.0
    let ctx = CfdiContext {
        lugar_expedicion: "06600".to_owned(),
        ..CfdiContext::default()
    };
    let cfdi = to_cfdi_xml(&doc, &ctx).unwrap();

    // 3. local validate (structural + Mexican identity shapes). Reference XSD +
    // cadena-original XSLT validation stays external (JVM).
    validate_rfc(ISSUER_RFC).unwrap();
    for needle in [
        "<cfdi:Comprobante",
        "Version=\"4.0\"",
        "TipoDeComprobante=\"I\"",
        "<cfdi:Emisor",
        "<cfdi:Receptor",
        "<cfdi:Conceptos>",
        "<cfdi:Impuestos",
        "Impuesto=\"002\"",
    ] {
        assert!(cfdi.contains(needle), "CFDI missing {needle}");
    }

    // 4. seal + timbrar (offline mock composing the real PAC signer path)
    let report = provider(reject)
        .report(&report_request(cfdi.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national XML + stamped XML + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/cfdi.xml".to_owned(), cfdi.into_bytes());
    artefacts.insert("timbrado.xml".to_owned(), report.timbrado_xml.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, report)
}

#[test]
fn mexico_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(false);

    // Happy path: the PAC stamped the CFDI (timbrado).
    assert!(report.envelope.status.is_stamped());
    assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
    validate_folio_fiscal(&report.envelope.folio_fiscal).unwrap();
    assert!(report.envelope.reason.is_none());
    assert!(report.envelope.sello_sat.is_some());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn mexico_rejection_still_bundles_and_verifies() {
    // A PAC rechazo is a receipt status, NOT an Err — the audit trail persists
    // the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(true);
    assert_eq!(report.envelope.status, TimbradoStatus::Rechazado);
    assert!(!report.envelope.status.is_stamped());
    assert!(report.envelope.reason.is_some());
    assert!(report.envelope.folio_fiscal.is_empty());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn mexico_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(false);
    let (b, _) = run_lifecycle(false);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

// ---------------------------------------------------------------------------
// Country-specific CFDI 4.0 format variations.
//
// Grounded in the SAT (Servicio de Administración Tributaria) Anexo 20 —
// "Guía de llenado de los Comprobantes Fiscales Digitales por Internet" — the
// normative CFDI 4.0 fill-in guide and its catalogues (c_TipoDeComprobante,
// c_Impuesto, c_TasaOCuota, c_ObjetoImp, c_TipoRelacion, RFC genéricos).
// Reference: SAT, "Comprobante Fiscal Digital por Internet (CFDI) versión 4.0",
// <https://www.sat.gob.mx/consultas/35025/formato-de-factura-(anexo-20)>.
// Fixtures are hand-built synthetic data; no copyrighted SAT XML is vendored.
// ---------------------------------------------------------------------------

/// Build a CFDI 4.0 nota de crédito (Egreso). Per Anexo 20 a credit note carries
/// `TipoDeComprobante="E"` and a `CfdiRelacionados` block whose `TipoRelacion`
/// `01` ("Nota de crédito de los documentos relacionados") points at the UUID of
/// the Ingreso it corrects. The IR `references` vector carries that relation.
fn mexican_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-mx-e2e-credit").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // A credit note carries no payment due-date of its own.
        due_date: None,
        document_number: DocumentNumber::new("NC-2026-MX-0007").unwrap(),
        currency: Iso4217Code::new("MXN").unwrap(),
        supplier: mexican_party("Comercializadora Azteca SA de CV", ISSUER_RFC, "Ciudad de Mexico"),
        customer: mexican_party("Distribuidora Maya SA de CV", "DMA020202BBB", "Monterrey"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Devolucion parcial servicios de consultoria".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("E48".to_owned()),
            unit_price: amt(20000),
            line_extension_amount: amt(20000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(20000),
            tax_amount: amt(3200),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            tax_inclusive_amount: amt(23200),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(23200),
        },
        attachments: Vec::new(),
        // TipoRelacion 01: this Egreso relates to the original Ingreso's Folio
        // Fiscal (UUID). The relation lives in the IR references vector.
        references: vec![DocumentReference {
            kind: "cfdi-relacion-01".to_owned(),
            id: "11111111-2222-4333-8444-555555555555".to_owned(),
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

/// Build a multi-line CFDI 4.0 Ingreso mixing two real SAT IVA rates: the 16%
/// general rate and the 8% IVA región fronteriza (border-region stimulus,
/// Decreto de estímulos fiscales región fronteriza). Each rate is a distinct
/// `TipoFactor="Tasa"` traslado; the document-level `TotalImpuestosTrasladados`
/// is their sum.
fn mexican_border_region_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-mx-e2e-multiline").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("FAC-2026-MX-0042").unwrap(),
        currency: Iso4217Code::new("MXN").unwrap(),
        supplier: mexican_party("Maquiladora Frontera SA de CV", ISSUER_RFC, "Tijuana"),
        customer: mexican_party("Distribuidora Maya SA de CV", "DMA020202BBB", "Monterrey"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Line 1: general 16% IVA. tax_category "S" (standard).
            DocumentLine {
                id: "1".to_owned(),
                description: "Equipo de computo".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("H87".to_owned()),
                unit_price: amt(100_000),
                line_extension_amount: amt(100_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            // Line 2: border-region 8% IVA. tax_category "AA" (reduced rate).
            DocumentLine {
                id: "2".to_owned(),
                description: "Servicio de instalacion (region fronteriza)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("E48".to_owned()),
                unit_price: amt(50000),
                line_extension_amount: amt(50000),
                tax_category: Some("AA".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(16000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
            },
            TaxCategorySummary {
                category_code: "AA".to_owned(),
                taxable_amount: amt(50000),
                tax_amount: amt(4000),
                tax_rate: Some(DecimalValue::new(Decimal::new(800, 2))),
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(150_000),
            tax_exclusive_amount: amt(150_000),
            // 1500.00 base + 160.00 + 40.00 IVA = 1700.00.
            tax_inclusive_amount: amt(170_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(170_000),
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

/// Build a CFDI 4.0 Ingreso to "público en general" at the 0% IVA rate.
/// Per Anexo 20, when the receptor is the general public the issuer uses the RFC
/// genérico `XAXX010101000` (`c_RFC`). The line is taxed (`ObjetoImp="02"`) but at
/// `TasaOCuota="0.000000"` — the SAT zero rate (`c_TasaOCuota`) for exports /
/// basic foods, distinct from an exempt (`ObjetoImp="01"`) concept.
fn mexican_zero_rate_publico_general() -> CommercialDocument {
    // Público en general: the receptor carries the RFC genérico, no tax id of
    // its own beyond that placeholder. We model it by giving the customer the
    // generic RFC directly.
    let mut customer = mexican_party("Publico en General", "XAXX010101000", "Tijuana");
    customer.id = Some("publico-general".to_owned());
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-mx-e2e-zerorate").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        document_number: DocumentNumber::new("FAC-2026-MX-0099").unwrap(),
        currency: Iso4217Code::new("MXN").unwrap(),
        supplier: mexican_party("Exportadora del Pacifico SA de CV", ISSUER_RFC, "Ensenada"),
        customer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Productos basicos para exportacion".to_owned(),
            quantity: DecimalValue::new(Decimal::from(10)),
            unit_code: Some("H87".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(100_000),
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            // 0% IVA: total equals the base.
            tax_inclusive_amount: amt(100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(100_000),
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

fn border_ctx() -> CfdiContext {
    CfdiContext {
        lugar_expedicion: "22000".to_owned(), // Tijuana, Baja California
        ..CfdiContext::default()
    }
}

/// Credit-note lifecycle: a CFDI Egreso (`TipoDeComprobante="E"`) serializes,
/// stamps as `CfdiKind::Egreso`, and the receipt carries a valid Folio Fiscal.
/// SAT Anexo 20 §`TipoDeComprobante` — E = Egreso (nota de crédito).
#[test]
fn mexico_credit_note_serializes_as_egreso_and_stamps() {
    let doc = mexican_credit_note();
    let xml = to_cfdi_xml(&doc, &border_ctx()).unwrap();

    // The comprobante must declare the Egreso type — not the Ingreso "I".
    assert!(
        xml.contains("TipoDeComprobante=\"E\""),
        "credit note must serialize as Egreso (E):\n{xml}"
    );
    assert!(!xml.contains("TipoDeComprobante=\"I\""));
    assert!(xml.contains("Folio=\"NC-2026-MX-0007\""));
    // 16% IVA on the 200.00 returned base = 32.00 traslado.
    assert!(xml.contains("TotalImpuestosTrasladados=\"32.00\""));

    // report_request_for derives the comprobante kind from the document type:
    // a CreditNote must become CfdiKind::Egreso, never Ingreso.
    let req = report_request_for(
        &doc,
        ISSUER_RFC,
        CfdiEnvironment::Sandbox,
        csd(),
        xml.into_bytes(),
    )
    .unwrap();
    assert_eq!(req.kind, CfdiComprobanteKind::Egreso);

    let report = provider(false).report(&req).unwrap();
    assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
    validate_folio_fiscal(&report.envelope.folio_fiscal).unwrap();
    let stamped = String::from_utf8(report.timbrado_xml).unwrap();
    assert!(stamped.contains("<tfd:TimbreFiscalDigital"));
}

/// Multi-line invoice mixing the 16% general IVA and the 8% IVA región
/// fronteriza. Both per-concepto traslados and the document-level total appear.
/// SAT Anexo 20 §"Impuestos" + the Decreto de estímulos fiscales región
/// fronteriza norte (8% IVA). `TasaOCuota` is the rate as a 6-decimal fraction.
#[test]
fn mexico_multiline_mixed_iva_rates() {
    let doc = mexican_border_region_invoice();
    let xml = to_cfdi_xml(&doc, &border_ctx()).unwrap();

    // Two conceptos serialize (match the open tag with its trailing space so the
    // `<cfdi:Conceptos>` container is not counted).
    assert_eq!(
        xml.matches("<cfdi:Concepto ").count(),
        2,
        "both invoice lines must serialize as conceptos:\n{xml}"
    );
    // Both SAT IVA fractions are present: 16% -> 0.160000, 8% -> 0.080000.
    assert!(xml.contains("TasaOCuota=\"0.160000\""), "missing 16% IVA:\n{xml}");
    assert!(xml.contains("TasaOCuota=\"0.080000\""), "missing 8% border IVA:\n{xml}");
    // Document-level total traslados: 160.00 + 40.00 = 200.00.
    assert!(
        xml.contains("TotalImpuestosTrasladados=\"200.00\""),
        "document-level total IVA must sum both rates:\n{xml}"
    );
    // Both per-concepto traslado importes appear.
    assert!(xml.contains("Importe=\"160.00\""));
    assert!(xml.contains("Importe=\"40.00\""));

    // Full lifecycle still stamps.
    let report = provider(false)
        .report(&report_request(xml.into_bytes()))
        .unwrap();
    assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
}

/// Zero-rated CFDI to público en general. Asserts the SAT RFC genérico
/// `XAXX010101000` on the Receptor and the 0% IVA fraction `0.000000`.
/// SAT Anexo 20: RFC genérico for público en general; `c_TasaOCuota` zero rate.
#[test]
fn mexico_zero_rate_publico_general() {
    let doc = mexican_zero_rate_publico_general();
    let ctx = CfdiContext {
        lugar_expedicion: "22800".to_owned(), // Ensenada, Baja California
        ..CfdiContext::default()
    };
    let xml = to_cfdi_xml(&doc, &ctx).unwrap();

    // The receptor carries the SAT RFC genérico for público en general.
    assert!(
        xml.contains("<cfdi:Receptor") && xml.contains("Rfc=\"XAXX010101000\""),
        "público-en-general receptor must use the RFC genérico XAXX010101000:\n{xml}"
    );
    // The zero rate is the SAT 0% fraction, not an absent traslado.
    assert!(xml.contains("TasaOCuota=\"0.000000\""), "missing 0% IVA fraction:\n{xml}");
    // No IVA collected at the document level.
    assert!(xml.contains("TotalImpuestosTrasladados=\"0.00\""));

    let report = provider(false)
        .report(&report_request(xml.into_bytes()))
        .unwrap();
    assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
}

/// Authority refusal (rechazo) surfaces the real SAT validation code. SAT's
/// CFDI 4.0 error catalogue defines CFDI40102 = "el sello del emisor no
/// corresponde al CSD". A rechazo is a *receipt status* inside `Ok`, never an
/// `Err`; the un-stamped comprobante is preserved for the audit trail and no
/// Folio Fiscal is issued. SAT Anexo 20 / "Estándar de servicios de validación".
#[test]
fn mexico_rejection_carries_sat_validation_code() {
    let doc = mexican_border_region_invoice();
    let xml = to_cfdi_xml(&doc, &border_ctx()).unwrap();
    let report = provider(true)
        .report(&report_request(xml.into_bytes()))
        .unwrap();

    assert_eq!(report.envelope.status, TimbradoStatus::Rechazado);
    assert!(report.envelope.folio_fiscal.is_empty());
    assert!(report.envelope.sello_sat.is_none());
    let reason = report.envelope.reason.expect("rechazo must carry a reason");
    assert!(
        reason.contains("CFDI40102"),
        "rejection reason must cite the SAT validation code CFDI40102, got: {reason:?}"
    );
    // validate_folio_fiscal must reject the empty (un-issued) folio.
    assert!(matches!(
        validate_folio_fiscal(&report.envelope.folio_fiscal),
        Err(CfdiReportError::BadXml(_))
    ));
}

/// Invalid Mexican identifiers are rejected pre-wire as `Err`, distinct from a
/// PAC rechazo. A 14-char RFC (neither 12 personas morales nor 13 personas
/// físicas), a digit in the name prefix, and a malformed Folio Fiscal all fail
/// the SAT shape rules. SAT RFC structure (Código Fiscal de la Federación) +
/// the TFD UUID (8-4-4-4-12 hex) shape.
#[test]
fn mexico_invalid_identifiers_are_rejected() {
    let doc = mexican_credit_note();
    let xml = to_cfdi_xml(&doc, &border_ctx()).unwrap().into_bytes();

    // 14-char RFC: report_request_for must refuse to build the request.
    let err = report_request_for(
        &doc,
        "CAZ010101AAAAA", // 14 chars
        CfdiEnvironment::Sandbox,
        csd(),
        xml,
    )
    .unwrap_err();
    assert!(matches!(err, CfdiReportError::BadRfc(_)));

    // A digit in the 3-letter razón-social prefix is not a valid persona-moral RFC.
    assert!(matches!(
        validate_rfc("CA1010101AAA"),
        Err(CfdiReportError::BadRfc(_))
    ));
    // A well-formed 12-char persona-moral RFC passes.
    validate_rfc(ISSUER_RFC).unwrap();

    // A truncated UUID is not a valid Folio Fiscal.
    assert!(matches!(
        validate_folio_fiscal("11111111-2222-4333-8444-55555555555"), // 11-hex tail
        Err(CfdiReportError::BadXml(_))
    ));
}

/// Determinism extends to the credit-note and multi-line variants: serializing
/// each twice yields byte-identical CFDI XML (required for reproducible evidence
/// bundles and replay).
#[test]
fn mexico_variant_serialization_is_deterministic() {
    let credit = mexican_credit_note();
    let multiline = mexican_border_region_invoice();
    let ctx = border_ctx();
    assert_eq!(
        to_cfdi_xml(&credit, &ctx).unwrap(),
        to_cfdi_xml(&credit, &ctx).unwrap(),
        "credit-note serialization must be byte-stable"
    );
    assert_eq!(
        to_cfdi_xml(&multiline, &ctx).unwrap(),
        to_cfdi_xml(&multiline, &ctx).unwrap(),
        "multi-line serialization must be byte-stable"
    );
    // Cross-variant: the two distinct documents must NOT serialize identically.
    assert_ne!(
        to_cfdi_xml(&credit, &ctx).unwrap(),
        to_cfdi_xml(&multiline, &ctx).unwrap()
    );
}
