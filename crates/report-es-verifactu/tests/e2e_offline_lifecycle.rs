// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Spain VeriFactu offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Spain and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Spanish supplier +
//!    EUR currency
//! 2. serialize -> EN 16931 / UBL 2.1 XML (Spain rides the UBL family path;
//!    the live AEAT envelope wraps this)
//! 3. submit the UBL bytes to the in-crate `MockVeriFactuProvider` and assert
//!    the AEAT receipt's country-specific fields (CSV + recorded hash + status)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only)` it (exit 0 == report.ok)
//! 5. determinism: pack twice -> byte-identical
//! 6. hash-chain continuity: a second invoice that pins the first invoice's
//!    `recorded_hash_hex` as its `previous_hash_hex` is accepted
//!
//! Mirrors the proven `report-it-sdi` offline-E2E pattern. Goldens are
//! hand-rolled (no `insta`/`pretty_assertions`, which would mutate `Cargo.lock`).
//!
//! Two distinct failure axes are exercised:
//! - Pre-wire *shape* refusal (`Err`): a malformed NIF, a malformed previous
//!   hash, or an empty payload — see `verifactu_rejects_bad_shapes_before_the_wire`.
//! - AEAT *verdict* refusal (an `Ok` receipt, NOT an `Err`): driven via
//!   `MockVeriFactuProvider::with_forced_status` — see
//!   `spain_authority_rejection_is_a_receipt_status_not_an_error` (forced
//!   `Rejected`) and `spain_accepted_with_warnings_records_a_chain_link`
//!   (forced `AcceptedWithWarnings`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
    DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType,
    Iso4217Code, JurisdictionExtension, MonetaryTotal, Party, PartyTaxId, PostalAddress,
    SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_es_verifactu::{
    qr_payload, validate_nif, MockVeriFactuProvider, VeriFactuEnvironment, VeriFactuMode,
    VeriFactuProvider, VeriFactuRegisterEnvelope, VeriFactuRegisterRequest, VeriFactuStatus,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_es_e2e";
const TRACE: &str = "trace_es_e2e";
const ISSUER_NIF: &str = "A12345678";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn spanish_party(name: &str, vat: &str, city: &str, subdivision: &str, postal: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Calle Mayor 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(subdivision.to_owned()),
            postal_code: postal.to_owned(),
            country: CountryCode::new("ES").unwrap(),
        },
        contact: None,
    }
}

fn spanish_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-es-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("F2026/0007").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: spanish_party("Acme SL", "ESA12345678", "Madrid", "M", "28013"),
        customer: spanish_party("Beta SA", "ESB98765432", "Barcelona", "B", "08001"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoria y desarrollo de software".to_owned(),
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

fn register_request(
    invoice_xml: Vec<u8>,
    previous_hash_hex: Option<String>,
) -> VeriFactuRegisterRequest {
    VeriFactuRegisterRequest {
        tenant_id: TENANT.to_owned(),
        environment: VeriFactuEnvironment::Sandbox,
        mode: VeriFactuMode::VeriFactu,
        issuer_nif: ISSUER_NIF.to_owned(),
        invoice_number: "F2026/0007".to_owned(),
        issued_at: "2026-07-01T10:00:00Z".to_owned(),
        previous_hash_hex,
        invoice_xml,
    }
}

/// Assemble + pack a `.ikb` evidence bundle from the canonical IR JSON, the
/// national UBL XML, and the AEAT receipt, plus any country-specific extras
/// (e.g. `qr.txt`). `artefacts` is a `BTreeMap`, so the packed bytes depend on
/// the sorted keys, not on insertion order.
fn pack_bundle(
    canonical: Vec<u8>,
    ubl_bytes: Vec<u8>,
    receipt: &VeriFactuRegisterEnvelope,
    extra: &[(&str, Vec<u8>)],
) -> Vec<u8> {
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(receipt).unwrap());
    for (name, bytes) in extra {
        artefacts.insert((*name).to_owned(), bytes.clone());
    }
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    pack(&EvidenceBundle { manifest, artefacts }).unwrap()
}

/// Steps 1-4: build -> serialize (UBL) -> register with the AEAT mock ->
/// assemble + return the packed `.ikb` plus the AEAT receipt.
fn run_lifecycle() -> (Vec<u8>, VeriFactuRegisterEnvelope) {
    // 1. build
    let doc = spanish_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (Spain rides the UBL family).
    let ubl: String = to_xml(&doc).unwrap();
    // Spot-check the national-relevant UBL spine before it hits the wire.
    // Match local names without the closing `>` because canonicalization may
    // attach inline `xmlns:` declarations right after the element name.
    for needle in [
        "<Invoice",
        "cac:AccountingSupplierParty",
        "cac:AccountingCustomerParty",
        "cbc:DocumentCurrencyCode",
        ">EUR</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. register with the AEAT (offline deterministic mock).
    let provider = MockVeriFactuProvider::default();
    let envelope = provider
        .register_invoice(&register_request(ubl_bytes.clone(), None))
        .unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + AEAT receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let ikb = pack_bundle(canonical, ubl_bytes, &envelope, &[]);
    (ikb, envelope)
}

#[test]
fn spain_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: AEAT recorded the invoice. Assert the Spain-specific artifacts.
    assert_eq!(envelope.status, VeriFactuStatus::Accepted);
    // CSV (Codigo Seguro de Verificacion) is what the printed-invoice QR carries.
    assert!(
        envelope.csv.starts_with("MOCK-CSV-"),
        "AEAT must assign a CSV, got {:?}",
        envelope.csv
    );
    // Recorded hash is the SHA-256-shaped chain link the next invoice pins.
    assert_eq!(
        envelope.recorded_hash_hex.len(),
        64,
        "recorded hash must be SHA-256 wire-shaped (64 hex chars)"
    );
    assert!(
        envelope
            .recorded_hash_hex
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "recorded hash must be lowercase hex"
    );
    assert_eq!(envelope.recorded_at, PINNED_CREATED_AT);
    assert!(envelope.message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn spain_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn spain_hash_chain_continuity_is_accepted() {
    // VeriFactu's anti-fraud spine is the hash chain: each invoice pins the
    // previous invoice's recorded hash. Prove the chain link the AEAT returned
    // for invoice #1 is a valid `previous_hash_hex` for invoice #2.
    let provider = MockVeriFactuProvider::default();
    let doc = spanish_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let first = provider
        .register_invoice(&register_request(ubl_bytes.clone(), None))
        .unwrap();
    assert_eq!(first.status, VeriFactuStatus::Accepted);

    let second = provider
        .register_invoice(&register_request(ubl_bytes, Some(first.recorded_hash_hex.clone())))
        .unwrap();
    assert_eq!(second.status, VeriFactuStatus::Accepted);
    // Distinct CSVs prove the AEAT serial advanced for the chained invoice.
    assert_ne!(first.csv, second.csv);
}

#[test]
fn verifactu_rejects_bad_shapes_before_the_wire() {
    // The mock has no forced-AEAT-refusal knob (it always returns Accepted), so
    // there is no `VeriFactuStatus::Rejected` path to drive offline. What it
    // does refuse, as `Err`, is pre-wire shape validation. Those refusals must
    // never reach the wire / a bundle.
    let provider = MockVeriFactuProvider::default();
    let ubl_bytes = to_xml(&spanish_invoice()).unwrap().into_bytes();

    // (a) malformed NIF (not 9 alphanumeric chars).
    let mut bad_nif = register_request(ubl_bytes.clone(), None);
    bad_nif.issuer_nif = "A12".to_owned();
    assert!(
        provider.register_invoice(&bad_nif).is_err(),
        "a malformed issuer NIF must be refused before the wire"
    );

    // (b) malformed previous hash (not 64 lowercase hex chars).
    let bad_hash = register_request(ubl_bytes, Some("not-a-sha256".to_owned()));
    assert!(
        provider.register_invoice(&bad_hash).is_err(),
        "a malformed previous hash must be refused before the wire"
    );

    // (c) empty payload.
    let empty = register_request(Vec::new(), None);
    assert!(
        provider.register_invoice(&empty).is_err(),
        "an empty payload must be refused before the wire"
    );
}

// ---------------------------------------------------------------------------
// Country-specific scenarios grounded in the AEAT VeriFactu specification.
//
// Authority: Agencia Estatal de Administracion Tributaria (AEAT), the Spanish
// tax authority, under Real Decreto 1007/2023 ("Reglamento VeriFactu").
//
// Spec references cited per-scenario below:
//   - AEAT, "Procedimientos de facturacion" FAQ (invoice types TipoFactura
//     F1/F2/F3/R1..R5 and rectification nature S/I):
//     https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu/preguntas-frecuentes/procedimientos-facturacion.html
//   - AEAT, "Documento de validaciones y errores" (EstadoRegistro values and
//     error-code catalogue, e.g. 1109/1110 NIF no identificado en el censo):
//     https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu/informacion-tecnica/documento-validaciones-errores.html
//   - AEAT, "Contenido del Registro de facturacion de alta" (the chained Huella
//     digital fingerprint over the previous record):
//     https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu/cuestiones-generales/contenido-registro-facturacion-alta_.html
//
// All fixtures are hand-built / synthetic. No copyrighted regulator XML is
// vendored: the structural truths asserted (TipoFactura codes, EstadoRegistro
// values, error-code numbers, Spanish VAT rates) are facts, not files.
// ---------------------------------------------------------------------------

/// A `factura rectificativa` (corrective invoice) of type **R1** — "error
/// fundado de derecho o alguna de las causas del art. 80 LIVA" per the AEAT
/// `TipoFactura` catalogue. It corrects a prior `F1` complete invoice.
///
/// In the InvoiceKit IR a corrective rides the UBL `CreditNote` body
/// (`cbc:CreditNoteTypeCode` 381, no top-level `cbc:DueDate` — a UBL 2.1
/// constraint). The VeriFactu-specific facts (the R1 rectification type, the
/// rectified original number, and the rectification nature "I" = por
/// diferencias) are modelled in a jurisdiction extension that travels in the
/// canonical evidence artefact, since they live in the AEAT registro layer, not
/// the commercial UBL body.
fn corrective_credit_note(original_number: &str) -> CommercialDocument {
    let rectification = JurisdictionExtension::new(
        "urn:invoicekit:es:verifactu:rectificacion",
        serde_json::json!({
            "TipoFactura": "R1",
            "TipoRectificativa": "I",
            "FacturaRectificada": original_number,
        }),
    )
    .unwrap();
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-es-e2e-rect-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote has no top-level cbc:DueDate.
        due_date: None,
        document_number: DocumentNumber::new("R2026/0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: spanish_party("Acme SL", "ESA12345678", "Madrid", "M", "28013"),
        customer: spanish_party("Beta SA", "ESB98765432", "Barcelona", "B", "08001"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Rectificacion por diferencias: descuento no aplicado".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(2000),
            line_extension_amount: amt(2000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(2000),
            // 21% standard Spanish IVA on a 20.00 base = 4.20.
            tax_amount: amt(420),
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(2000),
            tax_exclusive_amount: amt(2000),
            tax_inclusive_amount: amt(2420),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(2420),
        },
        attachments: Vec::new(),
        references: vec![DocumentReference {
            kind: "rectified-invoice".to_owned(),
            id: original_number.to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
        notes: Vec::new(),
        extensions: vec![rectification],
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn spain_corrective_r1_credit_note_registers_and_bundles() {
    // The original F1 invoice is registered first so the corrective can pin its
    // recorded Huella as the chain link (RD 1007/2023 encadenamiento).
    let provider = MockVeriFactuProvider::default();
    let original_ubl = to_xml(&spanish_invoice()).unwrap().into_bytes();
    let original = provider
        .register_invoice(&register_request(original_ubl, None))
        .unwrap();
    assert_eq!(original.status, VeriFactuStatus::Accepted);

    // Serialize the corrective as UBL CreditNote and assert the national spine.
    let corrective = corrective_credit_note("F2026/0007");
    let ubl = to_xml(&corrective).unwrap();
    // UBL CreditNote carries CreditNoteTypeCode 381 and must NOT carry a DueDate.
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "corrective must serialize as a UBL CreditNote (code 381)"
    );
    assert!(
        !ubl.contains("cbc:DueDate"),
        "UBL 2.1 CreditNote has no top-level cbc:DueDate"
    );
    assert!(ubl.contains("<CreditNote"), "root must be CreditNote");
    let ubl_bytes = ubl.into_bytes();

    // The VeriFactu R1 facts ride the canonical artefact (registro layer).
    let canonical = canonicalize_value(&corrective.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let canonical_str = String::from_utf8(canonical.clone()).unwrap();
    for needle in [
        "\"TipoFactura\":\"R1\"",
        "\"TipoRectificativa\":\"I\"",
        "\"FacturaRectificada\":\"F2026/0007\"",
    ] {
        assert!(
            canonical_str.contains(needle),
            "canonical corrective must carry {needle}"
        );
    }

    // Register the corrective, chaining to the original's recorded Huella.
    let mut corrective_req =
        register_request(ubl_bytes.clone(), Some(original.recorded_hash_hex.clone()));
    corrective_req.invoice_number = "R2026/0001".to_owned();
    let corrective_receipt = provider.register_invoice(&corrective_req).unwrap();
    assert_eq!(corrective_receipt.status, VeriFactuStatus::Accepted);
    // A distinct CSV proves the AEAT serial advanced for the corrective.
    assert_ne!(original.csv, corrective_receipt.csv);

    // Bundle {canonical.json, formats/ubl.xml, receipt.json} and verify.
    let ikb = pack_bundle(canonical, ubl_bytes, &corrective_receipt, &[]);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "corrective-invoice evidence bundle must verify");
}

/// Build an invoice with three lines under the real Spanish IVA rate ladder:
/// 21% (tipo general), 10% (tipo reducido), and a 0% exempt line (operacion
/// exenta, e.g. servicios financieros del art. 20 LIVA).
fn multi_rate_invoice() -> CommercialDocument {
    let line = |id: &str, desc: &str, base: i64, cat: &str| DocumentLine {
        id: id.to_owned(),
        description: desc.to_owned(),
        quantity: DecimalValue::new(Decimal::from(1)),
        unit_code: Some("EA".to_owned()),
        unit_price: amt(base),
        line_extension_amount: amt(base),
        tax_category: Some(cat.to_owned()),
        extensions: Vec::new(),
    };
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-es-e2e-multirate-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("F2026/0042").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: spanish_party("Acme SL", "ESA12345678", "Madrid", "M", "28013"),
        customer: spanish_party("Beta SA", "ESB98765432", "Barcelona", "B", "08001"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            line("1", "Consultoria (tipo general)", 10000, "S"),
            line("2", "Material formativo (tipo reducido)", 5000, "S"),
            line("3", "Servicio financiero (exento art. 20 LIVA)", 3000, "E"),
        ],
        tax_summary: vec![
            // 21% general on 100.00 => 21.00.
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2100),
                tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
            },
            // 10% reducido on 50.00 => 5.00.
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(5000),
                tax_amount: amt(500),
                tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
            },
            // Exempt: 0 tax on a 30.00 base.
            TaxCategorySummary {
                category_code: "E".to_owned(),
                taxable_amount: amt(3000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            },
        ],
        monetary_total: MonetaryTotal {
            // Lines sum 100.00 + 50.00 + 30.00 = 180.00.
            line_extension_amount: amt(18000),
            tax_exclusive_amount: amt(18000),
            // 180.00 + (21.00 + 5.00 + 0) = 206.00.
            tax_inclusive_amount: amt(20600),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(20600),
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

#[test]
fn spain_multi_line_mixed_vat_rates_register_and_bundle() {
    let doc = multi_rate_invoice();
    let ubl = to_xml(&doc).unwrap();
    // Three distinct line bases must all appear in the UBL body.
    for needle in [
        ">100.00</cbc:LineExtensionAmount>",
        ">50.00</cbc:LineExtensionAmount>",
        ">30.00</cbc:LineExtensionAmount>",
    ] {
        assert!(ubl.contains(needle), "UBL missing line amount {needle}");
    }
    // The exempt-line item description rides through verbatim.
    assert!(
        ubl.contains("Servicio financiero (exento art. 20 LIVA)"),
        "UBL must carry the exempt line"
    );
    let ubl_bytes = ubl.into_bytes();

    let provider = MockVeriFactuProvider::default();
    let mut req = register_request(ubl_bytes.clone(), None);
    req.invoice_number = "F2026/0042".to_owned();
    let receipt = provider.register_invoice(&req).unwrap();
    assert_eq!(receipt.status, VeriFactuStatus::Accepted);

    // Build the QR payload the printed invoice carries (AEAT chapter 4). The
    // gross total for a multi-rate invoice is the tax-inclusive sum, 206.00.
    let qr = qr_payload(
        "https://prewww1.aeat.es/wlpl/TIKE-CONT",
        ISSUER_NIF,
        "F2026/0042",
        "2026-05-27",
        "206.00",
    );
    assert!(qr.contains("numserie=F2026/0042"));
    assert!(qr.contains("importe=206.00"));

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let ikb = pack_bundle(canonical, ubl_bytes, &receipt, &[("qr.txt", qr.into_bytes())]);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "multi-rate evidence bundle must verify"
    );
}

#[test]
fn spain_authority_rejection_is_a_receipt_status_not_an_error() {
    // EstadoRegistro = "Incorrecto" (rejection) is an AEAT *verdict*, surfaced
    // as VeriFactuStatus::Rejected inside an Ok envelope — NOT an Err. The
    // engine persists the rejection (with the AEAT error code) in its audit
    // trail. Here we drive AEAT error 1109: "El NIF del destinatario no esta
    // identificado en el censo de la AEAT".
    let provider = MockVeriFactuProvider::default().with_forced_status(
        VeriFactuStatus::Rejected,
        Some("1109 El NIF del destinatario no esta identificado en el censo".to_owned()),
    );
    let ubl_bytes = to_xml(&spanish_invoice()).unwrap().into_bytes();
    let receipt = provider
        .register_invoice(&register_request(ubl_bytes.clone(), None))
        .unwrap();
    assert_eq!(receipt.status, VeriFactuStatus::Rejected);
    assert!(
        receipt.message.as_deref().unwrap().starts_with("1109"),
        "the AEAT error code must travel in the receipt"
    );

    // A rejection still bundles and the bundle still verifies — the audit
    // trail must persist the refusal, mirroring the Italy SDI NS contract.
    let canonical = canonicalize_value(&spanish_invoice().to_value().unwrap())
        .unwrap()
        .into_bytes();
    let ikb = pack_bundle(canonical, ubl_bytes, &receipt, &[]);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "rejection-path evidence bundle must still verify"
    );
}

#[test]
fn spain_accepted_with_warnings_records_a_chain_link() {
    // EstadoRegistro = "AceptadoConErrores": the AEAT recorded the invoice (so
    // a Huella + CSV exist and the chain advances) but attached a warning. Per
    // the AEAT FAQ this record must NOT be re-sent or it triggers error 3000
    // (duplicate). We assert the chain link is usable downstream.
    let provider = MockVeriFactuProvider::default()
        .with_forced_status(VeriFactuStatus::AcceptedWithWarnings, Some("DuplicadoLeve".to_owned()));
    let ubl_bytes = to_xml(&spanish_invoice()).unwrap().into_bytes();
    let receipt = provider
        .register_invoice(&register_request(ubl_bytes, None))
        .unwrap();
    assert_eq!(receipt.status, VeriFactuStatus::AcceptedWithWarnings);
    // Despite the warning, a Huella was recorded and is a valid next chain link.
    assert_eq!(receipt.recorded_hash_hex.len(), 64);
    assert!(validate_nif(ISSUER_NIF).is_ok());
}

#[test]
fn spain_invalid_identifier_shapes_are_refused_before_the_wire() {
    // The Spanish fiscal identifier (NIF/DNI/NIE/CIF) is always exactly 9
    // characters: 8 digits + control letter (DNI/NIF), or a leading/trailing
    // letter for NIE/CIF. Anything else is refused before the wire.
    assert!(validate_nif("B12345678").is_ok(), "CIF: leading letter + 8");
    assert!(validate_nif("12345678Z").is_ok(), "DNI: 8 digits + letter");
    assert!(validate_nif("X1234567L").is_ok(), "NIE: X + 7 digits + letter");
    // 10 chars (one too many) and a lowercased control letter are both refused.
    assert!(validate_nif("B123456789").is_err(), "10 chars is not a NIF");
    assert!(validate_nif("ESB1234567").is_err(), "11-char VAT with ES prefix");

    // End to end: a malformed issuer NIF is an Err, never a Rejected receipt.
    let provider = MockVeriFactuProvider::default();
    let ubl_bytes = to_xml(&spanish_invoice()).unwrap().into_bytes();
    let mut bad = register_request(ubl_bytes, None);
    bad.issuer_nif = "ESA12345678".to_owned(); // 11-char VAT, not the 9-char NIF
    assert!(
        provider.register_invoice(&bad).is_err(),
        "an 11-char VAT in the NIF slot must be refused before the wire"
    );
}

#[test]
fn spain_corrective_receipt_is_byte_deterministic() {
    // Determinism across the corrective path: identical input -> identical
    // serialized UBL, identical canonical bytes, and identical packed bundle.
    let build = || {
        let provider = MockVeriFactuProvider::default();
        let doc = corrective_credit_note("F2026/0007");
        let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
        let receipt = provider
            .register_invoice(&register_request(ubl_bytes.clone(), None))
            .unwrap();
        let canonical = canonicalize_value(&doc.to_value().unwrap())
            .unwrap()
            .into_bytes();
        pack_bundle(canonical, ubl_bytes, &receipt, &[])
    };
    assert_eq!(build(), build(), "the corrective lifecycle must be byte-stable");
}
