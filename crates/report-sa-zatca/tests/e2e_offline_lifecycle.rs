// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Saudi Arabia ZATCA Phase 2 offline end-to-end lifecycle (coverage-loop §1
//! honest bar).
//!
//! Drives the full local-only chain for Saudi Arabia and proves it
//! deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> ZATCA UBL 2.1 XML (UBL base + ZATCA extensions + ICV/PIH
//!    hash chain), reusing `invoicekit-format-ubl`
//! 3. local validate (structural + Saudi VAT / ICV / PIH identity shapes)
//! 4. stamp + clear/report via the offline `MockZatcaReportProvider` (composes
//!    `invoicekit-signer-zatca`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. byte-determinism: serialize twice and pack twice -> byte-identical
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). This mirrors the Italy SDI reference E2E. Capability-matrix
//! presence is asserted centrally, not here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_sa_zatca::{
    build_qr_fields, to_zatca_ubl_xml, validate_invoice_counter_value,
    validate_previous_invoice_hash, validate_saudi_vat_number, CsidRecord, InvoiceMode,
    MockZatcaReportProvider, ReportingStatus, ZatcaClearanceKind, ZatcaEnvironment, ZatcaReport,
    ZatcaReportProvider, ZatcaReportRequest, ZatcaUblContext,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;
use std::sync::Arc;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_sa_e2e";
const TRACE: &str = "trace_sa_e2e";
const CSID: &str = "csid-compliance-e2e";
// Valid Saudi VAT: 15 digits, starts and ends with 3, position 11 is 1.
const SELLER_VAT: &str = "300000000010003";
const INVOICE_UUID: &str = "uuid-sa-e2e-0001";
const QR_TIMESTAMP: &str = "2026-05-26T10:30:00Z";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn saudi_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["King Fahd Road 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("01".to_owned()),
            postal_code: "12345".to_owned(),
            country: CountryCode::new("SA").unwrap(),
        },
        contact: None,
    }
}

fn saudi_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-sa-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-SA-0001").unwrap(),
        currency: Iso4217Code::new("SAR").unwrap(),
        supplier: saudi_party("Acme KSA", SELLER_VAT, "Riyadh"),
        customer: saudi_party("Beta Trading", "311111111110003", "Jeddah"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulting & support services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
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

fn csid() -> CsidRecord {
    CsidRecord {
        csid: CSID.to_owned(),
        environment: ZatcaEnvironment::Compliance,
        vat_number: SELLER_VAT.to_owned(),
        stamp_uuid: Some("stamp-uuid-e2e".to_owned()),
        not_before: "2026-01-01T00:00:00Z".to_owned(),
        not_after: "2027-12-31T23:59:59Z".to_owned(),
    }
}

fn provider(forced: Option<ReportingStatus>) -> MockZatcaReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CSID, [9_u8; 32]));
    let p = MockZatcaReportProvider::new(signer, csid());
    match forced {
        Some(status) => p.with_forced_status(status),
        None => p,
    }
}

fn report_request(ubl_xml: Vec<u8>, mode: InvoiceMode) -> ZatcaReportRequest {
    ZatcaReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: ZatcaEnvironment::Compliance,
        seller_vat_number: SELLER_VAT.to_owned(),
        mode,
        invoice_counter_value: 1,
        previous_invoice_hash: ZatcaUblContext::GENESIS_PIH.to_owned(),
        qr_fields: build_qr_fields(&saudi_invoice(), QR_TIMESTAMP).unwrap(),
        ubl_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> stamp/clear -> evidence bundle.
fn run_lifecycle(mode: InvoiceMode, forced: Option<ReportingStatus>) -> (Vec<u8>, ZatcaReport) {
    // 1. build
    let doc = saudi_invoice();

    // 2. serialize -> ZATCA UBL (reusing format-ubl for the UBL 2.1 spine)
    let ctx = ZatcaUblContext::genesis(INVOICE_UUID, mode);
    let ubl = to_zatca_ubl_xml(&doc, &ctx).unwrap();

    // 3. local validate: structural ZATCA spine + Saudi identity shapes.
    for needle in [
        "<ext:UBLExtensions",
        "<cbc:ProfileID>reporting:1.0</cbc:ProfileID>",
        "<cbc:UUID>uuid-sa-e2e-0001</cbc:UUID>",
        "<cbc:ID>ICV</cbc:ID>",
        "<cbc:ID>PIH</cbc:ID>",
        "<cbc:CompanyID>300000000010003</cbc:CompanyID>",
    ] {
        assert!(ubl.contains(needle), "ZATCA UBL missing {needle}");
    }
    validate_saudi_vat_number(SELLER_VAT).unwrap();
    validate_invoice_counter_value(1).unwrap();
    validate_previous_invoice_hash(ZatcaUblContext::GENESIS_PIH).unwrap();

    // 4. stamp + clear/report (offline mock composing the real ZATCA signer)
    let report = provider(forced)
        .report(&report_request(ubl.clone().into_bytes(), mode))
        .unwrap();

    // 5. evidence bundle: canonical doc + ZATCA UBL + signed/stamped artifact +
    //    receipt. The "signed artifact" is the QR-code TLV envelope the stamp
    //    produced (ZATCA's printable cryptographic proof).
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/zatca-ubl.xml".to_owned(), ubl.into_bytes());
    artefacts.insert("signed/qr.tlv".to_owned(), report.qr_tlv.clone());
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
fn saudi_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(InvoiceMode::Standard, None);

    // Happy path: ZATCA cleared the standard (B2B) invoice.
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
    assert!(report.envelope.clearance_kind.is_accepted());
    assert_eq!(report.envelope.invoice_counter_value, 1);
    assert!(report.envelope.reason.is_none());
    assert!(!report.qr_tlv.is_empty());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn saudi_simplified_invoice_is_reported_b2c() {
    let (ikb, report) = run_lifecycle(InvoiceMode::Simplified, None);
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Reported);
    assert_eq!(report.envelope.mode, InvoiceMode::Simplified);

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "B2C reporting-path evidence bundle must verify");
}

#[test]
fn saudi_rejection_still_bundles_and_verifies() {
    // A portal rejection is a receipt kind, NOT an Err — the audit trail
    // persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(InvoiceMode::Standard, Some(ReportingStatus::Rejected));
    assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Rejected);
    assert!(!report.envelope.clearance_kind.is_accepted());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn saudi_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(InvoiceMode::Standard, None);
    let (b, _) = run_lifecycle(InvoiceMode::Standard, None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}
