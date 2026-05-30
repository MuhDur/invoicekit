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
    GdtEnvironment, GdtError, GdtProvider, GdtStatus, GdtSubmitEnvelope, GdtSubmitRequest,
    MockGdtProvider,
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

/// The fixed document metadata every VN fixture carries (pinned tenant/trace so
/// the canonical bytes stay deterministic).
fn vn_meta() -> DocumentMeta {
    DocumentMeta {
        tenant_id: TENANT.to_owned(),
        trace_id: TRACE.to_owned(),
        source_system: Some("e2e".to_owned()),
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
        invoice_period: None,
        delivery_date: None,
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
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Vietnam standard VAT is 10%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: dong(10_000_000),
            tax_amount: dong(1_000_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
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
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: vn_meta(),
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
    // 1. build, then 2-4 via the shared driver (serialize -> submit -> `.ikb`).
    let doc = vietnamese_invoice();
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let (ikb, ubl, envelope) = bundle_for(&doc, &provider);

    // 2. structural UBL spine check: the document core must be present.
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

    // 3. assert the GDT receipt.
    assert_eq!(envelope.status, GdtStatus::Cleared);
    assert!(
        envelope.ma_cqt.starts_with("VN-"),
        "mã CQT must carry the country-tagged prefix, got {:?}",
        envelope.ma_cqt
    );
    assert_eq!(envelope.recorded_at, PINNED_CREATED_AT);
    assert!(envelope.message.is_none());

    ikb
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

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios (added on top of the §1 honest bar;
// existing tests above are unchanged).
//
// Each scenario grounds its assertions in Vietnam's real e-invoice rules. The
// mandatory nationwide e-invoice regime took effect 2022-07-01 and is run by
// the General Department of Taxation (GDT / Tổng cục Thuế) through the official
// portal `hoadondientu.gdt.gov.vn`:
//
//   * Decree 123/2020/ND-CP (Nghị định 123/2020/NĐ-CP) — the e-invoice &
//     records framework, https://www.gdt.gov.vn/
//   * Circular 78/2021/TT-BTC (Thông tư 78/2021/TT-BTC) — the GDT technical
//     implementation (the `mã CQT` tax-authority code on cleared invoices, the
//     adjustment/replacement-invoice mechanism), https://www.gdt.gov.vn/
//   * VAT rates — Law on Value-Added Tax 13/2008/QH12 and amendments: the 10%
//     standard rate, the 5% reduced rate (e.g. clean water, medical equipment,
//     teaching aids), and the 0% rate for exported goods/services.
//   * The InvoiceKit capability matrix advertises the VN->VN route from
//     2022-07-01 (crates/cli/data/capabilities/matrix.json, source
//     "GDT e-invoice", https://www.gdt.gov.vn/).
//
// Fixtures are hand-built/synthetic — no copyrighted GDT files are vendored.
// The national wire format is UBL 2.1 (the EN 16931 / UBL family path the VN
// capability-matrix entry pins via the `ubl-2.1` profile); `report-vn-gdt`
// exposes no serializer of its own.
// ---------------------------------------------------------------------------

/// Steps 2-4 for an arbitrary document: serialize -> submit(GDT mock) ->
/// `.ikb`, reusing the pinned timestamps so output stays byte-stable. The
/// `provider` is supplied so a caller can force a `Rejected` verdict. Returns
/// `(ikb, ubl_xml, envelope)`.
fn bundle_for(doc: &CommercialDocument, provider: &MockGdtProvider) -> (Vec<u8>, String, GdtSubmitEnvelope) {
    let ubl = to_xml(doc).unwrap();
    let ubl_bytes = ubl.clone().into_bytes();
    let envelope = provider
        .submit_invoice(&submit_request(ubl_bytes.clone()))
        .unwrap();

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
    (ikb, ubl, envelope)
}

/// A Vietnamese adjustment **credit note** (`hóa đơn điều chỉnh giảm`) that
/// reverses part of an earlier invoice. Circular 78/2021/TT-BTC provides for
/// adjustment/replacement invoices as the corrective mechanism. Through the
/// UBL 2.1 family path a credit note serializes with a `<CreditNote>` root and
/// `<cbc:CreditNoteTypeCode>381</cbc:CreditNoteTypeCode>` (UBL/EN 16931 code
/// 381), each line wrapped in `<cac:CreditNoteLine>` with `CreditedQuantity` —
/// NOT the invoice's `<cac:InvoiceLine>` / code 380.
fn vietnamese_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-vn-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("ADJ-2026-VN-0001").unwrap(),
        currency: Iso4217Code::new("VND").unwrap(),
        supplier: vietnamese_party("Acme Vietnam Co Ltd", "0312345678", "Ho Chi Minh City"),
        customer: vietnamese_party("Beta Trading JSC", "0398765432", "Ha Noi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Điều chỉnh giảm dịch vụ tư vấn".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: dong(5_000_000),
            line_extension_amount: dong(5_000_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Same 10% standard band as the original, on the reversed base.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: dong(5_000_000),
            tax_amount: dong(500_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: dong(5_000_000),
            tax_exclusive_amount: dong(5_000_000),
            tax_inclusive_amount: dong(5_500_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: dong(5_500_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: vn_meta(),
    })
    .unwrap()
}

/// A two-line invoice mixing Vietnam's 10% standard VAT rate (category `S`)
/// with the 5% reduced rate (category `AA`). Vietnam's Law on VAT applies a
/// reduced 5% rate to defined essentials (clean water, medical equipment,
/// teaching aids, agricultural inputs) alongside the 10% standard rate. The
/// UBL serializer emits one line container per line and one `<cac:TaxSubtotal>`
/// per VAT band, each carrying its own `<cbc:Percent>`.
fn vietnamese_multiline_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-vn-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-VN-0002").unwrap(),
        currency: Iso4217Code::new("VND").unwrap(),
        supplier: vietnamese_party("Acme Vietnam Co Ltd", "0312345678", "Ho Chi Minh City"),
        customer: vietnamese_party("Beta Trading JSC", "0398765432", "Ha Noi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Dịch vụ tư vấn (thuế suất 10%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: dong(5_000_000),
                line_extension_amount: dong(10_000_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Thiết bị y tế (thuế suất 5%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: dong(20_000_000),
                line_extension_amount: dong(20_000_000),
                tax_category: Some("AA".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: dong(10_000_000),
                tax_amount: dong(1_000_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "AA".to_owned(),
                taxable_amount: dong(20_000_000),
                tax_amount: dong(1_000_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: dong(30_000_000),
            tax_exclusive_amount: dong(30_000_000),
            // 10m*10% + 20m*5% = 1m + 1m = 2m VAT.
            tax_inclusive_amount: dong(32_000_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: dong(32_000_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: vn_meta(),
    })
    .unwrap()
}

/// A zero-rated **export** invoice. Vietnam's Law on VAT applies a 0% rate to
/// exported goods and services; the supplier charges no output VAT but the
/// supply is still a taxable (not exempt) transaction. The UBL `Percent`
/// renders `0.00` and the supplier's output VAT is zero, so the taxable base
/// equals the payable total.
fn vietnamese_export_zero_rated_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-vn-e2e-export-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-VN-0003").unwrap(),
        currency: Iso4217Code::new("VND").unwrap(),
        supplier: vietnamese_party("Acme Vietnam Co Ltd", "0312345678", "Ho Chi Minh City"),
        customer: vietnamese_party("Beta Trading JSC", "0398765432", "Ha Noi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Hàng xuất khẩu (thuế suất 0%)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: dong(100_000_000),
            line_extension_amount: dong(100_000_000),
            // "Z" is the local zero-rated tax category here; the serializer
            // resolves the 0.00 Percent from the matching summary entry.
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: dong(100_000_000),
            tax_amount: dong(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: dong(100_000_000),
            tax_exclusive_amount: dong(100_000_000),
            tax_inclusive_amount: dong(100_000_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: dong(100_000_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: vn_meta(),
    })
    .unwrap()
}

/// An adjustment credit note must serialize through the UBL `CreditNote` path
/// (root `<CreditNote>`, `CreditNoteTypeCode` 381, `cac:CreditNoteLine` with
/// `CreditedQuantity`) — never the invoice's `InvoiceTypeCode` 380 /
/// `cac:InvoiceLine`. Circular 78/2021/TT-BTC provides for adjustment invoices
/// as Vietnam's corrective mechanism. The whole offline lifecycle must still
/// clear at the GDT and produce a verifiable evidence bundle.
#[test]
fn vietnam_credit_note_serializes_as_ubl_creditnote_and_bundles() {
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let doc = vietnamese_credit_note();
    let (ikb, ubl, envelope) = bundle_for(&doc, &provider);

    // The canonicalizer hoists namespace declarations onto the element that
    // introduces a prefix, so element-open tags carry an `xmlns:*` attribute;
    // match on close tags / text-with-close patterns, which stay clean.
    assert!(ubl.contains("<CreditNote"), "credit note must use the UBL CreditNote root:\n{ubl}");
    assert!(
        ubl.contains("381</cbc:CreditNoteTypeCode>"),
        "a credit note must carry UBL code 381, got:\n{ubl}"
    );
    assert!(
        ubl.contains("</cac:CreditNoteLine>"),
        "credit-note lines must be CreditNoteLine, not InvoiceLine"
    );
    assert!(
        !ubl.contains("</cbc:InvoiceTypeCode>"),
        "a credit note must not carry the invoice type code (InvoiceTypeCode)"
    );
    // The adjustment number and the reversed 10% base (đồng is scale-0, so the
    // amount renders as a bare integer with no decimal places).
    assert!(ubl.contains(">ADJ-2026-VN-0001</cbc:ID>"));
    assert!(ubl.contains(r#"currencyID="VND">5000000</cbc:TaxableAmount>"#));
    assert!(ubl.contains(">10.00</cbc:Percent>"));

    assert_eq!(envelope.status, GdtStatus::Cleared);
    assert!(envelope.ma_cqt.starts_with("VN-"));
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// A two-line invoice carrying both Vietnam's 10% standard rate and 5% reduced
/// rate. The UBL serializer emits one `<cac:TaxSubtotal>` per VAT band, each
/// with its own `<cbc:Percent>`; both lines appear in document order. The 5%
/// reduced rate is a real Vietnamese rate (Law on VAT) applied to defined
/// essentials such as medical equipment.
#[test]
fn vietnam_multiline_invoice_emits_per_band_subtotals() {
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let doc = vietnamese_multiline_mixed_rate_invoice();
    let (ikb, ubl, envelope) = bundle_for(&doc, &provider);

    // Two line items in document order. (Close tags stay clean; the
    // canonicalizer's hoisted namespace attributes only touch open tags.)
    assert!(ubl.contains("Dịch vụ tư vấn (thuế suất 10%)"));
    assert!(ubl.contains("Thiết bị y tế (thuế suất 5%)"));
    assert_eq!(
        ubl.matches("</cac:InvoiceLine>").count(),
        2,
        "a two-line invoice must emit two InvoiceLine blocks"
    );

    // One TaxSubtotal per VAT band, each with its own Percent.
    assert_eq!(
        ubl.matches("</cac:TaxSubtotal>").count(),
        2,
        "a mixed-rate invoice must emit one TaxSubtotal per VAT band"
    );
    assert!(ubl.contains(">10.00</cbc:Percent>"));
    assert!(ubl.contains(">5.00</cbc:Percent>"));
    // The 5% band's taxable base (đồng, scale-0).
    assert!(ubl.contains(r#"currencyID="VND">20000000</cbc:TaxableAmount>"#));

    assert_eq!(envelope.status, GdtStatus::Cleared);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

/// A zero-rated export invoice: `<cbc:Percent>0.00</cbc:Percent>` and zero
/// output VAT, with the taxable base equal to the payable total. Vietnam's
/// Law on VAT applies the 0% rate to exported goods/services. The 10% band
/// from the standard invoice must not appear.
#[test]
fn vietnam_export_invoice_is_zero_rated() {
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let doc = vietnamese_export_zero_rated_invoice();
    let (ikb, ubl, envelope) = bundle_for(&doc, &provider);

    assert!(
        ubl.contains(">0.00</cbc:Percent>"),
        "an export line must carry a 0.00 Percent, got:\n{ubl}"
    );
    assert!(
        ubl.contains(r#"currencyID="VND">0</cbc:TaxAmount>"#),
        "zero-rated export means zero output VAT"
    );
    // Taxable base equals the payable total (no VAT added).
    assert!(ubl.contains(r#"currencyID="VND">100000000</cbc:TaxableAmount>"#));
    assert!(ubl.contains(r#"currencyID="VND">100000000</cbc:PayableAmount>"#));
    // The 10% standard band must NOT appear on a pure export invoice.
    assert!(!ubl.contains(">10.00</cbc:Percent>"));

    assert_eq!(envelope.status, GdtStatus::Cleared);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "export-invoice evidence bundle must verify");
}

/// GDT authority **rejection** (`Bị từ chối`) is a verdict, NOT an `Err`. When
/// the portal refuses a submission it returns a `thông báo` (reason) and issues
/// NO `mã CQT` (the tax-authority code is granted only on clearance). The
/// contract (Decree 123/2020/ND-CP audit-trail logic, mirrored across every
/// InvoiceKit adapter): the refusal is surfaced inside an `Ok` envelope with
/// `GdtStatus::Rejected`, the audit trail still persists it, and its evidence
/// bundle must still verify.
#[test]
fn vietnam_authority_rejection_is_receipt_not_error_and_bundles() {
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT)
        .with_rejection("Mã số thuế người bán không còn hoạt động");
    let doc = vietnamese_invoice();
    let (ikb, _ubl, envelope) = bundle_for(&doc, &provider);

    assert_eq!(
        envelope.status,
        GdtStatus::Rejected,
        "a refused submission must surface GdtStatus::Rejected"
    );
    assert!(
        envelope.ma_cqt.is_empty(),
        "a rejected invoice gets NO mã CQT (the code is issued only on clearance), got {:?}",
        envelope.ma_cqt
    );
    assert_eq!(
        envelope.message.as_deref(),
        Some("Mã số thuế người bán không còn hoạt động"),
        "a rejection must carry the GDT thông báo reason text"
    );

    // The rejection still produces a verifiable audit bundle.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejected-submission evidence bundle must still verify");
}

/// A 13-character issuer MST whose final (branch-suffix) characters are not
/// ASCII digits must be refused **pre-wire** as `GdtError::BadMst`, never reach
/// the GDT. The MST is 10 digits for a head office, or 13 for a branch unit
/// (10 + a 3-digit suffix) per the Vietnamese tax-code rules; `validate_mst`
/// enforces exactly that shape. This is distinct from an authority rejection
/// (which would be an `Ok` `Rejected` envelope).
#[test]
fn vietnam_invalid_branch_mst_is_pre_wire_error() {
    let provider = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let doc = vietnamese_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // 13 chars but the 3-char branch suffix is alphabetic, not digits.
    let mut bad_branch = submit_request(ubl_bytes.clone());
    bad_branch.issuer_mst = "0312345678ABC".to_owned();
    assert!(
        matches!(
            provider.submit_invoice(&bad_branch).unwrap_err(),
            GdtError::BadMst(_)
        ),
        "a 13-char MST with a non-digit branch suffix must be a pre-wire BadMst"
    );

    // 11 digits is neither the 10-digit head-office nor 13-digit branch shape.
    let mut wrong_len = submit_request(ubl_bytes);
    wrong_len.issuer_mst = "03123456789".to_owned();
    assert!(matches!(
        provider.submit_invoice(&wrong_len).unwrap_err(),
        GdtError::BadMst(_)
    ));

    // The free-standing validator agrees: a valid 13-digit branch MST is OK.
    assert!(invoicekit_report_vn_gdt::validate_mst("0312345678001").is_ok());
}

/// The full credit-note lifecycle (build -> UBL -> GDT mock -> bundle) must be
/// byte-identical across runs. Determinism underpins the evidence bundle's
/// content address; the UBL serialization and pack must not vary.
#[test]
fn vietnam_credit_note_lifecycle_is_byte_deterministic() {
    let p1 = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let p2 = MockGdtProvider::with_fixed_recorded_at(PINNED_CREATED_AT);
    let doc = vietnamese_credit_note();
    let (a, ubl_a, _) = bundle_for(&doc, &p1);
    let (b, ubl_b, _) = bundle_for(&doc, &p2);
    assert_eq!(ubl_a, ubl_b, "UBL serialization must be byte-stable");
    assert_eq!(a, b, "the whole credit-note lifecycle must be byte-stable");
}
