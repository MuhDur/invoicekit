// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Italy SDI offline end-to-end lifecycle (coverage-loop §1 honest bar).
//!
//! Drives the full local-only chain for Italy and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national FatturaPA XML
//! 3. local validate (structural + Italian identity shapes)
//! 4. sign + transmit via the offline `MockSdiReportProvider` (composes
//!    `invoicekit-signer-sdi`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. assert the capability matrix advertises Italy
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! This is the first per-country end-to-end test in the workspace and the
//! reference pattern the flagship fan-out follows. Goldens are hand-rolled
//! (no `insta`/`pretty_assertions`, which would mutate `Cargo.lock`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_it_sdi::{
    to_fattura_pa_xml, ArubaQualifiedCertificate, FatturaPaContext, MockSdiReportProvider,
    SdiEnvironment, SdiReceiptKind, SdiReport, SdiReportProvider, SdiReportRequest, SdiTransport,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;
use std::sync::Arc;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_it_e2e";
const TRACE: &str = "trace_it_e2e";
const CERT_SERIAL: &str = "1234567890ABCDEF";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn italian_party(name: &str, vat: &str, city: &str, province: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Via Roma 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some(province.to_owned()),
            postal_code: "00100".to_owned(),
            country: CountryCode::new("IT").unwrap(),
        },
        contact: None,
    }
}

fn italian_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-it-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("INV-2026-IT-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: italian_party("Acme SRL", "IT12345678901", "Roma", "RM"),
        customer: italian_party("Beta SpA", "IT98765432109", "Milano", "MI"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consulenza & sviluppo software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2200),
            tax_rate: Some(DecimalValue::new(Decimal::new(2200, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(12200),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(12200),
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

fn cert() -> ArubaQualifiedCertificate {
    ArubaQualifiedCertificate {
        serial_number: CERT_SERIAL.to_owned(),
        codice_fiscale: "RSSMRA80A01H501U".to_owned(),
        subject_dn: "CN=Mario Rossi,O=Acme SRL,C=IT".to_owned(),
        certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
    }
}

fn provider(forced: Option<SdiReceiptKind>) -> MockSdiReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [2_u8; 32]));
    let p = MockSdiReportProvider::new(signer);
    match forced {
        Some(kind) => p.with_forced_receipt(kind),
        None => p,
    }
}

fn report_request(fattura_xml: Vec<u8>) -> SdiReportRequest {
    SdiReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: SdiEnvironment::Sandbox,
        issuer_tax_id: "12345678901".to_owned(),
        progressivo_invio: "ABCDE".to_owned(),
        transport: SdiTransport::WebService,
        certificate: cert(),
        fattura_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> sign/transmit -> evidence bundle.
fn run_lifecycle(forced: Option<SdiReceiptKind>) -> (Vec<u8>, SdiReport) {
    // 1. build
    let doc = italian_invoice();

    // 2. serialize -> FatturaPA (progressivo must match the report request)
    let ctx = FatturaPaContext {
        progressivo_invio: "ABCDE".to_owned(),
        codice_destinatario: "0000000".to_owned(),
    };
    let fattura = to_fattura_pa_xml(&doc, &ctx).unwrap();
    // 3. local validate (structural): the national artifact carries the
    // mandatory FatturaPA spine. Reference Schematron stays external (JVM).
    for needle in [
        "<p:FatturaElettronica",
        "<CedentePrestatore>",
        "<CessionarioCommittente>",
        "<TipoDocumento>TD01</TipoDocumento>",
        "<Imposta>22.00</Imposta>",
    ] {
        assert!(fattura.contains(needle), "FatturaPA missing {needle}");
    }

    // 4. sign + transmit (offline mock composing the real SDI signer path)
    let report = provider(forced).report(&report_request(fattura.clone().into_bytes())).unwrap();

    // 5. evidence bundle: canonical doc + national XML + signed XML + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap()).unwrap().into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/fattura.xml".to_owned(), fattura.into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_fattura_xml.clone());
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
fn italy_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(None);

    // Happy path: SDI delivered (RicevutaConsegna).
    assert!(report.envelope.receipt_kind.is_delivered());
    assert!(report.envelope.identificativo_sdi.starts_with("IT"));
    assert_eq!(report.envelope.progressivo_invio, "ABCDE");
    assert!(report.envelope.reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn italy_rejection_still_bundles_and_verifies() {
    // NS (Notifica di Scarto) is a receipt kind, NOT an Err — the audit trail
    // persists the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(Some(SdiReceiptKind::NotificaScarto));
    assert_eq!(report.envelope.receipt_kind, SdiReceiptKind::NotificaScarto);
    assert!(!report.envelope.receipt_kind.is_delivered());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn italy_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(None);
    let (b, _) = run_lifecycle(None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}

#[test]
fn capability_matrix_advertises_italy() {
    let repo_root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root is two levels above the crate dir")
        .to_path_buf();
    let matrix_path = repo_root.join("crates/cli/data/capabilities/matrix.json");
    let raw = std::fs::read_to_string(&matrix_path).expect("read capabilities matrix.json");
    let matrix: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let entries = matrix["entries"].as_array().expect("entries array");
    let has_italy = entries
        .iter()
        .any(|e| e["route_from"] == "IT" && e["route_to"] == "IT");
    assert!(
        has_italy,
        "capability matrix must advertise an IT->IT route so `invoicekit capabilities` answers honestly for Italy"
    );
}
