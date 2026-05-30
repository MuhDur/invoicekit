// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Kenya KRA eTIMS offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Kenya and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("KE")` + KES.
//! 2. serialize -> EN 16931 / UBL 2.1 bytes via `invoicekit_format_ubl::to_xml`
//!    (eTIMS itself accepts JSON over REST; the crate ships no own serializer, so
//!    the UBL family path is the canonical wire artefact we bundle as evidence).
//! 3. submit those bytes to the existing `MockEtimsProvider` and assert the
//!    KRA-specific receipt fields: CU Invoice Number, KRA signature, status.
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), pin `created_at`, `pack`, then `verify_packed` -> `ok == true`.
//! 5. determinism: pack twice -> byte-identical.
//! 6. refusal: the mock has no forced-`Rejected`-status knob (it always returns
//!    `Accepted`), so we exercise the genuine refusal surface it DOES expose —
//!    `Err(EtimsError::BadPin)` on a malformed KRA PIN and `Err(BadPayload)` on an
//!    empty payload — running the same `validate_pin` validator the real impl runs.
//!
//! No `insta`/`pretty_assertions` (they would mutate `Cargo.lock`); goldens are
//! the typed assertions below.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ke_etims::{
    EtimsEnvironment, EtimsError, EtimsProvider, EtimsStatus, EtimsSubmitRequest, MockEtimsProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_ke_e2e";
const TRACE: &str = "trace_ke_e2e";
const ISSUER_PIN: &str = "A123456789Z";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn kenyan_party(name: &str, kra_pin: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "kra-pin".to_owned(),
            value: kra_pin.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Kenyatta Avenue 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "00100".to_owned(),
            country: CountryCode::new("KE").unwrap(),
        },
        contact: None,
    }
}

/// Build a valid Kenyan B2B invoice in the IR. KES, 16% standard VAT.
fn kenyan_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ke-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-KE-0001").unwrap(),
        currency: Iso4217Code::new("KES").unwrap(),
        supplier: kenyan_party("Acme Kenya Ltd", "A123456789Z", "Nairobi"),
        customer: kenyan_party("Beta Traders Ltd", "P051234567M", "Mombasa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Cloud hosting subscription".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            // UBL family uses UN/ECE Rec 20 code "EA" for "each".
            unit_code: Some("EA".to_owned()),
            unit_price: amt(500_000),
            line_extension_amount: amt(1_000_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(1_000_000),
            tax_amount: amt(160_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(1_000_000),
            tax_exclusive_amount: amt(1_000_000),
            tax_inclusive_amount: amt(1_160_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1_160_000),
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

fn submit_request(payload: Vec<u8>) -> EtimsSubmitRequest {
    EtimsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EtimsEnvironment::Sandbox,
        issuer_pin: ISSUER_PIN.to_owned(),
        payload,
    }
}

/// Steps 1-4: build -> serialize -> submit (mock) -> evidence bundle.
///
/// Returns the packed `.ikb` plus the KRA receipt so callers can assert on both.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_ke_etims::EtimsSubmitEnvelope) {
    // 1. build
    let doc = kenyan_invoice();

    // 2. serialize -> EN 16931 / UBL 2.1 bytes (the canonical wire artefact)
    let ubl_xml = to_xml(&doc).unwrap();
    let ubl_bytes = ubl_xml.clone().into_bytes();
    // Structural sanity: the UBL spine carrying KE identity + KES is present.
    // Canonicalization normalizes namespace prefixes and pins `xmlns:` decls
    // inline, so we match on the stable closing tags / open tags that survive.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">KES</cbc:DocumentCurrencyCode>",
        "<cac:Country>",
        ">KE</cbc:IdentificationCode>",
    ] {
        assert!(ubl_xml.contains(needle), "UBL missing {needle}");
    }

    // 3. submit to the existing offline MockEtimsProvider
    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national wire XML + KRA receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&receipt).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, receipt)
}

#[test]
fn kenya_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, receipt) = run_lifecycle();

    // Happy path: KRA accepted, with the country-specific authority artefacts.
    assert_eq!(receipt.status, EtimsStatus::Accepted);
    assert!(
        receipt.cu_invoice_number.starts_with("KE-"),
        "CU Invoice Number must carry the KE prefix, got {:?}",
        receipt.cu_invoice_number
    );
    assert!(
        receipt.kra_signature.starts_with("MOCK-SIG-"),
        "KRA signature must be present, got {:?}",
        receipt.kra_signature
    );
    assert_eq!(receipt.recorded_at, "2026-01-01T00:00:00Z");
    assert!(receipt.reason.is_none());

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn kenya_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn kenya_refuses_malformed_pin_before_the_wire() {
    // The MockEtimsProvider exposes no forced-`Rejected`-status knob (it always
    // returns Accepted), so the genuine refusal surface is the pre-wire
    // validator: a malformed KRA PIN is an Err, never a silent accept.
    let provider = MockEtimsProvider::default();
    let doc = kenyan_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    let mut bad = submit_request(ubl_bytes);
    bad.issuer_pin = "NOT-A-PIN".to_owned();
    let err = provider.submit_invoice(&bad).unwrap_err();
    assert!(
        matches!(err, EtimsError::BadPin(_)),
        "malformed PIN must refuse with BadPin, got {err:?}"
    );
}

#[test]
fn kenya_refuses_empty_payload_before_the_wire() {
    // The second pre-wire refusal: an empty payload never reaches KRA.
    let provider = MockEtimsProvider::default();
    let err = provider.submit_invoice(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, EtimsError::BadPayload(_)),
        "empty payload must refuse with BadPayload, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Country-specific deepening — grounded in the Kenya Revenue Authority (KRA)
// "Technical Specification for the Trader Invoicing System (TIS) for OSCU/VSCU"
// v2.0 (the eTIMS control-unit contract).
//   spec: https://www.kra.go.ke/images/publications/TIS-for-OSCU--VSCU-Technical-Specifications-v2.0.pdf
//
// Anchors used by the scenarios below (section refs from that spec):
//   §6.20.4  TAX rates are labelled "A","B","C","D","E" (indexes 1-5).
//   §40/§41  The receipt "tax components" table maps the labels:
//              A = Goods exempted from VAT (printed "EX")
//              B = VAT at 16% (standard rate)
//              C = Zero rated goods (0%)
//              D = Non Vatable goods
//              E = VAT at 8%
//   §6.23.4  CU Invoice Number = "CU ID" + sequential receipt no., e.g.
//              KRACU04XXXXXXXX/1.
//   §6.16 / §14.1 / §53  A correction is a CREDIT NOTE (transaction type "NC")
//              and MUST reference the ORIGINAL CU INVOICE NO. of the sale it
//              cancels (each original may be cancelled only once).
//   §21.6.3  OSCU/VSCU error codes incl. "32 – wrong PIN in the TIS request
//              data" — i.e. an unregistered/malformed PIN is refused.
// ---------------------------------------------------------------------------

/// VAT amount = taxable * rate, both at minor-unit scale 2. `rate_bp` is the
/// rate in basis points (1600 == 16.00%, 800 == 8.00%, 0 == 0/exempt).
fn vat_minor(taxable_minor: i64, rate_bp: i64) -> i64 {
    taxable_minor * rate_bp / 10_000
}

/// A KES single-line invoice carrying exactly one KRA tax-type label.
/// `category_code` is the eTIMS label (`A`/`B`/`C`/`D`/`E`), `rate_bp` the rate
/// in basis points. Tax math is kept internally consistent so the bundled
/// artefacts are faithful, not a slop placeholder.
fn kenyan_invoice_with_tax(
    number: &str,
    description: &str,
    category_code: &str,
    rate_bp: i64,
    net_minor: i64,
) -> CommercialDocument {
    let tax_minor = vat_minor(net_minor, rate_bp);
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(format!("doc-ke-{}", number.to_lowercase())).unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new(number).unwrap(),
        currency: Iso4217Code::new("KES").unwrap(),
        supplier: kenyan_party("Acme Kenya Ltd", "A123456789Z", "Nairobi"),
        customer: kenyan_party("Beta Traders Ltd", "P051234567M", "Mombasa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: description.to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(net_minor),
            line_extension_amount: amt(net_minor),
            tax_category: Some(category_code.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: category_code.to_owned(),
            taxable_amount: amt(net_minor),
            tax_amount: amt(tax_minor),
            tax_rate: Some(DecimalValue::new(Decimal::new(rate_bp, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(net_minor),
            tax_exclusive_amount: amt(net_minor),
            tax_inclusive_amount: amt(net_minor + tax_minor),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(net_minor + tax_minor),
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

/// KRA receipt label "B" — VAT at 16% (standard rate). The UBL artefact the
/// crate bundles must carry the per-subtotal taxable/tax pair and the 16.00%
/// percent the receipt's tax-components table prints (KRA TIS §40, sample shows
/// `16% 5,485.6 854.40`). Submitting the bytes to eTIMS yields a CU Invoice
/// Number whose shape mirrors §6.23.4's `CU ID + sequence`.
#[test]
fn kenya_standard_rate_16pct_b_label_clears_and_bundles() {
    // 100,000.00 KES net @ 16% -> 16,000.00 VAT, 116,000.00 gross.
    let doc = kenyan_invoice_with_tax("INV-2026-KE-B16", "Steel reinforcement bar 12mm", "B", 1600, 10_000_000);
    let ubl = to_xml(&doc).unwrap();

    // The standard-rate subtotal + 16.00% percent must survive canonicalization.
    // (Canonicalization pins an inline `xmlns:` decl on each prefixed element,
    // so we match on the stable value-bearing close tags / attribute tails.)
    for needle in [
        "currencyID=\"KES\">16000.00</cbc:TaxAmount>",
        "currencyID=\"KES\">100000.00</cbc:TaxableAmount>",
        ">B</cbc:ID>",
        ">16.00</cbc:Percent>",
        "currencyID=\"KES\">116000.00</cbc:PayableAmount>",
    ] {
        assert!(ubl.contains(needle), "16% UBL missing {needle}");
    }

    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl.into_bytes())).unwrap();
    assert_eq!(receipt.status, EtimsStatus::Accepted);
    // §6.23.4: CU Invoice Number is CU-id + sequence; the mock tags it "KE-".
    assert!(receipt.cu_invoice_number.starts_with("KE-"));
}

/// KRA receipt label "C" — Zero rated goods (0%). Exports and Second-Schedule
/// supplies are taxed at 0%: a real, distinct category from exempt. The taxable
/// amount is non-zero, the tax amount is exactly zero, and the gross equals the
/// net (KRA TIS §40 prints `0% 0.00 0.00`).
#[test]
fn kenya_zero_rated_c_label_has_zero_tax_but_nonzero_base() {
    // 50,000.00 KES of zero-rated exports -> 0.00 VAT, gross == net.
    let doc = kenyan_invoice_with_tax("INV-2026-KE-C0", "Exported tea consignment", "C", 0, 5_000_000);
    let ubl = to_xml(&doc).unwrap();

    for needle in [
        ">C</cbc:ID>",
        "currencyID=\"KES\">50000.00</cbc:TaxableAmount>",
        // Both the subtotal tax and the header tax-total are zero.
        "currencyID=\"KES\">0.00</cbc:TaxAmount>",
        "currencyID=\"KES\">50000.00</cbc:PayableAmount>",
        // Zero-rated still produces tax-INCLUSIVE == tax-exclusive.
        "currencyID=\"KES\">50000.00</cbc:TaxInclusiveAmount>",
    ] {
        assert!(ubl.contains(needle), "zero-rated UBL missing {needle}");
    }
    // 0% rate is emitted as the percent, distinguishing it from exempt.
    assert!(ubl.contains(">0.00</cbc:Percent>"), "zero-rated must carry a 0.00 percent");

    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl.into_bytes())).unwrap();
    // Zero-rated is a valid clearable supply, not a refusal.
    assert_eq!(receipt.status, EtimsStatus::Accepted);
    assert!(receipt.reason.is_none());
}

/// KRA receipt label "A" — Goods exempted from VAT (printed "EX"). Exempt is
/// distinct from zero-rated: no output VAT and input VAT is not deductible.
/// The receipt's tax-components row reads `EX 1000.00 0.00` (KRA TIS §40/§41).
#[test]
fn kenya_exempt_a_label_carries_ex_base_and_zero_tax() {
    // 8,000.00 KES of exempt supply (e.g. unprocessed agricultural produce).
    let doc = kenyan_invoice_with_tax("INV-2026-KE-AEX", "Unprocessed maize (exempt)", "A", 0, 800_000);
    let ubl = to_xml(&doc).unwrap();

    for needle in [
        ">A</cbc:ID>",
        "currencyID=\"KES\">8000.00</cbc:TaxableAmount>",
        "currencyID=\"KES\">8000.00</cbc:PayableAmount>",
    ] {
        assert!(ubl.contains(needle), "exempt UBL missing {needle}");
    }
    // Exempt MUST NOT advertise the 16% standard label, nor a 16.00% percent.
    assert!(
        !ubl.contains(">B</cbc:ID>"),
        "exempt invoice must not carry the 16% B label"
    );
    assert!(
        !ubl.contains(">16.00</cbc:Percent>"),
        "exempt invoice must not carry the 16% rate"
    );

    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl.into_bytes())).unwrap();
    assert_eq!(receipt.status, EtimsStatus::Accepted);
}

/// A multi-line invoice mixing two KRA tax-type labels on one document — the
/// realistic case the KRA TIS §40 sample shows (a `B` cheese line beside an
/// `A-EX` bread line). Proves the bundled UBL carries BOTH per-line tax
/// categories and a header tax-total that is the SUM of only the taxed lines.
#[test]
fn kenya_mixed_b_and_a_lines_sum_only_taxable_vat() {
    // Line 1: 120,000.00 @ 16% (B) -> 19,200.00 VAT.
    // Line 2: 30,000.00 exempt (A) -> 0.00 VAT.
    // Header VAT total = 19,200.00 only; net = 150,000.00; gross = 169,200.00.
    let net_b = 12_000_000_i64;
    let net_a = 3_000_000_i64;
    let vat_b = vat_minor(net_b, 1600);
    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ke-mixed").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-KE-MIX1").unwrap(),
        currency: Iso4217Code::new("KES").unwrap(),
        supplier: kenyan_party("Acme Kenya Ltd", "A123456789Z", "Nairobi"),
        customer: kenyan_party("Beta Traders Ltd", "P051234567M", "Mombasa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Gouda cheese".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(net_b),
                line_extension_amount: amt(net_b),
                tax_category: Some("B".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Plain bread (exempt)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(net_a),
                line_extension_amount: amt(net_a),
                tax_category: Some("A".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "B".to_owned(),
                taxable_amount: amt(net_b),
                tax_amount: amt(vat_b),
                tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "A".to_owned(),
                taxable_amount: amt(net_a),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(net_b + net_a),
            tax_exclusive_amount: amt(net_b + net_a),
            tax_inclusive_amount: amt(net_b + net_a + vat_b),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(net_b + net_a + vat_b),
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

    let ubl = to_xml(&doc).unwrap();
    // Two invoice lines.
    assert_eq!(ubl.matches("</cac:InvoiceLine>").count(), 2, "expected two lines");
    // Both per-line tax categories present.
    assert!(ubl.contains(">B</cbc:ID>"), "missing B label");
    assert!(ubl.contains(">A</cbc:ID>"), "missing A label");
    // The header TaxTotal sums ONLY the taxed line: 19,200.00 (== the 16% line's
    // VAT), never 0 (which exempt-only would give) and never the net.
    assert!(
        ubl.contains("currencyID=\"KES\">19200.00</cbc:TaxAmount>"),
        "header VAT total must be the 16% line only (19200.00)"
    );
    assert!(
        !ubl.contains("currencyID=\"KES\">24000.00</cbc:TaxAmount>"),
        "VAT must NOT be charged on the exempt line (would be 24000.00 if taxed)"
    );
    // Gross = 169,200.00.
    assert!(ubl.contains("currencyID=\"KES\">169200.00</cbc:PayableAmount>"));

    let provider = MockEtimsProvider::default();
    let receipt = provider.submit_invoice(&submit_request(ubl.into_bytes())).unwrap();
    assert_eq!(receipt.status, EtimsStatus::Accepted);
}

/// CREDIT NOTE correction path (KRA transaction type "NC", spec §6.16/§14.1/§53).
/// A correction MUST reference the ORIGINAL CU INVOICE NO. of the sale it
/// cancels. We model that with `DocumentType::CreditNote` plus a
/// `DocumentReference` to the prior CU Invoice Number, serialize via UBL
/// (root `<CreditNote>`, type code 381), bundle it, and verify.
///
/// UBL constraint exercised: a UBL `CreditNote` MUST NOT carry a top-level
/// `DueDate`, so `due_date` is `None` (else the serializer rejects it).
#[test]
fn kenya_credit_note_references_original_cu_invoice_and_clears() {
    // First, the original sale clears and yields a CU Invoice Number.
    let provider = MockEtimsProvider::default();
    let original = kenyan_invoice_with_tax("INV-2026-KE-ORIG", "Gravel /t", "B", 1600, 9_000_000);
    let original_ubl = to_xml(&original).unwrap();
    let original_receipt = provider.submit_invoice(&submit_request(original_ubl.into_bytes())).unwrap();
    assert_eq!(original_receipt.status, EtimsStatus::Accepted);
    let original_cu = original_receipt.cu_invoice_number;

    // Now the credit note that cancels it, referencing the original CU number.
    let credit = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ke-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        // UBL CreditNote cannot carry a top-level DueDate.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-KE-0001").unwrap(),
        currency: Iso4217Code::new("KES").unwrap(),
        supplier: kenyan_party("Acme Kenya Ltd", "A123456789Z", "Nairobi"),
        customer: kenyan_party("Beta Traders Ltd", "P051234567M", "Mombasa"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Gravel /t (credit)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(9_000_000),
            line_extension_amount: amt(9_000_000),
            tax_category: Some("B".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "B".to_owned(),
            taxable_amount: amt(9_000_000),
            tax_amount: amt(vat_minor(9_000_000, 1600)),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(9_000_000),
            tax_exclusive_amount: amt(9_000_000),
            tax_inclusive_amount: amt(9_000_000 + vat_minor(9_000_000, 1600)),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(9_000_000 + vat_minor(9_000_000, 1600)),
        },
        attachments: Vec::new(),
        // KRA §6.16/§53: the correction MUST refer to the original CU invoice.
        references: vec![DocumentReference {
            kind: "original-cu-invoice".to_owned(),
            id: original_cu.clone(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
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

    let credit_ubl = to_xml(&credit).unwrap();
    // UBL CreditNote root + type code 381 (vs Invoice's 380).
    assert!(
        credit_ubl.contains(
            "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\""
        ),
        "credit note must serialize to a UBL CreditNote root"
    );
    assert!(credit_ubl.contains(">381</cbc:CreditNoteTypeCode>"));
    assert!(!credit_ubl.contains("</cbc:InvoiceTypeCode>"), "must not emit an Invoice type code");

    // The original CU Invoice Number is carried in the IR references and the
    // canonical artefact (the audit link back to the cancelled sale).
    let canonical = canonicalize_value(&credit.to_value().unwrap()).unwrap();
    assert!(
        canonical.contains(&original_cu),
        "credit note canonical JSON must reference the original CU invoice {original_cu}"
    );

    // It clears at eTIMS as its own transaction (the "NC" receipt).
    let credit_receipt = provider.submit_invoice(&submit_request(credit_ubl.clone().into_bytes())).unwrap();
    assert_eq!(credit_receipt.status, EtimsStatus::Accepted);
    // Distinct CU Invoice Number from the original sale (sequence advanced).
    assert_ne!(credit_receipt.cu_invoice_number, original_cu);

    // Bundle the credit note + its receipt and verify (exit 0 == report.ok).
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical.into_bytes());
    artefacts.insert("formats/ubl.xml".to_owned(), credit_ubl.into_bytes());
    artefacts.insert("receipt.json".to_owned(), serde_json::to_vec(&credit_receipt).unwrap());
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

/// An UNREGISTERED / malformed issuer PIN is refused before the wire — the
/// crate runs the same `validate_pin` shape gate the live impl runs, which
/// maps to KRA TIS §21.6.3 error "32 – wrong PIN in the TIS request data".
/// Several real-world malformations must each refuse with `BadPin`.
#[test]
fn kenya_refuses_each_malformed_pin_shape() {
    let provider = MockEtimsProvider::default();
    let doc = kenyan_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();

    // Every one of these violates the A123456789Z shape (letter + 9 digits +
    // letter) the KRA PIN must satisfy.
    for bad_pin in [
        "A12345678Z",   // 8 middle digits (too short)
        "A1234567890Z", // 10 middle digits (too long)
        "1123456789Z",  // first char not a letter
        "A1234567890",  // last char not a letter
        "A12X456789Z",  // non-digit in the middle
        "P 51234567M",  // space in the middle
    ] {
        let mut req = submit_request(ubl_bytes.clone());
        req.issuer_pin = bad_pin.to_owned();
        let err = provider.submit_invoice(&req).unwrap_err();
        assert!(
            matches!(err, EtimsError::BadPin(_)),
            "PIN {bad_pin:?} must refuse with BadPin (KRA TIS error 32), got {err:?}"
        );
    }

    // A well-formed buyer-style PIN (P-prefixed) still passes the shape gate.
    let mut ok_req = submit_request(ubl_bytes);
    ok_req.issuer_pin = "P051234567M".to_owned();
    assert_eq!(
        provider.submit_invoice(&ok_req).unwrap().status,
        EtimsStatus::Accepted
    );
}

/// Serialization determinism for the credit-note + zero-rated paths: the same
/// IR document must serialize to byte-identical UBL across repeated calls, so
/// the CU Invoice Number is the ONLY thing that changes between submissions.
#[test]
fn kenya_serialization_is_deterministic_across_categories() {
    for doc in [
        kenyan_invoice_with_tax("INV-DET-B", "Bar 12mm", "B", 1600, 10_000_000),
        kenyan_invoice_with_tax("INV-DET-C", "Exported tea", "C", 0, 5_000_000),
        kenyan_invoice_with_tax("INV-DET-A", "Maize (exempt)", "A", 0, 800_000),
    ] {
        let first = to_xml(&doc).unwrap();
        let second = to_xml(&doc).unwrap();
        assert_eq!(first, second, "UBL serialization must be byte-stable for {}", doc.document_number.as_str());

        // The canonical IR JSON is likewise byte-stable.
        let c1 = canonicalize_value(&doc.to_value().unwrap()).unwrap();
        let c2 = canonicalize_value(&doc.to_value().unwrap()).unwrap();
        assert_eq!(c1, c2, "canonical JSON must be byte-stable");
    }
}
