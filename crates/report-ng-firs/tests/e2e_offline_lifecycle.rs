// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Nigeria FIRS offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Nigeria and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Nigerian supplier +
//!    customer and the NGN (Nigerian Naira) currency
//! 2. serialize -> EN 16931 / UBL 2.1 XML (Nigeria rides the UBL family path;
//!    the live FIRS envelope wraps this) via `invoicekit_format_ubl::to_xml`
//! 3. submit the UBL bytes to the in-crate `MockFirsProvider` and assert the
//!    FIRS receipt's country-specific fields (IRN + status + recorded
//!    timestamp)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only)` it (exit 0 == report.ok)
//! 5. determinism: pack twice -> byte-identical
//! 6. rejection: the FIRS mock has no forced-authority-refusal knob (it always
//!    returns `FirsStatus::Accepted`), so a forced FIRS `Rejected` cannot be
//!    driven offline. What it *does* refuse, as `Err`, is pre-wire shape
//!    validation: a malformed issuer TIN or an empty payload. Those are
//!    exercised in `firs_rejects_bad_shapes_before_the_wire`.
//!
//! Mirrors the proven `report-it-sdi` / `report-es-verifactu` offline-E2E
//! pattern. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
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
use invoicekit_report_ng_firs::{
    FirsEnvironment, FirsProvider, FirsStatus, FirsSubmitEnvelope, FirsSubmitRequest,
    MockFirsProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const RECORDED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_ng_e2e";
const TRACE: &str = "trace_ng_e2e";
const ISSUER_TIN: &str = "12345678-9012";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn nigerian_party(name: &str, vat: &str, city: &str, state: &str, postal: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 Marina Road".to_owned()],
            city: city.to_owned(),
            subdivision: Some(state.to_owned()),
            postal_code: postal.to_owned(),
            country: CountryCode::new("NG").unwrap(),
        },
        contact: None,
    }
}

fn nigerian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ng-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-NG-0001").unwrap(),
        currency: Iso4217Code::new("NGN").unwrap(),
        supplier: nigerian_party("Acme Nigeria Ltd", "NG12345678901", "Lagos", "LA", "100001"),
        customer: nigerian_party("Beta Services Plc", "NG98765432109", "Abuja", "FC", "900001"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting and development".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // Nigeria's standard VAT rate is 7.5%.
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(750),
            tax_rate: Some(DecimalValue::new(Decimal::new(750, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(10750),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(10750),
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

fn submit_request(payload: Vec<u8>) -> FirsSubmitRequest {
    FirsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: FirsEnvironment::Sandbox,
        issuer_tin: ISSUER_TIN.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> submit to the FIRS mock ->
/// assemble + return the packed `.ikb` plus the FIRS receipt.
fn run_lifecycle() -> (Vec<u8>, FirsSubmitEnvelope) {
    // 1. build
    let doc = nigerian_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (Nigeria rides the UBL family).
    let ubl: String = to_xml(&doc).unwrap();
    // Spot-check the national-relevant UBL spine before it hits the wire.
    // Match local names without the closing `>` because canonicalization may
    // attach inline `xmlns:` declarations right after the element name.
    for needle in [
        "<Invoice",
        "cac:AccountingSupplierParty",
        "cac:AccountingCustomerParty",
        "cbc:DocumentCurrencyCode",
        ">NGN</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to FIRS (offline deterministic mock).
    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national UBL XML + FIRS receipt.
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
fn nigeria_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: FIRS accepted the invoice. Assert the Nigeria-specific
    // artifacts.
    assert_eq!(envelope.status, FirsStatus::Accepted);
    // The IRN (Invoice Reference Number) is the FIRS clearance handle the
    // printed invoice and downstream lookups carry. The mock tags it `NG-`.
    assert!(
        envelope.irn.starts_with("NG-"),
        "FIRS must assign a Nigeria-tagged IRN, got {:?}",
        envelope.irn
    );
    // IRN body is the zero-padded 16-digit serial after the `NG-` prefix.
    let serial = envelope.irn.strip_prefix("NG-").expect("NG- prefix");
    assert_eq!(serial.len(), 16, "IRN serial must be 16 chars, got {serial:?}");
    assert!(
        serial.bytes().all(|b| b.is_ascii_digit()),
        "IRN serial must be all ASCII digits, got {serial:?}"
    );
    assert_eq!(envelope.recorded_at, RECORDED_AT);
    // An accepted verdict carries no rejection reason.
    assert!(envelope.reason.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn nigeria_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn nigeria_irn_serial_advances_per_submission() {
    // FIRS assigns a fresh IRN to every cleared invoice. Prove the mock's
    // serial advances so two submissions through one provider get distinct
    // clearance handles (the audit trail must never collide IRNs).
    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let ubl_bytes = to_xml(&nigerian_invoice()).unwrap().into_bytes();

    let first = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();
    assert_eq!(first.status, FirsStatus::Accepted);

    let second = provider.submit_invoice(&submit_request(ubl_bytes)).unwrap();
    assert_eq!(second.status, FirsStatus::Accepted);

    assert_ne!(first.irn, second.irn, "each cleared invoice must get a distinct IRN");
}

#[test]
fn firs_rejects_bad_shapes_before_the_wire() {
    // The mock has no forced-FIRS-refusal knob (it always returns Accepted), so
    // there is no `FirsStatus::Rejected` path to drive offline. What it does
    // refuse, as `Err`, is pre-wire shape validation. Those refusals must never
    // reach the wire / a bundle.
    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let ubl_bytes = to_xml(&nigerian_invoice()).unwrap().into_bytes();

    // (a) malformed TIN (not 12 ASCII digits with the hyphens stripped).
    let mut bad_tin = submit_request(ubl_bytes.clone());
    bad_tin.issuer_tin = "BAD".to_owned();
    assert!(
        provider.submit_invoice(&bad_tin).is_err(),
        "a malformed issuer TIN must be refused before the wire"
    );

    // (b) empty payload.
    let empty = submit_request(Vec::new());
    assert!(
        provider.submit_invoice(&empty).is_err(),
        "an empty payload must be refused before the wire"
    );

    // The well-formed request still clears, proving the refusals above are
    // shape-specific and not a blanket failure.
    assert!(
        provider.submit_invoice(&submit_request(ubl_bytes)).is_ok(),
        "a well-formed request must still clear FIRS"
    );
}
