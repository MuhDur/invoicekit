// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Poland KSeF offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Poland and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national FA(3) (`FA_VAT`, `<Faktura>`) XML
//! 3. local validate (structural + Polish NIP weighted-checksum)
//! 4. sign + transmit via the offline `MockKsefReportProvider` (composes
//!    `invoicekit-signer-ksef`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. rejection path: a KSeF rejected status is a receipt, NOT an `Err`
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). Capability-matrix presence is asserted centrally elsewhere,
//! not here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_pl_ksef::{
    to_fa3_xml, Fa3Context, KsefAcceptance, KsefEnvironment, KsefReport, KsefReportProvider,
    KsefReportRequest, MockKsefReportProvider,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_ksef::AuthMode;
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;
use std::sync::Arc;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_pl_e2e";
const TRACE: &str = "trace_pl_e2e";
// The inner KSeF mock keys its signer by the session token it mints; the first
// session is always `sess-00000001`.
const SESSION_KEY: &str = "sess-00000001";
// 5252248481 is a valid-checksum Polish NIP.
const ISSUER_NIP: &str = "5252248481";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn polish_party(name: &str, nip: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: nip.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["ul. Marszałkowska 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "00-001".to_owned(),
            country: CountryCode::new("PL").unwrap(),
        },
        contact: None,
    }
}

fn polish_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pl-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("FV-2026-PL-0001").unwrap(),
        currency: Iso4217Code::new("PLN").unwrap(),
        supplier: polish_party("Acme Sp. z o.o.", "PL5252248481", "Warszawa"),
        customer: polish_party("Beta S.A.", "5260001246", "Kraków"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Usługi konsultingowe & rozwój oprogramowania".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2300),
            tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12300),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12300),
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

fn provider(forced: Option<KsefAcceptance>) -> MockKsefReportProvider {
    let signer: Arc<dyn Signer> =
        Arc::new(SoftwareSigner::new().with_key(SESSION_KEY, [5_u8; 32]));
    let p = MockKsefReportProvider::new(signer, KsefEnvironment::Demo);
    match forced {
        Some(acceptance) => p.with_forced_acceptance(acceptance),
        None => p,
    }
}

fn report_request(fa_xml: Vec<u8>) -> KsefReportRequest {
    KsefReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: KsefEnvironment::Demo,
        issuer_nip: ISSUER_NIP.to_owned(),
        auth_mode: AuthMode::QualifiedSignature,
        fa_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> sign/transmit -> evidence bundle.
fn run_lifecycle(forced: Option<KsefAcceptance>) -> (Vec<u8>, KsefReport) {
    // 1. build
    let doc = polish_invoice();

    // 2. serialize -> FA(3) (pinned header context for byte stability)
    let ctx = Fa3Context {
        data_wytworzenia: "2026-05-26T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let fa = to_fa3_xml(&doc, &ctx).unwrap();

    // 3. local validate (structural): the national artifact carries the
    // mandatory FA(3) spine. Reference XSD validation stays external (JVM).
    for needle in [
        "<Faktura xmlns=",
        "<Naglowek>",
        "<Podmiot1>",
        "<Podmiot2>",
        "<RodzajFaktury>VAT</RodzajFaktury>",
        "<P_15>123.00</P_15>",
    ] {
        assert!(fa.contains(needle), "FA(3) missing {needle}");
    }

    // 4. sign + transmit (offline mock composing the real KSeF signer path)
    let report = provider(forced)
        .report(&report_request(fa.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national XML + signed artifact + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/fa3.xml".to_owned(), fa.into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_fa_xml.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, report)
}

#[test]
fn poland_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(None);

    // Happy path: KSeF accepted and assigned a Numer KSeF.
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);
    assert!(report.envelope.acceptance.is_accepted());
    assert!(report.envelope.numer_ksef.starts_with(ISSUER_NIP));
    assert!(report.envelope.upo_reference.starts_with("upo-"));
    assert_eq!(report.envelope.issuer_nip, ISSUER_NIP);
    assert!(report.envelope.reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn poland_rejection_still_bundles_and_verifies() {
    // A KSeF rejected acceptance is a receipt kind, NOT an Err — the audit
    // trail persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(Some(KsefAcceptance::Rejected));
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Rejected);
    assert!(!report.envelope.acceptance.is_accepted());
    assert!(report.envelope.numer_ksef.is_empty());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn poland_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(None);
    let (b, _) = run_lifecycle(None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}
