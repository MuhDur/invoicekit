// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Chile **SII DTE** (Documento Tributario Electrónico) reporting adapter.
//!
//! Chile's Servicio de Impuestos Internos (SII) operates the
//! gold-standard LATAM clearance regime, in production since
//! 2003. Every Chilean B2B issuer signs a typed XML DTE,
//! consumes a **folio** from a CAF (Código de Autorización
//! de Folios) bundle the SII issued in advance, and submits
//! to the SII; the SII returns a TrackId for reconciliation
//! and within minutes a typed Aceptado / Rechazado state.
//!
//! Key DTE kinds with SII tipo codes:
//! - 33 Factura Electrónica
//! - 34 Factura No Afecta o Exenta
//! - 39 Boleta Electrónica
//! - 41 Boleta No Afecta o Exenta
//! - 46 Factura de Compra
//! - 52 Guía de Despacho
//! - 56 Nota de Débito
//! - 61 Nota de Crédito
//!
//! This crate ships the typed surface and a deterministic
//! [`MockSiiProvider`]. The live SII SOAP integration lands
//! in a follow-up `report-cl-dte-http` crate behind a feature
//! flag.

#![allow(clippy::doc_markdown)]

use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// SII DTE serialization (IR -> national DTE / Documento XML)
// ---------------------------------------------------------------------------

/// DTE context: the document-level fields that live in the SII `DTE`/`Documento`
/// envelope but are not part of the jurisdiction-agnostic IR.
///
/// Per the SII "Formato de Documentos Tributarios Electrónicos" the `Documento`
/// element carries an `ID` attribute and the `IdDoc` block carries a `Folio`
/// consumed from the issuer's CAF (Código de Autorización de Folios) bundle. The
/// emisor's `GiroEmis` (economic-activity / line-of-business descriptor) is also
/// mandatory in `Encabezado/Emisor` and has no IR home, so it travels here.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DteContext {
    /// `Folio` — the folio number consumed from the issuer's CAF bundle.
    pub folio: u64,
    /// `GiroEmis` — the issuer's economic-activity / line-of-business text
    /// (mandatory on `Encabezado/Emisor`).
    pub giro_emisor: String,
}

impl Default for DteContext {
    fn default() -> Self {
        Self {
            folio: 1,
            giro_emisor: "Servicios".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to SII DTE XML.
#[derive(Debug, Error)]
pub enum DteXmlError {
    /// The IR `document_type` has no SII `TipoDTE` mapping.
    #[error("document type {0:?} is not representable as a SII TipoDTE")]
    UnsupportedDocumentType(DocumentType),
    /// The emisor (issuer) carries no usable RUT identifier.
    #[error("supplier has no tax id usable as RUTEmisor")]
    MissingEmisorRut,
    /// The receptor (recipient) carries no usable RUT identifier.
    #[error("customer has no tax id usable as RUTRecep")]
    MissingReceptorRut,
    /// The DTE context was malformed (e.g. zero `Folio`).
    #[error("invalid SII DTE context: {0}")]
    BadContext(String),
    /// A monetary total (`MntNeto` / `IVA`) overflowed the representable
    /// `Decimal` range while summing the tax summary.
    #[error("DTE totals are not representable: {0}")]
    TotalsUnrepresentable(String),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic SII **DTE**
/// (`Documento Tributario Electrónico`) XML.
///
/// This emits Chile's real national format — the `DTE`/`Documento` tree defined
/// by the Servicio de Impuestos Internos (SII) "Formato de Documentos
/// Tributarios Electrónicos" — with its actual Spanish element names
/// (`Encabezado`, `IdDoc`, `TipoDTE`, `Folio`, `FchEmis`, `Emisor`, `RUTEmisor`,
/// `RznSoc`, `GiroEmis`, `Receptor`, `RUTRecep`, `RznSocRecep`, `Totales`,
/// `MntNeto`, `TasaIVA`, `IVA`, `MntTotal`, and per-line `Detalle` with
/// `NroLinDet`, `CdgItem`, `NmbItem`, `QtyItem`, `PrcItem`, `MontoItem`). It is
/// **not** UBL relabeled: UBL/CII serializers do not emit this tree.
///
/// Each IR line classification is emitted as a SII `Detalle/CdgItem` block with
/// `TpoCodigo` (the scheme/list identifier) and `VlrCodigo` (the code value),
/// both copied verbatim from the producer-supplied IR — InvoiceKit does not
/// derive, translate, or map any national code. Per the SII "Formato DTE", a
/// `Detalle` may repeat `CdgItem` (one per classification) and it is positioned
/// after `NroLinDet` and before `NmbItem`. A line with no classifications emits
/// no `CdgItem`, exactly as before.
///
/// Output is byte-stable by construction: a fixed element order with no maps and
/// no timestamps. Monetary totals (`MntNeto`, `IVA`, `MntTotal`, `MontoItem`)
/// render as integer Chilean pesos (CLP has no minor unit, per the SII format,
/// which types these fields as integers); `TasaIVA` renders the IVA percentage
/// and `QtyItem` / `PrcItem` keep their natural scale.
///
/// The document is expected to have passed IR validation already (it has, if
/// built via [`CommercialDocument::new`]).
///
/// # Errors
///
/// Returns [`DteXmlError::UnsupportedDocumentType`] for document types with no
/// `TipoDTE` mapping, [`DteXmlError::MissingEmisorRut`] /
/// [`DteXmlError::MissingReceptorRut`] when a party has no RUT,
/// [`DteXmlError::BadContext`] when the context is malformed, and
/// [`DteXmlError::TotalsUnrepresentable`] when summing the tax summary
/// overflows the representable `Decimal` range.
pub fn to_dte_xml(
    document: &CommercialDocument,
    context: &DteContext,
) -> Result<String, DteXmlError> {
    if context.folio == 0 {
        return Err(DteXmlError::BadContext("Folio must be > 0".to_owned()));
    }
    let tipo_dte = tipo_dte(document.document_type)?;
    let emisor_rut = party_rut(&document.supplier).ok_or(DteXmlError::MissingEmisorRut)?;
    let receptor_rut = party_rut(&document.customer).ok_or(DteXmlError::MissingReceptorRut)?;

    let (mnt_neto, tasa_iva, iva) = dte_tax_totals(document)?;
    let mnt_total = document.monetary_total.payable_amount.inner();

    let mut out = String::with_capacity(2048);
    // This serializer returns a Rust String (UTF-8), so the XML declaration must
    // be UTF-8 — accented Spanish (e.g. "consultoría") is pervasive in Chilean
    // DTEs and an ISO-8859-1 declaration would mis-decode the UTF-8 bytes as
    // mojibake. Production SII submission re-encodes to ISO-8859-1 at the wire;
    // that transcoding belongs in the follow-up report-cl-dte-http crate.
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<DTE xmlns=\"http://www.sii.cl/SiiDte\" version=\"1.0\">\n",
    );
    // Documento ID convention: "T<tipo>F<folio>" (e.g. T33F4242).
    let doc_id = format!("T{tipo_dte}F{}", context.folio);
    indent(&mut out, 1);
    out.push_str("<Documento ID=\"");
    push_escaped(&mut out, &doc_id);
    out.push_str("\">\n");

    // --- Encabezado ---
    open(&mut out, 2, "Encabezado");

    open(&mut out, 3, "IdDoc");
    el(&mut out, 4, "TipoDTE", &tipo_dte.to_string());
    el(&mut out, 4, "Folio", &context.folio.to_string());
    el(&mut out, 4, "FchEmis", document.issue_date.as_str());
    close(&mut out, 3, "IdDoc");

    open(&mut out, 3, "Emisor");
    el(&mut out, 4, "RUTEmisor", &emisor_rut);
    el(&mut out, 4, "RznSoc", &document.supplier.name);
    el(&mut out, 4, "GiroEmis", &context.giro_emisor);
    close(&mut out, 3, "Emisor");

    open(&mut out, 3, "Receptor");
    el(&mut out, 4, "RUTRecep", &receptor_rut);
    el(&mut out, 4, "RznSocRecep", &document.customer.name);
    close(&mut out, 3, "Receptor");

    open(&mut out, 3, "Totales");
    el(&mut out, 4, "MntNeto", &fmt_peso(mnt_neto));
    el(&mut out, 4, "TasaIVA", &fmt_rate(tasa_iva));
    el(&mut out, 4, "IVA", &fmt_peso(iva));
    el(&mut out, 4, "MntTotal", &fmt_peso(mnt_total));
    close(&mut out, 3, "Totales");

    close(&mut out, 2, "Encabezado");

    // --- Detalle (one block per line, in document order) ---
    for (index, line) in document.lines.iter().enumerate() {
        open(&mut out, 2, "Detalle");
        el(&mut out, 3, "NroLinDet", &(index + 1).to_string());
        // CdgItem — one per IR classification, positioned after NroLinDet and
        // before NmbItem per the SII "Formato DTE" Detalle child order. Both
        // children carry the producer-supplied IR strings verbatim: TpoCodigo is
        // the classification scheme/list identifier, VlrCodigo the code value.
        // No national code is derived, mapped, or invented; scheme_version has
        // no SII Detalle home, so it is not emitted here.
        for classification in &line.classifications {
            open(&mut out, 3, "CdgItem");
            el(&mut out, 4, "TpoCodigo", &classification.scheme_id);
            el(&mut out, 4, "VlrCodigo", &classification.code);
            close(&mut out, 3, "CdgItem");
        }
        el(&mut out, 3, "NmbItem", &line.description);
        el(&mut out, 3, "QtyItem", &fmt_qty(line.quantity.inner()));
        el(&mut out, 3, "PrcItem", &fmt_qty(line.unit_price.inner()));
        el(
            &mut out,
            3,
            "MontoItem",
            &fmt_peso(line.line_extension_amount.inner()),
        );
        close(&mut out, 2, "Detalle");
    }

    close(&mut out, 1, "Documento");
    out.push_str("</DTE>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to a SII `TipoDTE` code. Per the SII DTE format:
/// 33 = Factura Electrónica, 61 = Nota de Crédito, 56 = Nota de Débito.
fn tipo_dte(document_type: DocumentType) -> Result<u16, DteXmlError> {
    match document_type {
        DocumentType::Invoice => Ok(DteKind::FacturaElectronica.code()),
        DocumentType::CreditNote => Ok(DteKind::NotaCredito.code()),
        DocumentType::DebitNote => Ok(DteKind::NotaDebito.code()),
        other @ (DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(DteXmlError::UnsupportedDocumentType(other))
        }
    }
}

/// Extract a RUT string from a party. Chile keys parties by RUT, carried in the
/// IR as a tax id (scheme `CL:RUT`, or the first tax id as a fallback).
fn party_rut(party: &Party) -> Option<String> {
    party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("CL:RUT") || t.scheme.eq_ignore_ascii_case("rut"))
        .or_else(|| party.tax_ids.first())
        .map(|t| t.value.clone())
}

/// Derive the `Totales` triple `(MntNeto, TasaIVA, IVA)` from the IR tax summary.
///
/// `MntNeto` is the sum of taxable bases, `IVA` the sum of tax amounts, and
/// `TasaIVA` the (single) IVA percentage in effect — Chile's standard IVA is a
/// flat 19 %, so a DTE carries one `TasaIVA`. When the document is fully exempt
/// (tipo 34, no tax), `TasaIVA` is zero.
///
/// The summary amounts are untrusted here, so the running `MntNeto` / `IVA`
/// totals accumulate with [`Decimal::checked_add`] rather than the panicking
/// `+=`; an out-of-range total yields [`DteXmlError::TotalsUnrepresentable`].
fn dte_tax_totals(
    document: &CommercialDocument,
) -> Result<(Decimal, Decimal, Decimal), DteXmlError> {
    let mut mnt_neto = Decimal::ZERO;
    let mut iva = Decimal::ZERO;
    let mut tasa = Decimal::ZERO;
    for summary in &document.tax_summary {
        mnt_neto = mnt_neto
            .checked_add(summary.taxable_amount.inner())
            .ok_or_else(|| DteXmlError::TotalsUnrepresentable("MntNeto".to_owned()))?;
        iva = iva
            .checked_add(summary.tax_amount.inner())
            .ok_or_else(|| DteXmlError::TotalsUnrepresentable("IVA".to_owned()))?;
        if let Some(rate) = summary.tax_rate.as_ref() {
            if rate.inner() > tasa {
                tasa = rate.inner();
            }
        }
    }
    Ok((mnt_neto, tasa, iva))
}

/// Format an integer Chilean-peso amount: CLP has no minor unit, and the SII
/// format types `MntNeto` / `IVA` / `MntTotal` / `MontoItem` as integers.
fn fmt_peso(value: Decimal) -> String {
    value.round().trunc().to_string()
}

/// Format the IVA percentage rate (`TasaIVA`), trimming a trailing `.00` so the
/// common flat 19 % renders as `19` rather than `19.00`.
fn fmt_rate(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Format a quantity / unit price, preserving its natural (normalized) scale.
fn fmt_qty(value: Decimal) -> String {
    value.normalize().to_string()
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
// SII report adapter (typed surface + deterministic offline mock provider)
// ---------------------------------------------------------------------------

/// Environment selector for the SII transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiiEnvironment {
    /// `maullin.sii.cl` / SII certification (sandbox).
    Certification,
    /// `palena.sii.cl` / production.
    Production,
}

/// DTE class (subset of the most common SII tipo codes).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DteKind {
    /// 33 Factura Electrónica (B2B affected by IVA).
    FacturaElectronica,
    /// 34 Factura No Afecta o Exenta.
    FacturaExenta,
    /// 39 Boleta Electrónica (B2C).
    BoletaElectronica,
    /// 41 Boleta No Afecta o Exenta.
    BoletaExenta,
    /// 46 Factura de Compra (self-billed purchase).
    FacturaCompra,
    /// 52 Guía de Despacho (delivery / movement note).
    GuiaDespacho,
    /// 56 Nota de Débito.
    NotaDebito,
    /// 61 Nota de Crédito.
    NotaCredito,
}

impl DteKind {
    /// SII tipo code (`tipo DTE`) for this class.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::FacturaElectronica => 33,
            Self::FacturaExenta => 34,
            Self::BoletaElectronica => 39,
            Self::BoletaExenta => 41,
            Self::FacturaCompra => 46,
            Self::GuiaDespacho => 52,
            Self::NotaDebito => 56,
            Self::NotaCredito => 61,
        }
    }
}

/// What the operator passes in to [`SiiProvider::submit_dte`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SiiSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: SiiEnvironment,
    /// DTE class.
    pub kind: DteKind,
    /// Issuer RUT (`NNNNNNNN-X`, where `X` is digit or `K`).
    pub issuer_rut: String,
    /// Folio consumed from the issuer's CAF bundle.
    pub folio: u64,
    /// Canonical signed DTE XML payload.
    pub dte_xml: Vec<u8>,
}

/// SII per-DTE verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiiStatus {
    /// Recibido — SII received the upload; awaiting
    /// validation.
    Recibido,
    /// Aceptado — SII validation passed; the DTE is final.
    Aceptado,
    /// Aceptado con Reparos — validation passed with
    /// warnings.
    AceptadoConReparos,
    /// Rechazado — SII validation rejected the DTE.
    Rechazado,
}

/// What [`SiiProvider::submit_dte`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SiiSubmitEnvelope {
    /// SII-assigned TrackId.
    pub track_id: String,
    /// Latest observed status.
    pub status: SiiStatus,
    /// RFC-3339 UTC timestamp SII recorded.
    pub submitted_at: String,
    /// Glosa from SII when `status == Rechazado` or
    /// `AceptadoConReparos`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glosa: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum SiiError {
    /// DTE XML failed shape validation before the wire.
    #[error("dte xml rejected: {0}")]
    BadXml(String),
    /// RUT didn't match SII's `NNNNNNNN-X` shape.
    #[error("invalid RUT: {0}")]
    BadRut(String),
    /// Folio out of CAF range.
    #[error("invalid folio: {0}")]
    BadFolio(String),
    /// HTTP / TLS / DNS failure talking to SII.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The SII integration surface.
pub trait SiiProvider: Send + Sync {
    /// Submit one DTE to SII. The provider:
    ///
    /// 1. validates `issuer_rut` shape,
    /// 2. validates `folio` is non-zero,
    /// 3. POSTs the signed DTE XML,
    /// 4. returns the SII-assigned TrackId envelope.
    ///
    /// # Errors
    ///
    /// Returns [`SiiError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// SII-returned `Rechazado` verdict is NOT an `Err` —
    /// it's surfaced via `SiiStatus::Rechazado` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_dte(&self, request: &SiiSubmitRequest) -> Result<SiiSubmitEnvelope, SiiError>;

    /// Poll SII for the latest status of a previously
    /// submitted DTE.
    ///
    /// # Errors
    ///
    /// Returns [`SiiError::Transport`] when the TrackId is
    /// unknown.
    fn query_track_id(
        &self,
        environment: SiiEnvironment,
        track_id: &str,
    ) -> Result<SiiSubmitEnvelope, SiiError>;
}

/// Deterministic mock provider.
pub struct MockSiiProvider {
    fixed_submitted_at: String,
    next_serial: std::sync::Mutex<u64>,
    /// When set, [`SiiProvider::submit_dte`] still runs the real
    /// pre-wire validators but, instead of the happy-path
    /// `Recibido`, synthesizes this authority verdict in the
    /// receipt envelope. Lets the offline suite drive the
    /// `Rechazado` / `AceptadoConReparos` branches — which the
    /// real SII returns asynchronously — without an `Err`.
    forced_status: Option<SiiStatus>,
}

impl MockSiiProvider {
    /// Build a mock with deterministic timestamps + serial
    /// TrackIds.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_submitted_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_submitted_at(submitted_at: impl Into<String>) -> Self {
        Self {
            fixed_submitted_at: submitted_at.into(),
            next_serial: std::sync::Mutex::new(1),
            forced_status: None,
        }
    }

    /// Force the SII verdict the next `submit_dte` returns. The
    /// pre-wire validators still run; only the synthesized
    /// receipt `status` changes. This is how the audit trail
    /// captures an authority-side `Rechazado` (a receipt status,
    /// **not** an `Err`).
    #[must_use]
    pub fn with_forced_status(mut self, status: SiiStatus) -> Self {
        self.forced_status = Some(status);
        self
    }
}

impl Default for MockSiiProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SiiProvider for MockSiiProvider {
    fn submit_dte(&self, request: &SiiSubmitRequest) -> Result<SiiSubmitEnvelope, SiiError> {
        validate_rut(&request.issuer_rut)?;
        if request.folio == 0 {
            return Err(SiiError::BadFolio("folio must be > 0".to_owned()));
        }
        if request.dte_xml.is_empty() {
            return Err(SiiError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        let status = self.forced_status.unwrap_or(SiiStatus::Recibido);
        // The SII always returns a glosa with a `Rechazado` or
        // `AceptadoConReparos` verdict; the happy path carries none.
        let glosa = match status {
            SiiStatus::Rechazado => {
                Some("RECHAZADO: documento con errores de validación".to_owned())
            }
            SiiStatus::AceptadoConReparos => {
                Some("ACEPTADO CON REPAROS: revise observaciones".to_owned())
            }
            SiiStatus::Recibido | SiiStatus::Aceptado => None,
        };
        Ok(SiiSubmitEnvelope {
            track_id: format!("SII-{serial:012}"),
            status,
            submitted_at: self.fixed_submitted_at.clone(),
            glosa,
        })
    }

    fn query_track_id(
        &self,
        _environment: SiiEnvironment,
        track_id: &str,
    ) -> Result<SiiSubmitEnvelope, SiiError> {
        if track_id.is_empty() {
            return Err(SiiError::Transport("empty TrackId".to_owned()));
        }
        Ok(SiiSubmitEnvelope {
            track_id: track_id.to_owned(),
            status: SiiStatus::Aceptado,
            submitted_at: self.fixed_submitted_at.clone(),
            glosa: None,
        })
    }
}

/// Validate a Chilean RUT — `NNNNNNNN-X` where `X` is a
/// digit or `K`. The Chilean modulo-11 check digit is a
/// separate concern; this helper only catches obviously-wrong
/// shapes before the wire.
///
/// # Errors
///
/// Returns [`SiiError::BadRut`] on shape failure.
pub fn validate_rut(rut: &str) -> Result<(), SiiError> {
    if let Some((head, tail)) = rut.rsplit_once('-') {
        let head_ok = (1..=8).contains(&head.len()) && head.bytes().all(|b| b.is_ascii_digit());
        let tail_ok = tail.len() == 1
            && tail
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == 'K' || c == 'k');
        if head_ok && tail_ok {
            return Ok(());
        }
    }
    Err(SiiError::BadRut(format!(
        "RUT must be `NNNNNNNN-X` (1-8 digits, dash, digit/K), got {rut:?}"
    )))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_cl_dte::crate_name(),
///     "invoicekit-report-cl-dte"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-cl-dte"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SiiSubmitRequest {
        SiiSubmitRequest {
            tenant_id: "tenant-cl-test".to_owned(),
            environment: SiiEnvironment::Certification,
            kind: DteKind::FacturaElectronica,
            issuer_rut: "12345678-9".to_owned(),
            folio: 4242,
            dte_xml: b"<DTE/>".to_vec(),
        }
    }

    #[test]
    fn submit_dte_returns_recibido_with_track_id() {
        let p = MockSiiProvider::default();
        let env = p.submit_dte(&sample_request()).unwrap();
        assert_eq!(env.status, SiiStatus::Recibido);
        assert!(env.track_id.starts_with("SII-"));
    }

    #[test]
    fn submit_dte_serial_increments_per_provider() {
        let p = MockSiiProvider::default();
        let env1 = p.submit_dte(&sample_request()).unwrap();
        let env2 = p.submit_dte(&sample_request()).unwrap();
        assert_ne!(env1.track_id, env2.track_id);
    }

    #[test]
    fn submit_dte_rejects_empty_payload() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.dte_xml.clear();
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadXml(_)));
    }

    #[test]
    fn submit_dte_rejects_zero_folio() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.folio = 0;
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadFolio(_)));
    }

    #[test]
    fn submit_dte_rejects_bad_rut() {
        let p = MockSiiProvider::default();
        let mut req = sample_request();
        req.issuer_rut = "BAD".to_owned();
        let err = p.submit_dte(&req).unwrap_err();
        assert!(matches!(err, SiiError::BadRut(_)));
    }

    #[test]
    fn query_track_id_returns_aceptado() {
        let p = MockSiiProvider::default();
        let env = p
            .query_track_id(SiiEnvironment::Certification, "SII-000000000001")
            .unwrap();
        assert_eq!(env.status, SiiStatus::Aceptado);
    }

    #[test]
    fn query_track_id_rejects_empty() {
        let p = MockSiiProvider::default();
        let err = p
            .query_track_id(SiiEnvironment::Certification, "")
            .unwrap_err();
        assert!(matches!(err, SiiError::Transport(_)));
    }

    #[test]
    fn dte_kind_codes_match_sii_taxonomy() {
        assert_eq!(DteKind::FacturaElectronica.code(), 33);
        assert_eq!(DteKind::FacturaExenta.code(), 34);
        assert_eq!(DteKind::BoletaElectronica.code(), 39);
        assert_eq!(DteKind::BoletaExenta.code(), 41);
        assert_eq!(DteKind::FacturaCompra.code(), 46);
        assert_eq!(DteKind::GuiaDespacho.code(), 52);
        assert_eq!(DteKind::NotaDebito.code(), 56);
        assert_eq!(DteKind::NotaCredito.code(), 61);
    }

    #[test]
    fn validate_rut_accepts_well_formed_strings() {
        assert!(validate_rut("12345678-9").is_ok());
        assert!(validate_rut("12345678-K").is_ok());
        assert!(validate_rut("12345678-k").is_ok());
        assert!(validate_rut("1-9").is_ok());
        assert!(validate_rut("12345678-0").is_ok());
    }

    #[test]
    fn validate_rut_rejects_bad_shapes() {
        assert!(validate_rut("123456789").is_err()); // no dash
        assert!(validate_rut("12345678-XY").is_err()); // 2-char tail
        assert!(validate_rut("ABCDEFGH-9").is_err()); // non-digit head
        assert!(validate_rut("123456789-9").is_err()); // 9-digit head
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = SiiSubmitEnvelope {
            track_id: "SII-000000000007".to_owned(),
            status: SiiStatus::Rechazado,
            submitted_at: "2026-01-01T00:00:00Z".to_owned(),
            glosa: Some("RUT no existe".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: SiiSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn forced_rechazado_is_a_receipt_status_not_an_err() {
        // A well-formed submission whose CONTENT the SII refuses still
        // passes the pre-wire validators and returns an Ok envelope with
        // the Rechazado verdict + a glosa.
        let p = MockSiiProvider::new().with_forced_status(SiiStatus::Rechazado);
        let env = p.submit_dte(&sample_request()).unwrap();
        assert_eq!(env.status, SiiStatus::Rechazado);
        assert!(env.track_id.starts_with("SII-"));
        assert!(
            env.glosa.as_deref().is_some_and(|g| g.contains("RECHAZADO")),
            "a Rechazado verdict must carry a glosa, got {:?}",
            env.glosa
        );
    }

    #[test]
    fn forced_status_still_runs_pre_wire_validators() {
        // Forcing a verdict does NOT bypass shape validation; a bad RUT is
        // still an Err, never a receipt.
        let p = MockSiiProvider::new().with_forced_status(SiiStatus::Rechazado);
        let mut req = sample_request();
        req.issuer_rut = "BAD".to_owned();
        assert!(matches!(p.submit_dte(&req).unwrap_err(), SiiError::BadRut(_)));
    }

    #[test]
    fn forced_aceptado_con_reparos_carries_glosa() {
        let p = MockSiiProvider::new().with_forced_status(SiiStatus::AceptadoConReparos);
        let env = p.submit_dte(&sample_request()).unwrap();
        assert_eq!(env.status, SiiStatus::AceptadoConReparos);
        assert!(env.glosa.is_some());
    }

    #[test]
    fn validate_rut_rejects_k_in_the_head() {
        // The verifier digit slot may be `K`, but the body must be digits.
        assert!(validate_rut("1234K678-9").is_err());
        assert!(validate_rut("-9").is_err()); // empty head
        assert!(validate_rut("12345678-").is_err()); // empty tail
    }

    // -----------------------------------------------------------------------
    // SII DTE serializer (IR -> national DTE/Documento XML) tests
    // -----------------------------------------------------------------------

    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType,
        ItemClassification, Iso4217Code, MonetaryTotal, Party, PartyTaxId, PostalAddress,
        ReferenceKindClass, SchemaVersion, TaxCategorySummary,
    };
    use rust_decimal::Decimal;

    fn peso(units: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(units, 0))
    }

    fn chilean_party(name: &str, rut: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "CL:RUT".to_owned(),
                value: rut.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["Av. Providencia 1234".to_owned()],
                city: city.to_owned(),
                subdivision: Some("RM".to_owned()),
                postal_code: "7500000".to_owned(),
                country: CountryCode::new("CL").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_dte_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-cl-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("DTE-33-0001").unwrap(),
            currency: Iso4217Code::new("CLP").unwrap(),
            supplier: chilean_party("Proveedor SpA", "76192083-9", "Santiago"),
            customer: chilean_party("Cliente & Asociados Ltda", "77654321-0", "Valparaíso"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Servicios de consultoría".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: peso(5000),
                line_extension_amount: peso(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            }],
            // IVA (Chilean VAT) is a flat 19%.
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: peso(10000),
                tax_amount: peso(1900),
                tax_rate: Some(DecimalValue::new(Decimal::from(19))),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: peso(10000),
                tax_exclusive_amount: peso(10000),
                tax_inclusive_amount: peso(11900),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: peso(11900),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_cl".to_owned(),
                trace_id: "trace_cl".to_owned(),
                source_system: Some("inline-test".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_ctx() -> DteContext {
        DteContext {
            folio: 4242,
            giro_emisor: "Servicios de ingeniería".to_owned(),
        }
    }

    #[test]
    fn dte_emits_real_sii_element_names() {
        let xml = to_dte_xml(&sample_dte_invoice(), &sample_ctx()).unwrap();
        // The actual SII "Formato DTE" element names — NOT UBL relabeled.
        for needle in [
            "<DTE xmlns=\"http://www.sii.cl/SiiDte\" version=\"1.0\">",
            "<Documento ID=\"T33F4242\">",
            "<Encabezado>",
            "<IdDoc>",
            "<TipoDTE>33</TipoDTE>",
            "<Folio>4242</Folio>",
            "<FchEmis>2026-05-26</FchEmis>",
            "<Emisor>",
            "<RUTEmisor>76192083-9</RUTEmisor>",
            "<RznSoc>Proveedor SpA</RznSoc>",
            "<GiroEmis>Servicios de ingeniería</GiroEmis>",
            "<Receptor>",
            "<RUTRecep>77654321-0</RUTRecep>",
            "<RznSocRecep>Cliente &amp; Asociados Ltda</RznSocRecep>",
            "<Totales>",
            "<MntNeto>10000</MntNeto>",
            "<TasaIVA>19</TasaIVA>",
            "<IVA>1900</IVA>",
            "<MntTotal>11900</MntTotal>",
            "<Detalle>",
            "<NroLinDet>1</NroLinDet>",
            "<NmbItem>Servicios de consultoría</NmbItem>",
            "<QtyItem>2</QtyItem>",
            "<PrcItem>5000</PrcItem>",
            "<MontoItem>10000</MontoItem>",
        ] {
            assert!(xml.contains(needle), "DTE missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn dte_is_deterministic() {
        let doc = sample_dte_invoice();
        let ctx = sample_ctx();
        assert_eq!(
            to_dte_xml(&doc, &ctx).unwrap(),
            to_dte_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn dte_credit_note_maps_to_tipo_61() {
        let mut doc = sample_dte_invoice();
        doc.document_type = DocumentType::CreditNote;
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(xml.contains("<TipoDTE>61</TipoDTE>"));
        assert!(xml.contains("<Documento ID=\"T61F4242\">"));
        assert!(!xml.contains("<TipoDTE>33</TipoDTE>"));
    }

    #[test]
    fn dte_rejects_unsupported_document_type() {
        let err = tipo_dte(DocumentType::ProForma).unwrap_err();
        assert!(matches!(err, DteXmlError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn dte_rejects_zero_folio() {
        let ctx = DteContext {
            folio: 0,
            giro_emisor: "X".to_owned(),
        };
        let err = to_dte_xml(&sample_dte_invoice(), &ctx).unwrap_err();
        assert!(matches!(err, DteXmlError::BadContext(_)));
    }

    #[test]
    fn dte_multiline_emits_one_detalle_per_line() {
        let mut doc = sample_dte_invoice();
        doc.lines.push(DocumentLine {
            id: "2".to_owned(),
            description: "Licencia anual".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: peso(25000),
            line_extension_amount: peso(25000),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        });
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert_eq!(xml.matches("<Detalle>").count(), 2);
        assert!(xml.contains("<NroLinDet>1</NroLinDet>"));
        assert!(xml.contains("<NroLinDet>2</NroLinDet>"));
        assert!(xml.contains("<NmbItem>Licencia anual</NmbItem>"));
    }

    #[test]
    fn dte_exempt_invoice_has_zero_tasa_iva() {
        let mut doc = sample_dte_invoice();
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: peso(10000),
            tax_amount: peso(0),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
            exemption_reason: None,
            exemption_reason_code: None,
        }];
        doc.monetary_total.tax_inclusive_amount = peso(10000);
        doc.monetary_total.payable_amount = peso(10000);
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(xml.contains("<TasaIVA>0</TasaIVA>"));
        assert!(xml.contains("<IVA>0</IVA>"));
        assert!(xml.contains("<MntTotal>10000</MntTotal>"));
    }

    /// Two tax-summary bands each near `Decimal::MAX` overflow the `MntNeto`
    /// accumulator in [`dte_tax_totals`]. Before the `checked_add` fix this
    /// panicked (`rust_decimal`'s `+=` panics on overflow); now it surfaces a
    /// typed [`DteXmlError::TotalsUnrepresentable`].
    #[test]
    fn dte_totals_overflow_returns_error_not_panic() {
        let huge = DecimalValue::new(Decimal::MAX);
        let mut doc = sample_dte_invoice();
        // Two near-MAX taxable bases sum past Decimal::MAX. The amounts are
        // wired directly onto tax_summary (post-construction) so the overflow
        // lands squarely on the MntNeto accumulator inside dte_tax_totals.
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: huge.clone(),
                tax_amount: peso(0),
                tax_rate: Some(DecimalValue::new(Decimal::from(19))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: huge,
                tax_amount: peso(0),
                tax_rate: Some(DecimalValue::new(Decimal::from(19))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let err = to_dte_xml(&doc, &sample_ctx())
            .expect_err("two near-MAX taxable bases must overflow MntNeto");
        assert!(matches!(err, DteXmlError::TotalsUnrepresentable(_)));
    }

    #[test]
    fn dte_requires_party_ruts() {
        let mut doc = sample_dte_invoice();
        doc.supplier.tax_ids.clear();
        assert!(matches!(
            to_dte_xml(&doc, &sample_ctx()).unwrap_err(),
            DteXmlError::MissingEmisorRut
        ));

        let mut doc = sample_dte_invoice();
        doc.customer.tax_ids.clear();
        assert!(matches!(
            to_dte_xml(&doc, &sample_ctx()).unwrap_err(),
            DteXmlError::MissingReceptorRut
        ));
    }

    // -----------------------------------------------------------------------
    // New IR field wiring: line classifications -> Detalle/CdgItem.
    // -----------------------------------------------------------------------

    #[test]
    fn dte_emits_cdgitem_per_line_classification_verbatim() {
        let mut doc = sample_dte_invoice();
        // Two classifications on the single line: each becomes one CdgItem with
        // TpoCodigo = scheme_id and VlrCodigo = code, copied verbatim.
        doc.lines[0].classifications = vec![
            ItemClassification {
                code: "85.12.10".to_owned(),
                scheme_id: "INTERNO".to_owned(),
                scheme_version: None,
            },
            ItemClassification {
                code: "81111500".to_owned(),
                scheme_id: "UNSPSC".to_owned(),
                // scheme_version has no SII Detalle home; it must not appear. A
                // distinctive sentinel avoids colliding with dates/amounts.
                scheme_version: Some("SCHEMEVER-ZZZ".to_owned()),
            },
        ];
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();

        // Exactly two CdgItem blocks, carrying the verbatim TpoCodigo/VlrCodigo.
        assert_eq!(xml.matches("<CdgItem>").count(), 2);
        for needle in [
            "<TpoCodigo>INTERNO</TpoCodigo>",
            "<VlrCodigo>85.12.10</VlrCodigo>",
            "<TpoCodigo>UNSPSC</TpoCodigo>",
            "<VlrCodigo>81111500</VlrCodigo>",
        ] {
            assert!(xml.contains(needle), "DTE missing {needle:?}:\n{xml}");
        }
        // scheme_version is not a SII Detalle element and must not leak.
        assert!(
            !xml.contains("SCHEMEVER-ZZZ"),
            "scheme_version must not be emitted:\n{xml}"
        );
    }

    #[test]
    fn dte_cdgitem_sits_between_nrolindet_and_nmbitem() {
        // Verify the SII Detalle child order: NroLinDet, CdgItem*, NmbItem.
        let mut doc = sample_dte_invoice();
        doc.lines[0].classifications = vec![ItemClassification {
            code: "C-1".to_owned(),
            scheme_id: "INTERNO".to_owned(),
            scheme_version: None,
        }];
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        let nro = xml.find("<NroLinDet>").expect("NroLinDet present");
        let cdg = xml.find("<CdgItem>").expect("CdgItem present");
        let nmb = xml.find("<NmbItem>").expect("NmbItem present");
        assert!(
            nro < cdg && cdg < nmb,
            "CdgItem must sit after NroLinDet and before NmbItem:\n{xml}"
        );
    }

    #[test]
    fn dte_cdgitem_escapes_special_characters() {
        // Verbatim emission still XML-escapes content (no national mapping, but
        // the bytes must be well-formed).
        let mut doc = sample_dte_invoice();
        doc.lines[0].classifications = vec![ItemClassification {
            code: "A&B<C".to_owned(),
            scheme_id: "S&Z".to_owned(),
            scheme_version: None,
        }];
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(xml.contains("<TpoCodigo>S&amp;Z</TpoCodigo>"));
        assert!(xml.contains("<VlrCodigo>A&amp;B&lt;C</VlrCodigo>"));
    }

    #[test]
    fn dte_without_classifications_emits_no_cdgitem() {
        // Behavior-preserving: every existing fixture has empty classifications,
        // so the serializer must emit no CdgItem and the rest of the Detalle is
        // byte-for-byte what it was before this field was wired.
        let doc = sample_dte_invoice();
        assert!(doc.lines.iter().all(|l| l.classifications.is_empty()));
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(
            !xml.contains("<CdgItem>"),
            "a line with no classifications must emit no CdgItem:\n{xml}"
        );
        // The Detalle still threads NroLinDet straight into NmbItem.
        assert!(xml.contains("<NroLinDet>1</NroLinDet>\n      <NmbItem>"));
    }

    // -----------------------------------------------------------------------
    // SKIPPED national elements (documented refusals).
    //
    // These two scenarios are deliberately NOT serialized into the SII DTE
    // tree, and the tests pin that refusal so a future change can't silently
    // emit a wrong/invented national element.
    // -----------------------------------------------------------------------

    /// `references` (PrecedingInvoice) does NOT become a SII `Referencia`.
    ///
    /// The SII `Referencia` block makes `TpoDocRef` (the SII document-type code
    /// of the *referenced* document) mandatory, and the IR `DocumentReference`
    /// carries only `kind`/`id`/`issue_date` — there is no field telling us the
    /// referenced document's SII tipo. A credit note may reference a 33, 34, 39,
    /// 46, 56, ... so no single well-known constant is defensible; inventing one
    /// (or emitting a `Referencia` missing its mandatory `TpoDocRef`) would be a
    /// fabricated/invalid national element. So we skip the whole block.
    #[test]
    fn dte_preceding_invoice_reference_is_not_emitted_as_referencia() {
        let mut doc = sample_dte_invoice();
        doc.document_type = DocumentType::CreditNote;
        doc.references = vec![DocumentReference {
            kind: "original-invoice".to_owned(),
            id: "4242".to_owned(),
            issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
        }];
        // Sanity: the IR really does classify this as the preceding invoice.
        assert_eq!(
            doc.references[0].kind_class(),
            ReferenceKindClass::PrecedingInvoice
        );
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(
            !xml.contains("<Referencia>")
                && !xml.contains("<FolioRef>")
                && !xml.contains("<TpoDocRef>")
                && !xml.contains("<FchRef>"),
            "no SII Referencia is emitted (TpoDocRef is undefensible):\n{xml}"
        );
    }

    /// Tax-summary exemption reason / code are NOT serialized into the DTE.
    ///
    /// Chile signals exemption structurally (line-level `IndExe`, document-level
    /// `MntExe`), not via a free-text reason or a CEF-`VATEX`/IT-`Natura` code.
    /// There is no SII DTE element that carries BT-120/BT-121 verbatim, so these
    /// fields are skipped rather than mapped onto an invented element name.
    #[test]
    fn dte_exemption_reason_fields_are_not_emitted() {
        let mut doc = sample_dte_invoice();
        doc.tax_summary[0].exemption_reason = Some("Exportación de servicios".to_owned());
        doc.tax_summary[0].exemption_reason_code = Some("VATEX-EU-G".to_owned());
        let xml = to_dte_xml(&doc, &sample_ctx()).unwrap();
        assert!(
            !xml.contains("Exportación de servicios") && !xml.contains("VATEX-EU-G"),
            "no SII element carries the verbatim exemption reason/code:\n{xml}"
        );
    }
}
