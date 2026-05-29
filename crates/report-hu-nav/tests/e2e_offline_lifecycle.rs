// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Hungary NAV Online Számla offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Hungary and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Hungarian supplier +
//!    customer and the forint (HUF) currency
//! 2. serialize -> UBL 2.1 XML (the EN 16931 family path; NAV's wire payload is
//!    a typed `manageInvoiceRequest` wrapper around the invoice, and this crate
//!    exposes no national serializer, so we ship the UBL bytes as the payload)
//! 3. submit those bytes to the existing offline `MockNavProvider`
//!    (`manage_invoice`), asserting NAV's country-specific receipt fields:
//!    the `NAV-` transaction id, `Received` status, and the recorded timestamp
//! 4. poll `query_transaction` to reach the terminal `Done` status — the real
//!    two-step NAV async lifecycle (submit -> Received, poll -> Done)
//! 5. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true` (exit 0)
//! 6. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 7. refusal paths: NAV's mock refuses an empty payload (`BadXml`) and a
//!    malformed adóazonosító (`BadTaxId`) as typed `Err`s before the wire
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). The HU NAV mock has no signing layer, so no signer is wired.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_hu_nav::{
    MockNavProvider, NavEnvironment, NavManageRequest, NavOperation, NavProvider, NavStatus,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_hu_e2e";
const TRACE: &str = "trace_hu_e2e";
const FIXED_RECORDED_AT: &str = "2026-01-01T00:00:00Z";
/// 8-digit adóazonosító + 1 check digit + 2-digit area code, hyphenated as
/// NAV writes it on the portal: `12345678-2-41`.
const ISSUER_TAX_ID: &str = "12345678-2-41";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn hungarian_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Andrássy út 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "1061".to_owned(),
            country: CountryCode::new("HU").unwrap(),
        },
        contact: None,
    }
}

fn hungarian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-hu-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-HU-0001").unwrap(),
        currency: Iso4217Code::new("HUF").unwrap(),
        supplier: hungarian_party("Acme Kft", "HU12345678", "Budapest"),
        customer: hungarian_party("Beta Zrt", "HU98765432", "Debrecen"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Szoftverfejlesztési tanácsadás".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // Hungary's standard VAT is 27% (the highest in the EU).
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2700),
            tax_rate: Some(DecimalValue::new(Decimal::new(2700, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12700),
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

fn manage_request(manage_invoice_xml: Vec<u8>) -> NavManageRequest {
    NavManageRequest {
        tenant_id: TENANT.to_owned(),
        environment: NavEnvironment::Test,
        operation: NavOperation::Create,
        issuer_tax_id: ISSUER_TAX_ID.to_owned(),
        manage_invoice_xml,
    }
}

/// Steps 1-5: build -> serialize -> submit (Received) -> poll (Done) ->
/// evidence bundle. Returns the packed `.ikb` plus the two NAV envelopes so the
/// callers can assert the country-specific receipt fields.
fn run_lifecycle() -> (
    Vec<u8>,
    invoicekit_report_hu_nav::NavManageEnvelope,
    invoicekit_report_hu_nav::NavManageEnvelope,
) {
    // 1. build the canonical IR document.
    let doc = hungarian_invoice();

    // 2. serialize -> UBL 2.1 (EN 16931 family path). NAV's manageInvoiceRequest
    //    wraps the invoice; this crate exposes no national serializer, so the UBL
    //    bytes ARE the payload we submit.
    let ubl_xml = to_xml(&doc).unwrap();
    // Structural spot-check on the canonical UBL. Namespace declarations render
    // inline on the canonicalized open tags (`<cac:... xmlns:cac="...">`), so
    // match the element-name prefix, not the bare `<name>` form.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">HUF<",
        "currencyID=\"HUF\"",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit to the offline NAV mock; NAV returns a Received envelope with a
    //    NAV-assigned transaction id.
    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);
    let received = provider.manage_invoice(&manage_request(ubl_bytes.clone())).unwrap();

    // 4. poll the same transaction; NAV resolves it to the terminal Done status.
    let done = provider
        .query_transaction(NavEnvironment::Test, &received.transaction_id)
        .unwrap();

    // 5. evidence bundle: canonical IR + national-wire UBL + the final receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&done).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, received, done)
}

#[test]
fn hungary_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, received, done) = run_lifecycle();

    // Step 3 success criterion: NAV accepted the submission and assigned a
    // country-specific transaction id with the `NAV-` prefix.
    assert_eq!(received.status, NavStatus::Received);
    assert!(
        received.transaction_id.starts_with("NAV-"),
        "NAV transaction id must carry the NAV- prefix, got {:?}",
        received.transaction_id
    );
    assert_eq!(received.recorded_at, FIXED_RECORDED_AT);
    assert!(received.validation_result.is_none());

    // Step 4 success criterion: polling reaches the terminal Done status while
    // preserving the same transaction id (the NAV async lifecycle).
    assert_eq!(done.status, NavStatus::Done);
    assert_eq!(done.transaction_id, received.transaction_id);

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn hungary_lifecycle_is_byte_deterministic() {
    let (a, _, _) = run_lifecycle();
    let (b, _, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn hungary_mock_refuses_empty_payload_and_bad_tax_id() {
    // The HU NAV mock has no forced-status knob: manage_invoice always returns a
    // `Received` envelope on the happy path and CANNOT be made to emit an
    // `Aborted` status. The only refusals it models are pre-wire shape failures,
    // surfaced as typed `Err`s (which is the correct contract for shape errors;
    // an authority `Aborted` verdict would be an Ok-envelope, not an Err).
    use invoicekit_report_hu_nav::NavError;

    let provider = MockNavProvider::with_fixed_recorded_at(FIXED_RECORDED_AT);

    // Empty manageInvoiceRequest payload -> BadXml.
    let empty = provider.manage_invoice(&manage_request(Vec::new())).unwrap_err();
    assert!(
        matches!(empty, NavError::BadXml(_)),
        "empty payload must be refused as BadXml, got {empty:?}"
    );

    // Malformed adóazonosító -> BadTaxId, before the wire.
    let mut bad = manage_request(b"<Invoice/>".to_vec());
    bad.issuer_tax_id = "NOT-A-TAX-ID".to_owned();
    let err = provider.manage_invoice(&bad).unwrap_err();
    assert!(
        matches!(err, NavError::BadTaxId(_)),
        "malformed tax id must be refused as BadTaxId, got {err:?}"
    );
}
