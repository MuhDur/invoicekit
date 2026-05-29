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

// ---------------------------------------------------------------------------
// Country-specific deepening scenarios.
//
// Grounded in the NF-e **Manual de Orientacao ao Contribuinte (MOC)**, the
// national specification published by the ENCAT working group under the
// Receita Federal / SEFAZ "Portal da Nota Fiscal Eletronica":
//   https://www.nfe.fazenda.gov.br/portal/principal.aspx
// (technical packages "Pacote de Liberacao" / "Schemas XML"). The MOC fixes:
//   * layout version 4.00 and `mod` = 55 (NF-e modelo 55);
//   * `finNFe`: 1 = NF-e normal, 4 = devolucao/retorno de mercadoria;
//   * the 44-digit `chave de acesso` layout (cUF + AAMM + CNPJ + mod + serie
//     + nNF + tpEmis + cNF + cDV);
//   * `cStat` rejection codes: 110 Uso Denegado, 205 NF-e denegada na base de
//     dados, 215 Falha no schema XML, 539 Duplicidade de NF-e.
// CNPJ/CPF mod-11 check digits follow the Receita Federal algorithm. All
// fixtures below are hand-built synthetic taxpayers with genuinely-computed
// check digits; no copyrighted regulator XML is vendored.
// ---------------------------------------------------------------------------

/// A Brazilian customer keyed by CPF (natural person / final consumer), as
/// opposed to the CNPJ companies in [`br_party`]. Used to prove the B2C
/// destinatario path emits `<CPF>` rather than `<CNPJ>` in `<dest>`.
fn br_consumer_cpf(name: &str, cpf: &str, city: &str, uf: &str) -> Party {
    Party {
        id: Some(name.to_lowercase().replace(' ', "-")),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "cpf".to_owned(),
            value: cpf.to_owned(),
        }],
        address: PostalAddress {
            lines: vec!["Rua das Flores 42".to_owned()],
            city: city.to_owned(),
            subdivision: Some(uf.to_owned()),
            postal_code: "30110-013".to_owned(),
            country: CountryCode::new("BR").unwrap(),
        },
        contact: None,
    }
}

/// Build a single-line document parameterised by type, line, tax summary and
/// monetary total, reusing the canonical SP supplier from [`br_party`]. Keeps
/// each scenario focused on the one variation under test.
fn doc_with(
    document_type: DocumentType,
    customer: Party,
    lines: Vec<DocumentLine>,
    tax_summary: Vec<TaxCategorySummary>,
    monetary_total: MonetaryTotal,
) -> CommercialDocument {
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::default(),
        id: DocumentId::new("doc-br-scn-1").unwrap(),
        document_type,
        issue_date: DateOnly::new("2026-05-26").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25").unwrap()),
        document_number: DocumentNumber::new("NF-2026-BR-SCN").unwrap(),
        currency: Iso4217Code::new("BRL").unwrap(),
        supplier: br_party("Acme Comercio LTDA", ISSUER_CNPJ, "Sao Paulo", "SP"),
        customer,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines,
        tax_summary,
        monetary_total,
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

fn line(id: &str, description: &str, qty: i64, unit_minor: i64, total_minor: i64) -> DocumentLine {
    DocumentLine {
        id: id.to_owned(),
        description: description.to_owned(),
        quantity: DecimalValue::new(Decimal::from(qty)),
        unit_code: Some("UN".to_owned()),
        unit_price: amt(unit_minor),
        line_extension_amount: amt(total_minor),
        tax_category: Some("S".to_owned()),
        extensions: Vec::new(),
    }
}

/// **Devolucao (credit note) -> finNFe = 4.** Per the MOC, a return of goods
/// is an NF-e with `finNFe` = 4 (devolucao/retorno). InvoiceKit maps the IR
/// `CreditNote` document type onto that code. The XML must still carry the
/// full NF-e 4.00 spine and a 44-digit chave de acesso, and SEFAZ must
/// authorise it like any other NF-e.
#[test]
fn brazil_credit_note_serializes_as_devolucao_fin_nfe_4() {
    let doc = doc_with(
        DocumentType::CreditNote,
        br_party("Beta Servicos LTDA", "11444777000161", "Rio de Janeiro", "RJ"),
        vec![line("PROD-RET", "Devolucao de mercadoria", 1, 5000, 5000)],
        vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(5000),
            tax_amount: amt(900),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
        }],
        MonetaryTotal {
            line_extension_amount: amt(5000),
            tax_exclusive_amount: amt(5000),
            tax_inclusive_amount: amt(5900),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(5900),
        },
    );
    let ctx = NfeContext {
        uf: NfeUf::Sp,
        serie: 1,
        n_nf: N_NF,
        tp_nf: 0, // entrada (a devolucao re-enters the issuer's stock)
        nat_op: "Devolucao de venda".to_owned(),
    };
    let xml = to_inf_nfe_xml(&doc, &ctx).unwrap();
    assert!(
        xml.contains("<finNFe>4</finNFe>"),
        "credit note must map to finNFe 4 (devolucao):\n{xml}"
    );
    assert!(xml.contains("<tpNF>0</tpNF>"), "devolucao is an entrada (tpNF 0)");
    assert!(xml.contains("<natOp>Devolucao de venda</natOp>"));
    assert!(xml.contains("<vNF>59.00</vNF>"));

    let report = provider(None)
        .report(&report_request(xml.into_bytes()))
        .unwrap();
    assert!(report.envelope.is_authorized());
    assert_eq!(report.envelope.chave_acesso.len(), 44);
}

/// **Debit note has no NF-e shape.** Per the crate's `finNFe` mapping (MOC has
/// no `finNFe` for a standalone debit note), `DocumentType::DebitNote` is
/// rejected pre-wire with `UnsupportedDocumentType` — a serialization `Err`,
/// never a SEFAZ receipt.
#[test]
fn brazil_debit_note_is_unsupported_document_type() {
    let doc = doc_with(
        DocumentType::DebitNote,
        br_party("Beta Servicos LTDA", "11444777000161", "Rio de Janeiro", "RJ"),
        vec![line("PROD-DEB", "Ajuste", 1, 1000, 1000)],
        vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(1000),
            tax_amount: amt(0),
            tax_rate: None,
        }],
        MonetaryTotal {
            line_extension_amount: amt(1000),
            tax_exclusive_amount: amt(1000),
            tax_inclusive_amount: amt(1000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(1000),
        },
    );
    let err = to_inf_nfe_xml(&doc, &NfeContext::default()).unwrap_err();
    assert!(
        matches!(
            err,
            invoicekit_report_br_nfe::InfNfeError::UnsupportedDocumentType(DocumentType::DebitNote)
        ),
        "debit note must be UnsupportedDocumentType, got {err:?}"
    );
}

/// **Multi-line NF-e -> one `<det nItem="N">` per line.** Per the MOC, each
/// item is a `det` element numbered by the `nItem` attribute (1..=990), and
/// `<total><ICMSTot>` aggregates across lines. This proves the serializer
/// emits a stable, 1-based `det` sequence and that the per-line products and
/// the aggregated `vProd` are exact.
#[test]
fn brazil_multi_line_invoice_numbers_det_and_aggregates_total() {
    let doc = doc_with(
        DocumentType::Invoice,
        br_party("Beta Servicos LTDA", "11444777000161", "Rio de Janeiro", "RJ"),
        vec![
            line("PROD-1", "Teclado", 2, 5000, 10000),
            line("PROD-2", "Mouse", 3, 2000, 6000),
            line("PROD-3", "Monitor", 1, 40000, 40000),
        ],
        vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(56000),
            tax_amount: amt(10080),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
        }],
        MonetaryTotal {
            line_extension_amount: amt(56000),
            tax_exclusive_amount: amt(56000),
            tax_inclusive_amount: amt(66080),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(66080),
        },
    );
    let xml = to_inf_nfe_xml(&doc, &NfeContext::default()).unwrap();
    assert!(xml.contains("<det nItem=\"1\">"));
    assert!(xml.contains("<det nItem=\"2\">"));
    assert!(xml.contains("<det nItem=\"3\">"));
    assert!(!xml.contains("<det nItem=\"4\">"), "only three lines exist");
    // det blocks must appear in document order.
    let pos1 = xml.find("<det nItem=\"1\">").unwrap();
    let pos2 = xml.find("<det nItem=\"2\">").unwrap();
    let pos3 = xml.find("<det nItem=\"3\">").unwrap();
    assert!(pos1 < pos2 && pos2 < pos3, "det order must follow line order");
    assert!(xml.contains("<xProd>Monitor</xProd>"));
    // ICMSTot aggregates across all three lines.
    assert!(xml.contains("<vProd>560.00</vProd>"));
    assert!(xml.contains("<vICMS>100.80</vICMS>"));
    assert!(xml.contains("<vNF>660.80</vNF>"));
}

/// **B2C consumer with a CPF destinatario.** When the buyer is a natural
/// person (final consumer), the NF-e `<dest>` carries a `<CPF>` (11 digits)
/// rather than a `<CNPJ>`. The synthetic CPF 111.444.777-35 has genuine
/// Receita Federal mod-11 check digits.
#[test]
fn brazil_consumer_recipient_emits_cpf_not_cnpj() {
    let consumer_cpf = "11144477735";
    // Sanity: the fixture is a *valid* CPF (real check digits), so it is not
    // a placeholder that would slip a malformed id into the wire.
    assert!(invoicekit_report_br_nfe::validate_cpf(consumer_cpf).is_ok());

    let doc = doc_with(
        DocumentType::Invoice,
        br_consumer_cpf("Maria Consumidora", consumer_cpf, "Belo Horizonte", "MG"),
        vec![line("PROD-1", "Cafeteira", 1, 15000, 15000)],
        vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(15000),
            tax_amount: amt(2700),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
        }],
        MonetaryTotal {
            line_extension_amount: amt(15000),
            tax_exclusive_amount: amt(15000),
            tax_inclusive_amount: amt(17700),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(17700),
        },
    );
    let xml = to_inf_nfe_xml(&doc, &NfeContext::default()).unwrap();
    // The emitente is still a CNPJ; the destinatario is a CPF.
    assert!(xml.contains("<emit>"));
    assert!(xml.contains("<CNPJ>11222333000181</CNPJ>"), "emit keeps its CNPJ");
    // The consumer's CPF appears inside <dest>, and there is no second <CNPJ>.
    let dest_start = xml.find("<dest>").expect("dest block present");
    let dest_block = &xml[dest_start..];
    assert!(
        dest_block.contains("<CPF>11144477735</CPF>"),
        "dest must carry the consumer CPF:\n{dest_block}"
    );
    assert!(
        !dest_block.contains("<CNPJ>"),
        "a CPF consumer must not get a CNPJ element"
    );
    assert!(dest_block.contains("<UF>MG</UF>"), "Minas Gerais UF in dest");
}

/// **ICMS-isento / zero-rated line.** A tax-exempt operation (e.g. an item
/// under ICMS isencao) still serialises a full NF-e but with a zero `vICMS`
/// and `vBC`, while `vProd` and `vNF` carry the merchandise value. The
/// scale-2 formatting must produce `0.00`, never an empty or `0` field.
#[test]
fn brazil_tax_exempt_line_serializes_zero_icms() {
    let doc = doc_with(
        DocumentType::Invoice,
        br_party("Beta Servicos LTDA", "11444777000161", "Rio de Janeiro", "RJ"),
        vec![line("PROD-ISENTO", "Livro (isento de ICMS)", 4, 2500, 10000)],
        vec![TaxCategorySummary {
            // ICMS isento: taxable base and tax both zero.
            category_code: "S".to_owned(),
            taxable_amount: amt(0),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        MonetaryTotal {
            line_extension_amount: amt(10000),
            tax_exclusive_amount: amt(10000),
            tax_inclusive_amount: amt(10000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(10000),
        },
    );
    let xml = to_inf_nfe_xml(&doc, &NfeContext::default()).unwrap();
    assert!(xml.contains("<vBC>0.00</vBC>"), "exempt base must be 0.00:\n{xml}");
    assert!(xml.contains("<vICMS>0.00</vICMS>"), "exempt ICMS must be 0.00");
    // The goods value and the invoice total are unaffected by the exemption.
    assert!(xml.contains("<vProd>100.00</vProd>"));
    assert!(xml.contains("<vNF>100.00</vNF>"));

    // It still clears SEFAZ end to end.
    let report = provider(None)
        .report(&report_request(xml.into_bytes()))
        .unwrap();
    assert!(report.envelope.is_authorized());
}

/// **SEFAZ rejection codes are receipt statuses, not `Err`.** The MOC defines
/// distinct `cStat` rejection codes; the existing suite covers only 110. This
/// drives 205 (denegada na base de dados), 215 (falha no schema) and 539
/// (duplicidade), asserting the typed `NfeStatus` mapping and that each is an
/// `Ok` envelope whose bundle still verifies.
#[test]
fn brazil_distinct_rejection_cstats_are_ok_receipts() {
    use invoicekit_signer_nfe::NfeStatus;

    for (c_stat, expected_status, motivo) in [
        (205_u32, NfeStatus::DeniedInDatabase, "denegada na base de dados"),
        (215_u32, NfeStatus::SchemaFailure, "Falha no schema XML"),
        (539_u32, NfeStatus::Duplicate, "Duplicidade de NF-e"),
    ] {
        let (ikb, report) = run_lifecycle(Some(c_stat));
        assert_eq!(report.envelope.c_stat, c_stat, "echoed cStat");
        assert_eq!(report.envelope.status, expected_status, "typed status for {c_stat}");
        assert!(!report.envelope.is_authorized(), "{c_stat} is not authorized");
        assert!(report.envelope.reason.is_some(), "{c_stat} carries a denial reason");
        assert!(
            report.envelope.status_descricao.contains(motivo),
            "cStat {c_stat} xMotivo should mention {motivo:?}, got {:?}",
            report.envelope.status_descricao
        );
        // Even a rejected NF-e leaves a verifiable audit trail.
        let verify = verify_packed(&ikb, &VerifyOptions::content_only()).unwrap();
        assert!(verify.ok, "rejection-path bundle ({c_stat}) must verify");
    }
}

/// **Invalid issuer CNPJ is rejected pre-wire as `BadTaxId`.** A 14-digit
/// string with a wrong final mod-11 check digit is structurally a CNPJ but
/// fails the Receita Federal check, so the report provider must refuse it as a
/// shape error (`Err`), never submit it to SEFAZ.
#[test]
fn brazil_invalid_issuer_cnpj_check_digit_is_rejected() {
    use invoicekit_report_br_nfe::NfeReportError;

    let xml = to_inf_nfe_xml(&brazil_invoice(), &NfeContext::default())
        .unwrap()
        .into_bytes();
    let mut req = report_request(xml);
    // 11222333000181 is valid; flipping the last check digit to 0 breaks it.
    req.issuer_tax_id = "11222333000180".to_owned();
    let err = provider(None).report(&req).unwrap_err();
    assert!(
        matches!(err, NfeReportError::BadTaxId(_)),
        "wrong CNPJ check digit must be BadTaxId, got {err:?}"
    );
}

/// **Foreign (exterior) recipient -> UF "EX".** An export NF-e to a non-Brazil
/// buyer carries the special UF code `EX` for the destinatario (the MOC's
/// exterior marker), even though the emitente stays domestic. Proves the
/// country-aware UF fallback in `<enderDest>`.
#[test]
fn brazil_foreign_recipient_uses_ex_uf() {
    let foreign = Party {
        id: Some("foreign-buyer".to_owned()),
        name: "Globex Inc".to_owned(),
        tax_ids: Vec::new(), // foreign consumer: NF-e allows a dest with no id
        address: PostalAddress {
            lines: vec!["1 Market St".to_owned()],
            city: "Lisboa".to_owned(),
            subdivision: None,
            postal_code: "1100-148".to_owned(),
            country: CountryCode::new("PT").unwrap(),
        },
        contact: None,
    };
    let doc = doc_with(
        DocumentType::Invoice,
        foreign,
        vec![line("PROD-EXP", "Cafe exportacao", 10, 3000, 30000)],
        vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amt(0),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        }],
        MonetaryTotal {
            line_extension_amount: amt(30000),
            tax_exclusive_amount: amt(30000),
            tax_inclusive_amount: amt(30000),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(30000),
        },
    );
    let xml = to_inf_nfe_xml(&doc, &NfeContext::default()).unwrap();
    let dest_start = xml.find("<dest>").expect("dest present");
    let dest_block = &xml[dest_start..];
    assert!(
        dest_block.contains("<UF>EX</UF>"),
        "foreign recipient UF must be EX:\n{dest_block}"
    );
    // A dest with no fiscal id carries neither CNPJ nor CPF.
    assert!(!dest_block.contains("<CNPJ>"));
    assert!(!dest_block.contains("<CPF>"));
}

/// **Produção (live) NF-e clears end to end and maps to tpAmb 1.** SEFAZ's
/// environment dichotomy is `tpAmb` 1 = produção, 2 = homologação. A provider
/// bound to Produção must authorise the NF-e and report the produção signer
/// environment, distinct from the sandbox path the rest of the suite uses.
#[test]
fn brazil_producao_environment_authorizes_and_maps_tp_amb() {
    use invoicekit_signer_nfe::NfeEnvironment;

    // The signer-layer tpAmb mapping is the SEFAZ spec dichotomy.
    assert_eq!(
        NfeReportEnvironment::Producao
            .as_signer_environment()
            .tp_amb(),
        1
    );
    assert_eq!(
        NfeReportEnvironment::Homologacao
            .as_signer_environment()
            .tp_amb(),
        2
    );
    assert_eq!(
        NfeReportEnvironment::Producao.as_signer_environment(),
        NfeEnvironment::Producao
    );

    // A Produção-bound provider authorises a well-formed NF-e end to end.
    let signer: Arc<dyn Signer> =
        Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [3_u8; 32]));
    let prod_provider = MockNfeReportProvider::new(signer, NfeReportEnvironment::Producao);
    let xml = to_inf_nfe_xml(&brazil_invoice(), &NfeContext::default())
        .unwrap()
        .into_bytes();
    let mut req = report_request(xml);
    req.environment = NfeReportEnvironment::Producao;
    let report = prod_provider.report(&req).unwrap();
    assert!(report.envelope.is_authorized());
    assert_eq!(report.envelope.c_stat, 100);
    assert_eq!(report.envelope.chave_acesso.len(), 44);
}

/// **chave de acesso encodes cUF and nNF deterministically.** Per the MOC the
/// 44-digit key begins with the 2-digit IBGE state code (cUF) and embeds the
/// zero-padded 9-digit nNF. For RJ (cUF 33) the key starts "33"; the same
/// nNF must always produce the same key.
#[test]
fn brazil_chave_acesso_encodes_uf_and_is_stable() {
    let xml = to_inf_nfe_xml(&brazil_invoice(), &NfeContext::default())
        .unwrap()
        .into_bytes();
    let mut req = report_request(xml);
    req.uf = NfeUf::Rj; // Rio de Janeiro, IBGE cUF 33
    let a = provider(None).report(&req).unwrap();
    let b = provider(None).report(&req).unwrap();
    assert_eq!(a.envelope.chave_acesso.len(), 44);
    assert!(
        a.envelope.chave_acesso.starts_with("33"),
        "RJ chave must start with cUF 33, got {}",
        a.envelope.chave_acesso
    );
    assert_eq!(
        a.envelope.chave_acesso, b.envelope.chave_acesso,
        "chave de acesso must be deterministic for a fixed UF + nNF"
    );
    assert_eq!(a.envelope.uf, NfeUf::Rj);
}
