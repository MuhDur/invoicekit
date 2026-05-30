// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! South Africa SARS offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for South Africa and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a `ZA` country code and
//!    the `ZAR` (South African rand) ISO-4217 currency;
//! 2. serialize -> EN 16931 / UBL 2.1 XML bytes (the family path; SARS has no
//!    bespoke national serializer in-tree yet);
//! 3. submit those bytes to the crate's existing `MockSarsProvider` and assert
//!    the SARS-specific receipt fields (`sars_ref` prefix + `Accepted` status);
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`;
//! 5. determinism: pack twice -> byte-identical;
//! 6. refusal path: bad VAT and empty payload are surfaced as `Err`.
//!
//! Note on the rejection contract: the SARS mock always returns
//! `SarsStatus::Accepted` and exposes no knob to force a
//! `SarsStatus::Rejected` envelope. The authority-`Rejected` verdict is
//! therefore NOT exercised here; instead the refusal test covers the two
//! pre-wire `Err` buckets the mock does support (`BadVat`, `BadPayload`).
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
use invoicekit_report_za_sars::{
    MockSarsProvider, SarsEnvironment, SarsError, SarsProvider, SarsStatus, SarsSubmitRequest,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_za_e2e";
const TRACE: &str = "trace_za_e2e";
/// Issuer SARS VAT registration: 10 ASCII digits, always starting with `4`.
const ISSUER_VAT: &str = "4123456789";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn za_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 Adderley Street".to_owned()],
            city: city.to_owned(),
            subdivision: Some("Western Cape".to_owned()),
            postal_code: "8001".to_owned(),
            country: CountryCode::new("ZA").unwrap(),
        },
        contact: None,
    }
}

fn za_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ZA-0001").unwrap(),
        // South African rand.
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Beta Holdings", "4987654321", "Johannesburg"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL uses EA.
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // South African standard-rated VAT is 15%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(15000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(115_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(115_000),
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

fn submit_request(payload: Vec<u8>) -> SarsSubmitRequest {
    SarsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: SarsEnvironment::Sandbox,
        issuer_vat: ISSUER_VAT.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to SARS mock -> evidence
/// bundle. Returns the packed `.ikb` plus the SARS receipt.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_za_sars::SarsSubmitEnvelope) {
    // 1. build the IR document.
    let doc = za_invoice();

    // 2-4. serialize (UBL) -> submit to the SARS mock -> evidence bundle, via
    // the shared `bundle_for` assembler so the artefact layout and pack path
    // stay byte-identical with the deepened scenarios below.
    let (ikb, ubl_xml, receipt) = bundle_for(&doc, &MockSarsProvider::default());

    // Cheap structural sanity: the canonical UBL spine is present and the
    // South African currency surfaced on the wire. The canonicalizer emits
    // namespace declarations inline on the first use of each prefix, so match
    // on the element-name prefix rather than a bare closing angle bracket.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        "ZAR",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    (ikb, receipt)
}

#[test]
fn za_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // SARS authority artifacts: an accepted verdict carrying a ZA-prefixed
    // reference and the pinned recorded-at timestamp from the deterministic
    // mock.
    assert_eq!(receipt.status, SarsStatus::Accepted);
    assert!(
        receipt.sars_ref.starts_with("ZA-"),
        "SARS reference must carry the ZA country prefix, got {:?}",
        receipt.sars_ref
    );
    assert_eq!(receipt.recorded_at, "2026-01-01T00:00:00Z");
    assert!(receipt.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn za_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn za_refusal_paths_are_errors_not_envelopes() {
    // The SARS mock cannot be forced to return SarsStatus::Rejected. The
    // two refusal buckets it DOES support are pre-wire shape failures, both
    // surfaced as Err (never an Accepted/Rejected envelope).
    let provider = MockSarsProvider::default();

    // Bad VAT registration (does not start with `4`).
    let mut bad_vat = submit_request(b"<Invoice/>".to_vec());
    bad_vat.issuer_vat = "5123456789".to_owned();
    let err = provider.submit_invoice(&bad_vat).unwrap_err();
    assert!(
        matches!(err, SarsError::BadVat(_)),
        "expected BadVat, got {err:?}"
    );

    // Empty payload.
    let empty = submit_request(Vec::new());
    let err = provider.submit_invoice(&empty).unwrap_err();
    assert!(
        matches!(err, SarsError::BadPayload(_)),
        "expected BadPayload, got {err:?}"
    );
}

// ===========================================================================
// Deepened country-specific scenarios (added on top of the honest bar above;
// none of the original tests are weakened or removed).
//
// Every assertion below is grounded in real South African VAT law as
// administered by the South African Revenue Service (SARS):
//
//   * Value-Added Tax Act No. 89 of 1991 ("the VAT Act"). SARS administers
//     VAT under this Act. https://www.sars.gov.za/types-of-tax/value-added-tax/
//   * Standard rate of 15% — VAT Act s 7(1) (rate set by the Minister; 15%
//     since 1 April 2018).
//   * Zero-rated supplies (0%) — VAT Act s 11 (e.g. exports, certain
//     foodstuffs, services to non-residents). A vendor charges 0% yet may
//     still deduct input tax.
//   * Exempt supplies — VAT Act s 12 (e.g. financial services, residential
//     accommodation). No VAT is charged AND no input tax may be deducted —
//     this is legally distinct from zero-rating.
//   * Tax invoice content (supplier + recipient name/address/VAT number,
//     serial number, issue date) — VAT Act s 20; full tax invoice required
//     for consideration above R5 000.
//     https://www.sars.gov.za/businesses-and-employers/government/tax-invoices/
//   * Credit and debit notes — VAT Act s 21.
//   * Supplier VAT registration number: 10 digits beginning with `4`
//     (the SARS VAT vendor number shape the crate's `validate_vat` enforces).
//
// The wire format is EN 16931 / UBL 2.1 (the family path; SARS has no bespoke
// national serializer in-tree). EN 16931 tax-category codes come from UN/EDIFACT
// code list 5305: `S` = standard rate, `Z` = zero rated goods, `E` = exempt
// from tax. The UBL serializer emits these verbatim inside
// `cac:TaxCategory`/`cac:ClassifiedTaxCategory` under a `VAT` `cac:TaxScheme`.
// Goldens stay hand-rolled (no `insta`/`pretty_assertions`).
// ===========================================================================

/// A SARS **credit note** (VAT Act s 21) against the same supplier/customer as
/// [`za_invoice`]. UBL 2.1 maps a credit note to root `<CreditNote>` carrying
/// `<cbc:CreditNoteTypeCode>381</cbc:CreditNoteTypeCode>` (UNTDID 1001 code 381)
/// rather than the invoice code 380. A UBL credit note cannot carry a top-level
/// `cbc:DueDate`, so `due_date` is `None`.
fn za_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote must not carry a top-level cbc:DueDate.
        due_date: None,
        document_number: DocumentNumber::new("CN-2026-ZA-0001").unwrap(),
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Beta Holdings", "4987654321", "Johannesburg"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit: software consulting reversal".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(50000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        // Same 15% standard band as the original invoice, on the credited base.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50000),
            tax_amount: amt(7500),
            tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50000),
            tax_exclusive_amount: amt(50000),
            tax_inclusive_amount: amt(57500),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(57500),
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

/// A two-line invoice mixing the 15% standard rate (`S`, VAT Act s 7) with a
/// zero-rated export line (`Z`, VAT Act s 11). UBL emits one `cac:TaxSubtotal`
/// per VAT band inside `cac:TaxTotal`, each with its own `cbc:Percent`; this
/// proves the per-band summary path South African mixed-basket invoices need
/// (e.g. taxable services plus an exported good).
fn za_multiline_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ZA-0002").unwrap(),
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Beta Holdings", "4987654321", "Johannesburg"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Local consulting (15% standard-rated)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(50000),
                line_extension_amount: amt(100_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Exported goods (zero-rated, VAT Act s 11)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(40000),
                line_extension_amount: amt(40000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(15000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(40000),
                tax_amount: amt(0),
                // Scale-2 zero so cbc:Percent renders "0.00" like the 15% band.
                tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            },
        ],
        monetary_total: MonetaryTotal {
            // 1000.00 standard + 400.00 zero-rated = 1400.00 net; VAT only on
            // the standard band (150.00) -> 1550.00 payable.
            line_extension_amount: amt(140_000),
            tax_exclusive_amount: amt(140_000),
            tax_inclusive_amount: amt(155_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(155_000),
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

/// A fully zero-rated export invoice (VAT Act s 11): the supplier charges 0%
/// VAT yet the supply is still taxable (so input tax remains deductible — the
/// legal distinction from an exempt supply). The taxable base equals the
/// payable total because no VAT is added. UBL category code `Z`.
fn za_zero_rated_export_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-zr-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ZA-0003").unwrap(),
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Foreign Buyer Ltd", "4555666777", "Gqeberha"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Goods exported from RSA (zero-rated)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(200_000),
            line_extension_amount: amt(200_000),
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: amt(200_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(200_000),
            tax_exclusive_amount: amt(200_000),
            tax_inclusive_amount: amt(200_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(200_000),
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

/// An exempt-supply invoice (VAT Act s 12): no VAT is charged and — unlike a
/// zero-rated supply — no input tax may be deducted. EN 16931 / UBL category
/// code `E` (exempt from tax). This proves the `E` band serializes distinctly
/// from the `Z` (zero-rated) band, mirroring the SARS legal distinction.
fn za_exempt_supply_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-za-e2e-ex-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-30").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-29").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ZA-0004").unwrap(),
        currency: Iso4217Code::new("ZAR").unwrap(),
        supplier: za_party("Acme (Pty) Ltd", "4123456789", "Cape Town"),
        customer: za_party("Beta Holdings", "4987654321", "Johannesburg"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Residential accommodation (exempt, VAT Act s 12)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(300_000),
            line_extension_amount: amt(300_000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(300_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(300_000),
            tax_exclusive_amount: amt(300_000),
            tax_inclusive_amount: amt(300_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(300_000),
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

/// Serialize an arbitrary IR document to UBL, submit the bytes to a provider,
/// and assemble the same `.ikb` evidence bundle layout the honest-bar lifecycle
/// uses. Returns `(ikb, ubl_xml, receipt)`. Keeping the transmission context,
/// pinned timestamps, and artefact layout fixed keeps the output byte-stable.
fn bundle_for(
    doc: &CommercialDocument,
    provider: &dyn SarsProvider,
) -> (
    Vec<u8>,
    String,
    invoicekit_report_za_sars::SarsSubmitEnvelope,
) {
    let ubl_xml = to_xml(doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();
    let receipt = provider
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
        serde_json::to_vec(&receipt).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, ubl_xml, receipt)
}

/// A test-local `SarsProvider` that always returns an authority `Rejected`
/// verdict. It runs the same pre-wire `validate_vat`/empty-payload checks the
/// real `MockSarsProvider` runs, then synthesizes a `Rejected` envelope with a
/// populated reason. This exists because the in-tree `MockSarsProvider` has no
/// knob to force a rejection; modelling the refusal verdict locally lets us
/// exercise the documented contract ("the SARS-returned `Rejected` verdict is
/// NOT an `Err` — it's surfaced via `SarsStatus::Rejected` inside the
/// envelope") without weakening the shipped mock.
struct RejectingSarsProvider {
    reason: String,
}

impl SarsProvider for RejectingSarsProvider {
    fn submit_invoice(
        &self,
        request: &SarsSubmitRequest,
    ) -> Result<invoicekit_report_za_sars::SarsSubmitEnvelope, SarsError> {
        // Same pre-wire validators the shipped mock runs.
        invoicekit_report_za_sars::validate_vat(&request.issuer_vat)?;
        if request.payload.is_empty() {
            return Err(SarsError::BadPayload("payload is empty".to_owned()));
        }
        Ok(invoicekit_report_za_sars::SarsSubmitEnvelope {
            sars_ref: "ZA-000000000999".to_owned(),
            status: SarsStatus::Rejected,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some(self.reason.clone()),
        })
    }
}

/// A credit note (VAT Act s 21) must serialize as UBL `<CreditNote>` carrying
/// `cbc:CreditNoteTypeCode` 381, never the invoice code 380, and must not emit
/// a top-level `cbc:DueDate`. The whole offline lifecycle must still submit to
/// SARS and produce a verifiable evidence bundle.
#[test]
fn za_credit_note_serializes_as_ubl_credit_note_381() {
    let doc = za_credit_note();
    let (ikb, ubl, receipt) = bundle_for(&doc, &MockSarsProvider::default());

    // The canonicalizer inlines namespace declarations on the first use of
    // each prefix, so match on the element-name prefix or the value/close form
    // rather than a bare full open tag.
    assert!(
        ubl.contains("<CreditNote"),
        "a credit note must serialize to a UBL <CreditNote> root, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL credit notes carry CreditNoteTypeCode 381, got:\n{ubl}"
    );
    assert!(
        !ubl.contains("</cbc:InvoiceTypeCode>"),
        "a credit note must not carry an InvoiceTypeCode element"
    );
    assert!(
        !ubl.contains("</cbc:DueDate>"),
        "a UBL credit note must not emit a top-level cbc:DueDate"
    );
    // The credit-note quantity element is CreditedQuantity, not InvoicedQuantity.
    assert!(ubl.contains("unitCode=\"EA\">1</cbc:CreditedQuantity>"));
    // The credited base carries the 15% standard band (75.00 VAT on 500.00).
    assert!(ubl.contains(">75.00</cbc:TaxAmount>"));
    assert!(ubl.contains(">15.00</cbc:Percent>"));

    assert_eq!(receipt.status, SarsStatus::Accepted);
    assert!(receipt.sars_ref.starts_with("ZA-"));
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// A mixed-basket invoice (15% standard line + zero-rated export line) must
/// emit one `cac:TaxSubtotal` per VAT band, each with its own `cbc:Percent`,
/// and both UBL line items in document order. This is the South African
/// mixed-rate path (VAT Act s 7 standard rate alongside s 11 zero-rating).
#[test]
fn za_multiline_invoice_emits_per_band_subtotals() {
    let doc = za_multiline_mixed_rate_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, &MockSarsProvider::default());

    // Two line items in document order. The canonicalizer inlines the cac
    // namespace on the first cac element, so count the element-name prefix
    // (open form) rather than a bare full open tag.
    assert_eq!(
        ubl.matches("<cac:InvoiceLine").count(),
        2,
        "a two-line invoice must emit two cac:InvoiceLine blocks"
    );
    assert!(ubl.contains("Local consulting (15% standard-rated)"));
    assert!(ubl.contains("Exported goods (zero-rated, VAT Act s 11)"));

    // One TaxSubtotal per band: 15% on 1000.00 -> 150.00, 0% on 400.00 -> 0.00.
    assert_eq!(
        ubl.matches("<cac:TaxSubtotal").count(),
        2,
        "a mixed-rate invoice must emit one cac:TaxSubtotal per VAT band"
    );
    assert!(ubl.contains(">15.00</cbc:Percent>"));
    assert!(ubl.contains(">0.00</cbc:Percent>"));
    // Both category codes present under the VAT scheme.
    assert!(ubl.contains(">S</cbc:ID>"));
    assert!(ubl.contains(">Z</cbc:ID>"));
    assert!(ubl.contains(">VAT</cbc:ID>"));

    assert_eq!(receipt.status, SarsStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

/// A fully zero-rated export invoice (VAT Act s 11): 0% VAT, taxable base
/// equals payable total (no VAT added), category code `Z`. The 15% standard
/// band must NOT appear.
#[test]
fn za_zero_rated_export_charges_no_vat() {
    let doc = za_zero_rated_export_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, &MockSarsProvider::default());

    assert!(
        ubl.contains(">0.00</cbc:Percent>"),
        "a zero-rated line must carry a 0.00 VAT percent, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">Z</cbc:ID>"),
        "an exported supply must use the zero-rated category code Z"
    );
    // Taxable base equals payable total (no VAT added): 2000.00.
    assert!(ubl.contains(">2000.00</cbc:TaxExclusiveAmount>"));
    assert!(ubl.contains(">2000.00</cbc:TaxInclusiveAmount>"));
    assert!(ubl.contains(">2000.00</cbc:PayableAmount>"));
    // The 15% standard band must not surface on a fully zero-rated invoice.
    assert!(!ubl.contains(">15.00</cbc:Percent>"));

    assert_eq!(receipt.status, SarsStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "zero-rated evidence bundle must verify");
}

/// An exempt supply (VAT Act s 12) is legally distinct from a zero-rated one:
/// EN 16931 / UBL uses category code `E` (exempt), not `Z` (zero-rated). The
/// `E` band must serialize and the `Z`/`S` bands must not appear.
#[test]
fn za_exempt_supply_uses_exempt_category_e() {
    let doc = za_exempt_supply_invoice();
    let (ikb, ubl, receipt) = bundle_for(&doc, &MockSarsProvider::default());

    assert!(
        ubl.contains(">E</cbc:ID>"),
        "an exempt supply must use the exempt category code E, got:\n{ubl}"
    );
    assert!(
        !ubl.contains(">Z</cbc:ID>"),
        "an exempt supply is not zero-rated; the Z code must not appear"
    );
    assert!(
        !ubl.contains(">S</cbc:ID>"),
        "an exempt supply is not standard-rated; the S code must not appear"
    );
    // No VAT charged: exclusive == inclusive == payable == 3000.00.
    assert!(ubl.contains(">3000.00</cbc:TaxExclusiveAmount>"));
    assert!(ubl.contains(">3000.00</cbc:TaxInclusiveAmount>"));

    assert_eq!(receipt.status, SarsStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "exempt-supply evidence bundle must verify");
}

/// SARS authority **rejection** path. The crate's documented contract is that a
/// SARS-returned `Rejected` verdict is NOT an `Err`: it is surfaced via
/// `SarsStatus::Rejected` inside the envelope (with a reason), so the engine
/// persists the rejection alongside its audit trail. The evidence bundle of a
/// rejected submission must still assemble and verify. (The shipped
/// `MockSarsProvider` has no knob to force this, so a test-local provider
/// models the verdict while running the same pre-wire validators.)
#[test]
fn za_authority_rejection_is_receipt_status_not_error() {
    let provider = RejectingSarsProvider {
        reason: "supplier VAT registration not active on SARS eFiling".to_owned(),
    };
    let doc = za_invoice();
    let (ikb, _ubl, receipt) = bundle_for(&doc, &provider);

    // Rejection is a verdict, returned as Ok(envelope), never Err.
    assert_eq!(
        receipt.status,
        SarsStatus::Rejected,
        "a SARS refusal must be surfaced as SarsStatus::Rejected, not Err"
    );
    assert_eq!(
        receipt.reason.as_deref(),
        Some("supplier VAT registration not active on SARS eFiling"),
        "a rejected receipt must carry the SARS refusal reason"
    );
    assert!(
        receipt.sars_ref.starts_with("ZA-"),
        "even a rejected receipt carries a ZA-prefixed reference"
    );

    // The rejection still produces a verifiable audit-trail bundle.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejected-submission evidence bundle must verify");
}

/// A malformed recipient/issuer VAT registration is a pre-wire `Err`
/// (`BadVat`), distinct from a SARS authority rejection. The SARS VAT vendor
/// number is exactly 10 digits beginning with `4`; the error message names the
/// exact shape rule so operators can self-correct. This is an invalid-identifier
/// refusal, surfaced before the wire — not a `Rejected` envelope.
#[test]
fn za_invalid_vat_identifier_is_pre_wire_error_with_shape_reason() {
    let provider = MockSarsProvider::default();

    // 11 digits (too long) — a real over-length SARS VAT number typo.
    let mut too_long = submit_request(b"<Invoice/>".to_vec());
    too_long.issuer_vat = "41234567890".to_owned();
    let err = provider.submit_invoice(&too_long).unwrap_err();
    assert!(
        matches!(&err, SarsError::BadVat(msg) if msg.contains("10 ASCII digits starting with `4`")),
        "expected BadVat naming the SARS VAT shape rule for an 11-digit VAT, got {err:?}"
    );

    // A registration that does not begin with `4` (SARS VAT numbers always do).
    assert!(
        invoicekit_report_za_sars::validate_vat("5123456789").is_err(),
        "a SARS VAT number must begin with 4"
    );
    // A non-digit character must be rejected.
    assert!(
        invoicekit_report_za_sars::validate_vat("412345678A").is_err(),
        "a SARS VAT number is all digits"
    );
    // The canonical valid shape still passes.
    assert!(invoicekit_report_za_sars::validate_vat("4123456789").is_ok());
}

/// Determinism for the deepened document shapes: the credit note and the
/// mixed-rate multi-line invoice must each produce byte-identical UBL and
/// byte-identical `.ikb` across runs. Determinism is load-bearing for the
/// evidence bundle's content address (per-band subtotal ordering and per-line
/// ordering must not vary between runs).
#[test]
fn za_deepened_lifecycles_are_byte_deterministic() {
    for doc in [za_credit_note(), za_multiline_mixed_rate_invoice()] {
        let (ikb_a, ubl_a, _) = bundle_for(&doc, &MockSarsProvider::default());
        let (ikb_b, ubl_b, _) = bundle_for(&doc, &MockSarsProvider::default());
        assert_eq!(ubl_a, ubl_b, "UBL serialization must be byte-stable");
        assert_eq!(ikb_a, ikb_b, "the whole offline lifecycle must be byte-stable");
    }
}
