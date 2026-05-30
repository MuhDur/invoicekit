// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Taiwan MOF e-Invoice offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Taiwan and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("TW")` and
//!    the New Taiwan dollar (`TWD`).
//! 2. serialize -> UBL 2.1 XML (the EN 16931 / UBL family path) via
//!    `invoicekit_format_ubl::to_xml`.
//! 3. submit those bytes to the crate's existing `MockMofProvider` and assert
//!    the MOF authority receipt's Taiwan-specific fields: invoice number
//!    (發票字軌 `AA-nnnnnnnn`), the 4-digit lottery random number (統一發票
//!    兌獎隨機碼), and the `MofStatus::Accepted` (上傳成功) verdict.
//! 4. assemble an `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml` +
//!    `receipt.json`), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true` (exit 0 == report.ok).
//! 5. determinism: pack twice -> byte-identical.
//! 6. refusal: the mock refuses an invalid 統一編號 / empty payload with an
//!    `Err` (pre-wire shape validation). The mock does NOT forge an authority
//!    `MofStatus::Rejected`, so that wire-verdict branch cannot be forced here
//!    (see the note on `tw_mof_rejection_status_is_not_forceable`).
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
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
use invoicekit_report_tw_mof::{
    MockMofProvider, MofEnvironment, MofError, MofInvoiceKind, MofProvider, MofStatus,
    MofSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_tw_e2e";
const TRACE: &str = "trace_tw_e2e";
const ISSUER_UNIFORM_NUMBER: &str = "12345678";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn taiwanese_party(name: &str, uniform_number: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            // Taiwan's 統一編號 (uniform number / business id) carried as the
            // party tax id; MOF treats it as the issuer/buyer VAT-equivalent.
            scheme: "tw:ubn".to_owned(),
            value: uniform_number.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["No. 1, Section 1, Zhongxiao W. Rd.".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "100".to_owned(),
            country: CountryCode::new("TW").unwrap(),
        },
        contact: None,
    }
}

/// A minimal, valid B2B invoice routed inside Taiwan (TWD, 5% business tax).
fn taiwanese_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-tw-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-TW-0001").unwrap(),
        currency: Iso4217Code::new("TWD").unwrap(),
        supplier: taiwanese_party("Acme Co Ltd", ISSUER_UNIFORM_NUMBER, "Taipei"),
        customer: taiwanese_party("Beta Co Ltd", "87654321", "Kaohsiung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cloud platform subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Taiwan's standard business tax (營業稅) is 5%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(5_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(105_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(105_000),
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

fn submit_request(payload: Vec<u8>) -> MofSubmitRequest {
    MofSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: MofEnvironment::Test,
        kind: MofInvoiceKind::B2b,
        issuer_uniform_number: ISSUER_UNIFORM_NUMBER.to_owned(),
        payload,
    }
}

#[test]
fn taiwan_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, _ubl, receipt) = bundle_for(&taiwanese_invoice(), MofInvoiceKind::B2b, None);

    // Country-specific MOF authority artifacts:
    // - 發票字軌 invoice number: two-letter track + 8-digit serial.
    assert!(
        receipt.invoice_number.starts_with("AA-"),
        "MOF invoice number must carry the AA track prefix, got {:?}",
        receipt.invoice_number
    );
    assert_eq!(
        receipt.invoice_number.len(),
        11,
        "AA-nnnnnnnn is 11 chars, got {:?}",
        receipt.invoice_number
    );
    // - 統一發票兌獎 lottery random number: exactly 4 ASCII digits.
    assert_eq!(receipt.random_number.len(), 4);
    assert!(receipt.random_number.bytes().all(|b| b.is_ascii_digit()));
    // - 上傳成功 verdict + the timestamp MOF recorded.
    assert_eq!(receipt.status, MofStatus::Accepted);
    assert_eq!(receipt.issued_at, "2026-05-26T08:30:00Z");
    assert!(receipt.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn taiwan_lifecycle_is_byte_deterministic() {
    let (a, _ubl_a, _) = bundle_for(&taiwanese_invoice(), MofInvoiceKind::B2b, None);
    let (b, _ubl_b, _) = bundle_for(&taiwanese_invoice(), MofInvoiceKind::B2b, None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn taiwan_mof_refuses_invalid_uniform_number() {
    // Refusal path (anti-slop): a malformed 統一編號 is rejected pre-wire as an
    // `Err`, before any receipt is synthesized.
    let provider = MockMofProvider::default();
    let ubl_bytes = to_xml(&taiwanese_invoice()).unwrap().into_bytes();
    let mut req = submit_request(ubl_bytes);
    req.issuer_uniform_number = "BAD".to_owned();
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, MofError::BadUniformNumber(_)),
        "expected BadUniformNumber, got {err:?}"
    );
}

#[test]
fn taiwan_mof_refuses_empty_payload() {
    // The serializer always yields non-empty bytes; this guards the wire
    // contract that an empty MOF payload is refused pre-wire as an `Err`.
    let provider = MockMofProvider::default();
    let err = provider.submit(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, MofError::BadPayload(_)),
        "expected BadPayload, got {err:?}"
    );
}

#[test]
fn tw_mof_default_never_forges_a_rejection() {
    // The default `MockMofProvider` must never synthesize a 上傳失敗 verdict for
    // shape-valid input: for valid input it always returns
    // `MofStatus::Accepted`. The opt-in `with_forced_status` hook is the only
    // way to reach the authority `MofStatus::Rejected` branch (exercised by
    // `tw_mof_rejection_is_a_receipt_status_not_an_error` below); the default
    // path stays accept-only so a rejection is always a deliberate fixture.
    let provider = MockMofProvider::default();
    let ubl_bytes = to_xml(&taiwanese_invoice()).unwrap().into_bytes();
    let receipt = provider.submit(&submit_request(ubl_bytes)).unwrap();
    assert_eq!(receipt.status, MofStatus::Accepted);
    assert!(receipt.reason.is_none());
}

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios (added on top of the honest bar above).
//
// Each scenario grounds its assertions in Taiwan's real electronic uniform
// invoice (電子發票) regime, operated by the Ministry of Finance (財政部, MOF):
//
//   * MOF E-Invoice Platform "Message Implementation Guideline" (MIG) — the
//     B2B / B2C / 折讓 (allowance) message families and 字軌 number track —
//     https://www.einvoice.nat.gov.tw/ (MIG download set, e.g.
//     https://www.einvoice.nat.gov.tw/static/ptl/ein_upload/download/326.pdf).
//   * Uniform invoice usage & the 統一發票兌獎 (uniform-invoice lottery) random
//     number — National Taxation Bureau, https://www.ntbt.gov.tw/.
//   * Business tax (營業稅): the standard rate is 5%; exports are zero-rated
//     (零稅率) and certain supplies are tax-exempt (免稅) under the Value-added
//     and Non-value-added Business Tax Act, https://law.moj.gov.tw/.
//
// Fixtures are hand-built and synthetic — no regulator files are vendored.
// ---------------------------------------------------------------------------

/// A Taiwanese **allowance / credit note** (折讓單). Taiwan models a credit as
/// its own MOF allowance message family ([`MofInvoiceKind::Allowance`]); the
/// canonical IR is a [`DocumentType::CreditNote`], which the UBL family path
/// serializes as a `CreditNote` root (UBL `cbc:CreditNoteTypeCode` 381, lines
/// as `cac:CreditNoteLine` with `cbc:CreditedQuantity`). UBL 2.1 `CreditNote`
/// has no top-level `cbc:DueDate`, so the credit carries `due_date: None`.
fn taiwanese_allowance() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-tw-e2e-allow-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        document_number: DocumentNumber::new("ALW-2026-TW-0001").unwrap(),
        currency: Iso4217Code::new("TWD").unwrap(),
        supplier: taiwanese_party("Acme Co Ltd", ISSUER_UNIFORM_NUMBER, "Taipei"),
        customer: taiwanese_party("Beta Co Ltd", "87654321", "Kaohsiung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Subscription allowance (折讓)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(50_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // 5% business tax credited back on the allowed base.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50_000),
            tax_amount: amt(2_500),
            tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50_000),
            tax_exclusive_amount: amt(50_000),
            tax_inclusive_amount: amt(52_500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(52_500),
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

/// A two-line invoice mixing the **standard 5% business tax** (category `S`)
/// with a **zero-rated export** line (零稅率, category `Z`, 0%). Taiwan zero-rates
/// exports and certain international services under the Value-added and
/// Non-value-added Business Tax Act (営業稅 zero rate), distinct from a tax-exempt
/// (免稅) supply: a zero-rated supplier still files the line but charges 0% tax.
fn taiwanese_mixed_standard_and_zero_rated_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-tw-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("INV-2026-TW-0002").unwrap(),
        currency: Iso4217Code::new("TWD").unwrap(),
        supplier: taiwanese_party("Acme Co Ltd", ISSUER_UNIFORM_NUMBER, "Taipei"),
        customer: taiwanese_party("Beta Co Ltd", "87654321", "Kaohsiung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Domestic cloud subscription (5%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(50_000),
                line_extension_amount: amt(100_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Exported support service (零稅率)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(200_000),
                line_extension_amount: amt(200_000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(5_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(200_000),
                tax_amount: amt(0),
                // Scale-2 zero so the UBL Percent renders "0.00".
                tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(300_000),
            tax_exclusive_amount: amt(300_000),
            tax_inclusive_amount: amt(305_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(305_000),
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

/// Steps 2-4 for an arbitrary document + MOF kind + forced wire verdict, reusing
/// the same pinned timestamps so output stays byte-stable. Returns
/// `(ikb, ubl_xml, receipt)`.
fn bundle_for(
    doc: &CommercialDocument,
    kind: MofInvoiceKind,
    forced: Option<MofStatus>,
) -> (Vec<u8>, String, invoicekit_report_tw_mof::MofSubmitEnvelope) {
    let ubl = to_xml(doc).unwrap();
    let ubl_bytes = ubl.clone().into_bytes();

    let mut provider = MockMofProvider::with_fixed_issued_at("2026-05-26T08:30:00Z");
    if let Some(status) = forced {
        provider = provider.with_forced_status(status);
    }
    let mut req = submit_request(ubl_bytes.clone());
    req.kind = kind;
    let receipt = provider.submit(&req).unwrap();

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&receipt).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, ubl, receipt)
}

/// An **allowance / credit note** (折讓單) must serialize through the UBL family
/// path as a `CreditNote` document — `cbc:CreditNoteTypeCode` 381 and
/// `cac:CreditNoteLine` / `cbc:CreditedQuantity`, never the `Invoice` shape —
/// and submit under [`MofInvoiceKind::Allowance`]. The whole offline lifecycle
/// must still produce a verifiable evidence bundle. (MOF MIG allowance message
/// family, <https://www.einvoice.nat.gov.tw>.)
#[test]
fn taiwan_allowance_serializes_as_credit_note_and_bundles() {
    let doc = taiwanese_allowance();
    let (ikb, ubl, receipt) = bundle_for(&doc, MofInvoiceKind::Allowance, None);

    // UBL CreditNote shape (the national MOF allowance carrier here). The
    // serializer declares namespaces inline per element, so assert on the
    // namespace-stable parts: the CreditNote-2 root and the qualified close
    // tags / text content.
    assert!(
        ubl.contains(
            r#"<CreditNote xmlns="urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2""#
        ),
        "an allowance must serialize as a UBL CreditNote-2 root, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL credit-note type code 381 must be present"
    );
    assert!(
        ubl.contains("</cac:CreditNoteLine>") && ubl.contains("</cbc:CreditedQuantity>"),
        "credit lines must use CreditNoteLine / CreditedQuantity, not the invoice shape"
    );
    assert!(
        !ubl.contains("CreditNoteLine") || !ubl.contains("InvoicedQuantity"),
        "an allowance must not carry the invoice-line quantity element"
    );
    assert!(
        !ubl.contains("</cbc:InvoiceTypeCode>"),
        "an allowance must not carry the invoice type code"
    );
    // The credit-note number rides DatiGenerali / cbc:ID.
    assert!(ubl.contains(">ALW-2026-TW-0001</cbc:ID>"));
    // The allowed base credits 5% business tax: 500.00 base -> 25.00 tax.
    assert!(ubl.contains(r#"currencyID="TWD">25.00</cbc:TaxAmount>"#));
    assert!(ubl.contains(r#"currencyID="TWD">500.00</cbc:TaxableAmount>"#));
    assert!(ubl.contains(">5.00</cbc:Percent>"));

    // MOF still returns an Accepted receipt with the 字軌 number + lottery code.
    assert_eq!(receipt.status, MofStatus::Accepted);
    assert!(receipt.invoice_number.starts_with("AA-"));
    assert_eq!(receipt.invoice_number.len(), 11);
    assert_eq!(receipt.random_number.len(), 4);

    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "allowance evidence bundle must verify");
}

/// A two-line invoice mixing the standard 5% business tax with a zero-rated
/// (零稅率) export line. The UBL `cac:TaxTotal` must emit one `cac:TaxSubtotal`
/// per band: 5% on 1000.00 -> 50.00 tax, and 0% on 2000.00 -> 0.00 tax. This
/// proves the per-band Taiwanese tax summary and the zero-rate path end to end.
#[test]
fn taiwan_mixed_standard_and_zero_rated_emits_per_band_subtotals() {
    let doc = taiwanese_mixed_standard_and_zero_rated_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, MofInvoiceKind::B2b, None);

    // Two distinct lines in document order (names carry inline xmlns, so match
    // the namespace-stable text + close tag).
    assert!(ubl.contains(">Domestic cloud subscription (5%)</cbc:Name>"));
    assert!(ubl.contains(">Exported support service (零稅率)</cbc:Name>"));

    // One TaxSubtotal per band, with the right Taiwanese rates and amounts.
    assert_eq!(
        ubl.matches("<cac:TaxSubtotal>").count(),
        2,
        "a mixed standard/zero-rated invoice must emit one TaxSubtotal per band"
    );
    assert!(ubl.contains(">5.00</cbc:Percent>"));
    assert!(ubl.contains(">0.00</cbc:Percent>"));
    // Standard band: 5% of 1000.00 -> 50.00. Zero band: 0.00 on 2000.00.
    assert!(ubl.contains(r#"currencyID="TWD">1000.00</cbc:TaxableAmount>"#));
    assert!(ubl.contains(r#"currencyID="TWD">50.00</cbc:TaxAmount>"#));
    assert!(ubl.contains(r#"currencyID="TWD">2000.00</cbc:TaxableAmount>"#));
    // The TaxTotal header sums the two bands: 50.00 + 0.00 = 50.00. It is the
    // first TaxAmount after the TaxTotal open tag.
    let tax_total_at = ubl
        .find("<cac:TaxTotal")
        .expect("UBL must contain a TaxTotal");
    let header = &ubl[tax_total_at..];
    assert!(
        header.contains(r#"currencyID="TWD">50.00</cbc:TaxAmount>"#),
        "the TaxTotal header must sum the two bands to 50.00"
    );

    assert_eq!(receipt.status, MofStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "mixed-band evidence bundle must verify");
}

/// MOF authority **rejection** (上傳失敗) is a *receipt status*, not an `Err`.
/// When the MOF e-Invoice platform refuses an otherwise well-formed upload it
/// returns 上傳失敗 ([`MofStatus::Rejected`]) — the engine persists that verdict
/// (with its reason) alongside the audit trail rather than failing the call.
/// The submitted invoice number / lottery code are still assigned, and the
/// rejection evidence bundle must still verify. This is distinct from the
/// pre-wire `Err` refusals (`taiwan_mof_refuses_*`).
#[test]
fn taiwan_mof_rejection_is_a_receipt_status_not_an_error() {
    let doc = taiwanese_invoice();
    let (ikb, _ubl, receipt) = bundle_for(&doc, MofInvoiceKind::B2c, Some(MofStatus::Rejected));

    assert_eq!(
        receipt.status,
        MofStatus::Rejected,
        "a forced 上傳失敗 must surface as a Rejected receipt status, not an Err"
    );
    assert_eq!(
        receipt.reason.as_deref(),
        Some("MOF rejected the upload (上傳失敗)"),
        "a Rejected receipt must carry the MOF reason text"
    );
    // The 字軌 number track and lottery random number are still assigned.
    assert!(receipt.invoice_number.starts_with("AA-"));
    assert_eq!(receipt.random_number.len(), 4);

    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must still verify");
}

/// A B2C ([`MofInvoiceKind::B2c`], 二聯式) submission of the same invoice must
/// still receive an accepted MOF receipt carrying the lottery random number
/// (統一發票兌獎隨機碼): unlike B2B (三聯式), B2C uniform invoices participate in
/// the bi-monthly uniform-invoice lottery, so the 4-digit random number is the
/// load-bearing consumer-facing field.
#[test]
fn taiwan_b2c_submission_carries_lottery_random_number() {
    let doc = taiwanese_invoice();
    let (ikb, _ubl, receipt) = bundle_for(&doc, MofInvoiceKind::B2c, None);

    assert_eq!(receipt.status, MofStatus::Accepted);
    assert_eq!(
        receipt.random_number.len(),
        4,
        "B2C invoices must carry a 4-digit uniform-invoice lottery random number"
    );
    assert!(receipt.random_number.bytes().all(|b| b.is_ascii_digit()));

    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "B2C evidence bundle must verify");
}

/// The full allowance lifecycle (build -> UBL `CreditNote` -> submit -> bundle)
/// must be byte-identical across runs. Determinism is load-bearing for the
/// evidence bundle's content address; the credit-line ordering and per-band
/// summary order must not vary between runs.
#[test]
fn taiwan_allowance_lifecycle_is_byte_deterministic() {
    let doc = taiwanese_allowance();
    let (a, ubl_a, _) = bundle_for(&doc, MofInvoiceKind::Allowance, None);
    let (b, ubl_b, _) = bundle_for(&doc, MofInvoiceKind::Allowance, None);
    assert_eq!(ubl_a, ubl_b, "UBL CreditNote serialization must be stable");
    assert_eq!(a, b, "the whole allowance lifecycle must be byte-stable");
}

/// An invalid 統一編號 (uniform number) of the right length but with a non-digit
/// is refused pre-wire as an `Err` — distinct from an authority 上傳失敗
/// rejection. The MOF uniform number is exactly 8 ASCII digits; `1234567X`
/// (8 chars, one letter) matches the length but not the digit rule.
#[test]
fn taiwan_mof_refuses_uniform_number_with_letter_pre_wire() {
    let provider = MockMofProvider::default();
    let ubl_bytes = to_xml(&taiwanese_invoice()).unwrap().into_bytes();
    let mut req = submit_request(ubl_bytes);
    req.issuer_uniform_number = "1234567X".to_owned();
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, MofError::BadUniformNumber(_)),
        "an 8-char uniform number containing a letter must be a pre-wire Err, got {err:?}"
    );
}
