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
    BePeppolDeliverRequest, BePeppolEnvironment, BePeppolError, BePeppolMandate, BePeppolProvider,
    BePeppolReceiver, BePeppolStatus, BePeppolVatCategory, MockBePeppolProvider,
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
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2100), // 21% Belgian standard rate
            tax_rate: Some(DecimalValue::new(Decimal::new(2100, 2))),
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
    let delivered = provider.deliver(&deliver_request(ubl_bytes.clone())).unwrap();

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
    let bundle = EvidenceBundle { manifest, artefacts };
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
