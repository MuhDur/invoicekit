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
            extensions: Vec::new(),
        }],
        // Israel's standard VAT rate is 17%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1700),
            tax_rate: Some(DecimalValue::new(Decimal::new(1700, 2))),
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
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

fn allocation_request(payload: Vec<u8>) -> ItaAllocationRequest {
    ItaAllocationRequest {
        tenant_id: TENANT.to_owned(),
        environment: ItaEnvironment::Sandbox,
        issuer_id: ISSUER_ID.to_owned(),
        buyer_id: BUYER_ID.to_owned(),
        // 1.00 ILS == 10_000 basis points; gross is 117.00 ILS here.
        gross_basis_points: 1_170_000,
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> request allocation -> evidence bundle.
///
/// Returns the packed `.ikb` plus the ITA receipt so callers can assert both
/// the country-specific authority fields and that the bundle verifies.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_il_ita::ItaAllocationEnvelope) {
    // 1. build
    let doc = israeli_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path).
    let ubl_xml = to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.into_bytes();
    // The canonicalizer pushes namespace declarations down to first use, so
    // assert on prefix-qualified element starts (not the closed `>` form).
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">ILS</cbc:DocumentCurrencyCode>",
    ] {
        let xml = String::from_utf8(ubl_bytes.clone()).unwrap();
        assert!(xml.contains(needle), "UBL XML missing {needle}");
    }

    // 3. submit those bytes to the existing MockItaProvider for an Allocation
    //    Number. A deterministic fixed-timestamp mock keeps pack() byte-stable.
    let provider = MockItaProvider::with_fixed_issued_at(FIXED_ISSUED_AT);
    let envelope = provider
        .request_allocation(&allocation_request(ubl_bytes.clone()))
        .unwrap();

    // 4. evidence bundle: canonical IR + UBL XML + the ITA receipt.
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
