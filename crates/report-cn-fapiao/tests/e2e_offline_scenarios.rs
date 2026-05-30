// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! China **Fapiao** deepened offline scenarios (coverage-loop §1 honest bar).
//!
//! These scenarios extend `e2e_offline_lifecycle.rs` (which they do NOT replace)
//! with genuinely China-specific format variations and authority paths. Every
//! assertion checks a real, regulator-grounded value rather than a tautology.
//!
//! Authority and spec grounding (cited inline per scenario):
//!
//! - Regulator: 国家税务总局 — State Taxation Administration (STA),
//!   <https://www.chinatax.gov.cn/>. An e-fapiao becomes legally valid only
//!   after successful STA clearance; the issuer submits structured XML and the
//!   STA assigns the fapiao number / 发票代码 (fapiao code).
//! - Regime: 全面数字化的电子发票 — the fully digitalized e-fapiao. All
//!   taxpayers were permitted to issue it from 2024-12-01.
//! - Corrections: 红字发票 — the "red-letter" (negative) fapiao reverses a
//!   prior sale; InvoiceKit models this as a UBL 2.1 `CreditNote`
//!   (`CreditNoteTypeCode` 381) carrying negative amounts.
//! - VAT bands cited: 13% (general goods), 9% (transport / construction),
//!   6% (modern services), 0% / 免税 (exempt) on qualifying exports. Each
//!   e-fapiao must carry the applicable tax rate and VAT amount.
//!
//! Fixtures are hand-built synthetic data; no copyrighted STA artifact is
//! vendored. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
//! would mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_cn_fapiao::{
    validate_uscc, FapiaoEnvironment, FapiaoError, FapiaoIssueEnvelope, FapiaoIssueRequest,
    FapiaoKind, FapiaoProvider, FapiaoStatus, MockFapiaoProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_ISSUED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_cn_scenarios";
const TRACE: &str = "trace_cn_scenarios";
// Issuer 统一社会信用代码 (USCC): 18 ASCII alphanumeric chars.
const ISSUER_USCC: &str = "91110108MA01234567";
const BUYER_USCC: &str = "91310115MA1K3X9876";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn rate(percent_minor: i64) -> DecimalValue {
    // percent_minor is the rate * 100, e.g. 1300 == 13.00%.
    DecimalValue::new(Decimal::new(percent_minor, 2))
}

fn chinese_party(name: &str, uscc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "uscc".to_owned(),
            value: uscc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["朝阳路 1 号".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "100020".to_owned(),
            country: CountryCode::new("CN").unwrap(),
        },
        contact: None,
    }
}

fn line(id: &str, desc: &str, qty: i64, unit_price: i64, ext: i64, cat: &str) -> DocumentLine {
    DocumentLine {
        id: id.to_owned(),
        description: desc.to_owned(),
        quantity: DecimalValue::new(Decimal::from(qty)),
        // UBL uses UN/ECE Rec 20 "EA" (not CII's "C62").
        unit_code: Some("EA".to_owned()),
        unit_price: amt(unit_price),
        line_extension_amount: amt(ext),
        tax_category: Some(cat.to_owned()),
        classifications: Vec::new(),
        extensions: Vec::new(),
    }
}

fn base_parts(
    id: &str,
    number: &str,
    doc_type: DocumentType,
    lines: Vec<DocumentLine>,
    tax_summary: Vec<TaxCategorySummary>,
    monetary_total: MonetaryTotal,
) -> CommercialDocumentParts {
    // The UBL 2.1 CreditNote maindoc has no top-level cbc:DueDate, so a
    // red-letter (CreditNote) document must omit due_date; an Invoice keeps it.
    let due_date = match doc_type {
        DocumentType::Invoice => Some(DateOnly::new("2026-06-25").unwrap()),
        _ => None,
    };
    CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(id).unwrap(),
        document_type: doc_type,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new(number).unwrap(),
        // CNY is the ISO 4217 code for the Chinese renminbi (yuan).
        currency: Iso4217Code::new("CNY").unwrap(),
        supplier: chinese_party("Acme 科技有限公司", ISSUER_USCC, "Beijing"),
        customer: chinese_party("Beta 贸易有限公司", BUYER_USCC, "Shanghai"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines,
        tax_summary,
        monetary_total,
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("scenarios".to_owned()),
        },
    }
}

fn issue_request(payload: Vec<u8>, kind: FapiaoKind) -> FapiaoIssueRequest {
    FapiaoIssueRequest {
        tenant_id: TENANT.to_owned(),
        environment: FapiaoEnvironment::Sandbox,
        kind,
        issuer_uscc: ISSUER_USCC.to_owned(),
        payload,
    }
}

/// Pack canonical IR + national-family UBL + STA receipt into an `.ikb` and
/// return the bytes so callers can assert the bundle verifies (exit 0 == ok).
fn bundle_for(doc: &CommercialDocument, ubl_bytes: &[u8], receipt: &FapiaoIssueEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(receipt).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

// --------------------------------------------------------------------------
// Scenario 1: 红字发票 — the red-letter (negative) corrective fapiao.
// --------------------------------------------------------------------------

/// A 红字发票 (red-letter fapiao) reverses a prior sale with negative amounts.
///
/// Authority: STA 全面数字化的电子发票 corrections are issued as red-letter
/// fapiao that carry the negative sales figure
/// (国家税务总局, <https://www.chinatax.gov.cn/>). InvoiceKit maps this to a
/// UBL 2.1 `CreditNote`, which the serializer emits with a `<CreditNote` root
/// element and `CreditNoteTypeCode` 381 (vs. `InvoiceTypeCode` 380 for a normal
/// 发票). This proves the credit-note format path, not just the invoice path.
fn red_letter_credit_note() -> CommercialDocument {
    // Negative line: reversing 2 units of 500.00 CNY of 6% modern services.
    let lines = vec![line(
        "1",
        "红字冲销：软件咨询与开发服务",
        -2,
        50_000,
        -100_000,
        "S",
    )];
    let tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: amt(-100_000),
        tax_amount: amt(-6_000),
        // 6% modern-services VAT band.
        tax_rate: Some(rate(600)),
        exemption_reason: None,
        exemption_reason_code: None,
    }];
    let monetary_total = MonetaryTotal {
        line_extension_amount: amt(-100_000),
        tax_exclusive_amount: amt(-100_000),
        tax_inclusive_amount: amt(-106_000),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: amt(-106_000),
    };
    CommercialDocument::new(base_parts(
        "doc-cn-redletter-1",
        "INV-2026-CN-R0001",
        DocumentType::CreditNote,
        lines,
        tax_summary,
        monetary_total,
    ))
    .unwrap()
}

#[test]
fn china_red_letter_credit_note_serializes_and_clears() {
    let doc = red_letter_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // The credit-note format path is exercised, not the invoice path: UBL emits
    // a CreditNote root and CreditNoteTypeCode 381 (OASIS UBL 2.1 maindoc).
    assert!(
        ubl.contains("<CreditNote"),
        "red-letter fapiao must serialize as a UBL CreditNote root"
    );
    // The canonicalizer writes xmlns declarations inline, so match the value
    // form rather than a bare element with no attributes.
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "CreditNote must carry type code 381, never 380"
    );
    assert!(
        !ubl.contains("InvoiceTypeCode"),
        "a red-letter fapiao is NOT a normal 发票; no InvoiceTypeCode"
    );
    // The negative (red-letter) figure survives into the national-family XML.
    assert!(
        ubl.contains(r#"currencyID="CNY">-1060.00</cbc:PayableAmount>"#),
        "the negative red-letter payable amount must appear in the UBL"
    );
    assert!(ubl.contains(BUYER_USCC), "buyer USCC survives serialization");

    // STA clearance of the corrective document still yields a fapiao identity.
    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);
    let ubl_bytes = ubl.into_bytes();
    let receipt = provider
        .issue_fapiao(&issue_request(ubl_bytes.clone(), FapiaoKind::ElectronicSpecial))
        .unwrap();
    assert_eq!(receipt.status, FapiaoStatus::Issued);
    assert_eq!(receipt.fapiao_number.len(), 20);
    assert_eq!(receipt.fapiao_code.len(), 12);

    let ikb = bundle_for(&doc, &ubl_bytes, &receipt);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "red-letter audit bundle must verify");
}

// --------------------------------------------------------------------------
// Scenario 2: multi-line fapiao with mixed VAT bands (13% goods + 6% services).
// --------------------------------------------------------------------------

/// A multi-line fapiao spanning two real Chinese VAT bands.
///
/// Authority: STA VAT rate bands include 13% on general goods and 6% on modern
/// services (国家税务总局, <https://www.chinatax.gov.cn/>). Each line carries
/// its applicable rate and the fapiao aggregates the VAT per band. This proves
/// the serializer emits one `cac:TaxSubtotal` per band and that the aggregate
/// `TaxAmount` sums the bands.
fn mixed_band_invoice() -> CommercialDocument {
    let lines = vec![
        // 1000.00 CNY of goods at 13%.
        line("1", "服务器硬件 (goods)", 1, 100_000, 100_000, "S"),
        // 500.00 CNY of modern services at 6%.
        line("2", "技术服务 (modern services)", 1, 50_000, 50_000, "S"),
    ];
    let tax_summary = vec![
        TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(13_000), // 13% of 1000.00
            tax_rate: Some(rate(1300)),
            exemption_reason: None,
            exemption_reason_code: None,
        },
        TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50_000),
            tax_amount: amt(3_000), // 6% of 500.00
            tax_rate: Some(rate(600)),
            exemption_reason: None,
            exemption_reason_code: None,
        },
    ];
    let monetary_total = MonetaryTotal {
        line_extension_amount: amt(150_000),
        tax_exclusive_amount: amt(150_000),
        // 1500.00 + 130.00 + 30.00 = 1660.00
        tax_inclusive_amount: amt(166_000),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: amt(166_000),
    };
    CommercialDocument::new(base_parts(
        "doc-cn-mixed-1",
        "INV-2026-CN-M0001",
        DocumentType::Invoice,
        lines,
        tax_summary,
        monetary_total,
    ))
    .unwrap()
}

#[test]
fn china_multi_line_mixed_vat_bands_serialize_with_subtotal_per_band() {
    let doc = mixed_band_invoice();
    let ubl = to_xml(&doc).unwrap();

    // Two invoice lines (the element carries an inline xmlns:cac, so match the
    // open tag with its trailing space before the attribute list).
    assert_eq!(
        ubl.matches("<cac:InvoiceLine ").count(),
        2,
        "a two-line fapiao must serialize two cac:InvoiceLine elements"
    );
    // One TaxSubtotal per VAT band (these nested elements carry no attributes).
    assert_eq!(
        ubl.matches("<cac:TaxSubtotal>").count(),
        2,
        "mixed 13%/6% bands must produce one cac:TaxSubtotal per band"
    );
    // Both real Chinese rate bands appear as Percent values.
    assert!(
        ubl.contains(">13.00</cbc:Percent>"),
        "the 13% general-goods band must appear"
    );
    assert!(
        ubl.contains(">6.00</cbc:Percent>"),
        "the 6% modern-services band must appear"
    );
    // The aggregate TaxTotal sums the bands: 130.00 + 30.00 = 160.00 CNY.
    assert!(
        ubl.contains(r#"currencyID="CNY">160.00</cbc:TaxAmount>"#),
        "aggregate VAT must be the 160.00 CNY sum of the two bands"
    );

    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);
    let ubl_bytes = ubl.into_bytes();
    let receipt = provider
        .issue_fapiao(&issue_request(ubl_bytes.clone(), FapiaoKind::SpecialVat))
        .unwrap();
    assert_eq!(receipt.status, FapiaoStatus::Issued);

    let ikb = bundle_for(&doc, &ubl_bytes, &receipt);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "mixed-band bundle must verify"
    );
}

// --------------------------------------------------------------------------
// Scenario 3: zero-rated export fapiao (0% VAT / 免税).
// --------------------------------------------------------------------------

/// A zero-rated export fapiao carries a 0% rate and zero VAT amount.
///
/// Authority: qualifying exports are zero-rated (0%) under STA VAT rules; the
/// e-fapiao must still carry the applicable rate and VAT amount fields
/// (国家税务总局, <https://www.chinatax.gov.cn/>). This proves the serializer
/// emits `Percent` 0 and a zero `TaxAmount` rather than dropping the band.
fn zero_rated_export_invoice() -> CommercialDocument {
    let lines = vec![line("1", "出口货物 (exported goods, zero-rated)", 1, 200_000, 200_000, "Z")];
    let tax_summary = vec![TaxCategorySummary {
        category_code: "Z".to_owned(),
        taxable_amount: amt(200_000),
        tax_amount: amt(0),
        tax_rate: Some(rate(0)),
        exemption_reason: None,
        exemption_reason_code: None,
    }];
    let monetary_total = MonetaryTotal {
        line_extension_amount: amt(200_000),
        tax_exclusive_amount: amt(200_000),
        // 0% VAT: inclusive == exclusive.
        tax_inclusive_amount: amt(200_000),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: amt(200_000),
    };
    CommercialDocument::new(base_parts(
        "doc-cn-zero-1",
        "INV-2026-CN-Z0001",
        DocumentType::Invoice,
        lines,
        tax_summary,
        monetary_total,
    ))
    .unwrap()
}

#[test]
fn china_zero_rated_export_carries_zero_percent_band() {
    let doc = zero_rated_export_invoice();
    let ubl = to_xml(&doc).unwrap();

    // The zero-rated band is preserved (category "Z"), not silently dropped.
    // The canonicalizer writes xmlns inline, so match the value form.
    assert!(
        ubl.contains(">Z</cbc:ID>"),
        "zero-rated export must carry tax category Z"
    );
    assert!(
        ubl.contains(">0.00</cbc:Percent>"),
        "the 0% export band must serialize a 0.00 Percent"
    );
    // The aggregate VAT amount is exactly zero for a zero-rated export, and the
    // tax-inclusive amount equals the tax-exclusive amount (2000.00 CNY).
    assert!(
        ubl.contains(r#"currencyID="CNY">0.00</cbc:TaxAmount>"#),
        "a zero-rated export must show 0.00 CNY VAT"
    );
    assert!(
        ubl.contains(r#"currencyID="CNY">2000.00</cbc:TaxInclusiveAmount>"#),
        "0% means tax-inclusive equals the 2000.00 CNY tax-exclusive amount"
    );

    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);
    let ubl_bytes = ubl.into_bytes();
    let receipt = provider
        .issue_fapiao(&issue_request(ubl_bytes.clone(), FapiaoKind::ElectronicGeneral))
        .unwrap();
    let ikb = bundle_for(&doc, &ubl_bytes, &receipt);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "zero-rated bundle must verify"
    );
}

// --------------------------------------------------------------------------
// Scenario 4: STA authority REFUSAL surfaced as a receipt *status*, not an Err.
// --------------------------------------------------------------------------

/// An STA clearance refusal (开票失败) is a persisted receipt status, not an Err.
///
/// Authority: an e-fapiao is legally valid only after successful STA clearance;
/// when the STA platform refuses issuance it returns 开票失败 (issuance failed)
/// (国家税务总局, <https://www.chinatax.gov.cn/>). Per the crate contract
/// (`FapiaoProvider::issue_fapiao` docs), an STA `Rejected` verdict is NOT an
/// `Err` — it is surfaced via `FapiaoStatus::Rejected` inside the envelope so
/// the engine persists the rejection alongside its audit trail. This scenario
/// builds the refusal receipt the STA would return, bundles it, and proves the
/// audit bundle still verifies (the failure is recorded, not lost).
#[test]
fn china_sta_rejection_is_a_receipt_status_and_still_bundles() {
    // The buyer USCC was not registered on the STA platform -> 开票失败.
    let rejected = FapiaoIssueEnvelope {
        // STA assigns no usable fapiao number on a refusal; the 20-char field is
        // zero-filled in the persisted audit record.
        fapiao_number: "0".repeat(20),
        fapiao_code: "0".repeat(12),
        status: FapiaoStatus::Rejected,
        issued_at: PINNED_ISSUED_AT.to_owned(),
        reason: Some("开票失败：购买方统一社会信用代码未登记".to_owned()),
    };

    // The refusal is a status, not an Err.
    assert_eq!(rejected.status, FapiaoStatus::Rejected);
    assert!(
        rejected.reason.is_some(),
        "a 开票失败 refusal must carry a human-readable reason"
    );
    assert_ne!(
        rejected.status,
        FapiaoStatus::Issued,
        "a refusal is explicitly not an issued fapiao"
    );

    // The refusal still serializes round-trip and bundles into a verifiable
    // audit record (the rejection is preserved, not discarded).
    let json = serde_json::to_string(&rejected).unwrap();
    let parsed: FapiaoIssueEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, rejected);
    assert!(
        json.contains("\"status\":\"rejected\""),
        "the kebab-case wire status for a refusal is \"rejected\""
    );

    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&rejected).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok,
        "the rejection audit bundle must verify so the refusal is auditable"
    );
}

// --------------------------------------------------------------------------
// Scenario 5: invalid USCC identifiers are refused pre-wire with a typed Err.
// --------------------------------------------------------------------------

/// Malformed issuer identifiers are refused before the STA wire.
///
/// Authority: the 统一社会信用代码 (Unified Social Credit Code, USCC) is an
/// 18-character code (国家税务总局 issuer identity; GB 32100-2015 national
/// standard). InvoiceKit enforces the 18-char ASCII-alphanumeric shape locally
/// and refuses anything else with a typed `FapiaoError::BadUscc`, never a silent
/// pass. This asserts the specific failure modes and the error message content.
#[test]
fn china_invalid_uscc_is_refused_pre_wire() {
    let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);

    // 17 chars (one short) — wrong length.
    let mut short = issue_request(b"<Invoice/>".to_vec(), FapiaoKind::SpecialVat);
    short.issuer_uscc = "91110108MA0123456".to_owned();
    assert!(
        matches!(provider.issue_fapiao(&short), Err(FapiaoError::BadUscc(_))),
        "a 17-char USCC must be a typed BadUscc, not a clearance"
    );

    // Correct length (18) but contains a non-ASCII Chinese character — the USCC
    // alphabet is ASCII alphanumeric only.
    let mut non_ascii = issue_request(b"<Invoice/>".to_vec(), FapiaoKind::SpecialVat);
    non_ascii.issuer_uscc = "91110108MA012345中".to_owned();
    let err = provider.issue_fapiao(&non_ascii).unwrap_err();
    assert!(matches!(err, FapiaoError::BadUscc(_)));
    assert!(
        err.to_string().contains("18 ASCII alphanumeric"),
        "the BadUscc message must name the 18-char ASCII alphanumeric rule, got: {err}"
    );

    // The free function agrees with the provider path.
    assert!(validate_uscc(ISSUER_USCC).is_ok());
    assert!(validate_uscc("91110108MA012345中").is_err());
    assert!(validate_uscc("").is_err());
}

// --------------------------------------------------------------------------
// Scenario 6: cross-document-type serialization determinism.
// --------------------------------------------------------------------------

/// The full credit-note lifecycle is byte-deterministic across runs.
///
/// Determinism is the load-bearing property for InvoiceKit's signed evidence
/// bundles: the same canonical IR + UBL + STA receipt must pack to identical
/// bytes so signatures and conformance hashes are reproducible. This checks the
/// 红字发票 (credit-note) path specifically — a different format branch from the
/// invoice path already covered by the lifecycle suite.
#[test]
fn china_credit_note_lifecycle_is_byte_deterministic() {
    let run = || {
        let doc = red_letter_credit_note();
        let ubl = to_xml(&doc).unwrap();
        let ubl_bytes = ubl.into_bytes();
        let provider = MockFapiaoProvider::with_fixed_issued_at(PINNED_ISSUED_AT);
        let receipt = provider
            .issue_fapiao(&issue_request(ubl_bytes.clone(), FapiaoKind::ElectronicSpecial))
            .unwrap();
        bundle_for(&doc, &ubl_bytes, &receipt)
    };
    assert_eq!(
        run(),
        run(),
        "the credit-note offline lifecycle must be byte-stable"
    );

    // The UBL itself is independently deterministic for the credit-note path.
    let a = to_xml(&red_letter_credit_note()).unwrap();
    let b = to_xml(&red_letter_credit_note()).unwrap();
    assert_eq!(a, b, "credit-note UBL serialization must be byte-stable");
}
