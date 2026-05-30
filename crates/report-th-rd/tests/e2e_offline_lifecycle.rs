// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Thailand Revenue Department (RD) offline end-to-end lifecycle.
//!
//! Drives the full local-only chain for Thailand and proves it
//! deterministically, mirroring the proven `report-it-sdi` pattern:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("TH")`
//!    and `Iso4217Code("THB")` (Thai baht)
//! 2. serialize -> UBL 2.1 XML via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path; this crate exposes no national
//!    serializer of its own, so the e-Tax payload rides the UBL syntax)
//! 3. submit those bytes to the crate's existing `MockRdProvider` and
//!    assert the RD-specific receipt fields (`rd_ref` prefix `TH-`, the
//!    `Acknowledged` status, the pinned acknowledgement timestamp)
//! 4. assemble an `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml`
//!    + `receipt.json`) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock rejects an empty payload and a malformed Thai tax id
//!    with `Err` before the wire (see the note in `th_rejection_is_a_refusal`)
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would
//! mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_th_rd::{
    MockRdProvider, RdEnvironment, RdFlavour, RdProvider, RdStatus, RdSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ACKNOWLEDGED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_th_e2e";
const TRACE: &str = "trace_th_e2e";
// 13 ASCII digits — the exact Thai tax-id shape `validate_tax_id` enforces.
const ISSUER_TAX_ID: &str = "1234567890123";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn thai_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 Sukhumvit Road".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "10110".to_owned(),
            country: CountryCode::new("TH").unwrap(),
        },
        contact: None,
    }
}

fn thai_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-th-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-TH-0001").unwrap(),
        // Thai baht — a sensible ISO 4217 currency for a domestic RD invoice.
        currency: Iso4217Code::new("THB").unwrap(),
        supplier: thai_party("Acme (Thailand) Co Ltd", "TH0105551234567", "Bangkok"),
        customer: thai_party("Beta Trading Co Ltd", "TH0105559876543", "Nonthaburi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL uses EA
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Thai standard VAT is 7%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(7_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(700, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(107_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(107_000),
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

fn submit_request(ubl_xml: Vec<u8>) -> RdSubmitRequest {
    RdSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: RdEnvironment::Uat,
        flavour: RdFlavour::ETaxInvoice,
        issuer_tax_id: ISSUER_TAX_ID.to_owned(),
        payload: ubl_xml,
    }
}

fn provider() -> MockRdProvider {
    // Pin the acknowledgement timestamp so the receipt artefact is byte-stable.
    MockRdProvider::with_fixed_acknowledged_at(PINNED_ACKNOWLEDGED_AT)
}

/// Steps 1-4: build -> serialize -> submit -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the RD receipt so callers can assert
/// both the bundle and the country-specific receipt fields.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_th_rd::RdSubmitEnvelope) {
    // 1-4: build -> serialize -> submit -> bundle, via the shared `bundle_for`.
    let (ikb, ubl_xml, receipt) = bundle_for(&thai_invoice(), None);

    // Structural smoke check on the national-family artefact spine. The C14N
    // canonicalizer attaches per-element namespace declarations, so each
    // prefixed element opens as `<cac:Foo xmlns:cac="...">` — match the
    // prefix-plus-space form, and match the currency by its element close so
    // the leading-attribute on the open tag is irrelevant.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
        "<cac:AccountingSupplierParty ",
        "<cac:AccountingCustomerParty ",
        ">THB</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL XML missing {needle}");
    }

    (ikb, receipt)
}

#[test]
fn thailand_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Country-specific receipt assertions: RD reference, status, timestamp.
    assert_eq!(receipt.status, RdStatus::Acknowledged);
    assert!(
        receipt.rd_ref.starts_with("TH-"),
        "RD reference must carry the country-tagged TH- prefix, got {:?}",
        receipt.rd_ref
    );
    assert_eq!(receipt.acknowledged_at, PINNED_ACKNOWLEDGED_AT);
    assert!(receipt.reason.is_none(), "happy path carries no rejection reason");

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn thailand_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn thailand_rejection_is_a_refusal() {
    // The MockRdProvider always returns RdStatus::Acknowledged on a valid
    // submission — it exposes NO knob to force an RD-side `Rejected` verdict
    // envelope (unlike report-it-sdi's `with_forced_receipt`). The only
    // refusal paths it supports are the pre-wire `Err` validators, which we
    // exercise here: an empty payload and a malformed 13-digit Thai tax id.
    use invoicekit_report_th_rd::RdError;

    let p = provider();

    // Empty payload is refused before the wire.
    let mut empty = submit_request(Vec::new());
    empty.payload.clear();
    let err = p.submit_invoice(&empty).unwrap_err();
    assert!(matches!(err, RdError::BadPayload(_)), "empty payload must be a BadPayload Err");

    // A malformed Thai tax id is refused before the wire.
    let mut bad_id = submit_request(b"<Invoice/>".to_vec());
    bad_id.issuer_tax_id = "NOT-13-DIGITS".to_owned();
    let err = p.submit_invoice(&bad_id).unwrap_err();
    assert!(matches!(err, RdError::BadTaxId(_)), "malformed tax id must be a BadTaxId Err");
}

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios (added on top of the §1 honest bar).
//
// Each scenario grounds its assertions in the real Thai e-invoicing rules:
//
//   * Authority: the **Revenue Department** (กรมสรรพากร, "RD"), administered
//     under the Thai Revenue Code. e-Tax Invoice & e-Receipt regulation:
//     https://etax.rd.go.th/ and https://www.rd.go.th/english/ .
//   * Technical XML standard: ETDA Recommendation on ICT Standard for
//     Electronic Transactions — "Standard for e-Tax Invoice & e-Receipt
//     Messages", ขมธอ. 3-2560 (ETDA Rec. 3-2560), Electronic Transactions
//     Development Agency, https://www.etda.or.th/ . The standard covers the
//     tax invoice, the **credit note** (ใบลดหนี้, Revenue Code s.86/9) and the
//     **debit note** (ใบเพิ่มหนี้, Revenue Code s.86/10).
//   * VAT: the standard rate is **7 %** (reduced from the statutory 10 % by
//     Royal Decree, extended through 30 Sep 2026); a **0 % zero rate** applies
//     to exports and qualifying international services (Revenue Code s.80/1);
//     certain goods/services are **VAT-exempt** (Revenue Code s.81). The Thai
//     Revenue Department, "Value Added Tax", https://www.rd.go.th/english/6043.html .
//
// The crate is a national-clearance *report adapter*: it carries no national
// serializer of its own, so the e-Tax payload rides the EN 16931 / UBL 2.1
// syntax via `invoicekit_format_ubl::to_xml`. UBL emits the UNCL1001 document
// type code as `cbc:InvoiceTypeCode` 380 (invoice) / `cbc:CreditNoteTypeCode`
// 381 (credit note); the Thai-specific values (THB, the 7 %/0 % bands, the
// document numbers) are asserted at the IR + serialized level.
//
// Fixtures are hand-built synthetic data — no copyrighted RD/ETDA files are
// vendored.
// ---------------------------------------------------------------------------

/// A Thai **credit note** (ใบลดหนี้) that reverses part of [`thai_invoice`].
/// Revenue Code s.86/9 lets a VAT registrant issue a credit note when the tax
/// base falls after the original tax invoice; ETDA Rec. 3-2560 carries it as a
/// distinct message type. In UBL 2.1 a credit note serializes under the
/// `CreditNote` root with `cbc:CreditNoteTypeCode` 381, and — unlike an
/// invoice — it carries **no** top-level `cbc:DueDate` (so `due_date` is
/// `None`, else the UBL serializer rejects the field).
fn thai_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-th-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote has no top-level DueDate spine.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-TH-0001").unwrap(),
        currency: Iso4217Code::new("THB").unwrap(),
        supplier: thai_party("Acme (Thailand) Co Ltd", "TH0105551234567", "Bangkok"),
        customer: thai_party("Beta Trading Co Ltd", "TH0105559876543", "Nonthaburi"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit note: returned consulting hours".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(50_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Same 7 % standard band as the original invoice, on the reduced base.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50_000),
            tax_amount: amt(3_500),
            tax_rate: Some(DecimalValue::new(Decimal::new(700, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50_000),
            tax_exclusive_amount: amt(50_000),
            tax_inclusive_amount: amt(53_500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(53_500),
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

/// A two-line invoice mixing the **7 % standard** VAT band (Revenue Code s.80)
/// with a **0 % zero-rated export** line (Revenue Code s.80/1, exports of
/// goods). UBL emits one `cac:TaxSubtotal` per band: a 7 % `cbc:Percent` band
/// on the domestic line and a 0 % band (tax category `Z`, zero-rated) on the
/// export line. This exercises the per-band tax-summary path with the two real
/// Thai VAT rates side by side.
fn thai_mixed_standard_and_zero_rated_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-th-e2e-mixed-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-TH-0002").unwrap(),
        currency: Iso4217Code::new("THB").unwrap(),
        supplier: thai_party("Acme (Thailand) Co Ltd", "TH0105551234567", "Bangkok"),
        customer: thai_party("Gamma Export Co Ltd", "TH0105550001112", "Laem Chabang"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Domestic consulting (7% VAT)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(100_000),
                line_extension_amount: amt(100_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Exported goods (0% zero-rated, s.80/1)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(200_000),
                line_extension_amount: amt(200_000),
                // Z = zero-rated tax category.
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(7_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(700, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(200_000),
                tax_amount: amt(0),
                // Scale-2 zero so cbc:Percent renders "0.00", not bare "0".
                tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(300_000),
            tax_exclusive_amount: amt(300_000),
            // 7% of 1000.00 = 70.00 VAT on the standard line only.
            tax_inclusive_amount: amt(307_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(307_000),
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

/// A **VAT-exempt** invoice (Revenue Code s.81: exempt goods/services such as
/// certain educational, healthcare and agricultural supplies). An exempt
/// supply charges no output VAT — tax category `E`, a 0.00 tax amount, and the
/// taxable base equals the payable total. This is distinct from the 0 %
/// *zero-rated* path (`Z`): exempt supplies fall outside the VAT charge
/// altogether rather than being taxed at a 0 % rate.
fn thai_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-th-e2e-exempt-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-TH-0003").unwrap(),
        currency: Iso4217Code::new("THB").unwrap(),
        supplier: thai_party("Acme (Thailand) Co Ltd", "TH0105551234567", "Bangkok"),
        customer: thai_party("Delta Education Co Ltd", "TH0105550003334", "Chiang Mai"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exempt educational services (s.81)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(80_000),
            line_extension_amount: amt(80_000),
            // E = exempt from VAT.
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(80_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(80_000),
            tax_exclusive_amount: amt(80_000),
            // No VAT charged: payable equals the taxable base.
            tax_inclusive_amount: amt(80_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(80_000),
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

/// Steps 2-4 for an arbitrary document, reusing the same fixed transmission
/// context and pinned timestamps so output stays byte-stable. An optional
/// forced RD rejection reason drives the authority-refusal path. Returns
/// `(ikb, ubl_xml, receipt)`.
fn bundle_for(
    doc: &CommercialDocument,
    forced_rejection: Option<&str>,
) -> (Vec<u8>, String, invoicekit_report_th_rd::RdSubmitEnvelope) {
    let ubl_xml = to_xml(doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();

    let provider = forced_rejection
        .map_or_else(provider, |reason| provider().with_forced_rejection(reason));
    let receipt = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&receipt).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, ubl_xml, receipt)
}

/// A Thai credit note (ใบลดหนี้) must serialize under the UBL `CreditNote`
/// root with `cbc:CreditNoteTypeCode` 381 (UNCL1001), never the invoice code
/// 380. Revenue Code s.86/9 / ETDA Rec. 3-2560. The whole offline lifecycle
/// must still acknowledge and produce a verifiable evidence bundle.
#[test]
fn thailand_credit_note_serializes_as_creditnote_381_and_bundles() {
    let doc = thai_credit_note();
    let (ikb, ubl, receipt) = bundle_for(&doc, None);

    // CreditNote root + UNCL1001 type code 381.
    assert!(
        ubl.contains("<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\""),
        "a Thai credit note must serialize under the UBL CreditNote root, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "a credit note must carry UNCL1001 type code 381"
    );
    assert!(
        !ubl.contains("cbc:InvoiceTypeCode"),
        "a credit note must not carry an InvoiceTypeCode"
    );
    // The credit-note number rides cbc:ID; the 7% band stays at 7.00.
    assert!(ubl.contains(">CN-2026-TH-0001</cbc:ID>"));
    assert!(ubl.contains(">7.00</cbc:Percent>"));
    assert!(ubl.contains(">THB</cbc:DocumentCurrencyCode>"));

    assert_eq!(receipt.status, RdStatus::Acknowledged);
    assert!(receipt.rd_ref.starts_with("TH-"));
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// A mixed 7 % standard + 0 % zero-rated export invoice. UBL emits one
/// `cac:TaxSubtotal` per band: a 7.00 % band (category `S`) and a 0.00 % band
/// (category `Z`, zero-rated). Revenue Code s.80 (7 % standard) and s.80/1
/// (0 % exports). This proves the per-band tax-summary path with both real
/// Thai VAT rates.
#[test]
fn thailand_mixed_standard_and_zero_rated_emits_per_band_subtotals() {
    let doc = thai_mixed_standard_and_zero_rated_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, None);

    // Two tax subtotals, one per VAT band.
    assert_eq!(
        ubl.matches("<cac:TaxSubtotal>").count(),
        2,
        "a mixed-rate invoice must emit one TaxSubtotal per VAT band, got:\n{ubl}"
    );
    // The 7% standard band and the 0% zero-rated band both appear.
    assert!(ubl.contains(">7.00</cbc:Percent>"), "the 7% standard band must render");
    assert!(ubl.contains(">0.00</cbc:Percent>"), "the 0% zero-rated band must render");
    // Tax category codes: S (standard) and Z (zero-rated).
    assert!(ubl.contains(">S</cbc:ID>"), "the standard band carries category S");
    assert!(ubl.contains(">Z</cbc:ID>"), "the zero-rated band carries category Z");
    // The export line's taxable base appears at 2000.00, the standard line at
    // 1000.00 — one TaxableAmount per band.
    assert!(ubl.contains(">2000.00</cbc:TaxableAmount>"), "zero-rated band taxable base 2000.00");
    assert!(ubl.contains(">1000.00</cbc:TaxableAmount>"), "standard band taxable base 1000.00");
    // Header VAT (cac:TaxTotal/cbc:TaxAmount) is 70.00 — 7% of the 1000.00
    // domestic line only; the export line contributes 0.00.
    assert!(ubl.contains(">70.00</cbc:TaxAmount>"), "header VAT must be 70.00 (7% of the standard line)");
    // Tax-inclusive total = 3000.00 base + 70.00 VAT = 3070.00.
    assert!(ubl.contains(">3070.00</cbc:TaxInclusiveAmount>"), "tax-inclusive total must be 3070.00");
    assert!(ubl.contains(">3070.00</cbc:PayableAmount>"), "payable total must be 3070.00");

    assert_eq!(receipt.status, RdStatus::Acknowledged);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "mixed-rate evidence bundle must verify");
}

/// A VAT-exempt invoice (Revenue Code s.81): tax category `E`, no output VAT,
/// taxable base equals the payable total. Distinct from the zero-rated `Z`
/// path. The lifecycle must acknowledge and the bundle must verify.
#[test]
fn thailand_exempt_invoice_charges_no_vat() {
    let doc = thai_exempt_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, None);

    assert!(ubl.contains(">E</cbc:ID>"), "an exempt line must carry tax category E, got:\n{ubl}");
    // No VAT: the tax-exclusive and tax-inclusive totals are equal at 800.00.
    assert!(ubl.contains(">800.00</cbc:TaxExclusiveAmount>"));
    assert!(ubl.contains(">800.00</cbc:TaxInclusiveAmount>"));
    assert!(ubl.contains(">800.00</cbc:PayableAmount>"));
    // The 7% standard band must NOT appear on an exempt invoice.
    assert!(!ubl.contains(">7.00</cbc:Percent>"), "an exempt supply charges no 7% VAT");

    assert_eq!(receipt.status, RdStatus::Acknowledged);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "exempt-invoice evidence bundle must verify");
}

/// The Revenue Department portal can refuse an otherwise well-formed
/// submission (e.g. a signing-certificate or schema fault). Per the
/// `RdProvider` contract this RD refusal is an `Ok` envelope with
/// `RdStatus::Rejected` and a populated `reason` — NOT an `Err`. The audit
/// trail (and its evidence bundle) must still be produced and verify. This is
/// the authority-rejection path, distinct from the pre-wire validator `Err`s.
#[test]
fn thailand_authority_rejection_is_a_receipt_not_an_error() {
    let doc = thai_invoice();
    let reason = "RD: ใบกำกับภาษีถูกปฏิเสธ (digital signature invalid)";
    let (ikb, _ubl, receipt) = bundle_for(&doc, Some(reason));

    assert_eq!(
        receipt.status,
        RdStatus::Rejected,
        "an RD portal refusal must surface as RdStatus::Rejected, not an Err"
    );
    assert_eq!(
        receipt.reason.as_deref(),
        Some(reason),
        "a rejected RD receipt must carry the refusal reason"
    );
    // An RD reference is still assigned on a rejection.
    assert!(receipt.rd_ref.starts_with("TH-"), "rejected receipt still carries a TH- reference");

    // The rejection-path bundle must still verify (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

/// A 13-character-but-non-digit issuer tax id is refused **before** the wire
/// as a `BadTaxId` `Err` — distinct from an RD authority rejection (which is a
/// receipt). The Thai tax identification number is exactly 13 ASCII digits;
/// `validate_tax_id` enforces the shape, so a 13-char value with letters fails
/// even though its length is right. (Revenue Department 13-digit Tax ID.)
#[test]
fn thailand_rejects_thirteen_char_non_digit_tax_id_pre_wire() {
    use invoicekit_report_th_rd::RdError;

    let doc = thai_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let mut req = submit_request(ubl_bytes);
    // 13 chars, but the last is a letter — right length, wrong shape.
    req.issuer_tax_id = "123456789012X".to_owned();
    assert_eq!(req.issuer_tax_id.len(), 13, "fixture must be exactly 13 chars");

    let err = provider().submit_invoice(&req).unwrap_err();
    assert!(
        matches!(err, RdError::BadTaxId(_)),
        "a 13-char non-digit tax id must be a pre-wire BadTaxId Err, not an RD receipt: {err:?}"
    );
}

/// The full credit-note lifecycle (build -> UBL -> submit -> bundle) must be
/// byte-identical across runs. Determinism is load-bearing for the evidence
/// bundle's content address; the `CreditNote` serialization and the pinned RD
/// receipt must not vary between runs.
#[test]
fn thailand_credit_note_lifecycle_is_byte_deterministic() {
    let doc = thai_credit_note();
    let (a, ubl_a, _) = bundle_for(&doc, None);
    let (b, ubl_b, _) = bundle_for(&doc, None);
    assert_eq!(ubl_a, ubl_b, "UBL CreditNote serialization must be stable");
    assert_eq!(a, b, "the whole credit-note lifecycle must be byte-stable");
}
