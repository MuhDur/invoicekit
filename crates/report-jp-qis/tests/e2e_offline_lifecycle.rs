// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Japan QIS offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Japan and proves it deterministically.
//! Unlike a clearance country (Italy SDI), Japan does NOT operate a portal:
//! the National Tax Agency only runs a registration registry the buyer pings
//! to confirm the issuer is registered, and wire delivery rides Peppol-JP
//! (Peppol BIS Billing 3 over the EN 16931 / UBL family). The lifecycle here is
//! therefore:
//!
//! 1. build a canonical `CommercialDocument` (IR) with a JP `CountryCode` + JPY
//! 2. serialize -> UBL 2.1 bytes (the Peppol-JP / EN 16931 family path)
//! 3. look the issuer's NTA registration number up via the EXISTING
//!    `MockQisRegistryProvider` and assert the registry record's
//!    JP-specific fields (`T` + 13 digits, `effective_from`, revoked status)
//! 4. assemble a `.ikb` evidence bundle and `verify_packed` it (exit 0 == ok)
//! 5. determinism: pack twice -> byte-identical
//! 6. refusal: a malformed registration number is rejected up front
//!
//! Goldens are hand-rolled (no `insta` / `pretty_assertions`, which would
//! mutate `Cargo.lock`).

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_jp_qis::{
    jct_basis_points, validate_registration_number, JctCategory, MockQisRegistryProvider,
    NtaEnvironment, QisError, QisInvoiceKind, QisIssuerRegistration, QisLookupRequest,
    QisRegistryProvider,
};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_jp_e2e";
const TRACE: &str = "trace_jp_e2e";
const REGISTRATION_NUMBER: &str = "T1234567890123";

/// JPY is a zero-decimal currency, but the IR's `DecimalValue` carries no
/// currency and imposes no scale, so we keep amounts at integer yen.
fn yen(units: i64) -> DecimalValue {
    DecimalValue::new(Decimal::from(units))
}

fn japanese_party(name: &str, id: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "jp:nta".to_owned(),
            value: id.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["1-1 Chiyoda".to_owned()],
            city: city.to_owned(),
            subdivision: Some("Tokyo".to_owned()),
            postal_code: "100-0001".to_owned(),
            country: CountryCode::new("JP").unwrap(),
        },
        contact: None,
    }
}

fn japanese_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-jp-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-JP-0001").unwrap(),
        currency: Iso4217Code::new("JPY").unwrap(),
        // Supplier carries the NTA registration number (T + 13 digits) — the
        // mark that makes this a qualified invoice (適格請求書).
        supplier: japanese_party("Acme KK", REGISTRATION_NUMBER, "Chiyoda"),
        customer: japanese_party("Beta GK", "T9876543210987", "Minato"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Software consulting (10% standard JCT)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("EA".to_owned()),
            unit_price: yen(50_000),
            line_extension_amount: yen(100_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: yen(100_000),
            tax_amount: yen(10_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: yen(100_000),
            tax_exclusive_amount: yen(100_000),
            tax_inclusive_amount: yen(110_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: yen(110_000),
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

fn lookup_request() -> QisLookupRequest {
    QisLookupRequest {
        tenant_id: TENANT.to_owned(),
        environment: NtaEnvironment::Sandbox,
        registration_number: REGISTRATION_NUMBER.to_owned(),
    }
}

/// Assemble a packed `.ikb` evidence bundle from the canonical document, the
/// national-family UBL bytes, and the NTA registry receipt. Both the
/// happy-path lifecycle and the corrective (credit-note) path emit the same
/// three-artefact bundle under the pinned manifest.
fn pack_jp_bundle(
    doc: &CommercialDocument,
    ubl_bytes: Vec<u8>,
    registration: &QisIssuerRegistration,
) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(registration).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    pack(&bundle).unwrap()
}

/// Steps 1-4: build -> serialize -> NTA registry lookup -> evidence bundle.
///
/// `revoke` flips the issuer registration into the revoked state before the
/// lookup, exercising the refusal/expiry branch the mock supports.
fn run_lifecycle(revoke: bool) -> (Vec<u8>, QisIssuerRegistration) {
    // 1. build a JP IR document (JP country, JPY currency).
    let doc = japanese_invoice();

    // 2. serialize -> UBL 2.1 bytes (Peppol-JP / EN 16931 family path).
    let ubl = to_xml(&doc).unwrap();
    // Local structural sanity: the EN 16931 / UBL spine is present. The
    // canonicalizer emits namespace declarations inline on the first use of
    // each prefix, so we match the substrings that survive that, including the
    // JP-specific values: JPY currency, the issuer's NTA registration number
    // carried as the supplier `cbc:CompanyID`, and the payable yen total.
    for needle in [
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">",
        "<cac:AccountingSupplierParty",
        "<cac:AccountingCustomerParty",
        ">JPY</cbc:DocumentCurrencyCode>",
        ">T1234567890123</cbc:CompanyID>",
        " currencyID=\"JPY\">110000</cbc:PayableAmount>",
    ] {
        assert!(ubl.contains(needle), "UBL missing {needle}\n{ubl}");
    }
    let ubl_bytes = ubl.into_bytes();

    // 3. NTA registry lookup (the JP "authority receipt"). Japan has no
    //    clearance portal; the buyer confirms the issuer is registered so it
    //    can claim JCT input credit.
    let provider = MockQisRegistryProvider::default();
    if revoke {
        provider.revoke(REGISTRATION_NUMBER);
    }
    let registration = provider.lookup(&lookup_request()).unwrap();

    // 4. evidence bundle: canonical doc + national-family UBL + registry receipt.
    let ikb = pack_jp_bundle(&doc, ubl_bytes, &registration);
    (ikb, registration)
}

#[test]
fn japan_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, registration) = run_lifecycle(false);

    // JP-specific authority artifacts: the NTA-issued registration number
    // (T + 13 digits), the deterministic synthetic legal name, the effective
    // date, and an active (non-revoked) registration.
    assert!(validate_registration_number(&registration.registration_number).is_ok());
    assert_eq!(registration.registration_number, REGISTRATION_NUMBER);
    assert_eq!(registration.legal_name, "Mock JP Issuer 1234");
    assert_eq!(registration.effective_from, "2023-10-01T00:00:00Z");
    assert!(
        registration.revoked_at.is_none(),
        "active issuer must not carry a revocation date"
    );

    // Step-4 success criterion: the bundle verifies (exit 0 == report.ok).
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "evidence bundle must verify");
}

#[test]
fn japan_revoked_registration_still_bundles_and_verifies() {
    // A revoked registration is a registry STATE, not an error: the audit
    // trail records the revocation and the bundle still verifies.
    let (ikb, registration) = run_lifecycle(true);
    assert_eq!(registration.registration_number, REGISTRATION_NUMBER);
    assert_eq!(
        registration.revoked_at.as_deref(),
        Some("2025-12-31T23:59:59Z"),
        "revoked issuer must carry the revocation date"
    );

    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "revoked-path evidence bundle must verify");
}

#[test]
fn japan_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(false);
    let (b, _) = run_lifecycle(false);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn japan_rejects_malformed_registration_number() {
    // The mock runs the SAME `validate_registration_number` the real adapter
    // would: a malformed number (here, wrong prefix) is an Err up front, before
    // any registry record is synthesized — the country-id-shape refusal bucket.
    let provider = MockQisRegistryProvider::default();
    let mut bad = lookup_request();
    bad.registration_number = "X1234567890123".to_owned();
    let err = provider.lookup(&bad).unwrap_err();
    assert!(
        matches!(err, QisError::BadRegistrationNumber(_)),
        "malformed registration number must be a BadRegistrationNumber refusal, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Deepened, Japan-specific coverage.
//
// Authority: National Tax Agency of Japan (国税庁 / NTA), Qualified Invoice
// System (適格請求書等保存方式 / "invoice system"), live since 1 October 2023.
// Spec references cited per scenario below; the canonical landing page is
//   https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm
// (NTA "適格請求書等保存方式の概要" — overview of the qualified invoice system).
// Fixtures are hand-built synthetic data; no copyrighted regulator file is
// vendored.
// ---------------------------------------------------------------------------

const REDUCED_RATE_RN: &str = "T8011223344556";

/// A qualified invoice that mixes the 10% standard and 8% reduced JCT rates in
/// one document. This is THE defining feature of the QIS: NTA "適格請求書の記載
/// 事項" (required entries) item 4 mandates that a qualified invoice show, *per
/// applicable tax rate*, the rate-keyed taxable base and the consumption tax
/// amount. The 8% reduced rate applies to food/beverages (excluding alcohol and
/// eat-in) and subscription newspapers under the 軽減税率 (reduced-rate) regime.
/// Source: NTA pamphlet "消費税の仕入税額控除制度における適格請求書等保存方式
/// (インボイス制度)" — <https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm>
fn dual_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-jp-dualrate-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-JP-DUAL-1").unwrap(),
        currency: Iso4217Code::new("JPY").unwrap(),
        supplier: japanese_party("Konbini KK", REDUCED_RATE_RN, "Chiyoda"),
        customer: japanese_party("Beta GK", "T9876543210987", "Minato"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            // 10% standard: stationery.
            DocumentLine {
                id: "1".to_owned(),
                description: "Office stationery (10% standard JCT)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(10)),
                unit_code: Some("EA".to_owned()),
                unit_price: yen(300),
                line_extension_amount: yen(3_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            // 8% reduced: bottled tea (a non-alcoholic beverage qualifies for
            // the 軽減税率 reduced rate).
            DocumentLine {
                id: "2".to_owned(),
                description: "Bottled green tea, takeaway (8% reduced JCT 軽減税率)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(20)),
                unit_code: Some("EA".to_owned()),
                unit_price: yen(150),
                line_extension_amount: yen(3_000),
                tax_category: Some("AA".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            // 10% on 3,000 yen = 300 yen.
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: yen(3_000),
                tax_amount: yen(300),
                tax_rate: Some(DecimalValue::new(Decimal::from(10))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            // 8% on 3,000 yen = 240 yen.
            TaxCategorySummary {
                category_code: "AA".to_owned(),
                taxable_amount: yen(3_000),
                tax_amount: yen(240),
                tax_rate: Some(DecimalValue::new(Decimal::from(8))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: yen(6_000),
            tax_exclusive_amount: yen(6_000),
            // 6,000 + 300 + 240 = 6,540 yen.
            tax_inclusive_amount: yen(6_540),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: yen(6_540),
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

#[test]
fn japan_dual_rate_invoice_breaks_jct_out_per_rate() {
    // QIS required-entry #4: a qualified invoice must state, per applicable
    // rate, both the rate-keyed taxable base and the consumption-tax amount.
    // We assert the UBL TaxTotal carries the standard 10% AND the reduced 8%
    // subtotal as distinct yen amounts — not a single blended figure.
    let doc = dual_rate_invoice();
    let ubl = to_xml(&doc).unwrap();

    // The canonicalizer emits namespace declarations inline on the first use of
    // each prefix, so we match the substrings that survive that — the
    // JP-specific yen amounts, percentages, and the reduced-rate registration
    // number.
    for needle in [
        // Aggregate JCT across both rates: 300 + 240 = 540 yen.
        " currencyID=\"JPY\">540</cbc:TaxAmount>",
        // Standard 10% subtotal: 3,000 taxable.
        " currencyID=\"JPY\">3000</cbc:TaxableAmount>",
        // Standard 10% subtotal tax: 300 yen.
        " currencyID=\"JPY\">300</cbc:TaxAmount>",
        // Reduced 8% subtotal tax: 240 yen.
        " currencyID=\"JPY\">240</cbc:TaxAmount>",
        // Both rate percentages are present and distinct.
        ">8</cbc:Percent>",
        ">10</cbc:Percent>",
        // Reduced-rate category code on its subtotal.
        ">AA</cbc:ID>",
        // Yen total inclusive of both rates.
        " currencyID=\"JPY\">6540</cbc:PayableAmount>",
        // Reduced-rate registration number stamped on the supplier.
        ">T8011223344556</cbc:CompanyID>",
    ] {
        assert!(ubl.contains(needle), "dual-rate UBL missing {needle}\n{ubl}");
    }

    // The crate's JCT rate table backs those percentages with basis points.
    assert_eq!(jct_basis_points(JctCategory::Standard10), 1000);
    assert_eq!(jct_basis_points(JctCategory::Reduced8), 800);

    // The reduced-rate issuer is registrable and resolves in the NTA registry.
    assert!(validate_registration_number(REDUCED_RATE_RN).is_ok());
    let provider = MockQisRegistryProvider::default();
    let reg = provider
        .lookup(&QisLookupRequest {
            tenant_id: TENANT.to_owned(),
            environment: NtaEnvironment::Sandbox,
            registration_number: REDUCED_RATE_RN.to_owned(),
        })
        .unwrap();
    assert_eq!(reg.registration_number, REDUCED_RATE_RN);
    assert!(reg.revoked_at.is_none());
}

/// A qualified return/corrective document. Under the QIS a seller who returns
/// consideration (refund, rebate, sales return) must issue a 適格返還請求書
/// ("qualified return invoice"). NTA models this as a credit-side document; in
/// the EN 16931 / UBL family it serializes as a UBL `CreditNote`
/// (`CreditNoteTypeCode` 381). Source: NTA "適格返還請求書の交付義務" — qualified
/// return invoice issuance obligation,
/// <https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm>
fn qualified_return_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-jp-return-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-06-10").unwrap(),
        tax_point_date: None,
        // UBL 2.1 CreditNote carries no top-level cbc:DueDate; must stay None.
        due_date: None,
        document_number: DocumentNumber::new("CN-2026-JP-0001").unwrap(),
        currency: Iso4217Code::new("JPY").unwrap(),
        supplier: japanese_party("Acme KK", REGISTRATION_NUMBER, "Chiyoda"),
        customer: japanese_party("Beta GK", "T9876543210987", "Minato"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Returned: software consulting (10% standard JCT)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: yen(50_000),
            line_extension_amount: yen(50_000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: yen(50_000),
            tax_amount: yen(5_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: yen(50_000),
            tax_exclusive_amount: yen(50_000),
            tax_inclusive_amount: yen(55_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: yen(55_000),
        },
        attachments: Vec::new(),
        // The return invoice cites the original qualified invoice it corrects.
        references: vec![DocumentReference {
            kind: "original-invoice".to_owned(),
            id: "INV-2026-JP-0001".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }],
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

#[test]
fn japan_qualified_return_invoice_serializes_as_ubl_credit_note() {
    // 適格返還請求書 -> UBL CreditNote with CreditNoteTypeCode 381 and
    // CreditNoteLine/CreditedQuantity (NOT InvoiceLine), carrying the issuer's
    // T-registration number so the buyer's input-credit reversal is traceable.
    let doc = qualified_return_credit_note();
    let ubl = to_xml(&doc).unwrap();

    // Namespace declarations land inline on first prefix use, so match the
    // substrings that survive canonicalization.
    for needle in [
        "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\">",
        ">381</cbc:CreditNoteTypeCode>",
        "<cac:CreditNoteLine ",
        " unitCode=\"EA\">1</cbc:CreditedQuantity>",
        ">JPY</cbc:DocumentCurrencyCode>",
        ">T1234567890123</cbc:CompanyID>",
        " currencyID=\"JPY\">55000</cbc:PayableAmount>",
    ] {
        assert!(
            ubl.contains(needle),
            "qualified-return CreditNote UBL missing {needle}\n{ubl}"
        );
    }
    // It must NOT be an Invoice: no InvoiceTypeCode / InvoiceLine spine.
    assert!(
        !ubl.contains("InvoiceTypeCode"),
        "a credit note must not emit InvoiceTypeCode\n{ubl}"
    );
    assert!(
        !ubl.contains("<cac:InvoiceLine"),
        "a credit note must not emit InvoiceLine\n{ubl}"
    );

    // The corrective document still bundles into verifiable evidence.
    let provider = MockQisRegistryProvider::default();
    let registration = provider.lookup(&lookup_request()).unwrap();
    let ikb = pack_jp_bundle(&doc, ubl.into_bytes(), &registration);
    let report = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(report.ok, "qualified-return evidence bundle must verify");
}

#[test]
fn japan_credit_note_lifecycle_is_byte_deterministic() {
    // Determinism extends to the corrective-document path: the canonicalizer +
    // pinned manifest yield byte-identical UBL CreditNote bytes across runs.
    let a = to_xml(&qualified_return_credit_note()).unwrap();
    let b = to_xml(&qualified_return_credit_note()).unwrap();
    assert_eq!(a, b, "credit-note UBL serialization must be byte-stable");
}

#[test]
fn japan_simplified_qualified_invoice_kind_is_distinct() {
    // 適格簡易請求書 (simplified qualified invoice) is permitted for retail,
    // restaurant, taxi/transport, and parking businesses, where the buyer's
    // name may be omitted and a single rate-OR-tax figure is allowed. It is a
    // first-class QIS document kind distinct from the full qualified invoice.
    // Source: NTA "適格簡易請求書の記載事項" (simplified qualified invoice
    // required entries),
    // https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm
    assert_ne!(QisInvoiceKind::Qualified, QisInvoiceKind::Simplified);

    // Both kinds serde round-trip to their kebab-case wire tokens.
    assert_eq!(
        serde_json::to_string(&QisInvoiceKind::Simplified).unwrap(),
        "\"simplified\""
    );
    assert_eq!(
        serde_json::to_string(&QisInvoiceKind::Qualified).unwrap(),
        "\"qualified\""
    );
    let parsed: QisInvoiceKind = serde_json::from_str("\"simplified\"").unwrap();
    assert_eq!(parsed, QisInvoiceKind::Simplified);
}

#[test]
fn japan_jct_exempt_and_zero_supplies_carry_no_tax() {
    // The QIS distinguishes 0%-rated supplies (exports / 輸出免税) from
    // tax-exempt supplies (非課税 — e.g. medical, social welfare). Both yield
    // zero JCT, but they are modelled as separate categories because an
    // exempt-only business is not entitled to register as a qualified-invoice
    // issuer at all. Source: NTA "課税の対象とならない取引・非課税取引" guidance,
    // https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm
    assert_eq!(jct_basis_points(JctCategory::Zero), 0);
    assert_eq!(jct_basis_points(JctCategory::Exempt), 0);
    // But the standard and reduced rates are non-zero and distinct.
    assert!(jct_basis_points(JctCategory::Standard10) > jct_basis_points(JctCategory::Reduced8));
    assert_eq!(jct_basis_points(JctCategory::Standard10), 1000);
    assert_eq!(jct_basis_points(JctCategory::Reduced8), 800);
}

#[test]
fn japan_registry_rejects_short_and_alpha_registration_numbers() {
    // The NTA registration number is strictly `T` + 13 ASCII digits (法人番号 /
    // Corporate Number for incorporated issuers, a distinct NTA-assigned number
    // for individuals). Anything else is refused before any registry record is
    // synthesized — the country-id-shape refusal bucket, returned as Err (not a
    // registry "not found" state). Source: NTA registration-number format
    // guidance, https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm
    let provider = MockQisRegistryProvider::default();
    for bad in [
        "T123456789012",   // 12 digits — too short
        "T12345678901234", // 14 digits — too long
        "1234567890123",   // missing the T prefix
        "T123456789012X",  // non-digit in the body
        "t1234567890123",  // lowercase prefix
    ] {
        let mut req = lookup_request();
        req.registration_number = bad.to_owned();
        let err = provider.lookup(&req).unwrap_err();
        assert!(
            matches!(err, QisError::BadRegistrationNumber(_)),
            "{bad:?} must be a BadRegistrationNumber refusal, got {err:?}"
        );
    }
}

#[test]
fn japan_revoked_issuer_blocks_buyer_input_credit_in_audit_trail() {
    // QIS economics: a buyer may claim JCT input credit ONLY against a
    // qualified invoice from an issuer registered at the time of supply. The
    // NTA publishes revocations (登録の取消し) in its public registry. We assert
    // the registry surfaces the revocation date so the audit trail records that
    // input credit would be disallowed — yet the revoked state is data, not an
    // Err. Source: NTA "適格請求書発行事業者公表サイト" (public registry),
    // https://www.invoice-kohyo.nta.go.jp/
    let provider = MockQisRegistryProvider::default();
    provider.revoke(REGISTRATION_NUMBER);
    let reg = provider.lookup(&lookup_request()).unwrap();
    assert_eq!(reg.registration_number, REGISTRATION_NUMBER);
    assert_eq!(
        reg.revoked_at.as_deref(),
        Some("2025-12-31T23:59:59Z"),
        "a revoked issuer must carry a revocation date the audit trail can record"
    );
    // The revoked registration serializes the revoked_at field (it is NOT
    // skipped, unlike the active case) so downstream consumers see it.
    let json = serde_json::to_string(&reg).unwrap();
    assert!(
        json.contains("revoked_at"),
        "revoked registration must serialize revoked_at, got {json}"
    );
}
