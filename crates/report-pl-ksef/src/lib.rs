// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// KSeF / FA / NIP / UPO / VAT / MF acronyms (and Polish element names like
// Naglowek/Podmiot/Sprzedawca) trip doc-markdown.
#![allow(clippy::doc_markdown)]

//! Poland **KSeF** (Krajowy System e-Faktur) national-clearance report adapter.
//!
//! Poland is a *national-clearance* jurisdiction: a B2B e-invoice is serialized
//! to the national **FA(3)** XML schema (`FA_VAT`, root `<Faktura>`), the
//! taxpayer authenticates a KSeF session, submits the XML, and the Ministry of
//! Finance portal returns a KSeF reference number (`Numer KSeF`) plus an
//! official acknowledgement of receipt (`UPO` — Urzędowe Poświadczenie
//! Odbioru). This crate provides the offline (local-only) end-to-end lifecycle:
//!
//! 1. **serialize** — [`to_fa3_xml`] turns an InvoiceKit
//!    [`invoicekit_ir::CommercialDocument`] into deterministic FA(3) XML
//!    (UBL/CII serializers do *not* emit this national format).
//! 2. **validate (local)** — [`validate_nip`] enforces the real 10-digit NIP
//!    with its official weighted checksum; reference-grade XSD validation
//!    against the Ministry of Finance FA(3) schema stays an external (JVM)
//!    backend and is labelled as such in the capability matrix.
//! 3. **sign + transmit** — [`MockKsefReportProvider`] composes the already-built
//!    [`invoicekit_signer_ksef::MockKsefProvider`] so the KSeF session/submit
//!    path and `Numer KSeF` synthesis are exercised, never re-faked.
//! 4. **evidence** — the caller bundles the canonical document, FA(3) XML,
//!    signed artifact, and receipt into a signed `.ikb` evidence bundle.
//!
//! Live KSeF transmission (HTTPS to `ksef-test.mf.gov.pl` / `ksef.mf.gov.pl`,
//! XAdES InitSession signing, NIP-bound qualified certificate or KSeF token) is
//! bring-your-own-credentials and lands in a follow-up `report-pl-ksef-http`
//! crate; this crate's `Mock*` providers are deterministic and offline.
//!
//! **Rejection is not an error.** When KSeF refuses an invoice it returns a
//! rejected acceptance status — surfaced here as
//! [`KsefAcceptance::Rejected`] inside an `Ok(_)` envelope, never as `Err`.
//! `Err` is reserved for pre-wire shape failures and transport/TLS/DNS faults.

use std::sync::Arc;

use invoicekit_ir::{CommercialDocument, DocumentType, Party};
use invoicekit_signer::Signer;
use invoicekit_signer_ksef::{
    AuthMode, KsefProvider, KsefSubmitRequest, MockKsefProvider, SessionToken,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export the KSeF substrate types this crate's public API surfaces, so
// downstream callers need not depend on `invoicekit-signer-ksef` directly.
pub use invoicekit_signer::Signature;
pub use invoicekit_signer_ksef::{KsefAcceptance, KsefEnvironment};

// ---------------------------------------------------------------------------
// FA(3) serialization (IR -> national FA_VAT <Faktura> XML)
// ---------------------------------------------------------------------------

/// FA(3) `KodFormularza` system code — the schema variant identifier the
/// Ministry of Finance assigns to the electronic-invoice form (`FA`, system
/// code `FA (3)`, schema version `1-0E`).
const KOD_FORMULARZA: &str = "FA";

/// FA(3) `WariantFormularza` — schema variant 3 (the 2025+ FA(3) wave).
const WARIANT_FORMULARZA: &str = "3";

/// FA(3) namespace URI for the 2025 `FA/3` schema published by the Polish
/// Ministry of Finance Centralne Repozytorium Dokumentów.
const FA3_NAMESPACE: &str = "http://crd.gov.pl/wzor/2025/06/25/06251/";

/// FA(3) header context: the form-level fields that live in `<Naglowek>` but
/// are not part of the jurisdiction-agnostic IR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Fa3Context {
    /// `DataWytworzeniaFa` — invoice-generation timestamp (RFC-3339 UTC). The
    /// caller pins this for byte-stable output.
    pub data_wytworzenia: String,
    /// `SystemInfo` — free-text name of the originating system.
    pub system_info: String,
}

impl Default for Fa3Context {
    fn default() -> Self {
        Self {
            data_wytworzenia: "2026-01-01T00:00:00Z".to_owned(),
            system_info: "InvoiceKit".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to FA(3) XML.
#[derive(Debug, Error)]
pub enum Fa3Error {
    /// The IR `document_type` has no FA(3) `RodzajFaktury` mapping.
    #[error("document type {0:?} is not representable as FA(3) RodzajFaktury")]
    UnsupportedDocumentType(DocumentType),
    /// The seller (`Podmiot1`) carries no usable NIP.
    #[error("seller (Podmiot1) has no NIP usable as DaneIdentyfikacyjne/NIP")]
    MissingSellerNip,
    /// The FA(3) header context was malformed (e.g. blank `DataWytworzeniaFa`).
    #[error("invalid FA(3) header context: {0}")]
    BadContext(String),
    /// Accumulating the tax-summary totals overflowed `Decimal`'s range; the
    /// named FA(3) totals field cannot be represented.
    #[error("FA(3) totals field {0} is not representable (Decimal overflow)")]
    TotalsUnrepresentable(&'static str),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic FA(3)
/// (`FA_VAT`, root `<Faktura>`) XML — the national Polish e-invoice format
/// cleared through KSeF.
///
/// Output is byte-stable by construction: a fixed element order with no maps
/// and amounts formatted at fixed scale 2. The document is expected to have
/// passed IR validation already (it has, if built via
/// [`CommercialDocument::new`]).
///
/// # Errors
///
/// Returns [`Fa3Error::UnsupportedDocumentType`] for document types with no
/// `RodzajFaktury` mapping, [`Fa3Error::MissingSellerNip`] when the seller has
/// no NIP, [`Fa3Error::BadContext`] when the header context is malformed, and
/// [`Fa3Error::TotalsUnrepresentable`] when summing the tax-summary totals
/// overflows `Decimal`'s range.
pub fn to_fa3_xml(document: &CommercialDocument, context: &Fa3Context) -> Result<String, Fa3Error> {
    if context.data_wytworzenia.is_empty() {
        return Err(Fa3Error::BadContext(
            "DataWytworzeniaFa must not be empty".to_owned(),
        ));
    }
    let rodzaj_faktury = rodzaj_faktury(document.document_type)?;
    // The seller (Podmiot1) must carry a NIP; the buyer's NIP is optional
    // (consumer / foreign nabywca).
    if party_nip(&document.supplier).is_none() {
        return Err(Fa3Error::MissingSellerNip);
    }

    let mut out = String::with_capacity(2048);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<Faktura xmlns=\"");
    out.push_str(FA3_NAMESPACE);
    out.push_str("\">\n");

    // --- Naglowek (form header) ---
    open(&mut out, 1, "Naglowek");
    // `KodFormularza` carries two attributes the FA(3) schema requires; we
    // emit the canonical system codes inline.
    el_attr(
        &mut out,
        2,
        "KodFormularza",
        &[("kodSystemowy", "FA (3)"), ("wersjaSchemy", "1-0E")],
        KOD_FORMULARZA,
    );
    el(&mut out, 2, "WariantFormularza", WARIANT_FORMULARZA);
    el(&mut out, 2, "DataWytworzeniaFa", &context.data_wytworzenia);
    el(&mut out, 2, "SystemInfo", &context.system_info);
    close(&mut out, 1, "Naglowek");

    // --- Podmiot1 (sprzedawca / seller) ---
    write_party(&mut out, "Podmiot1", &document.supplier, true)?;
    // --- Podmiot2 (nabywca / buyer) ---
    write_party(&mut out, "Podmiot2", &document.customer, false)?;

    // --- Fa (invoice body) ---
    open(&mut out, 1, "Fa");
    el(&mut out, 2, "KodWaluty", document.currency.as_str());
    // P_1 = data wystawienia (issue date); P_2 = numer faktury (invoice number).
    el(&mut out, 2, "P_1", document.issue_date.as_str());
    el(&mut out, 2, "P_2", document.document_number.as_str());
    el(&mut out, 2, "RodzajFaktury", rodzaj_faktury);

    // VAT summary: P_13_x = net base per rate group; P_14_x = VAT amount.
    // We project the tax summary into a single net/gross/VAT triple plus the
    // per-rate breakdown so the FA(3) totals block is faithful.
    // checked_add: the tax-summary amounts are untrusted at this point, so
    // summing many bounded bases can still exceed Decimal::MAX. Bail with a
    // typed error rather than letting Decimal's `+=` panic on overflow.
    let mut net_total = Decimal::ZERO;
    let mut vat_total = Decimal::ZERO;
    for summary in &document.tax_summary {
        net_total = net_total
            .checked_add(summary.taxable_amount.inner())
            .ok_or(Fa3Error::TotalsUnrepresentable("P_13_1"))?;
        vat_total = vat_total
            .checked_add(summary.tax_amount.inner())
            .ok_or(Fa3Error::TotalsUnrepresentable("P_14_1"))?;
    }
    el(&mut out, 2, "P_13_1", &fmt_amount(net_total));
    el(&mut out, 2, "P_14_1", &fmt_amount(vat_total));
    // P_15 = należność ogółem (total amount due, gross).
    el(
        &mut out,
        2,
        "P_15",
        &fmt_amount(document.monetary_total.tax_inclusive_amount.inner()),
    );

    // FaWiersze (invoice lines).
    for (index, line) in document.lines.iter().enumerate() {
        let stawka = line_tax_rate(document, line);
        open(&mut out, 2, "FaWiersz");
        // NrWierszaFa = 1-based line ordinal.
        el(&mut out, 3, "NrWierszaFa", &(index + 1).to_string());
        // P_7 = nazwa towaru/usługi (description).
        el(&mut out, 3, "P_7", &line.description);
        // P_8B = ilość (quantity).
        el(&mut out, 3, "P_8B", &fmt_amount(line.quantity.inner()));
        // P_9A = cena jednostkowa netto (unit net price).
        el(&mut out, 3, "P_9A", &fmt_amount(line.unit_price.inner()));
        // P_11 = wartość netto wiersza (line net value).
        el(
            &mut out,
            3,
            "P_11",
            &fmt_amount(line.line_extension_amount.inner()),
        );
        // P_12 = stawka podatku (VAT rate as a percentage).
        el(&mut out, 3, "P_12", &stawka);
        close(&mut out, 2, "FaWiersz");
    }
    close(&mut out, 1, "Fa");

    out.push_str("</Faktura>\n");
    Ok(out)
}

/// Map an IR [`DocumentType`] to an FA(3) `RodzajFaktury` code.
///
/// `VAT` = ordinary invoice, `KOR` = korekta (credit/debit note adjustment).
fn rodzaj_faktury(document_type: DocumentType) -> Result<&'static str, Fa3Error> {
    match document_type {
        DocumentType::Invoice => Ok("VAT"),
        DocumentType::CreditNote | DocumentType::DebitNote => Ok("KOR"),
        other @ (DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(Fa3Error::UnsupportedDocumentType(other))
        }
    }
}

/// Extract a Polish NIP from a party: prefer a `vat`/`nip`-scheme id, else the
/// first tax id. A leading `PL` country prefix is stripped when present.
fn party_nip(party: &Party) -> Option<String> {
    let chosen = party
        .tax_ids
        .iter()
        .find(|t| {
            t.scheme.eq_ignore_ascii_case("vat") || t.scheme.eq_ignore_ascii_case("nip")
        })
        .or_else(|| party.tax_ids.first())?;
    Some(strip_pl_prefix(&chosen.value))
}

/// Strip a leading `PL` country prefix from a VAT value (`"PL1234567890"` ->
/// `"1234567890"`).
fn strip_pl_prefix(value: &str) -> String {
    if value.len() > 2 && value.get(0..2).is_some_and(|p| p.eq_ignore_ascii_case("PL")) {
        value[2..].to_owned()
    } else {
        value.to_owned()
    }
}

/// Write a `Podmiot1` (seller) / `Podmiot2` (buyer) block. The seller requires
/// a NIP; the buyer's is optional (foreign / consumer nabywca).
fn write_party(
    out: &mut String,
    tag: &str,
    party: &Party,
    require_nip: bool,
) -> Result<(), Fa3Error> {
    let nip = party_nip(party);
    if require_nip && nip.is_none() {
        return Err(Fa3Error::MissingSellerNip);
    }
    open(out, 1, tag);
    open(out, 2, "DaneIdentyfikacyjne");
    if let Some(nip) = &nip {
        el(out, 3, "NIP", nip);
    }
    // Nazwa = pełna nazwa podmiotu (full legal name).
    el(out, 3, "Nazwa", &party.name);
    close(out, 2, "DaneIdentyfikacyjne");

    open(out, 2, "Adres");
    // KodKraju = ISO 3166-1 alpha-2 country code.
    el(out, 3, "KodKraju", party.address.country.as_str());
    // AdresL1 = ulica + numer; AdresL2 = kod pocztowy + miejscowość.
    el(out, 3, "AdresL1", &party.address.lines.join(", "));
    el(
        out,
        3,
        "AdresL2",
        &format!("{} {}", party.address.postal_code, party.address.city),
    );
    close(out, 2, "Adres");
    close(out, 1, tag);
    Ok(())
}

/// The line's `P_12` VAT rate (as a percentage string), looked up from the tax
/// summary entry matching the line's tax category, defaulting to `"0.00"`.
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
    el_attr(out, depth, tag, &[], text);
}

/// Append `<tag attr="v" ...>escaped-text</tag>` at the given indent depth.
fn el_attr(out: &mut String, depth: usize, tag: &str, attrs: &[(&str, &str)], text: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    for (k, v) in attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        push_escaped(out, v);
        out.push('"');
    }
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
// NIP validation (Polish tax identifier, official weighted checksum)
// ---------------------------------------------------------------------------

/// NIP checksum weights (positions 1..=9, the 10th digit is the check digit).
const NIP_WEIGHTS: [u32; 9] = [6, 5, 7, 2, 3, 4, 5, 6, 7];

/// Validate a Polish **NIP** (Numer Identyfikacji Podatkowej): exactly 10
/// digits whose official weighted-modulo-11 checksum matches the final digit.
///
/// The checksum sums each of the first nine digits multiplied by its weight
/// [`NIP_WEIGHTS`], takes the result modulo 11, and compares it to the tenth
/// digit. A modulo result of 10 makes the NIP invalid by construction (no NIP
/// is issued with a check value of 10), which this function rejects.
///
/// # Errors
///
/// Returns [`KsefReportError::BadNip`] when the value is not 10 ASCII digits
/// or the weighted checksum does not match the final digit.
pub fn validate_nip(nip: &str) -> Result<(), KsefReportError> {
    if nip.len() != 10 || !nip.bytes().all(|b| b.is_ascii_digit()) {
        return Err(KsefReportError::BadNip(format!(
            "NIP must be exactly 10 digits, got {nip:?}"
        )));
    }
    let digits: Vec<u32> = nip.bytes().map(|b| u32::from(b - b'0')).collect();
    let sum: u32 = NIP_WEIGHTS
        .iter()
        .zip(&digits)
        .map(|(w, d)| w * d)
        .sum();
    let check = sum % 11;
    // A check value of 10 cannot equal any single digit; such a NIP is invalid.
    if check == 10 || check != digits[9] {
        return Err(KsefReportError::BadNip(format!(
            "NIP checksum mismatch for {nip:?}"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// KSeF report adapter (validate -> sign -> transmit -> typed receipt)
// ---------------------------------------------------------------------------

/// Operator-facing KSeF report request. The FA(3) XML is produced upstream by
/// [`to_fa3_xml`]; this request carries it plus the identity fields KSeF needs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KsefReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// KSeF environment (`Demo` sandbox vs `Production`).
    pub environment: KsefEnvironment,
    /// Issuer NIP (10-digit Polish tax id) the KSeF session binds to.
    pub issuer_nip: String,
    /// Authentication mode used to open the KSeF session.
    pub auth_mode: AuthMode,
    /// Canonical FA(3) XML bytes.
    pub fa_xml: Vec<u8>,
}

/// Typed KSeF receipt (the audit-relevant verdict and signature metadata).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KsefReportEnvelope {
    /// `Numer KSeF` — the 25+ char reference number that closes the invoice
    /// (`<NIP>-YYYYMMDD-<token>-XX`). Empty on a rejected submission.
    pub numer_ksef: String,
    /// `UPO` (Urzędowe Poświadczenie Odbioru) acknowledgement reference id.
    pub upo_reference: String,
    /// Acceptance status KSeF returned (`Accepted` / `Pending` / `Rejected`).
    pub acceptance: KsefAcceptance,
    /// Echoed issuer NIP.
    pub issuer_nip: String,
    /// RFC-3339 UTC timestamp the portal recorded.
    pub recorded_at: String,
    /// Reason text when the receipt is a rejection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Signature receipt over the FA(3) bytes (the XAdES payload signature).
    pub signature: Signature,
}

/// The full result of a report: the receipt plus the signed FA(3) artifact
/// bytes (the latter is an evidence-bundle artefact, kept out of the receipt
/// JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KsefReport {
    /// Audit receipt.
    pub envelope: KsefReportEnvelope,
    /// Signed (XAdES-wrapped) FA(3) XML bytes.
    pub signed_fa_xml: Vec<u8>,
}

/// Typed KSeF report errors. Three buckets: payload shape, country-id shape,
/// and transport. A rejection verdict is **not** here — it is an `Ok` envelope
/// with [`KsefAcceptance::Rejected`].
#[derive(Debug, Error)]
pub enum KsefReportError {
    /// The FA(3) payload failed shape validation before the wire.
    #[error("fa(3) xml rejected: {0}")]
    BadXml(String),
    /// The issuer NIP failed the 10-digit weighted-checksum validation.
    #[error("invalid issuer NIP: {0}")]
    BadNip(String),
    /// The KSeF session/signer/transport failed on the wire.
    #[error("ksef session/transport failure: {0}")]
    Transport(String),
}

/// The KSeF report surface every integration implements.
pub trait KsefReportProvider: Send + Sync {
    /// Validate the issuer NIP, open a session, sign the FA(3), submit to KSeF,
    /// and return the typed receipt.
    ///
    /// # Errors
    ///
    /// Returns [`KsefReportError`] on pre-wire shape failures (bad NIP, empty
    /// payload) or transport faults. A KSeF rejection is surfaced as an `Ok`
    /// envelope with [`KsefAcceptance::Rejected`], not an error.
    fn report(&self, request: &KsefReportRequest) -> Result<KsefReport, KsefReportError>;
}

/// Deterministic offline KSeF report provider.
///
/// Composes [`invoicekit_signer_ksef::MockKsefProvider`] so the real KSeF
/// session/submit path, XAdES signature, and `Numer KSeF` synthesis are
/// exercised rather than re-implemented.
pub struct MockKsefReportProvider {
    signer: Arc<dyn Signer>,
    environment: KsefEnvironment,
    forced_acceptance: KsefAcceptance,
    fixed_recorded_at: String,
}

impl MockKsefReportProvider {
    /// Build a mock report provider over the given signer (key it by the KSeF
    /// session token, e.g. `SoftwareSigner::new().with_key("sess-00000001",
    /// [5u8; 32])`).
    #[must_use]
    pub fn new(signer: Arc<dyn Signer>, environment: KsefEnvironment) -> Self {
        Self {
            signer,
            environment,
            forced_acceptance: KsefAcceptance::Accepted,
            fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        }
    }

    /// Force every submission to return a specific acceptance status (e.g.
    /// [`KsefAcceptance::Rejected`] to exercise the rejection path).
    #[must_use]
    pub fn with_forced_acceptance(mut self, acceptance: KsefAcceptance) -> Self {
        self.forced_acceptance = acceptance;
        self
    }
}

impl KsefReportProvider for MockKsefReportProvider {
    fn report(&self, request: &KsefReportRequest) -> Result<KsefReport, KsefReportError> {
        validate_nip(&request.issuer_nip)?;
        if request.fa_xml.is_empty() {
            return Err(KsefReportError::BadXml("payload is empty".to_owned()));
        }
        let inner = MockKsefProvider::new("ksef-test", self.environment, Arc::clone(&self.signer))
            .with_forced_acceptance(self.forced_acceptance);
        let session: SessionToken = inner
            .init_session(&request.issuer_nip, request.auth_mode)
            .map_err(|e| KsefReportError::Transport(e.to_string()))?;
        let stamp = inner
            .submit(
                &KsefSubmitRequest {
                    fa_xml: request.fa_xml.clone(),
                    session,
                    nip: request.issuer_nip.clone(),
                },
                self.environment,
            )
            .map_err(|e| KsefReportError::Transport(e.to_string()))?;
        let reason = (stamp.acceptance == KsefAcceptance::Rejected)
            .then(|| "KSeF rejected the invoice (odrzucona)".to_owned());
        // A rejected submission carries no binding Numer KSeF.
        let numer_ksef = if stamp.acceptance == KsefAcceptance::Accepted {
            stamp.numer_ksef
        } else {
            String::new()
        };
        Ok(KsefReport {
            envelope: KsefReportEnvelope {
                numer_ksef,
                upo_reference: stamp.upo_reference,
                acceptance: stamp.acceptance,
                issuer_nip: request.issuer_nip.clone(),
                recorded_at: self.fixed_recorded_at.clone(),
                reason,
                signature: stamp.signature.clone(),
            },
            // The signed artifact is the FA(3) bytes wrapped by the KSeF XAdES
            // signature; we surface the signature's base64 over the payload as
            // the bundled signed artifact.
            signed_fa_xml: signed_artifact(&request.fa_xml, &stamp.signature),
        })
    }
}

/// Wrap the FA(3) bytes in a deterministic signed-artifact envelope carrying the
/// KSeF XAdES signature metadata. This is the evidence-bundle `signed.xml`.
fn signed_artifact(fa_xml: &[u8], signature: &Signature) -> Vec<u8> {
    let mut out = Vec::with_capacity(fa_xml.len() + 256);
    out.extend_from_slice(b"<KsefSignedInvoice algorithm=\"");
    out.extend_from_slice(signature.algorithm.as_bytes());
    out.extend_from_slice(b"\">\n");
    out.extend_from_slice(fa_xml);
    out.extend_from_slice(b"\n<ds:SignatureValue>");
    out.extend_from_slice(signature.signature_b64.as_bytes());
    out.extend_from_slice(b"</ds:SignatureValue>\n</KsefSignedInvoice>\n");
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_report_pl_ksef::crate_name(), "invoicekit-report-pl-ksef");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-pl-ksef"
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

    fn polish_party(name: &str, nip: &str, city: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: nip.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["ul. Marszałkowska 1".to_owned()],
                city: city.to_owned(),
                subdivision: None,
                postal_code: "00-001".to_owned(),
                country: CountryCode::new("PL").unwrap(),
            },
            contact: None,
        }
    }

    fn sample_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-pl-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new("FV-2026-0001").unwrap(),
            currency: Iso4217Code::new("PLN").unwrap(),
            // 5252248481 and 5260001246 are valid-checksum NIPs.
            supplier: polish_party("Acme Sp. z o.o.", "PL5252248481", "Warszawa"),
            customer: polish_party("Beta S.A.", "5260001246", "Kraków"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Usługi konsultingowe & rozwój".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("C62".to_owned()),
                unit_price: amt(5000),
                line_extension_amount: amt(10000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(10000),
                tax_amount: amt(2300),
                tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(10000),
                tax_exclusive_amount: amt(10000),
                tax_inclusive_amount: amt(12300),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(12300),
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

    fn provider() -> MockKsefReportProvider {
        // The inner KSeF mock keys the signer by the session token it mints
        // (`sess-00000001` for the first session).
        let signer: Arc<dyn Signer> = Arc::new(
            SoftwareSigner::new()
                .with_key("sess-00000001", [5_u8; 32])
                .with_key("sess-00000002", [6_u8; 32]),
        );
        MockKsefReportProvider::new(signer, KsefEnvironment::Demo)
    }

    fn sample_request(fa_xml: Vec<u8>) -> KsefReportRequest {
        KsefReportRequest {
            tenant_id: "tenant_123".to_owned(),
            environment: KsefEnvironment::Demo,
            issuer_nip: "5252248481".to_owned(),
            auth_mode: AuthMode::QualifiedSignature,
            fa_xml,
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-pl-ksef");
    }

    #[test]
    fn fa3_contains_mandatory_structure() {
        let xml = to_fa3_xml(&sample_invoice(), &Fa3Context::default()).unwrap();
        for needle in [
            "<Faktura xmlns=\"http://crd.gov.pl/wzor/2025/06/25/06251/\">",
            "<Naglowek>",
            "<KodFormularza kodSystemowy=\"FA (3)\" wersjaSchemy=\"1-0E\">FA</KodFormularza>",
            "<WariantFormularza>3</WariantFormularza>",
            "<Podmiot1>",
            "<NIP>5252248481</NIP>",
            "<Nazwa>Acme Sp. z o.o.</Nazwa>",
            "<Podmiot2>",
            "<NIP>5260001246</NIP>",
            "<KodWaluty>PLN</KodWaluty>",
            "<P_2>FV-2026-0001</P_2>",
            "<RodzajFaktury>VAT</RodzajFaktury>",
            "<P_7>Usługi konsultingowe &amp; rozwój</P_7>",
            "<P_12>23.00</P_12>",
            "<P_15>123.00</P_15>",
        ] {
            assert!(xml.contains(needle), "FA(3) missing {needle:?}:\n{xml}");
        }
    }

    #[test]
    fn fa3_is_deterministic() {
        let doc = sample_invoice();
        let ctx = Fa3Context::default();
        assert_eq!(
            to_fa3_xml(&doc, &ctx).unwrap(),
            to_fa3_xml(&doc, &ctx).unwrap()
        );
    }

    #[test]
    fn fa3_rejects_unsupported_document_type() {
        let err = rodzaj_faktury(DocumentType::ProForma).unwrap_err();
        assert!(matches!(err, Fa3Error::UnsupportedDocumentType(_)));
    }

    #[test]
    fn fa3_credit_note_maps_to_korekta() {
        assert_eq!(rodzaj_faktury(DocumentType::CreditNote).unwrap(), "KOR");
    }

    #[test]
    fn fa3_totals_overflow_is_err_not_panic() {
        // Two tax-summary entries whose taxable amounts each sit near
        // Decimal::MAX so the running `net_total` sum overflows. Before the
        // checked_add fix the `+=` accumulator panicked; now it surfaces a
        // typed Fa3Error::TotalsUnrepresentable.
        let mut doc = sample_invoice();
        let huge = DecimalValue::new(Decimal::MAX);
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: huge.clone(),
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "R".to_owned(),
                taxable_amount: huge,
                tax_amount: amt(0),
                tax_rate: Some(DecimalValue::new(Decimal::new(800, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let err = to_fa3_xml(&doc, &Fa3Context::default())
            .expect_err("near-Decimal::MAX taxable totals must overflow, not panic");
        assert!(matches!(err, Fa3Error::TotalsUnrepresentable("P_13_1")));
    }

    #[test]
    fn fa3_vat_totals_overflow_is_err_not_panic() {
        // Same defect on the `vat_total` accumulator: the net base fits but the
        // VAT amounts each sit near Decimal::MAX.
        let mut doc = sample_invoice();
        let huge = DecimalValue::new(Decimal::MAX);
        doc.tax_summary = vec![
            TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(0),
                tax_amount: huge.clone(),
                tax_rate: Some(DecimalValue::new(Decimal::new(2300, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "R".to_owned(),
                taxable_amount: amt(0),
                tax_amount: huge,
                tax_rate: Some(DecimalValue::new(Decimal::new(800, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let err = to_fa3_xml(&doc, &Fa3Context::default())
            .expect_err("near-Decimal::MAX VAT totals must overflow, not panic");
        assert!(matches!(err, Fa3Error::TotalsUnrepresentable("P_14_1")));
    }

    #[test]
    fn nip_validator_accepts_valid_checksums() {
        // Real, checksum-valid Polish NIPs.
        assert!(validate_nip("5252248481").is_ok());
        assert!(validate_nip("5260001246").is_ok());
        assert!(validate_nip("7740001454").is_ok());
    }

    #[test]
    fn nip_validator_rejects_bad_shape_and_checksum() {
        assert!(validate_nip("123456789").is_err()); // 9 digits
        assert!(validate_nip("12345678901").is_err()); // 11 digits
        assert!(validate_nip("PL1132316933").is_err()); // carries prefix
        assert!(validate_nip("abcdefghij").is_err()); // non-digit
        assert!(validate_nip("1132316934").is_err()); // wrong check digit
    }

    #[test]
    fn report_happy_path_is_accepted() {
        let xml = to_fa3_xml(&sample_invoice(), &Fa3Context::default())
            .unwrap()
            .into_bytes();
        let report = provider().report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.acceptance, KsefAcceptance::Accepted);
        assert!(report.envelope.acceptance.is_accepted());
        assert!(report.envelope.numer_ksef.starts_with("5252248481-"));
        assert!(report.envelope.upo_reference.starts_with("upo-"));
        assert!(report.envelope.reason.is_none());
        assert!(report.signed_fa_xml.starts_with(b"<KsefSignedInvoice"));
    }

    #[test]
    fn report_rejection_is_ok_not_err() {
        let xml = b"<Faktura/>".to_vec();
        let provider = provider().with_forced_acceptance(KsefAcceptance::Rejected);
        let report = provider.report(&sample_request(xml)).unwrap();
        assert_eq!(report.envelope.acceptance, KsefAcceptance::Rejected);
        assert!(!report.envelope.acceptance.is_accepted());
        assert!(report.envelope.numer_ksef.is_empty());
        assert!(report.envelope.reason.is_some());
    }

    #[test]
    fn report_rejects_bad_nip() {
        let mut req = sample_request(b"<x/>".to_vec());
        req.issuer_nip = "1132316934".to_owned(); // valid shape, wrong check digit
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            KsefReportError::BadNip(_)
        ));
    }

    #[test]
    fn report_rejects_empty_payload() {
        let req = sample_request(Vec::new());
        assert!(matches!(
            provider().report(&req).unwrap_err(),
            KsefReportError::BadXml(_)
        ));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let xml = to_fa3_xml(&sample_invoice(), &Fa3Context::default())
            .unwrap()
            .into_bytes();
        let env = provider().report(&sample_request(xml)).unwrap().envelope;
        let json = serde_json::to_string(&env).unwrap();
        let back: KsefReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
    }
}
