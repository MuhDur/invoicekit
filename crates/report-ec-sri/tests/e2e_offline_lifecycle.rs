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
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ec_sri::{
    MockSriProvider, SriDocumentKind, SriEnvironment, SriProvider, SriStatus, SriSubmitEnvelope,
    SriSubmitRequest,
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
            extensions: Vec::new(),
        }],
        // Ecuadorian standard IVA is 15% (since 2024).
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1500),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
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
