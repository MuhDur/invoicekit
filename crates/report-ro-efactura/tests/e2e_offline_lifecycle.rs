// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Romania RO e-Factura offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Romania and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a Romanian `CountryCode`
//!    and the RON currency;
//! 2. serialize to UBL 2.1 bytes via `invoicekit_format_ubl::to_xml` (the
//!    EN 16931 / RO CIUS family path; RO e-Factura rides on UBL 2.1);
//! 3. submit those bytes to the existing `MockEFacturaProvider`, then poll, and
//!    assert ANAF's country-specific receipt fields (indice de incarcare /
//!    status / `uploaded_at`);
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`;
//! 5. determinism: pack twice -> byte-identical;
//! 6. refusal: the mock's local validators (CUI shape, empty payload) reject
//!    before the wire.
//!
//! Note on the rejection path: `MockEFacturaProvider` does NOT expose a way to
//! force an ANAF `Rejected` verdict (no `with_forced_*`), so the authority-side
//! rejection cannot be exercised offline. What IS exercised is the pre-wire
//! refusal contract (`EFacturaError::BadCui` / `EFacturaError::BadXml`), which
//! is the part the mock genuinely owns.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ro_efactura::{
    EFacturaDocumentKind, EFacturaEnvironment, EFacturaProvider, EFacturaStatus,
    EFacturaUploadEnvelope, EFacturaUploadRequest, MockEFacturaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_ro_e2e";
const TRACE: &str = "trace_ro_e2e";
const ISSUER_CUI: &str = "RO12345678";
const BUYER_CUI: &str = "87654321";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn romanian_party(name: &str, vat: &str, city: &str, county: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Strada Victoriei 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(county.to_owned()),
            postal_code: "010101".to_owned(),
            country: CountryCode::new("RO").unwrap(),
        },
        contact: None,
    }
}

fn romanian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ro-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-RO-0001").unwrap(),
        currency: Iso4217Code::new("RON").unwrap(),
        supplier: romanian_party("Acme SRL", "RO12345678", "Bucuresti", "B"),
        customer: romanian_party("Beta SA", "RO87654321", "Cluj-Napoca", "CJ"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicii de consultanta software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11900),
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

fn upload_request(invoice_xml: Vec<u8>) -> EFacturaUploadRequest {
    EFacturaUploadRequest {
        tenant_id: TENANT.to_owned(),
        environment: EFacturaEnvironment::Sandbox,
        kind: EFacturaDocumentKind::Invoice,
        issuer_cui: ISSUER_CUI.to_owned(),
        buyer_cui: Some(BUYER_CUI.to_owned()),
        invoice_xml,
    }
}

/// Steps 1-4: build -> serialize -> upload+poll -> evidence bundle bytes.
///
/// Returns the packed `.ikb` together with the upload and (cleared) poll
/// envelopes so the assertions can inspect the country-specific receipt fields.
fn run_lifecycle() -> (Vec<u8>, EFacturaUploadEnvelope, EFacturaUploadEnvelope) {
    // 1. build
    let doc = romanian_invoice();

    // 2. serialize -> UBL 2.1 (RO e-Factura is UBL 2.1 + RO CIUS)
    let ubl = to_xml(&doc).unwrap();
    let ubl_bytes = ubl.into_bytes();
    // local structural sanity: the UBL spine and the RON currency are present.
    let ubl_str = String::from_utf8(ubl_bytes.clone()).unwrap();
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">RON<",
    ] {
        assert!(ubl_str.contains(needle), "UBL missing {needle}");
    }

    // 3. upload to ANAF mock, then poll for clearance.
    let provider = MockEFacturaProvider::default();
    let uploaded = provider.upload(&upload_request(ubl_bytes.clone())).unwrap();
    let cleared = provider
        .poll_status(EFacturaEnvironment::Sandbox, &uploaded.indice_incarcare)
        .unwrap();

    // 4. evidence bundle: canonical doc + national UBL + cleared receipt.
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&cleared).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, uploaded, cleared)
}

#[test]
fn romania_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, uploaded, cleared) = run_lifecycle();

    // Upload receipt: ANAF assigns an "indice de incarcare" and accepts.
    assert_eq!(uploaded.status, EFacturaStatus::Uploaded);
    assert!(
        uploaded.indice_incarcare.starts_with("ANAF-"),
        "indice de incarcare must carry the ANAF prefix, got {:?}",
        uploaded.indice_incarcare
    );
    assert_eq!(uploaded.uploaded_at, "2026-01-01T00:00:00Z");
    assert!(uploaded.motivare.is_none());

    // Poll receipt: the same upload index, now Cleared.
    assert_eq!(cleared.status, EFacturaStatus::Cleared);
    assert_eq!(cleared.indice_incarcare, uploaded.indice_incarcare);
    assert!(cleared.motivare.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn romania_lifecycle_is_byte_deterministic() {
    let (a, _, _) = run_lifecycle();
    let (b, _, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn romania_refuses_bad_cui_before_the_wire() {
    // The mock has no force-rejection knob, so the authority-side ANAF
    // `Rejected` verdict cannot be exercised offline. The refusal the mock DOES
    // own is the pre-wire CUI shape check, surfaced as `Err`, never a status.
    let provider = MockEFacturaProvider::default();
    let mut req = upload_request(b"<Invoice/>".to_vec());
    req.issuer_cui = "NOT-A-CUI".to_owned();
    let err = provider.upload(&req).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_ro_efactura::EFacturaError::BadCui(_)
        ),
        "bad issuer CUI must be refused with BadCui, got {err:?}"
    );
}

#[test]
fn romania_refuses_empty_payload_before_the_wire() {
    let provider = MockEFacturaProvider::default();
    let err = provider.upload(&upload_request(Vec::new())).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_ro_efactura::EFacturaError::BadXml(_)
        ),
        "empty payload must be refused with BadXml, got {err:?}"
    );
}
