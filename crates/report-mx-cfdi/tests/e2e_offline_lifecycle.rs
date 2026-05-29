// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Mexico CFDI 4.0 offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Mexico and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national CFDI 4.0 XML (`cfdi:Comprobante`)
//! 3. local validate (structural + Mexican identity shapes: RFC, Folio Fiscal)
//! 4. seal + timbrar via the offline `MockCfdiReportProvider` (composes
//!    `invoicekit-signer-cfdi`, the PAC timbrado path)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. rejection path: a PAC *rechazo* is a receipt status, not an `Err`
//! 7. determinism: serialize twice and pack twice -> byte-identical
//!
//! This mirrors the Italy SDI reference E2E. Goldens are hand-rolled (no
//! `insta`/`pretty_assertions`, which would mutate `Cargo.lock`), and the
//! capability matrix is populated centrally — this test does not assert it.

use std::collections::BTreeMap;
use std::sync::Arc;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_mx_cfdi::{
    to_cfdi_xml, validate_folio_fiscal, validate_rfc, CfdiComprobanteKind, CfdiContext,
    CfdiEnvironment, CfdiReport, CfdiReportProvider, CfdiReportRequest, MockCfdiReportProvider,
    TimbradoStatus,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_cfdi::CertificadoSelloDigital;
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_mx_e2e";
const TRACE: &str = "trace_mx_e2e";
const ISSUER_RFC: &str = "CAZ010101AAA";
const CSD_SERIAL: &str = "30001000000400002434";

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn mexican_party(name: &str, rfc: &str, city: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "rfc".to_owned(),
            value: rfc.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Avenida Reforma 1".to_owned()],
            city: city.to_owned(),
            subdivision: Some("CMX".to_owned()),
            postal_code: "06600".to_owned(),
            country: CountryCode::new("MX").unwrap(),
        },
        contact: None,
    }
}

fn mexican_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-mx-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("FAC-2026-MX-0001").unwrap(),
        currency: Iso4217Code::new("MXN").unwrap(),
        supplier: mexican_party("Comercializadora Azteca SA de CV", ISSUER_RFC, "Ciudad de Mexico"),
        customer: mexican_party("Distribuidora Maya SA de CV", "DMA020202BBB", "Monterrey"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Servicios de consultoria & desarrollo".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("E48".to_owned()),
            unit_price: amt(50000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(100_000),
            tax_amount: amt(16000),
            tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(100_000),
            tax_exclusive_amount: amt(100_000),
            tax_inclusive_amount: amt(116_000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(116_000),
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

fn csd() -> CertificadoSelloDigital {
    CertificadoSelloDigital {
        serial_number: CSD_SERIAL.to_owned(),
        rfc: ISSUER_RFC.to_owned(),
        not_before: "2026-01-01T00:00:00Z".to_owned(),
        not_after: "2027-12-31T23:59:59Z".to_owned(),
        certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
    }
}

fn provider(reject: bool) -> MockCfdiReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CSD_SERIAL, [4_u8; 32]));
    let p = MockCfdiReportProvider::new(signer);
    if reject {
        p.with_rejection()
    } else {
        p
    }
}

fn report_request(cfdi_xml: Vec<u8>) -> CfdiReportRequest {
    CfdiReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: CfdiEnvironment::Sandbox,
        issuer_rfc: ISSUER_RFC.to_owned(),
        kind: CfdiComprobanteKind::Ingreso,
        csd: csd(),
        cfdi_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> seal/timbrar -> evidence bundle.
fn run_lifecycle(reject: bool) -> (Vec<u8>, CfdiReport) {
    // 1. build
    let doc = mexican_invoice();

    // 2. serialize -> CFDI 4.0
    let ctx = CfdiContext {
        lugar_expedicion: "06600".to_owned(),
        ..CfdiContext::default()
    };
    let cfdi = to_cfdi_xml(&doc, &ctx).unwrap();

    // 3. local validate (structural + Mexican identity shapes). Reference XSD +
    // cadena-original XSLT validation stays external (JVM).
    validate_rfc(ISSUER_RFC).unwrap();
    for needle in [
        "<cfdi:Comprobante",
        "Version=\"4.0\"",
        "TipoDeComprobante=\"I\"",
        "<cfdi:Emisor",
        "<cfdi:Receptor",
        "<cfdi:Conceptos>",
        "<cfdi:Impuestos",
        "Impuesto=\"002\"",
    ] {
        assert!(cfdi.contains(needle), "CFDI missing {needle}");
    }

    // 4. seal + timbrar (offline mock composing the real PAC signer path)
    let report = provider(reject)
        .report(&report_request(cfdi.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national XML + stamped XML + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/cfdi.xml".to_owned(), cfdi.into_bytes());
    artefacts.insert("timbrado.xml".to_owned(), report.timbrado_xml.clone());
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
fn mexico_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(false);

    // Happy path: the PAC stamped the CFDI (timbrado).
    assert!(report.envelope.status.is_stamped());
    assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
    validate_folio_fiscal(&report.envelope.folio_fiscal).unwrap();
    assert!(report.envelope.reason.is_none());
    assert!(report.envelope.sello_sat.is_some());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn mexico_rejection_still_bundles_and_verifies() {
    // A PAC rechazo is a receipt status, NOT an Err — the audit trail persists
    // the rejection and the bundle still verifies.
    let (ikb, report) = run_lifecycle(true);
    assert_eq!(report.envelope.status, TimbradoStatus::Rechazado);
    assert!(!report.envelope.status.is_stamped());
    assert!(report.envelope.reason.is_some());
    assert!(report.envelope.folio_fiscal.is_empty());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "rejection-path evidence bundle must verify");
}

#[test]
fn mexico_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(false);
    let (b, _) = run_lifecycle(false);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}
