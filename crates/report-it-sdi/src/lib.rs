// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// SDI / FatturaPA / Aruba / PEC / IVA / AdE acronyms trip doc-markdown.
#![allow(clippy::doc_markdown)]

//! Italy **SDI** (Sistema di Interscambio) national-clearance report adapter.
//!
//! Italy is a *national-clearance* jurisdiction: a B2B/B2G e-invoice is
//! serialized to the national **FatturaPA** (`FatturaElettronica`) XML format,
//! XAdES-signed by the issuer, and submitted to the Agenzia delle Entrate
//! Sistema di Interscambio (SDI), which returns one of five receipt kinds.
//! This crate provides the offline (local-only) end-to-end lifecycle:
//!
//! 1. **serialize** — [`to_fattura_pa_xml`] turns an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] into deterministic FatturaPA XML
//!    (UBL/CII serializers do *not* emit this national format).
//! 2. **validate (local)** — [`validate_italian_tax_id`] and
//!    [`validate_progressivo`] enforce the real Partita IVA / Codice Fiscale and
//!    `ProgressivoInvio` shapes; reference-grade Schematron validation stays an
//!    external (JVM) backend and is labelled as such in the capability matrix.
//! 3. **sign + transmit** — [`MockSdiReportProvider`] composes the already-built
//!    [`invoicekit_signer_sdi::MockSdiProvider`] so the SDI XAdES signature path
//!    and `IdentificativoSdI` synthesis are exercised, never re-faked.
//! 4. **evidence** — the caller bundles the canonical document, FatturaPA XML,
//!    signed XML, and receipt into a signed `.ikb` evidence bundle.
//!
//! Live SDI transmission (Aruba/Infocert/Namirial web-service or PEC) is
//! bring-your-own-credentials and lands in a follow-up `report-it-sdi-http`
//! crate; this crate's `Mock*` providers are deterministic and offline.
//!
//! **Rejection is not an error.** When SDI refuses an invoice it returns a
//! `Notifica di Scarto` (NS) — surfaced here as
//! [`invoicekit_signer_sdi::SdiReceiptKind::NotificaScarto`] inside an
//! `Ok(_)` envelope, never as `Err`. `Err` is reserved for pre-wire shape
//! failures and transport/TLS/DNS faults.

use std::sync::Arc;

use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use invoicekit_signer::Signer;
use invoicekit_signer_sdi::{MockSdiProvider, SdiProvider, SdiSubmitRequest};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the SDI substrate types this crate's public API surfaces, so
// downstream callers need not depend on `invoicekit-signer-sdi` directly.
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_sdi::{ArubaQualifiedCertificate, SdiReceiptKind, SdiTransport};

// ---------------------------------------------------------------------------
// FatturaPA serialization (IR -> national FatturaElettronica XML)
// ---------------------------------------------------------------------------

/// FatturaPA transmission context: the transmission-level fields that live in
/// `DatiTrasmissione` but are not part of the jurisdiction-agnostic IR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FatturaPaContext {
    /// `ProgressivoInvio` — 1..=5 alphanumeric chars assigned by the issuer.
    pub progressivo_invio: String,
    /// `CodiceDestinatario` — 7-char recipient routing code (`"0000000"`
    /// for PEC-routed or foreign recipients).
    pub codice_destinatario: String,
}

impl Default for FatturaPaContext {
    fn default() -> Self {
        Self {
            progressivo_invio: "00001".to_owned(),
            codice_destinatario: "0000000".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to FatturaPA XML.
#[derive(Debug, Error)]
pub enum FatturaPaError {
    /// The IR `document_type` has no FatturaPA `TipoDocumento` mapping.
    #[error("document type {0:?} is not representable as FatturaPA TipoDocumento")]
    UnsupportedDocumentType(DocumentType),
    /// The supplier (CedentePrestatore) carries no usable fiscal identifier.
    #[error("supplier has no tax id usable as IdFiscaleIVA/CodiceFiscale")]
    MissingSupplierTaxId,
    /// The transmission context was malformed (e.g. blank ProgressivoInvio).
    #[error("invalid FatturaPA transmission context: {0}")]
    BadContext(String),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic FatturaPA
/// (`FatturaElettronica`) XML, version `FPR12` (private B2B/B2G).
///
/// Output is byte-stable by construction: a fixed element order with no maps
/// and amounts formatted at fixed scale 2. The document is expected to have
/// passed IR validation already (it has, if built via
/// [`CommercialDocument::new`]).
///
/// # Errors
///
/// Returns [`FatturaPaError::UnsupportedDocumentType`] for document types with
/// no `TipoDocumento` mapping, [`FatturaPaError::MissingSupplierTaxId`] when the
/// supplier has no fiscal identifier, and [`FatturaPaError::BadContext`] when
/// the transmission context is malformed.
pub fn to_fattura_pa_xml(
    document: &CommercialDocument,
    context: &FatturaPaContext,
) -> Result<String, FatturaPaError> {
    if context.progressivo_invio.is_empty() {
        return Err(FatturaPaError::BadContext(
            "ProgressivoInvio must not be empty".to_owned(),
        ));
    }
    let tipo_documento = tipo_documento(document.document_type)?;
    let (supplier_country, supplier_code) =
        party_fiscal_id(&document.supplier).ok_or(FatturaPaError::MissingSupplierTaxId)?;

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<p:FatturaElettronica versione=\"FPR12\" xmlns:p=\"http://ivaservizi.agenziaentrate.gov.it/docs/xsd/fatture/v1.2\">\n",
    );

    // --- FatturaElettronicaHeader ---
    open(&mut out, 1, "FatturaElettronicaHeader");
    open(&mut out, 2, "DatiTrasmissione");
    open(&mut out, 3, "IdTrasmittente");
    el(&mut out, 4, "IdPaese", &supplier_country);
    el(&mut out, 4, "IdCodice", &supplier_code);
    close(&mut out, 3, "IdTrasmittente");
    el(&mut out, 3, "ProgressivoInvio", &context.progressivo_invio);
    el(&mut out, 3, "FormatoTrasmissione", "FPR12");
    el(&mut out, 3, "CodiceDestinatario", &context.codice_destinatario);
    close(&mut out, 2, "DatiTrasmissione");

    write_party(&mut out, "CedentePrestatore", &document.supplier, true)?;
    write_party(&mut out, "CessionarioCommittente", &document.customer, false)?;
    close(&mut out, 1, "FatturaElettronicaHeader");

    // --- FatturaElettronicaBody ---
    open(&mut out, 1, "FatturaElettronicaBody");
    open(&mut out, 2, "DatiGenerali");
    open(&mut out, 3, "DatiGeneraliDocumento");
    el(&mut out, 4, "TipoDocumento", tipo_documento);
    el(&mut out, 4, "Divisa", document.currency.as_str());
    el(&mut out, 4, "Data", document.issue_date.as_str());
    el(&mut out, 4, "Numero", document.document_number.as_str());
    // `ImportoTotaleDocumento` is the document grand total (gross), a standard
    // FatturaPA v1.2 element. It sits after the core fields in the XSD order;
    // the intervening optional elements (DatiRitenuta, DatiBollo, …) are not
    // emitted, so placing it directly before the close is order-correct.
    el(
        &mut out,
        4,
        "ImportoTotaleDocumento",
        &fmt_amount(document.monetary_total.payable_amount.inner()),
    );
    close(&mut out, 3, "DatiGeneraliDocumento");
    close(&mut out, 2, "DatiGenerali");

    open(&mut out, 2, "DatiBeniServizi");
    for (index, line) in document.lines.iter().enumerate() {
        let aliquota = line_tax_rate(document, line);
        open(&mut out, 3, "DettaglioLinee");
        el(&mut out, 4, "NumeroLinea", &(index + 1).to_string());
        el(&mut out, 4, "Descrizione", &line.description);
        el(&mut out, 4, "Quantita", &fmt_amount(line.quantity.inner()));
        // `UnitaMisura` follows `Quantita` in the FatturaPA `DettaglioLinee`
        // order; emit it from the IR line unit code when present.
        if let Some(unit) = &line.unit_code {
            el(&mut out, 4, "UnitaMisura", unit);
        }
        el(&mut out, 4, "PrezzoUnitario", &fmt_amount(line.unit_price.inner()));
        el(
            &mut out,
            4,
            "PrezzoTotale",
            &fmt_amount(line.line_extension_amount.inner()),
        );
        el(&mut out, 4, "AliquotaIVA", &aliquota);
        close(&mut out, 3, "DettaglioLinee");
    }
    for summary in &document.tax_summary {
        let rate = summary
            .tax_rate
            .as_ref()
            .map_or_else(|| "0.00".to_owned(), |r| fmt_amount(r.inner()));
        open(&mut out, 3, "DatiRiepilogo");
        el(&mut out, 4, "AliquotaIVA", &rate);
        el(
            &mut out,
            4,
            "ImponibileImporto",
            &fmt_amount(summary.taxable_amount.inner()),
        );
        el(&mut out, 4, "Imposta", &fmt_amount(summary.tax_amount.inner()));
        close(&mut out, 3, "DatiRiepilogo");
    }
    close(&mut out, 2, "DatiBeniServizi");
    close(&mut out, 1, "FatturaElettronicaBody");

    out.push_str("</p:FatturaElettronica>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to a FatturaPA `TipoDocumento` code.
fn tipo_documento(document_type: DocumentType) -> Result<&'static str, FatturaPaError> {
    match document_type {
        DocumentType::Invoice => Ok("TD01"),
        DocumentType::CreditNote => Ok("TD04"),
        DocumentType::DebitNote => Ok("TD05"),
        other @ (DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(FatturaPaError::UnsupportedDocumentType(other))
        }
    }
}

/// Extract `(IdPaese, IdCodice)` from a party: prefer a `vat` scheme id, else
/// the first tax id. The country prefix is stripped from the code when present.
fn party_fiscal_id(party: &Party) -> Option<(String, String)> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("vat"))
        .or_else(|| party.tax_ids.first())?;
    let country = party.address.country.as_str().to_owned();
    let code = strip_country_prefix(&chosen.value, &country);
    Some((country, code))
}

/// Strip a leading 2-letter country prefix from a VAT value (`"IT0123"` -> `"0123"`).
fn strip_country_prefix(value: &str, country: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() > 2
        && value.get(0..2).is_some_and(|p| p.eq_ignore_ascii_case(country))
        && bytes[..2].iter().all(u8::is_ascii_alphabetic)
    {
        value[2..].to_owned()
    } else {
        value.to_owned()
    }
}

/// Write a `CedentePrestatore` / `CessionarioCommittente` block. The supplier
/// requires a fiscal id; the customer's is optional (foreign / consumer).
fn write_party(
    out: &mut String,
    tag: &str,
    party: &Party,
    require_fiscal_id: bool,
) -> Result<(), FatturaPaError> {
    let fiscal = party_fiscal_id(party);
    if require_fiscal_id && fiscal.is_none() {
        return Err(FatturaPaError::MissingSupplierTaxId);
    }
    open(out, 2, tag);
    open(out, 3, "DatiAnagrafici");
    if let Some((country, code)) = &fiscal {
        open(out, 4, "IdFiscaleIVA");
        el(out, 5, "IdPaese", country);
        el(out, 5, "IdCodice", code);
        close(out, 4, "IdFiscaleIVA");
    }
    open(out, 4, "Anagrafica");
    el(out, 5, "Denominazione", &party.name);
    close(out, 4, "Anagrafica");
    if tag == "CedentePrestatore" {
        // RegimeFiscale is mandatory on the supplier; RF01 = ordinary regime.
        el(out, 4, "RegimeFiscale", "RF01");
    }
    close(out, 3, "DatiAnagrafici");

    open(out, 3, "Sede");
    el(out, 4, "Indirizzo", &party.address.lines.join(", "));
    el(out, 4, "CAP", &party.address.postal_code);
    el(out, 4, "Comune", &party.address.city);
    if let Some(prov) = province_code(party) {
        el(out, 4, "Provincia", &prov);
    }
    el(out, 4, "Nazione", party.address.country.as_str());
    close(out, 3, "Sede");
    close(out, 2, tag);
    Ok(())
}

/// A 2-letter uppercase `Provincia` derived from the address subdivision, if any.
fn province_code(party: &Party) -> Option<String> {
    let sub = party.address.subdivision.as_deref()?;
    let trimmed = sub.trim();
    (trimmed.len() == 2 && trimmed.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| trimmed.to_ascii_uppercase())
}

/// The line's `AliquotaIVA` percentage, looked up from the tax summary entry
/// matching the line's tax category, defaulting to `"0.00"`.
fn line_tax_rate(document: &CommercialDocument, line: &invoicekit_ir::DocumentLine) -> String {
    line.tax_category
        .as_ref()
        .and_then(|cat| {
            document
                .tax_summary
                .iter()
                .find(|s| &s.category_code == cat)
                .and_then(|s| s.tax_rate.as_ref())
        })
        .map_or_else(|| "0.00".to_owned(), |r| fmt_amount(r.inner()))
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
// SDI report adapter (validate -> sign -> transmit -> typed receipt)
// ---------------------------------------------------------------------------

/// SDI runtime environment selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdiEnvironment {
    /// SDI test environment.
    Sandbox,
    /// SDI production environment.
    Production,
}

/// Operator-facing SDI report request. The FatturaPA XML is produced upstream
/// by [`to_fattura_pa_xml`]; this request carries it plus the identity fields
/// SDI needs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SdiEnvironment,
    /// Issuer Partita IVA (11 digits) or Codice Fiscale (16 alphanumeric).
    pub issuer_tax_id: String,
    /// `ProgressivoInvio` — 1..=5 alphanumeric chars.
    pub progressivo_invio: String,
    /// Transport channel (web-service or PEC).
    pub transport: SdiTransport,
    /// Qualified certificate used to XAdES-sign the FatturaPA.
    pub certificate: ArubaQualifiedCertificate,
    /// Canonical FatturaPA XML bytes.
    pub fattura_xml: Vec<u8>,
}

/// Typed SDI receipt (the audit-relevant verdict and signature metadata).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiReportEnvelope {
    /// `IdentificativoSdI` routing id SDI assigns.
    pub identificativo_sdi: String,
    /// Receipt kind SDI returned (RC/NS/MC/NE/MT).
    pub receipt_kind: SdiReceiptKind,
    /// Echoed `ProgressivoInvio`.
    pub progressivo_invio: String,
    /// RFC-3339 UTC timestamp SDI recorded.
    pub recorded_at: String,
    /// XAdES signature receipt over the FatturaPA bytes.
    pub signature: Signature,
    /// Reason text when the receipt is a rejection (NS).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The full result of a report: the receipt plus the signed FatturaPA bytes
/// (the latter is an evidence-bundle artefact, kept out of the receipt JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SdiReport {
    /// Audit receipt.
    pub envelope: SdiReportEnvelope,
    /// Signed (XAdES-wrapped) FatturaPA XML bytes.
    pub signed_fattura_xml: Vec<u8>,
}

/// Typed SDI report errors. Three buckets: payload shape, country-id shape,
/// and transport. A rejection verdict (NS) is **not** here — it is an `Ok`
/// envelope with [`SdiReceiptKind::NotificaScarto`].
#[derive(Debug, Error)]
pub enum SdiReportError {
    /// The FatturaPA payload failed shape validation before the wire.
    #[error("fattura xml rejected: {0}")]
    BadXml(String),
    /// The issuer tax id did not match the expected P.IVA / CF shape.
    #[error("invalid issuer tax id: {0}")]
    BadTaxId(String),
    /// The `ProgressivoInvio` did not match the expected shape.
    #[error("invalid progressivo invio: {0}")]
    BadProgressivo(String),
    /// The SDI signer/transport failed on the wire.
    #[error("sdi signer/transport failure: {0}")]
    Transport(String),
}

/// The SDI report surface every integration (Aruba, Infocert, ...) implements.
pub trait SdiReportProvider: Send + Sync {
    /// Validate the issuer identity, sign the FatturaPA, transmit to SDI, and
    /// return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`SdiReportError`] on pre-wire shape failures (bad tax id, bad
    /// progressivo, empty payload) or transport faults. A `Notifica di Scarto`
    /// rejection is surfaced as an `Ok` envelope, not an error.
    fn report(&self, request: &SdiReportRequest) -> Result<SdiReport, SdiReportError>;
}

/// Deterministic offline SDI report provider.
///
/// Composes [`invoicekit_signer_sdi::MockSdiProvider`] so the real XAdES
/// signature path and `IdentificativoSdI` synthesis are exercised rather than
/// re-implemented.
pub struct MockSdiReportProvider {
    signer: Arc<dyn Signer>,
    forced_receipt: SdiReceiptKind,
    fixed_recorded_at: String,
}

impl MockSdiReportProvider {
    /// Build a mock report provider over the given signer (key it by the
    /// certificate serial number, e.g.
    /// `SoftwareSigner::new().with_key(serial, [2u8; 32])`).
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self {
            signer,
            forced_receipt: SdiReceiptKind::RicevutaConsegna,
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every submission to return a specific receipt kind (e.g.
    /// [`SdiReceiptKind::NotificaScarto`] to exercise the rejection path).
    #[must_use]
    pub fn with_forced_receipt(mut self, receipt: SdiReceiptKind) -> Self {
        self.forced_receipt = receipt;
        self
    }
}

impl SdiReportProvider for MockSdiReportProvider {
    fn report(&self, request: &SdiReportRequest) -> Result<SdiReport, SdiReportError> {
        validate_italian_tax_id(&request.issuer_tax_id)?;
        validate_progressivo(&request.progressivo_invio)?;
        if request.fattura_xml.is_empty() {
            return Err(SdiReportError::BadXml("payload is empty".to_owned()));
        }
        let inner = MockSdiProvider::new("aruba-test", Arc::clone(&self.signer))
            .with_forced_receipt(self.forced_receipt);
        let stamp = inner
            .submit(&SdiSubmitRequest {
                fattura_xml: request.fattura_xml.clone(),
                certificate: request.certificate.clone(),
                transport: request.transport,
                progressivo_invio: request.progressivo_invio.clone(),
            })
            .map_err(|e| SdiReportError::Transport(e.to_string()))?;
        let reason = (stamp.receipt_kind == SdiReceiptKind::NotificaScarto)
            .then(|| "SDI rejected the invoice (Notifica di Scarto)".to_owned());
        Ok(SdiReport {
            envelope: SdiReportEnvelope {
                identificativo_sdi: stamp.identificativo_sdi,
                receipt_kind: stamp.receipt_kind,
                progressivo_invio: stamp.progressivo_invio,
                recorded_at: self.fixed_recorded_at.clone(),
                signature: stamp.signature,
                reason,
            },
            signed_fattura_xml: stamp.signed_fattura_xml,
        })
    }
}

/// Validate an Italian issuer tax id: Partita IVA (11 digits) or Codice Fiscale
/// (16 alphanumeric).
///
/// # Errors
///
/// Returns [`SdiReportError::BadTaxId`] when the value matches neither shape.
pub fn validate_italian_tax_id(id: &str) -> Result<(), SdiReportError> {
    let piva = id.len() == 11 && id.bytes().all(|b| b.is_ascii_digit());
    let cf = id.len() == 16 && id.bytes().all(|b| b.is_ascii_alphanumeric());
    if piva || cf {
        Ok(())
    } else {
        Err(SdiReportError::BadTaxId(format!(
            "expected 11-digit Partita IVA or 16-char Codice Fiscale, got {id:?}"
        )))
    }
}

/// Validate a `ProgressivoInvio`: 1..=5 alphanumeric characters.
///
/// # Errors
///
/// Returns [`SdiReportError::BadProgressivo`] on shape failure.
pub fn validate_progressivo(progressivo: &str) -> Result<(), SdiReportError> {
    if (1..=5).contains(&progressivo.len())
        && progressivo.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        Ok(())
    } else {
        Err(SdiReportError::BadProgressivo(format!(
            "ProgressivoInvio must be 1..=5 alphanumeric chars, got {progressivo:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_report_it_sdi::crate_name(), "invoicekit-report-it-sdi");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-it-sdi"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, Iso4217Code, MonetaryTotal, Party,
        PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
    };
    use invoicekit_signer::SoftwareSigner;

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn italian_party(name: &str, vat: &str, city: &str) -> Party {
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
                subdivision: Some("RM".to_owned()),
                postal_code: "00100".to_owned(),
                country: CountryCode::new("IT").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-it-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("INV-2026-0001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: italian_party("Acme SRL", "IT12345678901", "Roma"),
            customer: italian_party("Beta SpA", "IT98765432109", "Milano"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Consulenza & sviluppo".to_owned(),
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
                tenant_id: "tenant_123".to_owned(),
                trace_id: "trace_abc".to_owned(),
                source_system: Some("e2e".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_cert() -> ArubaQualifiedCertificate {
        ArubaQualifiedCertificate {
            serial_number: "1234567890ABCDEF".to_owned(),
            codice_fiscale: "RSSMRA80A01H501U".to_owned(),
            subject_dn: "CN=Mario Rossi,O=Acme SRL,C=IT".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn provider() -> MockSdiReportProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key("1234567890ABCDEF", [2_u8; 32]));
        MockSdiReportProvider::new(signer)
    }

    fn sample_request(fattura_xml: Vec<u8>) -> SdiReportRequest {
        SdiReportRequest {
            tenant_id: "tenant_123".to_owned(),
            environment: SdiEnvironment::Sandbox,
            issuer_tax_id: "12345678901".to_owned(),
            progressivo_invio: "ABCDE".to_owned(),
            transport: SdiTransport::WebService,
            certificate: sample_cert(),
            fattura_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-it-sdi");
    }

    #[test]
    fn fatturapa_contains_mandatory_structure() {
        let xml = to_fattura_pa_xml(&sample_invoice(), &FatturaPaContext::default()).unwrap();
        for needle in [
            "<p:FatturaElettronica versione=\"FPR12\"",
            "<FatturaElettronicaHeader>",
            "<CedentePrestatore>",
            "<IdCodice>12345678901</IdCodice>",
            "<Denominazione>Acme SRL</Denominazione>",
            "<CessionarioCommittente>",
            "<TipoDocumento>TD01</TipoDocumento>",
            "<Divisa>EUR</Divisa>",
            "<Numero>INV-2026-0001</Numero>",
            "<ImportoTotaleDocumento>122.00</ImportoTotaleDocumento>",
            "<Descrizione>Consulenza &amp; sviluppo</Descrizione>",
            "<UnitaMisura>C62</UnitaMisura>",
            "<AliquotaIVA>22.00</AliquotaIVA>",
            "<ImponibileImporto>100.00</ImponibileImporto>",
            "<Imposta>22.00</Imposta>",
        ] {
            assert!(xml.contains(needle), "FatturaPA missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn fatturapa_is_deterministic() {
        let doc = sample_invoice();
        let ctx = FatturaPaContext::default();
        assert_eq!(
            to_fattura_pa_xml(&doc, &ctx).unwrap(),
            to_fattura_pa_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn fatturapa_rejects_unsupported_document_type() {
        let err = tipo_documento(DocumentType::ProForma).unwrap_err();
        assert!(matches!(err, FatturaPaError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn report_happy_path_is_delivered() {
        let xml = to_fattura_pa_xml(&sample_invoice(), &FatturaPaContext::default())
            .unwrap()
            .into_bytes();
        let report = provider().report(&sample_request(xml)).unwrap();
        assert!(report.envelope.receipt_kind.is_delivered());
        assert!(report.envelope.identificativo_sdi.starts_with("IT"));
        assert!(report.envelope.reason.is_none());
        assert!(report.signed_fattura_xml.starts_with(b"<XAdES-stub>"));
    }

    #[test]
    fn report_rejection_is_ok_not_err() {
        let xml = b"<FatturaElettronica/>".to_vec();
        let provider = provider().with_forced_receipt(SdiReceiptKind::NotificaScarto);
        let report = provider.report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.receipt_kind, SdiReceiptKind::NotificaScarto);
        assert!(!report.envelope.receipt_kind.is_delivered());
        assert!(report.envelope.reason.is_some());
    }

    #[test]
    fn report_rejects_bad_tax_id() {
        let mut req = sample_request(b"<x/>".to_vec());
        req.issuer_tax_id = "BAD".to_owned();
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            SdiReportError::BadTaxId(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new());
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            SdiReportError::BadXml(_)
        ));
    }

    #[test]
    fn tax_id_validator_shapes() {
        assert!(validate_italian_tax_id("12345678901").is_ok()); // P.IVA 11 digits
        assert!(validate_italian_tax_id("RSSMRA80A01H501U").is_ok()); // CF 16 alnum
        assert!(validate_italian_tax_id("1234567890").is_err()); // 10 digits
        assert!(validate_italian_tax_id("123456789012").is_err()); // 12 digits
        assert!(validate_italian_tax_id("RSSMRA80A01H501").is_err()); // 15 chars
    }

    #[test]
    fn progressivo_validator_shapes() {
        assert!(validate_progressivo("A").is_ok());
        assert!(validate_progressivo("ABCDE").is_ok());
        assert!(validate_progressivo("").is_err());
        assert!(validate_progressivo("ABCDEF").is_err());
        assert!(validate_progressivo("AB-DE").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_fattura_pa_xml(&sample_invoice(), &FatturaPaContext::default())
            .unwrap()
            .into_bytes();
        let env = provider().report(&sample_request(xml)).unwrap().envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: SdiReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }
}
