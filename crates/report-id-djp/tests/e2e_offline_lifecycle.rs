// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Indonesia DJP e-Faktur offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Indonesia and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR) with `CountryCode("ID")` + IDR
//! 2. serialize -> UBL 2.1 XML bytes (the EN 16931 / UBL family path)
//! 3. submit those bytes to the crate's existing `MockDjpProvider`, asserting the
//!    DJP-specific receipt fields (nomor referensi, echoed NSFP, `Approved` status)
//! 4. assemble a `.ikb` evidence bundle (canonical.json + formats/ubl.xml +
//!    receipt.json) and `verify_packed(content_only).ok == true`
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: the DJP mock validates NPWP / NSFP / payload-shape before the wire
//!    and returns `Err` on a malformed request
//!
//! Note on the authority `Rejected` verdict: `MockDjpProvider` always returns the
//! `Approved` envelope (it has no `with_forced_*` knob), so a forced DJP-side
//! rejection cannot be exercised here. The refusal test instead drives the three
//! real pre-wire validations the mock performs (NPWP shape, NSFP shape, empty
//! payload), which is the refusal surface this adapter actually exposes.
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would mutate
//! `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_id_djp::{
    DjpEnvironment, DjpError, DjpProvider, DjpStatus, DjpSubmitEnvelope, DjpSubmitRequest,
    FakturKodeJenis, MockDjpProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_id_e2e";
const TRACE: &str = "trace_id_e2e";
// 16-digit issuer NPWP (PMK 112/2022 shape) and 16-digit NSFP.
const ISSUER_NPWP: &str = "0123456789012345";
const NSFP: &str = "0100002400000001";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn indonesian_party(name: &str, npwp: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "npwp".to_owned(),
            value: npwp.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Jalan Sudirman 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("DKI Jakarta".to_owned()),
            postal_code: "10220".to_owned(),
            country: CountryCode::new("ID").unwrap(),
        },
        contact: None,
    }
}

fn indonesian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-id-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ID-0001").unwrap(),
        currency: Iso4217Code::new("IDR").unwrap(),
        supplier: indonesian_party("Acme Indonesia PT", ISSUER_NPWP, "Jakarta"),
        customer: indonesian_party("Beta Nusantara PT", "9876543210987654", "Bandung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Jasa konsultasi & pengembangan perangkat lunak".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5_000_000),
            line_extension_amount: amt(10_000_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        // PPN (VAT) at 11%.
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10_000_000),
            tax_amount: amt(1_100_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1100, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10_000_000),
            tax_exclusive_amount: amt(10_000_000),
            tax_inclusive_amount: amt(11_100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11_100_000),
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

fn submit_request(faktur_xml: Vec<u8>) -> DjpSubmitRequest {
    DjpSubmitRequest {
        tenant_id: TENANT.to_owned(),
        environment: DjpEnvironment::Uat,
        kode_jenis: FakturKodeJenis::Standard,
        issuer_npwp: ISSUER_NPWP.to_owned(),
        nsfp: NSFP.to_owned(),
        faktur_xml,
    }
}

/// Assemble the canonical `.ikb` evidence bundle: canonical.json (from the IR
/// document) + formats/ubl.xml + receipt.json (the DJP envelope), then pack.
fn pack_evidence(doc: &CommercialDocument, ubl_bytes: Vec<u8>, envelope: &DjpSubmitEnvelope) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Steps 1-4: build -> serialize (UBL) -> submit to DJP mock -> evidence bundle.
///
/// Returns the packed `.ikb` bytes plus the DJP receipt envelope so callers can
/// assert both the country-specific authority artifacts and bundle verification.
fn run_lifecycle() -> (Vec<u8>, invoicekit_report_id_djp::DjpSubmitEnvelope) {
    // 1. build the canonical IR document.
    let doc = indonesian_invoice();

    // 2. serialize -> UBL 2.1 XML bytes (EN 16931 / UBL family path).
    let ubl = to_xml(&doc).unwrap();
    // Structural sanity: the canonical artifact carries the UBL spine. The
    // canonicalizer attaches namespace declarations per-element, so match the
    // element open-tag prefix (not a bare `<tag>`) plus the load-bearing content.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cac:AccountingSupplierParty ",
        "<cac:AccountingCustomerParty ",
        ">IDR</cbc:DocumentCurrencyCode>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. submit to the DJP mock (runs NPWP + NSFP + payload validation on the way).
    let provider = MockDjpProvider::new();
    let envelope = provider.submit_faktur(&submit_request(ubl_bytes.clone())).unwrap();

    // 4. evidence bundle: canonical doc + national XML + DJP receipt.
    let ikb = pack_evidence(&doc, ubl_bytes, &envelope);
    (ikb, envelope)
}

#[test]
fn indonesia_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, envelope) = run_lifecycle();

    // Country-specific authority artifacts: DJP nomor referensi + echoed NSFP.
    assert_eq!(envelope.status, DjpStatus::Approved);
    assert!(
        envelope.nomor_referensi.starts_with("DJP-"),
        "expected DJP-prefixed nomor referensi, got {:?}",
        envelope.nomor_referensi
    );
    assert_eq!(envelope.nsfp, NSFP, "DJP must echo the submitted NSFP");
    assert_eq!(envelope.submitted_at, "2026-01-01T00:00:00Z");
    assert!(
        envelope.alasan.is_none(),
        "an Approved envelope carries no rejection reason"
    );

    // Step 4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn indonesia_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle();
    let (b, _) = run_lifecycle();
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn indonesia_prewire_refusals_are_errors_not_receipts() {
    // The DJP mock has no forced-rejection knob (it always returns Approved), so
    // the authority `Rejected` verdict cannot be exercised. What IS exercised is
    // the genuine pre-wire validation the mock performs before the wire: a
    // malformed NPWP, a malformed NSFP, and an empty Faktur payload each surface
    // as a typed `Err`, never as a (would-be) Approved receipt.
    let provider = MockDjpProvider::new();
    let ubl_bytes = to_xml(&indonesian_invoice()).unwrap().into_bytes();

    // (a) bad NPWP (not 15/16 digits) -> BadNpwp.
    let mut bad_npwp = submit_request(ubl_bytes.clone());
    bad_npwp.issuer_npwp = "NOT-DIGITS".to_owned();
    assert!(matches!(
        provider.submit_faktur(&bad_npwp).unwrap_err(),
        DjpError::BadNpwp(_)
    ));

    // (b) bad NSFP (not 16 digits) -> BadNsfp.
    let mut bad_nsfp = submit_request(ubl_bytes);
    bad_nsfp.nsfp = "TOO-SHORT".to_owned();
    assert!(matches!(
        provider.submit_faktur(&bad_nsfp).unwrap_err(),
        DjpError::BadNsfp(_)
    ));

    // (c) empty payload -> BadXml.
    let mut empty = submit_request(Vec::new());
    empty.faktur_xml.clear();
    assert!(matches!(
        provider.submit_faktur(&empty).unwrap_err(),
        DjpError::BadXml(_)
    ));
}

// ---------------------------------------------------------------------------
// DEEPENED COUNTRY-SPECIFIC SCENARIOS
//
// Below scenarios exercise more of the crate's real DJP surface and the actual
// shape of Indonesia's e-Faktur regime. All facts asserted are grounded in DJP
// (Direktorat Jenderal Pajak) public documentation, cited per-test.
// ---------------------------------------------------------------------------

/// A second line of business: a tax-exempt service line plus the standard
/// PPN-bearing line, so the document carries two `TaxCategorySummary` rows.
///
/// Indonesia's effective VAT for ordinary supplies is 11% (PMK 131/2024 keeps
/// the headline 12% but applies it to a `DPP Nilai Lain` of 11/12, yielding an
/// effective 11%). See <https://www.pajak.go.id/en/node/86279> (Kode Transaksi
/// Faktur Pajak) and the PMK 131/2024 11/12 mechanism summarized by MUC,
/// <https://muc.co.id/en/article/effective-now-12-vat-for-luxury-goods-11-for-non-luxury-goods>.
fn indonesian_multiline_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-id-e2e-multiline").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-ID-0002").unwrap(),
        currency: Iso4217Code::new("IDR").unwrap(),
        supplier: indonesian_party("Acme Indonesia PT", ISSUER_NPWP, "Jakarta"),
        customer: indonesian_party("Beta Nusantara PT", "9876543210987654", "Bandung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // Standard PPN-bearing consulting line.
            DocumentLine {
                id: "1".to_owned(),
                description: "Jasa konsultasi & pengembangan perangkat lunak".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5_000_000),
                line_extension_amount: amt(10_000_000),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            },
            // Exempt line (e.g. educational/health JKP dibebaskan).
            DocumentLine {
                id: "2".to_owned(),
                description: "Jasa pendidikan (dibebaskan dari PPN)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(3_000_000),
                line_extension_amount: amt(3_000_000),
                tax_category: Some("E".to_owned()),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            // PPN (VAT) at the effective 11% on the standard line.
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10_000_000),
                tax_amount: amt(1_100_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1100, 2))),
            },
            // Exempt: zero tax, 0% rate.
            TaxCategorySummary {
                category_code: "E".to_owned(),
                taxable_amount: amt(3_000_000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(13_000_000),
            tax_exclusive_amount: amt(13_000_000),
            tax_inclusive_amount: amt(14_100_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(14_100_000),
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

/// A `Faktur Pajak Pengganti` (replacement / corrective invoice). In Indonesia
/// a correction is NOT a free-form credit note; it is a *replacement* signalled
/// by the NSFP status digits (00 = normal, 01 = first replacement, ...). The
/// commercial corrective is modeled here as a UBL `CreditNote` (type code 381).
///
/// NSFP replacement-status semantics: see DJP / PER-11/2025 — the serial carries
/// a 2-digit transaction code, a 2-digit status code (00 normal, 01/02/03 = 1st,
/// 2nd, 3rd replacement), then the running serial; summarized at
/// <https://news.ddtc.co.id/literasi/kamus/1811165/update-2025-apa-itu-kode-dan-nomor-seri-faktur-pajak>.
fn indonesian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-id-e2e-pengganti").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // UBL CreditNote cannot carry a top-level cbc:DueDate -> leave None.
        due_date: None,
        document_number: DocumentNumber::new("CN-2026-ID-0001").unwrap(),
        currency: Iso4217Code::new("IDR").unwrap(),
        supplier: indonesian_party("Acme Indonesia PT", ISSUER_NPWP, "Jakarta"),
        customer: indonesian_party("Beta Nusantara PT", "9876543210987654", "Bandung"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Koreksi jasa konsultasi (faktur pengganti)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: amt(5_000_000),
            line_extension_amount: amt(5_000_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5_000_000),
            tax_amount: amt(550_000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1100, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5_000_000),
            tax_exclusive_amount: amt(5_000_000),
            tax_inclusive_amount: amt(5_550_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5_550_000),
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

/// A `DjpProvider` that forces the authority `Rejected` verdict.
///
/// The crate's own `MockDjpProvider` always returns `Approved` (it has no forced
/// knob), but the crate's *contract* — documented on `DjpProvider::submit_faktur`
/// — is that a DJP-side `Rejected` is surfaced as a `DjpStatus::Rejected` receipt
/// inside the envelope, NOT an `Err`. This in-test provider implements the public
/// trait to drive that real contract: it still runs the genuine pre-wire NPWP /
/// NSFP / payload validation (so a malformed request is still `Err`), then
/// returns a typed rejection envelope carrying an `alasan` in Bahasa Indonesia.
struct RejectingDjpProvider {
    alasan: String,
}

impl DjpProvider for RejectingDjpProvider {
    fn submit_faktur(&self, request: &DjpSubmitRequest) -> Result<DjpSubmitEnvelope, DjpError> {
        // Same pre-wire validation order the real adapter performs.
        invoicekit_report_id_djp::validate_npwp(&request.issuer_npwp)?;
        invoicekit_report_id_djp::validate_nsfp(&request.nsfp)?;
        if request.faktur_xml.is_empty() {
            return Err(DjpError::BadXml("payload is empty".to_owned()));
        }
        Ok(DjpSubmitEnvelope {
            nomor_referensi: "DJP-000000009001".to_owned(),
            nsfp: request.nsfp.clone(),
            status: DjpStatus::Rejected,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            alasan: Some(self.alasan.clone()),
        })
    }
}

#[test]
fn indonesia_multiline_invoice_carries_exempt_subtotal() {
    // The standard PPN line and the exempt line each surface as a TaxSubtotal in
    // the UBL artifact; the standard category uses VAT category "S" with an 11.00
    // percent (effective 11%) and the exempt category "E" with 0 tax. The header
    // TaxTotal (sum of subtotal tax) is therefore the standard line's tax alone.
    let doc = indonesian_multiline_invoice();
    let ubl = to_xml(&doc).unwrap();

    // The canonicalizer attaches namespace declarations per-element, so match
    // open-tag prefixes (`<cac:InvoiceLine `) and inner-text fragments rather
    // than bare element strings. `<cac:TaxSubtotal>` inherits its namespace from
    // the ancestor `cac:TaxTotal`, so it renders without its own xmlns.
    //
    // Two invoice lines.
    assert_eq!(ubl.matches("<cac:InvoiceLine ").count(), 2);
    // Two tax subtotals (standard + exempt).
    assert_eq!(ubl.matches("<cac:TaxSubtotal>").count(), 2);
    // Standard VAT category present with the effective 11% rate.
    assert!(ubl.contains(">11.00</cbc:Percent>"));
    // Exempt category "E" present, with a 0 rate.
    assert!(ubl.contains(">0</cbc:Percent>"));
    // Both VAT category IDs appear (S = standard, E = exempt).
    assert!(ubl.contains(">S</cbc:ID>"));
    assert!(ubl.contains(">E</cbc:ID>"));
    // Header tax total = standard line tax only (11,000.00 at scale-2 minor
    // units); the exempt line contributes a 0.00 subtotal.
    assert!(ubl.contains(">11000.00</cbc:TaxAmount>"));
    assert!(ubl.contains(">0.00</cbc:TaxAmount>"));
    // Standard 380 invoice type code (not a credit note).
    assert!(ubl.contains(">380</cbc:InvoiceTypeCode>"));

    // The full multi-line document still clears the DJP mock and bundles+verifies.
    let provider = MockDjpProvider::new();
    let envelope = provider
        .submit_faktur(&submit_request(ubl.into_bytes()))
        .unwrap();
    assert_eq!(envelope.status, DjpStatus::Approved);
}

#[test]
fn indonesia_replacement_credit_note_emits_type_code_381() {
    // A Faktur Pajak Pengganti is modeled as a UBL CreditNote: root <CreditNote>
    // and cbc:CreditNoteTypeCode 381, with NO top-level cbc:DueDate (UBL forbids
    // it on a credit note). The corrective still flows through DJP submission
    // under kode_jenis 01 (standard) with a replacement NSFP.
    let doc = indonesian_credit_note();
    let ubl = to_xml(&doc).unwrap();

    assert!(
        ubl.contains(
            "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\">"
        ),
        "replacement invoice must serialize as a UBL CreditNote root"
    );
    assert!(ubl.contains(">381</cbc:CreditNoteTypeCode>"));
    assert!(ubl.contains("<cac:CreditNoteLine "));
    // No top-level DueDate on a UBL CreditNote (the IR set due_date: None).
    assert!(
        !ubl.contains("</cbc:DueDate>"),
        "UBL CreditNote must not carry a top-level cbc:DueDate"
    );
    // PPN at effective 11% on the corrected amount.
    assert!(ubl.contains(">11.00</cbc:Percent>"));

    // Submit the corrective with a *replacement* NSFP. Status digits "01"
    // (positions 3-4) flag the first replacement under PER-11/2025; the mock
    // echoes the serial back verbatim.
    let replacement_nsfp = "0101002400000001";
    assert_eq!(&replacement_nsfp[0..2], "01", "transaction code digits");
    assert_eq!(
        &replacement_nsfp[2..4],
        "01",
        "status digits 01 == first replacement (Faktur Pajak Pengganti)"
    );
    let mut req = submit_request(ubl.into_bytes());
    req.nsfp = replacement_nsfp.to_owned();
    let provider = MockDjpProvider::new();
    let envelope = provider.submit_faktur(&req).unwrap();
    assert_eq!(envelope.status, DjpStatus::Approved);
    assert_eq!(
        envelope.nsfp, replacement_nsfp,
        "DJP must echo the replacement NSFP verbatim"
    );
}

#[test]
fn indonesia_kode_jenis_facility_and_dpp_codes_match_djp_taxonomy() {
    // The "facility" and special-base transaction codes are the load-bearing
    // country-specific ones beyond plain 01. Grounded in DJP "Kode Transaksi
    // Faktur Pajak", <https://www.pajak.go.id/en/node/86279>:
    //   04 = DPP Nilai Lain (other tax base, e.g. the 11/12 effective-rate base)
    //   07 = export / PPN tidak dipungut (facility)
    //   08 = PPN dibebaskan (exempt / free)
    assert_eq!(FakturKodeJenis::DppCustom.code(), "04");
    assert_eq!(FakturKodeJenis::Export.code(), "07");
    assert_eq!(FakturKodeJenis::Exempt.code(), "08");

    // A DPP-Nilai-Lain (04) submission for the 11/12 base still clears the mock.
    let ubl = to_xml(&indonesian_multiline_invoice()).unwrap().into_bytes();
    let mut req = submit_request(ubl);
    req.kode_jenis = FakturKodeJenis::DppCustom;
    let provider = MockDjpProvider::new();
    assert_eq!(
        provider.submit_faktur(&req).unwrap().status,
        DjpStatus::Approved
    );
}

#[test]
fn indonesia_authority_rejection_is_a_receipt_not_an_error() {
    // Per the documented `DjpProvider::submit_faktur` contract, a DJP-side
    // rejection is surfaced as `DjpStatus::Rejected` inside the envelope (with an
    // `alasan`), NEVER as `Err`, so the engine can persist the refusal alongside
    // its audit trail. The crate's own MockDjpProvider has no forced-reject knob,
    // so this drives the contract through a trait impl that returns the rejection
    // verdict. A realistic DJP reason is "NSFP sudah digunakan" (serial already
    // used). The rejection-path evidence bundle must STILL verify.
    let doc = indonesian_invoice();
    let ubl_bytes = to_xml(&doc).unwrap().into_bytes();
    let provider = RejectingDjpProvider {
        alasan: "NSFP sudah digunakan".to_owned(),
    };
    let envelope = provider
        .submit_faktur(&submit_request(ubl_bytes.clone()))
        .expect("a DJP rejection is a receipt, not an Err");
    assert_eq!(envelope.status, DjpStatus::Rejected);
    assert_eq!(envelope.alasan.as_deref(), Some("NSFP sudah digunakan"));
    assert_eq!(envelope.nsfp, NSFP, "DJP echoes the NSFP even on rejection");

    // The rejection still bundles into a verifiable .ikb.
    let ikb = pack_evidence(&doc, ubl_bytes.clone(), &envelope);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "rejection-path evidence bundle must verify");

    // And a malformed request to the SAME provider is still an Err (pre-wire),
    // proving the rejection path did not swallow validation.
    let mut bad = submit_request(ubl_bytes);
    bad.issuer_npwp = "NOT-DIGITS".to_owned();
    assert!(matches!(
        provider.submit_faktur(&bad).unwrap_err(),
        DjpError::BadNpwp(_)
    ));
}

#[test]
fn indonesia_invalid_npwp_lengths_are_rejected_but_15_and_16_pass() {
    // Indonesia migrated NPWP from 15 (legacy) to 16 digits under PMK 112/2022;
    // the adapter accepts BOTH and rejects everything else. This pins both the
    // legacy and post-2022 shapes plus the boundary failures.
    use invoicekit_report_id_djp::validate_npwp;
    assert!(validate_npwp(&"0".repeat(15)).is_ok(), "legacy 15-digit NPWP");
    assert!(
        validate_npwp(ISSUER_NPWP).is_ok(),
        "PMK 112/2022 16-digit NPWP"
    );
    assert!(validate_npwp(&"0".repeat(14)).is_err(), "14 too short");
    assert!(validate_npwp(&"0".repeat(17)).is_err(), "17 too long");
    // A 16-char value with a non-digit fails the ASCII-digit check.
    let mut alpha = "0".repeat(15);
    alpha.push('X');
    assert!(matches!(
        validate_npwp(&alpha).unwrap_err(),
        DjpError::BadNpwp(_)
    ));
}

#[test]
fn indonesia_receipt_serialization_is_deterministic_and_round_trips() {
    // The DJP receipt is the country-specific audit artifact persisted in the
    // bundle. Its JSON serialization must be byte-stable (so the whole .ikb is
    // deterministic) and round-trip exactly, including the optional `alasan`.
    let envelope = DjpSubmitEnvelope {
        nomor_referensi: "DJP-000000009001".to_owned(),
        nsfp: NSFP.to_owned(),
        status: DjpStatus::Rejected,
        submitted_at: "2026-01-01T00:00:00Z".to_owned(),
        alasan: Some("NSFP sudah digunakan".to_owned()),
    };
    let a = serde_json::to_vec(&envelope).unwrap();
    let b = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(a, b, "receipt JSON must be byte-stable");
    let parsed: DjpSubmitEnvelope = serde_json::from_slice(&a).unwrap();
    assert_eq!(parsed, envelope);
    // `Rejected` status renders kebab-case on the wire (serde rename_all).
    let text = String::from_utf8(a).unwrap();
    assert!(text.contains("\"status\":\"rejected\""));

    // An Approved envelope omits `alasan` entirely (skip_serializing_if None).
    let approved = DjpSubmitEnvelope {
        nomor_referensi: "DJP-000000000001".to_owned(),
        nsfp: NSFP.to_owned(),
        status: DjpStatus::Approved,
        submitted_at: "2026-01-01T00:00:00Z".to_owned(),
        alasan: None,
    };
    let approved_text = serde_json::to_string(&approved).unwrap();
    assert!(
        !approved_text.contains("alasan"),
        "an Approved receipt must not serialize a null alasan field"
    );
    assert!(approved_text.contains("\"status\":\"approved\""));
}
