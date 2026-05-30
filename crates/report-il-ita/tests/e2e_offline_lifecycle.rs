// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Israel ITA offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Israel and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("IL")` +
//!    ISO currency `ILS`
//! 2. serialize -> UBL 2.1 XML bytes via `invoicekit_format_ubl::to_xml`
//!    (the EN 16931 / UBL family path)
//! 3. submit those bytes to the crate's existing `MockItaProvider` and assert
//!    the authority receipt's country-specific fields (Allocation Number /
//!    status / timestamp)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the `MockItaProvider` always returns `Allocated`, so an
//!    authority-forced `Rejected` verdict is NOT supported (see the note on
//!    [`ita_refuses_bad_issuer_id_before_the_wire`]). The genuinely-supported
//!    pre-wire refusals (bad tax id / empty payload) ARE exercised as `Err`.
//!
//! Goldens are hand-rolled — no `insta` / `pretty_assertions`, which would
//! mutate `Cargo.lock`.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_il_ita::{
    ItaAllocationRequest, ItaEnvironment, ItaError, ItaProvider, ItaStatus, MockItaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const FIXED_ISSUED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_il_e2e";
const TRACE: &str = "trace_il_e2e";
const ISSUER_ID: &str = "123456789";
const BUYER_ID: &str = "987654321";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

/// An Israeli party. Israel uses Hebrew localities; the IR carries them as
/// plain UTF-8 strings and the UBL canonicalizer preserves them verbatim.
fn israeli_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Rothschild Blvd 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "6688101".to_owned(),
            country: CountryCode::new("IL").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Israeli B2B invoice priced in ILS (New Israeli Shekel).
fn israeli_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-il-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IL-0001").unwrap(),
        currency: Iso4217Code::new("ILS").unwrap(),
        supplier: israeli_party("Acme IL Ltd", "IL123456789", "Tel Aviv"),
        customer: israeli_party("Beta IL Ltd", "IL987654321", "Haifa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Israel's standard VAT rate is 17%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1700),
            tax_rate: Some(DecimalValue::new(Decimal::new(1700, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11700),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// The one canonical `ItaAllocationRequest` builder. Pins the tenant/env/ids
/// every scenario shares and varies only the gross and payload.
fn allocation_request_at(gross_basis_points: u64, payload: Vec<u8>) -> ItaAllocationRequest {
    ItaAllocationRequest {
        tenant_id: TENANT.to_owned(),
        environment: ItaEnvironment::Sandbox,
        issuer_id: ISSUER_ID.to_owned(),
        buyer_id: BUYER_ID.to_owned(),
        gross_basis_points,
        payload,
    }
}

/// Fixed-gross convenience for the pre-wire refusal tests: 1.00 ILS ==
/// `10_000` basis points, so gross is 117.00 ILS here.
fn allocation_request(payload: Vec<u8>) -> ItaAllocationRequest {
    allocation_request_at(1_170_000, payload)
}

/// Steps 1-4: build -> serialize (UBL) -> request allocation -> evidence bundle.
///
/// Returns the packed `.ikb` plus the ITA receipt so callers can assert both
/// the country-specific authority fields and that the bundle verifies. Shares
/// the serialize/submit/pack spine with [`run_lifecycle_for`]; this wrapper
/// pins the standard invoice + gross + `MockItaProvider` and adds the UBL
/// structural needle assertions.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_il_ita::ItaAllocationEnvelope) {
    // 1. build
    let doc = israeli_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path).
    //    The canonicalizer pushes namespace declarations down to first use, so
    //    assert on prefix-qualified element starts (not the closed `>` form).
    let xml = String::from_utf8(to_xml(&doc).unwrap().into_bytes()).unwrap();
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">ILS</cbc:DocumentCurrencyCode>",
    ] {
        assert!(xml.contains(needle), "UBL XML missing {needle}");
    }

    // 3-4. submit + bundle via the shared spine. A deterministic fixed-timestamp
    //      mock keeps pack() byte-stable; gross is 117.00 ILS == 1_170_000 bp.
    run_lifecycle_for(
        &doc,
        1_170_000,
        &MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT),
    )
}

#[test]
fn israel_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Country-specific authority fields: ITA grants a 9-digit Allocation
    // Number and records the verdict + timestamp it stamped.
    assert_eq!(envelope.status, ItaStatus::Allocated);
    assert_eq!(
        envelope.allocation_number.len(),
        9,
        "ITA Allocation Number is a 9-digit numeric"
    );
    assert!(envelope.allocation_number.bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(envelope.issued_at, FIXED_ISSUED_AT);
    assert!(envelope.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn israel_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

/// Refusal path. The `MockItaProvider` always returns `ItaStatus::Allocated`
/// and exposes no `with_forced_receipt`-style knob, so an authority-forced
/// `Rejected` verdict is NOT supported by this mock. What the mock DOES
/// support — and what the audit-trail contract demands as a hard `Err` — is
/// pre-wire identity-shape validation: a malformed issuer tax id is rejected
/// before any payload reaches ITA.
#[test]
fn ita_refuses_bad_issuer_id_before_the_wire() {
    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let mut req = allocation_request(b"<Invoice/>".to_vec());
    req.issuer_id = "12345".to_owned(); // not 9 digits
    let err = provider.request_allocation(&req).unwrap_err();
    assert!(matches!(err, ItaError::BadId(_)));
}

/// The other genuinely-supported pre-wire refusal: an empty payload never
/// reaches ITA. Surfaced as `Err(BadPayload)`, not a `Rejected` envelope.
#[test]
fn ita_refuses_empty_payload_before_the_wire() {
    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let err = provider
        .request_allocation(&allocation_request(Vec::new()))
        .unwrap_err();
    assert!(matches!(err, ItaError::BadPayload(_)));
}

// ===========================================================================
// Deepened, Israel-specific coverage.
//
// Authority: Israel Tax Authority — רשות המסים — operating the SHAAM (שע"מ)
// computerized clearance platform. SHAAM validates the invoice and returns a
// per-transaction Allocation Number (מספר הקצאה) the issuer prints on the
// document; without it the buyer cannot claim an input-VAT deduction.
//
// Spec references grounded below:
//   - ITA Open API for SHAAM (Hebrew technical specification, gov.il):
//     https://www.gov.il/BlobFolder/service/connect-to-shaam/he/Service_Pages_shaam_Tax-Authority-Open-API.pdf
//   - Allocation-Number thresholds (turnover *before* VAT) staged down over
//     2026: NIS 10,000 from 2026-01-01, then NIS 5,000 from 2026-06-01.
//   - Standard Israeli VAT rate is 18% since 2025-01-01 (raised from 17%).
//     The original single-line scenario above still asserts the historic 17%
//     figure deliberately and is left untouched; the new scenarios assert the
//     current 18% rate.
//   - Exports of goods/services are zero-rated (0%) under the Israeli VAT Law;
//     the Eilat free-trade-zone regime is a separate exemption. The zero-rated
//     export scenario below carries category `Z` at a 0.00% rate.
// ===========================================================================

/// Gross amount in the crate's basis-point convention: one shekel is ten
/// thousand basis points, so an `X.YY` shekel amount is `X*10_000 + YY*100`.
fn gross_bp(major: u64, minor: u64) -> u64 {
    major * 10_000 + minor * 100
}

/// An Israeli **credit note** (חשבונית זיכוי) — the corrective document a
/// supplier issues to reverse or reduce a prior tax invoice. UBL maps
/// `DocumentType::CreditNote` to `<cbc:CreditNoteTypeCode>381</...>` and forbids
/// a top-level `cbc:DueDate`, so `due_date` is `None` here.
fn israeli_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-il-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote carries no top-level cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-IL-0001").unwrap(),
        currency: Iso4217Code::new("ILS").unwrap(),
        supplier: israeli_party("Acme IL Ltd", "IL123456789", "Tel Aviv"),
        customer: israeli_party("Beta IL Ltd", "IL987654321", "Haifa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit: returned consulting hours".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Standard 18% Israeli VAT on the reversed base.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5900),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A multi-line standard-rated Israeli invoice priced at the current 18% VAT
/// rate. Two lines (200.00 + 50.00 = 250.00 base; 18% => 45.00 tax; 295.00
/// payable) above the 2026 Allocation-Number threshold.
fn israeli_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-il-ml-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-06-02").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-07-02").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IL-0042").unwrap(),
        currency: Iso4217Code::new("ILS").unwrap(),
        supplier: israeli_party("Acme IL Ltd", "IL123456789", "Tel Aviv"),
        customer: israeli_party("Beta IL Ltd", "IL987654321", "Haifa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Software consulting".to_owned(),
                quantity: DecimalValue::new(Decimal::from(4)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(20000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Hosting surcharge".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(5000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(25000),
            tax_amount: amt(4500),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(25000),
            tax_exclusive_amount: amt(25000),
            tax_inclusive_amount: amt(29500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(29500),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A zero-rated **export** invoice. Israeli VAT Law zero-rates exports of goods
/// and services (0%), so the tax category is `Z` and the tax amount is 0.00 —
/// the supplier still owes an Allocation Number once over threshold, but no VAT
/// is charged. Customer here is a foreign buyer; the IR carries them with a
/// non-IL country.
fn israeli_zero_rated_export() -> CommercialDocument {
    let mut foreign = israeli_party("Beta US Inc", "US-EIN-99", "New York");
    foreign.address.country = CountryCode::new("US").unwrap();
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-il-zr-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-06-03").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-07-03").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IL-EXP-1").unwrap(),
        currency: Iso4217Code::new("ILS").unwrap(),
        supplier: israeli_party("Acme IL Ltd", "IL123456789", "Tel Aviv"),
        customer: foreign,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exported SaaS subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(80000),
            line_extension_amount: amt(80000),
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(80000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(80000),
            tax_exclusive_amount: amt(80000),
            tax_inclusive_amount: amt(80000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(80000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// Drive build -> UBL serialize -> Allocation request -> evidence bundle for an
/// arbitrary Israeli document, parameterised on the SHAAM provider so a
/// rejection path can be exercised. Returns the packed `.ikb` plus the receipt.
fn run_lifecycle_for(
    doc: &CommercialDocument,
    gross_basis_points: u64,
    provider: &dyn ItaProvider,
) -> (Vec<u8>, invoicekit_report_il_ita::ItaAllocationEnvelope) {
    let ubl_bytes = to_xml(doc).unwrap().into_bytes();
    let request = allocation_request_at(gross_basis_points, ubl_bytes.clone());
    let envelope = provider.request_allocation(&request).unwrap();
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

/// Scenario: an Israeli **credit note** (חשבונית זיכוי) clears SHAAM, gets its
/// own Allocation Number, and the evidence bundle verifies. Asserts the UBL
/// projection carries the corrective-document spine (`CreditNoteTypeCode` 381
/// and a `cac:CreditNoteLine`) and never a `cbc:DueDate`.
#[test]
fn israel_credit_note_clears_and_bundles() {
    let doc = israeli_credit_note();
    let xml = String::from_utf8(to_xml(&doc).unwrap().into_bytes()).unwrap();
    assert!(xml.contains("<CreditNote"), "root must be a CreditNote");
    assert!(
        xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL CreditNote type code is 381:\n{xml}"
    );
    assert!(
        xml.contains("<cac:CreditNoteLine"),
        "credit notes carry cac:CreditNoteLine, not cac:InvoiceLine:\n{xml}"
    );
    assert!(
        !xml.contains("DueDate"),
        "UBL 2.1 CreditNote must not carry a top-level cbc:DueDate"
    );

    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let (ikb, envelope) = run_lifecycle_for(&doc, gross_bp(59, 0), &provider);
    assert_eq!(envelope.status, ItaStatus::Allocated);
    assert_eq!(envelope.allocation_number.len(), 9);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// Scenario: a multi-line invoice at the current **18%** Israeli VAT rate. Two
/// `InvoiceLine`s, a single standard-rated `S` summary, 250.00 base => 45.00
/// tax => 295.00 payable. Asserts UBL emits both lines, the 18 percent figure,
/// and the `VAT` tax scheme.
#[test]
fn israel_multiline_invoice_serializes_18_percent() {
    let doc = israeli_multiline_invoice();
    let xml = String::from_utf8(to_xml(&doc).unwrap().into_bytes()).unwrap();
    // Two invoice lines (canonicalizer expands to start tags `<cac:InvoiceLine`).
    let line_count = xml.matches("<cac:InvoiceLine").count();
    assert_eq!(line_count, 2, "multi-line invoice must emit 2 InvoiceLine:\n{xml}");
    assert!(
        xml.contains(">18.00</cbc:Percent>"),
        "standard Israeli VAT percent is 18 since 2025-01-01:\n{xml}"
    );
    assert!(
        xml.contains(">VAT</cbc:ID>"),
        "tax scheme id must be VAT:\n{xml}"
    );
    // Tax math the IR carries: 250.00 base, 45.00 tax, 295.00 payable.
    assert!(xml.contains(">295.00</cbc:PayableAmount>"), "payable 295.00:\n{xml}");
    assert!(
        xml.contains(">45.00</cbc:TaxAmount>"),
        "aggregate VAT is 45.00:\n{xml}"
    );

    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let (ikb, envelope) = run_lifecycle_for(&doc, gross_bp(295, 0), &provider);
    assert_eq!(envelope.status, ItaStatus::Allocated);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

/// Scenario: a **zero-rated export** invoice. Israeli VAT Law zero-rates
/// exports (0%); the tax category is `Z` and the VAT charged is 0.00, yet the
/// supplier still obtains an Allocation Number. Asserts UBL carries the `Z`
/// category, a 0 percent figure, and a 0.00 aggregate tax.
#[test]
fn israel_zero_rated_export_charges_no_vat() {
    let doc = israeli_zero_rated_export();
    let xml = String::from_utf8(to_xml(&doc).unwrap().into_bytes()).unwrap();
    assert!(xml.contains(">Z</cbc:ID>"), "zero-rate category id is Z:\n{xml}");
    assert!(xml.contains(">0.00</cbc:Percent>"), "export VAT percent is 0:\n{xml}");
    assert!(
        xml.contains(">0.00</cbc:TaxAmount>"),
        "zero-rated export charges 0.00 VAT:\n{xml}"
    );
    // Foreign buyer country travels through the UBL identification code.
    assert!(
        xml.contains(">US</cbc:IdentificationCode>"),
        "exported-to country must serialize:\n{xml}"
    );

    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let (ikb, envelope) = run_lifecycle_for(&doc, gross_bp(800, 0), &provider);
    assert_eq!(envelope.status, ItaStatus::Allocated);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "zero-rated export evidence bundle must verify");
}

/// A SHAAM provider that refuses an Allocation Number when the transaction is
/// below the statutory threshold (turnover *before* VAT). This is the real ITA
/// behaviour: a below-threshold B2B invoice does not require — and is not
/// granted — an Allocation Number. The refusal is a *receipt status*
/// (`ItaStatus::Rejected` with a reason), NOT an `Err`, so the audit trail
/// persists it. The 2026-06-01 threshold is NIS 5,000 before VAT; at the
/// crate's one-shekel-is-ten-thousand-basis-points convention that is fifty
/// million basis points (see `threshold_basis_points` below).
///
/// This exercises the crate's public `ItaProvider` seam exactly as the future
/// live `report-il-ita-http` provider will, and the `Rejected` half of
/// `ItaStatus` that the always-`Allocated` `MockItaProvider` cannot reach.
struct ThresholdItaProvider {
    threshold_basis_points: u64,
    issued_at: String,
}

impl ItaProvider for ThresholdItaProvider {
    fn request_allocation(
        &self,
        request: &ItaAllocationRequest,
    ) -> Result<invoicekit_report_il_ita::ItaAllocationEnvelope, ItaError> {
        // Run the same pre-wire identity/payload checks the real provider runs.
        invoicekit_report_il_ita::validate_id(&request.issuer_id)?;
        invoicekit_report_il_ita::validate_id(&request.buyer_id)?;
        if request.payload.is_empty() {
            return Err(ItaError::BadPayload("payload is empty".to_owned()));
        }
        if request.gross_basis_points < self.threshold_basis_points {
            return Ok(invoicekit_report_il_ita::ItaAllocationEnvelope {
                // ITA returns no Allocation Number for a refused request.
                allocation_number: "000000000".to_owned(),
                status: ItaStatus::Rejected,
                issued_at: self.issued_at.clone(),
                reason: Some(
                    "below SHAAM allocation threshold (NIS 5,000 before VAT)".to_owned(),
                ),
            });
        }
        Ok(invoicekit_report_il_ita::ItaAllocationEnvelope {
            allocation_number: "100000001".to_owned(),
            status: ItaStatus::Allocated,
            issued_at: self.issued_at.clone(),
            reason: None,
        })
    }
}

/// Scenario: an authority **rejection** path. A below-threshold transaction is
/// refused an Allocation Number; the verdict is surfaced as
/// `ItaStatus::Rejected` (with a reason), the bundle still packs, and it still
/// verifies — mirroring the SDI "Notifica di Scarto is a receipt, not an Err"
/// contract for Italy.
#[test]
fn israel_below_threshold_is_rejected_not_errored() {
    let doc = israeli_invoice();
    let provider = ThresholdItaProvider {
        // NIS 5,000 before VAT == 50_000_000 bp (the 2026-06-01 SHAAM threshold).
        threshold_basis_points: 50_000_000,
        issued_at: FIXED_ISSUED_AT.to_owned(),
    };
    // NIS 4,900.00 gross — just below the NIS 5,000 threshold.
    let (ikb, envelope) = run_lifecycle_for(&doc, gross_bp(4_900, 0), &provider);
    assert_eq!(envelope.status, ItaStatus::Rejected);
    assert_eq!(
        envelope.allocation_number, "000000000",
        "a refused request carries no usable Allocation Number"
    );
    let reason = envelope.reason.as_deref().expect("rejection carries a reason");
    assert!(
        reason.contains("threshold"),
        "rejection reason explains the SHAAM threshold: {reason:?}"
    );

    // Rejection is persisted, not lost: the evidence bundle still verifies.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");
}

/// The same provider grants an Allocation Number once the transaction is over
/// threshold — proving the threshold rule is the discriminator, not a constant.
#[test]
fn israel_over_threshold_is_allocated() {
    let doc = israeli_invoice();
    let provider = ThresholdItaProvider {
        threshold_basis_points: 50_000_000,
        issued_at: FIXED_ISSUED_AT.to_owned(),
    };
    // NIS 6,000.00 gross — above the NIS 5,000-before-VAT threshold.
    let (_ikb, envelope) = run_lifecycle_for(&doc, gross_bp(6_000, 0), &provider);
    assert_eq!(envelope.status, ItaStatus::Allocated);
    assert!(envelope.reason.is_none());
    assert!(envelope.allocation_number.bytes().all(|b| b.is_ascii_digit()));
}

/// ITA tax identifiers are exactly **9 ASCII digits** (the issuer/buyer
/// `מספר עוסק` / company number shape SHAAM keys on). Identifiers that are the
/// right length but carry a letter, or a 9-character alphanumeric token that
/// would be a valid VAT id elsewhere, are refused before the wire as
/// `ItaError::BadId`.
#[test]
fn ita_rejects_non_numeric_nine_char_identifier() {
    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let mut req = allocation_request(b"<Invoice/>".to_vec());
    // 9 chars, but alphanumeric — a plausible foreign id, invalid for ITA.
    req.buyer_id = "IL5140269".to_owned();
    let err = provider.request_allocation(&req).unwrap_err();
    assert!(matches!(err, ItaError::BadId(_)));

    // The free-standing validator agrees on the exact 9-ASCII-digit shape.
    assert!(invoicekit_report_il_ita::validate_id("514026900").is_ok());
    assert!(invoicekit_report_il_ita::validate_id("IL5140269").is_err());
    assert!(invoicekit_report_il_ita::validate_id("51402690").is_err()); // 8 digits
    assert!(invoicekit_report_il_ita::validate_id("5140269000").is_err()); // 10 digits
}

/// Determinism for the deepened document shapes: re-running the whole offline
/// lifecycle for the credit note and the zero-rated export yields byte-identical
/// `.ikb` archives (the canonicalizer + pinned `created_at` + fixed-timestamp
/// mock leave no nondeterminism).
#[test]
fn israel_deepened_shapes_are_byte_deterministic() {
    // A fresh provider per run: `MockItaProvider` carries an internal serial
    // counter, so the Allocation Number it mints advances across submissions.
    // Determinism is a property of the lifecycle from a clean start, exactly as
    // the existing `israel_lifecycle_is_byte_deterministic` exercises it.
    let cn = israeli_credit_note();
    let (a, _) = run_lifecycle_for(
        &cn,
        gross_bp(59, 0),
        &MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT),
    );
    let (b, _) = run_lifecycle_for(
        &cn,
        gross_bp(59, 0),
        &MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT),
    );
    assert_eq!(a, b, "credit-note lifecycle must be byte-stable");

    let zr = israeli_zero_rated_export();
    let (c, _) = run_lifecycle_for(
        &zr,
        gross_bp(800, 0),
        &MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT),
    );
    let (d, _) = run_lifecycle_for(
        &zr,
        gross_bp(800, 0),
        &MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT),
    );
    assert_eq!(c, d, "zero-rated-export lifecycle must be byte-stable");
}

/// A `Rejected` Allocation envelope round-trips losslessly through serde,
/// carrying its Hebrew-regime reason text — the receipt JSON the evidence
/// bundle persists for a refused SHAAM request.
#[test]
fn rejected_allocation_envelope_round_trips() {
    let env = invoicekit_report_il_ita::ItaAllocationEnvelope {
        allocation_number: "000000000".to_owned(),
        status: ItaStatus::Rejected,
        issued_at: FIXED_ISSUED_AT.to_owned(),
        reason: Some("below SHAAM allocation threshold (NIS 5,000 before VAT)".to_owned()),
    };
    let json = serde_json::to_string(&env).unwrap();
    // kebab-case serde on the status enum.
    assert!(json.contains("\"status\":\"rejected\""), "status serialises kebab-case: {json}");
    let parsed: invoicekit_report_il_ita::ItaAllocationEnvelope =
        serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, env);
}
