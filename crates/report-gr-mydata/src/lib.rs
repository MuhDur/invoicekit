// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Greece **myDATA** (Άυλο Διασύνδεσμο) e-books / e-invoicing reporting adapter.
//!
//! myDATA is Greece's mandatory continuous reporting of
//! invoices to the IAPR (Independent Authority for Public
//! Revenue, ΑΑΔΕ). Issuers transmit invoice summaries to the
//! IAPR REST endpoints; the authority returns a **MARK**
//! (Μοναδικός Αριθμός Καταχώρησης — Unique Registration
//! Number) plus a **UID** that the issuer must embed in the
//! printed invoice's QR code.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockMyDataProvider`]. The live REST integration lands in
//! a follow-up `crates/report-gr-mydata-http/` crate behind a
//! feature flag so operators who only need the substrate don't
//! pull in the HTTP stack.
//!
//! Reference reading: IAPR myDATA documentation portal at
//! <https://www.aade.gr/mydata>.

#![allow(clippy::doc_markdown)]

use invoicekit_ir::{CommercialDocument, DocumentType, Party, ReferenceKindClass};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// myDATA InvoicesDoc serialization (IR -> national AADE myDATA XML)
// ---------------------------------------------------------------------------

/// myDATA `InvoicesDoc` transmission context: the document-level header fields
/// that live in the AADE `invoiceHeader` but are not part of the
/// jurisdiction-agnostic IR.
///
/// Reference: AADE myDATA REST API / `InvoicesDoc` XSD, namespace
/// `http://www.aade.gr/myDATA/invoice/v1.0`
/// (<https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyDataDocContext {
    /// `series` — the invoice series the issuer assigns (e.g. `"A"`). The XSD
    /// element is named `series`.
    pub series: String,
    /// `branch` — the issuer establishment branch number. `0` is the head
    /// office. Carried on both `issuer` and `counterpart`.
    pub issuer_branch: u32,
    /// `branch` — the counterpart (buyer) establishment branch number.
    pub counterpart_branch: u32,
}

impl Default for MyDataDocContext {
    fn default() -> Self {
        Self {
            series: "A".to_owned(),
            issuer_branch: 0,
            counterpart_branch: 0,
        }
    }
}

/// Errors raised while serializing an IR document to a myDATA `InvoicesDoc`.
#[derive(Debug, Error)]
pub enum MyDataXmlError {
    /// The IR `document_type` has no myDATA `invoiceType` mapping.
    #[error("document type {0:?} is not representable as a myDATA invoiceType")]
    UnsupportedDocumentType(DocumentType),
    /// The issuer carries no usable VAT number for the `issuer/vatNumber`.
    #[error("issuer has no tax id usable as issuer/vatNumber")]
    MissingIssuerVatNumber,
    /// The counterpart carries no usable VAT number for `counterpart/vatNumber`.
    #[error("counterpart has no tax id usable as counterpart/vatNumber")]
    MissingCounterpartVatNumber,
    /// The transmission context was malformed (e.g. a blank `series`).
    #[error("invalid myDATA document context: {0}")]
    BadContext(String),
    /// A monetary total or a per-line pro-rated VAT product overflowed the
    /// representable `Decimal` range while accumulating the `invoiceSummary`.
    /// Untrusted invoice amounts near `Decimal::MAX` reach this rather than
    /// panicking the adapter.
    #[error("myDATA totals are not representable: {0}")]
    TotalsUnrepresentable(&'static str),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to a deterministic AADE
/// **myDATA** `InvoicesDoc` XML document.
///
/// This is the REAL Greek national reporting format, not UBL relabelled. The
/// element names and nesting follow the AADE myDATA `InvoicesDoc` XSD
/// (namespace `http://www.aade.gr/myDATA/invoice/v1.0`): an `InvoicesDoc` root
/// wrapping one `invoice`, whose children are — in XSD order — `issuer`,
/// `counterpart`, `invoiceHeader`, one `invoiceDetails` per line, and a single
/// `invoiceSummary`. Reference: AADE myDATA REST API documentation,
/// <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>.
///
/// * `issuer` / `counterpart` carry `vatNumber`, `country`, `branch`. The
///   `EL` EU-VAT prefix is stripped so `vatNumber` is the bare nine-digit AFM,
///   per the myDATA convention.
/// * `invoiceHeader` carries `series`, `aa` (the document number), `issueDate`,
///   and `invoiceType` (the AADE classification code, e.g. `1.1`). When the IR
///   document carries a preceding-invoice [`reference`](CommercialDocument::references)
///   (a credit/debit note pointing back at the corrected invoice), the
///   referenced identifier is emitted verbatim as `correlatedInvoices` — the
///   myDATA `InvoiceHeaderType` element that links a credit note (invoiceType
///   `5.x`) to the original invoice's MARK. The IR id is emitted byte-for-byte;
///   the adapter does not derive, parse, or validate it.
/// * Each `invoiceDetails` row carries `lineNumber`, `netValue`, `vatAmount`,
///   `vatCategory` (the integer myDATA code), and — only when `vatCategory` is
///   `7` (excluding VAT) — a mandatory `vatExemptionCategory`.
/// * `invoiceSummary` carries `totalNetValue`, `totalVatAmount`, and
///   `totalGrossValue`.
///
/// Output is byte-stable by construction: a fixed element order with no maps
/// and amounts formatted at fixed scale 2.
///
/// # Errors
///
/// Returns [`MyDataXmlError::UnsupportedDocumentType`] for document types with
/// no `invoiceType` mapping, [`MyDataXmlError::MissingIssuerVatNumber`] /
/// [`MyDataXmlError::MissingCounterpartVatNumber`] when a party has no VAT
/// number, and [`MyDataXmlError::BadContext`] when the context is malformed.
pub fn to_invoices_doc_xml(
    document: &CommercialDocument,
    context: &MyDataDocContext,
) -> Result<String, MyDataXmlError> {
    if context.series.trim().is_empty() {
        return Err(MyDataXmlError::BadContext(
            "series must not be empty".to_owned(),
        ));
    }
    let invoice_type = invoice_type_code(document.document_type)?;
    let issuer = party_vat(&document.supplier).ok_or(MyDataXmlError::MissingIssuerVatNumber)?;
    let counterpart =
        party_vat(&document.customer).ok_or(MyDataXmlError::MissingCounterpartVatNumber)?;

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<InvoicesDoc xmlns=\"http://www.aade.gr/myDATA/invoice/v1.0\">\n",
    );
    open(&mut out, 1, "invoice");

    // --- issuer ---
    open(&mut out, 2, "issuer");
    el(&mut out, 3, "vatNumber", &issuer.vat_number);
    el(&mut out, 3, "country", &issuer.country);
    el(&mut out, 3, "branch", &context.issuer_branch.to_string());
    close(&mut out, 2, "issuer");

    // --- counterpart ---
    open(&mut out, 2, "counterpart");
    el(&mut out, 3, "vatNumber", &counterpart.vat_number);
    el(&mut out, 3, "country", &counterpart.country);
    el(&mut out, 3, "branch", &context.counterpart_branch.to_string());
    close(&mut out, 2, "counterpart");

    // --- invoiceHeader ---
    open(&mut out, 2, "invoiceHeader");
    el(&mut out, 3, "series", &context.series);
    el(&mut out, 3, "aa", document.document_number.as_str());
    el(&mut out, 3, "issueDate", document.issue_date.as_str());
    el(&mut out, 3, "invoiceType", invoice_type.code());
    // `correlatedInvoices` (myDATA `InvoiceHeaderType`) links a credit/debit note
    // to the MARK / number of the original invoice it corrects. We emit the
    // producer-supplied reference identifier verbatim — no derivation, parsing,
    // or catalog lookup. The reference is selected by its EN 16931 classification
    // (`PrecedingInvoice`), so only an original-invoice link is routed here.
    if let Some(preceding) = document
        .references
        .iter()
        .find(|r| r.kind_class() == ReferenceKindClass::PrecedingInvoice)
    {
        el(&mut out, 3, "correlatedInvoices", preceding.id.as_str());
    }
    close(&mut out, 2, "invoiceHeader");

    // --- invoiceDetails (one row per line) ---
    let mut total_net = Decimal::ZERO;
    let mut total_vat = Decimal::ZERO;
    for (index, line) in document.lines.iter().enumerate() {
        let net = line.line_extension_amount.inner();
        let (vat_amount, vat_category, exemption) = line_vat(document, line)?;
        // checked_add: summing untrusted per-line amounts can exceed
        // Decimal::MAX. Bail with a typed error rather than panicking.
        total_net = total_net
            .checked_add(net)
            .ok_or(MyDataXmlError::TotalsUnrepresentable("totalNetValue"))?;
        total_vat = total_vat
            .checked_add(vat_amount)
            .ok_or(MyDataXmlError::TotalsUnrepresentable("totalVatAmount"))?;
        open(&mut out, 2, "invoiceDetails");
        el(&mut out, 3, "lineNumber", &(index + 1).to_string());
        el(&mut out, 3, "netValue", &fmt_amount(net));
        el(&mut out, 3, "vatAmount", &fmt_amount(vat_amount));
        el(&mut out, 3, "vatCategory", &vat_category.to_string());
        // `vatExemptionCategory` is mandatory exactly when vatCategory == 7
        // (excluding VAT) per the myDATA XSD; emitted only then.
        if vat_category == VAT_CATEGORY_EXCLUDING {
            if let Some(exemption_code) = exemption {
                el(
                    &mut out,
                    3,
                    "vatExemptionCategory",
                    &exemption_code.to_string(),
                );
            }
        }
        close(&mut out, 2, "invoiceDetails");
    }

    // --- invoiceSummary ---
    // checked_add: net + vat can itself overflow even when each fit.
    let total_gross = total_net
        .checked_add(total_vat)
        .ok_or(MyDataXmlError::TotalsUnrepresentable("totalGrossValue"))?;
    open(&mut out, 2, "invoiceSummary");
    el(&mut out, 3, "totalNetValue", &fmt_amount(total_net));
    el(&mut out, 3, "totalVatAmount", &fmt_amount(total_vat));
    el(&mut out, 3, "totalGrossValue", &fmt_amount(total_gross));
    close(&mut out, 2, "invoiceSummary");

    close(&mut out, 1, "invoice");
    out.push_str("</InvoicesDoc>\n");
    Ok(out)
}

/// myDATA `invoiceType` classification code derived from the IR document type.
///
/// The AADE `invoiceType` element takes a code from the myDATA classification
/// (`1.1` sales of goods, `2.1` services, `5.1` associated credit note, ...).
/// We map the structural IR [`DocumentType`] to a sensible default code; the
/// caller can target a finer sub-code through [`MyDataInvoiceCategory`] on the
/// report request. Reference: AADE myDATA `invoiceType` codelist.
fn invoice_type_code(document_type: DocumentType) -> Result<MyDataInvoiceCategory, MyDataXmlError> {
    match document_type {
        DocumentType::Invoice => Ok(MyDataInvoiceCategory::SalesGoods {
            code: "1.1".to_owned(),
        }),
        DocumentType::CreditNote => Ok(MyDataInvoiceCategory::CreditNote {
            code: "5.1".to_owned(),
        }),
        DocumentType::SelfBilled => Ok(MyDataInvoiceCategory::SelfBilling {
            code: "3.1".to_owned(),
        }),
        other @ (DocumentType::DebitNote | DocumentType::ProForma) => {
            Err(MyDataXmlError::UnsupportedDocumentType(other))
        }
    }
}

/// An issuer / counterpart VAT projection: the bare nine-digit AFM plus the
/// ISO country code.
struct PartyVat {
    vat_number: String,
    country: String,
}

/// Extract `(vatNumber, country)` from a party: prefer a `vat` scheme id, else
/// the first tax id. The two-letter country prefix (e.g. `EL`) is stripped so
/// `vatNumber` is the bare AFM the myDATA endpoints expect.
fn party_vat(party: &Party) -> Option<PartyVat> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("vat"))
        .or_else(|| party.tax_ids.first())?;
    let country = party.address.country.as_str().to_owned();
    let vat_number = strip_vat_country_prefix(&chosen.value);
    Some(PartyVat {
        vat_number,
        country,
    })
}

/// Strip a leading two-letter VAT country prefix from a VAT value
/// (`"EL123456789"` -> `"123456789"`). Greek EU-VAT ids prefix the AFM with
/// `EL`; the myDATA `vatNumber` field wants the bare AFM.
fn strip_vat_country_prefix(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() > 2 && bytes[..2].iter().all(u8::is_ascii_alphabetic) {
        value[2..].to_owned()
    } else {
        value.to_owned()
    }
}

/// The myDATA `vatCategory` integer code for "excluding VAT" (0% / exempt). The
/// XSD makes `vatExemptionCategory` mandatory exactly for this category.
const VAT_CATEGORY_EXCLUDING: u8 = 7;

/// Resolve a line's `(vatAmount, vatCategory, vatExemptionCategory?)` from the
/// matching tax-summary entry.
///
/// `vatCategory` is the myDATA integer code derived from the percentage rate
/// (`1` = 24% standard, `2` = 13%, `3` = 6%, `7` = 0% / excluding VAT) per the
/// AADE myDATA `vatCategory` codelist. The line's share of the band VAT is the
/// band tax pro-rated by the line's net over the band's taxable base.
fn line_vat(
    document: &CommercialDocument,
    line: &invoicekit_ir::DocumentLine,
) -> Result<(Decimal, u8, Option<u8>), MyDataXmlError> {
    let summary = line.tax_category.as_ref().and_then(|cat| {
        document
            .tax_summary
            .iter()
            .find(|s| &s.category_code == cat)
    });
    let Some(summary) = summary else {
        return Ok((Decimal::ZERO, VAT_CATEGORY_EXCLUDING, Some(1)));
    };
    let rate = summary
        .tax_rate
        .as_ref()
        .map_or(Decimal::ZERO, invoicekit_ir::DecimalValue::inner);
    let vat_category = vat_category_for_rate(rate);
    // Pro-rate the band VAT onto this line by its net share of the band base.
    let band_base = summary.taxable_amount.inner();
    let line_net = line.line_extension_amount.inner();
    let vat_amount = if band_base.is_zero() {
        Decimal::ZERO
    } else {
        // checked_mul/checked_div: tax_amount * line_net on untrusted amounts
        // can exceed Decimal::MAX. Bail with a typed error instead of panicking.
        summary
            .tax_amount
            .inner()
            .checked_mul(line_net)
            .and_then(|product| product.checked_div(band_base))
            .ok_or(MyDataXmlError::TotalsUnrepresentable("line vatAmount"))?
            .round_dp(2)
    };
    let exemption =
        (vat_category == VAT_CATEGORY_EXCLUDING).then_some(1_u8);
    Ok((vat_amount, vat_category, exemption))
}

/// Map a VAT percentage to the myDATA `vatCategory` integer code.
///
/// AADE myDATA `vatCategory` codelist: `1` = 24% (standard), `2` = 13%,
/// `3` = 6%, `4` = 17%, `5` = 9%, `6` = 4%, `7` = 0% / records excluding VAT.
fn vat_category_for_rate(rate: Decimal) -> u8 {
    // Compare on whole-percent values to stay robust to scale (24.00 vs 24).
    let whole = rate.round_dp(0).normalize();
    if whole == Decimal::from(24) {
        1
    } else if whole == Decimal::from(13) {
        2
    } else if whole == Decimal::from(6) {
        3
    } else if whole == Decimal::from(17) {
        4
    } else if whole == Decimal::from(9) {
        5
    } else if whole == Decimal::from(4) {
        6
    } else {
        // Zero / unknown -> "excluding VAT" (7); requires vatExemptionCategory.
        VAT_CATEGORY_EXCLUDING
    }
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`, `0` -> `"0.00"`),
/// deterministic. Rounds to two places then pins the scale so trailing zeros
/// always render.
fn fmt_amount(value: Decimal) -> String {
    let mut rounded = value.round_dp(2);
    rounded.rescale(2);
    rounded.to_string()
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

/// Environment selector for the IAPR transport. Operators
/// pick at engine-construction time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataEnvironment {
    /// `mydata-dev.azure-api.net` — the IAPR sandbox tier.
    Sandbox,
    /// `mydatapi.aade.gr` — production.
    Production,
}

/// myDATA invoice classification per IAPR taxonomy.
///
/// Codes mirror the official `invoiceType` field on the
/// myDATA REST API (`1.1` sales of goods, `1.2` ICA goods,
/// `2.1` services, `2.2` ICA services, etc.). The strings
/// stay opaque so the engine can target newer taxonomies
/// without bumping this enum.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataInvoiceCategory {
    /// `1.x` — sales of goods.
    SalesGoods {
        /// Sub-code, e.g. `"1.1"`, `"1.2"`, `"1.3"`.
        code: String,
    },
    /// `2.x` — provision of services.
    Services {
        /// Sub-code, e.g. `"2.1"`, `"2.2"`, `"2.3"`.
        code: String,
    },
    /// `3.x` — title of acquisition (self-billing).
    SelfBilling {
        /// Sub-code, e.g. `"3.1"`, `"3.2"`.
        code: String,
    },
    /// `5.x` — credit note.
    CreditNote {
        /// Sub-code, e.g. `"5.1"` (associated), `"5.2"`
        /// (non-associated).
        code: String,
    },
    /// `8.x` — payroll, deductions, statements.
    Statement {
        /// Sub-code, e.g. `"8.1"`, `"8.2"`.
        code: String,
    },
    /// Escape hatch for codes the engine hasn't yet enumerated.
    Other {
        /// Raw IAPR `invoiceType` code as published on the
        /// myDATA portal.
        code: String,
    },
}

impl MyDataInvoiceCategory {
    /// Borrow the IAPR sub-code as a string slice.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::SalesGoods { code }
            | Self::Services { code }
            | Self::SelfBilling { code }
            | Self::CreditNote { code }
            | Self::Statement { code }
            | Self::Other { code } => code.as_str(),
        }
    }
}

/// **MARK** — Μοναδικός Αριθμός Καταχώρησης / Unique Registration Number.
///
/// The IAPR assigns one per accepted invoice. Engine persists
/// this on the canonical document so every downstream artefact
/// carries it.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MyDataMark(pub String);

impl MyDataMark {
    /// Build a new MARK from any string-shaped value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Borrow the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// **UID** — the unique invoice identifier the IAPR computes
/// (SHA-1 over a canonical projection of the invoice fields).
/// Embedded in the printed-invoice QR code alongside the MARK.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MyDataUid(pub String);

impl MyDataUid {
    /// Build a new UID from any string-shaped value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Borrow the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the operator passes in to
/// [`MyDataProvider::report_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyDataReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: MyDataEnvironment,
    /// Issuer's Greek tax registration number (ΑΦΜ, Α.Φ.Μ.).
    pub issuer_afm: String,
    /// Optional buyer ΑΦΜ; some invoice types (e.g. retail)
    /// omit it.
    pub buyer_afm: Option<String>,
    /// myDATA category for this invoice.
    pub category: MyDataInvoiceCategory,
    /// Canonical InvoicesDoc XML payload the IAPR expects.
    pub invoices_doc_xml: Vec<u8>,
}

/// IAPR per-invoice verdict after a `report_invoice` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataStatus {
    /// Successfully recorded; MARK + UID are returned.
    Accepted,
    /// Accepted with warnings (e.g. cross-checking against
    /// the buyer's classifications produced a low-severity
    /// flag). Engine should surface the warning text but the
    /// MARK is valid.
    AcceptedWithWarnings,
    /// IAPR refused the submission; no MARK is assigned. Fix
    /// + resubmit.
    Rejected,
}

/// What [`MyDataProvider::report_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyDataReportEnvelope {
    /// IAPR verdict.
    pub status: MyDataStatus,
    /// MARK assigned by the IAPR when `status != Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<MyDataMark>,
    /// UID assigned by the IAPR when `status != Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<MyDataUid>,
    /// Raw error or warning text from the IAPR. Engines
    /// surface this verbatim in the audit log.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC-3339 UTC timestamp the IAPR recorded.
    pub reported_at: String,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum MyDataError {
    /// The invoices doc XML did not parse / wasn't InvoicesDoc.
    #[error("invoices doc xml rejected: {0}")]
    BadXml(String),
    /// The issuer ΑΦΜ wasn't 9 ASCII digits.
    #[error("invalid issuer AFM: {0}")]
    BadAfm(String),
    /// Transport-level failure talking to the IAPR.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The Greece myDATA reporting surface. Real IAPR HTTP
/// integrations satisfy this trait; the mock below is what
/// tests + cassette-replay use.
pub trait MyDataProvider: Send + Sync {
    /// Report one invoice to the IAPR.
    ///
    /// # Errors
    ///
    /// Returns [`MyDataError`] when validation fails before
    /// the wire or transport fails on the wire. The
    /// IAPR-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via [`MyDataStatus::Rejected`] inside
    /// [`MyDataReportEnvelope`] so the engine can persist the
    /// rejection alongside its audit trail.
    fn report_invoice(
        &self,
        request: &MyDataReportRequest,
    ) -> Result<MyDataReportEnvelope, MyDataError>;
}

/// Deterministic mock provider. Returns
/// [`MyDataStatus::Accepted`] with a synthesised MARK + UID
/// derived from the request, so cassette-replay tests are
/// byte-identical across runs.
pub struct MockMyDataProvider {
    fixed_reported_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockMyDataProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_reported_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp (the mock
    /// emits this value verbatim in every
    /// [`MyDataReportEnvelope`]).
    #[must_use]
    pub fn with_fixed_reported_at(reported_at: impl Into<String>) -> Self {
        Self {
            fixed_reported_at: reported_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockMyDataProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MyDataProvider for MockMyDataProvider {
    fn report_invoice(
        &self,
        request: &MyDataReportRequest,
    ) -> Result<MyDataReportEnvelope, MyDataError> {
        validate_afm(&request.issuer_afm)?;
        if request.invoices_doc_xml.is_empty() {
            return Err(MyDataError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut guard = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *guard;
            *guard += 1;
            v
        };
        let mark = MyDataMark::new(format!("4000{serial:012}"));
        let uid = MyDataUid::new(format!("MYDATA-MOCK-UID-{serial:08}"));
        Ok(MyDataReportEnvelope {
            status: MyDataStatus::Accepted,
            mark: Some(mark),
            uid: Some(uid),
            message: None,
            reported_at: self.fixed_reported_at.clone(),
        })
    }
}

/// Validate that an ΑΦΜ is exactly 9 ASCII digits. The Greek
/// AFM checksum is a separate concern; this helper only
/// catches obviously-wrong shapes before the wire.
///
/// # Errors
///
/// Returns [`MyDataError::BadAfm`] when the input isn't 9
/// ASCII digits.
pub fn validate_afm(afm: &str) -> Result<(), MyDataError> {
    if afm.len() == 9 && afm.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(MyDataError::BadAfm(format!(
            "AFM must be 9 ASCII digits, got {afm:?}"
        )))
    }
}

/// Build the QR-code payload string the IAPR's e-books portal
/// expects on a printed invoice. Format per IAPR Annex 1:
/// `{base_url}/?mark={MARK}&uid={UID}`.
///
/// # Errors
///
/// Returns [`MyDataError::BadXml`] when the supplied envelope
/// lacks a MARK or UID (i.e. status was `Rejected`).
pub fn qr_payload(base_url: &str, envelope: &MyDataReportEnvelope) -> Result<String, MyDataError> {
    let mark = envelope
        .mark
        .as_ref()
        .ok_or_else(|| MyDataError::BadXml("envelope carries no MARK".to_owned()))?;
    let uid = envelope
        .uid
        .as_ref()
        .ok_or_else(|| MyDataError::BadXml("envelope carries no UID".to_owned()))?;
    Ok(format!(
        "{base_url}/?mark={}&uid={}",
        mark.as_str(),
        uid.as_str()
    ))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_gr_mydata::crate_name(),
///     "invoicekit-report-gr-mydata"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-gr-mydata"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType,
        Iso4217Code, MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion,
        TaxCategorySummary,
    };

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn greek_party(name: &str, vat: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: vat.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["Leoforos Kifisias 1".to_owned()],
                city: city.to_owned(),
                subdivision: None,
                postal_code: "11523".to_owned(),
                country: CountryCode::new("GR").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-gr-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("INV-2026-GR-0001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: greek_party("Acme Hellas AE", "EL123456789", "Athina"),
            customer: greek_party("Beta EPE", "EL987654321", "Thessaloniki"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Symvouleftikes ypiresies".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            // Greek standard VAT rate is 24% -> myDATA vatCategory 1.
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2400),
                tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(10000),
                tax_exclusive_amount: amt(10000),
                tax_inclusive_amount: amt(12400),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(12400),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_gr".to_owned(),
                trace_id: "trace_gr".to_owned(),
                source_system: Some("unit".to_owned()),
            },
        })
        .unwrap()
    }

    #[test]
    fn invoices_doc_contains_mandatory_mydata_structure() {
        let xml = to_invoices_doc_xml(&sample_invoice(), &MyDataDocContext::default()).unwrap();
        for needle in [
            "<InvoicesDoc xmlns=\"http://www.aade.gr/myDATA/invoice/v1.0\">",
            "<invoice>",
            "<issuer>",
            "<vatNumber>123456789</vatNumber>",
            "<country>GR</country>",
            "<branch>0</branch>",
            "<counterpart>",
            "<vatNumber>987654321</vatNumber>",
            "<invoiceHeader>",
            "<series>A</series>",
            "<aa>INV-2026-GR-0001</aa>",
            "<issueDate>2026-05-26</issueDate>",
            "<invoiceType>1.1</invoiceType>",
            "<invoiceDetails>",
            "<lineNumber>1</lineNumber>",
            "<netValue>100.00</netValue>",
            "<vatAmount>24.00</vatAmount>",
            "<vatCategory>1</vatCategory>",
            "<invoiceSummary>",
            "<totalNetValue>100.00</totalNetValue>",
            "<totalVatAmount>24.00</totalVatAmount>",
            "<totalGrossValue>124.00</totalGrossValue>",
        ] {
            assert!(xml.contains(needle), "InvoicesDoc missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn invoices_doc_is_deterministic() {
        let doc = sample_invoice();
        let ctx = MyDataDocContext::default();
        assert_eq!(
            to_invoices_doc_xml(&doc, &ctx).unwrap(),
            to_invoices_doc_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn invoices_doc_escapes_xml_special_chars() {
        let mut doc = sample_invoice();
        doc.lines[0].description = "A & B <co>".to_owned();
        // Description is not emitted in the InvoicesDoc summary form, but the
        // counterpart name path / escaping helper must be exercised; assert the
        // escaper itself on a value-bearing field by routing through series.
        let ctx = MyDataDocContext {
            series: "A&B".to_owned(),
            issuer_branch: 0,
            counterpart_branch: 0,
        };
        let xml = to_invoices_doc_xml(&doc, &ctx).unwrap();
        assert!(xml.contains("<series>A&amp;B</series>"));
    }

    #[test]
    fn invoices_doc_strips_el_prefix_to_bare_afm() {
        let xml = to_invoices_doc_xml(&sample_invoice(), &MyDataDocContext::default()).unwrap();
        assert!(xml.contains("<vatNumber>123456789</vatNumber>"));
        assert!(
            !xml.contains("EL123456789"),
            "the EL EU-VAT prefix must be stripped for myDATA vatNumber"
        );
    }

    #[test]
    fn invoices_doc_credit_note_maps_to_invoice_type_5_1() {
        let mut doc = sample_invoice();
        // Rebuild as a credit note to exercise the invoiceType mapping.
        let cn = CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-gr-cn-1").unwrap(),
            document_type: DocumentType::CreditNote,
            issue_date: DateOnly::new("2026-06-02").unwrap(),
            tax_point_date: None,
            due_date: None,
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("CN-2026-GR-0001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: greek_party("Acme Hellas AE", "EL123456789", "Athina"),
            customer: greek_party("Beta EPE", "EL987654321", "Thessaloniki"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: std::mem::take(&mut doc.lines),
            tax_summary: doc.tax_summary.clone(),
            monetary_total: doc.monetary_total.clone(),
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_gr".to_owned(),
                trace_id: "trace_gr".to_owned(),
                source_system: Some("unit".to_owned()),
            },
        })
        .unwrap();
        let xml = to_invoices_doc_xml(&cn, &MyDataDocContext::default()).unwrap();
        assert!(xml.contains("<invoiceType>5.1</invoiceType>"));
    }

    /// Build a credit note that points back at the original invoice via an IR
    /// preceding-invoice reference, and assert the referenced identifier reaches
    /// the wire verbatim as the myDATA `correlatedInvoices` element inside
    /// `invoiceHeader`. The value is emitted byte-for-byte (here a MARK-shaped
    /// id) with no derivation.
    fn credit_note_with_reference(kind: &str, reference_id: &str) -> CommercialDocument {
        let mut doc = sample_invoice();
        let parts = CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-gr-cn-ref-1").unwrap(),
            document_type: DocumentType::CreditNote,
            issue_date: DateOnly::new("2026-06-02").unwrap(),
            tax_point_date: None,
            due_date: None,
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("CN-2026-GR-0002").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: greek_party("Acme Hellas AE", "EL123456789", "Athina"),
            customer: greek_party("Beta EPE", "EL987654321", "Thessaloniki"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: std::mem::take(&mut doc.lines),
            tax_summary: doc.tax_summary.clone(),
            monetary_total: doc.monetary_total.clone(),
            attachments: Vec::new(),
            references: vec![DocumentReference {
                kind: kind.to_owned(),
                id: reference_id.to_owned(),
                issue_date: Some(DateOnly::new("2026-05-26").unwrap()),
            }],
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_gr".to_owned(),
                trace_id: "trace_gr".to_owned(),
                source_system: Some("unit".to_owned()),
            },
        };
        CommercialDocument::new(parts).unwrap()
    }

    #[test]
    fn invoices_doc_emits_correlated_invoices_verbatim_for_preceding_reference() {
        // A MARK-shaped referenced-invoice id: must reach the wire byte-for-byte.
        let mark = "400001234567890";
        let doc = credit_note_with_reference("invoice", mark);
        let xml = to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();
        assert!(
            xml.contains(&format!("<correlatedInvoices>{mark}</correlatedInvoices>")),
            "the preceding-invoice reference must be emitted verbatim as \
             correlatedInvoices, got:\n{xml}"
        );
        // Placement: inside invoiceHeader, after invoiceType, before the close.
        let header_start = xml.find("<invoiceHeader>").expect("invoiceHeader present");
        let header_end = xml.find("</invoiceHeader>").expect("invoiceHeader closed");
        let type_pos = xml.find("<invoiceType>").expect("invoiceType present");
        let corr_pos = xml.find("<correlatedInvoices>").expect("correlatedInvoices present");
        assert!(
            header_start < corr_pos && corr_pos < header_end,
            "correlatedInvoices must sit inside invoiceHeader"
        );
        assert!(
            type_pos < corr_pos,
            "correlatedInvoices must follow invoiceType in XSD sequence order"
        );
    }

    #[test]
    fn invoices_doc_correlated_invoices_value_is_not_derived() {
        // An arbitrary, non-MARK-shaped id is still emitted verbatim: the adapter
        // must not parse, validate, or rewrite the producer's reference id.
        let raw = "Original Invoice INV-2026-GR-0001 & Co";
        let doc = credit_note_with_reference("original-invoice", raw);
        let xml = to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();
        // The element carries the XML-escaped verbatim value (the escaper is the
        // only transform applied to text content).
        assert!(
            xml.contains(
                "<correlatedInvoices>Original Invoice INV-2026-GR-0001 &amp; Co</correlatedInvoices>"
            ),
            "correlatedInvoices must carry the producer id verbatim (XML-escaped), got:\n{xml}"
        );
    }

    #[test]
    fn invoices_doc_omits_correlated_invoices_when_no_preceding_reference() {
        // The default sample invoice has empty `references`: behaviour-preserving,
        // no correlatedInvoices element is emitted (every existing fixture).
        let xml = to_invoices_doc_xml(&sample_invoice(), &MyDataDocContext::default()).unwrap();
        assert!(
            !xml.contains("correlatedInvoices"),
            "no preceding-invoice reference => no correlatedInvoices element"
        );
    }

    #[test]
    fn invoices_doc_omits_correlated_invoices_for_non_preceding_reference_kind() {
        // A purchase-order reference classifies as `Order`, not `PrecedingInvoice`,
        // so it is NOT routed to correlatedInvoices (which is the original-invoice
        // link only). The element stays absent.
        let doc = credit_note_with_reference("purchase-order", "PO-7788");
        let xml = to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();
        assert!(
            !xml.contains("correlatedInvoices"),
            "a non-preceding reference kind must not emit correlatedInvoices"
        );
        assert!(
            !xml.contains("PO-7788"),
            "the order reference id must not leak into the InvoicesDoc"
        );
    }

    /// SKIP DOCUMENTATION (D18): `DocumentLine.classifications` carries EN 16931
    /// BT-158 commodity/HS-style codes. myDATA's per-line classification elements
    /// (`incomeClassification`/`expensesClassification`) take AADE income/expense
    /// catalog codes (`classificationType`, `classificationCategory`), an entirely
    /// different national catalog — mapping a BT-158 code onto one would be
    /// invention. There is no myDATA element that carries a BT-158 code verbatim,
    /// so a populated `classifications` must NOT change the InvoicesDoc output.
    #[test]
    fn invoices_doc_ignores_line_classifications() {
        let baseline = to_invoices_doc_xml(&sample_invoice(), &MyDataDocContext::default()).unwrap();
        let mut doc = sample_invoice();
        doc.lines[0].classifications = vec![invoicekit_ir::ItemClassification {
            code: "85176200".to_owned(),
            scheme_id: "HS".to_owned(),
            scheme_version: None,
        }];
        let with_classification =
            to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();
        assert_eq!(
            baseline, with_classification,
            "BT-158 classifications have no verbatim myDATA target; output must be unchanged"
        );
    }

    /// SKIP DOCUMENTATION (D18): myDATA encodes VAT exemption only as the CODED
    /// integer `vatExemptionCategory` (already emitted from the rate mapping).
    /// There is no myDATA element that carries the free-text BT-120
    /// `exemption_reason`, and the BT-121 `exemption_reason_code` (a VATEX/Natura
    /// code) is not the myDATA integer — mapping it would be invention. So a
    /// populated exemption reason / code must NOT change the InvoicesDoc output.
    #[test]
    fn invoices_doc_ignores_exemption_reason_text_and_code() {
        let mut doc = sample_invoice();
        doc.lines[0].tax_category = Some("E".to_owned());
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }];
        let baseline = to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();

        let mut with_reason = doc.clone();
        with_reason.tax_summary[0].exemption_reason =
            Some("Exempt under Article 22 of the Greek VAT Code".to_owned());
        with_reason.tax_summary[0].exemption_reason_code = Some("VATEX-EU-132-1F".to_owned());
        let with_reason_xml =
            to_invoices_doc_xml(&with_reason, &MyDataDocContext::default()).unwrap();

        assert_eq!(
            baseline, with_reason_xml,
            "free-text/coded exemption reason has no verbatim myDATA target; output unchanged"
        );
    }

    #[test]
    fn invoices_doc_exempt_line_emits_vat_category_7_with_exemption() {
        let mut doc = sample_invoice();
        doc.lines[0].tax_category = Some("E".to_owned());
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "E".to_owned(),
            taxable_amount: amt(10000),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(0, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }];
        let xml = to_invoices_doc_xml(&doc, &MyDataDocContext::default()).unwrap();
        assert!(xml.contains("<vatCategory>7</vatCategory>"));
        assert!(
            xml.contains("<vatExemptionCategory>1</vatExemptionCategory>"),
            "vatCategory 7 requires a vatExemptionCategory per the myDATA XSD:\n{xml}"
        );
        assert!(xml.contains("<vatAmount>0.00</vatAmount>"));
    }

    #[test]
    fn invoices_doc_rejects_unsupported_document_type() {
        let err = invoice_type_code(DocumentType::DebitNote).unwrap_err();
        assert!(matches!(err, MyDataXmlError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn invoices_doc_rejects_blank_series() {
        let ctx = MyDataDocContext {
            series: "  ".to_owned(),
            issuer_branch: 0,
            counterpart_branch: 0,
        };
        let err = to_invoices_doc_xml(&sample_invoice(), &ctx).unwrap_err();
        assert!(matches!(err, MyDataXmlError::BadContext(_)));
    }

    #[test]
    fn vat_category_codes_follow_the_mydata_codelist() {
        assert_eq!(vat_category_for_rate(Decimal::new(2400, 2)), 1); // 24%
        assert_eq!(vat_category_for_rate(Decimal::new(1300, 2)), 2); // 13%
        assert_eq!(vat_category_for_rate(Decimal::new(600, 2)), 3); // 6%
        assert_eq!(vat_category_for_rate(Decimal::new(0, 2)), 7); // exempt
    }

    fn sample_request(category: MyDataInvoiceCategory) -> MyDataReportRequest {
        MyDataReportRequest {
            tenant_id: "tenant-gr-test".to_owned(),
            environment: MyDataEnvironment::Sandbox,
            issuer_afm: "123456789".to_owned(),
            buyer_afm: Some("987654321".to_owned()),
            category,
            invoices_doc_xml: b"<InvoicesDoc/>".to_vec(),
        }
    }

    #[test]
    fn report_invoice_returns_accepted_with_mark_and_uid() {
        let p = MockMyDataProvider::default();
        let env = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        assert_eq!(env.status, MyDataStatus::Accepted);
        assert!(env.mark.is_some());
        assert!(env.uid.is_some());
        assert_eq!(env.reported_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn report_invoice_serial_increments_per_provider() {
        let p = MockMyDataProvider::default();
        let env1 = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        let env2 = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        assert_ne!(env1.mark.as_ref().unwrap().0, env2.mark.as_ref().unwrap().0);
    }

    #[test]
    fn report_invoice_rejects_empty_xml() {
        let p = MockMyDataProvider::default();
        let mut req = sample_request(MyDataInvoiceCategory::Services {
            code: "2.1".to_owned(),
        });
        req.invoices_doc_xml.clear();
        let err = p.report_invoice(&req).unwrap_err();
        assert!(matches!(err, MyDataError::BadXml(_)));
    }

    #[test]
    fn report_invoice_rejects_bad_afm() {
        let p = MockMyDataProvider::default();
        let mut req = sample_request(MyDataInvoiceCategory::CreditNote {
            code: "5.1".to_owned(),
        });
        req.issuer_afm = "12345".to_owned();
        let err = p.report_invoice(&req).unwrap_err();
        assert!(matches!(err, MyDataError::BadAfm(_)));
    }

    #[test]
    fn category_code_borrows_inner_string() {
        assert_eq!(
            MyDataInvoiceCategory::SalesGoods {
                code: "1.2".to_owned()
            }
            .code(),
            "1.2"
        );
        assert_eq!(
            MyDataInvoiceCategory::Other {
                code: "9.9".to_owned()
            }
            .code(),
            "9.9"
        );
    }

    #[test]
    fn validate_afm_accepts_9_digit_string() {
        assert!(validate_afm("123456789").is_ok());
    }

    #[test]
    fn validate_afm_rejects_wrong_length() {
        assert!(validate_afm("1234567890").is_err());
        assert!(validate_afm("12345678").is_err());
    }

    #[test]
    fn validate_afm_rejects_non_digits() {
        assert!(validate_afm("12345678A").is_err());
        assert!(validate_afm("123 56789").is_err());
    }

    #[test]
    fn qr_payload_renders_mark_and_uid_into_url() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::Accepted,
            mark: Some(MyDataMark::new("400000000000001")),
            uid: Some(MyDataUid::new("MYDATA-UID-1")),
            message: None,
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let qr = qr_payload("https://www.aade.gr/mydata", &envelope).unwrap();
        assert!(qr.contains("mark=400000000000001"));
        assert!(qr.contains("uid=MYDATA-UID-1"));
    }

    #[test]
    fn qr_payload_rejects_envelope_without_mark() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::Rejected,
            mark: None,
            uid: None,
            message: Some("schema validation failed".to_owned()),
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let err = qr_payload("https://www.aade.gr/mydata", &envelope).unwrap_err();
        assert!(matches!(err, MyDataError::BadXml(_)));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::AcceptedWithWarnings,
            mark: Some(MyDataMark::new("400000000000007")),
            uid: Some(MyDataUid::new("MYDATA-UID-7")),
            message: Some("buyer classification missing".to_owned()),
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: MyDataReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-gr-mydata");
    }

    /// Two lines each near `Decimal::MAX` overflow the `totalNetValue`
    /// accumulator. Before the `checked_add` fix this panicked (Decimal's
    /// `+=` panics on overflow); now it returns a typed error.
    #[test]
    fn invoices_doc_totals_overflow_returns_error_not_panic() {
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
                // No tax_category -> line_vat short-circuits to zero VAT, so
                // the overflow lands squarely on the net accumulator.
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
        let err = to_invoices_doc_xml(&doc, &MyDataDocContext::default())
            .expect_err("two near-MAX lines must overflow the net total");
        assert!(matches!(err, MyDataXmlError::TotalsUnrepresentable(_)));
    }

    /// The per-line pro-rate `tax_amount * line_net` overflows `Decimal::MAX`
    /// before the divide. Before the `checked_mul` fix this panicked; now it
    /// returns a typed error.
    #[test]
    fn invoices_doc_prorate_product_overflow_returns_error_not_panic() {
        let huge = DecimalValue::new(Decimal::MAX);
        let mut doc = sample_invoice();
        doc.lines = vec![DocumentLine {
            id: "1".to_owned(),
            description: "near-max line".to_owned(),
            quantity: DecimalValue::new(Decimal::ONE),
            unit_code: Some("EA".to_owned()),
            unit_price: huge.clone(),
            line_extension_amount: huge.clone(),
            tax_category: Some("S".to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        }];
        // band_base non-zero so the pro-rate branch runs; tax_amount * line_net
        // = MAX * MAX overflows well before the division by band_base.
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: DecimalValue::new(Decimal::from(2)),
            tax_amount: huge,
            tax_rate: Some(DecimalValue::new(Decimal::new(2400, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }];
        let err = to_invoices_doc_xml(&doc, &MyDataDocContext::default())
            .expect_err("MAX * MAX pro-rate product must overflow");
        assert!(matches!(err, MyDataXmlError::TotalsUnrepresentable(_)));
    }
}
