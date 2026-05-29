// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! India GST / IRP offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for India and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("IN")` + INR
//! 2. serialize -> EN 16931 / UBL 2.1 XML (the family path; India layers GST on
//!    top of an EN 16931-shaped invoice)
//! 3. submit the serialized bytes to the crate's existing `MockIrpProvider` and
//!    assert the IRP receipt's India-specific fields (IRN / ack no / signed QR /
//!    signed JWS / status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock rejects a malformed GSTIN before the wire (`Err`)
//!
//! The `MockIrpProvider` synthesises the IRN/QR/JWS itself, so no `Signer` is
//! wired here. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
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
use invoicekit_report_in_gst::{
    IrpBackend, IrpEnvironment, IrpError, IrpProvider, IrpRegisterRequest, IrpStatus,
    MockIrpProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_in_e2e";
const TRACE: &str = "trace_in_e2e";
const ISSUER_GSTIN: &str = "29AAAPL2356Q1ZS";
const BUYER_GSTIN: &str = "27AAAPL2356Q1ZT";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn indian_party(name: &str, gstin: &str, city: &str, state: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            // India's GST identity. `vat` is the IR's generic tax-scheme slot.
            scheme: "gst".to_owned(),
            value: gstin.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 MG Road".to_owned()],
            city: city.to_owned(),
            subdivision: Some(state.to_owned()),
            postal_code: "560001".to_owned(),
            country: CountryCode::new("IN").unwrap(),
        },
        contact: None,
    }
}

fn indian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-IN-0001").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier: indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA"),
        customer: indian_party("Beta Solutions Pvt Ltd", BUYER_GSTIN, "Mumbai", "MH"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting services (SAC 998314)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(500_000),            // 5000.00
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // 18% GST (9% CGST + 9% SGST collapses to one EN16931 summary line).
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000),     // 10000.00
            tax_amount: amt(180_000),           // 1800.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))), // 18.00
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_exclusive_amount: amt(1_000_000),  // 10000.00
            tax_inclusive_amount: amt(1_180_000),  // 11800.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_180_000), // 11800.00
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

fn register_request(invoice_json: Vec<u8>) -> IrpRegisterRequest {
    IrpRegisterRequest {
        tenant_id: TENANT.to_owned(),
        environment: IrpEnvironment::Sandbox,
        backend: IrpBackend::Nic1,
        issuer_gstin: ISSUER_GSTIN.to_owned(),
        buyer_gstin: Some(BUYER_GSTIN.to_owned()),
        invoice_json,
    }
}

/// Steps 1-4: build -> serialize -> submit to IRP -> assemble `.ikb`.
///
/// Returns the packed bundle bytes plus the IRP receipt so the callers can
/// assert both the India-specific receipt fields and bundle verifiability.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_in_gst::IrpRegisterEnvelope) {
    // 1. build
    let doc = indian_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 XML bytes
    let ubl_xml = to_xml(&doc).unwrap();
    // local structural check: the canonical UBL spine is present. The C14N
    // pass relocates namespace declarations onto each element's first use, so
    // assert on prefix-stable substrings, not bare `<cac:X>` tags.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">INR</cbc:DocumentCurrencyCode>",
        "currencyID=\"INR\">11800.00</cbc:PayableAmount>",
        // The issuer's GSTIN survives the round-trip into the tax-scheme block.
        "29AAAPL2356Q1ZS</cbc:CompanyID>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit the serialized bytes to the IRP mock (it signs + assigns IRN).
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
    let envelope = provider.register_invoice(&register_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national-family XML + IRP receipt.
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
    let ikb = pack(&bundle).unwrap();
    (ikb, envelope)
}

#[test]
fn india_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // India-specific authority receipt fields.
    assert_eq!(envelope.status, IrpStatus::Accepted);
    let irn = envelope.irn.as_ref().expect("IRN present on Accepted");
    assert_eq!(irn.len(), 64, "IRN is a 64-char SHA-256 hex");
    assert!(
        irn.bytes().all(|b| b.is_ascii_hexdigit()),
        "IRN must be hex"
    );
    assert!(
        envelope.ack_no.as_ref().is_some_and(|s| s.starts_with("ACK-")),
        "IRP acknowledgement number present"
    );
    assert_eq!(envelope.ack_dt, PINNED_CREATED_AT);
    assert!(
        envelope.signed_qr_code.is_some(),
        "signed QR for the printed invoice"
    );
    assert!(
        envelope.signed_invoice_jws.is_some(),
        "JWS for offline verification"
    );
    assert!(envelope.error_message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn india_lifecycle_is_byte_deterministic() {
    let (a, env_a) = run_lifecycle();
    let (b, env_b) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
    // Same payload -> same synthesised IRN across independent provider instances.
    assert_eq!(env_a.irn, env_b.irn, "IRN derivation must be deterministic");
}

#[test]
fn india_duplicate_resubmit_is_reported_not_errored() {
    // Resubmitting the same payload to the SAME provider yields a Duplicate
    // verdict (the IRP returns the existing IRN) — surfaced as a status, not an
    // `Err`, so the audit trail records the reconciliation. The bundle from
    // either submission still verifies.
    let doc = indian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    let first = provider.register_invoice(&register_request(ubl_bytes.clone())).unwrap();
    let second = provider.register_invoice(&register_request(ubl_bytes)).unwrap();

    assert_eq!(first.status, IrpStatus::Accepted);
    assert_eq!(second.status, IrpStatus::Duplicate);
    assert_eq!(first.irn, second.irn, "Duplicate returns the existing IRN");
}

#[test]
fn india_refuses_malformed_gstin_before_the_wire() {
    // The MockIrpProvider does NOT expose a force-rejection hook, so an
    // authority `IrpStatus::Rejected` verdict cannot be synthesised offline.
    // The refusal path the mock DOES support is pre-wire shape validation: a
    // malformed GSTIN is an `Err`, never a packed bundle.
    let doc = indian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    let mut bad = register_request(ubl_bytes);
    bad.issuer_gstin = "TOO-SHORT".to_owned();
    let err = provider.register_invoice(&bad).unwrap_err();
    assert!(matches!(err, IrpError::BadGstin(_)), "got {err:?}");
}
