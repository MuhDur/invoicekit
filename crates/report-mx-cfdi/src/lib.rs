// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// CFDI / SAT acronyms (CFDI, SAT, PAC, RFC, UUID, CSD, TFD,
// IVA, ISR, IEPS, XSLT) trip doc-markdown; suppress it crate-wide.
#![allow(clippy::doc_markdown)]

//! Mexico **CFDI 4.0** national-clearance report adapter.
//!
//! Mexico is a *national-clearance* jurisdiction. A B2B/B2C invoice is
//! serialized to the national **CFDI** (`cfdi:Comprobante`) XML, the taxpayer
//! seals it with their Certificado de Sello Digital (CSD), and a
//! Proveedor Autorizado de Certificación (PAC) stamps it — the *timbrado* —
//! adding the SAT **Timbre Fiscal Digital** (TFD): a UUID (Folio Fiscal) plus
//! the SAT seal. This crate provides the offline (local-only) end-to-end
//! lifecycle:
//!
//! 1. **serialize** — [`to_cfdi_xml`] turns an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] into deterministic CFDI 4.0 XML
//!    (`cfdi:Comprobante` with `Emisor` / `Receptor` / `Conceptos` /
//!    `Impuestos`). UBL/CII serializers do *not* emit this national format.
//! 2. **validate (local)** — [`validate_rfc`] and [`validate_folio_fiscal`]
//!    enforce the real RFC (12 chars personas morales / 13 personas físicas)
//!    and SAT UUID shapes; reference-grade XSD + cadena-original XSLT
//!    validation stays an external (JVM) backend and is labelled as such in
//!    the capability matrix.
//! 3. **sign + transmit** — [`MockCfdiReportProvider`] composes the already-built
//!    [`invoicekit_signer_cfdi::MockCfdiPacProvider`] so the PAC timbrado path
//!    (cadena original + selloCFDI + selloSAT + Folio Fiscal synthesis) is
//!    exercised, never re-faked.
//! 4. **evidence** — the caller bundles the canonical document, CFDI XML,
//!    stamped artifact, and receipt into a signed `.ikb` evidence bundle.
//!
//! Live PAC transmission (Solución Factible / Edicom / Facturando web service)
//! is bring-your-own-credentials and lands in a follow-up `report-mx-cfdi-http`
//! crate; this crate's `Mock*` providers are deterministic and offline.
//!
//! **Rejection is not an error.** When a PAC refuses to stamp an invoice it
//! returns a *rechazo* with a SAT validation code (e.g. `CFDI40102`) — surfaced
//! here as [`TimbradoStatus::Rechazado`] inside an `Ok(_)` envelope, never as
//! `Err`. `Err` is reserved for pre-wire shape failures and transport faults.

use std::sync::Arc;

use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use invoicekit_signer::Signer;
use invoicekit_signer_cfdi::{
    CertificadoSelloDigital, CfdiKind, CfdiPacProvider, CfdiSignRequest, MockCfdiPacProvider,
    PacEnvironment,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the CFDI signer substrate types this crate's public API surfaces,
// so downstream callers need not depend on `invoicekit-signer-cfdi` directly.
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_cfdi::{CfdiKind as CfdiComprobanteKind, PacEnvironment as CfdiEnvironment};

// ---------------------------------------------------------------------------
// CFDI serialization (IR -> national cfdi:Comprobante XML)
// ---------------------------------------------------------------------------

/// CFDI 4.0 transmission context: the comprobante-level SAT fields that live in
/// the `cfdi:Comprobante` attributes but are not part of the
/// jurisdiction-agnostic IR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CfdiContext {
    /// `LugarExpedicion` — issuer's postal code (5 digits), the place of issue.
    pub lugar_expedicion: String,
    /// `MetodoPago` — payment method: `PUE` (single exhibition) or `PPD`
    /// (deferred / instalments).
    pub metodo_pago: String,
    /// `FormaPago` — c_FormaPago catalogue code (e.g. `03` = transferencia).
    pub forma_pago: String,
    /// `Emisor.RegimenFiscal` — c_RegimenFiscal code (e.g. `601` = general de
    /// ley personas morales).
    pub regimen_fiscal: String,
    /// `Receptor.UsoCFDI` — c_UsoCFDI code (e.g. `G03` = gastos en general).
    pub uso_cfdi: String,
    /// `Receptor.DomicilioFiscalReceptor` — recipient's tax-domicile postal code.
    pub domicilio_fiscal_receptor: String,
}

impl Default for CfdiContext {
    fn default() -> Self {
        Self {
            lugar_expedicion: "00000".to_owned(),
            metodo_pago: "PUE".to_owned(),
            forma_pago: "99".to_owned(),
            regimen_fiscal: "601".to_owned(),
            uso_cfdi: "G03".to_owned(),
            domicilio_fiscal_receptor: "00000".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to CFDI 4.0 XML.
#[derive(Debug, Error)]
pub enum CfdiSerializeError {
    /// The IR `document_type` has no CFDI `TipoDeComprobante` mapping.
    #[error("document type {0:?} is not representable as CFDI TipoDeComprobante")]
    UnsupportedDocumentType(DocumentType),
    /// The supplier (Emisor) carries no usable RFC.
    #[error("supplier (Emisor) has no RFC usable as cfdi:Emisor@Rfc")]
    MissingEmisorRfc,
    /// The transmission context was malformed (e.g. blank LugarExpedicion).
    #[error("invalid CFDI context: {0}")]
    BadContext(String),
    /// Summing the document-level traslado tax amounts overflowed the
    /// `Decimal` range. The amounts are untrusted at this point, so an
    /// out-of-range sum is reported rather than allowed to panic.
    #[error("tax amount summation overflowed the decimal range")]
    AmountOverflow,
}

/// CFDI 4.0 fixed comprobante version attribute.
const CFDI_VERSION: &str = "4.0";

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic CFDI 4.0
/// (`cfdi:Comprobante`) XML.
///
/// Output is byte-stable by construction: a fixed attribute/element order with
/// no maps and amounts formatted at fixed scale 2. The document is expected to
/// have passed IR validation already (it has, if built via
/// [`CommercialDocument::new`]). The emitted XML is *pre-timbrado* — it carries
/// no `Sello`/`Certificado` and no `TimbreFiscalDigital`; those are added by the
/// PAC during [`MockCfdiReportProvider::report`].
///
/// # Errors
///
/// Returns [`CfdiSerializeError::UnsupportedDocumentType`] for document types
/// with no `TipoDeComprobante` mapping, [`CfdiSerializeError::MissingEmisorRfc`]
/// when the supplier has no RFC, and [`CfdiSerializeError::BadContext`] when the
/// context is malformed.
pub fn to_cfdi_xml(
    document: &CommercialDocument,
    context: &CfdiContext,
) -> Result<String, CfdiSerializeError> {
    if context.lugar_expedicion.is_empty() {
        return Err(CfdiSerializeError::BadContext(
            "LugarExpedicion must not be empty".to_owned(),
        ));
    }
    let tipo = tipo_de_comprobante(document.document_type)?;
    let emisor_rfc = party_rfc(&document.supplier).ok_or(CfdiSerializeError::MissingEmisorRfc)?;
    let receptor_rfc = party_rfc(&document.customer).unwrap_or_else(|| "XAXX010101000".to_owned());

    let subtotal = fmt_amount(document.monetary_total.tax_exclusive_amount.inner());
    let total = fmt_amount(document.monetary_total.tax_inclusive_amount.inner());

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // cfdi:Comprobante root with the mandatory 4.0 attribute spine.
    out.push_str("<cfdi:Comprobante");
    out.push_str(" xmlns:cfdi=\"http://www.sat.gob.mx/cfd/4\"");
    attr(&mut out, "Version", CFDI_VERSION);
    attr(&mut out, "TipoDeComprobante", tipo);
    attr(&mut out, "Fecha", &cfdi_fecha(document.issue_date.as_str()));
    attr(&mut out, "Folio", document.document_number.as_str());
    attr(&mut out, "Moneda", document.currency.as_str());
    attr(&mut out, "SubTotal", &subtotal);
    attr(&mut out, "Total", &total);
    attr(&mut out, "Exportacion", "01");
    attr(&mut out, "LugarExpedicion", &context.lugar_expedicion);
    attr(&mut out, "MetodoPago", &context.metodo_pago);
    attr(&mut out, "FormaPago", &context.forma_pago);
    out.push_str(">\n");

    // --- cfdi:Emisor ---
    out.push_str("  <cfdi:Emisor");
    attr(&mut out, "Rfc", &emisor_rfc);
    attr(&mut out, "Nombre", &document.supplier.name);
    attr(&mut out, "RegimenFiscal", &context.regimen_fiscal);
    out.push_str("/>\n");

    // --- cfdi:Receptor ---
    out.push_str("  <cfdi:Receptor");
    attr(&mut out, "Rfc", &receptor_rfc);
    attr(&mut out, "Nombre", &document.customer.name);
    attr(
        &mut out,
        "DomicilioFiscalReceptor",
        &context.domicilio_fiscal_receptor,
    );
    attr(&mut out, "RegimenFiscalReceptor", &context.regimen_fiscal);
    attr(&mut out, "UsoCFDI", &context.uso_cfdi);
    out.push_str("/>\n");

    // --- cfdi:Conceptos ---
    out.push_str("  <cfdi:Conceptos>\n");
    for line in &document.lines {
        let importe = fmt_amount(line.line_extension_amount.inner());
        let valor_unitario = fmt_amount(line.unit_price.inner());
        let cantidad = fmt_amount(line.quantity.inner());
        out.push_str("    <cfdi:Concepto");
        // ClaveProdServ / ClaveUnidad are SAT product/unit catalogue codes.
        // Prefer the line's own SAT product/service key (EN 16931 BT-158 with
        // scheme_id "ClaveProdServ"); fall back to the generic catch-all when
        // the line carries no such classification.
        attr(
            &mut out,
            "ClaveProdServ",
            clave_prod_serv(line).unwrap_or("01010101"),
        );
        attr(&mut out, "ClaveUnidad", line.unit_code.as_deref().unwrap_or("H87"));
        attr(&mut out, "Cantidad", &cantidad);
        attr(&mut out, "Descripcion", &line.description);
        attr(&mut out, "ValorUnitario", &valor_unitario);
        attr(&mut out, "Importe", &importe);
        attr(&mut out, "ObjetoImp", "02");
        out.push_str(">\n");
        // Per-concepto Impuestos/Traslados carrying the line's IVA.
        if let Some((rate, tax)) = line_tax(document, line) {
            out.push_str("      <cfdi:Impuestos>\n");
            out.push_str("        <cfdi:Traslados>\n");
            out.push_str("          <cfdi:Traslado");
            attr(&mut out, "Base", &importe);
            attr(&mut out, "Impuesto", "002"); // 002 = IVA
            attr(&mut out, "TipoFactor", "Tasa");
            attr(&mut out, "TasaOCuota", &fmt_rate(rate));
            attr(&mut out, "Importe", &fmt_amount(tax));
            out.push_str("/>\n");
            out.push_str("        </cfdi:Traslados>\n");
            out.push_str("      </cfdi:Impuestos>\n");
        }
        out.push_str("    </cfdi:Concepto>\n");
    }
    out.push_str("  </cfdi:Conceptos>\n");

    // --- cfdi:Impuestos (document-level totals) ---
    out.push_str("  <cfdi:Impuestos");
    attr(
        &mut out,
        "TotalImpuestosTrasladados",
        &fmt_amount(total_traslados(document)?),
    );
    out.push_str(">\n");
    out.push_str("    <cfdi:Traslados>\n");
    for summary in &document.tax_summary {
        let rate = summary_rate(summary);
        out.push_str("      <cfdi:Traslado");
        attr(&mut out, "Base", &fmt_amount(summary.taxable_amount.inner()));
        attr(&mut out, "Impuesto", "002");
        attr(&mut out, "TipoFactor", "Tasa");
        attr(&mut out, "TasaOCuota", &fmt_rate(rate));
        attr(&mut out, "Importe", &fmt_amount(summary.tax_amount.inner()));
        out.push_str("/>\n");
    }
    out.push_str("    </cfdi:Traslados>\n");
    out.push_str("  </cfdi:Impuestos>\n");

    out.push_str("</cfdi:Comprobante>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to a CFDI `TipoDeComprobante` code.
///
/// `I` = Ingreso (invoice), `E` = Egreso (credit note). CFDI has no debit-note
/// comprobante type (debit adjustments are issued as a fresh Ingreso), and
/// pro-forma / self-billed documents are not CFDI-representable.
fn tipo_de_comprobante(document_type: DocumentType) -> Result<&'static str, CfdiSerializeError> {
    match document_type {
        DocumentType::Invoice => Ok("I"),
        DocumentType::CreditNote => Ok("E"),
        other @ (DocumentType::DebitNote
        | DocumentType::ProForma
        | DocumentType::SelfBilled) => Err(CfdiSerializeError::UnsupportedDocumentType(other)),
    }
}

/// Map the IR [`CfdiComprobanteKind`] from the document type, for the PAC
/// stamping request.
fn comprobante_kind(document_type: DocumentType) -> CfdiKind {
    match document_type {
        DocumentType::CreditNote => CfdiKind::Egreso,
        _ => CfdiKind::Ingreso,
    }
}

/// Extract a usable RFC from a party: prefer a `rfc`-scheme id, else a
/// `vat`-scheme id, else the first tax id, with any leading `MX` prefix
/// stripped.
fn party_rfc(party: &Party) -> Option<String> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("rfc"))
        .or_else(|| party.tax_ids.iter().find(|t| t.scheme.eq_ignore_ascii_case("vat")))
        .or_else(|| party.tax_ids.first())?;
    Some(strip_mx_prefix(&chosen.value))
}

/// Strip a leading `MX` country prefix from a tax-id value (`"MXAAA010101AAA"`
/// -> `"AAA010101AAA"`). The RFC itself never starts with a country code.
fn strip_mx_prefix(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() > 2 && value.get(0..2).is_some_and(|p| p.eq_ignore_ascii_case("MX")) {
        value[2..].to_owned()
    } else {
        value.to_owned()
    }
}

/// The line's SAT product/service key (`ClaveProdServ`) from its EN 16931
/// BT-158 classifications.
///
/// Selects the classification whose `scheme_id` is `ClaveProdServ`
/// (case-insensitive — `c_ClaveProdServ` is the SAT catalogue, but UBL/CII
/// `listID` casing varies in the wild). Returns `None` when the line carries no
/// such classification, so the caller can keep the generic catch-all key.
fn clave_prod_serv(line: &invoicekit_ir::DocumentLine) -> Option<&str> {
    line.classifications
        .iter()
        .find(|c| c.scheme_id.eq_ignore_ascii_case("ClaveProdServ"))
        .map(|c| c.code.as_str())
}

/// The `(rate, tax_amount)` for a line, looked up from the tax summary entry
/// matching the line's tax category.
fn line_tax(document: &CommercialDocument, line: &invoicekit_ir::DocumentLine) -> Option<(Decimal, Decimal)> {
    let cat = line.tax_category.as_ref()?;
    let summary = document.tax_summary.iter().find(|s| &s.category_code == cat)?;
    Some((summary_rate(summary), summary.tax_amount.inner()))
}

/// The IVA rate of a tax-summary entry as a [`Decimal`], defaulting to zero when
/// the entry carries no explicit rate (e.g. exempt categories).
fn summary_rate(summary: &invoicekit_ir::TaxCategorySummary) -> Decimal {
    summary
        .tax_rate
        .as_ref()
        .map_or(Decimal::ZERO, invoicekit_ir::DecimalValue::inner)
}

/// Sum the document-level traslado tax amounts with checked addition.
///
/// The tax-summary amounts are untrusted at this point, so the sum is
/// accumulated with [`Decimal::checked_add`] rather than the panicking `Sum`
/// impl; an out-of-range total yields [`CfdiSerializeError::AmountOverflow`].
fn total_traslados(document: &CommercialDocument) -> Result<Decimal, CfdiSerializeError> {
    document
        .tax_summary
        .iter()
        .try_fold(Decimal::ZERO, |acc, s| {
            acc.checked_add(s.tax_amount.inner())
                .ok_or(CfdiSerializeError::AmountOverflow)
        })
}

/// Format a `Fecha`: CFDI 4.0 wants `YYYY-MM-DDThh:mm:ss` (no timezone). The IR
/// carries a date-only `YYYY-MM-DD`; pin the time-of-day to midnight so the
/// output stays byte-stable.
fn cfdi_fecha(issue_date: &str) -> String {
    format!("{issue_date}T00:00:00")
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`), deterministic.
fn fmt_amount(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

/// Format an IVA rate as the SAT `TasaOCuota` fraction at fixed scale 6 (a
/// `16.00` percentage -> `"0.160000"`), deterministic. `round_dp` alone drops
/// trailing zeros, so re-pin the scale to 6 afterwards.
fn fmt_rate(percent: Decimal) -> String {
    let fraction = (percent / Decimal::from(100)).round_dp(6);
    format!("{fraction:.6}")
}

/// Append ` Name="escaped-value"` to an open element tag.
fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_escaped(out, value);
    out.push('"');
}

/// Append XML-escaped attribute-value content.
///
/// Besides the markup-significant characters (`&`, `<`, `>`, `"`, `'`), this
/// also emits numeric character references for tab, newline, and carriage
/// return. In attribute context an XML processor performs attribute-value
/// normalization, replacing a literal tab/newline/CR with a space (0x20) before
/// the value reaches the application. Free-text names and line descriptions can
/// legitimately contain those characters, so they are escaped here to survive
/// the round trip — matching the canonical UBL/CII serializers (`&#x9;`,
/// `&#xA;`, `&#xD;`).
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}

// ---------------------------------------------------------------------------
// CFDI report adapter (validate -> sign -> timbrar -> typed receipt)
// ---------------------------------------------------------------------------

/// The PAC timbrado verdict — the authority-equivalent status SAT/the PAC
/// returns for a CFDI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimbradoStatus {
    /// The CFDI was stamped: a UUID (Folio Fiscal) + TFD were issued.
    Timbrado,
    /// The PAC refused to stamp the CFDI (a SAT validation error). This is a
    /// *receipt status*, not an `Err`.
    Rechazado,
}

impl TimbradoStatus {
    /// Whether the verdict is a successful stamp (true only for
    /// [`TimbradoStatus::Timbrado`]).
    #[must_use]
    pub const fn is_stamped(self) -> bool {
        matches!(self, Self::Timbrado)
    }
}

/// Operator-facing CFDI report request. The CFDI XML is produced upstream by
/// [`to_cfdi_xml`]; this request carries it plus the identity fields the PAC
/// needs to timbrar.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CfdiReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// PAC environment selector (sandbox vs production).
    pub environment: PacEnvironment,
    /// Issuer RFC (12 chars personas morales / 13 personas físicas).
    pub issuer_rfc: String,
    /// CFDI comprobante kind being stamped (Ingreso/Egreso/...).
    pub kind: CfdiKind,
    /// Taxpayer's Certificado de Sello Digital used to compute the selloCFDI.
    pub csd: CertificadoSelloDigital,
    /// Canonical CFDI 4.0 XML bytes (pre-timbrado).
    pub cfdi_xml: Vec<u8>,
}

/// Typed CFDI receipt: the audit-relevant verdict and SAT stamp metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CfdiReportEnvelope {
    /// PAC timbrado verdict.
    pub status: TimbradoStatus,
    /// `UUID` (Folio Fiscal) the SAT assigns when stamped; empty on rejection.
    pub folio_fiscal: String,
    /// PAC's certificate serial number that signed the TFD.
    pub pac_certificate_serial: String,
    /// `FechaTimbrado` (RFC-3339 UTC) the PAC recorded.
    pub fecha_timbrado: String,
    /// RFC-3339 UTC timestamp this adapter recorded the verdict.
    pub recorded_at: String,
    /// `selloSAT` — the SAT/PAC outer seal (base64) when stamped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sello_sat: Option<String>,
    /// Reason / SAT validation code text when the verdict is a rejection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The TFD seal receipt (selloCFDI) over the cadena original.
    pub signature: Signature,
}

/// The full result of a report: the receipt plus the timbrado (TFD-wrapped)
/// CFDI bytes (kept out of the receipt JSON; an evidence-bundle artifact).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CfdiReport {
    /// Audit receipt.
    pub envelope: CfdiReportEnvelope,
    /// The stamped CFDI XML bytes (the original comprobante with the
    /// `TimbreFiscalDigital` complemento appended).
    pub timbrado_xml: Vec<u8>,
}

/// Typed CFDI report errors. Three buckets: payload shape, country-id shape,
/// and transport. A rejection verdict (rechazo) is **not** here — it is an `Ok`
/// envelope with [`TimbradoStatus::Rechazado`].
#[derive(Debug, Error)]
pub enum CfdiReportError {
    /// The CFDI payload failed shape validation before the wire.
    #[error("cfdi xml rejected: {0}")]
    BadXml(String),
    /// The issuer RFC did not match the expected 12/13-char shape.
    #[error("invalid issuer RFC: {0}")]
    BadRfc(String),
    /// The PAC signer/transport failed on the wire.
    #[error("pac signer/transport failure: {0}")]
    Transport(String),
}

/// The CFDI report surface every PAC integration (Solución Factible, Edicom,
/// ...) implements.
pub trait CfdiReportProvider: Send + Sync {
    /// Validate the issuer identity, seal + stamp the CFDI through the PAC, and
    /// return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`CfdiReportError`] on pre-wire shape failures (bad RFC, empty
    /// payload) or transport faults. A PAC *rechazo* (rejection) is surfaced as
    /// an `Ok` envelope with [`TimbradoStatus::Rechazado`], not an error.
    fn report(&self, request: &CfdiReportRequest) -> Result<CfdiReport, CfdiReportError>;
}

/// Deterministic offline CFDI report provider.
///
/// Composes [`invoicekit_signer_cfdi::MockCfdiPacProvider`] so the real PAC
/// timbrado path (cadena original + selloCFDI + selloSAT + Folio Fiscal
/// synthesis) is exercised rather than re-implemented.
pub struct MockCfdiReportProvider {
    signer: Arc<dyn Signer>,
    pac_certificate_serial: String,
    rejected: bool,
    fixed_recorded_at: String,
    fixed_fecha_timbrado: String,
}

impl MockCfdiReportProvider {
    /// Build a mock report provider over the given signer.
    ///
    /// Key the signer by the CSD serial number, e.g.
    /// `SoftwareSigner::new().with_key(serial, [4u8; 32])`.
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self {
            signer,
            pac_certificate_serial: "30001000000400002434".to_owned(),
            rejected: false,
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            fixed_fecha_timbrado: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every submission to be rejected by the PAC (exercises the
    /// [`TimbradoStatus::Rechazado`] path).
    #[must_use]
    pub fn with_rejection(mut self) -> Self {
        self.rejected = true;
        self
    }

    /// Override the PAC certificate serial number the mock stamps with.
    #[must_use]
    pub fn with_pac_certificate_serial(mut self, serial: impl Into<String>) -> Self {
        self.pac_certificate_serial = serial.into();
        self
    }
}

impl CfdiReportProvider for MockCfdiReportProvider {
    fn report(&self, request: &CfdiReportRequest) -> Result<CfdiReport, CfdiReportError> {
        validate_rfc(&request.issuer_rfc)?;
        if request.cfdi_xml.is_empty() {
            return Err(CfdiReportError::BadXml("payload is empty".to_owned()));
        }

        let inner = MockCfdiPacProvider::new(
            "solucion-factible-test",
            request.environment,
            Arc::clone(&self.signer),
            self.pac_certificate_serial.clone(),
        )
        .with_fixed_fecha_timbrado(self.fixed_fecha_timbrado.clone());

        let stamp = inner
            .stamp(
                &CfdiSignRequest {
                    cfdi_xml: request.cfdi_xml.clone(),
                    csd: request.csd.clone(),
                    kind: request.kind,
                    metadata: std::collections::BTreeMap::new(),
                },
                request.environment,
            )
            .map_err(|e| CfdiReportError::Transport(e.to_string()))?;

        if self.rejected {
            // PAC rechazo: a SAT validation error, surfaced as a receipt
            // status inside an Ok envelope (NEVER an Err). No Folio Fiscal is
            // issued; the un-stamped comprobante is preserved for the audit
            // trail.
            return Ok(CfdiReport {
                envelope: CfdiReportEnvelope {
                    status: TimbradoStatus::Rechazado,
                    folio_fiscal: String::new(),
                    pac_certificate_serial: self.pac_certificate_serial.clone(),
                    fecha_timbrado: stamp.fecha_timbrado,
                    recorded_at: self.fixed_recorded_at.clone(),
                    sello_sat: None,
                    reason: Some(
                        "PAC rejected the CFDI (CFDI40102: sello del emisor no corresponde al CSD)"
                            .to_owned(),
                    ),
                    signature: stamp.signature,
                },
                timbrado_xml: request.cfdi_xml.clone(),
            });
        }

        let timbrado_xml = wrap_tfd_complemento(
            &request.cfdi_xml,
            &stamp.uuid,
            &stamp.sello_cfdi,
            &stamp.sello_sat,
            &stamp.fecha_timbrado,
            &self.pac_certificate_serial,
        );

        Ok(CfdiReport {
            envelope: CfdiReportEnvelope {
                status: TimbradoStatus::Timbrado,
                folio_fiscal: stamp.uuid,
                pac_certificate_serial: self.pac_certificate_serial.clone(),
                fecha_timbrado: stamp.fecha_timbrado,
                recorded_at: self.fixed_recorded_at.clone(),
                sello_sat: Some(stamp.sello_sat),
                reason: None,
                signature: stamp.signature,
            },
            timbrado_xml,
        })
    }
}

/// Append the SAT `TimbreFiscalDigital` (TFD 1.1) complemento to a CFDI body.
///
/// Real PACs insert a `cfdi:Complemento/tfd:TimbreFiscalDigital` block carrying
/// the UUID + selloCFDI + selloSAT before the closing `</cfdi:Comprobante>`.
/// The mock reproduces the element shape deterministically so the evidence
/// bundle carries an honest stamped artifact.
fn wrap_tfd_complemento(
    cfdi_xml: &[u8],
    uuid: &str,
    sello_cfdi: &str,
    sello_sat: &str,
    fecha_timbrado: &str,
    pac_serial: &str,
) -> Vec<u8> {
    let body = String::from_utf8_lossy(cfdi_xml);
    let closing = "</cfdi:Comprobante>";
    let mut tfd = String::with_capacity(512);
    tfd.push_str("  <cfdi:Complemento>\n");
    tfd.push_str("    <tfd:TimbreFiscalDigital");
    attr(&mut tfd, "xmlns:tfd", "http://www.sat.gob.mx/TimbreFiscalDigital");
    attr(&mut tfd, "Version", "1.1");
    attr(&mut tfd, "UUID", uuid);
    attr(&mut tfd, "FechaTimbrado", fecha_timbrado);
    attr(&mut tfd, "RfcProvCertif", "AAA010101AAA");
    attr(&mut tfd, "SelloCFD", sello_cfdi);
    attr(&mut tfd, "NoCertificadoSAT", pac_serial);
    attr(&mut tfd, "SelloSAT", sello_sat);
    tfd.push_str("/>\n");
    tfd.push_str("  </cfdi:Complemento>\n");

    let stamped = if let Some(idx) = body.rfind(closing) {
        let mut s = String::with_capacity(body.len() + tfd.len());
        s.push_str(&body[..idx]);
        s.push_str(&tfd);
        s.push_str(&body[idx..]);
        s
    } else {
        let mut s = body.into_owned();
        s.push_str(&tfd);
        s
    };
    stamped.into_bytes()
}

/// Validate a Mexican RFC: 12 characters for personas morales (companies),
/// 13 for personas físicas (individuals).
///
/// Shape rules enforced (real SAT structure):
/// * personas morales: 3 letters (razón social) + 6 date digits + 3 homoclave.
/// * personas físicas: 4 letters (apellidos + nombre) + 6 date digits + 3 homoclave.
/// * the homoclave is alphanumeric (`&` and `Ñ` permitted in the name prefix
///   are normalised away upstream and not accepted here).
///
/// # Errors
///
/// Returns [`CfdiReportError::BadRfc`] when the value matches neither shape.
pub fn validate_rfc(rfc: &str) -> Result<(), CfdiReportError> {
    let bad = |reason: &str| {
        Err(CfdiReportError::BadRfc(format!(
            "expected 12-char (persona moral) or 13-char (persona física) RFC, {reason}: {rfc:?}"
        )))
    };
    let len = rfc.len();
    if len != 12 && len != 13 {
        return bad("wrong length");
    }
    if !rfc.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return bad("non-alphanumeric character");
    }
    // The name prefix is 3 letters (moral) or 4 letters (física); the trailing
    // 9 characters are 6 date digits + 3-char homoclave.
    let name_len = len - 9;
    let (name, rest) = rfc.split_at(name_len);
    if !name.bytes().all(|b| b.is_ascii_alphabetic()) {
        return bad("name prefix must be letters");
    }
    let date = &rest[..6];
    if !date.bytes().all(|b| b.is_ascii_digit()) {
        return bad("date segment must be 6 digits");
    }
    Ok(())
}

/// Validate a SAT Folio Fiscal: the canonical UUID shape
/// `8-4-4-4-12` hexadecimal characters (e.g.
/// `00000000-0000-4000-8000-000000000001`).
///
/// # Errors
///
/// Returns [`CfdiReportError::BadXml`] when the value is not a 36-char
/// hyphen-grouped hex UUID.
pub fn validate_folio_fiscal(folio: &str) -> Result<(), CfdiReportError> {
    let groups: Vec<&str> = folio.split('-').collect();
    let shape_ok = groups.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(&want, g)| g.len() == want && g.bytes().all(|b| b.is_ascii_hexdigit()));
    if shape_ok {
        Ok(())
    } else {
        Err(CfdiReportError::BadXml(format!(
            "Folio Fiscal must be a 8-4-4-4-12 hex UUID, got {folio:?}"
        )))
    }
}

/// Build a [`CfdiReportRequest`] for an already-serialized CFDI, deriving the
/// comprobante kind from the document type.
///
/// # Errors
///
/// Returns [`CfdiReportError::BadRfc`] when `issuer_rfc` fails [`validate_rfc`].
pub fn report_request_for(
    document: &CommercialDocument,
    issuer_rfc: impl Into<String>,
    environment: PacEnvironment,
    csd: CertificadoSelloDigital,
    cfdi_xml: Vec<u8>,
) -> Result<CfdiReportRequest, CfdiReportError> {
    let issuer_rfc = issuer_rfc.into();
    validate_rfc(&issuer_rfc)?;
    Ok(CfdiReportRequest {
        tenant_id: document.meta.tenant_id.clone(),
        environment,
        issuer_rfc,
        kind: comprobante_kind(document.document_type),
        csd,
        cfdi_xml,
    })
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_report_mx_cfdi::crate_name(), "invoicekit-report-mx-cfdi");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-mx-cfdi"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId, DocumentLine,
        DocumentMeta, DocumentNumber, Iso4217Code, MonetaryTotal, PartyTaxId, PostalAddress,
        SchemaVersion, TaxCategorySummary,
    };
    use invoicekit_signer::SoftwareSigner;

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

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-mx-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("FAC-2026-0001").unwrap(),
            currency: Iso4217Code::new("MXN").unwrap(),
            supplier: mexican_party("Comercializadora Azteca SA de CV", "CAZ010101AAA", "Ciudad de Mexico"),
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
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(16000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
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
            allowance_charges: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant_mx".to_owned(),
                trace_id: "trace_mx".to_owned(),
                source_system: Some("e2e".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_csd() -> CertificadoSelloDigital {
        CertificadoSelloDigital {
            serial_number: CSD_SERIAL.to_owned(),
            rfc: "CAZ010101AAA".to_owned(),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-12-31T23:59:59Z".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn provider() -> MockCfdiReportProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key(CSD_SERIAL, [4_u8; 32]));
        MockCfdiReportProvider::new(signer)
    }

    fn sample_request(cfdi_xml: Vec<u8>) -> CfdiReportRequest {
        CfdiReportRequest {
            tenant_id: "tenant_mx".to_owned(),
            environment: PacEnvironment::Sandbox,
            issuer_rfc: "CAZ010101AAA".to_owned(),
            kind: CfdiKind::Ingreso,
            csd: sample_csd(),
            cfdi_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-mx-cfdi");
    }

    #[test]
    fn cfdi_contains_mandatory_structure() {
        let xml = to_cfdi_xml(&sample_invoice(), &CfdiContext::default()).unwrap();
        for needle in [
            "<cfdi:Comprobante",
            "xmlns:cfdi=\"http://www.sat.gob.mx/cfd/4\"",
            "Version=\"4.0\"",
            "TipoDeComprobante=\"I\"",
            "Moneda=\"MXN\"",
            "Folio=\"FAC-2026-0001\"",
            "<cfdi:Emisor",
            "Rfc=\"CAZ010101AAA\"",
            "<cfdi:Receptor",
            "UsoCFDI=\"G03\"",
            "<cfdi:Conceptos>",
            "<cfdi:Concepto",
            "Descripcion=\"Servicios de consultoria &amp; desarrollo\"",
            "Impuesto=\"002\"",
            "TasaOCuota=\"0.160000\"",
            "<cfdi:Impuestos",
            "TotalImpuestosTrasladados=\"160.00\"",
        ] {
            assert!(xml.contains(needle), "CFDI missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn concepto_clave_prod_serv_comes_from_classification() {
        use invoicekit_ir::ItemClassification;

        // Baseline: a line with EMPTY classifications keeps the generic SAT
        // catch-all key — behavior preservation for every existing document.
        let baseline = to_cfdi_xml(&sample_invoice(), &CfdiContext::default()).unwrap();
        assert!(
            baseline.contains("ClaveProdServ=\"01010101\""),
            "unclassified line must keep the catch-all ClaveProdServ:\n{baseline}"
        );

        // Classify the line with a real SAT product/service key (BT-158 with
        // scheme_id ClaveProdServ). It must surface on cfdi:Concepto verbatim.
        let mut doc = sample_invoice();
        doc.lines[0].classifications = vec![
            // A non-SAT scheme that must be ignored by this national report.
            ItemClassification {
                code: "0901".to_owned(),
                scheme_id: "HSN".to_owned(),
                scheme_version: Some("2017".to_owned()),
            },
            ItemClassification {
                code: "80101500".to_owned(),
                scheme_id: "ClaveProdServ".to_owned(),
                scheme_version: None,
            },
        ];
        let xml = to_cfdi_xml(&doc, &CfdiContext::default()).unwrap();
        assert!(
            xml.contains("ClaveProdServ=\"80101500\""),
            "ClaveProdServ must carry the classification code:\n{xml}"
        );
        // The generic catch-all key must NOT appear once a real key is present.
        assert!(
            !xml.contains("ClaveProdServ=\"01010101\""),
            "classified line must replace the catch-all key:\n{xml}"
        );
        // The non-SAT classification must not leak into the SAT key.
        assert!(
            !xml.contains("ClaveProdServ=\"0901\""),
            "non-SAT (HSN) scheme must not be used for ClaveProdServ:\n{xml}"
        );
    }

    #[test]
    fn clave_prod_serv_scheme_match_is_case_insensitive() {
        use invoicekit_ir::{DateOnly, DecimalValue, DocumentLine, ItemClassification};

        let mut doc = sample_invoice();
        doc.lines = vec![DocumentLine {
            id: "1".to_owned(),
            description: "Consultoria".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("E48".to_owned()),
            unit_price: amt(100_000),
            line_extension_amount: amt(100_000),
            tax_category: Some("S".to_owned()),
            classifications: vec![ItemClassification {
                code: "84111506".to_owned(),
                scheme_id: "claveprodserv".to_owned(), // lowercase listID
                scheme_version: None,
            }],
            extensions: Vec::new(),
        }];
        // Touch the issue date so the helper is exercised on a fresh document.
        doc.issue_date = DateOnly::new("2026-05-27").unwrap();
        let xml = to_cfdi_xml(&doc, &CfdiContext::default()).unwrap();
        assert!(
            xml.contains("ClaveProdServ=\"84111506\""),
            "case-insensitive scheme_id match must still bind ClaveProdServ:\n{xml}"
        );
    }

    #[test]
    fn cfdi_is_deterministic() {
        let doc = sample_invoice();
        let ctx = CfdiContext::default();
        assert_eq!(
            to_cfdi_xml(&doc, &ctx).unwrap(),
            to_cfdi_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn cfdi_rejects_unsupported_document_type() {
        let err = tipo_de_comprobante(DocumentType::DebitNote).unwrap_err();
        assert!(matches!(err, CfdiSerializeError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn credit_note_maps_to_egreso() {
        assert_eq!(tipo_de_comprobante(DocumentType::CreditNote).unwrap(), "E");
        assert_eq!(comprobante_kind(DocumentType::CreditNote), CfdiKind::Egreso);
        assert_eq!(comprobante_kind(DocumentType::Invoice), CfdiKind::Ingreso);
    }

    #[test]
    fn report_happy_path_is_stamped() {
        let xml = to_cfdi_xml(&sample_invoice(), &CfdiContext::default())
            .unwrap()
            .into_bytes();
        let report = provider().report(&sample_request(xml)).unwrap();
        assert!(report.envelope.status.is_stamped());
        assert_eq!(report.envelope.status, TimbradoStatus::Timbrado);
        validate_folio_fiscal(&report.envelope.folio_fiscal).unwrap();
        assert!(report.envelope.reason.is_none());
        assert!(report.envelope.sello_sat.is_some());
        // The stamped artifact must carry the TFD complemento.
        let stamped = String::from_utf8(report.timbrado_xml).unwrap();
        assert!(stamped.contains("<tfd:TimbreFiscalDigital"));
        assert!(stamped.contains(&report.envelope.folio_fiscal));
    }

    #[test]
    fn report_rejection_is_ok_not_err() {
        let xml = to_cfdi_xml(&sample_invoice(), &CfdiContext::default())
            .unwrap()
            .into_bytes();
        let provider = provider().with_rejection();
        let report = provider.report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.status, TimbradoStatus::Rechazado);
        assert!(!report.envelope.status.is_stamped());
        assert!(report.envelope.reason.is_some());
        assert!(report.envelope.folio_fiscal.is_empty());
        assert!(report.envelope.sello_sat.is_none());
    }

    #[test]
    fn report_rejects_bad_rfc() {
        let mut req = sample_request(b"<cfdi:Comprobante/>".to_vec());
        req.issuer_rfc = "BAD".to_owned();
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            CfdiReportError::BadRfc(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new());
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            CfdiReportError::BadXml(_)
        ));
    }

    #[test]
    fn rfc_validator_shapes() {
        // 13-char persona física (4 letters + 6 digits + 3 homoclave).
        assert!(validate_rfc("VECJ880326XXX").is_ok());
        // 12-char persona moral (3 letters + 6 digits + 3 homoclave).
        assert!(validate_rfc("CAZ010101AAA").is_ok());
        assert!(validate_rfc("ABC").is_err()); // too short
        assert!(validate_rfc("CAZ010101AAAAA").is_err()); // 14 chars
        assert!(validate_rfc("CA1010101AAA").is_err()); // digit in name prefix
        assert!(validate_rfc("CAZ0101X1AAA").is_err()); // letter in date segment
        assert!(validate_rfc("CAZ-10101AAA").is_err()); // non-alphanumeric
    }

    #[test]
    fn folio_fiscal_validator_shapes() {
        assert!(validate_folio_fiscal("00000000-0000-4000-8000-000000000001").is_ok());
        assert!(validate_folio_fiscal("ABCDEF12-3456-7890-ABCD-EF1234567890").is_ok());
        assert!(validate_folio_fiscal("not-a-uuid").is_err());
        assert!(validate_folio_fiscal("00000000-0000-4000-8000-00000000001").is_err()); // short tail
        assert!(validate_folio_fiscal("0000000Z-0000-4000-8000-000000000001").is_err()); // non-hex
    }

    #[test]
    fn fmt_rate_is_sat_fraction() {
        assert_eq!(fmt_rate(Decimal::new(1600, 2)), "0.160000"); // 16.00% -> 0.160000
        assert_eq!(fmt_rate(Decimal::ZERO), "0.000000");
        assert_eq!(fmt_rate(Decimal::new(800, 2)), "0.080000"); // 8% border IVA
    }

    #[test]
    fn report_request_for_derives_kind_and_tenant() {
        let doc = sample_invoice();
        let req = report_request_for(
            &doc,
            "CAZ010101AAA",
            PacEnvironment::Sandbox,
            sample_csd(),
            b"<cfdi:Comprobante/>".to_vec(),
        )
        .unwrap();
        assert_eq!(req.kind, CfdiKind::Ingreso);
        assert_eq!(req.tenant_id, "tenant_mx");
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_cfdi_xml(&sample_invoice(), &CfdiContext::default())
            .unwrap()
            .into_bytes();
        let env = provider().report(&sample_request(xml)).unwrap().envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: CfdiReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn total_traslados_overflow_is_reported_not_panicked() {
        // Regression: untrusted tax-summary amounts must be summed with checked
        // addition. Two near-maximum Decimals overflow the range; the unchecked
        // `Sum` impl would panic, so the serializer must surface a clean error.
        let mut doc = sample_invoice();
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: DecimalValue::new(Decimal::MAX),
                tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "S2".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: DecimalValue::new(Decimal::MAX),
                tax_rate: Some(DecimalValue::new(Decimal::new(1600, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let err = to_cfdi_xml(&doc, &CfdiContext::default())
            .expect_err("two Decimal::MAX tax amounts must overflow the sum");
        assert!(matches!(err, CfdiSerializeError::AmountOverflow));
        // The single-summary happy path still sums cleanly.
        assert_eq!(
            total_traslados(&sample_invoice()).expect("single summary sums without overflow"),
            Decimal::new(16000, 2)
        );
    }

    #[test]
    fn attr_escapes_tab_newline_carriage_return() {
        // Regression: tab/newline/CR in free-text names and descriptions must be
        // emitted as numeric character references, or XML attribute-value
        // normalization on the recipient silently collapses them to spaces.
        let mut doc = sample_invoice();
        doc.supplier.name = "Razon\tSocial".to_owned();
        doc.customer.name = "Cliente\nDos".to_owned();
        doc.lines[0].description = "Linea\runo".to_owned();
        let xml = to_cfdi_xml(&doc, &CfdiContext::default()).expect("serializes");
        assert!(
            xml.contains("Nombre=\"Razon&#x9;Social\""),
            "tab not escaped:\n{xml}"
        );
        assert!(
            xml.contains("Nombre=\"Cliente&#xA;Dos\""),
            "newline not escaped:\n{xml}"
        );
        assert!(
            xml.contains("Descripcion=\"Linea&#xD;uno\""),
            "carriage return not escaped:\n{xml}"
        );
        // The raw control characters must NOT survive in the output.
        assert!(!xml.contains('\t'), "raw tab leaked into XML");
        // The emitted XML has its own structural newlines; assert the literal
        // control characters did not leak into the attribute values instead.
        assert!(!xml.contains("Razon\tSocial"));
        assert!(!xml.contains("Cliente\nDos"));
        assert!(!xml.contains("Linea\runo"));
    }

    #[test]
    fn party_rfc_strips_mx_prefix() {
        let mut party = mexican_party("X", "MXCAZ010101AAA", "CDMX");
        assert_eq!(party_rfc(&party).unwrap(), "CAZ010101AAA");
        party.tax_ids[0].value = "CAZ010101AAA".to_owned();
        assert_eq!(party_rfc(&party).unwrap(), "CAZ010101AAA");
    }
}
