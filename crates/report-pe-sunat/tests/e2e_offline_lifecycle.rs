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
            extensions: Vec::new(),
        }],
        // IGV (Impuesto General a las Ventas) at 18%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1800),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
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

fn submit_request(invoice_xml: Vec<u8>) -> SunatSubmitRequest {
    SunatSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SunatEnvironment::Beta,
        kind: SunatDocumentKind::Factura,
        issuer_ruc: ISSUER_RUC.to_owned(),
        document_id: DOCUMENT_ID.to_owned(),
        invoice_xml,
    }
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
    let envelope = provider.submit_document(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical IR doc + national UBL + SUNAT receipt.
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

    let mut req = submit_request(ubl_bytes);
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

    let mut req = submit_request(ubl_bytes);
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
