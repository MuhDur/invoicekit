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
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2000), // Turkey standard KDV/VAT 20%
            tax_rate: Some(DecimalValue::new(Decimal::new(2000, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
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

    // 2-5. serialize -> submit (GİB mock) -> pack evidence (shared with the
    // deepened scenarios). The default `submit_request` (no mutation) mirrors
    // the original inline path byte-for-byte.
    let (ikb, ubl_xml, envelope) = bundle_for(&doc, |_| {});

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

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios (added on top of the §1 honest bar).
//
// Each scenario grounds an assertion in the real Turkish e-invoicing rules
// published by the Gelir İdaresi Başkanlığı (GİB — the Revenue Administration)
// in the UBL-TR specification and its accompanying code lists:
//
//   * UBL-TR 1.2 / e-Fatura & e-Arşiv packages and the "UBL-TR Kod Listeleri"
//     (code-lists) guide — GİB e-Belge portal, https://ebelge.gib.gov.tr/ and
//     https://efatura.gov.tr/ . The `cbc:InvoiceTypeCode` value list there
//     defines SATIS (sale), IADE (return / corrective), TEVKIFAT (VAT
//     withholding), ISTISNA (VAT exemption) and IHRACKAYITLI (export-registered).
//   * The IADE / TEVKIFATIADE rule: a return invoice MUST carry at least one
//     `cac:BillingReference/cac:InvoiceDocumentReference` pointing at the
//     refunded invoice (GİB "UBL-TR Kod Listeleri" guide; e-Fatura package
//     validation rule on cbc:DocumentTypeCode = İADE).
//   * Turkish KDV (VAT) bands: 20% standard, with reduced 10% and 1% bands
//     (Katma Değer Vergisi Kanunu; rates set by Cumhurbaşkanı kararı).
//   * ETTN (Evrensel Tekil Tanımlama Numarası), VKN (10-digit legal-entity tax
//     id) and TCKN (11-digit individual id) — the identifiers GİB binds to each
//     cleared e-Fatura / e-Arşiv document.
// ---------------------------------------------------------------------------

/// Build a one-line Turkish UBL-TR document at a given KDV band. `category_code`
/// is the UBL tax-category code the serializer echoes into both the line
/// `ClassifiedTaxCategory` and the document `TaxSubtotal`; `rate_bps` is the KDV
/// percentage in basis points (2000 => 20.00%), and `tax_minor` the KDV amount
/// in minor units. Reused so every band/scenario stays byte-stable.
#[allow(clippy::too_many_arguments)] // a test-fixture builder; a params struct would be noise here
fn tr_doc(
    id: &str,
    number: &str,
    document_type: DocumentType,
    due_date: Option<DateOnly>,
    description: &str,
    category_code: &str,
    base_minor: i64,
    rate_bps: i64,
    tax_minor: i64,
    extensions: Vec<invoicekit_ir::JurisdictionExtension>,
) -> CommercialDocument {
    let total_minor = base_minor + tax_minor;
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(id).unwrap(),
        document_type,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date,
        document_number: DocumentNumber::new(number).unwrap(),
        currency: Iso4217Code::new("TRY").unwrap(),
        supplier: turkish_party("Acme Anonim Sirketi", ISSUER_VKN, "Istanbul", "34"),
        customer: turkish_party("Beta Limited Sirketi", BUYER_VKN, "Ankara", "06"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: description.to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(base_minor),
            line_extension_amount: amt(base_minor),
            tax_category: Some(category_code.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: category_code.to_owned(),
            taxable_amount: amt(base_minor),
            tax_amount: amt(tax_minor),
            // Scale-2 percentage so it renders e.g. "20.00" / "10.00" / "0.00".
            tax_rate: Some(DecimalValue::new(Decimal::new(rate_bps, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(base_minor),
            tax_exclusive_amount: amt(base_minor),
            tax_inclusive_amount: amt(total_minor),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(total_minor),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions,
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// Steps 2-5 for an arbitrary Turkish document and submit request, reusing the
/// pinned transmission context and timestamps so the output stays byte-stable.
/// Returns `(ikb, ubl_xml, envelope)`.
fn bundle_for(
    doc: &CommercialDocument,
    mutate_request: impl FnOnce(&mut EFaturaSubmitRequest),
) -> (Vec<u8>, String, EFaturaSubmitEnvelope) {
    let ubl_xml = to_xml(doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();

    let mut request = submit_request(ubl_bytes.clone());
    mutate_request(&mut request);

    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);
    let envelope = provider.submit_invoice(&request).unwrap();

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
    (ikb, ubl_xml, envelope)
}

/// A GİB IADE (return / corrective) document: the UBL-TR equivalent of a credit
/// note. GİB does not use a UBL `CreditNote` for B2B corrections — it uses an
/// `Invoice` whose `cbc:InvoiceTypeCode` is `IADE`, and the e-Fatura package
/// validation rule requires at least one
/// `cac:BillingReference/cac:InvoiceDocumentReference` carrying the refunded
/// invoice's id (GİB "UBL-TR Kod Listeleri" guide, <https://ebelge.gib.gov.tr/>).
/// We thread a real `cac:BillingReference` (pointing at the original ETTN) into
/// the UBL via the document-fields top-level override extension, so the
/// corrective reference actually appears in the wire payload.
#[test]
fn turkey_iade_return_invoice_carries_billing_reference() {
    let original_ettn = "MOCK-00000000001"; // the cleared id we are reversing
    let billing_reference = format!(
        r#"<cac:BillingReference xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cac:InvoiceDocumentReference><cbc:ID>{original_ettn}</cbc:ID><cbc:DocumentTypeCode>IADE</cbc:DocumentTypeCode></cac:InvoiceDocumentReference></cac:BillingReference>"#
    );
    let document_fields = invoicekit_ir::JurisdictionExtension::new(
        invoicekit_format_ubl::UBL_DOCUMENT_FIELDS_EXTENSION_URN,
        serde_json::json!({
            "top_level": [
                { "element": "cac:BillingReference", "xml": billing_reference }
            ]
        }),
    )
    .unwrap();

    // An IADE document reverses a 20% KDV sale of 100.00 (KDV 20.00).
    let doc = tr_doc(
        "doc-tr-e2e-iade-1",
        "IADE-2026-TR-0001",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Iade faturasi (return invoice)",
        "S",
        10000,
        2000,
        2000,
        vec![document_fields],
    );

    let (ikb, ubl, envelope) = bundle_for(&doc, |_| {});

    // The corrective billing reference (and the refunded id) reached the wire.
    assert!(
        ubl.contains("BillingReference"),
        "an IADE return invoice must carry a BillingReference, got:\n{ubl}"
    );
    assert!(
        ubl.contains("InvoiceDocumentReference"),
        "the BillingReference must wrap an InvoiceDocumentReference"
    );
    assert!(
        ubl.contains(&format!(">{original_ettn}<")),
        "the BillingReference must point at the refunded invoice's id {original_ettn}"
    );
    assert!(
        ubl.contains(">IADE<"),
        "the referenced document type code must be IADE per the GİB code list"
    );
    // GİB clears the corrective document and binds an ETTN to it.
    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "IADE evidence bundle must verify");
}

/// A standard-rate domestic B2B sale carries Turkey's 20% KDV band. The UBL-TR
/// payload must render `<cbc:Percent>20.00</cbc:Percent>` inside the document
/// `TaxSubtotal` and the per-line `ClassifiedTaxCategory`. Turkey's standard KDV
/// rate is 20% (Katma Değer Vergisi Kanunu; rate set by Cumhurbaşkanı kararı).
#[test]
fn turkey_standard_kdv_band_renders_twenty_percent() {
    let doc = tr_doc(
        "doc-tr-e2e-kdv20-1",
        "INV-2026-TR-KDV20",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Standart oranli teslim 20%",
        "S",
        10000,
        2000,
        2000,
        Vec::new(),
    );
    let (ikb, ubl, envelope) = bundle_for(&doc, |_| {});

    assert!(
        ubl.contains(">20.00</cbc:Percent>"),
        "the 20% KDV band must render a 20.00 Percent, got:\n{ubl}"
    );
    // VAT scheme id and the taxable/charged amounts for the band.
    assert!(ubl.contains(">VAT<"), "the tax scheme id must be VAT");
    assert!(ubl.contains(">100.00<"), "taxable base 100.00 must appear");
    assert!(ubl.contains(">120.00<"), "tax-inclusive total 120.00 must appear");
    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "standard-rate evidence bundle must verify");
}

/// A reduced-rate sale at Turkey's 10% KDV band (applied to e.g. certain food
/// and textile supplies). Proves the per-band rate lookup renders the reduced
/// band, distinct from the 20% standard band, end to end. Turkey runs reduced
/// KDV bands of 10% and 1% alongside the 20% standard rate (KDV Kanunu).
#[test]
fn turkey_reduced_kdv_band_renders_ten_percent() {
    let doc = tr_doc(
        "doc-tr-e2e-kdv10-1",
        "INV-2026-TR-KDV10",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Indirimli oranli teslim 10%",
        "S",
        20000,
        1000,
        2000,
        Vec::new(),
    );
    let (ikb, ubl, envelope) = bundle_for(&doc, |_| {});

    assert!(
        ubl.contains(">10.00</cbc:Percent>"),
        "the reduced 10% KDV band must render a 10.00 Percent, got:\n{ubl}"
    );
    assert!(
        !ubl.contains(">20.00</cbc:Percent>"),
        "the standard 20% band must not leak into a reduced-rate-only invoice"
    );
    // 10% of 200.00 == 20.00 KDV, total 220.00.
    assert!(ubl.contains(">200.00<"), "reduced-rate taxable base 200.00 must appear");
    assert!(ubl.contains(">220.00<"), "reduced-rate total 220.00 must appear");
    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "reduced-rate evidence bundle must verify");
}

/// A KDV-exempt (ISTISNA) sale: the supplier charges no VAT, so the line and
/// the document `TaxSubtotal` carry a 0.00 KDV amount and a 0.00 Percent. In
/// UBL-TR an exemption is signalled by an `ISTISNA` `cbc:InvoiceTypeCode` plus
/// the relevant exemption-reason code (GİB "UBL-TR Kod Listeleri" guide). This
/// exercises the zero-rate path: taxable base equals the payable total.
#[test]
fn turkey_istisna_exempt_invoice_is_zero_rated() {
    let doc = tr_doc(
        "doc-tr-e2e-istisna-1",
        "INV-2026-TR-ISTISNA",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "KDV istisnasi (exempt supply)",
        "E", // UBL category "E" = Exempt from tax
        50000,
        0,
        0,
        Vec::new(),
    );
    let (ikb, ubl, envelope) = bundle_for(&doc, |_| {});

    assert!(
        ubl.contains(">0.00</cbc:Percent>"),
        "an ISTISNA exempt line must carry a 0.00 KDV Percent, got:\n{ubl}"
    );
    assert!(
        ubl.contains(">E<"),
        "the exempt tax-category code E must appear in the ClassifiedTaxCategory"
    );
    // No VAT charged: taxable base equals the payable total (500.00).
    assert!(ubl.contains(">500.00<"), "the exempt taxable/payable amount 500.00 must appear");
    assert!(
        !ubl.contains(">20.00</cbc:Percent>"),
        "the 20% standard band must not appear on an exempt invoice"
    );
    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "ISTISNA exempt evidence bundle must verify");
}

/// e-Arşiv B2C export to a non-registered individual receiver: the buyer is not
/// on the e-Fatura mukellef (registered-taxpayer) list, so `buyer_tax_id` is
/// `None` and the mandate is `EArsiv`. GİB's e-Arşiv path accepts the same
/// UBL-TR wire format and clears the document; the receipt still binds an ETTN.
#[test]
fn turkey_earsiv_b2c_no_buyer_tax_id_clears_and_bundles() {
    let doc = tr_doc(
        "doc-tr-e2e-earsiv-1",
        "INV-2026-TR-EARSIV",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "e-Arsiv perakende satis (B2C)",
        "S",
        10000,
        2000,
        2000,
        Vec::new(),
    );
    let (ikb, _ubl, envelope) = bundle_for(&doc, |req| {
        req.mandate = EFaturaMandate::EArsiv;
        req.buyer_tax_id = None;
    });

    assert_eq!(
        envelope.status,
        EFaturaStatus::Cleared,
        "e-Arşiv B2C with no buyer tax id must still clear"
    );
    assert!(
        envelope.ettn.starts_with("MOCK-"),
        "GİB must bind an ETTN even on the e-Arşiv B2C path"
    );
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "e-Arşiv B2C evidence bundle must verify");
}

/// e-Fatura to an individual buyer identified by an 11-digit TCKN (Türkiye
/// Cumhuriyeti Kimlik Numarası), distinct from a 10-digit legal-entity VKN. The
/// adapter accepts either shape on the buyer side; the full lifecycle must clear
/// and bundle. TCKN is the GİB identifier for natural persons.
#[test]
fn turkey_efatura_to_individual_tckn_buyer_clears() {
    const BUYER_TCKN: &str = "12345678901"; // 11-digit individual id
    let doc = tr_doc(
        "doc-tr-e2e-tckn-1",
        "INV-2026-TR-TCKN",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Sahis aliciya satis (TCKN)",
        "S",
        10000,
        2000,
        2000,
        Vec::new(),
    );
    let (ikb, _ubl, envelope) = bundle_for(&doc, |req| {
        req.buyer_tax_id = Some(BUYER_TCKN.to_owned());
    });

    assert_eq!(envelope.status, EFaturaStatus::Cleared);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "TCKN-buyer evidence bundle must verify");
}

/// An 11-character (not 10) issuer VKN is a malformed legal-entity tax id and
/// must be refused **before** the wire with `BadTaxId`. A VKN is exactly 10
/// digits; an 11-digit value is a TCKN shape, never a valid issuer VKN. This is
/// distinct from a GİB wire rejection (a receipt status), which the mock cannot
/// synthesize.
#[test]
fn turkey_eleven_digit_issuer_vkn_refused_pre_wire() {
    let doc = tr_doc(
        "doc-tr-e2e-badvkn-1",
        "INV-2026-TR-BADVKN",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Hatali VKN",
        "S",
        10000,
        2000,
        2000,
        Vec::new(),
    );
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);

    let mut bad = submit_request(ubl_bytes);
    bad.issuer_vkn = "12345678901".to_owned(); // 11 digits => TCKN shape, invalid VKN
    let err = provider.submit_invoice(&bad).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_tr_efatura::EFaturaError::BadTaxId(_)
        ),
        "an 11-digit issuer VKN must refuse pre-wire as BadTaxId, got {err:?}"
    );
}

/// A non-numeric buyer tax id (neither a 10-digit VKN nor an 11-digit TCKN) must
/// refuse pre-wire with `BadTaxId`, never as a cleared receipt. The buyer id is
/// validated as a VKN-or-TCKN shape before anything reaches GİB.
#[test]
fn turkey_non_numeric_buyer_tax_id_refused_pre_wire() {
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);
    let mut bad = submit_request(b"<Invoice/>".to_vec());
    bad.buyer_tax_id = Some("TR-NOT-DIGITS".to_owned());
    let err = provider.submit_invoice(&bad).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_tr_efatura::EFaturaError::BadTaxId(_)
        ),
        "a non-numeric buyer tax id must refuse pre-wire as BadTaxId, got {err:?}"
    );
}

/// GİB buyer rejection (Red Yanıtı) is a receipt **status**, NOT an `Err`. The
/// `MockEFaturaProvider` always clears, so the mock cannot synthesize a wire
/// rejection — but the typed `EFaturaStatus::Rejected` verdict is part of the
/// envelope contract and the audit trail must persist it. We assemble an
/// evidence bundle carrying a `Rejected` receipt (with the buyer's reason) and
/// prove the bundle still packs and verifies — mirroring the §1 contract that a
/// refusal is recorded, not thrown.
#[test]
fn turkey_buyer_rejection_red_yaniti_is_receipt_not_error() {
    let doc = tr_doc(
        "doc-tr-e2e-red-1",
        "INV-2026-TR-RED",
        DocumentType::Invoice,
        Some(DateOnly::new("2026-06-25").unwrap()),
        "Reddedilen fatura",
        "S",
        10000,
        2000,
        2000,
        Vec::new(),
    );
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // A buyer-rejection (Red Yanıtı) receipt: a typed verdict, carried in an
    // envelope, with the buyer's stated reason.
    let rejected = EFaturaSubmitEnvelope {
        ettn: "MOCK-0000000002a".to_owned(),
        status: EFaturaStatus::Rejected,
        submitted_at: PINNED_SUBMITTED_AT.to_owned(),
        message: Some("Alici reddetti: hatali tutar (Red Yaniti)".to_owned()),
    };
    assert_eq!(rejected.status, EFaturaStatus::Rejected);

    // The rejection round-trips through serde unchanged (audit-trail fidelity).
    let receipt_bytes = serde_json::to_vec(&rejected).unwrap();
    let reparsed: EFaturaSubmitEnvelope = serde_json::from_slice(&receipt_bytes).unwrap();
    assert_eq!(reparsed, rejected);

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert("receipt.json".to_owned(), receipt_bytes);
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejected-receipt evidence bundle must still verify");
}

/// İptal (cancellation) within the legal window is a `Cancelled` receipt, not an
/// `Err`. The cancel verb echoes the operator's reason. GİB allows an issuer to
/// cancel a cleared e-Fatura inside the statutory window; the audit trail keeps
/// the cancellation as a receipt.
#[test]
fn turkey_iptal_cancellation_returns_cancelled_receipt() {
    let provider = MockEFaturaProvider::with_fixed_submitted_at(PINNED_SUBMITTED_AT);
    let envelope = provider
        .cancel_invoice(
            EFaturaEnvironment::Production,
            "MOCK-00000000001",
            "Yanlis alici (wrong buyer) - iptal",
        )
        .unwrap();
    assert_eq!(envelope.status, EFaturaStatus::Cancelled);
    assert_eq!(envelope.ettn, "MOCK-00000000001");
    assert_eq!(
        envelope.message.as_deref(),
        Some("Yanlis alici (wrong buyer) - iptal")
    );
    assert_eq!(envelope.submitted_at, PINNED_SUBMITTED_AT);
}

/// The full IADE corrective lifecycle (build -> UBL-TR -> submit -> bundle) must
/// be byte-identical across runs, including the threaded `cac:BillingReference`.
/// Determinism is load-bearing for the evidence bundle's content address.
#[test]
fn turkey_iade_lifecycle_is_byte_deterministic() {
    let billing_reference = r#"<cac:BillingReference xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cac:InvoiceDocumentReference><cbc:ID>MOCK-00000000001</cbc:ID><cbc:DocumentTypeCode>IADE</cbc:DocumentTypeCode></cac:InvoiceDocumentReference></cac:BillingReference>"#;
    let make = || {
        let ext = invoicekit_ir::JurisdictionExtension::new(
            invoicekit_format_ubl::UBL_DOCUMENT_FIELDS_EXTENSION_URN,
            serde_json::json!({
                "top_level": [
                    { "element": "cac:BillingReference", "xml": billing_reference }
                ]
            }),
        )
        .unwrap();
        tr_doc(
            "doc-tr-e2e-iade-det-1",
            "IADE-2026-TR-DET",
            DocumentType::Invoice,
            Some(DateOnly::new("2026-06-25").unwrap()),
            "Iade faturasi (deterministic)",
            "S",
            10000,
            2000,
            2000,
            vec![ext],
        )
    };
    let (a, ubl_a, _) = bundle_for(&make(), |_| {});
    let (b, ubl_b, _) = bundle_for(&make(), |_| {});
    assert_eq!(ubl_a, ubl_b, "UBL-TR serialization must be stable");
    assert_eq!(a, b, "the whole IADE lifecycle must be byte-stable");
}
