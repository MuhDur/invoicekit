// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Brazil NF-e offline end-to-end lifecycle (coverage-loop honest bar).
//!
//! Drives the full local-only chain for Brazil and proves it deterministically:
//!
//! 1. build a canonical `CommercialDocument` (IR)
//! 2. serialize -> national infNFe XML (`ide`/`emit`/`dest`/`det`/`total`)
//! 3. local validate (structural + CNPJ/CPF mod-11 identity shapes)
//! 4. sign + transmit via the offline `MockNfeReportProvider` (composes
//!    `invoicekit-signer-nfe`)
//! 5. assemble a `.ikb` evidence bundle and `verify` it (exit 0 == report.ok)
//! 6. determinism: serialize twice and pack twice -> byte-identical
//!
//! A SEFAZ denial (Uso Denegado, cStat 110) is a *receipt status*, not an
//! `Err`: the audit trail persists the denial and the bundle still verifies.
//! Goldens are hand-rolled (no `insta`/`pretty_assertions`, which would mutate
//! `Cargo.lock`). The capability matrix is populated centrally, so this test
//! does not assert matrix presence.

use std::collections::BTreeMap;
use std::sync::Arc;

use invoicekit_canonical::canonicalize_value;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId,
    DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal, Party,
    PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use invoicekit_report_br_nfe::{
    to_inf_nfe_xml, MockNfeReportProvider, NfeContext, NfeReport, NfeReportEnvironment,
    NfeReportProvider, NfeReportRequest, NfeUf,
};
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_nfe::IcpBrasilCertificate;
use invoicekit_verify::{verify_packed, VerifyOptions};
use rust_decimal::Decimal;

const PINNED_CREATED_AT: &str = "2026-07-01T00:00:00Z";
const TENANT: &str = "tenant_br_e2e";
const TRACE: &str = "trace_br_e2e";
const CERT_SERIAL: &str = "ABCDEF1234567890";
const ISSUER_CNPJ: &str = "11222333000181";
const N_NF: u64 = 4242;

fn amt(minor: i64) -> DecimalValue {
    DecimalValue::new(Decimal::new(minor, 2))
}

fn br_party(name: &str, cnpj: &str, city: &str, uf: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "cnpj".to_owned(),
            value: cnpj.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Av. Paulista 1000".to_owned()],
            city: city.to_owned(),
            subdivision: Some(uf.to_owned()),
            postal_code: "01310-100".to_owned(),
            country: CountryCode::new("BR").unwrap(),
        },
        contact: None,
    }
}

fn brazil_invoice() -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-br-e2e-1").unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("NF-2026-BR-0001").unwrap(),
        currency: Iso4217Code::new("BRL").unwrap(),
        supplier: br_party("Acme Comercio LTDA", ISSUER_CNPJ, "Sao Paulo", "SP"),
        customer: br_party("Beta Servicos LTDA", "11444777000161", "Rio de Janeiro", "RJ"),
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "PROD-1".to_owned(),
            description: "Servico & consultoria de software".to_owned(),
            quantity: DecimalValue::new(Decimal::from(2)),
            unit_code: Some("UN".to_owned()),
            unit_price: amt(5000),
            line_extension_amount: amt(10000),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(1800),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(11800),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11800),
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

fn cert() -> IcpBrasilCertificate {
    IcpBrasilCertificate {
        serial_number: CERT_SERIAL.to_owned(),
        cnpj: ISSUER_CNPJ.to_owned(),
        subject_dn: "CN=Acme Comercio LTDA,O=Acme,C=BR".to_owned(),
        certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
    }
}

fn provider(forced_c_stat: Option<u32>) -> MockNfeReportProvider {
    let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [3_u8; 32]));
    let p = MockNfeReportProvider::new(signer, NfeReportEnvironment::Homologacao);
    match forced_c_stat {
        Some(c_stat) => p.with_forced_c_stat(c_stat),
        None => p,
    }
}

fn report_request(inf_nfe_xml: Vec<u8>) -> NfeReportRequest {
    NfeReportRequest {
        tenant_id: TENANT.to_owned(),
        environment: NfeReportEnvironment::Homologacao,
        issuer_tax_id: ISSUER_CNPJ.to_owned(),
        uf: NfeUf::Sp,
        n_nf: N_NF,
        certificate: cert(),
        inf_nfe_xml,
    }
}

/// Steps 1-5: build -> serialize -> validate -> sign/transmit -> evidence bundle.
fn run_lifecycle(forced_c_stat: Option<u32>) -> (Vec<u8>, NfeReport) {
    // 1. build
    let doc = brazil_invoice();

    // 2. serialize -> infNFe (nNF must match the report request)
    let ctx = NfeContext {
        uf: NfeUf::Sp,
        serie: 1,
        n_nf: N_NF,
        tp_nf: 1,
        nat_op: "Venda de mercadoria".to_owned(),
    };
    let inf_nfe = to_inf_nfe_xml(&doc, &ctx).unwrap();

    // 3. local validate (structural): the national artifact carries the
    // mandatory NF-e spine. Reference SEFAZ schema stays an external backend.
    for needle in [
        "<NFe xmlns=\"http://www.portalfiscal.inf.br/nfe\">",
        "<infNFe versao=\"4.00\" Id=\"NFe",
        "<ide>",
        "<emit>",
        "<CNPJ>11222333000181</CNPJ>",
        "<dest>",
        "<det nItem=\"1\">",
        "<total>",
        "<vNF>118.00</vNF>",
    ] {
        assert!(inf_nfe.contains(needle), "infNFe missing {needle}");
    }

    // 4. sign + transmit (offline mock composing the real NF-e signer path)
    let report = provider(forced_c_stat)
        .report(&report_request(inf_nfe.clone().into_bytes()))
        .unwrap();

    // 5. evidence bundle: canonical doc + national XML + signed XML + receipt
    let canonical = canonicalize_value(&doc.to_value().unwrap())
        .unwrap()
        .into_bytes();
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    artefacts.insert("canonical.json".to_owned(), canonical);
    artefacts.insert("formats/nfe.xml".to_owned(), inf_nfe.into_bytes());
    artefacts.insert("signed.xml".to_owned(), report.signed_nfe_xml.clone());
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
fn brazil_offline_lifecycle_produces_verifiable_evidence() {
    let (ikb, report) = run_lifecycle(None);

    // Happy path: SEFAZ authorised (cStat 100).
    assert!(report.envelope.is_authorized());
    assert_eq!(report.envelope.c_stat, 100);
    assert_eq!(report.envelope.chave_acesso.len(), 44);
    assert!(report.envelope.protocolo_autorizacao.starts_with("135"));
    assert_eq!(report.envelope.uf, NfeUf::Sp);
    assert!(report.envelope.reason.is_none());

    // Step 5 success criterion: the bundle verifies (exit 0 == report.ok).
    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "evidence bundle must verify");
}

#[test]
fn brazil_denial_still_bundles_and_verifies() {
    // cStat 110 (Uso Denegado) is a receipt status, NOT an Err — the audit
    // trail persists the denial and the bundle still verifies.
    let (ikb, report) = run_lifecycle(Some(110));
    assert_eq!(report.envelope.c_stat, 110);
    assert!(!report.envelope.is_authorized());
    assert!(report.envelope.reason.is_some());

    let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
    assert!(verify.ok, "denial-path evidence bundle must verify");
}

#[test]
fn brazil_lifecycle_is_byte_deterministic() {
    let (a, _) = run_lifecycle(None);
    let (b, _) = run_lifecycle(None);
    assert_eq!(a, b, "the whole offline lifecycle must be byte-stable");
}
