// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Nigeria FIRS format-variation coverage — deepens the offline lifecycle with
//! genuinely country-specific document shapes and the authority-rejection path.
//!
//! Authority: the Federal Inland Revenue Service (FIRS) National e-Invoicing
//! System / Merchant-Buyer Solution (FIRSMBS). Nigeria adopted the **Peppol BIS
//! Billing 3.0 / OASIS UBL 2.1** data format as the basis for its e-invoice
//! schema, so InvoiceKit serializes Nigerian documents through the UBL family
//! path. Cleared documents receive an Invoice Reference Number (IRN) plus a
//! Cryptographic Stamp Identifier (CSID).
//!
//! External references grounding the country-specific assertions below:
//! - FIRS National e-Invoicing System portal + system-integrator docs:
//!   <https://einvoice.firs.gov.ng/> and
//!   <https://einvoice.firs.gov.ng/docs/system-integrator/generate-irn>
//! - Peppol BIS Billing 3.0 tax-category code list (UNCL5305 subset) — the
//!   `cac:TaxCategory/cbc:ID` values used below (`S`, `Z`, `E`, `AE`):
//!   <https://docs.peppol.eu/poacc/billing/3.0/codelist/UNCL5305/>
//! - EY tax alert, "Nigeria's Federal Inland Revenue Service rolls out
//!   e-Invoicing platform" (UBL/BIS 3.0 + IRN + CSID confirmation).
//!
//! These scenarios ADD to (never weaken) `tests/e2e_offline_lifecycle.rs`.
//! Fixtures are hand-built / synthetic — no copyrighted regulator files are
//! vendored. Goldens are hand-rolled (no `insta`/`pretty_assertions`, which
//! would mutate `Cargo.lock`).
//!
//! Scenarios:
//! 1. Credit note (corrective document) — UBL `<CreditNote>` with
//!    `cac:CreditNoteLine`/`cbc:CreditedQuantity`, links the original invoice,
//!    clears FIRS, and produces a verifiable evidence bundle.
//! 2. Multi-line invoice mixing standard-rated (`S`, 7.5%) and zero-rated (`Z`,
//!    exports at 0%) lines — both Nigerian VAT treatments land in the UBL.
//! 3. Tax-exempt supply (`E`) — exempt category + zero tax round-trips to UBL.
//! 4. Reverse-charge (`AE`) on imported services — the reverse-charge category
//!    lands in the UBL tax breakdown.
//! 5. Authority REJECTION path — a `FirsStatus::Rejected` receipt (NOT an `Err`)
//!    still bundles and verifies, honoring the audit-trail contract.
//! 6. Invalid-identifier rejection — malformed FIRS TINs are refused as `Err`
//!    before the wire (table-driven over real TIN shapes).
//! 7. Serialization determinism for the corrective-document path.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_ng_firs::{
    validate_tin, FirsEnvironment, FirsError, FirsProvider, FirsStatus, FirsSubmitEnvelope,
    FirsSubmitRequest, MockFirsProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const RECORDED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_ng_fmt";
const TRACE: &str = "trace_ng_fmt";
const ISSUER_TIN: &str = "12345678-9012";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn rate(percent_minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(percent_minor, 2))
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

fn supplier() -> Party {
    nigerian_party("Acme Nigeria Ltd", "NG12345678901", "Lagos", "LA", "100001")
}

fn customer() -> Party {
    nigerian_party("Beta Services Plc", "NG98765432109", "Abuja", "FC", "900001")
}

fn submit_request(payload: Vec<u8>) -> FirsSubmitRequest {
    FirsSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: FirsEnvironment::Sandbox,
        issuer_tin: ISSUER_TIN.to_owned(),
        payload,
    }
}

/// Pack the Nigerian artefact set (canonical IR + national UBL + FIRS receipt)
/// into a `.ikb` exactly like the lifecycle test does, so every scenario proves
/// it bundles AND verifies — not just that the receipt looks right.
fn bundle_and_verify(doc: &CommercialDocument, ubl_bytes: &[u8], envelope: &FirsSubmitEnvelope) {
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
    let ikb = pack(&bundle).unwrap();
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "FIRS evidence bundle must verify (exit 0 == report.ok)");
}

// ---------------------------------------------------------------------------
// Scenario 1: credit note (corrective document).
// ---------------------------------------------------------------------------

/// A FIRS corrective document. Nigeria's UBL family emits credit notes as a
/// root `<CreditNote>` with `cac:CreditNoteLine`/`cbc:CreditedQuantity`. The
/// UBL serializer forbids a top-level `cbc:DueDate` on a `CreditNote`, so
/// `due_date` MUST be `None`. The credit note carries a reference back to the
/// original cleared invoice, which is the legal basis a FIRS correction needs.
fn nigerian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ng-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // CreditNote cannot carry a top-level DueDate in UBL.
        due_date: None,
        document_number: DocumentNumber::new("CN-2026-NG-0001").unwrap(),
        currency: Iso4217Code::new("NGN").unwrap(),
        supplier: supplier(),
        customer: customer(),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Credit: over-billed consulting hours".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            // Nigeria's standard VAT rate is 7.5%.
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(375),
            tax_rate: Some(rate(750)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5375),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5375),
        },
        attachments: Vec::new(),
        // Correct the original cleared invoice. The reference is the legal
        // basis a FIRS credit note must carry.
        references: vec![DocumentReference {
            kind: "credit-note-original-invoice".to_owned(),
            id: "INV-2026-NG-0001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("fmt".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn nigeria_credit_note_serializes_and_clears_firs() {
    let doc = nigerian_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // A corrective document is a UBL CreditNote, not an Invoice. These markers
    // are what distinguish the FIRS corrective shape from a fresh invoice.
    assert!(ubl.contains("<CreditNote"), "corrective doc must be a UBL CreditNote root");
    assert!(
        ubl.contains("cac:CreditNoteLine"),
        "CreditNote must use cac:CreditNoteLine, not cac:InvoiceLine"
    );
    assert!(
        ubl.contains("cbc:CreditedQuantity"),
        "CreditNote line quantity is cbc:CreditedQuantity"
    );
    assert!(
        !ubl.contains("<cac:InvoiceLine"),
        "a CreditNote must not emit InvoiceLine elements"
    );
    // No DueDate on a credit note.
    assert!(
        !ubl.contains("<cbc:DueDate"),
        "UBL forbids a top-level DueDate on a CreditNote"
    );
    assert!(ubl.contains(">NGN</cbc:DocumentCurrencyCode>"), "currency must be NGN");

    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(ubl.clone().into_bytes()))
        .unwrap();
    // FIRS clears the correction and assigns it its own IRN.
    assert_eq!(envelope.status, FirsStatus::Accepted);
    assert!(envelope.irn.starts_with("NG-"), "corrective doc gets a Nigeria-tagged IRN");

    bundle_and_verify(&doc, ubl.as_bytes(), &envelope);
}

#[test]
fn nigeria_credit_note_lifecycle_is_byte_deterministic() {
    // The corrective-document path must be as byte-stable as the invoice path:
    // FIRS clearance and downstream audit replay depend on a stable artefact.
    let doc = nigerian_credit_note();
    let a = to_xml(&doc).unwrap();
    let b = to_xml(&doc).unwrap();
    assert_eq!(a, b, "credit-note UBL serialization must be deterministic");

    let canon_a = canonicalize_value(&doc.to_value().unwrap()).unwrap();
    let canon_b = canonicalize_value(&doc.to_value().unwrap()).unwrap();
    assert_eq!(canon_a, canon_b, "credit-note canonical IR must be deterministic");
}

// ---------------------------------------------------------------------------
// Scenario 2: multi-line invoice mixing standard-rated and zero-rated supplies.
// ---------------------------------------------------------------------------

/// Nigeria zero-rates exports (UNCL5305 `Z`, levied at 0%) but they must still
/// be e-invoiced through FIRS, alongside domestic standard-rated (`S`, 7.5%)
/// lines. This document carries one of each so the UBL tax breakdown shows both
/// Nigerian VAT treatments — a real multi-line FIRS invoice shape.
fn nigerian_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-ng-mix-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-NG-0042").unwrap(),
        currency: Iso4217Code::new("NGN").unwrap(),
        supplier: supplier(),
        customer: customer(),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Domestic standard-rated line (7.5%).
            DocumentLine {
                id: "1".to_owned(),
                description: "Domestic software consulting".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            // Exported service: zero-rated under Nigerian VAT.
            DocumentLine {
                id: "2".to_owned(),
                description: "Exported SaaS license (zero-rated)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(750),
                tax_rate: Some(rate(750)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(20000),
                tax_amount: amt(0),
                tax_rate: Some(rate(0)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            // Only the standard-rated line carries 750 (7.50) tax.
            tax_inclusive_amount: amt(30750),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(30750),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("fmt".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn nigeria_mixed_rate_invoice_carries_both_vat_treatments() {
    let doc = nigerian_mixed_rate_invoice();
    let ubl = to_xml(&doc).unwrap();

    // Two invoice lines.
    assert_eq!(
        ubl.matches("<cac:InvoiceLine").count(),
        2,
        "the mixed-rate invoice must serialize two InvoiceLine elements"
    );
    // Standard-rated classification on the domestic line + zero-rated on the
    // export line. Both are real UNCL5305 codes Nigeria/Peppol BIS uses. Match
    // on the element text only (`>CODE</cbc:ID>`): canonicalization attaches an
    // inline `xmlns:cbc=...` declaration to each opening tag, so the open tag is
    // never the bare `<cbc:ID>`.
    assert!(
        ubl.contains(">S</cbc:ID>"),
        "standard-rated (S) tax category must appear in the UBL"
    );
    assert!(
        ubl.contains(">Z</cbc:ID>"),
        "zero-rated (Z) tax category must appear in the UBL"
    );
    // The 7.5% standard rate must be present in the tax breakdown.
    assert!(
        ubl.contains(">7.50</cbc:Percent>"),
        "Nigeria's 7.5% standard VAT rate must be emitted as a Percent"
    );
    // Zero-rated line is levied at 0%.
    assert!(
        ubl.contains(">0.00</cbc:Percent>"),
        "the zero-rated line must be emitted at a 0.00 Percent"
    );

    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(ubl.clone().into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, FirsStatus::Accepted);
    bundle_and_verify(&doc, ubl.as_bytes(), &envelope);
}

// ---------------------------------------------------------------------------
// Scenarios 3 & 4: exempt and reverse-charge supplies (single-category docs).
// ---------------------------------------------------------------------------

/// Build a single-line, single-category Nigerian invoice for a given UNCL5305
/// tax category with no tax charged. Used for exempt (`E`) and reverse-charge
/// (`AE`) supplies, both of which carry zero tax on the supplier's invoice.
fn nigerian_no_tax_invoice(category: &str, number: &str, description: &str) -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new(format!("doc-ng-{}", category.to_lowercase())).unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new(number).unwrap(),
        currency: Iso4217Code::new("NGN").unwrap(),
        supplier: supplier(),
        customer: customer(),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: description.to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(40000),
            line_extension_amount: amt(40000),
            tax_category: Some(category.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: category.to_owned(),
            taxable_amount: amt(40000),
            tax_amount: amt(0),
            tax_rate: Some(rate(0)),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(40000),
            tax_exclusive_amount: amt(40000),
            tax_inclusive_amount: amt(40000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(40000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("fmt".to_owned()),
        },
    })
    .unwrap()
}

#[test]
fn nigeria_exempt_supply_serializes_with_zero_tax() {
    // Exempt supplies (UNCL5305 `E`) carry no VAT. Where FIRS requires an
    // e-invoice for an exempt B2B supply, the exempt category and zero tax must
    // round-trip into the national UBL.
    let doc = nigerian_no_tax_invoice("E", "INV-2026-NG-EXEMPT", "VAT-exempt medical supplies");
    let ubl = to_xml(&doc).unwrap();

    // Match element text only — canonicalization attaches an inline
    // `xmlns:cbc=...` to each opening tag.
    assert!(
        ubl.contains(">E</cbc:ID>"),
        "exempt (E) tax category must appear in the UBL tax breakdown"
    );
    assert!(
        ubl.contains("currencyID=\"NGN\">0.00</cbc:TaxAmount>"),
        "an exempt supply must carry a 0.00 NGN tax amount"
    );

    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(ubl.clone().into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, FirsStatus::Accepted);
    bundle_and_verify(&doc, ubl.as_bytes(), &envelope);
}

#[test]
fn nigeria_reverse_charge_supply_serializes() {
    // Imported / non-resident-supplier services shift the VAT liability to the
    // recipient (UNCL5305 `AE`, VAT reverse charge). The supplier's invoice
    // charges no VAT but must flag the reverse-charge category.
    let doc = nigerian_no_tax_invoice(
        "AE",
        "INV-2026-NG-REVCHG",
        "Imported cloud hosting (reverse charge)",
    );
    let ubl = to_xml(&doc).unwrap();

    // Match element text only — canonicalization attaches an inline
    // `xmlns:cbc=...` to each opening tag.
    assert!(
        ubl.contains(">AE</cbc:ID>"),
        "reverse-charge (AE) tax category must appear in the UBL"
    );
    // Reverse charge is still levied from the invoicee at the standard rate, so
    // the supplier invoice itself shows no VAT charged.
    assert!(
        ubl.contains("currencyID=\"NGN\">0.00</cbc:TaxAmount>"),
        "a reverse-charge supply must carry a 0.00 NGN tax amount on the supplier invoice"
    );

    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let envelope = provider
        .submit_invoice(&submit_request(ubl.clone().into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, FirsStatus::Accepted);
    bundle_and_verify(&doc, ubl.as_bytes(), &envelope);
}

// ---------------------------------------------------------------------------
// Scenario 5: authority REJECTION path (receipt status, NOT an Err).
// ---------------------------------------------------------------------------

/// A deterministic FIRS provider that mimics the authority *refusing* a
/// document. FIRS runs server-side validation against its mandatory-field set
/// before issuing an IRN; a refused document comes back with a rejected status
/// and a reason, NOT a transport error. The crate's `FirsStatus::Rejected` +
/// `reason` fields exist precisely for this verdict. Per the adapter contract,
/// rejection is surfaced inside the envelope (so the engine persists it in the
/// audit trail), never raised as `Err`. This provider implements the crate's
/// public `FirsProvider` trait in-test, without touching the crate.
struct RejectingFirsProvider {
    recorded_at: String,
    reason: String,
}

impl FirsProvider for RejectingFirsProvider {
    fn submit_invoice(
        &self,
        request: &FirsSubmitRequest,
    ) -> Result<FirsSubmitEnvelope, FirsError> {
        // Run the same pre-wire shape validators the real adapter runs: a
        // malformed TIN or empty payload is still an `Err`, never a verdict.
        validate_tin(&request.issuer_tin)?;
        if request.payload.is_empty() {
            return Err(FirsError::BadPayload("payload is empty".to_owned()));
        }
        // The authority accepted the request shape but refused the document.
        Ok(FirsSubmitEnvelope {
            // FIRS does not mint an IRN for a refused document.
            irn: String::new(),
            status: FirsStatus::Rejected,
            recorded_at: self.recorded_at.clone(),
            reason: Some(self.reason.clone()),
        })
    }
}

#[test]
fn nigeria_authority_rejection_still_bundles_and_verifies() {
    let doc = nigerian_mixed_rate_invoice();
    let ubl = to_xml(&doc).unwrap();

    let provider = RejectingFirsProvider {
        recorded_at: RECORDED_AT.to_owned(),
        reason: "issuer TIN not registered on the FIRS taxpayer roll".to_owned(),
    };
    // A FIRS refusal is an Ok(envelope-with-Rejected), NOT an Err — this is the
    // load-bearing audit-trail contract for every report-* adapter.
    let envelope = provider
        .submit_invoice(&submit_request(ubl.clone().into_bytes()))
        .expect("a FIRS refusal must surface as Ok(Rejected), never as Err");

    assert_eq!(envelope.status, FirsStatus::Rejected);
    // A refused document carries a human-readable reason and no IRN.
    assert!(
        envelope.reason.is_some(),
        "a rejected verdict must carry a reason for the audit trail"
    );
    assert!(
        envelope.irn.is_empty(),
        "FIRS does not mint an IRN for a refused document"
    );

    // The rejection still persists in a verifiable evidence bundle.
    bundle_and_verify(&doc, ubl.as_bytes(), &envelope);
}

#[test]
fn nigeria_rejected_receipt_round_trips_through_serde() {
    // The rejection receipt must serialize and deserialize losslessly so the
    // audit trail can replay the FIRS verdict offline.
    let envelope = FirsSubmitEnvelope {
        irn: String::new(),
        status: FirsStatus::Rejected,
        recorded_at: RECORDED_AT.to_owned(),
        reason: Some("missing mandatory buyer TIN".to_owned()),
    };
    let json = serde_json::to_string(&envelope).unwrap();
    // Status serializes kebab-case per the crate's serde contract.
    assert!(json.contains("\"status\":\"rejected\""), "status must be kebab-case `rejected`");
    let parsed: FirsSubmitEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, envelope);
}

// ---------------------------------------------------------------------------
// Scenario 6: invalid-identifier rejection (Err before the wire).
// ---------------------------------------------------------------------------

#[test]
fn nigeria_invalid_tins_are_refused_before_the_wire() {
    // The FIRS TIN is 12 ASCII digits (the crate accepts an optional hyphen
    // after the 8th, mirroring the printed FIRS TIN form `XXXXXXXX-YYYY`). Any
    // other shape must be refused as `Err(BadTin)` BEFORE the document reaches
    // the wire — a malformed identifier never produces a verdict or a bundle.
    let provider = MockFirsProvider::with_fixed_recorded_at(RECORDED_AT);
    let ubl_bytes = to_xml(&nigerian_mixed_rate_invoice()).unwrap().into_bytes();

    // Well-formed FIRS TINs (with and without the hyphen) clear validation.
    for good in ["123456789012", "12345678-9012"] {
        assert!(validate_tin(good).is_ok(), "{good:?} is a valid FIRS TIN shape");
    }

    // Malformed TINs FIRS would never mint — each must be refused.
    for (bad, why) in [
        ("1234567890", "only 10 digits"),
        ("1234567890123", "13 digits — too long"),
        ("12345678901A", "trailing non-digit"),
        ("ABCDEFGHIJKL", "12 letters, no digits"),
        ("", "empty TIN"),
        ("  12345678 9012", "embedded spaces"),
    ] {
        assert!(
            validate_tin(bad).is_err(),
            "TIN {bad:?} ({why}) must fail the FIRS TIN shape check"
        );
        let mut req = submit_request(ubl_bytes.clone());
        req.issuer_tin = bad.to_owned();
        let err = provider.submit_invoice(&req).unwrap_err();
        assert!(
            matches!(err, FirsError::BadTin(_)),
            "a malformed TIN must surface as FirsError::BadTin before the wire, got {err:?}"
        );
    }

    // The well-formed request still clears, proving the refusals above are
    // identifier-specific and not a blanket failure.
    assert_eq!(
        provider
            .submit_invoice(&submit_request(ubl_bytes))
            .unwrap()
            .status,
        FirsStatus::Accepted,
        "a well-formed TIN must still clear FIRS"
    );
}
