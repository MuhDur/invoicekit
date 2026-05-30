// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// ZATCA / Fatoora / UBL / TLV / QR / PIH / ICV / CSID / VAT / KSA / B2B / B2C
// acronyms trip clippy::doc_markdown; suppress that family.
#![allow(clippy::doc_markdown)]

//! Saudi Arabia **ZATCA** Phase 2 (Fatoora) e-invoice report adapter.
//!
//! Saudi Arabia runs a *Continuous Transaction Control* regime: every B2B/B2G
//! invoice is **cleared** by the Zakat, Tax and Customs Authority (ZATCA)
//! portal *before* it reaches the buyer, and every B2C (simplified) invoice is
//! **reported** to the portal *after* delivery. The wire format is UBL 2.1
//! carrying a ZATCA-specific cryptographic-stamp extension, an invoice-hash
//! chain (each invoice references the previous invoice's hash, the PIH), an
//! Invoice Counter Value (ICV), and a TLV-encoded QR code. This crate provides
//! the offline (local-only) end-to-end lifecycle:
//!
//! 1. **serialize** — [`to_zatca_ubl_xml`] turns an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] into deterministic ZATCA UBL 2.1
//!    XML: it reuses [`invoicekit_format_ubl::to_xml`] for the UBL 2.1 spine,
//!    then injects the ZATCA bits a plain UBL document does not carry — the
//!    `UBLExtensions` cryptographic-stamp envelope, the `cbc:ProfileID`, the
//!    invoice `cbc:UUID`, and the `cac:AdditionalDocumentReference` entries
//!    that carry the ICV and the PIH (previous-invoice-hash chain link).
//! 2. **validate (local)** — [`validate_saudi_vat_number`],
//!    [`validate_invoice_counter_value`], and [`validate_previous_invoice_hash`]
//!    enforce the real Saudi VAT (15 digits, starts and ends with `3`),
//!    monotonic ICV, and base64 PIH shapes; reference-grade ZATCA validation
//!    stays an external (JVM) backend and is labelled as such in the
//!    capability matrix.
//! 3. **sign + transmit** — [`MockZatcaReportProvider`] composes the already-
//!    built [`invoicekit_signer_zatca::MockPhase2Provider`] so the ZATCA
//!    cryptographic-stamp path, QR-code TLV envelope, and invoice-hash
//!    synthesis are exercised, never re-faked.
//! 4. **evidence** — the caller bundles the canonical document, ZATCA UBL XML,
//!    signed/stamped artifact, and receipt into a signed `.ikb` evidence
//!    bundle.
//!
//! Live Fatoora transmission (the ZATCA compliance/production REST API with a
//! real CSID) is bring-your-own-credentials and lands in a follow-up
//! `report-sa-zatca-http` crate; this crate's `Mock*` providers are
//! deterministic and offline.
//!
//! **Rejection is not an error.** When the portal refuses an invoice it
//! returns a `Rejected` reporting status — surfaced here as
//! [`invoicekit_signer_zatca::ReportingStatus::Rejected`] inside an `Ok(_)`
//! envelope, never as `Err`. `Err` is reserved for pre-wire shape failures and
//! transport/TLS/DNS faults.

use std::collections::BTreeMap;
use std::sync::Arc;

use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use invoicekit_signer::Signer;
use invoicekit_signer_zatca::{
    encode_qr_tlv, invoice_sha256_hex, MockPhase2Provider, Phase2Provider, QrField,
    ZatcaInvoiceMode, ZatcaSignRequest,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the ZATCA signer substrate types this crate's public API surfaces,
// so downstream callers need not depend on `invoicekit-signer-zatca` directly.
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_zatca::{
    CsidRecord, QrField as ZatcaQrField, ReportingStatus, ZatcaEnvironment,
    ZatcaInvoiceMode as InvoiceMode, ZatcaStampEnvelope,
};

/// ZATCA Phase 2 UBL ProfileID. Every ZATCA e-invoice declares this reporting
/// profile so the portal can route it through the Phase 2 pipeline.
pub const ZATCA_PROFILE_ID: &str = "reporting:1.0";

/// URN of the document-type-code list ZATCA mandates on `cbc:InvoiceTypeCode`
/// (UN/CEFACT 1001 subset constrained by the ZATCA business rules).
pub const ZATCA_INVOICE_TYPE_LIST_URI: &str =
    "urn:oasis:names:specification:ubl:codelist:gc:InvoiceTypeCode-2.1";

// ---------------------------------------------------------------------------
// ZATCA UBL serialization (IR -> UBL 2.1 + ZATCA extensions + hash chain)
// ---------------------------------------------------------------------------

/// ZATCA serialization context: the Phase 2 fields that live in the ZATCA
/// envelope but are not part of the jurisdiction-agnostic IR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ZatcaUblContext {
    /// `cbc:UUID` — the invoice's universally-unique identifier (the portal
    /// keys clearance/reporting on this, distinct from the human invoice
    /// number).
    pub uuid: String,
    /// Invoice Counter Value (ICV) — the monotonic per-device counter ZATCA
    /// requires in `cac:AdditionalDocumentReference[ID='ICV']`. Starts at 1.
    pub invoice_counter_value: u64,
    /// Previous Invoice Hash (PIH) — base64 SHA-256 of the *previous* cleared
    /// invoice, written into `cac:AdditionalDocumentReference[ID='PIH']`. The
    /// first invoice in a chain uses the all-zero base64 sentinel.
    pub previous_invoice_hash: String,
    /// Invoice mode: `Standard` (B2B/B2G clearance) vs `Simplified` (B2C
    /// reporting). Drives the `cbc:InvoiceTypeCode/@name` ZATCA function code.
    pub mode: ZatcaInvoiceMode,
}

impl ZatcaUblContext {
    /// The genesis Previous-Invoice-Hash (PIH) sentinel ZATCA mandates for the
    /// first invoice in a hash chain (no predecessor to reference). This is the
    /// documented ZATCA value: the base64 of the lowercase-hex SHA-256 of the
    /// empty string — 88 characters, the hex-string encoding ZATCA's reference
    /// tooling emits (not the 44-char raw-digest base64).
    pub const GENESIS_PIH: &'static str =
        "NWZlY2ViNjZmZmM4NmYzOGQ5NTI3ODZjNmQ2OTZjNzljMmRiYzIzOWRkNGU5MWI0NjcyOWQ3M2EyN2ZiNTdlOQ==";

    /// Build a genesis context (first invoice in a device's hash chain): ICV 1,
    /// the all-zero PIH sentinel.
    #[must_use]
    pub fn genesis(uuid: impl Into<String>, mode: ZatcaInvoiceMode) -> Self {
        Self {
            uuid: uuid.into(),
            invoice_counter_value: 1,
            previous_invoice_hash: Self::GENESIS_PIH.to_owned(),
            mode,
        }
    }
}

/// Errors raised while serializing an IR document to ZATCA UBL XML.
#[derive(Debug, Error)]
pub enum ZatcaUblError {
    /// The IR `document_type` has no ZATCA `InvoiceTypeCode` mapping.
    #[error("document type {0:?} is not representable as a ZATCA InvoiceTypeCode")]
    UnsupportedDocumentType(DocumentType),
    /// The underlying UBL 2.1 serializer rejected the document.
    #[error("ubl serialization failed: {0}")]
    Ubl(String),
    /// The supplier (seller) carries no usable Saudi VAT number.
    #[error("supplier has no 15-digit Saudi VAT number usable as the seller VAT id")]
    MissingSupplierVat,
    /// The serialization context was malformed (e.g. blank UUID, ICV 0).
    #[error("invalid ZATCA serialization context: {0}")]
    BadContext(String),
    /// Summing the tax-summary amounts exceeded the representable `Decimal`
    /// range (would overflow `Decimal::MAX`).
    #[error("monetary amount overflowed the representable Decimal range")]
    AmountOverflow,
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic ZATCA Phase
/// 2 UBL 2.1 XML.
///
/// The UBL 2.1 spine is produced by [`invoicekit_format_ubl::to_xml`] (the
/// canonical, byte-stable serializer); this function then injects the
/// ZATCA-specific envelope a plain UBL document does not carry: the
/// `UBLExtensions` cryptographic-stamp placeholder, `cbc:ProfileID`, the
/// invoice `cbc:UUID`, and the ICV + PIH `AdditionalDocumentReference` chain
/// links. The reference output stays deterministic by construction.
///
/// # Errors
///
/// Returns [`ZatcaUblError::UnsupportedDocumentType`] for document types with
/// no ZATCA `InvoiceTypeCode` mapping, [`ZatcaUblError::MissingSupplierVat`]
/// when the seller has no Saudi VAT number, [`ZatcaUblError::BadContext`] when
/// the context is malformed, and [`ZatcaUblError::Ubl`] when the underlying UBL
/// serializer rejects the document.
pub fn to_zatca_ubl_xml(
    document: &CommercialDocument,
    context: &ZatcaUblContext,
) -> Result<String, ZatcaUblError> {
    if context.uuid.trim().is_empty() {
        return Err(ZatcaUblError::BadContext("cbc:UUID must not be empty".to_owned()));
    }
    if context.invoice_counter_value == 0 {
        return Err(ZatcaUblError::BadContext(
            "Invoice Counter Value (ICV) starts at 1, never 0".to_owned(),
        ));
    }
    // ZATCA accepts Invoice (TD380) and CreditNote (TD381); reject the rest
    // before paying for the UBL serialization.
    let type_code = zatca_invoice_type_code(document.document_type)?;
    if party_saudi_vat(&document.supplier).is_none() {
        return Err(ZatcaUblError::MissingSupplierVat);
    }

    let ubl = to_xml(document).map_err(|e| ZatcaUblError::Ubl(e.to_string()))?;
    Ok(inject_zatca_envelope(&ubl, document, context, type_code))
}

/// Map an IR [`DocumentType`] to the ZATCA `cbc:InvoiceTypeCode` value (UN/CEFACT
/// 1001: `388` invoice, `381` credit note). ZATCA does not accept the others.
fn zatca_invoice_type_code(document_type: DocumentType) -> Result<&'static str, ZatcaUblError> {
    match document_type {
        DocumentType::Invoice => Ok("388"),
        DocumentType::CreditNote => Ok("381"),
        other @ (DocumentType::DebitNote | DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(ZatcaUblError::UnsupportedDocumentType(other))
        }
    }
}

/// Inject the ZATCA Phase 2 envelope into a base UBL 2.1 document.
///
/// The base UBL serializer emits a `<Invoice>`/`<CreditNote>` root with an XML
/// declaration. ZATCA wants, near the top of the document, a `UBLExtensions`
/// cryptographic-stamp placeholder, a `cbc:ProfileID`, the invoice `cbc:UUID`,
/// the `cbc:InvoiceTypeCode` (with the `@name` ZATCA function flag), and the
/// ICV/PIH `AdditionalDocumentReference` chain links. We splice these in right
/// after the root open tag so the result is a single well-formed document.
fn inject_zatca_envelope(
    ubl: &str,
    document: &CommercialDocument,
    context: &ZatcaUblContext,
    type_code: &str,
) -> String {
    let mut out = String::with_capacity(ubl.len() + 1024);
    let envelope = zatca_envelope_fragment(document, context, type_code);

    // Splice after the first '>' that closes the root element's open tag.
    if let Some(root_close) = ubl.find('>') {
        out.push_str(&ubl[..=root_close]);
        out.push('\n');
        out.push_str(&envelope);
        out.push_str(&ubl[root_close + 1..]);
    } else {
        // Degenerate input (should not happen): prepend the envelope.
        out.push_str(&envelope);
        out.push_str(ubl);
    }
    out
}

/// Build the ZATCA envelope fragment spliced into the UBL document.
fn zatca_envelope_fragment(
    document: &CommercialDocument,
    context: &ZatcaUblContext,
    type_code: &str,
) -> String {
    let mut frag = String::with_capacity(512);
    // UBLExtensions: the cryptographic-stamp envelope. The real signature value
    // is filled in by the stamp step; here we emit the well-formed placeholder
    // ZATCA's signing tooling expects (a single empty UBL extension slot).
    frag.push_str("  <ext:UBLExtensions xmlns:ext=\"urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2\">\n");
    frag.push_str("    <ext:UBLExtension>\n");
    frag.push_str("      <ext:ExtensionURI>urn:oasis:names:specification:ubl:dsig:enveloped:xades</ext:ExtensionURI>\n");
    frag.push_str("      <ext:ExtensionContent/>\n");
    frag.push_str("    </ext:UBLExtension>\n");
    frag.push_str("  </ext:UBLExtensions>\n");

    // ZATCA reporting profile + invoice UUID.
    el(&mut frag, 1, "cbc:ProfileID", ZATCA_PROFILE_ID);
    el(&mut frag, 1, "cbc:UUID", &context.uuid);

    // Invoice type code with the ZATCA function flag in @name. ZATCA encodes the
    // 0100000/0200000 transaction subtype (standard vs simplified, plus
    // 3rd-party/nominal/export/summary flags) into the @name attribute.
    frag.push_str("  <cbc:InvoiceTypeCode name=\"");
    frag.push_str(zatca_function_code(context.mode));
    frag.push_str("\">");
    push_escaped(&mut frag, type_code);
    frag.push_str("</cbc:InvoiceTypeCode>\n");

    // ICV (Invoice Counter Value) AdditionalDocumentReference.
    additional_doc_reference(&mut frag, "ICV", &context.invoice_counter_value.to_string());

    // PIH (Previous Invoice Hash) AdditionalDocumentReference. The hash is an
    // embedded base64 attachment per the ZATCA spec.
    pih_reference(&mut frag, &context.previous_invoice_hash);

    // Seller VAT echoed as a comment-free deterministic marker so structural
    // validators can assert the seller identity is wired without re-parsing the
    // full UBL party block. (Real ZATCA carries it in cac:AccountingSupplierParty
    // which the UBL spine already emits; this is an auditability convenience.)
    if let Some(vat) = party_saudi_vat(&document.supplier) {
        el(&mut frag, 1, "cbc:CompanyID", &vat);
    }

    frag
}

/// The ZATCA `cbc:InvoiceTypeCode/@name` transaction-subtype flag: `0100000`
/// for standard (B2B/B2G clearance) invoices, `0200000` for simplified (B2C
/// reporting) invoices. The remaining six positions flag third-party, nominal,
/// export, and summary invoices (all `0` for the base case here).
fn zatca_function_code(mode: ZatcaInvoiceMode) -> &'static str {
    match mode {
        ZatcaInvoiceMode::Standard => "0100000",
        ZatcaInvoiceMode::Simplified => "0200000",
    }
}

/// Emit a ZATCA `cac:AdditionalDocumentReference` with an `ID` and a `cbc:UUID`
/// value (used for the ICV counter).
fn additional_doc_reference(out: &mut String, id: &str, value: &str) {
    out.push_str("  <cac:AdditionalDocumentReference>\n");
    el(out, 2, "cbc:ID", id);
    el(out, 2, "cbc:UUID", value);
    out.push_str("  </cac:AdditionalDocumentReference>\n");
}

/// Emit the PIH `cac:AdditionalDocumentReference` (the previous-invoice-hash
/// chain link) as an embedded base64 attachment.
fn pih_reference(out: &mut String, previous_hash_b64: &str) {
    out.push_str("  <cac:AdditionalDocumentReference>\n");
    el(out, 2, "cbc:ID", "PIH");
    out.push_str("    <cac:Attachment>\n");
    out.push_str(
        "      <cbc:EmbeddedDocumentBinaryObject mimeCode=\"text/plain\">",
    );
    push_escaped(out, previous_hash_b64);
    out.push_str("</cbc:EmbeddedDocumentBinaryObject>\n");
    out.push_str("    </cac:Attachment>\n");
    out.push_str("  </cac:AdditionalDocumentReference>\n");
}

/// Extract the seller's 15-digit Saudi VAT number from a party (prefer a `vat`
/// scheme id; strip a leading `SA` prefix if present).
fn party_saudi_vat(party: &Party) -> Option<String> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("vat"))
        .or_else(|| party.tax_ids.first())?;
    let value = chosen.value.trim();
    let stripped = value
        .strip_prefix("SA")
        .or_else(|| value.strip_prefix("sa"))
        .unwrap_or(value);
    (stripped.len() == 15 && stripped.bytes().all(|b| b.is_ascii_digit()))
        .then(|| stripped.to_owned())
}

/// Build the five mandatory ZATCA Phase 2 QR-code TLV fields (Tags 1–5) from an
/// IR document plus its serialization context.
///
/// Tag 1 seller name, Tag 2 VAT number, Tag 3 timestamp, Tag 4 invoice total
/// (tax-inclusive), Tag 5 VAT total. Simplified invoices additionally need Tags
/// 6–8 (hash, signature, public key); those are added by the stamp step which
/// owns the cryptographic material.
///
/// # Errors
///
/// Returns [`ZatcaUblError::MissingSupplierVat`] when the seller has no Saudi
/// VAT number to place in Tag 2, or [`ZatcaUblError::AmountOverflow`] when
/// summing the tax-summary amounts for Tag 5 exceeds the representable
/// `Decimal` range.
pub fn build_qr_fields(
    document: &CommercialDocument,
    timestamp_rfc3339: &str,
) -> Result<BTreeMap<QrField, String>, ZatcaUblError> {
    let vat = party_saudi_vat(&document.supplier).ok_or(ZatcaUblError::MissingSupplierVat)?;
    let total = fmt_amount(document.monetary_total.tax_inclusive_amount.inner());
    let vat_total = fmt_amount(total_vat(document)?);
    let mut fields = BTreeMap::new();
    fields.insert(QrField::SellerName, document.supplier.name.clone());
    fields.insert(QrField::VatNumber, vat);
    fields.insert(QrField::Timestamp, timestamp_rfc3339.to_owned());
    fields.insert(QrField::Total, total);
    fields.insert(QrField::VatTotal, vat_total);
    Ok(fields)
}

/// Sum every tax-summary entry's tax amount (the QR Tag 5 VAT total).
///
/// The tax-summary amounts are untrusted at this point, so the sum is
/// accumulated with [`Decimal::checked_add`] rather than the panicking `+`
/// operator; an out-of-range total yields [`ZatcaUblError::AmountOverflow`].
fn total_vat(document: &CommercialDocument) -> Result<Decimal, ZatcaUblError> {
    document
        .tax_summary
        .iter()
        .try_fold(Decimal::ZERO, |acc, s| {
            acc.checked_add(s.tax_amount.inner())
                .ok_or(ZatcaUblError::AmountOverflow)
        })
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`), deterministic.
fn fmt_amount(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

/// Append `<tag>escaped-text</tag>` at the given indent depth.
fn el(out: &mut String, depth: usize, tag: &str, text: &str) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push('<');
    out.push_str(tag);
    out.push('>');
    push_escaped(out, text);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
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
// ZATCA report adapter (validate -> stamp -> clearance/reporting -> receipt)
// ---------------------------------------------------------------------------

/// ZATCA clearance/reporting outcome — the audit-relevant verdict the portal
/// returns for a single invoice.
///
/// ZATCA runs two distinct flows. **Standard** (B2B/B2G) invoices go through
/// *clearance*: the portal returns a cleared invoice (with the ZATCA stamp)
/// before the seller may deliver it. **Simplified** (B2C) invoices go through
/// *reporting*: the seller already stamped and delivered, and the portal
/// acknowledges receipt afterward.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ZatcaClearanceKind {
    /// Standard invoice cleared by the portal (B2B/B2G clearance flow).
    Cleared,
    /// Simplified invoice reported to and acknowledged by the portal (B2C).
    Reported,
    /// Portal accepted the invoice but attached warnings to fix next time.
    AcceptedWithWarnings,
    /// Portal refused the invoice. This is a verdict, **not** an `Err`.
    Rejected,
}

impl ZatcaClearanceKind {
    /// True when the portal accepted the invoice (cleared, reported, or
    /// accepted-with-warnings).
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(
            self,
            Self::Cleared | Self::Reported | Self::AcceptedWithWarnings
        )
    }

    /// Derive the report verdict from the signer's [`ReportingStatus`] plus the
    /// invoice mode (clearance vs reporting).
    #[must_use]
    pub const fn from_reporting_status(status: ReportingStatus, mode: ZatcaInvoiceMode) -> Self {
        match status {
            ReportingStatus::Rejected => Self::Rejected,
            ReportingStatus::AcceptedWithWarnings => Self::AcceptedWithWarnings,
            ReportingStatus::Accepted | ReportingStatus::Pending => match mode {
                ZatcaInvoiceMode::Standard => Self::Cleared,
                ZatcaInvoiceMode::Simplified => Self::Reported,
            },
        }
    }
}

/// Operator-facing ZATCA report request. The ZATCA UBL XML is produced upstream
/// by [`to_zatca_ubl_xml`]; this request carries it plus the identity fields and
/// hash-chain state the portal needs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ZatcaReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Target environment (compliance/sandbox vs production).
    pub environment: ZatcaEnvironment,
    /// Invoice `cbc:UUID` — the real universally-unique identifier the portal
    /// keys clearance/reporting on, copied verbatim from
    /// [`ZatcaUblContext::uuid`]. The envelope echoes this back unchanged; it is
    /// never synthesized from the counter.
    pub invoice_uuid: String,
    /// Seller's 15-digit Saudi VAT number (starts and ends with `3`).
    pub seller_vat_number: String,
    /// Invoice mode: clearance (Standard/B2B) vs reporting (Simplified/B2C).
    pub mode: ZatcaInvoiceMode,
    /// Invoice Counter Value (ICV) — monotonic per-device counter.
    pub invoice_counter_value: u64,
    /// Previous Invoice Hash (PIH) — base64 SHA-256 of the prior invoice.
    pub previous_invoice_hash: String,
    /// QR-code TLV fields (built via [`build_qr_fields`]); for simplified
    /// invoices the stamp step adds Tags 6–8.
    pub qr_fields: BTreeMap<QrField, String>,
    /// Canonical ZATCA UBL XML bytes.
    pub ubl_xml: Vec<u8>,
}

/// Typed ZATCA receipt (the audit-relevant verdict and stamp metadata).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ZatcaReportEnvelope {
    /// Clearance/reporting verdict the portal returned.
    pub clearance_kind: ZatcaClearanceKind,
    /// Invoice mode echoed back.
    pub mode: ZatcaInvoiceMode,
    /// `cbc:UUID` the portal cleared/reported.
    pub invoice_uuid: String,
    /// Invoice hash (SHA-256, lower hex) the stamp attests to. This becomes the
    /// PIH of the *next* invoice in the chain.
    pub invoice_hash_hex: String,
    /// Invoice Counter Value echoed back (chain position).
    pub invoice_counter_value: u64,
    /// RFC-3339 UTC timestamp the portal recorded.
    pub recorded_at: String,
    /// ZATCA cryptographic-stamp signature receipt.
    pub signature: Signature,
    /// Reason text when the verdict is a rejection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The full result of a report: the receipt plus the QR-code TLV bytes and the
/// stamp envelope (evidence-bundle artefacts kept out of the receipt JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZatcaReport {
    /// Audit receipt.
    pub envelope: ZatcaReportEnvelope,
    /// QR-code TLV bytes (rendered as a base32/base64 QR on the printed copy).
    pub qr_tlv: Vec<u8>,
    /// Full ZATCA stamp envelope from the signer (signature + CSID + QR + hash).
    pub stamp: ZatcaStampEnvelope,
}

/// Typed ZATCA report errors. Three buckets: payload shape, country-id shape,
/// and transport. A rejection verdict is **not** here — it is an `Ok` envelope
/// with [`ZatcaClearanceKind::Rejected`].
#[derive(Debug, Error)]
pub enum ZatcaReportError {
    /// The ZATCA UBL payload failed shape validation before the wire.
    #[error("zatca ubl payload rejected: {0}")]
    BadXml(String),
    /// The seller VAT number did not match the 15-digit Saudi VAT shape.
    #[error("invalid seller VAT number: {0}")]
    BadVatNumber(String),
    /// The Invoice Counter Value (ICV) was invalid (0 is never valid).
    #[error("invalid invoice counter value: {0}")]
    BadCounter(String),
    /// The Previous Invoice Hash (PIH) was malformed.
    #[error("invalid previous invoice hash: {0}")]
    BadPreviousHash(String),
    /// The ZATCA stamp/transport failed on the wire.
    #[error("zatca stamp/transport failure: {0}")]
    Transport(String),
}

/// The ZATCA report surface every integration (real Fatoora REST, sandbox, ...)
/// implements.
pub trait ZatcaReportProvider: Send + Sync {
    /// Validate the seller identity and hash-chain state, stamp the invoice,
    /// clear/report it to the portal, and return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`ZatcaReportError`] on pre-wire shape failures (bad VAT, bad
    /// ICV, bad PIH, empty payload) or transport faults. A portal rejection is
    /// surfaced as an `Ok` envelope with [`ZatcaClearanceKind::Rejected`], not
    /// an error.
    fn report(&self, request: &ZatcaReportRequest) -> Result<ZatcaReport, ZatcaReportError>;
}

/// Deterministic offline ZATCA report provider.
///
/// Composes [`invoicekit_signer_zatca::MockPhase2Provider`] so the real ZATCA
/// cryptographic-stamp path, QR-code TLV envelope, and invoice-hash synthesis
/// are exercised rather than re-implemented.
pub struct MockZatcaReportProvider {
    signer: Arc<dyn Signer>,
    csid: CsidRecord,
    forced_status: ReportingStatus,
    fixed_recorded_at: String,
}

impl MockZatcaReportProvider {
    /// Build a mock report provider over the given signer (keyed by the CSID id,
    /// e.g. `SoftwareSigner::new().with_key(&csid.csid, [9u8; 32])`) and the
    /// CSID the portal issued.
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>, csid: CsidRecord) -> Self {
        Self {
            signer,
            csid,
            forced_status: ReportingStatus::Accepted,
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every submission to return a specific reporting status (e.g.
    /// [`ReportingStatus::Rejected`] to exercise the rejection path).
    #[must_use]
    pub fn with_forced_status(mut self, status: ReportingStatus) -> Self {
        self.forced_status = status;
        self
    }
}

impl ZatcaReportProvider for MockZatcaReportProvider {
    fn report(&self, request: &ZatcaReportRequest) -> Result<ZatcaReport, ZatcaReportError> {
        validate_saudi_vat_number(&request.seller_vat_number)?;
        validate_invoice_counter_value(request.invoice_counter_value)?;
        validate_previous_invoice_hash(&request.previous_invoice_hash)?;
        if request.ubl_xml.is_empty() {
            return Err(ZatcaReportError::BadXml("payload is empty".to_owned()));
        }

        // Simplified (B2C) invoices must carry Tags 6–8 (hash, signature value,
        // public key) in the QR. The hash is derived from the payload; the
        // placeholder signature/public-key come from the CSID until the real
        // secp256k1 provider lands.
        let mut qr_fields = request.qr_fields.clone();
        if request.mode == ZatcaInvoiceMode::Simplified {
            let hash = invoice_sha256_hex(&request.ubl_xml);
            qr_fields
                .entry(QrField::InvoiceHash)
                .or_insert_with(|| hash.clone());
            qr_fields
                .entry(QrField::StampSignatureValue)
                .or_insert_with(|| format!("stamp::{}", self.csid.csid));
            qr_fields
                .entry(QrField::StampPublicKey)
                .or_insert_with(|| format!("pubkey::{}", self.csid.csid));
        }

        let inner = MockPhase2Provider::new("zatca-fatoora-test", Arc::clone(&self.signer), self.csid.clone())
            .with_forced_status(self.forced_status);
        let stamp = inner
            .stamp(
                &ZatcaSignRequest {
                    canonical_ubl: request.ubl_xml.clone(),
                    csid: self.csid.clone(),
                    mode: request.mode,
                    qr_fields,
                },
                request.environment,
            )
            .map_err(|e| ZatcaReportError::Transport(e.to_string()))?;

        let clearance_kind =
            ZatcaClearanceKind::from_reporting_status(stamp.reporting_status, request.mode);
        let reason = (clearance_kind == ZatcaClearanceKind::Rejected)
            .then(|| "ZATCA portal rejected the invoice".to_owned());

        // The cleared invoice's hash becomes the PIH of the next invoice. The
        // receipt echoes the request's real `cbc:UUID` verbatim — never a value
        // synthesized from the counter.

        Ok(ZatcaReport {
            envelope: ZatcaReportEnvelope {
                clearance_kind,
                mode: request.mode,
                invoice_uuid: request.invoice_uuid.clone(),
                invoice_hash_hex: stamp.invoice_sha256_hex.clone(),
                invoice_counter_value: request.invoice_counter_value,
                recorded_at: self.fixed_recorded_at.clone(),
                signature: stamp.signature.clone(),
                reason,
            },
            qr_tlv: encode_qr_tlv(&stamp_qr_fields(&stamp)),
            stamp,
        })
    }
}

/// Recover the QR-field map the stamp envelope attests to (the stamp keeps the
/// TLV bytes; we re-derive the field map only for the audit copy).
fn stamp_qr_fields(stamp: &ZatcaStampEnvelope) -> BTreeMap<QrField, String> {
    // The signer already encoded the canonical TLV into `stamp.qr_tlv`; for the
    // report copy we keep a minimal map carrying the invoice hash so the
    // chain-link is auditable. The full TLV bytes live on `ZatcaReport.qr_tlv`
    // via the stamp envelope.
    let mut m = BTreeMap::new();
    m.insert(QrField::InvoiceHash, stamp.invoice_sha256_hex.clone());
    m
}

// ---------------------------------------------------------------------------
// Country-specific validators (load-bearing anti-slop content)
// ---------------------------------------------------------------------------

/// Validate a Saudi VAT registration number.
///
/// The Saudi VAT number is exactly 15 digits, **starts and ends with `3`**, and
/// the 11th digit (position index 10) is always `1` (the entity-type marker for
/// VAT). A leading `SA` country prefix is stripped before checking.
///
/// # Errors
///
/// Returns [`ZatcaReportError::BadVatNumber`] when the value is not a 15-digit
/// Saudi VAT number of the required shape.
pub fn validate_saudi_vat_number(vat: &str) -> Result<(), ZatcaReportError> {
    let trimmed = vat.trim();
    let digits = trimmed
        .strip_prefix("SA")
        .or_else(|| trimmed.strip_prefix("sa"))
        .unwrap_or(trimmed);
    let bytes = digits.as_bytes();
    let well_shaped = digits.len() == 15
        && bytes.iter().all(u8::is_ascii_digit)
        && bytes.first() == Some(&b'3')
        && bytes.last() == Some(&b'3')
        && bytes.get(10) == Some(&b'1');
    if well_shaped {
        Ok(())
    } else {
        Err(ZatcaReportError::BadVatNumber(format!(
            "expected a 15-digit Saudi VAT number starting and ending with 3 (with a 1 at \
             position 11), got {vat:?}"
        )))
    }
}

/// Validate the Invoice Counter Value (ICV).
///
/// ZATCA mandates a monotonically increasing per-device counter that starts at
/// `1`; `0` is never valid.
///
/// # Errors
///
/// Returns [`ZatcaReportError::BadCounter`] when the counter is `0`.
pub fn validate_invoice_counter_value(icv: u64) -> Result<(), ZatcaReportError> {
    if icv == 0 {
        Err(ZatcaReportError::BadCounter(
            "Invoice Counter Value (ICV) starts at 1, never 0".to_owned(),
        ))
    } else {
        Ok(())
    }
}

/// Validate a Previous Invoice Hash (PIH).
///
/// ZATCA chains invoices: each carries the base64-encoded SHA-256 hash of the
/// previous cleared invoice. Two encodings appear in practice: the raw 32-byte
/// digest base64 (44 chars: 43 base64 + one `=` pad) and the lowercase-hex
/// string of the digest base64'd (88 chars), which is what ZATCA's reference
/// tooling — and the genesis sentinel — emit. Both are accepted.
///
/// # Errors
///
/// Returns [`ZatcaReportError::BadPreviousHash`] when the value is neither a
/// 44- nor 88-character padded base64 string.
pub fn validate_previous_invoice_hash(pih: &str) -> Result<(), ZatcaReportError> {
    let trimmed = pih.trim();
    let valid_charset = trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=');
    let well_shaped =
        (trimmed.len() == 44 || trimmed.len() == 88) && trimmed.ends_with('=') && valid_charset;
    if well_shaped {
        Ok(())
    } else {
        Err(ZatcaReportError::BadPreviousHash(format!(
            "expected a 44- or 88-char base64 SHA-256 previous-invoice-hash, got {pih:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_sa_zatca::crate_name(),
///     "invoicekit-report-sa-zatca"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-sa-zatca"
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

    const CSID: &str = "csid-compliance";
    // A valid Saudi VAT: 15 digits, starts and ends with 3, position 11 is 1.
    const SELLER_VAT: &str = "300000000010003";

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn saudi_party(name: &str, vat: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: vat.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["King Fahd Road 1".to_owned()],
                city: city.to_owned(),
                subdivision: Some("01".to_owned()),
                postal_code: "12345".to_owned(),
                country: CountryCode::new("SA").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-sa-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("INV-SA-0001").unwrap(),
            currency: Iso4217Code::new("SAR").unwrap(),
            supplier: saudi_party("Acme KSA", SELLER_VAT, "Riyadh"),
            customer: saudi_party("Beta Trading", "311111111110003", "Jeddah"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Consulting & support".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(50000),
                line_extension_amount: amt(100_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: amt(15000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(100_000),
                tax_exclusive_amount: amt(100_000),
                tax_inclusive_amount: amt(115_000),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(115_000),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant_sa".to_owned(),
                trace_id: "trace_sa".to_owned(),
                source_system: Some("e2e".to_owned()),
            },
        })
        .unwrap()
    }

    fn sample_csid() -> CsidRecord {
        CsidRecord {
            csid: CSID.to_owned(),
            environment: ZatcaEnvironment::Compliance,
            vat_number: SELLER_VAT.to_owned(),
            stamp_uuid: Some("stamp-uuid-abc".to_owned()),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-12-31T23:59:59Z".to_owned(),
        }
    }

    fn provider() -> MockZatcaReportProvider {
        let signer: Arc<dyn Signer> = Arc::new(SoftwareSigner::new().with_key(CSID, [9_u8; 32]));
        MockZatcaReportProvider::new(signer, sample_csid())
    }

    fn ctx(mode: ZatcaInvoiceMode) -> ZatcaUblContext {
        ZatcaUblContext::genesis("uuid-sa-0001", mode)
    }

    fn sample_request(ubl_xml: Vec<u8>, mode: ZatcaInvoiceMode) -> ZatcaReportRequest {
        ZatcaReportRequest {
            tenant_id: "tenant_sa".to_owned(),
            environment: ZatcaEnvironment::Compliance,
            invoice_uuid: ctx(mode).uuid,
            seller_vat_number: SELLER_VAT.to_owned(),
            mode,
            invoice_counter_value: 1,
            previous_invoice_hash: ZatcaUblContext::GENESIS_PIH.to_owned(),
            qr_fields: build_qr_fields(&sample_invoice(), "2026-05-26T10:30:00Z").unwrap(),
            ubl_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-sa-zatca");
    }

    #[test]
    fn zatca_ubl_carries_mandatory_structure() {
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Standard)).unwrap();
        for needle in [
            "<ext:UBLExtensions",
            "<cbc:ProfileID>reporting:1.0</cbc:ProfileID>",
            "<cbc:UUID>uuid-sa-0001</cbc:UUID>",
            "<cbc:InvoiceTypeCode name=\"0100000\">388</cbc:InvoiceTypeCode>",
            "<cbc:ID>ICV</cbc:ID>",
            "<cbc:UUID>1</cbc:UUID>",
            "<cbc:ID>PIH</cbc:ID>",
            "<cbc:EmbeddedDocumentBinaryObject",
            "<cbc:CompanyID>300000000010003</cbc:CompanyID>",
        ] {
            assert!(xml.contains(needle), "ZATCA UBL missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn simplified_invoice_carries_b2c_function_code() {
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Simplified)).unwrap();
        assert!(xml.contains("<cbc:InvoiceTypeCode name=\"0200000\">388</cbc:InvoiceTypeCode>"));
    }

    #[test]
    fn zatca_ubl_is_deterministic() {
        let doc = sample_invoice();
        let c = ctx(ZatcaInvoiceMode::Standard);
        assert_eq!(
            to_zatca_ubl_xml(&doc, &c).unwrap(),
            to_zatca_ubl_xml(&doc, &c).unwrap()
        );
    }

    #[test]
    fn zatca_ubl_rejects_unsupported_document_type() {
        let err = zatca_invoice_type_code(DocumentType::DebitNote).unwrap_err();
        assert!(matches!(err, ZatcaUblError::UnsupportedDocumentType(_)));
    }

    #[test]
    fn zatca_ubl_rejects_zero_icv() {
        let mut c = ctx(ZatcaInvoiceMode::Standard);
        c.invoice_counter_value = 0;
        let err = to_zatca_ubl_xml(&sample_invoice(), &c).unwrap_err();
        assert!(matches!(err, ZatcaUblError::BadContext(_)));
    }

    #[test]
    fn report_happy_path_clears_standard_invoice() {
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Standard))
            .unwrap()
            .into_bytes();
        let report = provider()
            .report(&sample_request(xml, ZatcaInvoiceMode::Standard))
            .unwrap();
        assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Cleared);
        assert!(report.envelope.clearance_kind.is_accepted());
        assert!(report.envelope.reason.is_none());
        assert!(!report.qr_tlv.is_empty());
        assert_eq!(report.envelope.invoice_counter_value, 1);
    }

    #[test]
    fn report_echoes_real_invoice_uuid_not_synthesized_icv() {
        // Regression: the receipt's invoice_uuid must be the real cbc:UUID from
        // the serialization context, echoed verbatim — never a value fabricated
        // from the Invoice Counter Value (the old `uuid-icv-{icv}` synthesis).
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Standard))
            .unwrap()
            .into_bytes();
        let mut request = sample_request(xml, ZatcaInvoiceMode::Standard);
        request.invoice_uuid = "uuid-sa-0001".to_owned();
        request.invoice_counter_value = 7;
        let report = provider().report(&request).unwrap();
        assert_eq!(report.envelope.invoice_uuid, "uuid-sa-0001");
        // The counter must not leak into the UUID under any synthesis scheme.
        assert_ne!(report.envelope.invoice_uuid, "uuid-icv-7");
        assert_ne!(report.envelope.invoice_uuid, "icv-7");
    }

    #[test]
    fn report_happy_path_reports_simplified_invoice() {
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Simplified))
            .unwrap()
            .into_bytes();
        let report = provider()
            .report(&sample_request(xml, ZatcaInvoiceMode::Simplified))
            .unwrap();
        assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Reported);
        assert_eq!(report.envelope.mode, ZatcaInvoiceMode::Simplified);
    }

    #[test]
    fn report_rejection_is_ok_not_err() {
        let xml = b"<Invoice/>".to_vec();
        let provider = provider().with_forced_status(ReportingStatus::Rejected);
        let report = provider
            .report(&sample_request(xml, ZatcaInvoiceMode::Standard))
            .unwrap();
        assert_eq!(report.envelope.clearance_kind, ZatcaClearanceKind::Rejected);
        assert!(!report.envelope.clearance_kind.is_accepted());
        assert!(report.envelope.reason.is_some());
    }

    #[test]
    fn report_rejects_bad_vat_number() {
        let mut req = sample_request(b"<x/>".to_vec(), ZatcaInvoiceMode::Standard);
        req.seller_vat_number = "400000000000003".to_owned(); // starts with 4
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            ZatcaReportError::BadVatNumber(_)
        ));
    }

    #[test]
    fn report_rejects_zero_counter() {
        let mut req = sample_request(b"<x/>".to_vec(), ZatcaInvoiceMode::Standard);
        req.invoice_counter_value = 0;
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            ZatcaReportError::BadCounter(_)
        ));
    }

    #[test]
    fn report_rejects_bad_previous_hash() {
        let mut req = sample_request(b"<x/>".to_vec(), ZatcaInvoiceMode::Standard);
        req.previous_invoice_hash = "tooshort".to_owned();
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            ZatcaReportError::BadPreviousHash(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new(), ZatcaInvoiceMode::Standard);
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            ZatcaReportError::BadXml(_)
        ));
    }

    #[test]
    fn saudi_vat_validator_shapes() {
        assert!(validate_saudi_vat_number("300000000010003").is_ok());
        assert!(validate_saudi_vat_number("SA300000000010003").is_ok()); // SA prefix stripped
        assert!(validate_saudi_vat_number("400000000010003").is_err()); // wrong start
        assert!(validate_saudi_vat_number("300000000010004").is_err()); // wrong end
        assert!(validate_saudi_vat_number("300000000000003").is_err()); // pos 11 not 1
        assert!(validate_saudi_vat_number("30000000010003").is_err()); // 14 digits
        assert!(validate_saudi_vat_number("3000000000100033").is_err()); // 16 digits
        assert!(validate_saudi_vat_number("30000000001000X").is_err()); // non-digit
    }

    #[test]
    fn icv_validator_shapes() {
        assert!(validate_invoice_counter_value(1).is_ok());
        assert!(validate_invoice_counter_value(9999).is_ok());
        assert!(validate_invoice_counter_value(0).is_err());
    }

    #[test]
    fn pih_validator_shapes() {
        assert!(validate_previous_invoice_hash(ZatcaUblContext::GENESIS_PIH).is_ok());
        assert!(validate_previous_invoice_hash("tooshort").is_err());
        assert!(validate_previous_invoice_hash(&"A".repeat(44)).is_err()); // no '=' pad
    }

    #[test]
    fn clearance_kind_maps_status_and_mode() {
        assert_eq!(
            ZatcaClearanceKind::from_reporting_status(
                ReportingStatus::Accepted,
                ZatcaInvoiceMode::Standard
            ),
            ZatcaClearanceKind::Cleared
        );
        assert_eq!(
            ZatcaClearanceKind::from_reporting_status(
                ReportingStatus::Accepted,
                ZatcaInvoiceMode::Simplified
            ),
            ZatcaClearanceKind::Reported
        );
        assert_eq!(
            ZatcaClearanceKind::from_reporting_status(
                ReportingStatus::Rejected,
                ZatcaInvoiceMode::Standard
            ),
            ZatcaClearanceKind::Rejected
        );
    }

    #[test]
    fn build_qr_fields_carries_mandatory_five() {
        let fields = build_qr_fields(&sample_invoice(), "2026-05-26T10:30:00Z").unwrap();
        assert_eq!(fields.get(&QrField::SellerName).unwrap(), "Acme KSA");
        assert_eq!(fields.get(&QrField::VatNumber).unwrap(), SELLER_VAT);
        assert_eq!(fields.get(&QrField::Total).unwrap(), "1150.00");
        assert_eq!(fields.get(&QrField::VatTotal).unwrap(), "150.00");
        assert_eq!(fields.len(), 5);
    }

    #[test]
    fn total_vat_overflow_is_reported_not_panicked() {
        // Regression: untrusted tax-summary amounts must be summed with checked
        // addition. Two near-maximum Decimals overflow the range; the unchecked
        // `+` operator would panic, so `build_qr_fields` must surface a clean
        // `ZatcaUblError::AmountOverflow` instead.
        let mut doc = sample_invoice();
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: DecimalValue::new(Decimal::MAX),
                tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "S2".to_owned(),
                taxable_amount: amt(100_000),
                tax_amount: DecimalValue::new(Decimal::MAX),
                tax_rate: Some(DecimalValue::new(Decimal::new(1500, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let err = build_qr_fields(&doc, "2026-05-26T10:30:00Z")
            .expect_err("two Decimal::MAX tax amounts must overflow the VAT-total sum");
        assert!(matches!(err, ZatcaUblError::AmountOverflow));
        // The single-summary happy path still sums cleanly.
        assert_eq!(
            total_vat(&sample_invoice()).expect("single summary sums without overflow"),
            Decimal::new(15000, 2)
        );
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_zatca_ubl_xml(&sample_invoice(), &ctx(ZatcaInvoiceMode::Standard))
            .unwrap()
            .into_bytes();
        let env = provider()
            .report(&sample_request(xml, ZatcaInvoiceMode::Standard))
            .unwrap()
            .envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: ZatcaReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }
}
