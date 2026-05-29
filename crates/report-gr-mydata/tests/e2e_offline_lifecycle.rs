// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Greece **myDATA** offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Greece and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) for a Greek (`GR`) supplier +
//!    customer, currency `EUR`
//! 2. serialize -> EN 16931 / UBL XML via `invoicekit_format_ubl::to_xml`
//!    (myDATA's wire payload is the IAPR `InvoicesDoc`; the foundation UBL path
//!    is the family serializer this crate composes — this crate ships no
//!    serializer of its own)
//! 3. submit those bytes to the EXISTING `MockMyDataProvider` and assert the
//!    Greek authority artefacts: an `Accepted` verdict, the IAPR **MARK**
//!    (Μοναδικός Αριθμός Καταχώρησης), the **UID**, and the pinned
//!    `reported_at` timestamp; also assert the QR payload the printed invoice
//!    must carry embeds both MARK + UID
//! 4. assemble a `.ikb` evidence bundle (`canonical.json` + `formats/ubl.xml` +
//!    `receipt.json`) and `verify_packed(content_only).ok == true` (exit 0)
//! 5. determinism: run the whole lifecycle twice -> byte-identical `.ikb`
//! 6. refusal: the mock returns `Err` for the two pre-wire shape failures it
//!    validates (bad ΑΦΜ / AFM, empty payload). See the note on the test for
//!    why an authority-side `Rejected` verdict cannot be forced here.
//!
//! This mirrors `crates/report-it-sdi/tests/e2e_offline_lifecycle.rs`, the
//! proven offline-E2E reference pattern. Goldens are hand-rolled (no `insta` /
//! `pretty_assertions`, which would mutate `Cargo.lock`). The capability matrix
//! is intentionally NOT asserted here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_gr_mydata::{
    qr_payload, MockMyDataProvider, MyDataEnvironment, MyDataError, MyDataInvoiceCategory,
    MyDataProvider, MyDataReportEnvelope, MyDataReportRequest, MyDataStatus,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const PINNED_REPORTED_AT: &str = "2026-01-01T00:00:00Z";
const TENANT: &str = "tenant_gr_e2e";
const TRACE: &str = "trace_gr_e2e";
const ISSUER_AFM: &str = "123456789";
const BUYER_AFM: &str = "987654321";
const QR_BASE_URL: &str = "https://www.aade.gr/mydata";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn greek_party(name: &str, vat: &str, street: &str, city: &str, postal: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec![street.to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: postal.to_owned(),
            country: CountryCode::new("GR").unwrap(),
        },
        contact: None,
    }
}

fn greek_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-gr-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-GR-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: greek_party(
            "Acme Hellas AE",
            "EL123456789",
            "Leoforos Kifisias 1",
            "Athina",
            "11523",
        ),
        customer: greek_party(
            "Beta EPE",
            "EL987654321",
            "Egnatia 100",
            "Thessaloniki",
            "54622",
        ),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Symvouleftikes ypiresies logismikou".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // UBL family uses EA (CII/Factur-X would use C62).
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // Greek standard VAT rate is 24%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2400),
            tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12400),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12400),
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

fn report_request(invoices_doc_xml: Vec<u8>) -> MyDataReportRequest {
    MyDataReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: MyDataEnvironment::Sandbox,
        issuer_afm: ISSUER_AFM.to_owned(),
        buyer_afm: Some(BUYER_AFM.to_owned()),
        category: MyDataInvoiceCategory::SalesGoods {
            code: "1.1".to_owned(),
        },
        invoices_doc_xml,
    }
}

/// Steps 1-4: build -> serialize (UBL) -> report (mock IAPR) -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the authority envelope so callers can
/// assert both the bundle verifies and the Greek artefacts are present.
fn run_lifecycle() -> (Vec<u8>, MyDataReportEnvelope) {
    // 1. build the canonical IR document.
    let doc = greek_invoice();

    // 2. serialize -> EN 16931 / UBL XML bytes (this crate ships no serializer
    //    of its own; it composes the UBL family path).
    let ubl_xml = to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();
    // Sanity: the UBL spine the IAPR mapping reads from must be present.
    // The canonicalizer pins namespace declarations inline on each element, so
    // we match the element-name prefix (open angle + name), not a bare `>`.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">EUR</cbc:DocumentCurrencyCode>",
        ">GR</cbc:IdentificationCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    // 3. report to the offline IAPR mock.
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);
    let envelope = provider.report_invoice(&report_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national/family XML + receipt.
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
fn greece_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Happy path: the IAPR accepted and assigned a MARK + UID.
    assert_eq!(envelope.status, MyDataStatus::Accepted);
    assert_eq!(envelope.reported_at, PINNED_REPORTED_AT);
    assert!(envelope.message.is_none());

    let mark = envelope.mark.as_ref().expect("accepted invoice carries a MARK");
    let uid = envelope.uid.as_ref().expect("accepted invoice carries a UID");
    // The mock derives a 16-digit IAPR-shaped MARK from its serial.
    assert!(mark.as_str().starts_with("4000"));
    assert!(uid.as_str().starts_with("MYDATA-MOCK-UID-"));

    // The printed-invoice QR payload must embed both MARK + UID.
    let qr = qr_payload(QR_BASE_URL, &envelope).unwrap();
    assert!(qr.contains(&format!("mark={}", mark.as_str())));
    assert!(qr.contains(&format!("uid={}", uid.as_str())));

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn greece_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn greece_refuses_invalid_afm_and_empty_payload_before_the_wire() {
    // Refusal note: `MockMyDataProvider` always synthesises an `Accepted`
    // verdict for a well-shaped request — it exposes no knob to force an
    // authority-side `MyDataStatus::Rejected` (unlike Italy's
    // `with_forced_receipt`). The genuine refusal surface it DOES implement is
    // pre-wire shape validation, which returns `Err`, not a `Rejected`
    // envelope. We exercise both shape refusals here.
    let provider = MockMyDataProvider::with_fixed_reported_at(PINNED_REPORTED_AT);

    // Well-shaped UBL bytes so only the AFM is wrong.
    let ubl_bytes = to_xml(&greek_invoice()).unwrap().into_bytes();

    // (a) bad issuer ΑΦΜ (AFM) — must be exactly 9 ASCII digits.
    let mut bad_afm = report_request(ubl_bytes.clone());
    bad_afm.issuer_afm = "12345".to_owned();
    let err = provider.report_invoice(&bad_afm).unwrap_err();
    assert!(
        matches!(err, MyDataError::BadAfm(_)),
        "short AFM must be refused as BadAfm, got {err:?}"
    );

    // (b) empty InvoicesDoc payload — refused before any synthesis.
    let mut empty_payload = report_request(ubl_bytes);
    empty_payload.invoices_doc_xml.clear();
    let err = provider.report_invoice(&empty_payload).unwrap_err();
    assert!(
        matches!(err, MyDataError::BadXml(_)),
        "empty payload must be refused as BadXml, got {err:?}"
    );
}
