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
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_jp_qis::{
    validate_registration_number, MockQisRegistryProvider, NtaEnvironment, QisError,
    QisIssuerRegistration, QisLookupRequest, QisRegistryProvider,
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
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: yen(100_000),
            tax_amount: yen(10_000),
            tax_rate: Some(DecimalValue::new(Decimal::from(10))),
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
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/ubl.xml".to_owned(), ubl_bytes);
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&registration).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
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
