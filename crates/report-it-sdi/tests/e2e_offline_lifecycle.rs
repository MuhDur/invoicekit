// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// FatturaPA / SDI / AdE / XAdES and the FatturaPA element names (TipoDocumento,
// DatiRiepilogo, AliquotaIVA, ...) trip doc-markdown in test doc-comments; the
// crate's lib.rs suppresses the same lint crate-wide.
#![allow(clippy::doc_markdown)]

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
        invoice_period: None,
        delivery_date: None,
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
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(2200),
            tax_rate: Some(DecimalValue::new(Decimal::new(2200, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
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
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A `TD04` *Nota di Credito* (credit note) referencing the same Italian
/// supplier/customer as [`italian_invoice`]. FatturaPA maps a credit note to
/// `TipoDocumento` `TD04` per the Agenzia delle Entrate technical spec
/// (provvedimento prot. n. 89757/2018, Allegato A, table "Tipi documento").
fn italian_credit_note() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-it-e2e-cn-1").unwrap(),
        document_type: DocumentType::CreditNote,
        issue_date: DateOnly::new("2026-05-28").unwrap(),
        tax_point_date: None,
        // Credit notes carry no DueDate spine in this flow.
        due_date: None,
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("NC-2026-IT-0001").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: italian_party("Acme SRL", "IT12345678901", "Roma", "RM"),
        customer: italian_party("Beta SpA", "IT98765432109", "Milano", "MI"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Storno consulenza (nota di credito)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(5000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(1100),
            tax_rate: Some(DecimalValue::new(Decimal::new(2200, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(6100),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(6100),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A two-line invoice mixing the ordinary 22 % rate (FatturaPA `Natura` empty,
/// `category_code` `S`) with the 10 % reduced rate (`category_code` `R`). Italy
/// runs three reduced VAT rates (4 %, 5 %, 10 %) alongside the 22 % ordinary
/// rate; the 10 % band applies to e.g. certain food and tourism services
/// (Agenzia delle Entrate, "Aliquote IVA").
fn italian_multiline_mixed_rate_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-it-e2e-multi-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IT-0002").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: italian_party("Acme SRL", "IT12345678901", "Roma", "RM"),
        customer: italian_party("Beta SpA", "IT98765432109", "Milano", "MI"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "Consulenza ordinaria 22%".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "Servizio ridotto 10%".to_owned(),
                quantity: DecimalValue::new(Decimal::from(1)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(20000),
                line_extension_amount: amt(20000),
                tax_category: Some("R".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ],
        tax_summary: vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2200),
                tax_rate: Some(DecimalValue::new(Decimal::new(2200, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "R".to_owned(),
                taxable_amount: amt(20000),
                tax_amount: amt(2000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1000, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            tax_inclusive_amount: amt(34200),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(34200),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        meta: DocumentMeta {
            tenant_id: TENANT.to_owned(),
            trace_id: TRACE.to_owned(),
            source_system: Some("e2e".to_owned()),
        },
    })
    .unwrap()
}

/// A domestic reverse-charge ("inversione contabile") invoice: the supplier
/// charges no VAT and the buyer self-accounts. In FatturaPA the line carries a
/// 0 % `AliquotaIVA` and a `Natura` of the N6.x family (e.g. `N6.9` per the
/// Agenzia delle Entrate "Natura operazione" codelist used since FatturaPA
/// v1.2.1). This flow exercises the zero-rate `AliquotaIVA` / `Imposta` path.
fn italian_reverse_charge_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-it-e2e-rc-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-28").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new("INV-2026-IT-0003").unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: italian_party("Acme SRL", "IT12345678901", "Roma", "RM"),
        customer: italian_party("Beta SpA", "IT98765432109", "Milano", "MI"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Prestazioni in reverse charge (N6.9)".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("C62".to_owned()),
            unit_price: amt(100_000),
            line_extension_amount: amt(100_000),
            // "AC" stands for the local reverse-charge tax category here; the
            // serializer resolves AliquotaIVA from the matching summary entry.
            tax_category: Some("AC".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "AC".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(0),
            // Scale-2 zero so AliquotaIVA renders "0.00" (fmt_amount keeps the
            // input scale; a scale-0 Decimal::ZERO would render bare "0").
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
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
///
/// Delegates the serialize/sign/bundle chain to [`bundle_for`] over the canonical
/// [`italian_invoice`], then runs the step-3 structural validation: the national
/// artifact must carry the mandatory FatturaPA spine. Reference Schematron stays
/// external (JVM).
fn run_lifecycle(forced: Option<SdiReceiptKind>) -> (Vec<u8>, SdiReport) {
    let (ikb, fattura, report) = bundle_for(&italian_invoice(), forced);
    for needle in [
        "<p:FatturaElettronica",
        "<CedentePrestatore>",
        "<CessionarioCommittente>",
        "<TipoDocumento>TD01</TipoDocumento>",
        "<Imposta>22.00</Imposta>",
    ] {
        assert!(fattura.contains(needle), "FatturaPA missing {needle}");
    }
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

// ---------------------------------------------------------------------------
// Deepened country-specific scenarios (added on top of the §1 honest bar).
//
// Each scenario grounds an assertion in the real Italian e-invoicing rules
// published by the Agenzia delle Entrate (Sistema di Interscambio / FatturaPA):
//
//   * FatturaPA technical specification & TipoDocumento / Natura codelists —
//     provvedimento del Direttore dell'Agenzia delle Entrate prot. n.
//     89757/2018 and successive updates, https://www.fatturapa.gov.it/
//   * Receipt kinds (RC/NS/MC/NE/MT) — the SDI "ricevute" returned per
//     https://www.agenziaentrate.gov.it/ (Fatturazione elettronica).
// ---------------------------------------------------------------------------

/// Steps 2-5 for an arbitrary document, reusing the same fixed transmission
/// context, signer, and pinned timestamps so output stays byte-stable. Returns
/// `(ikb, fattura_xml, report)`.
fn bundle_for(
    doc: &CommercialDocument,
    forced: Option<SdiReceiptKind>,
) -> (Vec<u8>, String, SdiReport) {
    let ctx = FatturaPaContext {
        progressivo_invio: "ABCDE".to_owned(),
        codice_destinatario: "0000000".to_owned(),
    };
    let fattura = to_fattura_pa_xml(doc, &ctx).unwrap();
    let report = provider(forced)
        .report(&report_request(fattura.clone().into_bytes()))
        .unwrap();

    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/fattura.xml".to_owned(), fattura.clone().into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_fattura_xml.clone());
    artefacts.insert(
        "receipt.json".to_owned(),
        serde_json::to_vec(&report.envelope).unwrap(),
    );
    let manifest = manifest_for(&artefacts, TENANT, TRACE, PINNED_CREATED_AT);
    let bundle = EvidenceBundle { manifest, artefacts };
    let ikb = pack(&bundle).unwrap();
    (ikb, fattura, report)
}

/// A *Nota di Credito* (credit note) must serialize as FatturaPA
/// `TipoDocumento` `TD04`, NOT `TD01`. This is the corrective-document path:
/// the AdE technical spec assigns `TD04` to credit notes (provvedimento prot.
/// n. 89757/2018, Allegato A). The whole offline lifecycle must still deliver
/// and produce a verifiable evidence bundle.
#[test]
fn italy_credit_note_serializes_as_td04_and_bundles() {
    let doc = italian_credit_note();
    let (ikb, fattura, report) = bundle_for(&doc, None);

    assert!(
        fattura.contains("<TipoDocumento>TD04</TipoDocumento>"),
        "a credit note must map to FatturaPA TipoDocumento TD04, got:\n{fattura}"
    );
    assert!(
        !fattura.contains("<TipoDocumento>TD01</TipoDocumento>"),
        "a credit note must not carry the invoice code TD01"
    );
    assert!(
        fattura.contains("<Numero>NC-2026-IT-0001</Numero>"),
        "the credit-note number must appear in DatiGeneraliDocumento"
    );
    // Same 22% band as the original invoice, but on the storno taxable base.
    assert!(fattura.contains("<ImponibileImporto>50.00</ImponibileImporto>"));
    assert!(fattura.contains("<Imposta>11.00</Imposta>"));

    assert!(report.envelope.receipt_kind.is_delivered());
    assert!(report.envelope.identificativo_sdi.starts_with("IT"));
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "credit-note evidence bundle must verify");
}

/// A two-line invoice carrying both the ordinary 22 % rate and the reduced
/// 10 % rate. FatturaPA emits one `DettaglioLinee` per line (each with its own
/// `AliquotaIVA`) and one `DatiRiepilogo` per VAT band. Italy's reduced rates
/// (4 %, 5 %, 10 %) coexist with the 22 % ordinary rate (Agenzia delle Entrate,
/// "Aliquote IVA"); this proves the per-line rate lookup and per-band summary.
#[test]
fn italy_multiline_invoice_emits_per_band_summaries() {
    let doc = italian_multiline_mixed_rate_invoice();
    let (ikb, fattura, report) = bundle_for(&doc, None);

    // Two distinct line items in document order.
    assert!(fattura.contains("<NumeroLinea>1</NumeroLinea>"));
    assert!(fattura.contains("<NumeroLinea>2</NumeroLinea>"));
    assert!(fattura.contains("<Descrizione>Consulenza ordinaria 22%</Descrizione>"));
    assert!(fattura.contains("<Descrizione>Servizio ridotto 10%</Descrizione>"));

    // Each line resolved its own band rate from the matching summary entry.
    assert!(fattura.contains("<AliquotaIVA>22.00</AliquotaIVA>"));
    assert!(fattura.contains("<AliquotaIVA>10.00</AliquotaIVA>"));

    // Two DatiRiepilogo blocks: 22% on 100.00 -> 22.00, 10% on 200.00 -> 20.00.
    assert_eq!(
        fattura.matches("<DatiRiepilogo>").count(),
        2,
        "a mixed-rate invoice must emit one DatiRiepilogo per VAT band"
    );
    assert!(fattura.contains("<ImponibileImporto>200.00</ImponibileImporto>"));
    assert!(fattura.contains("<Imposta>20.00</Imposta>"));

    assert!(report.envelope.receipt_kind.is_delivered());
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "multi-line evidence bundle must verify");
}

/// Domestic reverse charge ("inversione contabile"): the supplier charges no
/// VAT (`AliquotaIVA` 0.00, `Imposta` 0.00) and the taxable base equals the
/// payable total. In FatturaPA such a line carries a `Natura` code of the N6.x
/// family (Agenzia delle Entrate "Natura operazione" codelist). This exercises
/// the zero-rate path end to end.
#[test]
fn italy_reverse_charge_invoice_is_zero_rated() {
    let doc = italian_reverse_charge_invoice();
    let (ikb, fattura, report) = bundle_for(&doc, None);

    assert!(
        fattura.contains("<AliquotaIVA>0.00</AliquotaIVA>"),
        "a reverse-charge line must carry a 0.00 AliquotaIVA, got:\n{fattura}"
    );
    assert!(
        fattura.contains("<Imposta>0.00</Imposta>"),
        "reverse charge means zero VAT charged by the supplier"
    );
    // Taxable base equals the payable total (no VAT added).
    assert!(fattura.contains("<ImponibileImporto>1000.00</ImponibileImporto>"));
    assert!(fattura.contains("<PrezzoTotale>1000.00</PrezzoTotale>"));
    // The 22% band from the ordinary invoice must NOT appear here.
    assert!(!fattura.contains("<AliquotaIVA>22.00</AliquotaIVA>"));

    assert!(report.envelope.receipt_kind.is_delivered());
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "reverse-charge evidence bundle must verify");
}

/// SDI refusal path with a corrective document. A *Notifica di Scarto* (NS) is
/// the AdE receipt class for a rejected transmission (schema / business-rule
/// failure). The contract is: NS is a receipt **kind** carried inside an `Ok`
/// envelope with a populated `reason`, never an `Err`. The audit trail (and its
/// evidence bundle) must still be produced and verify. The signed XAdES wrapper
/// is emitted regardless of the SDI verdict.
#[test]
fn italy_credit_note_scarto_is_receipt_not_error() {
    let doc = italian_credit_note();
    let (ikb, fattura, report) = bundle_for(&doc, Some(SdiReceiptKind::NotificaScarto));

    assert_eq!(report.envelope.receipt_kind, SdiReceiptKind::NotificaScarto);
    assert!(!report.envelope.receipt_kind.is_delivered());
    assert_eq!(
        report.envelope.reason.as_deref(),
        Some("SDI rejected the invoice (Notifica di Scarto)"),
        "an NS receipt must carry the Scarto reason text"
    );
    // The corrective TD04 payload was still signed and wrapped.
    assert!(fattura.contains("<TipoDocumento>TD04</TipoDocumento>"));
    assert!(report.signed_fattura_xml.starts_with(b"<XAdES-stub>"));

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejected credit-note bundle must still verify");
}

/// SDI can also report a *Mancata Consegna* (MC): the invoice was accepted by
/// SDI but could not be delivered to the buyer's channel (e.g. a full PEC
/// mailbox). Per AdE this is still a non-delivered outcome that is NOT a
/// rejection — `is_delivered()` is false yet, unlike NS, the report carries no
/// rejection `reason`. The lifecycle must surface it as an `Ok` receipt and the
/// bundle must verify.
#[test]
fn italy_mancata_consegna_is_non_delivered_without_rejection_reason() {
    let doc = italian_invoice();
    let (ikb, _fattura, report) = bundle_for(&doc, Some(SdiReceiptKind::MancataConsegna));

    assert_eq!(report.envelope.receipt_kind, SdiReceiptKind::MancataConsegna);
    assert!(
        !report.envelope.receipt_kind.is_delivered(),
        "Mancata Consegna is a non-delivered outcome"
    );
    assert!(
        report.envelope.reason.is_none(),
        "MC is not a Scarto, so it carries no rejection reason"
    );
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "mancata-consegna bundle must verify");
}

/// A foreign (non-Italian) issuer identity must be refused before the wire.
/// SDI requires the transmitter to carry an 11-digit Partita IVA or a 16-char
/// Codice Fiscale (FatturaPA `IdFiscaleIVA` / `CodiceFiscale`). A French SIREN
/// passed as the issuer tax id matches neither shape and is a pre-wire `Err`
/// (`BadTaxId`) — distinct from an SDI rejection, which would be an NS receipt.
#[test]
fn italy_rejects_foreign_issuer_identity_pre_wire() {
    use invoicekit_report_it_sdi::SdiReportError;

    let doc = italian_invoice();
    let ctx = FatturaPaContext {
        progressivo_invio: "ABCDE".to_owned(),
        codice_destinatario: "0000000".to_owned(),
    };
    let fattura = to_fattura_pa_xml(&doc, &ctx).unwrap().into_bytes();

    let mut req = report_request(fattura);
    // A French SIREN: 9 digits, neither an 11-digit P.IVA nor a 16-char CF.
    req.issuer_tax_id = "552100554".to_owned();

    let err = provider(None).report(&req).unwrap_err();
    assert!(
        matches!(err, SdiReportError::BadTaxId(_)),
        "a foreign issuer id must be a pre-wire BadTaxId Err, not an SDI receipt: {err:?}"
    );
}

/// The full multi-line lifecycle (build -> FatturaPA -> sign -> bundle) must be
/// byte-identical across runs. Determinism is a load-bearing property for the
/// evidence bundle's content address; the per-line ordering and per-band
/// summary order must not vary between runs.
#[test]
fn italy_multiline_lifecycle_is_byte_deterministic() {
    let doc = italian_multiline_mixed_rate_invoice();
    let (a, fattura_a, _) = bundle_for(&doc, None);
    let (b, fattura_b, _) = bundle_for(&doc, None);
    assert_eq!(fattura_a, fattura_b, "FatturaPA serialization must be stable");
    assert_eq!(a, b, "the whole multi-line lifecycle must be byte-stable");
}
