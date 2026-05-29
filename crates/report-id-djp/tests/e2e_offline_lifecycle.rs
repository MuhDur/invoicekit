// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Indonesia DJP e-Faktur offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Indonesia and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("ID")` + IDR
//! 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path)
//! 3. submit those bytes to the crate's existing `MockDjpProvider`, asserting the
//!    DJP-specific receipt fields (nomor referensi, echoed NSFP, `Approved` status)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the DJP mock validates NPWP / NSFP / payload-shape before the wire
//!    and returns `Err` on a malformed request
//!
//! Note on the authority `Rejected` verdict: `MockDjpProvider` always returns the
//! `Approved` envelope (it has no `with_forced_*` knob), so a forced DJP-side
//! rejection cannot be exercised here. The refusal test instead drives the three
//! real pre-wire validations the mock performs (NPWP shape, NSFP shape, empty
//! payload), which is the refusal surface this adapter actually exposes.
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would mutate
//! `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_id_djp::{
    DjpEnvironment, DjpError, DjpStatus, DjpSubmitRequest, DjpProvider, FakturKodeJenis,
    MockDjpProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_id_e2e";
const TRACE: &str = "trace_id_e2e";
// 16-digit issuer NPWP (PMK 112/2022 shape) and 16-digit NSFP.
const ISSUER_NPWP: &str = "0123456789012345";
const NSFP: &str = "0100002400000001";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn indonesian_party(name: &str, npwp: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "npwp".to_owned(),
            value: npwp.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Jalan Sudirman 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("DKI Jakarta".to_owned()),
            postal_code: "10220".to_owned(),
            country: CountryCode::new("ID").unwrap(),
        },
        contact: None,
    }
}

fn indonesian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-id-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ID-0001").unwrap(),
        currency: Iso4217Code::new("IDR").unwrap(),
        supplier: indonesian_party("Acme Indonesia PT", ISSUER_NPWP, "Jakarta"),
        customer: indonesian_party("Beta Nusantara PT", "9876543210987654", "Bandung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Jasa konsultasi & pengembangan perangkat lunak".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5_000_000),
            line_extension_amount: amt(10_000_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // PPN (VAT) at 11%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10_000_000),
            tax_amount: amt(1_100_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1100, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10_000_000),
            tax_exclusive_amount: amt(10_000_000),
            tax_inclusive_amount: amt(11_100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11_100_000),
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

fn submit_request(faktur_xml: Vec<u8>) -> DjpSubmitRequest {
    DjpSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DjpEnvironment::Uat,
        kode_jenis: FakturKodeJenis::Standard,
        issuer_npwp: ISSUER_NPWP.to_owned(),
        nsfp: NSFP.to_owned(),
        faktur_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to DJP mock -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the DJP receipt envelope so callers can
/// assert both the country-specific authority artifacts and bundle verification.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_id_djp::DjpSubmitEnvelope) {
    // 1. build the canonical IR document.
    let doc = indonesian_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (EN 16931 / UBL family path).
    let ubl = to_xml(&doc).unwrap();
    // Structural sanity: the canonical artifact carries the UBL spine. The
    // canonicalizer attaches namespace declarations per-element, so match the
    // element open-tag prefix (not a bare `<tag>`) plus the load-bearing content.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cac:AccountingSupplierParty ",
        "<cac:AccountingCustomerParty ",
        ">IDR</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the DJP mock (runs NPWP + NSFP + payload validation on the way).
    let provider = MockDjpProvider::new();
    let envelope = provider.submit_faktur(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national XML + DJP receipt.
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
fn indonesia_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Country-specific authority artifacts: DJP nomor referensi + echoed NSFP.
    assert_eq!(envelope.status, DjpStatus::Approved);
    assert!(
        envelope.nomor_referensi.starts_with("DJP-"),
        "expected DJP-prefixed nomor referensi, got {:?}",
        envelope.nomor_referensi
    );
    assert_eq!(envelope.nsfp, NSFP, "DJP must echo the submitted NSFP");
    assert_eq!(envelope.submitted_at, "2026-01-01T00:00:00Z");
    assert!(
        envelope.alasan.is_none(),
        "an Approved envelope carries no rejection reason"
    );

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn indonesia_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn indonesia_prewire_refusals_are_errors_not_receipts() {
    // The DJP mock has no forced-rejection knob (it always returns Approved), so
    // the authority `Rejected` verdict cannot be exercised. What IS exercised is
    // the genuine pre-wire validation the mock performs before the wire: a
    // malformed NPWP, a malformed NSFP, and an empty Faktur payload each surface
    // as a typed `Err`, never as a (would-be) Approved receipt.
    let provider = MockDjpProvider::new();
    let ubl_bytes = to_xml(&indonesian_invoice()).unwrap().into_bytes();

    // (a) bad NPWP (not 15/16 digits) -> BadNpwp.
    let mut bad_npwp = submit_request(ubl_bytes.clone());
    bad_npwp.issuer_npwp = "NOT-DIGITS".to_owned();
    assert!(matches!(
        provider.submit_faktur(&bad_npwp).unwrap_err(),
        DjpError::BadNpwp(_)
    ));

    // (b) bad NSFP (not 16 digits) -> BadNsfp.
    let mut bad_nsfp = submit_request(ubl_bytes);
    bad_nsfp.nsfp = "TOO-SHORT".to_owned();
    assert!(matches!(
        provider.submit_faktur(&bad_nsfp).unwrap_err(),
        DjpError::BadNsfp(_)
    ));

    // (c) empty payload -> BadXml.
    let mut empty = submit_request(Vec::new());
    empty.faktur_xml.clear();
    assert!(matches!(
        provider.submit_faktur(&empty).unwrap_err(),
        DjpError::BadXml(_)
    ));
}
