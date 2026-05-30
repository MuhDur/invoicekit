// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! India GST / IRP offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for India and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("IN")` + INR
//! 2. serialize -> EN 16931 / UBL 2.1 XML (the family path; India layers GST on
//!    top of an EN 16931-shaped invoice)
//! 3. submit the serialized bytes to the crate's existing `MockIrpProvider` and
//!    assert the IRP receipt's India-specific fields (IRN / ack no / signed QR /
//!    signed JWS / status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for` with a pinned `created_at`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the mock rejects a malformed GSTIN before the wire (`Err`)
//!
//! The `MockIrpProvider` synthesises the IRN/QR/JWS itself, so no `Signer` is
//! wired here. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
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
use invoicekit_report_in_gst::{
    to_inv01_json, validate_hsn_sac, Inv01Context, IrpBackend, IrpEnvironment, IrpError,
    IrpProvider, IrpRegisterEnvelope, IrpRegisterRequest, IrpStatus, MockIrpProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_in_e2e";
const TRACE: &str = "trace_in_e2e";
const ISSUER_GSTIN: &str = "29AAAPL2356Q1ZS";
const BUYER_GSTIN: &str = "27AAAPL2356Q1ZT";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn indian_party(name: &str, gstin: &str, city: &str, state: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            // India's GST identity. `vat` is the IR's generic tax-scheme slot.
            scheme: "gst".to_owned(),
            value: gstin.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1 MG Road".to_owned()],
            city: city.to_owned(),
            subdivision: Some(state.to_owned()),
            postal_code: "560001".to_owned(),
            country: CountryCode::new("IN").unwrap(),
        },
        contact: None,
    }
}

fn indian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IN-0001").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier: indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA"),
        customer: indian_party("Beta Solutions Pvt Ltd", BUYER_GSTIN, "Mumbai", "MH"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting services (SAC 998314)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()), // UBL family uses EA
            unit_price: amt(500_000),            // 5000.00
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // 18% GST (9% CGST + 9% SGST collapses to one EN16931 summary line).
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000),     // 10000.00
            tax_amount: amt(180_000),           // 1800.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))), // 18.00
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_exclusive_amount: amt(1_000_000),  // 10000.00
            tax_inclusive_amount: amt(1_180_000),  // 11800.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_180_000), // 11800.00
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

fn register_request(invoice_json: Vec<u8>) -> IrpRegisterRequest {
    IrpRegisterRequest {
        tenant_id: TENANT.to_owned(),
        environment: IrpEnvironment::Sandbox,
        backend: IrpBackend::Nic1,
        issuer_gstin: ISSUER_GSTIN.to_owned(),
        buyer_gstin: Some(BUYER_GSTIN.to_owned()),
        invoice_json,
    }
}

/// Steps 1-4: build -> serialize -> submit to IRP -> assemble `.ikb`.
///
/// Returns the packed bundle bytes plus the IRP receipt so the callers can
/// assert both the India-specific receipt fields and bundle verifiability.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_in_gst::IrpRegisterEnvelope) {
    // 1. build
    let doc = indian_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 XML bytes
    let ubl_xml = to_xml(&doc).unwrap();
    // local structural check: the canonical UBL spine is present. The C14N
    // pass relocates namespace declarations onto each element's first use, so
    // assert on prefix-stable substrings, not bare `<cac:X>` tags.
    for needle in [
        "<Invoice",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">INR</cbc:DocumentCurrencyCode>",
        "currencyID=\"INR\">11800.00</cbc:PayableAmount>",
        // The issuer's GSTIN survives the round-trip into the tax-scheme block.
        "29AAAPL2356Q1ZS</cbc:CompanyID>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    // 3. submit the serialized bytes to the IRP mock (it signs + assigns IRN).
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
    let envelope = provider.register_invoice(&register_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national-family XML + IRP receipt.
    let ikb = pack_bundle(&doc, &ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn india_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // India-specific authority receipt fields.
    assert_eq!(envelope.status, IrpStatus::Accepted);
    let irn = envelope.irn.as_ref().expect("IRN present on Accepted");
    assert_eq!(irn.len(), 64, "IRN is a 64-char SHA-256 hex");
    assert!(
        irn.bytes().all(|b| b.is_ascii_hexdigit()),
        "IRN must be hex"
    );
    assert!(
        envelope.ack_no.as_ref().is_some_and(|s| s.starts_with("ACK-")),
        "IRP acknowledgement number present"
    );
    assert_eq!(envelope.ack_dt, PINNED_CREATED_AT);
    assert!(
        envelope.signed_qr_code.is_some(),
        "signed QR for the printed invoice"
    );
    assert!(
        envelope.signed_invoice_jws.is_some(),
        "JWS for offline verification"
    );
    assert!(envelope.error_message.is_none());

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn india_lifecycle_is_byte_deterministic() {
    let (a, env_a) = run_lifecycle();
    let (b, env_b) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
    // Same payload -> same synthesised IRN across independent provider instances.
    assert_eq!(env_a.irn, env_b.irn, "IRN derivation must be deterministic");
}

#[test]
fn india_duplicate_resubmit_is_reported_not_errored() {
    // Resubmitting the same payload to the SAME provider yields a Duplicate
    // verdict (the IRP returns the existing IRN) — surfaced as a status, not an
    // `Err`, so the audit trail records the reconciliation. The bundle from
    // either submission still verifies.
    let doc = indian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    let first = provider.register_invoice(&register_request(ubl_bytes.clone())).unwrap();
    let second = provider.register_invoice(&register_request(ubl_bytes)).unwrap();

    assert_eq!(first.status, IrpStatus::Accepted);
    assert_eq!(second.status, IrpStatus::Duplicate);
    assert_eq!(first.irn, second.irn, "Duplicate returns the existing IRN");
}

#[test]
fn india_refuses_malformed_gstin_before_the_wire() {
    // The MockIrpProvider does NOT expose a force-rejection hook, so an
    // authority `IrpStatus::Rejected` verdict cannot be synthesised offline.
    // The refusal path the mock DOES support is pre-wire shape validation: a
    // malformed GSTIN is an `Err`, never a packed bundle.
    let doc = indian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    let mut bad = register_request(ubl_bytes);
    bad.issuer_gstin = "TOO-SHORT".to_owned();
    let err = provider.register_invoice(&bad).unwrap_err();
    assert!(matches!(err, IrpError::BadGstin(_)), "got {err:?}");
}

// ===========================================================================
// Native national-format lifecycle: serialize the REAL IRP INV-01 JSON (NOT
// the UBL family XML), validate its structure, transmit via the existing mock
// IRP, bundle, and verify. This is the country-format spine the IRP actually
// accepts; the UBL path above is the family-format spine.
//
// Authority: GSTN / NIC e-Invoice JSON schema `INV-01` (schema version 1.1).
// Spec: <https://einvoice1.gst.gov.in/Others/BulkGenerationTools>
// ===========================================================================

/// Build a packed `.ikb` bundle whose national artefact is the INV-01 JSON
/// (under `formats/inv01.json`) rather than the UBL XML.
fn pack_inv01_bundle(
    doc: &CommercialDocument,
    inv01_bytes: &[u8],
    envelope: &IrpRegisterEnvelope,
) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/inv01.json".to_owned(), inv01_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Serialize -> structurally validate -> transmit -> bundle, over the native
/// INV-01 JSON. Returns `(ikb, inv01_json_string, envelope)`.
fn run_inv01_lifecycle(
    doc: &CommercialDocument,
) -> (Vec<u8>, String, IrpRegisterEnvelope) {
    // 1. serialize -> REAL national INV-01 JSON (NOT UBL).
    let inv01 = to_inv01_json(doc, &Inv01Context::default()).unwrap();

    // 2. validate structure: parse the JSON and assert the mandatory INV-01
    //    spine with the schema's actual abbreviated field names.
    let v: serde_json::Value = serde_json::from_str(&inv01).unwrap();
    assert_eq!(v["Version"], "1.1", "INV-01 schema version pin");
    assert_eq!(v["TranDtls"]["TaxSch"], "GST", "TranDtls.TaxSch must be GST");
    assert!(v["DocDtls"]["Typ"].is_string(), "DocDtls.Typ present");
    assert!(v["DocDtls"]["No"].is_string(), "DocDtls.No present");
    assert!(v["SellerDtls"]["Gstin"].is_string(), "SellerDtls.Gstin present");
    assert!(v["BuyerDtls"]["Gstin"].is_string(), "BuyerDtls.Gstin present");
    assert!(v["ItemList"].is_array(), "ItemList is an array");
    assert!(v["ValDtls"]["TotInvVal"].is_string(), "ValDtls.TotInvVal present");
    // Each item carries the mandatory per-line fields.
    for item in v["ItemList"].as_array().unwrap() {
        for key in ["SlNo", "HsnCd", "Qty", "UnitPrice", "TotAmt", "AssAmt", "GstRt", "TotItemVal"] {
            assert!(item.get(key).is_some(), "ItemList entry missing {key}");
        }
    }

    let inv01_bytes = inv01.clone().into_bytes();

    // 3. transmit via the existing mock IRP (signs + assigns IRN).
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
    let envelope = provider
        .register_invoice(&register_request(inv01_bytes.clone()))
        .unwrap();

    // 4. evidence bundle.
    let ikb = pack_inv01_bundle(doc, &inv01_bytes, &envelope);
    (ikb, inv01, envelope)
}

#[test]
fn india_native_inv01_lifecycle_produces_verifiable_evidence() {
    // Inter-state supply (Karnataka 29 -> Maharashtra 27): the native INV-01
    // charges IGST at the full headline rate (no CGST/SGST split).
    let doc = indian_invoice();
    let (ikb, inv01, envelope) = run_inv01_lifecycle(&doc);

    // The serialized national artefact is JSON with INV-01 IGST fields.
    let v: serde_json::Value = serde_json::from_str(&inv01).unwrap();
    assert_eq!(v["SellerDtls"]["Stcd"], "29", "Karnataka state code");
    assert_eq!(v["BuyerDtls"]["Stcd"], "27", "Maharashtra state code");
    let item = &v["ItemList"][0];
    assert_eq!(item["IgstAmt"], "1800.00", "18% IGST on 10000.00 inter-state");
    assert!(item.get("CgstAmt").is_none(), "inter-state carries no CgstAmt");
    assert_eq!(v["ValDtls"]["IgstVal"], "1800.00");
    assert_eq!(v["ValDtls"]["TotInvVal"], "11800.00");

    // The IRP registered the native payload and returned an IRN.
    assert_eq!(envelope.status, IrpStatus::Accepted);
    assert!(envelope.irn.as_ref().is_some_and(|s| s.len() == 64));

    // The evidence bundle over the native INV-01 artefact verifies.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "INV-01 evidence bundle must verify");
}

#[test]
fn india_native_inv01_intra_state_splits_cgst_sgst() {
    // Intra-state supply (both Karnataka, state code 29): the headline 18%
    // splits into 9% CGST + 9% SGST per item (CGST/IGST Acts 2017). NIC error
    // 2172 fires when IGST is wrongly used intra-state, so the split matters.
    let supplier = indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA");
    let buyer = indian_party("Gamma Pvt Ltd", "29BBBPL6789Q1Z5", "Mysuru", "KA");
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-inv01-intra").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IN-INTRA1").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier,
        customer: buyer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Local IT support (SAC 998314)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(1_000_000),
            line_extension_amount: amt(1_000_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000),
            tax_amount: amt(180_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000),
            tax_exclusive_amount: amt(1_000_000),
            tax_inclusive_amount: amt(1_180_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_180_000),
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
    .unwrap();

    let (ikb, inv01, envelope) = run_inv01_lifecycle(&doc);
    let v: serde_json::Value = serde_json::from_str(&inv01).unwrap();
    let item = &v["ItemList"][0];
    assert_eq!(item["CgstAmt"], "900.00", "9% CGST on 10000.00");
    assert_eq!(item["SgstAmt"], "900.00", "9% SGST on 10000.00");
    assert!(item.get("IgstAmt").is_none(), "intra-state carries no IgstAmt");
    assert_eq!(v["ValDtls"]["CgstVal"], "900.00");
    assert_eq!(v["ValDtls"]["SgstVal"], "900.00");
    assert_eq!(v["ValDtls"]["IgstVal"], "0.00");
    assert_eq!(v["ValDtls"]["TotInvVal"], "11800.00");

    assert_eq!(envelope.status, IrpStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "intra-state INV-01 bundle must verify");
}

#[test]
fn india_native_inv01_lifecycle_is_byte_deterministic() {
    // The whole native-format lifecycle (serialize -> transmit -> bundle) must
    // be byte-identical across runs: the evidence bundle's content address
    // depends on it. INV-01 key order is fixed (no map-driven reordering).
    let doc = indian_invoice();
    let (a, json_a, env_a) = run_inv01_lifecycle(&doc);
    let (b, json_b, env_b) = run_inv01_lifecycle(&doc);
    assert_eq!(json_a, json_b, "INV-01 serialization must be byte-stable");
    assert_eq!(a, b, "the whole native-format lifecycle must be byte-stable");
    assert_eq!(env_a.irn, env_b.irn, "IRN derivation is deterministic");
}

#[test]
fn india_native_inv01_credit_note_maps_to_crn() {
    // A GST credit note serializes to native INV-01 with DocDtls.Typ = CRN
    // (CGST Act 2017 s.34), not the UBL CreditNoteTypeCode 381. The lifecycle
    // still registers and the bundle verifies.
    let doc = indian_credit_note();
    let (ikb, inv01, envelope) = run_inv01_lifecycle(&doc);
    let v: serde_json::Value = serde_json::from_str(&inv01).unwrap();
    assert_eq!(v["DocDtls"]["Typ"], "CRN", "credit note maps to INV-01 Typ CRN");
    assert_eq!(v["DocDtls"]["No"], "CRN-2026-IN-0001");
    assert_eq!(envelope.status, IrpStatus::Accepted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "CRN INV-01 bundle must verify");
}

// ===========================================================================
// Deepened, India-GST-specific scenarios (added on top of the §1 honest bar).
//
// Each scenario is grounded in a real Goods and Services Tax Network (GSTN) /
// National Informatics Centre (NIC) Invoice Registration Portal (IRP) rule and
// cites the authority in its doc-comment. Fixtures are hand-built synthetic
// data; no copyrighted regulator file is vendored.
// ===========================================================================

/// A second invoice line carrying a distinct HSN/SAC code, so the multi-line
/// and multi-rate scenarios exercise more than one tax slab.
fn igst_line(id: &str, description: &str, minor_each: i64, qty: i64) -> DocumentLine {
    let total = minor_each * qty;
    DocumentLine {
        id: id.to_owned(),
        description: description.to_owned(),
        quantity: DecimalValue::new(Decimal::from(qty)),
        unit_code: Some("EA".to_owned()),
        unit_price: amt(minor_each),
        line_extension_amount: amt(total),
        tax_category: Some("S".to_owned()),
        classifications: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
    }
}

/// India GST **credit note** (Typ = `CRN`).
///
/// Under the Central Goods and Services Tax Act 2017 section 34, a supplier
/// issues a credit note to reduce the taxable value or tax of a previously
/// reported tax invoice. In the NIC IRP `generate IRN` schema the document type
/// `Typ` takes the value `CRN`, and a credit note must reference the original
/// invoice (`PrecDocDtls`). The InvoiceKit IR maps this to
/// [`DocumentType::CreditNote`], serialized as a UBL 2.1 `CreditNote`
/// (`CreditNoteTypeCode` 381). A UBL `CreditNote` may not carry a top-level
/// `cbc:DueDate`, so `due_date` is `None`.
///
/// Authority: Goods and Services Tax Network / National Informatics Centre,
/// e-Invoice schema, document type `CRN`; CGST Act 2017 s.34.
/// Spec: <https://einvoice1.gst.gov.in/Others/BulkGenerationTools>
fn indian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-06-02").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote carries no top-level cbc:DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CRN-2026-IN-0001").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier: indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA"),
        customer: indian_party("Beta Solutions Pvt Ltd", BUYER_GSTIN, "Mumbai", "MH"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit: partial rollback of SAC 998314 consulting".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(200_000),            // 2000.00
            line_extension_amount: amt(200_000), // 2000.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // 18% GST on the credited amount.
            category_code: "S".to_owned(),
            taxable_amount: amt(200_000), // 2000.00
            tax_amount: amt(36_000),      // 360.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))), // 18.00
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(200_000), // 2000.00
            tax_exclusive_amount: amt(200_000),  // 2000.00
            tax_inclusive_amount: amt(236_000),  // 2360.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(236_000), // 2360.00
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

/// Multi-line domestic invoice spanning a goods HSN and a services SAC.
///
/// Real GST tax invoices list one line per supplied item, each tagged with its
/// own Harmonised System of Nomenclature (HSN) code for goods or Services
/// Accounting Code (SAC) for services. The IRP `ItemList` validates each
/// `HsnCd` independently (4-8 digits), and the NIC `Note on Top Errors` flags
/// missing/short HSN codes as a common refusal.
///
/// Authority: NIC IRP e-Invoice schema, `ItemList[].HsnCd` (4-8 chars).
/// Spec: <https://einv-apisandbox.nic.in/NoteonTopErrors.html>
fn indian_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-ml-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IN-ML01").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier: indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA"),
        customer: indian_party("Beta Solutions Pvt Ltd", BUYER_GSTIN, "Mumbai", "MH"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // HSN 84713010 — laptops (goods).
            igst_line("1", "Laptop computers (HSN 84713010)", 600_000, 2),
            // SAC 998314 — IT consulting (services).
            igst_line("2", "Software consulting (SAC 998314)", 500_000, 1),
        ],
        tax_summary: vec![TaxCategorySummary {
            // Both lines at 18% IGST for an inter-state (KA -> MH) supply.
            category_code: "S".to_owned(),
            taxable_amount: amt(1_700_000), // 17000.00
            tax_amount: amt(306_000),       // 3060.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))), // 18.00
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_700_000), // 17000.00
            tax_exclusive_amount: amt(1_700_000),  // 17000.00
            tax_inclusive_amount: amt(2_006_000),  // 20060.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(2_006_000), // 20060.00
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

/// Export-without-payment (zero-rated) invoice, supply type `EXPWOP`.
///
/// Under the Integrated GST Act 2017 section 16, exports are a zero-rated
/// supply. A registered exporter shipping under a Letter of Undertaking (LUT)
/// supplies the goods/services **without payment of integrated tax** — the NIC
/// IRP schema models this with supply type `SupTyp = EXPWOP`. The foreign buyer
/// has no GSTIN, so `buyer_gstin` is `None`; the IRP still registers the export
/// invoice and returns an IRN.
///
/// Authority: NIC IRP e-Invoice schema, `SupTyp = EXPWOP`; IGST Act 2017 s.16.
/// Spec: <https://einvoice1.gst.gov.in/Others/BulkGenerationTools>
fn indian_export_lut_invoice() -> CommercialDocument {
    let foreign_buyer = Party {
        id: Some("acme-usa-inc".to_owned()),
        name: "Acme USA Inc".to_owned(),
        // Foreign buyer carries no GSTIN under EXPWOP.
        tax_ids: Vec::new(),
        address: PostalAddress {
            lines: vec!["1 Market Street".to_owned()],
            city: "San Francisco".to_owned(),
            subdivision: Some("CA".to_owned()),
            postal_code: "94105".to_owned(),
            country: CountryCode::new("US").unwrap(),
        },
        contact: None,
    };
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-exp-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("EXP-2026-IN-0001").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier: indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA"),
        customer: foreign_buyer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exported software services (SAC 998314), under LUT".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5_000_000),            // 50000.00
            line_extension_amount: amt(5_000_000), // 50000.00
            // Zero-rated: tax category Z.
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // Zero-rated export under LUT: 0% IGST, zero tax.
            category_code: "Z".to_owned(),
            taxable_amount: amt(5_000_000), // 50000.00
            tax_amount: amt(0),             // 0.00
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)), // 0
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5_000_000), // 50000.00
            tax_exclusive_amount: amt(5_000_000),  // 50000.00
            // No tax added: payable equals taxable.
            tax_inclusive_amount: amt(5_000_000), // 50000.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5_000_000), // 50000.00
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

/// Build a packed `.ikb` bundle for a given document + registered envelope so
/// the new scenarios reuse the same evidence assembly as `run_lifecycle`.
fn pack_bundle(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &IrpRegisterEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes.to_vec());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

#[test]
fn india_credit_note_registers_and_bundles() {
    // GST credit note (Typ = CRN) serialized as a UBL 2.1 CreditNote, then
    // registered with the IRP exactly like a tax invoice. See CGST Act 2017
    // s.34. The UBL spine must be a CreditNote (TypeCode 381), not an Invoice.
    let doc = indian_credit_note();
    let ubl_xml = to_xml(&doc).unwrap();
    assert!(
        ubl_xml.contains("<CreditNote"),
        "credit note must serialize to a UBL CreditNote root"
    );
    assert!(
        ubl_xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL CreditNoteTypeCode 381 expected"
    );
    assert!(
        ubl_xml.contains("currencyID=\"INR\">2360.00</cbc:PayableAmount>"),
        "credit note payable total 2360.00 INR expected"
    );
    let ubl_bytes = ubl_xml.into_bytes();

    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
    let envelope = provider
        .register_invoice(&register_request(ubl_bytes.clone()))
        .unwrap();
    assert_eq!(envelope.status, IrpStatus::Accepted);
    let irn = envelope.irn.as_ref().expect("IRN on accepted credit note");
    assert_eq!(irn.len(), 64, "IRN is a 64-char SHA-256 hex");

    let ikb = pack_bundle(&doc, &ubl_bytes, &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

#[test]
fn india_multiline_inter_state_invoice_registers() {
    // A two-line inter-state (Karnataka -> Maharashtra) supply: one goods HSN
    // line and one services SAC line. The serialized UBL must carry both lines
    // and both per-line extension amounts; the IRP registers the whole invoice
    // and returns one IRN for the document (not per line). NIC `Note on Top
    // Errors` lists HSN problems as a common refusal cause.
    let doc = indian_multiline_invoice();
    assert_eq!(doc.lines.len(), 2, "multi-line invoice has two lines");

    let ubl_xml = to_xml(&doc).unwrap();
    for needle in [
        "Laptop computers (HSN 84713010)",
        "Software consulting (SAC 998314)",
        "currencyID=\"INR\">12000.00</cbc:LineExtensionAmount>", // line 1: 6000 x 2
        "currencyID=\"INR\">5000.00</cbc:LineExtensionAmount>",  // line 2: 5000 x 1
        "currencyID=\"INR\">20060.00</cbc:PayableAmount>",       // grand total incl. 18% IGST
    ] {
        assert!(ubl_xml.contains(needle), "multi-line UBL missing {needle}");
    }
    let ubl_bytes = ubl_xml.into_bytes();

    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
    let envelope = provider
        .register_invoice(&register_request(ubl_bytes.clone()))
        .unwrap();
    assert_eq!(envelope.status, IrpStatus::Accepted);

    let ikb = pack_bundle(&doc, &ubl_bytes, &envelope);
    assert!(
        verify_packed(&ikb, &VerifyOptions::content_only())
            .unwrap()
            .ok
    );
}

#[test]
fn india_export_under_lut_is_zero_rated_and_has_no_buyer_gstin() {
    // EXPWOP: export without payment of tax, supplied under a Letter of
    // Undertaking. IGST Act 2017 s.16 makes exports zero-rated; the foreign
    // buyer carries no GSTIN. The IRP accepts the invoice and the engine never
    // calls the buyer-GSTIN validator (because buyer_gstin is None).
    let doc = indian_export_lut_invoice();
    // Zero-rated: the tax summary tax amount is exactly zero.
    assert_eq!(
        doc.tax_summary[0].tax_amount.inner(),
        Decimal::ZERO,
        "export under LUT carries zero IGST"
    );
    // The foreign buyer carries no tax identifier at all.
    assert!(
        doc.customer.tax_ids.is_empty(),
        "foreign export buyer has no GSTIN"
    );

    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    // No buyer GSTIN: the request mirrors the export schema (buyer_gstin None).
    let mut req = register_request(ubl_bytes);
    req.buyer_gstin = None;
    let envelope = provider.register_invoice(&req).unwrap();
    assert_eq!(
        envelope.status,
        IrpStatus::Accepted,
        "IRP registers a zero-rated export invoice"
    );
    assert!(
        envelope.irn.as_ref().is_some_and(|s| s.len() == 64),
        "export invoice still earns a 64-char IRN"
    );
}

#[test]
fn india_intra_state_reverse_charge_invoice_registers() {
    // Reverse charge (RegRev = Y): under CGST Act 2017 s.9(3)/9(4) the
    // *recipient* discharges the GST liability, not the supplier. This is an
    // intra-state (Karnataka -> Karnataka) supply, so tax splits into CGST +
    // SGST (NIC error 2172 fires when IGST is wrongly used intra-state). We
    // model the combined 18% (9% CGST + 9% SGST) as one EN 16931 summary line.
    //
    // Authority: CGST Act 2017 s.9; NIC IRP error 2172 (IGST on intra-state).
    // Spec: <https://einv-apisandbox.nic.in/NoteonTopErrors.html>
    let supplier = indian_party("Acme Technologies Pvt Ltd", ISSUER_GSTIN, "Bengaluru", "KA");
    // Same-state buyer: GSTIN also starts with state code 29 (Karnataka).
    let buyer_gstin = "29BBBPL6789Q1Z5";
    let buyer = indian_party("Gamma Legal Services LLP", buyer_gstin, "Bengaluru", "KA");

    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-in-rcm-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IN-RCM1").unwrap(),
        currency: Iso4217Code::new("INR").unwrap(),
        supplier,
        customer: buyer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            // Legal services from an advocate are a notified reverse-charge supply.
            description: "Legal advisory services (SAC 998213) - reverse charge".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(1_000_000),            // 10000.00
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000), // 10000.00
            tax_amount: amt(180_000),       // 1800.00 (9% CGST + 9% SGST)
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))), // 18.00
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000), // 10000.00
            tax_exclusive_amount: amt(1_000_000),  // 10000.00
            tax_inclusive_amount: amt(1_180_000),  // 11800.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_180_000), // 11800.00
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
    .unwrap();

    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);

    let mut req = register_request(ubl_bytes);
    // Both parties are Karnataka (state code 29): a genuine intra-state supply.
    req.buyer_gstin = Some(buyer_gstin.to_owned());
    assert_eq!(
        &req.issuer_gstin[..2],
        &req.buyer_gstin.as_ref().unwrap()[..2],
        "intra-state: issuer and buyer share GST state code 29"
    );
    let envelope = provider.register_invoice(&req).unwrap();
    assert_eq!(envelope.status, IrpStatus::Accepted);
    assert!(envelope.signed_qr_code.is_some(), "signed QR for the printed invoice");
}

#[test]
fn india_irp_rejection_receipt_round_trips() {
    // The IRP refusal verdict is a *receipt status*, not a transport error.
    // `IrpStatus::Rejected` carries no IRN and an `error_message` quoting the
    // IRP error code. The canonical example is error 2150 (Duplicate IRN): the
    // same {supplier GSTIN, document type, document number, financial year}
    // cannot mint two IRNs. The MockIrpProvider does not synthesise Rejected
    // (it only models Accepted/Duplicate), so we assert the rejection envelope
    // the real IRP returns survives the receipt.json serde round-trip
    // unchanged — the shape the evidence bundle persists.
    //
    // Authority: NIC IRP error code 2150 (Duplicate IRN).
    // Spec: <https://einv-apisandbox.nic.in/NoteonTopErrors.html>
    let rejected = IrpRegisterEnvelope {
        status: IrpStatus::Rejected,
        irn: None,
        ack_no: None,
        ack_dt: PINNED_CREATED_AT.to_owned(),
        signed_qr_code: None,
        signed_invoice_jws: None,
        error_message: Some(
            "2150 : Duplicate IRN; IRN already generated for the document".to_owned(),
        ),
    };

    // A rejection carries no IRN / QR / JWS, only the authority error text.
    assert_eq!(rejected.status, IrpStatus::Rejected);
    assert!(rejected.irn.is_none(), "rejected receipt has no IRN");
    assert!(
        rejected
            .error_message
            .as_ref()
            .is_some_and(|m| m.starts_with("2150")),
        "rejection quotes the IRP error code 2150"
    );

    // receipt.json round-trip: the envelope the bundle persists is stable.
    let json = serde_json::to_string(&rejected).unwrap();
    // skip_serializing_if drops the None fields from the wire entirely.
    assert!(!json.contains("\"irn\""), "absent IRN is not serialized");
    assert!(!json.contains("signed_qr_code"), "absent QR is not serialized");
    let back: IrpRegisterEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, rejected, "rejection receipt round-trips byte-stable");
}

#[test]
fn india_rejects_too_short_hsn_code() {
    // The NIC IRP `ItemList[].HsnCd` is 4-8 digits; a 3-digit HSN is refused as
    // a malformed item code before the wire. The crate's `validate_hsn_sac`
    // mirrors that 4-8-digit rule exactly. NIC `Note on Top Errors` lists
    // invalid HSN as a common rejection.
    //
    // Authority: NIC IRP e-Invoice schema, `HsnCd` (4-8 chars).
    // Spec: <https://einv-apisandbox.nic.in/NoteonTopErrors.html>
    // A 3-digit HSN is too short.
    let err = validate_hsn_sac("847").unwrap_err();
    assert!(matches!(err, IrpError::BadJson(_)), "got {err:?}");
    // A 9-digit HSN is too long.
    assert!(validate_hsn_sac("847130100").is_err());
    // The real laptop HSN (84713010) and a SAC (998314) are both accepted.
    assert!(validate_hsn_sac("84713010").is_ok(), "8-digit HSN accepted");
    assert!(validate_hsn_sac("998314").is_ok(), "6-digit SAC accepted");
}

#[test]
fn india_credit_note_lifecycle_is_byte_deterministic() {
    // Determinism across the credit-note path: serialize + register + pack
    // twice and assert byte-identical bundles, mirroring the invoice-path
    // determinism guarantee for the CRN document type.
    let build = || {
        let doc = indian_credit_note();
        let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
        let provider = MockIrpProvider::with_fixed_ack_dt(PINNED_CREATED_AT);
        let envelope = provider
            .register_invoice(&register_request(ubl_bytes.clone()))
            .unwrap();
        (pack_bundle(&doc, &ubl_bytes, &envelope), envelope.irn)
    };
    let (a, irn_a) = build();
    let (b, irn_b) = build();
    assert_eq!(a, b, "credit-note lifecycle must be byte-stable");
    assert_eq!(irn_a, irn_b, "credit-note IRN derivation is deterministic");
}
