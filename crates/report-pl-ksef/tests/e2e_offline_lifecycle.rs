// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Poland KSeF offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Poland and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national FA(3) (`FA_VAT`, `<Faktura>`) XML
//! 3. local validate (structural + Polish NIP weighted-checksum)
//! 4. sign + transmit via the offline `MockKsefReportProvider` (composes
//!    `invoicekit-signer-ksef`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. rejection path: a KSeF rejected status is a receipt, NOT an `Err`
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). Capability-matrix presence is asserted centrally elsewhere,
//! not here.

use std::collections::BTreeMap;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_pl_ksef::{
    to_fa3_xml, validate_nip, Fa3Context, KsefAcceptance, KsefEnvironment, KsefReport,
    KsefReportError, KsefReportProvider, KsefReportRequest, MockKsefReportProvider,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_ksef::AuthMode;
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;
use std::sync::Arc;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_pl_e2e";
const TRACE: &str = "trace_pl_e2e";
// The inner KSeF mock keys its signer by the session token it mints; the first
// session is always `sess-00000001`.
const SESSION_KEY: &str = "sess-00000001";
// 5252248481 is a valid-checksum Polish NIP.
const ISSUER_NIP: &str = "5252248481";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn polish_party(name: &str, nip: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: nip.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["ul. Marszałkowska 1".to_owned()],
            city: city.to_owned(),
            subdivision: None,
            postal_code: "00-001".to_owned(),
            country: CountryCode::new("PL").unwrap(),
        },
        contact: None,
    }
}

fn polish_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pl-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FV-2026-PL-0001").unwrap(),
        currency: Iso4217Code::new("PLN").unwrap(),
        supplier: polish_party("Acme Sp. z o.o.", "PL5252248481", "Warszawa"),
        customer: polish_party("Beta S.A.", "5260001246", "Kraków"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Usługi konsultingowe & rozwój oprogramowania".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("C62".to_owned()),
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
            tax_amount: amt(2300),
            tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12300),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12300),
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

/// Build a report provider. The inner KSeF mock keys its signer by the
/// *session token* it mints (`sess-00000001` for the first session),
/// independent of the issuer NIP — so a single session key serves every NIP.
fn provider(forced: Option<KsefAcceptance>) -> MockKsefReportProvider {
    let signer: Arc<dyn Signer> =
        Arc::new(SoftwareSigner::new().with_key(SESSION_KEY, [5_u8; 32]));
    let p = MockKsefReportProvider::new(signer, KsefEnvironment::Demo);
    match forced {
        Some(acceptance) => p.with_forced_acceptance(acceptance),
        None => p,
    }
}

fn report_request(fa_xml: Vec<u8>) -> KsefReportRequest {
    report_request_for(ISSUER_NIP, fa_xml)
}

/// Steps 1-5: build -> serialize -> validate -> sign/transmit -> evidence bundle.
fn run_lifecycle(forced: Option<KsefAcceptance>) -> (Vec<u8>, KsefReport) {
    // 1. build
    let doc = polish_invoice();

    // 2. serialize -> FA(3) (pinned header context for byte stability)
    let ctx = Fa3Context {
        data_wytworzenia: "2026-05-26T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let fa = to_fa3_xml(&doc, &ctx).unwrap();

    // 3. local validate (structural): the national artifact carries the
    // mandatory FA(3) spine. Reference XSD validation stays external (JVM).
    for needle in [
        "<Faktura xmlns=",
        "<Naglowek>",
        "<Podmiot1>",
        "<Podmiot2>",
        "<RodzajFaktury>VAT</RodzajFaktury>",
        "<P_15>123.00</P_15>",
    ] {
        assert!(fa.contains(needle), "FA(3) missing {needle}");
    }

    // 4. sign + transmit (offline mock composing the real KSeF signer path)
    let report = provider(forced)
        .report(&report_request(fa.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national XML + signed artifact + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/fa3.xml".to_owned(), fa.into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_fa_xml.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, report)
}

#[test]
fn poland_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(None);

    // Happy path: KSeF accepted and assigned a Numer KSeF.
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);
    assert!(report.envelope.acceptance.is_accepted());
    assert!(report.envelope.numer_ksef.starts_with(ISSUER_NIP));
    assert!(report.envelope.upo_reference.starts_with("upo-"));
    assert_eq!(report.envelope.issuer_nip, ISSUER_NIP);
    assert!(report.envelope.reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn poland_rejection_still_bundles_and_verifies() {
    // A KSeF rejected acceptance is a receipt kind, NOT an Err — the audit
    // trail persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(Some(KsefAcceptance::Rejected));
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Rejected);
    assert!(!report.envelope.acceptance.is_accepted());
    assert!(report.envelope.numer_ksef.is_empty());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn poland_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(None);
    let (b, _) = run_lifecycle(None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

// ===========================================================================
// Deepened, genuinely Poland-specific scenarios.
//
// All facts below are grounded in the Polish national e-invoice (KSeF / FA(3))
// specification published by the Ministerstwo Finansów (Ministry of Finance):
//
//   * FA(3) logical structure ("Struktura logiczna e-Faktury FA(3)") and the
//     `RodzajFaktury` codelist — Krajowy System e-Faktur, schema namespace
//     http://crd.gov.pl/wzor/2025/06/25/06251/ (the `06251` FA(3) wzór).
//     Reference: https://www.podatki.gov.pl/ksef/ (Struktury FA / FA(3)).
//   * The KSeF reference-number ("Numer KSeF") shape
//     `NIP-YYYYMMDD-XXXXXXXXXXXX-XX` documented in the KSeF API specification.
//     Reference: https://ksef.mf.gov.pl/ (Specyfikacja API KSeF).
//   * The Polish VAT rate bands (23 % standard, 8 % / 5 % reduced, 0 %, and
//     "zw" exemption) per ustawa o VAT (Dz.U. 2004 nr 54 poz. 535, art. 41 /
//     art. 43). Reference: https://www.podatki.gov.pl/vat/.
//   * The NIP (Numer Identyfikacji Podatkowej) weighted-modulo-11 checksum,
//     weights {6,5,7,2,3,4,5,6,7}. Reference: ustawa o NIP, Dz.U. 1995 nr 142
//     poz. 702.
//
// Fixtures are hand-built and license-safe: no regulator XML is vendored.
// ===========================================================================

// A second valid-checksum Polish NIP, used as the corrective-document seller so
// the synthesized Numer KSeF differs from the ordinary-invoice path.
// 5213003700 passes the official NIP modulo-11 checksum.
const CORRECTIVE_ISSUER_NIP: &str = "5213003700";

fn report_request_for(nip: &str, fa_xml: Vec<u8>) -> KsefReportRequest {
    KsefReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: KsefEnvironment::Demo,
        issuer_nip: nip.to_owned(),
        auth_mode: AuthMode::QualifiedSignature,
        fa_xml,
    }
}

/// A *faktura korygująca* (corrective invoice). Under the FA(3) structure a
/// correction carries `RodzajFaktury` = `KOR` (Ministerstwo Finansów, FA(3)
/// logical structure, element `RodzajFaktury`, value `KOR` = faktura
/// korygująca). The IR `CreditNote` document type maps to `KOR`.
fn polish_corrective_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pl-e2e-kor-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // A korekta carries no DueDate spine in this flow.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FK-2026-PL-0001").unwrap(),
        currency: Iso4217Code::new("PLN").unwrap(),
        supplier: polish_party("Acme Sp. z o.o.", "PL5213003700", "Warszawa"),
        customer: polish_party("Beta S.A.", "5260001246", "Kraków"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Korekta wartości usługi konsultingowej".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(1150),
            // Standard Polish VAT rate is 23 %.
            tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(6150),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(6150),
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

/// A multi-line invoice mixing the three live Polish VAT bands on one document:
/// 23 % standard (`S`), 8 % reduced (`R`), and 0 % (`Z`). Poland runs a 23 %
/// standard rate, 8 % and 5 % reduced rates, a 0 % rate, and a "zw" exemption
/// (ustawa o VAT, art. 41 / art. 146ef). The FA(3) `FaWiersz` block must carry
/// each line's own `P_12` rate while `P_13_1`/`P_14_1` aggregate the bases.
fn polish_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pl-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FV-2026-PL-0002").unwrap(),
        currency: Iso4217Code::new("PLN").unwrap(),
        supplier: polish_party("Acme Sp. z o.o.", "PL5252248481", "Warszawa"),
        customer: polish_party("Gamma Sp.j.", "7740001454", "Płock"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Usługi konsultingowe (stawka podstawowa 23%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Wydawnictwo (stawka obniżona 8%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("R".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
            DocumentLine {
                id: "3".to_owned(),
                description: "Eksport towarów (stawka 0%)".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(30000),
                line_extension_amount: amt(30000),
                tax_category: Some("Z".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2300),
                tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "R".to_owned(),
                taxable_amount: amt(20000),
                tax_amount: amt(1600),
                tax_rate: Some(DecimalValue::new(Decimal::new(800, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "Z".to_owned(),
                taxable_amount: amt(30000),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(60000),
            tax_exclusive_amount: amt(60000),
            // 600.00 net + (23.00 + 16.00 + 0.00) VAT = 639.00 gross.
            tax_inclusive_amount: amt(63900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(63900),
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

/// A domestic reverse-charge ("odwrotne obciążenie") / VAT-exempt ("zwolniona,
/// zw") invoice: the supplier charges no VAT and the buyer self-accounts. The
/// FA(3) line carries a 0.00 `P_12` and the totals block reports `P_14_1` =
/// 0.00 (no output VAT). Reference: ustawa o VAT art. 17 ust. 1 pkt 7-8
/// (odwrotne obciążenie) and art. 43 (zwolnienia).
fn polish_reverse_charge_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-pl-e2e-rc-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("FV-2026-PL-0003").unwrap(),
        currency: Iso4217Code::new("PLN").unwrap(),
        supplier: polish_party("Acme Sp. z o.o.", "PL5270103391", "Warszawa"),
        customer: polish_party("Delta Sp. z o.o.", "9512078671", "Gdańsk"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Usługa w odwrotnym obciążeniu (zw)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
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
            // No output VAT: gross == net under reverse charge.
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

/// Pack a serialized FA(3) document plus its KSeF receipt into a verifiable
/// `.ikb` evidence bundle (the step-5 success criterion, factored so each new
/// scenario proves the bundle still verifies).
fn pack_evidence(doc: &CommercialDocument, fa: &str, report: &KsefReport) -> Vec<u8> {
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/fa3.xml".to_owned(), fa.to_owned().into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_fa_xml.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    pack(&EvidenceBundle { manifest, artefacts }).unwrap()
}

#[test]
fn poland_corrective_invoice_maps_to_korekta_and_verifies() {
    // FA(3): a faktura korygująca (correction) carries RodzajFaktury = KOR.
    let doc = polish_corrective_invoice();
    let ctx = Fa3Context {
        data_wytworzenia: "2026-05-28T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let fa = to_fa3_xml(&doc, &ctx).unwrap();

    // The corrective is serialized as KOR, not VAT, and binds the corrective
    // seller's NIP (the PL prefix is stripped by the serializer).
    assert!(
        fa.contains("<RodzajFaktury>KOR</RodzajFaktury>"),
        "corrective invoice must serialize as RodzajFaktury KOR:\n{fa}"
    );
    assert!(fa.contains("<NIP>5213003700</NIP>"));
    assert!(fa.contains("<P_2>FK-2026-PL-0001</P_2>"));
    // 50.00 net at 23 % => 11.50 VAT => 61.50 gross.
    assert!(fa.contains("<P_14_1>11.50</P_14_1>"));
    assert!(fa.contains("<P_15>61.50</P_15>"));

    let report = provider(None)
        .report(&report_request_for(CORRECTIVE_ISSUER_NIP, fa.clone().into_bytes()))
        .unwrap();
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);
    assert!(report.envelope.numer_ksef.starts_with(CORRECTIVE_ISSUER_NIP));

    let ikb = pack_evidence(&doc, &fa, &report);
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "corrective-invoice evidence bundle must verify");
}

#[test]
fn poland_mixed_rate_invoice_emits_per_line_rates_and_verifies() {
    // Three live Polish VAT bands on one FA(3): 23 % / 8 % / 0 %.
    let doc = polish_mixed_rate_invoice();
    let ctx = Fa3Context {
        data_wytworzenia: "2026-05-27T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let fa = to_fa3_xml(&doc, &ctx).unwrap();

    // Each FaWiersz must carry its own P_12 rate.
    assert!(fa.contains("<NrWierszaFa>1</NrWierszaFa>"));
    assert!(fa.contains("<NrWierszaFa>3</NrWierszaFa>"));
    assert!(fa.contains("<P_12>23.00</P_12>"), "missing 23% line rate:\n{fa}");
    assert!(fa.contains("<P_12>8.00</P_12>"), "missing 8% line rate:\n{fa}");
    // A zero VAT rate built from `Decimal::ZERO` (scale 0) renders as the
    // bare `0` — `fmt_amount` calls `round_dp(2).to_string()`, which keeps the
    // source scale for an exact zero. This is the serializer's real behavior.
    assert!(fa.contains("<P_12>0</P_12>"), "missing 0% line rate:\n{fa}");
    // Aggregated base (600.00) and aggregated VAT (23.00 + 16.00 = 39.00).
    assert!(fa.contains("<P_13_1>600.00</P_13_1>"), "wrong aggregated net base:\n{fa}");
    assert!(fa.contains("<P_14_1>39.00</P_14_1>"), "wrong aggregated VAT:\n{fa}");
    assert!(fa.contains("<P_15>639.00</P_15>"), "wrong gross total:\n{fa}");

    let report = provider(None)
        .report(&report_request_for(ISSUER_NIP, fa.clone().into_bytes()))
        .unwrap();
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);

    let ikb = pack_evidence(&doc, &fa, &report);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

#[test]
fn poland_reverse_charge_invoice_has_zero_output_vat_and_verifies() {
    // Odwrotne obciążenie / zwolnienie: no output VAT, gross == net.
    let doc = polish_reverse_charge_invoice();
    let ctx = Fa3Context {
        data_wytworzenia: "2026-05-29T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let fa = to_fa3_xml(&doc, &ctx).unwrap();

    // The line VAT rate comes from `Decimal::ZERO` (scale 0) -> bare `0`,
    // whereas the aggregated `P_14_1` is built from `amt(0)` (scale 2) -> the
    // fixed-scale `0.00`. Both encode "no output VAT".
    assert!(fa.contains("<P_12>0</P_12>"), "reverse-charge line must be 0%:\n{fa}");
    assert!(fa.contains("<P_14_1>0.00</P_14_1>"), "reverse charge must carry no output VAT:\n{fa}");
    // 1000.00 net, no VAT => 1000.00 gross.
    assert!(fa.contains("<P_13_1>1000.00</P_13_1>"));
    assert!(fa.contains("<P_15>1000.00</P_15>"));

    let report = provider(None)
        .report(&report_request_for("5270103391", fa.clone().into_bytes()))
        .unwrap();
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);

    let ikb = pack_evidence(&doc, &fa, &report);
    assert!(verify_packed(&ikb, &VerifyOptions::content_only()).unwrap().ok);
}

#[test]
fn poland_numer_ksef_has_official_reference_shape() {
    // The KSeF reference number (Numer KSeF) is NIP-YYYYMMDD-XXXXXXXXXXXX-XX
    // per the Ministry of Finance KSeF API specification.
    let fa = to_fa3_xml(&polish_invoice(), &Fa3Context::default())
        .unwrap()
        .into_bytes();
    let report = provider(None)
        .report(&report_request(fa))
        .unwrap();
    let numer = &report.envelope.numer_ksef;
    let parts: Vec<&str> = numer.split('-').collect();
    assert_eq!(parts.len(), 4, "Numer KSeF must have 4 dash-separated groups: {numer:?}");
    // Group 1 = the issuer NIP (10 digits).
    assert_eq!(parts[0], ISSUER_NIP);
    assert_eq!(parts[0].len(), 10);
    // Group 2 = an 8-digit YYYYMMDD date.
    assert_eq!(parts[1].len(), 8, "date group must be YYYYMMDD: {numer:?}");
    assert!(parts[1].bytes().all(|b| b.is_ascii_digit()));
    // Group 4 = a 2-char trailing checksum block.
    assert_eq!(parts[3].len(), 2, "trailing block must be 2 chars: {numer:?}");
    // The matching UPO acknowledgement reference is always present.
    assert!(report.envelope.upo_reference.starts_with("upo-"));
}

#[test]
fn poland_rejection_carries_polish_reason_and_no_numer() {
    // KSeF refusal: the receipt is "odrzucona" (rejected) with no binding Numer
    // KSeF, surfaced as an Ok envelope (NOT an Err) per the audit contract.
    let fa = to_fa3_xml(&polish_invoice(), &Fa3Context::default())
        .unwrap()
        .into_bytes();
    let report = provider(Some(KsefAcceptance::Rejected))
        .report(&report_request(fa))
        .unwrap();
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Rejected);
    assert!(report.envelope.numer_ksef.is_empty());
    let reason = report.envelope.reason.as_deref().unwrap();
    assert!(
        reason.contains("odrzucona"),
        "rejection reason should name the Polish 'odrzucona' status, got {reason:?}"
    );
}

#[test]
fn poland_pending_status_is_a_receipt_not_an_error() {
    // KSeF can also return Pending ("przyjęta, oczekuje na weryfikację"): the
    // invoice was received but not yet validated. Like Rejected, it is a
    // receipt kind, not an Err, and carries no binding Numer KSeF yet.
    let fa = to_fa3_xml(&polish_invoice(), &Fa3Context::default())
        .unwrap()
        .into_bytes();
    let report = provider(Some(KsefAcceptance::Pending))
        .report(&report_request(fa))
        .unwrap();
    assert_eq!(report.envelope.acceptance, KsefAcceptance::Pending);
    assert!(!report.envelope.acceptance.is_accepted());
    assert!(report.envelope.numer_ksef.is_empty());
}

#[test]
fn poland_rejects_invalid_issuer_nip() {
    // NIP validation is a pre-wire shape failure (Err), distinct from a KSeF
    // business rejection. Three Polish-specific failure modes:
    let fa = b"<Faktura/>".to_vec();

    // (a) valid 10-digit shape, wrong weighted-modulo-11 check digit.
    let mut req = report_request_for("5842672558", fa.clone());
    assert!(matches!(
        provider(None).report(&req).unwrap_err(),
        KsefReportError::BadNip(_)
    ));

    // (b) the special "check value == 10" case: a NIP whose checksum computes
    // to 10 is invalid by construction (no NIP is issued with check digit 10).
    req = report_request_for("1180048507", fa.clone());
    assert!(matches!(
        provider(None).report(&req).unwrap_err(),
        KsefReportError::BadNip(_)
    ));

    // (c) a NIP still carrying its "PL" VAT-prefix is not a bare 10-digit NIP.
    req = report_request_for("PL5252248481", fa);
    assert!(matches!(
        provider(None).report(&req).unwrap_err(),
        KsefReportError::BadNip(_)
    ));
}

#[test]
fn poland_nip_checksum_accepts_real_and_rejects_corrupted() {
    // The standalone validator enforces the official weights {6,5,7,2,3,4,5,6,7}.
    // Real, checksum-valid Polish NIPs used across the fixtures above:
    for good in [ISSUER_NIP, CORRECTIVE_ISSUER_NIP, "5260001246", "7740001454"] {
        assert!(validate_nip(good).is_ok(), "{good} should pass the NIP checksum");
    }
    // Flip the last digit of a valid NIP -> the checksum no longer matches.
    assert!(validate_nip("5252248480").is_err());
    assert!(validate_nip("1180048507").is_err()); // check value 10
}

#[test]
fn poland_corrective_and_mixed_rate_serialization_is_deterministic() {
    // Determinism must hold across distinct document shapes, not just the
    // ordinary invoice: re-serializing yields byte-identical FA(3).
    let kor = polish_corrective_invoice();
    let kor_ctx = Fa3Context {
        data_wytworzenia: "2026-05-28T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    assert_eq!(
        to_fa3_xml(&kor, &kor_ctx).unwrap(),
        to_fa3_xml(&kor, &kor_ctx).unwrap()
    );

    let multi = polish_mixed_rate_invoice();
    let multi_ctx = Fa3Context {
        data_wytworzenia: "2026-05-27T08:00:00Z".to_owned(),
        system_info: "InvoiceKit".to_owned(),
    };
    let first = to_fa3_xml(&multi, &multi_ctx).unwrap();
    assert_eq!(first, to_fa3_xml(&multi, &multi_ctx).unwrap());
    // And a corrective and an ordinary invoice never collide on RodzajFaktury.
    assert!(first.contains("<RodzajFaktury>VAT</RodzajFaktury>"));
    assert!(to_fa3_xml(&kor, &kor_ctx)
        .unwrap()
        .contains("<RodzajFaktury>KOR</RodzajFaktury>"));
}
