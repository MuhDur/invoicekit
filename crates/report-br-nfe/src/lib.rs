// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// NF-e / SEFAZ / CNPJ / CPF / ICMS / IBGE / infNFe / ICP-Brasil
// acronyms trip doc-markdown; suppress crate-wide.
#![allow(clippy::doc_markdown)]

//! Brazil **NF-e** (Nota Fiscal Eletronica) national-clearance report adapter.
//!
//! Brazil is a *per-state national-clearance* jurisdiction: a B2B/B2C invoice
//! is serialized to the national **infNFe** XML (inside `<NFe>`), signed with
//! the taxpayer's ICP-Brasil A1 certificate, and submitted to the state
//! Secretaria da Fazenda (SEFAZ), which returns a numeric `cStat` status, a
//! `chave de acesso` (44-digit access key) and a `protocolo de autorizacao`.
//! This crate provides the offline (local-only) end-to-end lifecycle:
//!
//! 1. **serialize** — [`to_inf_nfe_xml`] turns an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] into deterministic infNFe XML
//!    (`ide` / `emit` / `dest` / `det` / `total`). UBL/CII serializers do
//!    *not* emit this national format.
//! 2. **validate (local)** — [`validate_cnpj`] / [`validate_cpf`] enforce the
//!    real Brazilian taxpayer-id shapes (14-digit CNPJ, 11-digit CPF) with
//!    their mod-11 check digits; reference-grade SEFAZ schema validation stays
//!    an external backend and is labelled as such in the capability matrix.
//! 3. **sign + transmit** — [`MockNfeReportProvider`] composes the already-built
//!    [`invoicekit_signer_nfe::MockNfeProvider`] so the NF-e signature path,
//!    chave-de-acesso synthesis and protocolo assignment are exercised, never
//!    re-faked.
//! 4. **evidence** — the caller bundles the canonical document, infNFe XML,
//!    signed XML, and receipt into a signed `.ikb` evidence bundle.
//!
//! Live SEFAZ transmission (per-state SOAP web services over the ICP-Brasil
//! mutual-TLS channel) is bring-your-own-credentials and lands in a follow-up
//! `report-br-nfe-http` crate; this crate's `Mock*` providers are
//! deterministic and offline.
//!
//! **Rejection is not an error.** When SEFAZ denies authorization it returns a
//! denial `cStat` (`110`/`205`/`301`/`302`) — surfaced here as a
//! [`NfeReportEnvelope`] whose [`NfeReportEnvelope::status`] is a denial inside
//! an `Ok(_)` envelope, never as `Err`. `Err` is reserved for pre-wire shape
//! failures and transport faults.

use std::sync::Arc;

use invoicekit_ir::{CommercialDocument, DocumentLine, DocumentType, Party};
use invoicekit_signer::Signer;
use invoicekit_signer_nfe::{
    IcpBrasilCertificate, MockNfeProvider, NfeProvider, NfeStatus, NfeSubmitRequest, UfCode,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the NF-e substrate types this crate's public API surfaces, so
// downstream callers need not depend on `invoicekit-signer-nfe` directly.
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_nfe::{NfeEnvironment, UfCode as NfeUf};

// ---------------------------------------------------------------------------
// infNFe serialization (IR -> national NF-e XML)
// ---------------------------------------------------------------------------

/// NF-e serialization context: the NF-e-specific header fields that live in
/// `<ide>` but are not part of the jurisdiction-agnostic IR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NfeContext {
    /// `cUF` — IBGE numeric code of the issuing state (e.g. `35` for SP).
    pub uf: UfCode,
    /// `serie` — NF-e series (`1`..=`999`).
    pub serie: u32,
    /// `nNF` — sequential invoice number (`1`..=`999999999`).
    pub n_nf: u64,
    /// `tpNF` — operation direction: `0` = entrada, `1` = saida.
    pub tp_nf: u8,
    /// `natOp` — free-text nature of the operation (`Venda de mercadoria`).
    pub nat_op: String,
}

impl Default for NfeContext {
    fn default() -> Self {
        Self {
            uf: UfCode::Sp,
            serie: 1,
            n_nf: 1,
            tp_nf: 1,
            nat_op: "Venda de mercadoria".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to infNFe XML.
#[derive(Debug, Error)]
pub enum InfNfeError {
    /// The IR `document_type` has no NF-e `finNFe`/`tpNF` mapping.
    #[error("document type {0:?} is not representable as an NF-e finNFe")]
    UnsupportedDocumentType(DocumentType),
    /// The issuer (emitente) carries no usable CNPJ/CPF.
    #[error("emitente has no CNPJ/CPF usable as emit/CNPJ")]
    MissingEmitenteTaxId,
    /// The serialization context was malformed (e.g. zero nNF).
    #[error("invalid NF-e context: {0}")]
    BadContext(String),
    /// Summing the tax-summary entries overflowed `Decimal`'s range.
    /// The payload `String` names the total that could not be represented.
    #[error("NF-e total {0} is not representable as a Decimal")]
    TotalsUnrepresentable(&'static str),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic infNFe XML
/// (the national NF-e payload, layout version `4.00`).
///
/// Emits the mandatory NF-e spine — `<ide>` (identification), `<emit>`
/// (emitente / issuer), `<dest>` (destinatario / recipient), one `<det>` per
/// line, and `<total><ICMSTot>` — inside `<NFe><infNFe>`. Output is byte-stable
/// by construction: a fixed element order with no maps and amounts formatted at
/// fixed scale 2.
///
/// # Errors
///
/// Returns [`InfNfeError::UnsupportedDocumentType`] for document types with no
/// NF-e mapping, [`InfNfeError::MissingEmitenteTaxId`] when the issuer has no
/// CNPJ/CPF, [`InfNfeError::BadContext`] when the context is malformed, and
/// [`InfNfeError::TotalsUnrepresentable`] when summing the tax-summary entries
/// overflows `Decimal`'s range.
pub fn to_inf_nfe_xml(
    document: &CommercialDocument,
    context: &NfeContext,
) -> Result<String, InfNfeError> {
    if context.n_nf == 0 {
        return Err(InfNfeError::BadContext("nNF must be >= 1".to_owned()));
    }
    if context.nat_op.trim().is_empty() {
        return Err(InfNfeError::BadContext("natOp must not be empty".to_owned()));
    }
    let fin_nfe = fin_nfe(document.document_type)?;
    let emit_id = party_tax_id(&document.supplier).ok_or(InfNfeError::MissingEmitenteTaxId)?;

    // The infNFe Id attribute is `NFe` + the 44-digit chave de acesso.
    let chave = invoicekit_signer_nfe::build_chave_acesso(context.uf, emit_id.digits(), context.n_nf);

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<NFe xmlns=\"http://www.portalfiscal.inf.br/nfe\">\n");
    indent(&mut out, 1);
    out.push_str("<infNFe versao=\"4.00\" Id=\"NFe");
    push_escaped(&mut out, &chave);
    out.push_str("\">\n");

    // --- ide (identificacao) ---
    open(&mut out, 2, "ide");
    el(&mut out, 3, "cUF", &uf_numeric_code(context.uf).to_string());
    el(&mut out, 3, "natOp", &context.nat_op);
    el(&mut out, 3, "mod", "55");
    el(&mut out, 3, "serie", &context.serie.to_string());
    el(&mut out, 3, "nNF", &context.n_nf.to_string());
    el(&mut out, 3, "dhEmi", &format!("{}T00:00:00-03:00", document.issue_date.as_str()));
    el(&mut out, 3, "tpNF", &context.tp_nf.to_string());
    el(&mut out, 3, "finNFe", fin_nfe);
    close(&mut out, 2, "ide");

    // --- emit (emitente / issuer) ---
    write_party(&mut out, "emit", &document.supplier, true)?;
    // --- dest (destinatario / recipient) ---
    write_party(&mut out, "dest", &document.customer, false)?;

    // --- det (one per line) ---
    for (index, line) in document.lines.iter().enumerate() {
        write_det(&mut out, index + 1, line);
    }

    // --- total (ICMSTot) ---
    open(&mut out, 2, "total");
    open(&mut out, 3, "ICMSTot");
    let (taxable, tax) = totals(document)?;
    el(&mut out, 4, "vBC", &fmt_amount(taxable));
    el(&mut out, 4, "vICMS", &fmt_amount(tax));
    el(
        &mut out,
        4,
        "vProd",
        &fmt_amount(document.monetary_total.line_extension_amount.inner()),
    );
    el(
        &mut out,
        4,
        "vNF",
        &fmt_amount(document.monetary_total.payable_amount.inner()),
    );
    close(&mut out, 3, "ICMSTot");
    close(&mut out, 2, "total");

    close(&mut out, 1, "infNFe");
    out.push_str("</NFe>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to an NF-e `finNFe` code.
///
/// `1` = NF-e normal, `4` = devolucao / retorno (credit note). Debit notes,
/// pro-forma and self-billed have no NF-e shape and are rejected.
fn fin_nfe(document_type: DocumentType) -> Result<&'static str, InfNfeError> {
    match document_type {
        DocumentType::Invoice => Ok("1"),
        DocumentType::CreditNote => Ok("4"),
        other @ (DocumentType::DebitNote | DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(InfNfeError::UnsupportedDocumentType(other))
        }
    }
}

/// A Brazilian federal taxpayer identifier: either a CNPJ (14 digits,
/// company) or a CPF (11 digits, natural person).
#[derive(Clone, Debug, Eq, PartialEq)]
enum BrazilTaxId {
    /// CNPJ (Cadastro Nacional da Pessoa Juridica), 14 digits.
    Cnpj(String),
    /// CPF (Cadastro de Pessoas Fisicas), 11 digits.
    Cpf(String),
}

impl BrazilTaxId {
    /// The NF-e element name carrying this identifier (`CNPJ` or `CPF`).
    const fn element(&self) -> &'static str {
        match self {
            Self::Cnpj(_) => "CNPJ",
            Self::Cpf(_) => "CPF",
        }
    }

    /// The bare digit string.
    fn digits(&self) -> &str {
        match self {
            Self::Cnpj(d) | Self::Cpf(d) => d,
        }
    }
}

/// Extract a Brazilian taxpayer id from a party: the first tax id whose digit
/// count is 14 (CNPJ) or 11 (CPF), preferring a `cnpj`/`cpf` scheme when set.
fn party_tax_id(party: &Party) -> Option<BrazilTaxId> {
    let scheme_match = party.tax_ids.iter().find(|t| {
        t.scheme.eq_ignore_ascii_case("cnpj") || t.scheme.eq_ignore_ascii_case("cpf")
    });
    let chosen = scheme_match.or_else(|| party.tax_ids.first())?;
    let digits: String = chosen.value.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        14 => Some(BrazilTaxId::Cnpj(digits)),
        11 => Some(BrazilTaxId::Cpf(digits)),
        _ => None,
    }
}

/// Write an `emit` / `dest` block. The emitente requires a CNPJ/CPF; the
/// destinatario's is optional (consumer with no id).
fn write_party(
    out: &mut String,
    tag: &str,
    party: &Party,
    require_tax_id: bool,
) -> Result<(), InfNfeError> {
    let tax_id = party_tax_id(party);
    if require_tax_id && tax_id.is_none() {
        return Err(InfNfeError::MissingEmitenteTaxId);
    }
    // Address element name follows the block: <enderEmit> / <enderDest>.
    let ender_tag = if tag == "emit" { "enderEmit" } else { "enderDest" };
    open(out, 2, tag);
    if let Some(id) = &tax_id {
        el(out, 3, id.element(), id.digits());
    }
    // emit uses xNome (corporate name); dest uses xNome as well in NF-e 4.00.
    el(out, 3, "xNome", &party.name);
    open(out, 3, ender_tag);
    el(out, 4, "xLgr", &party.address.lines.join(", "));
    el(out, 4, "xMun", &party.address.city);
    el(out, 4, "UF", uf_from_country_or_address(party));
    el(out, 4, "CEP", &cep(party));
    el(out, 4, "cPais", "1058");
    el(out, 4, "xPais", "BRASIL");
    close(out, 3, ender_tag);
    close(out, 2, tag);
    Ok(())
}

/// Write a `det` (line item) block with its nested `prod` element.
fn write_det(out: &mut String, numero: usize, line: &DocumentLine) {
    indent(out, 2);
    out.push_str("<det nItem=\"");
    push_escaped(out, &numero.to_string());
    out.push_str("\">\n");
    open(out, 3, "prod");
    el(out, 4, "cProd", &line.id);
    el(out, 4, "xProd", &line.description);
    el(out, 4, "uCom", line.unit_code.as_deref().unwrap_or("UN"));
    el(out, 4, "qCom", &fmt_amount(line.quantity.inner()));
    el(out, 4, "vUnCom", &fmt_amount(line.unit_price.inner()));
    el(out, 4, "vProd", &fmt_amount(line.line_extension_amount.inner()));
    close(out, 3, "prod");
    close(out, 2, "det");
}

/// Sum the tax-summary entries into `(vBC, vICMS)` totals at scale 2.
///
/// # Errors
///
/// Returns [`InfNfeError::TotalsUnrepresentable`] when summing the untrusted
/// per-category amounts would exceed `Decimal`'s range; bail with a typed
/// error rather than panicking on the overflowing `AddAssign`.
fn totals(document: &CommercialDocument) -> Result<(Decimal, Decimal), InfNfeError> {
    let mut taxable = Decimal::ZERO;
    let mut tax = Decimal::ZERO;
    for summary in &document.tax_summary {
        taxable = taxable
            .checked_add(summary.taxable_amount.inner())
            .ok_or(InfNfeError::TotalsUnrepresentable("vBC"))?;
        tax = tax
            .checked_add(summary.tax_amount.inner())
            .ok_or(InfNfeError::TotalsUnrepresentable("vICMS"))?;
    }
    Ok((taxable, tax))
}

/// The 2-letter UF for a party, from the address subdivision when present,
/// else `"EX"` (exterior) for non-Brazil countries, else `"SP"` fallback.
fn uf_from_country_or_address(party: &Party) -> &str {
    if let Some(sub) = party.address.subdivision.as_deref() {
        let trimmed = sub.trim();
        if trimmed.len() == 2 && trimmed.bytes().all(|b| b.is_ascii_alphabetic()) {
            return trimmed;
        }
    }
    if party.address.country.as_str().eq_ignore_ascii_case("BR") {
        "SP"
    } else {
        "EX"
    }
}

/// The CEP (postal code) digits, zero-padded to 8.
fn cep(party: &Party) -> String {
    let digits: String = party
        .address
        .postal_code
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if digits.len() >= 8 {
        digits.chars().take(8).collect()
    } else {
        format!("{}{digits}", "0".repeat(8 - digits.len()))
    }
}

/// IBGE numeric code for a UF (subset; catch-all `99`).
fn uf_numeric_code(uf: UfCode) -> u8 {
    match uf {
        UfCode::Sp => 35,
        UfCode::Rj => 33,
        UfCode::Mg => 31,
        UfCode::Pr => 41,
        UfCode::Rs => 43,
        UfCode::Sc => 42,
        UfCode::Ba => 29,
        UfCode::Df => 53,
        UfCode::Other => 99,
    }
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`), deterministic.
fn fmt_amount(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

/// Append `<tag>escaped-text</tag>` at the given indent depth.
fn el(out: &mut String, depth: usize, tag: &str, text: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    out.push('>');
    push_escaped(out, text);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

/// Append an opening `<tag>` at the given indent depth.
fn open(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    out.push_str(">\n");
}

/// Append a closing `</tag>` at the given indent depth.
fn close(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Append XML-escaped text content.
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

// ---------------------------------------------------------------------------
// NF-e report adapter (validate -> sign -> transmit -> typed receipt)
// ---------------------------------------------------------------------------

/// NF-e runtime environment selector. Mirrors the SEFAZ tpAmb dichotomy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NfeReportEnvironment {
    /// Homologacao (SEFAZ sandbox, tpAmb = 2).
    Homologacao,
    /// Producao (live SEFAZ clearance, tpAmb = 1).
    Producao,
}

impl NfeReportEnvironment {
    /// Map to the signer-layer [`NfeEnvironment`].
    #[must_use]
    pub const fn as_signer_environment(self) -> NfeEnvironment {
        match self {
            Self::Homologacao => NfeEnvironment::Homologacao,
            Self::Producao => NfeEnvironment::Producao,
        }
    }
}

/// Operator-facing NF-e report request. The infNFe XML is produced upstream by
/// [`to_inf_nfe_xml`]; this request carries it plus the identity SEFAZ needs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NfeReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: NfeReportEnvironment,
    /// Issuer CNPJ (14 digits) or CPF (11 digits).
    pub issuer_tax_id: String,
    /// Destination state UF.
    pub uf: UfCode,
    /// `nNF` — sequential invoice number (must match the serialized infNFe).
    pub n_nf: u64,
    /// ICP-Brasil A1 certificate used to sign the NF-e.
    pub certificate: IcpBrasilCertificate,
    /// Canonical infNFe XML bytes.
    pub inf_nfe_xml: Vec<u8>,
}

/// Typed report verdict — the audit-relevant SEFAZ receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NfeReportEnvelope {
    /// Chave de acesso (44-digit NF-e access key).
    pub chave_acesso: String,
    /// Protocolo de autorizacao SEFAZ assigned.
    pub protocolo_autorizacao: String,
    /// Numeric SEFAZ `cStat` code.
    pub c_stat: u32,
    /// Typed status mapping.
    pub status: NfeStatus,
    /// SEFAZ `xMotivo` status description.
    pub status_descricao: String,
    /// Destination state UF.
    pub uf: UfCode,
    /// RFC-3339 UTC timestamp SEFAZ recorded.
    pub recorded_at: String,
    /// XAdES signature receipt over the infNFe bytes.
    pub signature: Signature,
    /// Reason text when the receipt is a denial (Uso Denegado).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl NfeReportEnvelope {
    /// True only when SEFAZ authorised the NF-e (`cStat = 100`).
    #[must_use]
    pub const fn is_authorized(&self) -> bool {
        self.status.is_authorized()
    }
}

/// The full result of a report: the receipt plus the signed infNFe bytes (the
/// latter is an evidence-bundle artefact, kept out of the receipt JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NfeReport {
    /// Audit receipt.
    pub envelope: NfeReportEnvelope,
    /// Signed (XAdES-wrapped) infNFe XML bytes.
    pub signed_nfe_xml: Vec<u8>,
}

/// Typed NF-e report errors.
///
/// Three buckets: payload shape, country-id shape, and transport. A denial
/// verdict is **not** here — it is an `Ok` envelope whose
/// [`NfeReportEnvelope::status`] is a denial cStat.
#[derive(Debug, Error)]
pub enum NfeReportError {
    /// The infNFe payload failed shape validation before the wire.
    #[error("infNFe xml rejected: {0}")]
    BadXml(String),
    /// The issuer tax id did not match the expected CNPJ/CPF shape.
    #[error("invalid issuer tax id: {0}")]
    BadTaxId(String),
    /// The NF-e signer/transport failed on the wire.
    #[error("nfe signer/transport failure: {0}")]
    Transport(String),
}

/// The NF-e report surface every SEFAZ integration (per state) implements.
pub trait NfeReportProvider: Send + Sync {
    /// Validate the issuer identity, sign the infNFe, transmit to SEFAZ, and
    /// return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`NfeReportError`] on pre-wire shape failures (bad tax id, empty
    /// payload) or transport faults. A SEFAZ denial (Uso Denegado) is surfaced
    /// as an `Ok` envelope, not an error.
    fn report(&self, request: &NfeReportRequest) -> Result<NfeReport, NfeReportError>;
}

/// Deterministic offline NF-e report provider.
///
/// Composes [`invoicekit_signer_nfe::MockNfeProvider`] so the real NF-e
/// signature path, chave-de-acesso synthesis and protocolo assignment are
/// exercised rather than re-implemented.
pub struct MockNfeReportProvider {
    signer: Arc<dyn Signer>,
    environment: NfeReportEnvironment,
    forced_c_stat: u32,
    fixed_recorded_at: String,
}

impl MockNfeReportProvider {
    /// Build a mock report provider over the given signer (key it by the
    /// certificate serial number, e.g.
    /// `SoftwareSigner::new().with_key(serial, [3u8; 32])`).
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>, environment: NfeReportEnvironment) -> Self {
        Self {
            signer,
            environment,
            forced_c_stat: 100, // Autorizado o uso da NF-e
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every submission to return a specific SEFAZ `cStat` (e.g. `110` to
    /// exercise the Uso Denegado / rejection path).
    #[must_use]
    pub fn with_forced_c_stat(mut self, c_stat: u32) -> Self {
        self.forced_c_stat = c_stat;
        self
    }
}

impl NfeReportProvider for MockNfeReportProvider {
    fn report(&self, request: &NfeReportRequest) -> Result<NfeReport, NfeReportError> {
        validate_brazil_tax_id(&request.issuer_tax_id)?;
        if request.inf_nfe_xml.is_empty() {
            return Err(NfeReportError::BadXml("payload is empty".to_owned()));
        }
        let inner = MockNfeProvider::new(
            "sefaz-test",
            self.environment.as_signer_environment(),
            Arc::clone(&self.signer),
        )
        .with_forced_c_stat(self.forced_c_stat);
        let stamp = inner
            .submit(
                &NfeSubmitRequest {
                    nfe_xml: request.inf_nfe_xml.clone(),
                    certificate: request.certificate.clone(),
                    uf: request.uf,
                    n_nf: request.n_nf,
                },
                self.environment.as_signer_environment(),
            )
            .map_err(|e| NfeReportError::Transport(e.to_string()))?;
        let reason = (!stamp.status.is_authorized()).then(|| {
            format!(
                "SEFAZ denied authorization (cStat={}, {})",
                stamp.c_stat, stamp.status_descricao
            )
        });
        Ok(NfeReport {
            envelope: NfeReportEnvelope {
                chave_acesso: stamp.chave_acesso,
                protocolo_autorizacao: stamp.protocolo_autorizacao,
                c_stat: stamp.c_stat,
                status: stamp.status,
                status_descricao: stamp.status_descricao,
                uf: stamp.uf,
                recorded_at: self.fixed_recorded_at.clone(),
                signature: stamp.signature,
                reason,
            },
            signed_nfe_xml: stamp.signed_nfe_xml,
        })
    }
}

// ---------------------------------------------------------------------------
// Country-specific identifier validators (load-bearing anti-slop content)
// ---------------------------------------------------------------------------

/// Validate a Brazilian issuer tax id: CNPJ (14 digits) or CPF (11 digits),
/// each with a valid mod-11 check digit.
///
/// # Errors
///
/// Returns [`NfeReportError::BadTaxId`] when the value is neither a valid CNPJ
/// nor a valid CPF.
pub fn validate_brazil_tax_id(id: &str) -> Result<(), NfeReportError> {
    let digits = digit_values(id);
    match digits.len() {
        14 if cnpj_check_digits_ok(&digits) => Ok(()),
        11 if cpf_check_digits_ok(&digits) => Ok(()),
        _ => Err(NfeReportError::BadTaxId(format!(
            "expected a valid 14-digit CNPJ or 11-digit CPF, got {id:?}"
        ))),
    }
}

/// Validate a CNPJ (14-digit Brazilian company id) including its two mod-11
/// check digits. Punctuation (`.`/`/`/`-`) is ignored.
///
/// # Errors
///
/// Returns [`NfeReportError::BadTaxId`] when the value is not 14 digits or the
/// check digits fail.
pub fn validate_cnpj(cnpj: &str) -> Result<(), NfeReportError> {
    let digits = digit_values(cnpj);
    if digits.len() == 14 && cnpj_check_digits_ok(&digits) {
        Ok(())
    } else {
        Err(NfeReportError::BadTaxId(format!(
            "expected a valid 14-digit CNPJ, got {cnpj:?}"
        )))
    }
}

/// Validate a CPF (11-digit Brazilian natural-person id) including its two
/// mod-11 check digits. Punctuation (`.`/`-`) is ignored.
///
/// # Errors
///
/// Returns [`NfeReportError::BadTaxId`] when the value is not 11 digits or the
/// check digits fail.
pub fn validate_cpf(cpf: &str) -> Result<(), NfeReportError> {
    let digits = digit_values(cpf);
    if digits.len() == 11 && cpf_check_digits_ok(&digits) {
        Ok(())
    } else {
        Err(NfeReportError::BadTaxId(format!(
            "expected a valid 11-digit CPF, got {cpf:?}"
        )))
    }
}

/// Extract the numeric value (`0..=9`) of every ASCII digit in `s`, dropping
/// any punctuation. The shared front end of all three taxpayer-id validators.
fn digit_values(s: &str) -> Vec<u8> {
    s.chars()
        .filter(char::is_ascii_digit)
        .map(|c| c as u8 - b'0')
        .collect()
}

/// CNPJ check-digit verification: two mod-11 digits over weights cycling
/// `2..=9`, right-to-left.
fn cnpj_check_digits_ok(digits: &[u8]) -> bool {
    if digits.len() != 14 || digits.iter().all(|&d| d == digits[0]) {
        return false;
    }
    let weights_first = [5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let weights_second = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
    let d1 = mod11_digit(&digits[..12], &weights_first);
    let d2 = mod11_digit(&digits[..13], &weights_second);
    digits[12] == d1 && digits[13] == d2
}

/// CPF check-digit verification: two mod-11 digits over descending weights.
fn cpf_check_digits_ok(digits: &[u8]) -> bool {
    if digits.len() != 11 || digits.iter().all(|&d| d == digits[0]) {
        return false;
    }
    let weights_first = [10, 9, 8, 7, 6, 5, 4, 3, 2];
    let weights_second = [11, 10, 9, 8, 7, 6, 5, 4, 3, 2];
    let d1 = mod11_digit(&digits[..9], &weights_first);
    let d2 = mod11_digit(&digits[..10], &weights_second);
    digits[9] == d1 && digits[10] == d2
}

/// Compute one Brazilian mod-11 check digit: weighted sum, remainder of the
/// sum modulo 11; a remainder below 2 yields `0`, else `11 - remainder`.
fn mod11_digit(digits: &[u8], weights: &[u8]) -> u8 {
    let sum: u32 = digits
        .iter()
        .zip(weights.iter())
        .map(|(&d, &w)| u32::from(d) * u32::from(w))
        .sum();
    let remainder = sum % 11;
    // The result is always in 0..=9, so the narrowing conversion cannot fail.
    let digit = if remainder < 2 { 0 } else { 11 - remainder };
    u8::try_from(digit).unwrap_or(0)
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_report_br_nfe::crate_name(), "invoicekit-report-br-nfe");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-br-nfe"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocumentParts, CountryCode, DateOnly, DocumentId, DocumentMeta, DocumentNumber,
        Iso4217Code, MonetaryTotal, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
    };
    use invoicekit_ir::DecimalValue;
    use invoicekit_signer::SoftwareSigner;

    const CERT_SERIAL: &str = "ABCDEF1234567890";

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn br_party(name: &str, tax_scheme: &str, tax_value: &str, city: &str, uf: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: tax_scheme.to_owned(),
                value: tax_value.to_owned(),
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

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-br-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("NF-2026-0001").unwrap(),
            currency: Iso4217Code::new("BRL").unwrap(),
            // Real valid CNPJ / CPF (check digits computed).
            supplier: br_party("Acme Comercio LTDA", "cnpj", "11.222.333/0001-81", "Sao Paulo", "SP"),
            customer: br_party("Beta Servicos LTDA", "cnpj", "11444777000161", "Rio de Janeiro", "RJ"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "PROD-1".to_owned(),
                description: "Servico & consultoria".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("UN".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
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
                tenant_id: "tenant_123".to_owned(),
                trace_id: "trace_abc".to_owned(),
                source_system: Some("e2e".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_cert() -> IcpBrasilCertificate {
        IcpBrasilCertificate {
            serial_number: CERT_SERIAL.to_owned(),
            cnpj: "11222333000181".to_owned(),
            subject_dn: "CN=Acme Comercio LTDA,O=Acme,C=BR".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn provider(forced: Option<u32>) -> MockNfeReportProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key(CERT_SERIAL, [3_u8; 32]));
        let p = MockNfeReportProvider::new(signer, NfeReportEnvironment::Homologacao);
        match forced {
            Some(c_stat) => p.with_forced_c_stat(c_stat),
            None => p,
        }
    }

    fn sample_request(inf_nfe_xml: Vec<u8>) -> NfeReportRequest {
        NfeReportRequest {
            tenant_id: "tenant_123".to_owned(),
            environment: NfeReportEnvironment::Homologacao,
            issuer_tax_id: "11222333000181".to_owned(),
            uf: UfCode::Sp,
            n_nf: 4242,
            certificate: sample_cert(),
            inf_nfe_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-br-nfe");
    }

    #[test]
    fn inf_nfe_contains_mandatory_structure() {
        let xml = to_inf_nfe_xml(&sample_invoice(), &NfeContext::default()).unwrap();
        for needle in [
            "<NFe xmlns=\"http://www.portalfiscal.inf.br/nfe\">",
            "<infNFe versao=\"4.00\" Id=\"NFe",
            "<ide>",
            "<cUF>35</cUF>",
            "<mod>55</mod>",
            "<finNFe>1</finNFe>",
            "<emit>",
            "<CNPJ>11222333000181</CNPJ>",
            "<xNome>Acme Comercio LTDA</xNome>",
            "<dest>",
            "<det nItem=\"1\">",
            "<xProd>Servico &amp; consultoria</xProd>",
            "<total>",
            "<ICMSTot>",
            "<vICMS>18.00</vICMS>",
            "<vNF>118.00</vNF>",
        ] {
            assert!(xml.contains(needle), "infNFe missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn inf_nfe_is_deterministic() {
        let doc = sample_invoice();
        let ctx = NfeContext::default();
        assert_eq!(
            to_inf_nfe_xml(&doc, &ctx).unwrap(),
            to_inf_nfe_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn inf_nfe_id_embeds_44_digit_chave() {
        let xml = to_inf_nfe_xml(&sample_invoice(), &NfeContext::default()).unwrap();
        // Id="NFe" + 44 chars; assert the prefix and that 44 digits follow.
        let start = xml.find("Id=\"NFe").unwrap() + "Id=\"NFe".len();
        let rest = &xml[start..];
        let chave: String = rest.chars().take_while(char::is_ascii_digit).collect();
        assert_eq!(chave.len(), 44, "chave de acesso must be 44 digits");
    }

    #[test]
    fn inf_nfe_rejects_unsupported_document_type() {
        let err = fin_nfe(DocumentType::DebitNote).unwrap_err();
        assert!(matches!(err, InfNfeError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn inf_nfe_rejects_zero_n_nf() {
        let ctx = NfeContext {
            n_nf: 0,
            ..NfeContext::default()
        };
        let err = to_inf_nfe_xml(&sample_invoice(), &ctx).unwrap_err();
        assert!(matches!(err, InfNfeError::BadContext(_)));
    }

    #[test]
    fn inf_nfe_overflowing_tax_summary_errors_not_panics() {
        // Two tax-summary entries each at half of Decimal::MAX make the running
        // `taxable` accumulator exceed the range on the second entry. Before the
        // `checked_add` fix the `+=` AddAssign panicked; now it surfaces a typed
        // [`InfNfeError::TotalsUnrepresentable`].
        let near_max = DecimalValue::new(Decimal::MAX);
        let mut doc = sample_invoice();
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: near_max.clone(),
                tax_amount: amt(0),
                tax_rate: None,
            },
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: near_max,
                tax_amount: amt(0),
                tax_rate: None,
            },
        ];
        let err = to_inf_nfe_xml(&doc, &NfeContext::default())
            .expect_err("summing two Decimal::MAX taxable amounts must error, not panic");
        assert!(
            matches!(err, InfNfeError::TotalsUnrepresentable("vBC")),
            "expected TotalsUnrepresentable(\"vBC\"), got {err:?}"
        );
    }

    #[test]
    fn report_happy_path_is_authorized() {
        let xml = to_inf_nfe_xml(&sample_invoice(), &NfeContext::default())
            .unwrap()
            .into_bytes();
        let report = provider(None).report(&sample_request(xml)).unwrap();
        assert!(report.envelope.is_authorized());
        assert_eq!(report.envelope.c_stat, 100);
        assert_eq!(report.envelope.chave_acesso.len(), 44);
        assert!(report.envelope.protocolo_autorizacao.starts_with("135"));
        assert!(report.envelope.reason.is_none());
        assert!(report.signed_nfe_xml.starts_with(b"<XAdES-stub>"));
        assert_eq!(report.envelope.status_descricao, "Autorizado o uso da NF-e");
    }

    #[test]
    fn report_denial_is_ok_not_err() {
        // cStat 110 = Uso Denegado. A denial is a receipt status, NOT an Err.
        let xml = b"<NFe/>".to_vec();
        let report = provider(Some(110)).report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.c_stat, 110);
        assert_eq!(report.envelope.status, NfeStatus::Denied);
        assert!(!report.envelope.is_authorized());
        assert!(report.envelope.reason.is_some());
    }

    #[test]
    fn report_rejects_bad_tax_id() {
        let mut req = sample_request(b"<x/>".to_vec());
        req.issuer_tax_id = "123".to_owned();
        assert!(matches!(
            provider(None).report(&req).unwrap_err(),
            NfeReportError::BadTaxId(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new());
        assert!(matches!(
            provider(None).report(&req).unwrap_err(),
            NfeReportError::BadXml(_)
        ));
    }

    #[test]
    fn cnpj_validator_accepts_valid_and_rejects_invalid() {
        // Known-valid CNPJ with correct check digits.
        assert!(validate_cnpj("11.222.333/0001-81").is_ok());
        assert!(validate_cnpj("11222333000181").is_ok());
        // Wrong check digit.
        assert!(validate_cnpj("11222333000180").is_err());
        // Repeated digits are rejected (a classic mod-11 trap).
        assert!(validate_cnpj("00000000000000").is_err());
        // Wrong length.
        assert!(validate_cnpj("1122233300018").is_err());
    }

    #[test]
    fn cpf_validator_accepts_valid_and_rejects_invalid() {
        // Known-valid CPF (check digits computed).
        assert!(validate_cpf("529.982.247-25").is_ok());
        assert!(validate_cpf("52998224725").is_ok());
        // Wrong check digit.
        assert!(validate_cpf("52998224724").is_err());
        // Repeated digits rejected.
        assert!(validate_cpf("11111111111").is_err());
        // Wrong length.
        assert!(validate_cpf("5299822472").is_err());
    }

    #[test]
    fn brazil_tax_id_accepts_both_shapes() {
        assert!(validate_brazil_tax_id("11222333000181").is_ok()); // CNPJ
        assert!(validate_brazil_tax_id("52998224725").is_ok()); // CPF
        assert!(validate_brazil_tax_id("not-an-id").is_err());
        assert!(validate_brazil_tax_id("12345678901234").is_err()); // 14 digits, bad CD
    }

    #[test]
    fn environment_maps_to_signer_layer() {
        assert_eq!(
            NfeReportEnvironment::Producao.as_signer_environment(),
            NfeEnvironment::Producao
        );
        assert_eq!(
            NfeReportEnvironment::Homologacao.as_signer_environment(),
            NfeEnvironment::Homologacao
        );
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_inf_nfe_xml(&sample_invoice(), &NfeContext::default())
            .unwrap()
            .into_bytes();
        let env = provider(None).report(&sample_request(xml)).unwrap().envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: NfeReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }
}
