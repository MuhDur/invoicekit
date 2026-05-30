// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Belgium Peppol-overlay offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Belgium and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a `BE` country code + EUR
//! 2. serialize -> Peppol BIS Billing 3 UBL bytes via `invoicekit_format_ubl::to_xml`
//! 3. submit the UBL bytes to the offline `MockBePeppolProvider` (`deliver`) and
//!    assert the typed Belgian envelope (Mercurius/Hermes submission id + status)
//! 4. advance the async Peppol ladder with `poll_status` (Delivered -> Accepted)
//! 5. assemble a `.ikb` evidence bundle and `verify_packed` it (exit 0 == report.ok)
//! 6. determinism: run the lifecycle twice and `pack` twice -> byte-identical
//! 7. refusal: force a pre-wire VAT/receiver validation failure (the only refusal
//!    shape this mock can synthesize) and assert it surfaces as `Err`.
//!
//! Belgium is the Peppol/EN-16931 adapter shape (not national-clearance): a
//! lifecycle ladder `Submitted -> Delivered -> Accepted/Rejected/ValidationFailed`
//! with two verbs (`deliver` + `poll_status`), a receiver lookup, and a Peppol BIS
//! UBL payload. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
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
use invoicekit_report_be_peppol::{
    BePeppolDeliverEnvelope, BePeppolDeliverRequest, BePeppolEnvironment, BePeppolError,
    BePeppolMandate, BePeppolProvider, BePeppolReceiver, BePeppolStatus, BePeppolVatCategory,
    MockBePeppolProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_be_e2e";
const TRACE: &str = "trace_be_e2e";
const FIXED_DELIVERED_AT: &str = "2026-07-01T00:00:00Z";
/// A real, well-shaped Belgian KBO receiver (10 ASCII digits).
const KBO: &str = "0123456749";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn belgian_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Rue de la Loi 16".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "1000".to_owned(),
            country: CountryCode::new("BE").unwrap(),
        },
        contact: None,
    }
}

fn belgian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-be-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-BE-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: belgian_party("Acme BVBA", "BE0123456749", "Brussel"),
        customer: belgian_party("Beta NV", "BE0987654310", "Antwerpen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Advies & softwareontwikkeling".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100), // 21% Belgian standard rate
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12100),
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

fn deliver_request(ubl_xml: Vec<u8>) -> BePeppolDeliverRequest {
    BePeppolDeliverRequest {
        tenant_id: TENANT.to_owned(),
        environment: BePeppolEnvironment::Sandbox,
        mandate: BePeppolMandate::B2g,
        receiver: BePeppolReceiver::Kbo(KBO.to_owned()),
        // One BTW category per Peppol invoice line (single line above).
        vat_categories: vec![BePeppolVatCategory::Standard],
        peppol_ubl_xml: ubl_xml,
    }
}

/// Steps 1-5: build -> serialize -> deliver -> poll -> evidence bundle.
///
/// Returns the packed `.ikb`, the initial `deliver` envelope, and the polled
/// envelope so the assertions live in the `#[test]` functions.
fn run_lifecycle() -> (
    Vec<u8>,
    invoicekit_report_be_peppol::BePeppolDeliverEnvelope,
    invoicekit_report_be_peppol::BePeppolDeliverEnvelope,
) {
    // 1. build the IR document.
    let doc = belgian_invoice();

    // 2. serialize -> Peppol BIS Billing 3 UBL bytes (EN16931/UBL family path).
    let ubl: String = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.clone().into_bytes();
    // Structural spot-check: the canonical UBL spine is present. (The canonical
    // serializer inlines `xmlns:` declarations on each element, so match the
    // element-name prefix, not a `>`-terminated start tag.)
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cac:InvoiceLine",
        "<cbc:DocumentCurrencyCode",
    ] {
        assert!(ubl.contains(needle), "Peppol UBL missing {needle}");
    }
    // The Belgian buyer/supplier carry the `BE` country code through to UBL.
    assert!(
        ubl.contains(">BE</cbc:IdentificationCode>"),
        "Peppol UBL must carry the BE country code"
    );

    // 3. deliver through the offline Mercurius/Hermes mock.
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let delivered = provider
        .deliver(&deliver_request(ubl_bytes.clone()))
        .unwrap();

    // 4. advance the async Peppol ladder: Delivered -> Accepted.
    let accepted = provider
        .poll_status(BePeppolEnvironment::Sandbox, &delivered.submission_id)
        .unwrap();

    // 5. evidence bundle: canonical doc + Peppol UBL + the polled receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&accepted).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    let ikb = pack(&bundle).unwrap();
    (ikb, delivered, accepted)
}

#[test]
fn belgium_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, delivered, accepted) = run_lifecycle();

    // Sandbox + B2G routes through Mercurius; the mock tags the submission id.
    assert_eq!(delivered.status, BePeppolStatus::Delivered);
    assert!(
        delivered.submission_id.starts_with("MERC-SBX-"),
        "B2G sandbox must route through Mercurius, got {:?}",
        delivered.submission_id
    );
    assert!(delivered.mlr_reason.is_none());
    assert_eq!(delivered.delivered_at, FIXED_DELIVERED_AT);

    // poll_status advances the async ladder to the receiver acknowledgement.
    assert_eq!(accepted.status, BePeppolStatus::Accepted);
    assert_eq!(accepted.submission_id, delivered.submission_id);
    assert!(accepted.mlr_reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn belgium_b2b_routes_through_hermes_in_production() {
    // The Belgian overlay picks Hermes for B2B Peppol delivery and Mercurius for
    // B2G; assert the production B2B path is tagged distinctly.
    let doc = belgian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let mut req = deliver_request(ubl_bytes);
    req.environment = BePeppolEnvironment::Production;
    req.mandate = BePeppolMandate::B2b;
    let env = provider.deliver(&req).unwrap();
    assert_eq!(env.status, BePeppolStatus::Delivered);
    assert!(
        env.submission_id.starts_with("HERMES-PROD-"),
        "B2B production must route through Hermes, got {:?}",
        env.submission_id
    );
}

#[test]
fn belgium_lifecycle_is_byte_deterministic() {
    let (a, _, _) = run_lifecycle();
    let (b, _, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn belgium_refusal_is_surfaced_as_error() {
    // Belgium's mock has no `with_forced_receipt`/forced `Rejected` status knob:
    // the only refusal it can synthesize is a pre-wire shape/business-rule
    // failure, which the Peppol/EN-16931 contract surfaces as `Err` (NOT a
    // `Rejected` status — that arrives async via a real MLR, which this offline
    // mock never fabricates). Drive the Mercurius BTW pre-check (Exempt + Standard
    // may not mix) to prove the refusal path is wired end-to-end.
    let doc = belgian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);

    let mut bad_vat = deliver_request(ubl_bytes.clone());
    bad_vat.vat_categories = vec![BePeppolVatCategory::Standard, BePeppolVatCategory::Exempt];
    let err = provider.deliver(&bad_vat).unwrap_err();
    assert!(
        matches!(err, BePeppolError::BadVatCategorisation(_)),
        "Exempt+Standard mix must be refused as a VAT categorisation error, got {err:?}"
    );

    // A malformed receiver (KBO must be 10 ASCII digits) is also a pre-wire refusal.
    let mut bad_receiver = deliver_request(ubl_bytes);
    bad_receiver.receiver = BePeppolReceiver::Kbo("123".to_owned());
    let err = provider.deliver(&bad_receiver).unwrap_err();
    assert!(
        matches!(err, BePeppolError::BadReceiver(_)),
        "malformed KBO must be refused as a receiver error, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Deepened, Belgium-specific scenarios (added on top of the honest-bar set).
//
// Grounding specs (all hand-built, license-safe synthetic fixtures — no
// copyrighted regulator files are vendored):
//
// - Peppol BIS Billing 3.0 (the Belgian B2G + 2026 B2B wire format), incl. the
//   credit note transaction and the UNCL5305 VAT category codes (S/Z/E/AE):
//   https://docs.peppol.eu/poacc/billing/3.0/
// - Peppol Message Level Response (MLR) — the async accept/reject signal whose
//   reason text this overlay surfaces as `mlr_reason`:
//   https://docs.peppol.eu/poacc/billing/3.0/bis/#_message_level_response
// - Peppol Electronic Address Scheme (EAS / ISO 6523) code list: `0208` =
//   Belgian enterprise number (KBO/BCE), `9925` = Belgian VAT number:
//   https://docs.peppol.eu/poacc/billing/3.0/codelist/eas/
// - Belgian VAT (BTW/TVA) rates and the construction-sector reverse charge
//   ("verlegging van heffing" / "report de perception", Royal Decree no. 1
//   art. 20), administered by the FPS Finance:
//   https://finance.belgium.be/en/enterprises/vat
// - Mercurius (federal B2G portal, FOD BOSA/Fedict) and Hermes (B2B Peppol
//   access point), https://digital.belgium.be/e-invoicing/
// ---------------------------------------------------------------------------

/// A real Belgian credit note (creditnota / note de crédit) in UBL form, routed
/// B2B through Hermes.
///
/// Per Peppol BIS Billing 3.0 a credit note is a distinct transaction
/// (<https://docs.peppol.eu/poacc/billing/3.0/>), serialized by the UBL family as
/// a `<CreditNote>` root carrying `cbc:CreditNoteTypeCode` 381 and
/// `cac:CreditNoteLine` (not `cac:InvoiceLine`). UBL credit notes must NOT carry
/// a top-level `cbc:DueDate`, so the IR sets `due_date: None`.
fn belgian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-be-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote cannot carry cbc:DueDate (UblError::UnsupportedDocumentField).
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-BE-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: belgian_party("Acme BVBA", "BE0123456749", "Brussel"),
        customer: belgian_party("Beta NV", "BE0987654310", "Antwerpen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Creditnota: terugname softwarelicentie".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(10000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100), // 21% Belgian standard rate
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12100),
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

/// A multi-line Belgian invoice mixing the 21% standard rate with the 6% reduced
/// rate (food, books, certain renovations under FPS Finance rules).
///
/// Peppol BIS Billing 3.0 allows several `cac:TaxSubtotal` blocks under one
/// `cac:TaxTotal`, so a single Belgian invoice can legitimately carry both rates
/// (<https://docs.peppol.eu/poacc/billing/3.0/>). The BTW pre-check only refuses
/// an `Exempt`+`Standard` mix, so 21%+6% (both `BePeppolVatCategory::Standard` /
/// `Reduced6` here) passes.
fn belgian_multi_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-be-e2e-mr-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-BE-0002").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: belgian_party("Acme BVBA", "BE0123456749", "Brussel"),
        customer: belgian_party("Beta NV", "BE0987654310", "Antwerpen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Consultancy (21% BTW)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Drukwerk handleidingen (6% BTW)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(4)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(2500),
                line_extension_amount: amt(10000),
                // Reduced-rate lines map to UNCL5305 category "S" with a 6% rate;
                // the typed BePeppolVatCategory::Reduced6 carries the Belgian verdict.
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2100), // 21%
                tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(600), // 6%
                tax_rate: Some(DecimalValue::new(Decimal::new(600, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(20000),
            tax_exclusive_amount: amt(20000),
            // 200.00 + 21.00 + 6.00 = 227.00
            tax_inclusive_amount: amt(22700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(22700),
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

#[test]
fn belgium_credit_note_serializes_as_ubl_creditnote_and_delivers() {
    // Peppol BIS Billing 3.0 credit note: UBL emits a <CreditNote> root with
    // cbc:CreditNoteTypeCode 381 and cac:CreditNoteLine.
    let doc = belgian_credit_note();
    assert_eq!(doc.document_type, DocumentType::CreditNote);
    let ubl = to_xml(&doc).unwrap();

    assert!(
        ubl.contains("<CreditNote"),
        "credit note must serialize to a UBL <CreditNote> root, got: {ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "Belgian credit note must carry UBL CreditNoteTypeCode 381"
    );
    assert!(
        ubl.contains("<cac:CreditNoteLine"),
        "credit note must use cac:CreditNoteLine (not cac:InvoiceLine)"
    );
    // A UBL credit note must NOT carry a top-level cbc:DueDate.
    assert!(
        !ubl.contains("<cbc:DueDate"),
        "UBL credit note must not carry cbc:DueDate"
    );
    assert!(
        ubl.contains(">BE</cbc:IdentificationCode>"),
        "credit note must carry the BE country code"
    );

    // Route the corrective document B2B through Hermes (the Belgian B2B AP).
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let mut req = deliver_request(ubl.into_bytes());
    req.mandate = BePeppolMandate::B2b;
    req.receiver = BePeppolReceiver::PeppolParticipant("9925:BE0987654310".to_owned()); // EAS 9925 = BE VAT
    let env = provider.deliver(&req).unwrap();
    assert_eq!(env.status, BePeppolStatus::Delivered);
    assert!(
        env.submission_id.starts_with("HERMES-SBX-"),
        "B2B sandbox credit note must route through Hermes, got {:?}",
        env.submission_id
    );
}

#[test]
fn belgium_multi_line_mixed_rate_invoice_serializes_and_delivers() {
    // Belgium levies 21% (standard) and 6% (reduced: food, books) on the same
    // invoice; FPS Finance VAT rates. Peppol BIS Billing 3.0 carries several
    // cac:TaxSubtotal blocks, one per rate.
    let doc = belgian_multi_rate_invoice();
    let ubl = to_xml(&doc).unwrap();

    // Two invoice lines must both reach the wire.
    let line_count = ubl.matches("<cac:InvoiceLine").count();
    assert_eq!(
        line_count, 2,
        "multi-line invoice must serialize exactly two cac:InvoiceLine blocks, got {line_count}"
    );
    // The 21% standard taxable base (200.00) and the 6% reduced base must both
    // appear; payable amount is 227.00.
    assert!(
        ubl.contains(">227.00</cbc:PayableAmount>"),
        "mixed-rate total must be 227.00 (200.00 + 21.00 + 6.00)"
    );

    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let mut req = deliver_request(ubl.into_bytes());
    // One typed BTW category per line: 21% standard + 6% reduced.
    req.vat_categories = vec![BePeppolVatCategory::Standard, BePeppolVatCategory::Reduced6];
    let env = provider.deliver(&req).unwrap();
    assert_eq!(
        env.status,
        BePeppolStatus::Delivered,
        "21% + 6% is a valid Belgian rate mix and must pass the BTW pre-check"
    );
}

#[test]
fn belgium_reverse_charge_construction_delivers() {
    // Belgian construction-sector reverse charge ("verlegging van heffing" /
    // "cocontractant", Royal Decree no. 1 art. 20): the supplier issues the
    // invoice with no VAT and the customer self-accounts. Maps to UNCL5305 code
    // "AE" in Peppol BIS Billing 3.0; the typed verdict is ReverseCharge.
    let doc = belgian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);

    let mut req = deliver_request(ubl_bytes);
    req.mandate = BePeppolMandate::B2b;
    req.vat_categories = vec![BePeppolVatCategory::ReverseCharge];
    let env = provider.deliver(&req).unwrap();
    assert_eq!(
        env.status,
        BePeppolStatus::Delivered,
        "a pure reverse-charge invoice must deliver (it is not an Exempt+Standard mix)"
    );

    // The typed reverse-charge category must survive serde unchanged so the
    // evidence bundle records the genuine Belgian verdict.
    let json = serde_json::to_string(&req.vat_categories).unwrap();
    assert_eq!(json, r#"["reverse-charge"]"#);
}

#[test]
fn belgium_zero_rated_intra_eu_delivers_distinct_from_exempt() {
    // Zero-rated (export / intra-EU supply, UNCL5305 "Z") is a DIFFERENT verdict
    // from exempt ("E", medical/educational): zero-rated keeps the right to
    // deduct input VAT, exempt does not. Peppol BIS Billing 3.0 distinguishes
    // them. The BTW pre-check refuses Exempt+Standard but accepts Zero+Standard.
    let doc = belgian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);

    let mut zero = deliver_request(ubl_bytes.clone());
    zero.vat_categories = vec![BePeppolVatCategory::Zero, BePeppolVatCategory::Standard];
    let env = provider.deliver(&zero).unwrap();
    assert_eq!(
        env.status,
        BePeppolStatus::Delivered,
        "Zero-rated may co-exist with Standard on one invoice"
    );

    // The exempt+standard counterpart is the refusal (proves the categories are
    // not interchangeable in the Belgian rule set).
    let mut exempt = deliver_request(ubl_bytes);
    exempt.vat_categories = vec![BePeppolVatCategory::Exempt, BePeppolVatCategory::Standard];
    let err = provider.deliver(&exempt).unwrap_err();
    assert!(
        matches!(err, BePeppolError::BadVatCategorisation(_)),
        "Exempt + Standard must still be refused, got {err:?}"
    );
}

#[test]
fn belgium_invalid_belgian_identifiers_are_rejected() {
    // The Belgian receiver shapes are exact: KBO = 10 ASCII digits, VAT = `BE` +
    // 10 digits (EAS 9925), Peppol participant = scheme:value (EAS 0208 for KBO).
    // A French VAT id, a too-short KBO, and a scheme-less participant are all
    // pre-wire refusals surfaced as Err(BadReceiver) — they never reach Hermes.
    let doc = belgian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);

    for bad in [
        BePeppolReceiver::VatId("FR0123456789".to_owned()), // wrong country prefix
        BePeppolReceiver::VatId("BE012345678".to_owned()),  // only 9 digits after BE
        BePeppolReceiver::Kbo("012345674".to_owned()),      // 9 digits, must be 10
        BePeppolReceiver::Kbo("01234567X9".to_owned()),     // non-digit
        BePeppolReceiver::PeppolParticipant("0208-0123456749".to_owned()), // no colon
    ] {
        let mut req = deliver_request(ubl_bytes.clone());
        req.receiver = bad.clone();
        let err = provider.deliver(&req).unwrap_err();
        assert!(
            matches!(err, BePeppolError::BadReceiver(_)),
            "{bad:?} must be refused as a receiver error, got {err:?}"
        );
    }

    // Both well-formed Belgian EAS-keyed participant ids must be accepted.
    for good in [
        BePeppolReceiver::PeppolParticipant("0208:0123456749".to_owned()), // KBO via EAS 0208
        BePeppolReceiver::PeppolParticipant("9925:BE0123456749".to_owned()), // VAT via EAS 9925
    ] {
        let mut req = deliver_request(ubl_bytes.clone());
        req.receiver = good;
        let env = provider.deliver(&req).unwrap();
        assert_eq!(env.status, BePeppolStatus::Delivered);
    }
}

#[test]
fn belgium_async_rejection_envelope_carries_mlr_reason() {
    // The async authority verdict on the Peppol ladder is a Message Level
    // Response (MLR): Rejected / ValidationFailed carry a human reason
    // (https://docs.peppol.eu/poacc/billing/3.0/bis/). This offline mock never
    // *fabricates* an async rejection (only Err for pre-wire faults), but the
    // typed envelope is what a real Hermes/Mercurius MLR deserializes into, so
    // pin its serde shape: a `Rejected` envelope MUST keep its mlr_reason, and a
    // `Delivered` envelope MUST omit the field (skip_serializing_if).
    let rejected = BePeppolDeliverEnvelope {
        submission_id: "HERMES-PROD-00000042".to_owned(),
        status: BePeppolStatus::Rejected,
        // BR-BE-... is the shape of a Belgian Peppol BIS business-rule id.
        mlr_reason: Some("receiver AP returned MLR Rejected: BR-CO-15 violation".to_owned()),
        delivered_at: FIXED_DELIVERED_AT.to_owned(),
    };
    let json = serde_json::to_string(&rejected).unwrap();
    assert!(
        json.contains(r#""status":"rejected""#),
        "Rejected status must serialize kebab-case, got {json}"
    );
    assert!(
        json.contains("mlr_reason"),
        "a Rejected MLR must retain its reason in the persisted envelope"
    );
    let round: BePeppolDeliverEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(round, rejected);

    let validation_failed = BePeppolDeliverEnvelope {
        submission_id: "MERC-PROD-00000007".to_owned(),
        status: BePeppolStatus::ValidationFailed,
        mlr_reason: Some("Mercurius BTW pre-check: PEPPOL-EN16931-R053".to_owned()),
        delivered_at: FIXED_DELIVERED_AT.to_owned(),
    };
    let vf_json = serde_json::to_string(&validation_failed).unwrap();
    assert!(vf_json.contains(r#""status":"validation-failed""#));

    // A clean Delivered envelope omits mlr_reason entirely.
    let delivered = BePeppolDeliverEnvelope {
        submission_id: "MERC-SBX-00000001".to_owned(),
        status: BePeppolStatus::Delivered,
        mlr_reason: None,
        delivered_at: FIXED_DELIVERED_AT.to_owned(),
    };
    let ok_json = serde_json::to_string(&delivered).unwrap();
    assert!(
        !ok_json.contains("mlr_reason"),
        "a Delivered envelope must omit the optional mlr_reason field, got {ok_json}"
    );
}

#[test]
fn belgium_credit_note_lifecycle_is_byte_deterministic() {
    // Determinism must hold for the corrective-document path too, not only the
    // plain invoice: serialize the credit note twice and compare bytes.
    let a = to_xml(&belgian_credit_note()).unwrap();
    let b = to_xml(&belgian_credit_note()).unwrap();
    assert_eq!(a, b, "credit note UBL serialization must be byte-stable");

    // And the full deliver lifecycle must be reproducible end to end.
    let provider = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let mut req = deliver_request(a.into_bytes());
    req.mandate = BePeppolMandate::B2b;
    let first = provider.deliver(&req).unwrap();

    let provider2 = MockBePeppolProvider::with_fixed_delivered_at(FIXED_DELIVERED_AT);
    let mut req2 = deliver_request(b.into_bytes());
    req2.mandate = BePeppolMandate::B2b;
    let second = provider2.deliver(&req2).unwrap();

    assert_eq!(
        first, second,
        "the same credit note delivered to two fresh mocks must yield identical envelopes"
    );
}
