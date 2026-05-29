// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Vietnam GDT offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Vietnam and proves it
//! deterministically, following the proven `report-it-sdi` pattern:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("VN")`
//!    and the đồng (`VND`, a zero-decimal ISO 4217 currency)
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path; `report-vn-gdt` exposes no
//!    serializer of its own)
//! 3. submit the bytes to the crate's existing `MockGdtProvider` and
//!    assert the GDT authority receipt's country-specific fields
//!    (`mã CQT` / `ma_cqt`, status, timestamp)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true` (exit 0 == report.ok)
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal paths that the mock DOES support (bad MST, empty payload)
//!    surface as `Err`, never a bundle
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would
//! mutate `Cargo.lock`).
//!
//! NOTE ON FORCED REJECTION: the `MockGdtProvider` cannot be forced into a
//! `GdtStatus::Rejected` envelope — on a valid request it always clears
//! (`Cleared`), and the only refusal it models is pre-wire `Err`
//! (`GdtError::BadMst` / `GdtError::BadXml`). So the rejection test below
//! exercises those `Err` paths rather than an authority `Rejected` verdict.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_vn_gdt::{
    GdtEnvironment, GdtError, GdtStatus, GdtSubmitRequest, GdtProvider, MockGdtProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_vn_e2e";
const TRACE: &str = "trace_vn_e2e";
/// Issuer mã số thuế (MST): 10 ASCII digits — the shape `validate_mst` enforces.
const ISSUER_MST: &str = "0312345678";

/// VND is a zero-decimal currency, so amounts are whole đồng (scale 0).
fn dong(value: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(value, 0))
}

fn vietnamese_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["12 Nguyễn Huệ".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "700000".to_owned(),
            country: CountryCode::new("VN").unwrap(),
        },
        contact: None,
    }
}

fn vietnamese_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-vn-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-VN-0001").unwrap(),
        currency: Iso4217Code::new("VND").unwrap(),
        supplier: vietnamese_party("Acme Vietnam Co Ltd", "0312345678", "Ho Chi Minh City"),
        customer: vietnamese_party("Beta Trading JSC", "0398765432", "Ha Noi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Dịch vụ tư vấn phần mềm".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: dong(5_000_000),
            line_extension_amount: dong(10_000_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Vietnam standard VAT is 10%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: dong(10_000_000),
            tax_amount: dong(1_000_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: dong(10_000_000),
            tax_exclusive_amount: dong(10_000_000),
            tax_inclusive_amount: dong(11_000_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: dong(11_000_000),
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

fn submit_request(invoice_xml: Vec<u8>) -> GdtSubmitRequest {
    GdtSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: GdtEnvironment::Sandbox,
        issuer_mst: ISSUER_MST.to_owned(),
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize -> submit(GDT mock) -> evidence `.ikb`.
fn run_lifecycle() -> Vec<u8> {
    // 1. build
    let doc = vietnamese_invoice();

    // 2. serialize -> UBL 2.1 (the EN 16931 / UBL family path).
    let ubl = to_xml(&doc).unwrap();
    // local validate (structural): the UBL spine must carry the document core.
    // The canonicalizer may hoist namespace declarations onto the first element
    // that introduces a prefix, so match on element-open prefixes (no trailing
    // `>`) rather than fully-closed start tags.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">VND</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the existing MockGdtProvider; assert the GDT receipt.
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();
    assert_eq!(envelope.status, GdtStatus::Cleared);
    assert!(
        envelope.ma_cqt.starts_with("VN-"),
        "mã CQT must carry the country-tagged prefix, got {:?}",
        envelope.ma_cqt
    );
    assert_eq!(envelope.recorded_at, PINNED_CREATED_AT);
    assert!(envelope.message.is_none());

    // 4. evidence bundle: canonical doc + national UBL XML + GDT receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

#[test]
fn vietnam_offline_lifecycle_produces_verifiable_evidence() {
    let ikb = run_lifecycle();

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn vietnam_lifecycle_is_byte_deterministic() {
    let a = run_lifecycle();
    let b = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn vietnam_refusal_paths_are_errors_not_bundles() {
    // The MockGdtProvider cannot be forced into a GDT-side `Rejected` verdict
    // (a valid request always clears). The refusals it DOES model are pre-wire
    // `Err`: a malformed MST and an empty payload. Both must fail before any
    // receipt — and therefore before any evidence bundle — is produced.
    let doc = vietnamese_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);

    // Bad MST (not 10 / 13 ASCII digits) -> GdtError::BadMst.
    let mut bad_mst = submit_request(ubl_bytes);
    bad_mst.issuer_mst = "NOT-A-MST".to_owned();
    assert!(matches!(
        provider.submit_invoice(&bad_mst).unwrap_err(),
        GdtError::BadMst(_)
    ));

    // Empty payload -> GdtError::BadXml.
    let empty = submit_request(Vec::new());
    assert!(matches!(
        provider.submit_invoice(&empty).unwrap_err(),
        GdtError::BadXml(_)
    ));
}
