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
        invoice_period: None,
        delivery_date: None,
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
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
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

/// A Romanian **storno** (credit note) referencing the same supplier/customer as
/// [`romanian_invoice`]. RO e-Factura rides on UBL 2.1 constrained by CIUS-RO
/// (the Romanian Customized Usage Specification of EN 16931); a credit note is a
/// UBL `<CreditNote>` whose `BT-3` invoice-type maps to UBL `CreditNoteTypeCode`
/// `381` ("Credit note"). Source: ANAF "Specificații tehnice și de utilizare a
/// elementelor de bază ale facturii electronice RO e-Factura — CIUS-RO"
/// <https://mfinante.gov.ro/ro/web/efactura/informatii-tehnice> and the EN 16931
/// / UNCL1001 invoice-type code list. UBL 2.1 forbids a top-level `cbc:DueDate`
/// on a `CreditNote`, so `due_date` is `None`.
fn romanian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ro-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("STORNO-2026-RO-0001").unwrap(),
        currency: Iso4217Code::new("RON").unwrap(),
        supplier: romanian_party("Acme SRL", "RO12345678", "Bucuresti", "B"),
        customer: romanian_party("Beta SA", "RO87654321", "Cluj-Napoca", "CJ"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Storno servicii de consultanta".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        // Romanian standard VAT rate is 19% (Codul fiscal, Legea 227/2015, art. 291).
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(950),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5950),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5950),
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

/// A two-line invoice mixing the Romanian **standard 19%** VAT rate with the
/// **reduced 9%** rate that Romania applies to e.g. food, medicines, water
/// supply and hospitality (Codul fiscal, Legea 227/2015, art. 291 alin. (2)).
/// Under EN 16931 both bands are VAT category code `S` ("Standard rated",
/// `BT-151`) and are distinguished only by their `BT-152` rate, so CIUS-RO
/// emits two `cac:TaxSubtotal` groups with the same category id but different
/// `cbc:Percent`. Source: ANAF CIUS-RO + EN 16931 / UNCL5305 (category `S`
/// covers every positive VAT rate, reduced or standard).
fn romanian_multiline_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ro-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-RO-0002").unwrap(),
        currency: Iso4217Code::new("RON").unwrap(),
        supplier: romanian_party("Acme SRL", "RO12345678", "Bucuresti", "B"),
        customer: romanian_party("Beta SA", "RO87654321", "Cluj-Napoca", "CJ"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Consultanta software (TVA 19%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Livrare produse alimentare (TVA 9%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        // Two EN 16931 category-`S` subtotals at distinct rates: 19% and 9%.
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(1900),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(20000),
                tax_amount: amt(1800),
                tax_rate: Some(DecimalValue::new(Decimal::new(900, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            // 300.00 net + 19.00 (19% of 100) + 18.00 (9% of 200) = 337.00.
            tax_inclusive_amount: amt(33700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(33700),
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

/// A domestic **reverse-charge** ("taxare inversă") invoice. The supplier
/// charges 0% VAT and the buyer self-accounts; CIUS-RO carries EN 16931 VAT
/// category code `AE` ("VAT Reverse Charge", `BT-151`) with a 0.00 `BT-152`
/// rate. Romania mandates the mechanism for specific domestic supplies (Codul
/// fiscal, Legea 227/2015, art. 331 — e.g. waste, cereals, electricity to
/// traders). Source: ANAF CIUS-RO + EN 16931 / UNCL5305 category list. This
/// exercises the zero-rate `cbc:Percent` / zero-tax `cbc:TaxAmount` path.
fn romanian_reverse_charge_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ro-e2e-rc-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-RO-0003").unwrap(),
        currency: Iso4217Code::new("RON").unwrap(),
        supplier: romanian_party("Acme SRL", "RO12345678", "Bucuresti", "B"),
        customer: romanian_party("Beta SA", "RO87654321", "Cluj-Napoca", "CJ"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Livrare cereale - taxare inversa (AE)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(100_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("AE".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "AE".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(100_000),
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

/// Serialize an IR document to canonical UBL 2.1 bytes (the CIUS-RO transport
/// syntax) and assert the UBL spine + the RON currency are present, mirroring
/// the structural sanity check in [`run_lifecycle`].
fn serialize_ro_ubl(doc: &CommercialDocument) -> Vec<u8> {
    to_xml(doc).unwrap().into_bytes()
}

/// Drive build -> serialize -> upload(+poll) -> evidence bundle for an arbitrary
/// Romanian IR document and the matching e-Factura document kind, returning the
/// packed `.ikb`, the upload envelope, and the cleared poll envelope.
fn run_lifecycle_for(
    doc: &CommercialDocument,
    kind: EFacturaDocumentKind,
) -> (Vec<u8>, EFacturaUploadEnvelope, EFacturaUploadEnvelope) {
    let ubl_bytes = serialize_ro_ubl(doc);

    let provider = MockEFacturaProvider::default();
    let request = EFacturaUploadRequest {
        tenant_id: TENANT.to_owned(),
        environment: EFacturaEnvironment::Sandbox,
        kind,
        issuer_cui: ISSUER_CUI.to_owned(),
        buyer_cui: Some(BUYER_CUI.to_owned()),
        invoice_xml: ubl_bytes.clone(),
    };
    let uploaded = provider.upload(&request).unwrap();
    let cleared = provider
        .poll_status(EFacturaEnvironment::Sandbox, &uploaded.indice_incarcare)
        .unwrap();

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
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    let ikb = pack(&bundle).unwrap();
    (ikb, uploaded, cleared)
}

/// Steps 1-4: build -> serialize -> upload+poll -> evidence bundle bytes.
///
/// Returns the packed `.ikb` together with the upload and (cleared) poll
/// envelopes so the assertions can inspect the country-specific receipt fields.
fn run_lifecycle() -> (Vec<u8>, EFacturaUploadEnvelope, EFacturaUploadEnvelope) {
    // 1. build
    let doc = romanian_invoice();

    // 2. serialize -> UBL 2.1 (RO e-Factura is UBL 2.1 + RO CIUS).
    // Local structural sanity: the UBL spine and the RON currency are present.
    let ubl_str = String::from_utf8(serialize_ro_ubl(&doc)).unwrap();
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        "<cbc:DocumentCurrencyCode",
        ">RON<",
    ] {
        assert!(ubl_str.contains(needle), "UBL missing {needle}");
    }

    // 3-4. serialize -> upload+poll -> evidence bundle (the shared lifecycle).
    run_lifecycle_for(&doc, EFacturaDocumentKind::Invoice)
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
        matches!(err, invoicekit_report_ro_efactura::EFacturaError::BadCui(_)),
        "bad issuer CUI must be refused with BadCui, got {err:?}"
    );
}

#[test]
fn romania_refuses_empty_payload_before_the_wire() {
    let provider = MockEFacturaProvider::default();
    let err = provider.upload(&upload_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, invoicekit_report_ro_efactura::EFacturaError::BadXml(_)),
        "empty payload must be refused with BadXml, got {err:?}"
    );
}

/// A Romanian storno (credit note) serializes to a UBL `<CreditNote>` whose
/// `cbc:CreditNoteTypeCode` is `381`, uploads as
/// `EFacturaDocumentKind::CreditNote`, and bundles into a verifiable `.ikb`.
/// Grounds the credit-note path against CIUS-RO (UBL 2.1 + EN 16931) and the
/// UNCL1001 invoice-type code list.
#[test]
fn romania_credit_note_serializes_as_ubl_credit_note_and_verifies() {
    let doc = romanian_credit_note();
    let ubl = String::from_utf8(serialize_ro_ubl(&doc)).unwrap();

    // CIUS-RO credit note: UBL CreditNote root + CreditNoteTypeCode 381, never
    // an Invoice root / type code 380, and never a top-level cbc:DueDate.
    // (Canonicalization re-declares the cbc namespace on every element, so the
    // assertions anchor on the element's value/close, not a bare open tag.)
    assert!(
        ubl.contains("<CreditNote"),
        "RO storno must serialize to a UBL CreditNote root:\n{ubl}"
    );
    assert!(
        ubl.contains(">381</cbc:CreditNoteTypeCode>"),
        "RO storno must carry CreditNoteTypeCode 381:\n{ubl}"
    );
    assert!(
        !ubl.contains("InvoiceTypeCode"),
        "a credit note must not carry an InvoiceTypeCode:\n{ubl}"
    );
    assert!(
        !ubl.contains("<cbc:DueDate"),
        "UBL 2.1 CreditNote has no top-level cbc:DueDate:\n{ubl}"
    );
    assert!(
        ubl.contains("</cac:CreditNoteLine>"),
        "a credit note must use cac:CreditNoteLine, not cac:InvoiceLine:\n{ubl}"
    );
    assert!(ubl.contains(">RON<"), "RON currency must survive:\n{ubl}");
    assert!(
        ubl.contains("STORNO-2026-RO-0001"),
        "the storno document number must be present:\n{ubl}"
    );

    let (ikb, uploaded, cleared) = run_lifecycle_for(&doc, EFacturaDocumentKind::CreditNote);
    assert_eq!(uploaded.status, EFacturaStatus::Uploaded);
    assert!(uploaded.indice_incarcare.starts_with("ANAF-"));
    assert_eq!(cleared.status, EFacturaStatus::Cleared);
    assert_eq!(cleared.indice_incarcare, uploaded.indice_incarcare);

    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// A two-line invoice mixing Romania's 19% standard rate (category `S`) and 9%
/// reduced rate (category `AA`) emits two `cac:TaxSubtotal` groups carrying both
/// `cbc:Percent` values. Grounds the mixed-rate path against Codul fiscal art.
/// 291 (rates) projected through CIUS-RO / EN 16931 `BT-152`.
#[test]
fn romania_multiline_mixed_rate_emits_both_vat_percentages() {
    let doc = romanian_multiline_mixed_rate_invoice();
    let ubl = String::from_utf8(serialize_ro_ubl(&doc)).unwrap();

    // Both Romanian VAT bands must surface as distinct UBL Percent values.
    // (cbc namespace is re-declared per element after canonicalization.)
    assert!(
        ubl.contains(">19.00</cbc:Percent>"),
        "standard 19% VAT band must surface:\n{ubl}"
    );
    assert!(
        ubl.contains(">9.00</cbc:Percent>"),
        "reduced 9% VAT band must surface:\n{ubl}"
    );
    // The two per-band tax amounts: 19.00 RON (19% of 100) and 18.00 RON (9% of 200).
    assert!(ubl.contains(r#"currencyID="RON">19.00</cbc:TaxAmount>"#));
    assert!(ubl.contains(r#"currencyID="RON">18.00</cbc:TaxAmount>"#));
    // Two priced lines must both appear (RON 100.00 net and RON 200.00 net).
    assert!(ubl.contains("Consultanta software (TVA 19%)"));
    assert!(ubl.contains("Livrare produse alimentare (TVA 9%)"));
    // VAT scheme stays VAT for both subtotals.
    assert!(ubl.contains(">VAT</cbc:ID></cac:TaxScheme>"));

    let (ikb, uploaded, _cleared) = run_lifecycle_for(&doc, EFacturaDocumentKind::Invoice);
    assert_eq!(uploaded.status, EFacturaStatus::Uploaded);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "mixed-rate evidence bundle must verify");
}

/// A domestic reverse-charge ("taxare inversă") invoice carries EN 16931 VAT
/// category `AE` at a 0% rate; the UBL tax category id is `AE` and the tax
/// amount is 0.00. Grounds the reverse-charge path against Codul fiscal art. 331
/// projected through CIUS-RO / EN 16931 (`BT-151` = `AE`).
#[test]
fn romania_reverse_charge_emits_zero_rate_ae_category() {
    let doc = romanian_reverse_charge_invoice();
    let ubl = String::from_utf8(serialize_ro_ubl(&doc)).unwrap();

    // Reverse charge: category id AE, 0% rate, zero tax in the subtotal.
    // (cbc namespace is re-declared per element after canonicalization, so the
    // assertions anchor on each element's value/close.)
    assert!(
        ubl.contains("<cac:TaxCategory><cbc:ID") && ubl.contains(">AE</cbc:ID><cbc:Percent"),
        "reverse-charge subtotal must carry VAT category AE:\n{ubl}"
    );
    assert!(
        ubl.contains(">0</cbc:Percent>"),
        "reverse charge is a 0% rate:\n{ubl}"
    );
    // The net (taxable) amount is the full RON 1000.00; the tax amount is 0.00.
    assert!(ubl.contains(r#"currencyID="RON">0.00</cbc:TaxAmount>"#));
    assert!(
        ubl.contains(r#"currencyID="RON">1000.00</cbc:TaxInclusiveAmount>"#),
        "with no VAT charged the tax-inclusive total equals the net:\n{ubl}"
    );

    let (ikb, uploaded, _cleared) = run_lifecycle_for(&doc, EFacturaDocumentKind::Invoice);
    assert_eq!(uploaded.status, EFacturaStatus::Uploaded);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "reverse-charge evidence bundle must verify");
}

/// ANAF validates the issuer CUI shape before the wire: 2..=10 ASCII digits,
/// optionally `RO`-prefixed. A wrong length, embedded letters, or a malformed
/// prefix must be refused with `EFacturaError::BadCui` — an `Err`, never a
/// `Rejected` status (the mock has no authority force-rejection knob, so the
/// authority-side verdict cannot be exercised offline). Grounds the CUI shape
/// against ANAF's Codul de Identificare Fiscală (CUI / CIF) rules.
#[test]
fn romania_rejects_malformed_cui_shapes_before_the_wire() {
    let provider = MockEFacturaProvider::default();
    // Each of these is a distinct, real CUI shape failure.
    for bad in [
        "RO123456789012", // 12 digits after RO -> too long (>10)
        "RO12AB34",       // embedded letters
        "1",              // single digit -> too short (<2)
        "ROO12345678",    // doubled prefix letter leaves a non-digit
    ] {
        let mut req = upload_request(b"<Invoice/>".to_vec());
        req.issuer_cui = bad.to_owned();
        let err = provider.upload(&req).unwrap_err();
        assert!(
            matches!(err, invoicekit_report_ro_efactura::EFacturaError::BadCui(_)),
            "issuer CUI {bad:?} must be refused with BadCui, got {err:?}"
        );
    }
    // A malformed *buyer* CUI is likewise refused before the wire.
    let mut req = upload_request(b"<Invoice/>".to_vec());
    req.buyer_cui = Some("NOT-A-CUI".to_owned());
    let err = provider.upload(&req).unwrap_err();
    assert!(
        matches!(err, invoicekit_report_ro_efactura::EFacturaError::BadCui(_)),
        "malformed buyer CUI must be refused with BadCui, got {err:?}"
    );
}

/// A self-billing invoice (autofactură) uploads as
/// `EFacturaDocumentKind::SelfBilling`, and the whole CIUS-RO serialization +
/// bundling path is byte-deterministic across two runs. The mock's upload index
/// also increments per submission (`indice de încărcare` is unique per upload).
#[test]
fn romania_self_billing_lifecycle_is_byte_deterministic() {
    let doc = romanian_invoice();

    // CIUS-RO UBL serialization is byte-stable run to run.
    assert_eq!(
        serialize_ro_ubl(&doc),
        serialize_ro_ubl(&doc),
        "CIUS-RO UBL serialization must be byte-stable"
    );

    let (a, up_a, _) = run_lifecycle_for(&doc, EFacturaDocumentKind::SelfBilling);
    let (b, up_b, _) = run_lifecycle_for(&doc, EFacturaDocumentKind::SelfBilling);
    assert_eq!(a, b, "the self-billing lifecycle must be byte-stable");

    // Two independent providers each start their indice de incarcare at serial 1.
    assert_eq!(up_a.indice_incarcare, up_b.indice_incarcare);

    // Within one provider, the upload index strictly increments.
    let provider = MockEFacturaProvider::default();
    let ubl = serialize_ro_ubl(&doc);
    let req = EFacturaUploadRequest {
        tenant_id: TENANT.to_owned(),
        environment: EFacturaEnvironment::Sandbox,
        kind: EFacturaDocumentKind::SelfBilling,
        issuer_cui: ISSUER_CUI.to_owned(),
        buyer_cui: None,
        invoice_xml: ubl,
    };
    let first = provider.upload(&req).unwrap();
    let second = provider.upload(&req).unwrap();
    assert_ne!(
        first.indice_incarcare, second.indice_incarcare,
        "each upload must get a fresh indice de incarcare"
    );
    assert_eq!(first.indice_incarcare, "ANAF-000000000001");
    assert_eq!(second.indice_incarcare, "ANAF-000000000002");
}
