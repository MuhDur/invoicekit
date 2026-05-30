// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Peru SUNAT offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Peru and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("PE")` + PEN
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml` (the
//!    EN16931/UBL family path SUNAT's SEE regime layers on top of)
//! 3. submit those bytes to the crate's `MockSunatProvider`, asserting the
//!    SUNAT CDR fields (`response_code == "0"`, `status == Aceptado`,
//!    SUNAT-recorded timestamp) and re-running the real RUC / document-id
//!    validators on the wire
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), pack, then `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the two pre-wire validation refusals (bad RUC / bad document id)
//!    surface as `Err`, NOT as an envelope verdict
//!
//! Mirrors the proven Italy SDI pattern in
//! `crates/report-it-sdi/tests/e2e_offline_lifecycle.rs`. Goldens are
//! hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). This test does NOT assert capability-matrix presence.
//!
//! Note on the rejection path: `MockSunatProvider` has no forced-receipt knob
//! — it always returns SUNAT `Aceptado` for a well-formed submission. So the
//! authority-side `Rechazado` verdict cannot be forced here; the
//! anti-slop "rejection is not an error" contract is instead exercised through
//! the pre-wire refusals (`SunatError::BadRuc` / `SunatError::BadDocumentId`),
//! which the crate's real country-specific validators raise as `Err`.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_pe_sunat::{
    MockSunatProvider, SunatDocumentKind, SunatEnvironment, SunatProvider, SunatStatus,
    SunatSubmitEnvelope, SunatSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const SUNAT_FIXED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_pe_e2e";
const TRACE: &str = "trace_pe_e2e";
const ISSUER_RUC: &str = "20123456789";
const DOCUMENT_ID: &str = "F001-00012345";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn peruvian_party(name: &str, ruc: &str, line: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "ruc".to_owned(),
            value: ruc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec![line.to_owned()],
            city: "Lima".to_owned(),
            subdivision: Some("LIM".to_owned()),
            postal_code: "15001".to_owned(),
            country: CountryCode::new("PE").unwrap(),
        },
        contact: None,
    }
}

fn peruvian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pe-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new(DOCUMENT_ID).unwrap(),
        // PEN — Peruvian Sol, the SUNAT-cleared domestic currency.
        currency: Iso4217Code::new("PEN").unwrap(),
        supplier: peruvian_party("Acme SAC", ISSUER_RUC, "Av. Javier Prado 100"),
        customer: peruvian_party("Beta EIRL", "20987654321", "Jr. de la Union 200"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoria de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // IGV (Impuesto General a las Ventas) at 18%.
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
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A single-line PE document parameterised on document type, SUNAT
/// catálogo-07 affectation code, currency, and amounts so each scenario can
/// assert real, country-specific wire values rather than reusing one fixture.
///
/// SUNAT catálogo 07 ("Tipo de afectación del IGV", verified against
/// `thegreenter/xcodes`, the open-source mirror of SUNAT's published code
/// lists, <https://github.com/thegreenter/xcodes>): `10` Gravado - Operación
/// Onerosa, `20` Exonerado, `30` Inafecto, `40` Exportación. We carry that code
/// in `tax_category`, which the UBL serializer emits as the
/// `cac:TaxCategory/cbc:ID` SUNAT reads off the SEE document.
#[allow(clippy::too_many_arguments)]
fn pe_single_line(
    id: &str,
    document_type: DocumentType,
    document_number: &str,
    currency: &str,
    afectacion: &str,
    net_minor: i64,
    igv_minor: i64,
    igv_rate_bp: i64,
) -> CommercialDocument {
    let due_date = match document_type {
        // UBL CreditNote cannot carry a top-level cbc:DueDate; SUNAT notas de
        // crédito reference the originating invoice, they do not re-date a due.
        DocumentType::CreditNote => None,
        _ => Some(DateOnly::new("2026-06-25").unwrap()),
    };
    let total_minor = net_minor + igv_minor;
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(id).unwrap(),
        document_type,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date,
        document_number: DocumentNumber::new(document_number).unwrap(),
        currency: Iso4217Code::new(currency).unwrap(),
        supplier: peruvian_party("Acme SAC", ISSUER_RUC, "Av. Javier Prado 100"),
        customer: peruvian_party("Beta EIRL", "20987654321", "Jr. de la Union 200"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoria de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(net_minor),
            line_extension_amount: amt(net_minor),
            tax_category: Some(afectacion.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: afectacion.to_owned(),
            taxable_amount: amt(net_minor),
            tax_amount: amt(igv_minor),
            tax_rate: Some(DecimalValue::new(Decimal::new(igv_rate_bp, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(net_minor),
            tax_exclusive_amount: amt(net_minor),
            tax_inclusive_amount: amt(total_minor),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(total_minor),
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

fn submit_request_for(
    kind: SunatDocumentKind,
    document_id: &str,
    invoice_xml: Vec<u8>,
) -> SunatSubmitRequest {
    SunatSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SunatEnvironment::Beta,
        kind,
        issuer_ruc: ISSUER_RUC.to_owned(),
        document_id: document_id.to_owned(),
        invoice_xml,
    }
}

/// Pack a {canonical.json, formats/ubl.xml, receipt.json} `.ikb` for `doc` +
/// `envelope` (the same artefact layout the happy-path lifecycle uses). Returns
/// the packed bytes so a scenario can assert it `verify`s.
fn bundle_for(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &SunatSubmitEnvelope) -> Vec<u8> {
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

/// Steps 1-4: build -> serialize (UBL) -> submit (Mock SUNAT) -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, SunatSubmitEnvelope) {
    // 1. build
    let doc = peruvian_invoice();

    // 2. serialize -> UBL 2.1 (the EN16931/UBL family path; SUNAT SEE is UBL 2.1)
    let ubl = to_xml(&doc).unwrap();
    // Structural sanity: the canonical UBL spine SUNAT submits over SOAP. The
    // canonicalizer attaches each namespace declaration inline on first use, so
    // match element-name prefixes rather than `name>`-terminated tags.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">20123456789<", // issuer RUC carried as cbc:CompanyID
        "PEN",           // PEN currency on every monetary amount
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the crate's existing Mock SUNAT provider (runs the real
    //    RUC + document-id validators on the wire, returns the CDR envelope).
    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
    let envelope = provider
        .submit_document(&submit_request_for(
            SunatDocumentKind::Factura,
            DOCUMENT_ID,
            ubl_bytes.clone(),
        ))
        .unwrap();

    // 4. evidence bundle: canonical IR doc + national UBL + SUNAT receipt.
    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn peru_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: SUNAT accepted the CDR (responseCode 0).
    assert_eq!(envelope.status, SunatStatus::Aceptado);
    assert_eq!(envelope.response_code, "0");
    assert_eq!(envelope.submitted_at, SUNAT_FIXED_AT);
    assert!(envelope.description.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn peru_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn peru_bad_ruc_is_refused_before_the_wire() {
    // Anti-slop contract: pre-wire shape failure is an `Err`, not an envelope.
    // MockSunatProvider exposes no forced-`Rechazado` knob (it always accepts a
    // well-formed submission), so the refusal path is proven through the real
    // RUC validator instead of an authority-side rejection.
    let doc = peruvian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);

    let mut req = submit_request_for(SunatDocumentKind::Factura, DOCUMENT_ID, ubl_bytes);
    req.issuer_ruc = "2012345678".to_owned(); // 10 digits — too short
    let err = provider.submit_document(&req).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_pe_sunat::SunatError::BadRuc(_)
        ),
        "short RUC must be refused with BadRuc, got {err:?}"
    );
}

#[test]
fn peru_bad_document_id_is_refused_before_the_wire() {
    let doc = peruvian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);

    let mut req = submit_request_for(SunatDocumentKind::Factura, DOCUMENT_ID, ubl_bytes);
    req.document_id = "NO-PREFIX".to_owned(); // wrong SSSS-NNNNNNNN shape
    let err = provider.submit_document(&req).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_pe_sunat::SunatError::BadDocumentId(_)
        ),
        "malformed document id must be refused with BadDocumentId, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Country-specific format / capability coverage (added on top of the baseline).
//
// All values below are grounded in SUNAT's published code lists, verified
// against `thegreenter/xcodes` (the open-source mirror of the SUNAT catálogos,
// <https://github.com/thegreenter/xcodes>) and SUNAT's own CPE portal
// (<https://cpe.sunat.gob.pe/>). Fixtures are hand-built; no regulator file is
// vendored.
// ---------------------------------------------------------------------------

/// Nota de Crédito Electrónica — SUNAT catálogo 06 document class `07`.
///
/// SUNAT's "Guía de Elaboración de Documentos XML — Nota de Crédito Electrónica
/// UBL 2.1" (<https://cpe.sunat.gob.pe/>) maps a credit note onto the OASIS UBL
/// 2.1 `CreditNote` root, with a `DiscrepancyResponse/ResponseCode` drawn from
/// catálogo 09 ("Códigos de nota de crédito", e.g. `01` = "Anulación de la
/// operación"). This exercises the credit-note serialization branch
/// (`<CreditNote>` root, `cac:CreditNoteLine`, `cbc:CreditedQuantity`) and the
/// SUNAT catálogo-06 code for that class.
#[test]
fn peru_credit_note_serializes_as_ubl_credit_note_and_clears() {
    // catálogo 07 `10` (Gravado), IGV 18% — a credit note that reverses a gravado sale.
    let cn = pe_single_line(
        "doc-pe-e2e-nc-1",
        DocumentType::CreditNote,
        "FC01-00000123",
        "PEN",
        "10",
        10_000, // 100.00 net reversed
        1_800,  // 18.00 IGV reversed
        1_800,  // 18.00% rate
    );

    let ubl = to_xml(&cn).unwrap();
    // UBL CreditNote spine — the root and line element names differ from Invoice.
    for needle in [
        "<CreditNote",
        "<cac:CreditNoteLine",
        "<cbc:CreditedQuantity",
        ">10</cbc:ID>", // catálogo-07 affectation code carried on cac:TaxCategory/cbc:ID
        ">PEN</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "credit-note UBL missing {needle}");
    }
    // A credit note must NOT carry the Invoice <cbc:DueDate> spine.
    assert!(
        !ubl.contains("<cbc:DueDate"),
        "SUNAT nota de crédito must not carry a top-level DueDate"
    );
    let ubl_bytes = ubl.into_bytes();

    // catálogo 06 code for Nota de Crédito is `07`.
    assert_eq!(SunatDocumentKind::NotaCredito.code(), "07");

    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
    let envelope = provider
        .submit_document(&submit_request_for(
            SunatDocumentKind::NotaCredito,
            "FC01-00000123",
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(envelope.status, SunatStatus::Aceptado);
    assert_eq!(envelope.response_code, "0");

    let ikb = bundle_for(&cn, &ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// Factura de Exportación — SUNAT catálogo 07 affectation `40` (Exportación).
///
/// Per SUNAT, an export operation is not subject to IGV: the line carries a 0%
/// rate and a zero `cbc:TaxAmount`, and is conventionally billed in foreign
/// currency (USD). This exercises the zero-tax `cac:TaxTotal` path and a
/// non-PEN `DocumentCurrencyCode`, both of which an export adapter must get
/// right. Affectation code `40` is verified against `thegreenter/xcodes`
/// catálogo 07.
#[test]
fn peru_export_invoice_is_zero_rated_in_usd() {
    let export = pe_single_line(
        "doc-pe-e2e-exp-1",
        DocumentType::Invoice,
        "F001-00099001",
        "USD",
        "40", // Exportación
        50_000, // 500.00 net
        0,      // no IGV on an export
        0,      // 0.00% rate
    );

    let ubl = to_xml(&export).unwrap();
    // The canonicalizer attaches each namespace declaration inline on first use,
    // so match on the value-bearing closing tail rather than the bare open tag.
    for needle in [
        ">USD</cbc:DocumentCurrencyCode>",
        r#"currencyID="USD""#,
        "<cac:TaxTotal ",
        r#"currencyID="USD">0.00</cbc:TaxAmount>"#,
        ">40</cbc:ID>", // catálogo-07 Exportación code on cac:TaxCategory/cbc:ID
    ] {
        assert!(ubl.contains(needle), "export UBL missing {needle}");
    }
    // Exports are billed in USD, never PEN.
    assert!(
        !ubl.contains(">PEN<") && !ubl.contains(r#"currencyID="PEN""#),
        "an export invoice must not carry PEN amounts"
    );
    let ubl_bytes = ubl.into_bytes();

    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
    let envelope = provider
        .submit_document(&submit_request_for(
            SunatDocumentKind::Factura,
            "F001-00099001",
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(envelope.status, SunatStatus::Aceptado);

    let ikb = bundle_for(&export, &ubl_bytes, &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "export evidence bundle must verify"
    );
}

/// Operación Exonerada — SUNAT catálogo 07 affectation `20` (Exonerado).
///
/// An exonerated domestic operation (e.g. certain agricultural goods under the
/// IGV exemption regime) is billed in PEN with a zero IGV amount but, unlike an
/// export, stays domestic. This proves the zero-tax domestic branch is distinct
/// from the export branch above (same 0 IGV, different currency + affectation
/// code). Affectation `20` is verified against `thegreenter/xcodes` catálogo 07.
#[test]
fn peru_exonerated_invoice_carries_pen_zero_igv() {
    let exo = pe_single_line(
        "doc-pe-e2e-exo-1",
        DocumentType::Invoice,
        "F001-00099002",
        "PEN",
        "20", // Exonerado
        30_000, // 300.00 net
        0,      // exonerado => no IGV
        0,
    );

    let ubl = to_xml(&exo).unwrap();
    for needle in [
        ">PEN</cbc:DocumentCurrencyCode>",
        r#"currencyID="PEN">0.00</cbc:TaxAmount>"#,
        r#"currencyID="PEN">300.00</cbc:TaxableAmount>"#,
        ">20</cbc:ID>", // catálogo-07 Exonerado code on cac:TaxCategory/cbc:ID
    ] {
        assert!(ubl.contains(needle), "exonerado UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
    let envelope = provider
        .submit_document(&submit_request_for(
            SunatDocumentKind::Factura,
            "F001-00099002",
            ubl_bytes,
        ))
        .unwrap();
    assert_eq!(envelope.status, SunatStatus::Aceptado);
    assert_eq!(envelope.response_code, "0");
}

/// A two-line factura mixing affectations: one gravado line (catálogo 07 `10`,
/// IGV 18%) and one exonerado line (catálogo 07 `20`, IGV 0%). SUNAT requires a
/// separate IGV breakdown per affectation, emitted as one `cac:TaxSubtotal` per
/// `TaxCategorySummary`.
fn pe_multi_line_mixed() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pe-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("F001-00099003").unwrap(),
        currency: Iso4217Code::new("PEN").unwrap(),
        supplier: peruvian_party("Acme SAC", ISSUER_RUC, "Av. Javier Prado 100"),
        customer: peruvian_party("Beta EIRL", "20987654321", "Jr. de la Union 200"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Consultoria gravada (IGV 18%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(10_000),
                line_extension_amount: amt(10_000),
                tax_category: Some("10".to_owned()), // Gravado
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Bien exonerado".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5_000),
                line_extension_amount: amt(5_000),
                tax_category: Some("20".to_owned()), // Exonerado
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "10".to_owned(),
                taxable_amount: amt(10_000),
                tax_amount: amt(1_800),
                tax_rate: Some(DecimalValue::new(Decimal::new(1_800, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "20".to_owned(),
                taxable_amount: amt(5_000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(15_000),
            tax_exclusive_amount: amt(15_000),
            tax_inclusive_amount: amt(16_800),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(16_800),
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

/// Multi-line / mixed-affectation factura: proves the multi-line, multi-rate
/// path (two `cac:InvoiceLine` entries, two `cac:TaxSubtotal` breakdowns, both
/// catálogo-07 codes present, and the aggregate IGV of only the gravado line).
#[test]
fn peru_multi_line_mixed_affectation_invoice() {
    let doc = pe_multi_line_mixed();

    let ubl = to_xml(&doc).unwrap();
    // Two InvoiceLine entries and two TaxSubtotal breakdowns.
    assert_eq!(
        ubl.matches("<cac:InvoiceLine").count(),
        2,
        "multi-line invoice must emit two InvoiceLine entries"
    );
    assert_eq!(
        ubl.matches("<cac:TaxSubtotal").count(),
        2,
        "mixed affectation must emit one TaxSubtotal per IGV category"
    );
    for needle in [
        ">10</cbc:ID>", // Gravado affectation on a TaxCategory/cbc:ID
        ">20</cbc:ID>", // Exonerado affectation on a TaxCategory/cbc:ID
        // Aggregate IGV across both subtotals: 18.00 (only the gravado line is taxed).
        r#"currencyID="PEN">18.00</cbc:TaxAmount>"#,
    ] {
        assert!(ubl.contains(needle), "multi-line UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
    let envelope = provider
        .submit_document(&submit_request_for(
            SunatDocumentKind::Factura,
            "F001-00099003",
            ubl_bytes.clone(),
        ))
        .unwrap();
    assert_eq!(envelope.status, SunatStatus::Aceptado);

    let ikb = bundle_for(&doc, &ubl_bytes, &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "multi-line evidence bundle must verify"
    );
}

/// Authority REJECTION path — SUNAT CDR `responseCode` 2335 ("El documento
/// electrónico ingresado ha sido alterado"), verified against
/// `thegreenter/xcodes`'s `CodeErrors.xml`
/// (<https://github.com/thegreenter/xcodes/blob/master/src/data/CodeErrors.xml>).
///
/// SUNAT returns rejections (responseCode 2000–3999) inside the CDR, NOT as a
/// transport error. The crate's contract (see `SunatProvider::submit_document`
/// docs) makes `SunatStatus::Rechazado` a *verdict carried in the envelope*, so
/// the audit trail persists the refusal and the evidence bundle still verifies.
///
/// `MockSunatProvider` exposes no forced-`Rechazado` knob (it always accepts a
/// well-formed submission), so — exactly as the module header notes — the
/// authority-side rejection is modelled by constructing the real CDR verdict
/// envelope SUNAT would return for code 2335 and proving it bundles + verifies.
#[test]
fn peru_authority_rejection_2335_is_a_verdict_not_an_error() {
    let doc = peruvian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // The CDR SUNAT returns when it refuses an altered document.
    let rejected = SunatSubmitEnvelope {
        response_code: "2335".to_owned(),
        status: SunatStatus::Rechazado,
        submitted_at: SUNAT_FIXED_AT.to_owned(),
        description: Some("El documento electronico ingresado ha sido alterado".to_owned()),
    };

    // Anti-slop contract: a refusal is a verdict, never an Err. The numeric code
    // falls in SUNAT's documented rejection band 2000–3999.
    assert_eq!(rejected.status, SunatStatus::Rechazado);
    let code: u32 = rejected.response_code.parse().unwrap();
    assert!(
        (2000..=3999).contains(&code),
        "2335 must fall in SUNAT's documented rejection band"
    );

    // The rejection still produces a verifiable audit bundle.
    let ikb = bundle_for(&doc, &ubl_bytes, &rejected);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

/// Invalid-identifier rejection (extra country-specific shapes beyond the
/// baseline two). The Peruvian RUC is exactly 11 ASCII digits and the SUNAT
/// document id is `SSSS-NNNNNNNN` (4-char series + 1..8 digit correlative).
/// Boletas use a `B`-prefixed series; a Spanish-letter accent or an over-long
/// correlative must be refused *before* the wire as a typed `Err`, not as a CDR.
#[test]
fn peru_additional_identifier_shapes_are_refused_before_the_wire() {
    let doc = peruvian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);

    // RUC with a non-ASCII-digit character (still 11 chars) — refused.
    let mut req = submit_request_for(SunatDocumentKind::Factura, DOCUMENT_ID, ubl_bytes.clone());
    req.issuer_ruc = "2012345678X".to_owned();
    assert!(
        matches!(
            provider.submit_document(&req).unwrap_err(),
            invoicekit_report_pe_sunat::SunatError::BadRuc(_)
        ),
        "non-digit RUC must be refused with BadRuc"
    );

    // Boleta document id with a 9-digit correlative (max is 8) — refused.
    let mut req = submit_request_for(SunatDocumentKind::Boleta, "B001-123456789", ubl_bytes);
    req.issuer_ruc = ISSUER_RUC.to_owned();
    assert!(
        matches!(
            provider.submit_document(&req).unwrap_err(),
            invoicekit_report_pe_sunat::SunatError::BadDocumentId(_)
        ),
        "over-long correlative must be refused with BadDocumentId"
    );
}

/// Determinism for the non-Invoice path: serializing + bundling a Nota de
/// Crédito twice must be byte-identical (the baseline determinism test only
/// covers the gravado Factura). Determinism is the load-bearing property behind
/// reproducible SUNAT evidence.
#[test]
fn peru_credit_note_lifecycle_is_byte_deterministic() {
    let build = || {
        let cn = pe_single_line(
            "doc-pe-e2e-nc-det",
            DocumentType::CreditNote,
            "FC01-00000777",
            "PEN",
            "10",
            10_000,
            1_800,
            1_800,
        );
        let ubl_bytes = to_xml(&cn).unwrap().into_bytes();
        let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);
        let envelope = provider
            .submit_document(&submit_request_for(
                SunatDocumentKind::NotaCredito,
                "FC01-00000777",
                ubl_bytes.clone(),
            ))
            .unwrap();
        bundle_for(&cn, &ubl_bytes, &envelope)
    };
    assert_eq!(build(), build(), "credit-note lifecycle must be byte-stable");
}

/// Every SUNAT catálogo-06 document class the crate models clears the wire and
/// reports its real code. Confirms `SunatDocumentKind` is not a dead enum:
/// each variant carries the published catálogo-06 code (01/03/07/08/09) and the
/// provider accepts a well-formed submission of that kind. Codes verified
/// against `thegreenter/xcodes` catálogo 06.
#[test]
fn peru_all_catalog_06_kinds_clear_the_wire() {
    let doc = peruvian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockSunatProvider::with_fixed_submitted_at(SUNAT_FIXED_AT);

    // (kind, expected catálogo-06 code, a valid SSSS-NNNNNNNN id for that kind)
    let cases = [
        (SunatDocumentKind::Factura, "01", "F001-00000001"),
        (SunatDocumentKind::Boleta, "03", "B001-00000001"),
        (SunatDocumentKind::NotaCredito, "07", "FC01-00000001"),
        (SunatDocumentKind::NotaDebito, "08", "FD01-00000001"),
        (SunatDocumentKind::GuiaRemision, "09", "T001-00000001"),
    ];
    for (kind, code, doc_id) in cases {
        assert_eq!(kind.code(), code, "catálogo-06 code for {kind:?}");
        let envelope = provider
            .submit_document(&submit_request_for(kind, doc_id, ubl_bytes.clone()))
            .unwrap();
        assert_eq!(
            envelope.status,
            SunatStatus::Aceptado,
            "{kind:?} ({code}) should clear the wire"
        );
    }
}
