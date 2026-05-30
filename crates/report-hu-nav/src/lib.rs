// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Hungary **NAV Online Számla** reporting adapter.
//!
//! Hungary's Nemzeti Adó- és Vámhivatal (NAV, the National
//! Tax and Customs Administration) runs the Online Számla
//! v3.0 reporting endpoints at `api.onlineszamla.nav.gov.hu`.
//! Every Hungarian B2B issuer submits invoices via a typed
//! XML wrapper (`manageInvoiceRequest`); NAV runs a
//! token-exchange + transaction-id flow and returns
//! per-invoice processing status the engine reconciles
//! against.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockNavProvider`]. The live REST integration lands in a
//! follow-up `report-hu-nav-http` crate behind a feature
//! flag.

#![allow(clippy::doc_markdown)]

use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// NAV Online Számla `InvoiceData` serialization (IR -> national RTIR XML)
// ---------------------------------------------------------------------------
//
// NAV's wire payload is the base64-encoded `InvoiceData` XML carried inside a
// `manageInvoiceRequest`/`invoiceOperation`. `InvoiceData` is the REAL national
// invoice document defined by the NAV Online Számla Interface Specification
// v3.0 (XSD `invoiceData.xsd`, namespace
// `http://schemas.nav.gov.hu/OSA/3.0/data`, importing the common types from
// `http://schemas.nav.gov.hu/OSA/3.0/base`). This is NOT UBL relabelled: the
// element names (`invoiceNumber`, `invoiceIssueDate`, `supplierInfo`,
// `customerInfo`, `invoiceLines`/`line`, `invoiceSummary`/`summaryNormal`) and
// the 8+1+2 `taxNumber` decomposition come straight from that schema.
//
// Spec: NAV Online Számla Interfész-specifikáció 3.0, `InvoiceData` /
// `invoiceData.xsd`. <https://onlineszamla.nav.gov.hu/dokumentaciok>

/// Hungarian VAT identifier (`adószám`) decomposed into the three NAV
/// `taxNumber` sub-elements: the 8-digit `taxpayerId`, the 1-digit `vatCode`,
/// and the 2-digit `countyCode` (the 8+1+2 split NAV prints as
/// `12345678-2-41`).
#[derive(Clone, Debug, Eq, PartialEq)]
struct HuTaxNumber {
    taxpayer_id: String,
    vat_code: Option<String>,
    county_code: Option<String>,
}

/// The NAV `InvoiceData` namespace constant, used on the root element.
const NS_DATA: &str = "http://schemas.nav.gov.hu/OSA/3.0/data";
/// The NAV common-types namespace, bound to the `base` prefix.
const NS_BASE: &str = "http://schemas.nav.gov.hu/OSA/3.0/base";

/// Errors raised while serializing an IR document to NAV `InvoiceData` XML.
#[derive(Debug, Error)]
pub enum InvoiceDataError {
    /// The IR `document_type` has no NAV `invoiceCategory` mapping.
    #[error("document type {0:?} is not representable as a NAV invoiceCategory")]
    UnsupportedDocumentType(DocumentType),
    /// The supplier carries no usable Hungarian `taxNumber`.
    #[error("supplier has no tax id usable as NAV supplierTaxNumber")]
    MissingSupplierTaxId,
    /// A per-line VAT product or a running net/VAT/gross total overflowed the
    /// representable `Decimal` range while building the `invoiceSummary`.
    /// Untrusted invoice amounts near `Decimal::MAX` reach this rather than
    /// panicking the adapter.
    #[error("NAV invoice totals are not representable: {0}")]
    TotalsUnrepresentable(&'static str),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic NAV Online
/// Számla `InvoiceData` (RTIR) XML, per the Online Számla Interface
/// Specification v3.0 (`invoiceData.xsd`).
///
/// This emits the REAL Hungarian national document, not a UBL relabelling: the
/// document carries `invoiceNumber` / `invoiceIssueDate` on `invoiceHead`, a
/// `supplierInfo`/`customerInfo` pair whose `taxNumber` is split 8+1+2 into
/// `taxpayerId`/`vatCode`/`countyCode`, an `invoiceLines` block of `line`
/// elements (`lineNumber`, `lineNetAmountData/lineNetAmount`,
/// `lineVatRate/vatPercentage`, `lineVatData/lineVatAmount`), and an
/// `invoiceSummary`/`summaryNormal` with `invoiceNetAmount` + `invoiceVatAmount`.
///
/// Output is byte-stable by construction: a fixed element order, no maps, and
/// amounts formatted at fixed scale 2. The document is expected to have passed
/// IR validation already (it has, if built via [`CommercialDocument::new`]).
///
/// # Errors
///
/// Returns [`InvoiceDataError::UnsupportedDocumentType`] for document types with
/// no `invoiceCategory` mapping, and [`InvoiceDataError::MissingSupplierTaxId`]
/// when the supplier has no Hungarian tax id.
pub fn to_invoice_data_xml(document: &CommercialDocument) -> Result<String, InvoiceDataError> {
    let invoice_category = invoice_category(document.document_type)?;
    let supplier_tax =
        party_tax_number(&document.supplier).ok_or(InvoiceDataError::MissingSupplierTaxId)?;
    let customer_tax = party_tax_number(&document.customer);

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<InvoiceData xmlns=\"");
    out.push_str(NS_DATA);
    out.push_str("\" xmlns:base=\"");
    out.push_str(NS_BASE);
    out.push_str("\">\n");

    // --- invoiceNumber / invoiceIssueDate (InvoiceData header) ---
    el(&mut out, 1, "invoiceNumber", document.document_number.as_str());
    el(&mut out, 1, "invoiceIssueDate", document.issue_date.as_str());
    el(&mut out, 1, "completenessIndicator", "false");

    open(&mut out, 1, "invoiceMain");
    open(&mut out, 2, "invoice");

    // --- invoiceHead: issuer/recipient + general invoice data ---
    open(&mut out, 3, "invoiceHead");

    open(&mut out, 4, "supplierInfo");
    write_tax_number(&mut out, 5, "supplierTaxNumber", &supplier_tax);
    el(&mut out, 5, "supplierName", &document.supplier.name);
    write_address(&mut out, 5, "supplierAddress", &document.supplier);
    close(&mut out, 4, "supplierInfo");

    open(&mut out, 4, "customerInfo");
    // `customerVatStatus` is mandatory; a Hungarian taxable buyer is `DOMESTIC`.
    el(&mut out, 5, "customerVatStatus", "DOMESTIC");
    if let Some(tax) = &customer_tax {
        open(&mut out, 5, "customerVatData");
        write_tax_number(&mut out, 6, "customerTaxNumber", tax);
        close(&mut out, 5, "customerVatData");
    }
    el(&mut out, 5, "customerName", &document.customer.name);
    write_address(&mut out, 5, "customerAddress", &document.customer);
    close(&mut out, 4, "customerInfo");

    open(&mut out, 4, "invoiceDetail");
    el(&mut out, 5, "invoiceCategory", invoice_category);
    el(&mut out, 5, "invoiceDeliveryDate", document.issue_date.as_str());
    el(&mut out, 5, "currencyCode", document.currency.as_str());
    // Reported in HUF; a foreign currency would carry an exchangeRate here.
    el(&mut out, 5, "exchangeRate", "1");
    close(&mut out, 4, "invoiceDetail");

    close(&mut out, 3, "invoiceHead");

    // --- invoiceLines: one `line` per IR document line ---
    open(&mut out, 3, "invoiceLines");
    el(&mut out, 4, "mergedItemIndicator", "false");
    let mut net_total = Decimal::ZERO;
    let mut vat_total = Decimal::ZERO;
    for (index, line) in document.lines.iter().enumerate() {
        let rate = line_tax_rate(document, line);
        let net = line.line_extension_amount.inner();
        // checked_mul/checked_div: net * rate on untrusted amounts can exceed
        // Decimal::MAX before the divide brings it back into range. Bail with a
        // typed error rather than panicking.
        let vat = net
            .checked_mul(rate)
            .and_then(|product| product.checked_div(Decimal::ONE_HUNDRED))
            .ok_or(InvoiceDataError::TotalsUnrepresentable("lineVatAmount"))?
            .round_dp(2);
        // checked_add: summing untrusted per-line amounts can itself overflow.
        net_total = net_total
            .checked_add(net)
            .ok_or(InvoiceDataError::TotalsUnrepresentable("invoiceNetAmount"))?;
        vat_total = vat_total
            .checked_add(vat)
            .ok_or(InvoiceDataError::TotalsUnrepresentable("invoiceVatAmount"))?;

        open(&mut out, 4, "line");
        el(&mut out, 5, "lineNumber", &(index + 1).to_string());
        open(&mut out, 5, "lineDescription");
        el(&mut out, 6, "lineDescription", &line.description);
        close(&mut out, 5, "lineDescription");
        el(&mut out, 5, "quantity", &fmt_amount(line.quantity.inner()));
        el(&mut out, 5, "unitPrice", &fmt_amount(line.unit_price.inner()));

        open(&mut out, 5, "lineAmountsNormal");
        open(&mut out, 6, "lineNetAmountData");
        el(&mut out, 7, "lineNetAmount", &fmt_amount(net));
        el(&mut out, 7, "lineNetAmountHUF", &fmt_amount(net));
        close(&mut out, 6, "lineNetAmountData");
        open(&mut out, 6, "lineVatRate");
        el(&mut out, 7, "vatPercentage", &fmt_rate(rate));
        close(&mut out, 6, "lineVatRate");
        open(&mut out, 6, "lineVatData");
        el(&mut out, 7, "lineVatAmount", &fmt_amount(vat));
        el(&mut out, 7, "lineVatAmountHUF", &fmt_amount(vat));
        close(&mut out, 6, "lineVatData");
        close(&mut out, 5, "lineAmountsNormal");

        close(&mut out, 4, "line");
    }
    close(&mut out, 3, "invoiceLines");

    // --- invoiceSummary/summaryNormal: net + VAT + gross totals ---
    open(&mut out, 3, "invoiceSummary");
    open(&mut out, 4, "summaryNormal");
    el(&mut out, 5, "invoiceNetAmount", &fmt_amount(net_total));
    el(&mut out, 5, "invoiceNetAmountHUF", &fmt_amount(net_total));
    el(&mut out, 5, "invoiceVatAmount", &fmt_amount(vat_total));
    el(&mut out, 5, "invoiceVatAmountHUF", &fmt_amount(vat_total));
    close(&mut out, 4, "summaryNormal");
    // checked_add: net + vat can overflow even when each total fits on its own.
    let gross_total = net_total
        .checked_add(vat_total)
        .ok_or(InvoiceDataError::TotalsUnrepresentable("invoiceGrossAmount"))?;
    open(&mut out, 4, "summaryGrossData");
    el(&mut out, 5, "invoiceGrossAmount", &fmt_amount(gross_total));
    el(&mut out, 5, "invoiceGrossAmountHUF", &fmt_amount(gross_total));
    close(&mut out, 4, "summaryGrossData");
    close(&mut out, 3, "invoiceSummary");

    close(&mut out, 2, "invoice");
    close(&mut out, 1, "invoiceMain");
    out.push_str("</InvoiceData>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to a NAV `invoiceCategory` code (`NORMAL`,
/// `SIMPLIFIED`, `AGGREGATE`). A credit note / corrective document is reported
/// as a `NORMAL` invoice carrying a `MODIFY`/`STORNO` `invoiceOperation`
/// upstream, so it still maps to the `NORMAL` category here.
fn invoice_category(document_type: DocumentType) -> Result<&'static str, InvoiceDataError> {
    match document_type {
        DocumentType::Invoice | DocumentType::CreditNote | DocumentType::DebitNote => Ok("NORMAL"),
        other @ (DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(InvoiceDataError::UnsupportedDocumentType(other))
        }
    }
}

/// Extract a Hungarian `taxNumber` from a party: prefer a `vat` scheme id, else
/// the first tax id. The 2-letter `HU` country prefix is stripped, then the
/// digits are split 8+1+2 into `taxpayerId`/`vatCode`/`countyCode` (NAV's
/// `taxNumber` decomposition). The 8-digit core alone is valid; the 1-digit VAT
/// code and 2-digit county code are optional in the schema.
fn party_tax_number(party: &Party) -> Option<HuTaxNumber> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("vat"))
        .or_else(|| party.tax_ids.first())?;
    let digits: String = chosen
        .value
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if digits.len() < 8 {
        return None;
    }
    let taxpayer_id = digits[..8].to_owned();
    let vat_code = digits.get(8..9).map(str::to_owned);
    let county_code = digits.get(9..11).map(str::to_owned);
    Some(HuTaxNumber {
        taxpayer_id,
        vat_code,
        county_code,
    })
}

/// Write a NAV `taxNumber` structure: `base:taxpayerId` (8 digits) plus the
/// optional `base:vatCode` (1 digit) and `base:countyCode` (2 digits).
fn write_tax_number(out: &mut String, depth: usize, tag: &str, tax: &HuTaxNumber) {
    open(out, depth, tag);
    el(out, depth + 1, "base:taxpayerId", &tax.taxpayer_id);
    if let Some(vat_code) = &tax.vat_code {
        el(out, depth + 1, "base:vatCode", vat_code);
    }
    if let Some(county_code) = &tax.county_code {
        el(out, depth + 1, "base:countyCode", county_code);
    }
    close(out, depth, tag);
}

/// Write a NAV detailed address (`base:detailedAddress`): country, postal code,
/// city, and the joined street lines.
fn write_address(out: &mut String, depth: usize, tag: &str, party: &Party) {
    open(out, depth, tag);
    open(out, depth + 1, "base:detailedAddress");
    el(
        out,
        depth + 2,
        "base:countryCode",
        party.address.country.as_str(),
    );
    el(
        out,
        depth + 2,
        "base:postalCode",
        &party.address.postal_code,
    );
    el(out, depth + 2, "base:city", &party.address.city);
    el(
        out,
        depth + 2,
        "base:additionalAddressDetail",
        &party.address.lines.join(", "),
    );
    close(out, depth + 1, "base:detailedAddress");
    close(out, depth, tag);
}

/// The line's VAT percentage as a fraction (27 -> `0.27`), looked up from the
/// tax-summary entry matching the line's tax category, defaulting to zero.
fn line_tax_rate(document: &CommercialDocument, line: &invoicekit_ir::DocumentLine) -> Decimal {
    line.tax_category
        .as_ref()
        .and_then(|cat| {
            document
                .tax_summary
                .iter()
                .find(|s| &s.category_code == cat)
                .and_then(|s| s.tax_rate.as_ref())
        })
        .map_or(Decimal::ZERO, invoicekit_ir::DecimalValue::inner)
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`), deterministic.
fn fmt_amount(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

/// Format a VAT percentage as a NAV `vatPercentage` fraction (`27.00` -> the
/// `0.27` the schema expects), deterministic.
fn fmt_rate(percent: Decimal) -> String {
    (percent / Decimal::ONE_HUNDRED).normalize().to_string()
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

/// Environment selector for the NAV transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavEnvironment {
    /// `api-test.onlineszamla.nav.gov.hu` — NAV test tier.
    Test,
    /// `api.onlineszamla.nav.gov.hu` — production.
    Production,
}

/// Which operation the engine is asking the NAV to perform.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavOperation {
    /// Create — first-time submission of a new invoice.
    Create,
    /// Modify — issue a follow-up that corrects a
    /// previously-submitted invoice.
    Modify,
    /// Storno — annul a previously-submitted invoice.
    Storno,
    /// Annul (NAV-side technical annulment for accidentally
    /// duplicated submissions).
    Annul,
}

/// What the operator passes in to
/// [`NavProvider::manage_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavManageRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: NavEnvironment,
    /// Operation to perform.
    pub operation: NavOperation,
    /// Issuer's Hungarian adóazonosító (8 digits + check
    /// digit) or adószám (8 + 1 + 2 digits, hyphenated).
    pub issuer_tax_id: String,
    /// Canonical NAV `manageInvoiceRequest` XML payload.
    pub manage_invoice_xml: Vec<u8>,
}

/// NAV per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavStatus {
    /// Accepted; transaction id assigned and queued for
    /// async processing.
    Received,
    /// Processing in progress (Online Számla processes in
    /// batches).
    InProgress,
    /// Done — invoice is final and visible on the NAV
    /// portal.
    Done,
    /// Aborted — a typed validation rule rejected the
    /// payload.
    Aborted,
}

/// What [`NavProvider::manage_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavManageEnvelope {
    /// NAV-assigned transaction id.
    pub transaction_id: String,
    /// Latest observed status.
    pub status: NavStatus,
    /// RFC-3339 UTC timestamp NAV recorded.
    pub recorded_at: String,
    /// Free-form `validationResult` text when `status ==
    /// Aborted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_result: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum NavError {
    /// `manageInvoiceRequest` XML failed shape validation
    /// before the wire.
    #[error("manage invoice xml rejected: {0}")]
    BadXml(String),
    /// Issuer tax id didn't match the NAV pattern.
    #[error("invalid tax id: {0}")]
    BadTaxId(String),
    /// HTTP / TLS / DNS failure talking to NAV.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The NAV integration surface.
pub trait NavProvider: Send + Sync {
    /// Submit a `manageInvoice` request to NAV. The provider:
    ///
    /// 1. validates `issuer_tax_id` shape,
    /// 2. exchanges the engine's API credentials for a NAV
    ///    one-shot token,
    /// 3. POSTs the `manageInvoiceRequest` XML and returns
    ///    the NAV-issued envelope.
    ///
    /// # Errors
    ///
    /// Returns [`NavError`] when validation fails before the
    /// wire or transport fails on the wire. The
    /// NAV-returned `Aborted` verdict is NOT an `Err` — it's
    /// surfaced via `NavStatus::Aborted` inside the envelope
    /// so the engine persists the rejection alongside its
    /// audit trail.
    fn manage_invoice(&self, request: &NavManageRequest) -> Result<NavManageEnvelope, NavError>;

    /// Poll NAV for the latest status of a previously
    /// submitted transaction.
    ///
    /// # Errors
    ///
    /// Returns [`NavError::Transport`] when the
    /// transaction_id is unknown.
    fn query_transaction(
        &self,
        environment: NavEnvironment,
        transaction_id: &str,
    ) -> Result<NavManageEnvelope, NavError>;
}

/// Deterministic mock provider.
///
/// Emits a `Received` envelope per `manage_invoice` call and
/// `Done` per subsequent `query_transaction` so
/// cassette-replay tests can exercise the full lifecycle
/// without spinning up the NAV test tier.
pub struct MockNavProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockNavProvider {
    /// Build a mock with deterministic timestamps + serial
    /// transaction ids.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_recorded_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_recorded_at(recorded_at: impl Into<String>) -> Self {
        Self {
            fixed_recorded_at: recorded_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockNavProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl NavProvider for MockNavProvider {
    fn manage_invoice(&self, request: &NavManageRequest) -> Result<NavManageEnvelope, NavError> {
        validate_tax_id(&request.issuer_tax_id)?;
        if request.manage_invoice_xml.is_empty() {
            return Err(NavError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(NavManageEnvelope {
            transaction_id: format!("NAV-{serial:016}"),
            status: NavStatus::Received,
            recorded_at: self.fixed_recorded_at.clone(),
            validation_result: None,
        })
    }

    fn query_transaction(
        &self,
        _environment: NavEnvironment,
        transaction_id: &str,
    ) -> Result<NavManageEnvelope, NavError> {
        if transaction_id.is_empty() {
            return Err(NavError::Transport("empty transaction id".to_owned()));
        }
        Ok(NavManageEnvelope {
            transaction_id: transaction_id.to_owned(),
            status: NavStatus::Done,
            recorded_at: self.fixed_recorded_at.clone(),
            validation_result: None,
        })
    }
}

/// Validate a Hungarian tax id — either an 8-digit
/// adóazonosító (plus optional 1-digit check + 2-digit
/// area), allowing both `12345678` and `12345678-1-23`
/// shapes.
///
/// # Errors
///
/// Returns [`NavError::BadTaxId`] on shape failure.
pub fn validate_tax_id(tax_id: &str) -> Result<(), NavError> {
    let collapsed: String = tax_id.chars().filter(|c| *c != '-').collect();
    let len_ok = matches!(collapsed.len(), 8 | 9 | 11);
    let digits_ok = collapsed.bytes().all(|b| b.is_ascii_digit());
    if len_ok && digits_ok {
        Ok(())
    } else {
        Err(NavError::BadTaxId(format!(
            "tax id must be 8/9/11 digits (optionally hyphenated as 8-1-2), got {tax_id:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_hu_nav::crate_name(),
///     "invoicekit-report-hu-nav"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-hu-nav"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId, DocumentLine,
        DocumentMeta, DocumentNumber, Iso4217Code, MonetaryTotal, PartyTaxId, PostalAddress,
        SchemaVersion, TaxCategorySummary,
    };

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn hu_party(name: &str, vat: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: vat.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["Andrássy út 1".to_owned()],
                city: city.to_owned(),
                subdivision: None,
                postal_code: "1061".to_owned(),
                country: CountryCode::new("HU").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-hu-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("INV-2026-HU-0001").unwrap(),
            currency: Iso4217Code::new("HUF").unwrap(),
            // "HU12345678-2-41" -> taxpayerId 12345678, vatCode 2, countyCode 41.
            supplier: hu_party("Acme Kft", "HU12345678-2-41", "Budapest"),
            customer: hu_party("Beta Zrt", "HU98765432-2-42", "Debrecen"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Szoftverfejlesztés & tanácsadás".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                // Hungary's 27% standard VAT rate.
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2700),
                tax_rate: Some(DecimalValue::new(Decimal::new(2700, 2))),
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(10000),
                tax_exclusive_amount: amt(10000),
                tax_inclusive_amount: amt(12700),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(12700),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant_hu".to_owned(),
                trace_id: "trace_hu".to_owned(),
                source_system: Some("unit".to_owned()),
            },
        })
        .unwrap()
    }

    #[test]
    fn invoice_data_contains_real_nav_element_names() {
        let xml = to_invoice_data_xml(&sample_invoice()).unwrap();
        for needle in [
            "<InvoiceData xmlns=\"http://schemas.nav.gov.hu/OSA/3.0/data\"",
            "xmlns:base=\"http://schemas.nav.gov.hu/OSA/3.0/base\"",
            "<invoiceNumber>INV-2026-HU-0001</invoiceNumber>",
            "<invoiceIssueDate>2026-05-26</invoiceIssueDate>",
            "<supplierInfo>",
            "<supplierTaxNumber>",
            "<base:taxpayerId>12345678</base:taxpayerId>",
            "<base:vatCode>2</base:vatCode>",
            "<base:countyCode>41</base:countyCode>",
            "<supplierName>Acme Kft</supplierName>",
            "<customerInfo>",
            "<customerTaxNumber>",
            "<base:taxpayerId>98765432</base:taxpayerId>",
            "<invoiceCategory>NORMAL</invoiceCategory>",
            "<currencyCode>HUF</currencyCode>",
            "<invoiceLines>",
            "<line>",
            "<lineNumber>1</lineNumber>",
            "<lineNetAmount>100.00</lineNetAmount>",
            "<vatPercentage>0.27</vatPercentage>",
            "<lineVatData>",
            "<lineVatAmount>27.00</lineVatAmount>",
            "<invoiceSummary>",
            "<summaryNormal>",
            "<invoiceNetAmount>100.00</invoiceNetAmount>",
            "<invoiceVatAmount>27.00</invoiceVatAmount>",
            // XML escaping is applied to text content.
            "<lineDescription>Szoftverfejlesztés &amp; tanácsadás</lineDescription>",
        ] {
            assert!(xml.contains(needle), "InvoiceData missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn invoice_data_is_deterministic() {
        let doc = sample_invoice();
        assert_eq!(
            to_invoice_data_xml(&doc).unwrap(),
            to_invoice_data_xml(&doc).unwrap()
        );
    }

    #[test]
    fn invoice_data_summary_aggregates_lines() {
        let xml = to_invoice_data_xml(&sample_invoice()).unwrap();
        // 100.00 net @ 27% -> 27.00 VAT -> 127.00 gross.
        assert!(xml.contains("<invoiceGrossAmount>127.00</invoiceGrossAmount>"));
    }

    #[test]
    fn invoice_data_rejects_unsupported_document_type() {
        let err = invoice_category(DocumentType::ProForma).unwrap_err();
        assert!(matches!(err, InvoiceDataError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn invoice_data_credit_note_is_normal_category() {
        // A credit note still serializes as a NORMAL invoiceCategory (the
        // reversal is carried by the upstream STORNO invoiceOperation).
        assert_eq!(invoice_category(DocumentType::CreditNote).unwrap(), "NORMAL");
    }

    #[test]
    fn invoice_data_requires_supplier_tax_id() {
        // A supplier with no usable tax id must be unrepresentable as a NAV
        // supplierTaxNumber. The IR is built valid, then the supplier's tax ids
        // are cleared before serialization.
        let mut doc = sample_invoice();
        doc.supplier.tax_ids.clear();
        let err = to_invoice_data_xml(&doc).unwrap_err();
        assert!(matches!(err, InvoiceDataError::MissingSupplierTaxId));
    }

    fn sample_request() -> NavManageRequest {
        NavManageRequest {
            tenant_id: "tenant-hu-test".to_owned(),
            environment: NavEnvironment::Test,
            operation: NavOperation::Create,
            issuer_tax_id: "12345678-1-23".to_owned(),
            manage_invoice_xml: b"<manageInvoiceRequest/>".to_vec(),
        }
    }

    #[test]
    fn manage_invoice_returns_received_with_transaction_id() {
        let p = MockNavProvider::default();
        let env = p.manage_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, NavStatus::Received);
        assert!(env.transaction_id.starts_with("NAV-"));
        assert_eq!(env.recorded_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn manage_invoice_serial_increments_per_provider() {
        let p = MockNavProvider::default();
        let env1 = p.manage_invoice(&sample_request()).unwrap();
        let env2 = p.manage_invoice(&sample_request()).unwrap();
        assert_ne!(env1.transaction_id, env2.transaction_id);
    }

    #[test]
    fn manage_invoice_rejects_empty_payload() {
        let p = MockNavProvider::default();
        let mut req = sample_request();
        req.manage_invoice_xml.clear();
        let err = p.manage_invoice(&req).unwrap_err();
        assert!(matches!(err, NavError::BadXml(_)));
    }

    #[test]
    fn manage_invoice_rejects_bad_tax_id() {
        let p = MockNavProvider::default();
        let mut req = sample_request();
        req.issuer_tax_id = "BAD".to_owned();
        let err = p.manage_invoice(&req).unwrap_err();
        assert!(matches!(err, NavError::BadTaxId(_)));
    }

    #[test]
    fn query_transaction_returns_done() {
        let p = MockNavProvider::default();
        let env = p
            .query_transaction(NavEnvironment::Test, "NAV-0000000000000001")
            .unwrap();
        assert_eq!(env.status, NavStatus::Done);
    }

    #[test]
    fn query_transaction_rejects_empty_id() {
        let p = MockNavProvider::default();
        let err = p.query_transaction(NavEnvironment::Test, "").unwrap_err();
        assert!(matches!(err, NavError::Transport(_)));
    }

    #[test]
    fn validate_tax_id_accepts_8_9_or_11_digit_shapes() {
        assert!(validate_tax_id("12345678").is_ok());
        assert!(validate_tax_id("123456789").is_ok());
        assert!(validate_tax_id("12345678123").is_ok());
        assert!(validate_tax_id("12345678-1-23").is_ok());
    }

    #[test]
    fn validate_tax_id_rejects_wrong_lengths() {
        assert!(validate_tax_id("1234567").is_err());
        assert!(validate_tax_id("1234567890").is_err());
    }

    #[test]
    fn validate_tax_id_rejects_non_digits() {
        assert!(validate_tax_id("1234567A").is_err());
        assert!(validate_tax_id("12345678-1-2A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = NavManageEnvelope {
            transaction_id: "NAV-0000000000000007".to_owned(),
            status: NavStatus::Aborted,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            validation_result: Some("INVOICE_NUMBER required".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: NavManageEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    /// Two lines each near `Decimal::MAX` overflow the `net_total` accumulator.
    /// Before the `checked_add` fix this panicked (Decimal's `+=` panics on
    /// overflow); now it returns a typed [`InvoiceDataError::TotalsUnrepresentable`].
    #[test]
    fn invoice_data_net_total_overflow_returns_error_not_panic() {
        let huge = DecimalValue::new(Decimal::MAX);
        let mut doc = sample_invoice();
        doc.lines = vec![
            DocumentLine {
                id: "1".to_owned(),
                description: "near-max line a".to_owned(),
                quantity: DecimalValue::new(Decimal::ONE),
                unit_code: Some("EA".to_owned()),
                unit_price: huge.clone(),
                line_extension_amount: huge.clone(),
                // No tax_category -> line_tax_rate is zero, so the per-line VAT
                // product stays zero and the overflow lands on the net total.
                tax_category: None,
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
            DocumentLine {
                id: "2".to_owned(),
                description: "near-max line b".to_owned(),
                quantity: DecimalValue::new(Decimal::ONE),
                unit_code: Some("EA".to_owned()),
                unit_price: huge.clone(),
                line_extension_amount: huge,
                tax_category: None,
                classifications: Vec::new(),
                extensions: Vec::new(),
            },
        ];
        doc.tax_summary = Vec::new();
        let err = to_invoice_data_xml(&doc)
            .expect_err("two near-MAX net lines must overflow the net total");
        assert!(matches!(err, InvoiceDataError::TotalsUnrepresentable(_)));
    }

    /// A single line whose `net * rate` product overflows `Decimal::MAX` before
    /// the divide by 100. Before the `checked_mul` fix this panicked; now it
    /// returns a typed [`InvoiceDataError::TotalsUnrepresentable`].
    #[test]
    fn invoice_data_vat_product_overflow_returns_error_not_panic() {
        let huge = DecimalValue::new(Decimal::MAX);
        let mut doc = sample_invoice();
        doc.lines = vec![DocumentLine {
            id: "1".to_owned(),
            description: "near-max line".to_owned(),
            quantity: DecimalValue::new(Decimal::ONE),
            unit_code: Some("EA".to_owned()),
            unit_price: huge.clone(),
            line_extension_amount: huge,
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }];
        // tax_rate of 27 means net * 27 = MAX * 27 overflows before the
        // division by 100 can bring it back into range.
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: DecimalValue::new(Decimal::MAX),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::from(27))),
        }];
        let err = to_invoice_data_xml(&doc)
            .expect_err("MAX * 27 VAT product must overflow before the divide");
        assert!(matches!(err, InvoiceDataError::TotalsUnrepresentable(_)));
    }

    #[test]
    fn operation_serde_round_trips_all_four_variants() {
        for op in [
            NavOperation::Create,
            NavOperation::Modify,
            NavOperation::Storno,
            NavOperation::Annul,
        ] {
            let json = serde_json::to_string(&op).unwrap();
            let parsed: NavOperation = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, op);
        }
    }
}
