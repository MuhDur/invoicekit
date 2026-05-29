// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Turkey e-Fatura / e-Arşiv offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Turkey and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("TR")` + `TRY`
//! 2. serialize -> UBL 2.1 (the UBL-TR / EN 16931 family wire format e-Fatura rides)
//! 3. local validate (structural: the UBL spine is present)
//! 4. submit those bytes to the offline `MockEFaturaProvider` and assert the
//!    GİB-issued receipt fields (ETTN + Cleared status + pinned timestamp)
//! 5. assemble a `.ikb` evidence bundle and `verify_packed` it (exit 0 == report.ok)
//! 6. determinism: serialize twice and pack twice -> byte-identical
//! 7. refusal: the mock rejects pre-wire on a malformed VKN and on an empty payload
//!
//! The Turkey mock (`MockEFaturaProvider`) does NOT compose a `Signer`, so this
//! lifecycle carries no `signed.xml` artefact and wires no `invoicekit-signer`
//! dev-dependency. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
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
use invoicekit_report_tr_efatura::{
    EFaturaEnvironment, EFaturaMandate, EFaturaProvider, EFaturaStatus, EFaturaSubmitEnvelope,
    EFaturaSubmitRequest, MockEFaturaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_SUBMITTED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_tr_e2e";
const TRACE: &str = "trace_tr_e2e";
const ISSUER_VKN: &str = "1234567890"; // 10-digit Turkish VKN
const BUYER_VKN: &str = "0987654321";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn turkish_party(name: &str, vkn: &str, city: &str, subdivision: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vkn".to_owned(),
            value: vkn.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Atatürk Caddesi 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(subdivision.to_owned()),
            postal_code: "34000".to_owned(),
            country: CountryCode::new("TR").unwrap(),
        },
        contact: None,
    }
}

fn turkish_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-tr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-TR-0001").unwrap(),
        currency: Iso4217Code::new("TRY").unwrap(),
        supplier: turkish_party("Acme Anonim Sirketi", ISSUER_VKN, "Istanbul", "34"),
        customer: turkish_party("Beta Limited Sirketi", BUYER_VKN, "Ankara", "06"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Yazilim danismanligi".to_owned(),
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
            tax_amount: amt(2000), // Turkey standard KDV/VAT 20%
            tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12000),
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

fn submit_request(invoice_xml: Vec<u8>) -> EFaturaSubmitRequest {
    EFaturaSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EFaturaEnvironment::Sandbox,
        mandate: EFaturaMandate::EFatura,
        issuer_vkn: ISSUER_VKN.to_owned(),
        buyer_tax_id: Some(BUYER_VKN.to_owned()),
        invoice_xml,
    }
}

/// Steps 1-5: build -> serialize -> local-validate -> submit (GİB mock) -> pack evidence.
fn run_lifecycle() -> (Vec<u8>, EFaturaSubmitEnvelope) {
    // 1. build the canonical IR document.
    let doc = turkish_invoice();

    // 2. serialize -> UBL 2.1 bytes (the UBL-TR family path).
    let ubl_xml = to_xml(&doc).unwrap();

    // 3. local validate (structural): the UBL spine is present. Canonicalization
    // redeclares namespaces per element and sorts attributes, so we assert on
    // stable local-name fragments rather than a single fixed namespace shape.
    if std::env::var_os("DUMP_UBL").is_some() {
        eprintln!("---UBL---\n{ubl_xml}\n---END---");
    }
    for needle in [
        "<Invoice",
        "AccountingSupplierParty",
        "AccountingCustomerParty",
        ">TRY<",
        ">120.00<",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 4. submit to the offline GİB mock provider.
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);
    let envelope = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 5. evidence bundle: canonical IR + UBL artefact + GİB receipt.
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
fn turkey_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: GİB clears and assigns an ETTN. The mock derives a
    // 16-char-ish ETTN from its serial (prefixed `MOCK-`).
    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    assert!(
        envelope.ettn.starts_with("MOCK-"),
        "GİB receipt must carry a mock ETTN, got {:?}",
        envelope.ettn
    );
    assert_eq!(envelope.submitted_at, PINNED_SUBMITTED_AT);
    assert!(envelope.message.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn turkey_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn turkey_mock_refuses_malformed_vkn_pre_wire() {
    // The Turkey mock has no forced-rejection knob: `submit_invoice` always
    // returns `Cleared`, and `EFaturaStatus::Rejected` (Red Yanıtı) is a wire
    // verdict the mock cannot synthesize. What IS testable is the pre-wire
    // refusal contract: a malformed issuer VKN is an `Err`, not a receipt.
    let doc = turkish_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);

    let mut bad = submit_request(ubl_bytes);
    bad.issuer_vkn = "12345".to_owned(); // not 10 digits
    let err = provider.submit_invoice(&bad).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_tr_efatura::EFaturaError::BadTaxId(_)
        ),
        "malformed VKN must refuse pre-wire, got {err:?}"
    );
}

#[test]
fn turkey_mock_refuses_empty_payload_pre_wire() {
    // Empty UBL bytes never reach GİB: the mock refuses pre-wire with BadXml.
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);
    let err = provider
        .submit_invoice(&submit_request(Vec::new()))
        .unwrap_err();
    assert!(
        matches!(err, invoicekit_report_tr_efatura::EFaturaError::BadXml(_)),
        "empty payload must refuse pre-wire, got {err:?}"
    );
}
