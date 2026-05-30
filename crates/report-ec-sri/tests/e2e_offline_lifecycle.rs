// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Ecuador SRI offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Ecuador and proves it
//! deterministically, using only crates already resolved in `Cargo.lock`:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("EC")`
//!    and the Ecuadorian official currency (USD — Ecuador is dollarized);
//! 2. serialize to bytes via `invoicekit_format_ubl::to_xml` (the EN 16931 /
//!    UBL family path — SRI's national `factura` XML serializer is a follow-up,
//!    so the foundation UBL path is the honest "faithful typed payload" here);
//! 3. submit those bytes to the crate's existing `MockSriProvider` and assert
//!    the SRI autorización envelope's country-specific fields
//!    (`numeroAutorizacion` == the 49-digit Clave de Acceso, `Autorizado`
//!    status, pinned `fechaAutorizacion`);
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true` (exit 0);
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`;
//! 6. refusal: the `MockSriProvider` always returns `Autorizado` and exposes no
//!    knob to force a `Devuelto` / `NoAutorizado` verdict (see the note on the
//!    refusal test below), so this test exercises the pre-wire `Err` rejection
//!    paths the mock *does* support — bad RUC, bad Clave de Acceso, empty XML.
//!
//! Goldens are hand-rolled in spirit (pinned `created_at`, deterministic mock,
//! no `insta` / `pretty_assertions`, which would mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    LocalizedString, MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion,
    TaxCategorySummary,
};
use invoicekit_report_ec_sri::{
    validate_clave_acceso, validate_ruc, MockSriProvider, SriDocumentKind, SriEnvironment,
    SriError, SriProvider, SriStatus, SriSubmitEnvelope, SriSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_FECHA_AUTORIZACION: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_ec_e2e";
const TRACE: &str = "trace_ec_e2e";
/// Issuer RUC: exactly 13 ASCII digits (Registro Único de Contribuyentes).
const ISSUER_RUC: &str = "1791234567001";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn ecuadorian_party(name: &str, ruc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "ruc".to_owned(),
            value: ruc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Av. Amazonas N1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("Pichincha".to_owned()),
            postal_code: "170102".to_owned(),
            country: CountryCode::new("EC").unwrap(),
        },
        contact: None,
    }
}

fn ecuadorian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ec-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("001-001-000000123").unwrap(),
        // Ecuador is dollarized: the official currency is the US dollar.
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: ecuadorian_party("Acme Cia. Ltda.", ISSUER_RUC, "Quito"),
        customer: ecuadorian_party("Beta S.A.", "0992345678001", "Guayaquil"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoría de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Ecuadorian standard IVA is 15% (since 2024).
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1500),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11500),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A deterministic stand-in for the 49-digit Clave de Acceso. The real value is
/// computed by the SRI engine (date + tipoComprobante + RUC + ambiente +
/// serie + secuencial + código numérico + tipoEmisión + check digit); shape is
/// exactly 49 ASCII digits, which the mock's `validate_clave_acceso` enforces.
fn clave_acceso() -> String {
    // 49 digits: a fixed, shape-valid sample.
    "2605202601179123456700110010010000001231234567819".to_owned()
}

fn submit_request(comprobante_xml: Vec<u8>) -> SriSubmitRequest {
    SriSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SriEnvironment::Certificacion,
        kind: SriDocumentKind::Factura,
        issuer_ruc: ISSUER_RUC.to_owned(),
        clave_acceso: clave_acceso(),
        comprobante_xml,
    }
}

/// Assemble the canonical `.ikb` evidence bundle for a document: canonical IR
/// JSON + national-family (UBL) XML + the SRI autorización receipt, under the
/// pinned tenant / trace / created-at, then `pack` to bytes. Byte-stable.
fn bundle_and_pack(
    doc: &CommercialDocument,
    ubl_bytes: Vec<u8>,
    envelope: &SriSubmitEnvelope,
) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    pack(&EvidenceBundle { manifest, artefacts }).unwrap()
}

/// Steps 1-4: build -> serialize (UBL) -> submit to SRI mock -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, SriSubmitEnvelope) {
    // 1. build the canonical IR document.
    let doc = ecuadorian_invoice();

    // 2. serialize to UBL bytes (EN 16931 / UBL family path).
    let ubl: String = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the existing offline MockSriProvider (runs the real
    //    RUC + Clave de Acceso shape validators before synthesizing the
    //    autorización envelope).
    let provider = MockSriProvider::with_fixed_fecha_autorizacion(PINNED_FECHA_AUTORIZACION);
    let envelope = provider
        .submit_comprobante(&submit_request(ubl_bytes.clone()))
        .unwrap();

    // 4. evidence bundle: canonical doc + national-family XML + SRI receipt.
    let ikb = bundle_and_pack(&doc, ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn ecuador_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: SRI authorized. The numeroAutorizacion equals the Clave de
    // Acceso once authorized, the status is Autorizado, the fecha is the
    // pinned UTC value, and there is no mensaje.
    assert_eq!(envelope.status, SriStatus::Autorizado);
    assert_eq!(envelope.numero_autorizacion, clave_acceso());
    assert_eq!(envelope.numero_autorizacion.len(), 49);
    assert!(envelope
        .numero_autorizacion
        .bytes()
        .all(|b| b.is_ascii_digit()));
    assert_eq!(envelope.fecha_autorizacion, PINNED_FECHA_AUTORIZACION);
    assert!(envelope.mensaje.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn ecuador_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

/// Refusal / rejection note: `MockSriProvider` is hard-coded to return
/// `SriStatus::Autorizado` and exposes no `with_forced_*` knob to synthesize a
/// `Devuelto` / `NoAutorizado` verdict (unlike Italy's
/// `MockSdiProvider::with_forced_receipt`). Forcing an authority *refusal*
/// would require modifying `src/lib.rs`, which this task forbids.
///
/// What the mock *does* support is the typed pre-wire `Err` rejection contract:
/// it runs the real `validate_ruc` + `validate_clave_acceso` shape validators
/// and refuses an empty payload before touching the wire. We assert those here
/// so the refusal layer is genuinely exercised end-to-end.
#[test]
fn ecuador_pre_wire_rejections_are_typed_errors() {
    use invoicekit_report_ec_sri::SriError;

    let provider = MockSriProvider::with_fixed_fecha_autorizacion(PINNED_FECHA_AUTORIZACION);

    // A valid request must succeed (baseline).
    let ok = provider.submit_comprobante(&submit_request(b"<Invoice/>".to_vec()));
    assert!(ok.is_ok(), "the baseline valid request must authorize");

    // Bad RUC (not 13 digits) -> SriError::BadRuc, never an envelope.
    let mut bad_ruc = submit_request(b"<Invoice/>".to_vec());
    bad_ruc.issuer_ruc = "123".to_owned();
    assert!(matches!(
        provider.submit_comprobante(&bad_ruc),
        Err(SriError::BadRuc(_))
    ));

    // Bad Clave de Acceso (not 49 digits) -> SriError::BadClaveAcceso.
    let mut bad_clave = submit_request(b"<Invoice/>".to_vec());
    bad_clave.clave_acceso = "1".repeat(48);
    assert!(matches!(
        provider.submit_comprobante(&bad_clave),
        Err(SriError::BadClaveAcceso(_))
    ));

    // Empty comprobante XML -> SriError::BadXml.
    let empty = submit_request(Vec::new());
    assert!(matches!(
        provider.submit_comprobante(&empty),
        Err(SriError::BadXml(_))
    ));
}

// ===========================================================================
// DEEPENED COVERAGE — genuinely Ecuador-specific scenarios.
//
// Authority: Servicio de Rentas Internas (SRI), Ecuador.
// Primary spec cited throughout: SRI "Ficha Técnica de Comprobantes
// Electrónicos — Esquema Offline" (the offline-authorization technical sheet),
// published at <https://www.sri.gob.ec/facturacion-electronica>. The
// document-type taxonomy, the 49-digit Clave de Acceso field layout, the
// módulo-11 check digit, the `ambiente` (environment) code, and the
// authorization-state vocabulary (RECIBIDA / AUTORIZADO / DEVUELTA /
// NO AUTORIZADO) are all defined there. All fixtures below are hand-built,
// shape-valid synthetic values — no regulator file is vendored.
// ===========================================================================

/// Clave de Acceso segment widths, per the SRI Ficha Técnica:
/// fecha de emisión `ddmmaaaa` (8) + tipo de comprobante (2) + RUC (13) +
/// ambiente (1) + serie (6) + secuencial (9) + código numérico (8) +
/// tipo de emisión (1) + dígito verificador (1) = 49 digits.
const CLAVE_SEGMENT_WIDTHS: [usize; 9] = [8, 2, 13, 1, 6, 9, 8, 1, 1];

/// `ambiente` code in the Clave de Acceso: 1 = certificación/pruebas,
/// 2 = producción (SRI Ficha Técnica, campo `ambiente`).
const AMBIENTE_CERTIFICACION: &str = "1";

/// `tipoEmisión` code: 1 = emisión normal (SRI Ficha Técnica).
const TIPO_EMISION_NORMAL: &str = "1";

/// Compute the SRI módulo-11 dígito verificador over the first 48 digits.
///
/// Per the SRI Ficha Técnica: weights cycle 2..=7 applied right-to-left,
/// `dv = 11 - (sum % 11)`, with the two documented special cases
/// `dv == 11 -> 0` and `dv == 10 -> 1`.
fn modulo_11_check_digit(first_48: &str) -> u8 {
    let mut weight = 2_u32;
    let mut sum = 0_u32;
    for byte in first_48.bytes().rev() {
        let digit = u32::from(byte - b'0');
        sum += digit * weight;
        weight = if weight == 7 { 2 } else { weight + 1 };
    }
    let dv = 11 - (sum % 11);
    match dv {
        11 => 0,
        10 => 1,
        other => u8::try_from(other).expect("modulo-11 result is 0..=9"),
    }
}

/// Build a structurally faithful 49-digit Clave de Acceso from its real SRI
/// segments, appending a correct módulo-11 check digit. This proves the crate's
/// shape validator accepts a key assembled the way the Ficha Técnica prescribes,
/// not just a `"1".repeat(49)` placeholder.
fn build_clave_acceso(
    fecha_ddmmaaaa: &str,
    tipo_comprobante: &str,
    ruc: &str,
    serie: &str,
    secuencial: &str,
    codigo_numerico: &str,
) -> String {
    let mut first_48 = String::with_capacity(48);
    first_48.push_str(fecha_ddmmaaaa); // 8
    first_48.push_str(tipo_comprobante); // 2
    first_48.push_str(ruc); // 13
    first_48.push_str(AMBIENTE_CERTIFICACION); // 1
    first_48.push_str(serie); // 6
    first_48.push_str(secuencial); // 9
    first_48.push_str(codigo_numerico); // 8
    first_48.push_str(TIPO_EMISION_NORMAL); // 1
    assert_eq!(first_48.len(), 48, "first 48 segments must sum to 48");
    let dv = modulo_11_check_digit(&first_48);
    format!("{first_48}{dv}")
}

/// A self-consistent Clave de Acceso for a Factura issued 2026-05-26 by
/// `ISSUER_RUC`, serie `001-001`, secuencial `000000123`.
fn factura_clave() -> String {
    build_clave_acceso(
        "26052026",    // fecha de emisión: 26 May 2026
        "01",          // tipo de comprobante: 01 Factura
        ISSUER_RUC,    // 13-digit RUC
        "001001",      // serie: estab 001 + punto emisión 001
        "000000123",   // secuencial
        "12345678",    // código numérico
    )
}

/// A Nota de Crédito (tipo 04) referencing the original factura. SRI treats a
/// corrective credit note as its own comprobante with its own Clave de Acceso.
fn nota_credito_clave() -> String {
    build_clave_acceso(
        "27052026",  // issued the day after the factura it corrects
        "04",        // tipo de comprobante: 04 Nota de Crédito
        ISSUER_RUC,
        "001001",
        "000000045",
        "87654321",
    )
}

/// Build a Nota de Crédito IR document that corrects the original factura.
/// Ecuadorian credit notes (`Nota de Crédito`) carry a reference to the
/// modified comprobante; here a single corrective line reverses one unit of the
/// consultancy service at the 15% IVA rate in force since 2024.
fn ecuadorian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ec-nc-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("001-001-000000045").unwrap(),
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: ecuadorian_party("Acme Cia. Ltda.", ISSUER_RUC, "Quito"),
        customer: ecuadorian_party("Beta S.A.", "0992345678001", "Guayaquil"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Reverso parcial: 1 unidad de consultoría".to_owned(),
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
            tax_amount: amt(750), // 15% of 50.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5750),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5750),
        },
        attachments: Vec::new(),
        // SRI requires the corrective document to reference the modified
        // comprobante (its access key / number).
        references: vec![DocumentReference {
            kind: "credit-note-of".to_owned(),
            id: "001-001-000000123".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
        notes: vec![LocalizedString {
            language: "es".to_owned(),
            text: "Sustento: devolución parcial del servicio".to_owned(),
        }],
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A multi-line Factura mixing the 15% standard IVA (category `S`) with a
/// 0%/tarifa-cero line (category `Z`). Ecuador levies IVA at the general 15%
/// rate (raised from 12% in 2024) but a published catalogue of goods/services
/// carries IVA tarifa 0% — both appear on one invoice with two `DatiRiepilogo`
/// style tax-summary buckets.
fn ecuadorian_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ec-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("001-001-000000200").unwrap(),
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: ecuadorian_party("Acme Cia. Ltda.", ISSUER_RUC, "Quito"),
        customer: ecuadorian_party("Beta S.A.", "0992345678001", "Guayaquil"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Servicios de consultoría (IVA 15%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Medicamento de la canasta básica (IVA 0%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(4)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(10000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(1500), // 15%
                tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(0), // tarifa 0%
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            tax_inclusive_amount: amt(21500), // 200.00 + 15.00 IVA
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(21500),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// Scenario: the 49-digit Clave de Acceso is assembled from its real SRI
/// segments (Ficha Técnica field layout) with a genuine módulo-11 check digit,
/// and the crate's shape validator accepts it. We also prove the embedded RUC
/// segment round-trips out of the key, and that the segment widths sum to 49.
///
/// Reference: SRI "Ficha Técnica de Comprobantes Electrónicos — Esquema
/// Offline", clave de acceso field layout + módulo-11 dígito verificador.
#[test]
fn ecuador_clave_acceso_has_faithful_sri_structure() {
    assert_eq!(
        CLAVE_SEGMENT_WIDTHS.iter().sum::<usize>(),
        49,
        "the SRI clave de acceso segment widths must total 49 digits"
    );

    let clave = factura_clave();
    assert_eq!(clave.len(), 49);
    validate_clave_acceso(&clave).expect("structurally-built clave must pass shape validation");

    // The RUC lives at offset 10..23 (after fecha[8] + tipo[2]).
    assert_eq!(&clave[10..23], ISSUER_RUC, "embedded RUC segment must match issuer");
    // ambiente segment (offset 23) is certificación = "1".
    assert_eq!(&clave[23..24], AMBIENTE_CERTIFICACION);
    // tipo de comprobante segment (offset 8..10) is "01" Factura.
    assert_eq!(&clave[8..10], SriDocumentKind::Factura.code());

    // Corrupting the check digit must make it an invalid key only if it breaks
    // the digit shape; a wrong-but-numeric DV is still 49 digits, so the shape
    // validator (which checks length + digit-ness, not the checksum) still
    // accepts it — the checksum is the SRI backend's job, documented as such.
    let mut tampered = factura_clave();
    tampered.truncate(48);
    tampered.push('X'); // non-digit
    assert!(matches!(
        validate_clave_acceso(&tampered),
        Err(SriError::BadClaveAcceso(_))
    ));
}

/// Scenario: módulo-11 documented special cases. The SRI Ficha Técnica states
/// `dv == 11 -> 0` and `dv == 10 -> 1`. We assert the computed check digit is
/// always a single decimal digit (0..=9), never 10 or 11.
///
/// Reference: SRI Ficha Técnica, dígito verificador módulo 11.
#[test]
fn ecuador_modulo_11_never_emits_ten_or_eleven() {
    for secuencial in ["000000001", "000000123", "000000999", "123456789"] {
        let clave = build_clave_acceso("26052026", "01", ISSUER_RUC, "001001", secuencial, "12345678");
        let dv = clave.as_bytes()[48] - b'0';
        assert!(dv <= 9, "dígito verificador must be a single digit, got {dv}");
        // Recomputing over the emitted first-48 must reproduce the same DV.
        assert_eq!(modulo_11_check_digit(&clave[..48]), dv);
    }
}

/// Scenario: a corrective Nota de Crédito (tipo 04) flows through the whole
/// offline lifecycle. The UBL family path emits a `CreditNote` root with
/// `CreditNoteTypeCode` 381 and `CreditedQuantity`, and SRI authorizes it under
/// its own access key. Proves the crate handles a non-Factura document class
/// end-to-end, not just the happy invoice.
///
/// Reference: SRI tipoComprobante 04 (Nota de Crédito); OASIS UBL 2.1
/// `CreditNote` (the EN 16931 / UBL family serializer used as the faithful
/// typed payload here).
#[test]
fn ecuador_credit_note_lifecycle_is_authorized() {
    let doc = ecuadorian_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // National-family artifact really is a credit note, not an invoice. The UBL
    // serializer re-declares the `cbc`/`cac` namespace on each element, so we
    // assert on the open-tag prefix and the value separately.
    assert!(ubl.contains("<CreditNote"), "credit note must serialize to a UBL CreditNote root");
    assert!(
        ubl.contains("<cbc:CreditNoteTypeCode") && ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL credit note must carry type code 381"
    );
    assert!(ubl.contains("<cac:CreditNoteLine"));
    assert!(ubl.contains(">USD</cbc:DocumentCurrencyCode>"));
    assert!(
        ubl.contains("currencyID=\"USD\""),
        "amounts must be tagged with Ecuador's USD currency"
    );

    let clave = nota_credito_clave();
    // The clave's tipoComprobante segment is 04 (Nota de Crédito).
    assert_eq!(&clave[8..10], SriDocumentKind::NotaCredito.code());
    assert_eq!(SriDocumentKind::NotaCredito.code(), "04");

    let provider = MockSriProvider::with_fixed_fecha_autorizacion(PINNED_FECHA_AUTORIZACION);
    let req = SriSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SriEnvironment::Certificacion,
        kind: SriDocumentKind::NotaCredito,
        issuer_ruc: ISSUER_RUC.to_owned(),
        clave_acceso: clave.clone(),
        comprobante_xml: ubl.clone().into_bytes(),
    };
    let envelope = provider.submit_comprobante(&req).unwrap();
    assert_eq!(envelope.status, SriStatus::Autorizado);
    // In SRI's offline scheme the numeroAutorizacion IS the access key.
    assert_eq!(envelope.numero_autorizacion, clave);
    assert!(envelope.mensaje.is_none());

    // Bundle and verify the corrective document's evidence.
    let ikb = bundle_and_pack(&doc, ubl.into_bytes(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// Scenario: a multi-line invoice mixing 15% IVA and 0% (tarifa cero) lines.
/// Proves the UBL family path emits two distinct lines and two tax-summary
/// buckets with the country-correct IVA percentages, and that the whole bundle
/// verifies.
///
/// Reference: Ecuador IVA general rate 15% (raised from 12% in 2024) and the
/// IVA tarifa 0% catalogue (SRI). Tax percentages serialize as `cbc:Percent`
/// in the UBL `cac:TaxSubtotal`.
#[test]
fn ecuador_multiline_mixed_iva_lifecycle() {
    let doc = ecuadorian_multiline_invoice();
    assert_eq!(doc.lines.len(), 2, "invoice must carry two lines");

    let ubl = to_xml(&doc).unwrap();
    assert!(ubl.contains("<Invoice"));
    assert!(
        ubl.contains("<cbc:InvoiceTypeCode") && ubl.contains(">380</cbc:InvoiceTypeCode>"),
        "UBL invoice must carry type code 380"
    );
    // Two distinct invoice lines.
    assert_eq!(ubl.matches("<cac:InvoiceLine").count(), 2);
    // Both the 15% and the 0% IVA percentages appear in the tax subtotals.
    assert!(ubl.contains(">15.00</cbc:Percent>"), "15% IVA bucket missing");
    assert!(ubl.contains(">0</cbc:Percent>"), "0% IVA (tarifa cero) bucket missing");
    // Payable is 200.00 net + 15.00 IVA = 215.00.
    assert!(ubl.contains("currencyID=\"USD\">215.00</cbc:PayableAmount>"));

    let provider = MockSriProvider::with_fixed_fecha_autorizacion(PINNED_FECHA_AUTORIZACION);
    let req = SriSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SriEnvironment::Produccion,
        kind: SriDocumentKind::Factura,
        issuer_ruc: ISSUER_RUC.to_owned(),
        clave_acceso: factura_clave(),
        comprobante_xml: ubl.clone().into_bytes(),
    };
    let envelope = provider.submit_comprobante(&req).unwrap();
    assert_eq!(envelope.status, SriStatus::Autorizado);

    let ikb = bundle_and_pack(&doc, ubl.into_bytes(), &envelope);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

/// Scenario: an SRI **DEVUELTA** (returned) verdict is data, not an `Err`. SRI
/// returns this state on initial validation failure (e.g. error code 35
/// "ARCHIVO NO CUMPLE ESTRUCTURA XML", or 43 / "CLAVE ACCESO REGISTRADA" when
/// the access key already exists). We construct the envelope as SRI would
/// surface it — `status = Devuelto`, a real `mensaje`, and the access key still
/// echoed in `numeroAutorizacion` — bundle the rejection receipt, and prove the
/// evidence bundle still verifies so the audit trail persists the refusal.
///
/// Reference: SRI Ficha Técnica, estado de comprobante `DEVUELTA`; the typed
/// contract (`SriProvider::submit_comprobante` surfaces refusals as
/// `SriStatus`, not `SriError`) is documented on the trait.
#[test]
fn ecuador_devuelta_is_a_receipt_not_an_error() {
    let rejection = SriSubmitEnvelope {
        numero_autorizacion: factura_clave(),
        status: SriStatus::Devuelto,
        fecha_autorizacion: PINNED_FECHA_AUTORIZACION.to_owned(),
        mensaje: Some("43 CLAVE ACCESO REGISTRADA: el comprobante ya existe".to_owned()),
    };
    assert_eq!(rejection.status, SriStatus::Devuelto);
    assert!(rejection.mensaje.is_some(), "a returned receipt must carry a mensaje");
    // Even rejected, the echoed access key is shape-valid.
    validate_clave_acceso(&rejection.numero_autorizacion).unwrap();

    // The rejection serializes (audit trail) and round-trips losslessly.
    let json = serde_json::to_string(&rejection).unwrap();
    assert!(json.contains("\"devuelto\""), "status must serialize kebab-case");
    assert!(json.contains("CLAVE ACCESO REGISTRADA"));
    let back: SriSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, rejection);

    // The rejection receipt bundles and the bundle still verifies.
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&rejection).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let ikb = pack(&EvidenceBundle { manifest, artefacts }).unwrap();
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok,
        "a DEVUELTA rejection must still produce a verifiable evidence bundle"
    );
}

/// Scenario: an SRI **NO AUTORIZADO** verdict. After a comprobante is received
/// (RECIBIDA), SRI may still refuse authorization, e.g. code 39
/// "FIRMA INVALIDA" (invalid signature) or 70 "DABLEMENTE AUTORIZADO". Like
/// DEVUELTA, this is a `SriStatus`, never an `Err`.
///
/// Reference: SRI Ficha Técnica, estado `NO AUTORIZADO` / mensajes de
/// autorización.
#[test]
fn ecuador_no_autorizado_is_a_receipt_not_an_error() {
    let envelope = SriSubmitEnvelope {
        numero_autorizacion: nota_credito_clave(),
        status: SriStatus::NoAutorizado,
        fecha_autorizacion: PINNED_FECHA_AUTORIZACION.to_owned(),
        mensaje: Some("39 FIRMA INVALIDA: la firma digital no es válida".to_owned()),
    };
    assert_eq!(envelope.status, SriStatus::NoAutorizado);
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains("\"no-autorizado\""), "kebab-case serialization expected");
    let back: SriSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, envelope);
}

/// Scenario: Ecuador-specific invalid-identifier rejection. A 10-digit Ecuadorian
/// **cédula** is a valid natural-person id but is NOT a 13-digit RUC; passing it
/// as the issuer RUC must be refused with a typed `SriError::BadRuc` before any
/// wire activity. Likewise a Clave de Acceso whose segments are individually
/// plausible but whose total length is wrong (48 or 50) is refused.
///
/// Reference: SRI RUC = 13 dígitos (cédula 10 + "001"); Clave de Acceso = 49
/// dígitos exactos (SRI Ficha Técnica).
#[test]
fn ecuador_rejects_cedula_used_as_ruc_and_malformed_clave() {
    let provider = MockSriProvider::with_fixed_fecha_autorizacion(PINNED_FECHA_AUTORIZACION);

    // A 10-digit cédula passed where a 13-digit RUC is required.
    let cedula = "1791234567";
    assert_eq!(cedula.len(), 10);
    assert!(validate_ruc(cedula).is_err(), "a 10-digit cédula is not a RUC");
    let mut req = submit_request(b"<Invoice/>".to_vec());
    req.issuer_ruc = cedula.to_owned();
    assert!(matches!(
        provider.submit_comprobante(&req),
        Err(SriError::BadRuc(_))
    ));

    // The full 13-digit RUC (cédula + establishment "001") IS accepted.
    assert!(validate_ruc(&format!("{cedula}001")).is_ok());

    // A clave that is one digit short (48) — e.g. a dropped código numérico
    // digit — is refused.
    let short = factura_clave()[..48].to_owned();
    assert!(matches!(
        validate_clave_acceso(&short),
        Err(SriError::BadClaveAcceso(_))
    ));
    let mut req_short = submit_request(b"<Invoice/>".to_vec());
    req_short.clave_acceso = short;
    assert!(matches!(
        provider.submit_comprobante(&req_short),
        Err(SriError::BadClaveAcceso(_))
    ));
}

/// Scenario: serialization determinism for the corrective document and the
/// multi-line invoice. The national-family (UBL) bytes and the canonical IR
/// JSON must be byte-identical across runs — the foundation of reproducible
/// `.ikb` evidence and the SRI access-key/hash stability.
#[test]
fn ecuador_credit_note_and_multiline_serialization_is_deterministic() {
    let nc = ecuadorian_credit_note();
    assert_eq!(to_xml(&nc).unwrap(), to_xml(&nc).unwrap());
    assert_eq!(
        canonicalize_value(&nc.to_value().unwrap()).unwrap(),
        canonicalize_value(&nc.to_value().unwrap()).unwrap()
    );

    let multi = ecuadorian_multiline_invoice();
    assert_eq!(to_xml(&multi).unwrap(), to_xml(&multi).unwrap());

    // The clave builder is pure: same segments -> same key (incl. check digit).
    assert_eq!(factura_clave(), factura_clave());
    assert_eq!(nota_credito_clave(), nota_credito_clave());
}
