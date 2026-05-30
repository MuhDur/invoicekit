// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Egypt ETA offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Egypt and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("EG")` and
//!    a sensible ISO currency (`EGP`, the Egyptian Pound)
//! 2. serialize -> EN 16931 / UBL 2.1 bytes via `invoicekit_format_ubl::to_xml`
//!    (the ETA crate exposes no own serializer; it consumes signed payload
//!    bytes, so the family UBL path is the honest upstream)
//! 3. submit those bytes to the crate's existing `MockEtaProvider` and assert
//!    the ETA-specific receipt fields (UUID prefix, Long ID, 64-char content
//!    hash, status)
//! 4. assemble an `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json), `manifest_for(... pinned created_at ...)`, `pack`, then
//!    `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the local pre-wire validators reject a bad national id and an
//!    empty payload with `Err`
//!
//! Deepened country-specific coverage (added on top of the base lifecycle,
//! grounded in the Egyptian Tax Authority — ETA — SDK):
//!
//!   * credit note as an ETA corrective document (`documentType` `"c"`),
//!     carrying the mandatory reference to the amended invoice's UUID;
//!   * a multi-line standard-rated B2B invoice (ETA VAT subtype `V009`,
//!     "General Item sales");
//!   * a VAT-exempt supply (ETA tax type `T1`, subtype `V003`, "Exempted
//!     good or service") with zero output tax;
//!   * an export invoice (ETA VAT subtype `V001`, "Export") zero-rated for
//!     output VAT;
//!   * the authority `Invalid` clearance verdict — a per-document STATUS, not
//!     an `Err` — surfaced inside the receipt so the evidence bundle still
//!     packs and verifies;
//!   * a B2C e-Receipt issued against a 14-digit national id;
//!   * canonical-serialization determinism across document classes.
//!
//! Note on the authority-`Invalid` verdict: the base suite predates the
//! `MockEtaProvider::with_forced_verdict` knob and used pre-wire shape
//! refusals as its only refusal coverage. The knob (added in `lib.rs`)
//! now lets the offline suite drive the post-wire `Valid` / `Invalid`
//! verdicts that ETA records after its eight server-side validators run.
//! Pre-wire shape refusals (`EtaError::BadId` / `EtaError::BadPayload`)
//! remain a distinct, complementary path that returns `Err`.
//!
//! ETA references cited in the scenarios below:
//!   * tax types & VAT subtypes (T1, V001/V003/V004/V009):
//!     <https://sdk.invoicing.eta.gov.eg/codes/tax-types/>
//!   * document types ("i"/"c"/"d") & corrective references:
//!     <https://sdk.invoicing.eta.gov.eg/documents/credit-note-v1-0/>
//!   * the eight document validators & rejection model:
//!     <https://sdk.invoicing.eta.gov.eg/document-validation-rules/>
//!
//! All fixtures are license-safe hand-built synthetic data — no regulator
//! files are vendored. Goldens are hand-rolled (no `insta` /
//! `pretty_assertions`, which would mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_eg_eta::{
    EtaDocumentKind, EtaEnvironment, EtaError, EtaProvider, EtaStatus, EtaSubmitEnvelope,
    EtaSubmitRequest, MockEtaProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_eg_e2e";
const TRACE: &str = "trace_eg_e2e";
/// Egyptian tax registration number — 9 ASCII digits (the ETA shape).
const ISSUER_TAX_ID: &str = "100200300";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

/// The shared `DocumentMeta` every fixture carries — identical tenant, trace,
/// and source-system across all document classes.
fn e2e_meta() -> DocumentMeta {
    DocumentMeta {
        tenant_id: TENANT.to_owned(),
        trace_id: TRACE.to_owned(),
        source_system: Some("e2e".to_owned()),
    }
}

fn egyptian_party(name: &str, vat: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "eta-tin".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["12 Tahrir Square".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "11511".to_owned(),
            country: CountryCode::new("EG").unwrap(),
        },
        contact: None,
    }
}

/// Step 1: a valid Egyptian B2B invoice in `EGP`.
fn egyptian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-EG-0001").unwrap(),
        currency: Iso4217Code::new("EGP").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", "100200300", "Cairo"),
        customer: egyptian_party("Delta Imports", "400500600", "Alexandria"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulting services".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(14_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(114_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(114_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: e2e_meta(),
    })
    .unwrap()
}

fn submit_request(payload: Vec<u8>) -> EtaSubmitRequest {
    submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, payload)
}

/// Build a submit request for an arbitrary ETA document kind / issuer id.
///
/// ETA distinguishes document classes by `documentType` — `"i"` invoice,
/// `"c"` credit note, `"d"` debit note, plus B2C e-Receipts — per the SDK
/// (<https://sdk.invoicing.eta.gov.eg/documents/credit-note-v1-0/>).
fn submit_request_for(kind: EtaDocumentKind, id: &str, payload: Vec<u8>) -> EtaSubmitRequest {
    EtaSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: EtaEnvironment::Preprod,
        kind,
        issuer_tax_or_national_id: id.to_owned(),
        payload,
    }
}

/// Generic offline lifecycle: build-bytes -> serialize (UBL upstream) ->
/// submit to a caller-supplied mock ETA -> assemble + pack an `.ikb` bundle.
///
/// Returns the packed bundle and the ETA receipt. Used by every scenario so
/// each document class travels the identical evidence path.
fn bundle_lifecycle(
    doc: &CommercialDocument,
    request: &EtaSubmitRequest,
    provider: &MockEtaProvider,
) -> (Vec<u8>, EtaSubmitEnvelope) {
    let ubl: Vec<u8> = to_xml(doc).unwrap().into_bytes();
    let mut req = request.clone();
    req.payload.clone_from(&ubl);
    let envelope = provider.submit(&req).unwrap();

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, envelope)
}

/// Steps 1-4: build -> serialize -> submit to the mock ETA -> evidence bundle.
fn run_lifecycle() -> (Vec<u8>, EtaSubmitEnvelope) {
    let doc = egyptian_invoice();
    let request = submit_request(Vec::new());
    let provider = MockEtaProvider::new();
    bundle_lifecycle(&doc, &request, &provider)
}

#[test]
fn egypt_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // ETA-specific receipt assertions: UUID prefix, Long ID prefix, 64-char
    // content hash, and the cleared status.
    assert_eq!(envelope.status, EtaStatus::Submitted);
    assert!(
        envelope.uuid.starts_with("EG-"),
        "ETA UUID must carry the EG- prefix, got {:?}",
        envelope.uuid
    );
    assert!(
        envelope.long_id.starts_with("ETA-LONG-"),
        "ETA Long ID must carry the ETA-LONG- prefix, got {:?}",
        envelope.long_id
    );
    assert_eq!(
        envelope.content_hash_hex.len(),
        64,
        "ETA content hash must be a 64-char SHA-256 hex string"
    );
    assert_eq!(envelope.submitted_at, "2026-01-01T00:00:00Z");
    assert!(envelope.reason.is_none());

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn egypt_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn egypt_refuses_bad_national_id_before_the_wire() {
    // The mock runs the SAME pre-wire validators the real impl runs. A bad
    // tax/national id is an Err (shape refusal), not a cleared receipt.
    let provider = MockEtaProvider::new();
    let mut req = submit_request(to_xml(&egyptian_invoice()).unwrap().into_bytes());
    req.issuer_tax_or_national_id = "NOT-DIGITS".to_owned();
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, EtaError::BadId(_)),
        "a malformed national id must refuse with EtaError::BadId, got {err:?}"
    );
}

#[test]
fn egypt_refuses_empty_payload_before_the_wire() {
    // An empty payload is a pre-wire refusal that returns `Err` BEFORE any
    // bytes reach ETA. This is distinct from the post-wire authority `Invalid`
    // clearance verdict (exercised separately in
    // `egypt_authority_invalid_verdict_*`), which is a receipt STATUS, not an
    // `Err`.
    let provider = MockEtaProvider::new();
    let err = provider.submit(&submit_request(Vec::new())).unwrap_err();
    assert!(
        matches!(err, EtaError::BadPayload(_)),
        "an empty payload must refuse with EtaError::BadPayload, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios.
//
// Every fixture below is hand-built synthetic data. Identifiers, tax subtype
// codes (T1 / V001 / V003 / V009) and document classes ("i"/"c"/"d") are
// grounded in the ETA SDK references cited in the module doc-comment.
// ---------------------------------------------------------------------------

/// A 14-digit Egyptian national id — the ETA shape for a B2C e-Receipt issuer
/// (a natural person), distinct from the 9-digit company tax registration
/// number. Per the crate's `validate_tax_or_national_id` and the ETA SDK
/// (national-id validator).
const ISSUER_NATIONAL_ID: &str = "29001011234567";

/// A second registered company, used as the issuer of the credit note's
/// referenced original invoice (9-digit tax registration number).
const RECEIVER_TAX_ID: &str = "400500600";

/// Multi-line standard-rated B2B invoice. Two lines taxed at the 14% Egyptian
/// VAT rate (ETA tax type `T1`, VAT subtype `V009` "General Item sales").
///
/// Source: <https://sdk.invoicing.eta.gov.eg/codes/tax-types/>.
fn egyptian_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-multiline-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-EG-0002").unwrap(),
        currency: Iso4217Code::new("EGP").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", ISSUER_TAX_ID, "Cairo"),
        customer: egyptian_party("Delta Imports", RECEIVER_TAX_ID, "Alexandria"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Office chairs".to_owned(),
                quantity: DecimalValue::new(Decimal::from(10)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(20_000),
                line_extension_amount: amt(200_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Desks".to_owned(),
                quantity: DecimalValue::new(Decimal::from(5)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(60_000),
                line_extension_amount: amt(300_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        // 14% Egyptian standard VAT on 5,000.00 EGP base = 700.00 EGP.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(500_000),
            tax_amount: amt(70_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(500_000),
            tax_exclusive_amount: amt(500_000),
            tax_inclusive_amount: amt(570_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(570_000),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: e2e_meta(),
    })
    .unwrap()
}

/// Credit note correcting `INV-2026-EG-0001`. ETA credit notes (`documentType`
/// `"c"`) MUST reference the original document by its registered UUID and may
/// not add new lines or exceed the referenced invoice amounts.
///
/// Source: <https://sdk.invoicing.eta.gov.eg/documents/credit-note-v1-0/>.
/// Note: UBL 2.1 `CreditNote` has no top-level due date, so it is omitted.
fn egyptian_credit_note(original_uuid: &str) -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-credit-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("CN-2026-EG-0001").unwrap(),
        currency: Iso4217Code::new("EGP").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", ISSUER_TAX_ID, "Cairo"),
        customer: egyptian_party("Delta Imports", RECEIVER_TAX_ID, "Alexandria"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        // Partial credit: one of the two consulting units returned (500.00 EGP).
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulting services (returned)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(50_000),
            line_extension_amount: amt(50_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(50_000),
            tax_amount: amt(7_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(50_000),
            tax_exclusive_amount: amt(50_000),
            tax_inclusive_amount: amt(57_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(57_000),
        },
        attachments: Vec::new(),
        // The mandatory ETA corrective reference to the original invoice UUID.
        references: vec![DocumentReference {
            kind: "eta-original-uuid".to_owned(),
            id: original_uuid.to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: e2e_meta(),
    })
    .unwrap()
}

/// VAT-exempt supply: ETA tax type `T1`, subtype `V003` "Exempted good or
/// service". Output VAT is zero and the payable equals the taxable base.
///
/// Source: <https://sdk.invoicing.eta.gov.eg/codes/tax-types/>.
fn egyptian_exempt_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-exempt-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-EG-0003").unwrap(),
        currency: Iso4217Code::new("EGP").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", ISSUER_TAX_ID, "Cairo"),
        customer: egyptian_party("Cairo Clinic", RECEIVER_TAX_ID, "Giza"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        // Tax category "E" (exempt) — zero output VAT.
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exempt medical supplies".to_owned(),
            quantity: DecimalValue::new(Decimal::from(4)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(25_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("E".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "E".to_owned(),
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
        meta: e2e_meta(),
    })
    .unwrap()
}

/// Export invoice: ETA VAT subtype `V001` "Export". Zero-rated for output VAT;
/// the customer is a foreign company, so the receiver country is not EG.
///
/// Source: <https://sdk.invoicing.eta.gov.eg/codes/tax-types/>.
fn egyptian_export_invoice() -> CommercialDocument {
    let foreign_customer = Party {
        id: Some("globex-gmbh".to_owned()),
        name: "Globex GmbH".to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: "DE811234567".to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Hauptstrasse 5".to_owned()],
            city: "Munich".to_owned(),
            subdivision: None,
            postal_code: "80331".to_owned(),
            country: CountryCode::new("DE").unwrap(),
        },
        contact: None,
    };
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-eg-export-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("EXP-2026-EG-0001").unwrap(),
        currency: Iso4217Code::new("USD").unwrap(),
        supplier: egyptian_party("Nile Trading LLC", ISSUER_TAX_ID, "Cairo"),
        customer: foreign_customer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Exported cotton textiles".to_owned(),
            quantity: DecimalValue::new(Decimal::from(100)),
            unit_code: Some("KGM".to_owned()),
            unit_price: amt(1_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("Z".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        // Zero-rated export: tax base recorded, output tax zero.
        tax_summary: vec![TaxCategorySummary {
            category_code: "Z".to_owned(),
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
        meta: e2e_meta(),
    })
    .unwrap()
}

#[test]
fn egypt_credit_note_corrective_references_original_uuid() {
    // First clear the original invoice to obtain its ETA UUID.
    let original = egyptian_invoice();
    let provider = MockEtaProvider::new();
    let (_, original_receipt) = bundle_lifecycle(
        &original,
        &submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    assert!(original_receipt.uuid.starts_with("EG-"));

    // Now issue a credit note that references that UUID. ETA classifies it as
    // documentType "c"; the corrective reference is mandatory.
    let credit = egyptian_credit_note(&original_receipt.uuid);
    assert_eq!(credit.document_type, DocumentType::CreditNote);
    assert_eq!(credit.references.len(), 1);
    assert_eq!(credit.references[0].id, original_receipt.uuid);

    // The UBL serializer emits a CreditNote root with code 381 (UNCL1001 for
    // a commercial credit note) — the family upstream the ETA crate consumes.
    let xml = to_xml(&credit).unwrap();
    assert!(
        xml.contains("<CreditNote"),
        "credit note must serialize to a UBL CreditNote root, got: {xml}"
    );
    assert!(
        xml.contains(">381</cbc:CreditNoteTypeCode>"),
        "UBL credit note must carry CreditNoteTypeCode 381, got: {xml}"
    );
    assert!(
        xml.contains("<cac:CreditNoteLine"),
        "credit note lines must use cac:CreditNoteLine, not cac:InvoiceLine"
    );
    assert!(
        !xml.contains("<cac:InvoiceLine"),
        "a credit note must not emit any InvoiceLine elements"
    );

    // The credit clears its own evidence path; a fresh serial is assigned.
    let (ikb, credit_receipt) = bundle_lifecycle(
        &credit,
        &submit_request_for(EtaDocumentKind::CreditNote, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    assert_ne!(
        credit_receipt.uuid, original_receipt.uuid,
        "the credit note must get its own ETA UUID, distinct from the invoice"
    );
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "credit-note evidence bundle must verify");
}

#[test]
fn egypt_multiline_standard_rated_invoice_carries_both_lines() {
    let doc = egyptian_multiline_invoice();
    let xml = to_xml(&doc).unwrap();

    // Both invoice lines must serialize, each as a UBL InvoiceLine (code 380).
    // (The serializer hoists xmlns:cbc onto each element, so we anchor on the
    // closing tag rather than an exact open tag.)
    assert!(xml.contains(">380</cbc:InvoiceTypeCode>"));
    assert_eq!(
        xml.matches("<cac:InvoiceLine").count(),
        2,
        "multi-line invoice must serialize exactly two InvoiceLine elements"
    );
    assert!(xml.contains("Office chairs") && xml.contains("Desks"));

    // 14% Egyptian standard VAT on 5,000.00 EGP = 700.00 EGP; total 5,700.00.
    assert!(
        xml.contains(r#"currencyID="EGP">5700.00</cbc:TaxInclusiveAmount>"#),
        "standard-rated total must be 5,700.00 EGP, got: {xml}"
    );

    let provider = MockEtaProvider::new();
    let (ikb, receipt) = bundle_lifecycle(
        &doc,
        &submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    assert_eq!(receipt.status, EtaStatus::Submitted);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "multi-line evidence bundle must verify");
}

#[test]
fn egypt_vat_exempt_supply_has_zero_output_tax() {
    let doc = egyptian_exempt_invoice();
    let xml = to_xml(&doc).unwrap();

    // ETA subtype V003 "Exempted good or service": tax-exclusive == tax-inclusive
    // == payable, all 1,000.00 EGP — no output VAT is added.
    assert!(
        xml.contains(r#"currencyID="EGP">1000.00</cbc:TaxExclusiveAmount>"#),
        "exempt tax-exclusive must be 1,000.00 EGP, got: {xml}"
    );
    assert!(
        xml.contains(r#"currencyID="EGP">1000.00</cbc:TaxInclusiveAmount>"#),
        "an exempt supply must add no VAT, got: {xml}"
    );
    assert!(xml.contains(r#"currencyID="EGP">1000.00</cbc:PayableAmount>"#));
    // The tax category carried into the UBL line is the exempt code "E".
    assert!(xml.contains(">E</cbc:ID>"), "exempt category code E must appear");

    let provider = MockEtaProvider::new();
    let (ikb, _) = bundle_lifecycle(
        &doc,
        &submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "exempt-supply evidence bundle must verify");
}

#[test]
fn egypt_export_invoice_is_zero_rated_to_a_foreign_buyer() {
    let doc = egyptian_export_invoice();

    // ETA subtype V001 "Export": foreign buyer, foreign-currency invoice,
    // zero output VAT.
    assert_eq!(doc.customer.address.country, CountryCode::new("DE").unwrap());
    let xml = to_xml(&doc).unwrap();
    assert!(
        xml.contains(">USD</cbc:DocumentCurrencyCode>"),
        "export invoice must be denominated in the foreign currency, got: {xml}"
    );
    assert!(
        xml.contains(">DE</cbc:IdentificationCode>"),
        "the receiver country must be the foreign buyer's (DE), not EG"
    );
    // Zero-rated: no VAT on top of the 1,000.00 USD base.
    assert!(
        xml.contains(r#"currencyID="USD">1000.00</cbc:TaxInclusiveAmount>"#),
        "export total must be zero-rated 1,000.00 USD, got: {xml}"
    );

    let provider = MockEtaProvider::new();
    let (ikb, _) = bundle_lifecycle(
        &doc,
        &submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "export-invoice evidence bundle must verify");
}

#[test]
fn egypt_authority_invalid_verdict_bundles_and_verifies() {
    // ETA runs eight server-side validators after submission and records a
    // per-document status. A document that fails (here: the Reference Document
    // Validator — a credit note whose referenced invoice UUID is unknown) is
    // recorded `Invalid`. Per the EtaProvider::submit contract this is a
    // receipt STATUS, NOT an `Err`: the engine persists the rejection in its
    // audit trail and the evidence bundle still packs and verifies.
    //
    // Source: <https://sdk.invoicing.eta.gov.eg/document-validation-rules/>.
    let doc = egyptian_credit_note("EG-deadbeef-deadbeef");
    let provider = MockEtaProvider::new().with_forced_verdict(
        EtaStatus::Invalid,
        Some("Reference Document Validator: referenced UUID not registered".to_owned()),
    );
    let (ikb, receipt) = bundle_lifecycle(
        &doc,
        &submit_request_for(EtaDocumentKind::CreditNote, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );

    assert_eq!(
        receipt.status,
        EtaStatus::Invalid,
        "an ETA validator failure must surface as EtaStatus::Invalid, not an Err"
    );
    assert!(
        receipt
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("Reference Document Validator")),
        "the Invalid receipt must carry the validator rejection reason, got {:?}",
        receipt.reason
    );
    // The rejection is still durable, auditable evidence.
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(
        report.ok,
        "an Invalid-verdict evidence bundle must still verify"
    );
}

#[test]
fn egypt_authority_valid_verdict_clears_the_document() {
    // The complementary happy verdict: all eight validators pass, ETA records
    // `Valid`, and there is no rejection reason.
    let doc = egyptian_invoice();
    let provider = MockEtaProvider::new().with_forced_verdict(EtaStatus::Valid, None);
    let (ikb, receipt) = bundle_lifecycle(
        &doc,
        &submit_request_for(EtaDocumentKind::Invoice, ISSUER_TAX_ID, Vec::new()),
        &provider,
    );
    assert_eq!(receipt.status, EtaStatus::Valid);
    assert!(receipt.reason.is_none());
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "cleared-document evidence bundle must verify");
}

#[test]
fn egypt_b2c_receipt_uses_a_14_digit_national_id() {
    // B2C e-Receipts (documentType "r") are issued by a natural person against
    // a 14-digit national id, not a 9-digit company tax registration number.
    // Both shapes are accepted by the crate's id validator.
    let doc = egyptian_invoice();
    let provider = MockEtaProvider::new();
    let req = submit_request_for(EtaDocumentKind::Receipt, ISSUER_NATIONAL_ID, Vec::new());
    assert_eq!(req.kind, EtaDocumentKind::Receipt);
    assert_eq!(req.issuer_tax_or_national_id.len(), 14);

    let (ikb, receipt) = bundle_lifecycle(&doc, &req, &provider);
    assert!(receipt.uuid.starts_with("EG-"));
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "B2C receipt evidence bundle must verify");
}

#[test]
fn egypt_b2c_receipt_rejects_a_12_digit_id() {
    // A 12-digit id is neither a valid 9-digit tax registration number nor a
    // 14-digit national id, so the pre-wire shape validator refuses it.
    let provider = MockEtaProvider::new();
    let mut req = submit_request_for(
        EtaDocumentKind::Receipt,
        "123456789012",
        to_xml(&egyptian_invoice()).unwrap().into_bytes(),
    );
    let err = provider.submit(&req).unwrap_err();
    assert!(
        matches!(err, EtaError::BadId(_)),
        "a 12-digit id must refuse with EtaError::BadId, got {err:?}"
    );
    // Sanity: switching to the valid 14-digit national id clears the gate.
    req.issuer_tax_or_national_id = ISSUER_NATIONAL_ID.to_owned();
    assert!(provider.submit(&req).is_ok());
}

#[test]
fn egypt_canonical_serialization_is_deterministic_across_classes() {
    // Canonical serialization must be byte-identical run-to-run for each
    // document class (invoice, multi-line, credit note, exempt, export), the
    // property the signed ETA payload + evidence hash depend on.
    for doc in [
        egyptian_invoice(),
        egyptian_multiline_invoice(),
        egyptian_credit_note("EG-00000001-00000007"),
        egyptian_exempt_invoice(),
        egyptian_export_invoice(),
    ] {
        let a = canonicalize_value(&doc.to_value().unwrap()).unwrap();
        let b = canonicalize_value(&doc.to_value().unwrap()).unwrap();
        assert_eq!(
            a, b,
            "canonical JSON for {:?} {} must be byte-stable",
            doc.document_type,
            doc.document_number.as_str()
        );
        // And the UBL family serialization must equally be deterministic.
        assert_eq!(to_xml(&doc).unwrap(), to_xml(&doc).unwrap());
    }
}
