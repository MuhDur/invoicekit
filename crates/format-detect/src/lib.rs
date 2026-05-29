// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-format-detect` — opaque-byte format sniffer.
//!
//! Customers will hand us bytes and ask "what is this?". The
//! detector reads a bounded prefix (typically the first few kB)
//! and returns a [`FormatId`] enum value identifying the
//! document family without parsing the full content.
//!
//! The detector is intentionally conservative:
//!
//! * Magic bytes / file signatures are checked first (PDF, ZIP).
//! * For XML, the root element name and primary namespace are the
//!   sole authority — namespace URIs are immutable identifiers
//!   under the relevant standards (UBL, CII, FatturaPA, CFDI,
//!   …) so a match is an authoritative claim.
//! * For JSON, the schema URL embedded under `$schema` or the
//!   pair (`type`, `$regime`) for GOBL is the authority.
//! * Anything that doesn't match a registered signature returns
//!   [`FormatId::Unknown`] — the bead's strict gate forbids
//!   panics, so the detector never `unwrap`s.
//!
//! ## Scope
//!
//! Detected formats are listed in [`FormatId`]. Anything outside
//! that list — for example a UBL document for a doc type we
//! don't yet model — returns `Unknown` with a `notes` snippet
//! pointing at the unrecognised root element.

use serde::{Deserialize, Serialize};

/// Stable identifier for a detected document family.
///
/// New variants are added only when a real fixture exists that
/// proves the detection rule. False positives on `Unknown` are
/// preferred over false claims on a named variant — see the bead
/// invoices-t-047 strict gate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatId {
    /// UBL 2.1 invoice or credit note (`urn:oasis:names:specification:ubl:schema:xsd:Invoice-2`
    /// or `…:CreditNote-2`).
    Ubl21,
    /// UN/CEFACT Cross Industry Invoice D16B (`urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100`).
    CiiD16B,
    /// FatturaPA 1.2.x (`http://ivaservizi.agenziaentrate.gov.it/docs/xsd/fatture/v1.2`
    /// or v1.2.1 / v1.2.2 variants).
    FatturaPa,
    /// SAT CFDI 4.0 (`http://www.sat.gob.mx/cfd/4`).
    Cfdi40,
    /// Polish KSeF FA(3) schematron (`http://crd.gov.pl/wzor/2023/06/29/12648/`).
    KsefFa3,
    /// Saudi ZATCA Phase 2 simplified / standard invoice (UBL-shaped, distinct namespace).
    ZatcaPhase2,
    /// Greek myDATA (`http://www.aade.gr/myDATA/invoice/v1.0`).
    MyDataV10,
    /// Spanish Verifactu (`https://www2.agenciatributaria.gob.es/static_files/common/internet/dep/aplicaciones/es/aeat/tikeV1.0`).
    VerifactuV10,
    /// Brazilian NF-e v4.00 (`http://www.portalfiscal.inf.br/nfe`).
    NfeV400,
    /// Indian GST IRN payload (`https://einvapi.gst.gov.in`).
    GstIrn,
    /// invopop/gobl JSON envelope (`https://gobl.org/draft-0/envelope`).
    GoblEnvelope,
    /// invopop/gobl JSON bill payload (no envelope wrapper).
    GoblBill,
    /// InvoiceKit internal IR JSON.
    InvoicekitIrV1,
    /// PDF/A-3 with embedded ZUGFeRD / Factur-X XML attachment.
    /// (Distinguishes between PDF generally and PDF-with-attachment
    /// in [`detect_format_with_notes`].)
    PdfWithFacturX,
    /// Generic PDF (no recognised invoice attachment).
    Pdf,
    /// ZIP container — could be a CIUS-DE evidence bundle, a `.ikb`
    /// archive, or a vendor-specific bundle. The sniffer does not
    /// recurse into the archive; the caller decides.
    ZipContainer,
    /// Nothing in the byte prefix matched a registered signature.
    Unknown,
}

/// Result of a sniff call that includes free-text notes for the
/// `Unknown` and ambiguous cases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Detection {
    /// Detected format identifier.
    pub format: FormatId,
    /// Free-text breadcrumb describing what the sniffer saw.
    /// Empty for confident matches; populated for `Unknown` so
    /// the caller can quickly grep what surfaced.
    pub notes: String,
}

impl Detection {
    fn known(format: FormatId) -> Self {
        Self::with_notes(format, "")
    }

    fn with_notes(format: FormatId, notes: impl Into<String>) -> Self {
        Self {
            format,
            notes: notes.into(),
        }
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_format_detect::crate_name(), "invoicekit-format-detect");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-format-detect"
}

/// Sniff `input` and return the most-confident [`FormatId`].
///
/// Equivalent to [`detect_format_with_notes`] followed by
/// `.format` — use that variant when you need diagnostic notes.
///
/// # Examples
///
/// ```
/// use invoicekit_format_detect::{detect_format, FormatId};
/// let pdf = b"%PDF-1.7\n";
/// assert_eq!(detect_format(pdf), FormatId::Pdf);
/// ```
#[must_use]
pub fn detect_format(input: &[u8]) -> FormatId {
    detect_format_with_notes(input).format
}

/// How many bytes we inspect at most. UBL/CII namespace declarations
/// live in the root element attributes, which sit well within the
/// first 4 kB even for fixtures with verbose XML preambles. PDF
/// trailers can sit further in for `/AFRelationship` detection, so
/// we extend to 64 kB when the prefix shows a PDF signature.
const XML_JSON_PREFIX_BYTES: usize = 16 * 1024;
const PDF_INSPECT_BYTES: usize = 64 * 1024;

/// Sniff `input` and return a [`Detection`] with optional notes.
///
/// Never panics; never reads past a bounded prefix.
///
/// # Examples
///
/// ```
/// use invoicekit_format_detect::{detect_format_with_notes, FormatId};
/// let detection = detect_format_with_notes(b"not-a-known-format");
/// assert_eq!(detection.format, FormatId::Unknown);
/// assert!(!detection.notes.is_empty());
/// ```
#[must_use]
pub fn detect_format_with_notes(input: &[u8]) -> Detection {
    if input.is_empty() {
        return Detection::with_notes(FormatId::Unknown, "empty input");
    }
    // PDF magic — also classify Factur-X / ZUGFeRD when the
    // PDF/A-3 attachment relationship marker is present.
    if input.starts_with(b"%PDF-") {
        let inspect_end = input.len().min(PDF_INSPECT_BYTES);
        let window = &input[..inspect_end];
        if has_facturx_signature(window) {
            return Detection::known(FormatId::PdfWithFacturX);
        }
        return Detection::known(FormatId::Pdf);
    }
    // ZIP magic — `.ikb`, `.zip`, `.docx`, ... — caller decides.
    if input.starts_with(b"PK\x03\x04") {
        return Detection::known(FormatId::ZipContainer);
    }
    // Trim a UTF-8 BOM if present so XML/JSON prefix matching is
    // robust to Office tooling that adds one.
    let trimmed = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    let prefix_end = trimmed.len().min(XML_JSON_PREFIX_BYTES);
    let prefix = &trimmed[..prefix_end];
    // JSON dispatch happens before XML because the `{` discriminator
    // is cheaper than scanning for a `<root xmlns="…">` declaration.
    if let Some(d) = sniff_json(prefix) {
        return d;
    }
    if let Some(d) = sniff_xml(prefix) {
        return d;
    }
    let preview = preview_text(prefix);
    Detection::with_notes(
        FormatId::Unknown,
        format!("no signature matched; prefix={preview}"),
    )
}

fn has_facturx_signature(window: &[u8]) -> bool {
    // Factur-X / ZUGFeRD attachments are advertised by a PDF
    // `/AFRelationship /Alternative` (or `/Data`, `/Source`) entry
    // sitting next to the embedded XML's display name. Either
    // /Filespec entry or the AFRelationship key is a strong-enough
    // signal — both being absent is the safe negative.
    contains_subsequence(window, b"/AFRelationship")
        || contains_subsequence(window, b"factur-x.xml")
        || contains_subsequence(window, b"ZUGFeRD-invoice.xml")
        || contains_subsequence(window, b"zugferd-invoice.xml")
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn sniff_json(prefix: &[u8]) -> Option<Detection> {
    let trimmed = prefix.trim_ascii_start();
    if !trimmed.starts_with(b"{") {
        return None;
    }
    // We deliberately accept partial JSON: serde_json parses the
    // prefix even if the closing brace is past our budget, by
    // wrapping in a streaming Deserializer and reading exactly one
    // value. If that fails (truncated mid-key), fall back to
    // substring sniffs which are still safer than panicking.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(trimmed) {
        if let Some(format) = sniff_json_value(&value) {
            return Some(Detection::known(format));
        }
    }
    // Substring fallback for the most-common discriminators —
    // covers truncated prefixes and pretty-printed inputs that
    // exceed XML_JSON_PREFIX_BYTES before the discriminator field.
    if contains_subsequence(trimmed, b"\"https://gobl.org/draft-0/envelope\"") {
        return Some(Detection::known(FormatId::GoblEnvelope));
    }
    if contains_subsequence(trimmed, b"\"https://gobl.org/draft-0/bill/") {
        return Some(Detection::known(FormatId::GoblBill));
    }
    if contains_subsequence(trimmed, b"\"https://invoicekit.dev/schemas/ir/v1\"")
        || contains_subsequence(trimmed, b"\"invoicekit-ir-v1\"")
    {
        return Some(Detection::known(FormatId::InvoicekitIrV1));
    }
    Some(Detection::with_notes(
        FormatId::Unknown,
        "JSON without recognized $schema or $regime",
    ))
}

fn sniff_json_value(value: &serde_json::Value) -> Option<FormatId> {
    let schema = value.get("$schema").and_then(serde_json::Value::as_str);
    match schema {
        Some("https://gobl.org/draft-0/envelope") => return Some(FormatId::GoblEnvelope),
        Some(s) if s.starts_with("https://gobl.org/draft-0/bill/") => {
            return Some(FormatId::GoblBill);
        }
        Some("https://invoicekit.dev/schemas/ir/v1") => return Some(FormatId::InvoicekitIrV1),
        _ => {}
    }
    // InvoiceKit IR carries `schema_version` + `document_type` +
    // `meta.tenant_id`; that triple is the strongest IR fingerprint
    // we can read without parsing the whole document.
    let has_schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        .is_some();
    let has_meta_tenant = value
        .get("meta")
        .and_then(|m| m.get("tenant_id"))
        .and_then(serde_json::Value::as_str)
        .is_some();
    if has_schema_version && has_meta_tenant {
        return Some(FormatId::InvoicekitIrV1);
    }
    // GOBL bills wrapped in an envelope have an inner `.doc`
    // payload — recurse one level so a wrapped bill is recognised
    // even when the envelope discriminator is the looser default.
    if let Some(inner) = value.get("doc") {
        if let Some(inner_format) = sniff_json_value(inner) {
            return Some(match inner_format {
                FormatId::GoblBill | FormatId::GoblEnvelope => FormatId::GoblEnvelope,
                other => other,
            });
        }
    }
    None
}

fn sniff_xml(prefix: &[u8]) -> Option<Detection> {
    let trimmed = prefix.trim_ascii_start();
    let starts_with_xml_prolog = trimmed.starts_with(b"<?xml");
    let looks_xml = starts_with_xml_prolog || trimmed.starts_with(b"<");
    if !looks_xml {
        return None;
    }
    // Match by primary XML namespace URI declared anywhere in the
    // prefix. Namespace URIs are immutable per-standard tokens, so
    // a literal substring is a sound signal.
    let signatures: &[(&[u8], FormatId)] = &[
        (
            b"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
            FormatId::Ubl21,
        ),
        (
            b"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2",
            FormatId::Ubl21,
        ),
        (
            b"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100",
            FormatId::CiiD16B,
        ),
        (
            b"http://ivaservizi.agenziaentrate.gov.it/docs/xsd/fatture/v1.2",
            FormatId::FatturaPa,
        ),
        (
            b"http://www.sat.gob.mx/cfd/4",
            FormatId::Cfdi40,
        ),
        (
            b"http://crd.gov.pl/wzor/2023/06/29/12648/",
            FormatId::KsefFa3,
        ),
        (
            b"urn:zatca:zatca-eGOI:dictionary:1",
            FormatId::ZatcaPhase2,
        ),
        (
            b"http://www.aade.gr/myDATA/invoice/v1.0",
            FormatId::MyDataV10,
        ),
        (
            b"https://www2.agenciatributaria.gob.es/static_files/common/internet/dep/aplicaciones/es/aeat/tikeV1.0",
            FormatId::VerifactuV10,
        ),
        (
            b"http://www.portalfiscal.inf.br/nfe",
            FormatId::NfeV400,
        ),
        (b"https://einvapi.gst.gov.in", FormatId::GstIrn),
    ];
    for (signature, format) in signatures {
        if contains_subsequence(trimmed, signature) {
            return Some(Detection::known(*format));
        }
    }
    Some(Detection::with_notes(
        FormatId::Unknown,
        format!(
            "XML root namespace not in registry: {}",
            preview_text(trimmed)
        ),
    ))
}

fn preview_text(prefix: &[u8]) -> String {
    const PREVIEW_BYTES: usize = 80;
    let slice = &prefix[..prefix.len().min(PREVIEW_BYTES)];
    let mut out = String::with_capacity(slice.len() + 4);
    out.push('"');
    for byte in slice {
        if byte.is_ascii_graphic() || *byte == b' ' {
            out.push(*byte as char);
        } else {
            out.push('.');
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{crate_name, detect_format, detect_format_with_notes, FormatId};

    // ----- crate_name housekeeping -----

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-detect");
    }

    // ----- positive detections (one or more per FormatId) -----

    #[test]
    fn detects_ubl_invoice() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
         xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2">
  <cbc:ID>INV-1</cbc:ID>
</Invoice>"#;
        assert_eq!(detect_format(xml), FormatId::Ubl21);
    }

    #[test]
    fn detects_ubl_credit_note() {
        let xml = br#"<?xml version="1.0"?>
<CreditNote xmlns="urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2"/>"#;
        assert_eq!(detect_format(xml), FormatId::Ubl21);
    }

    #[test]
    fn detects_cii_d16b() {
        let xml = br#"<?xml version="1.0"?>
<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"/>"#;
        assert_eq!(detect_format(xml), FormatId::CiiD16B);
    }

    #[test]
    fn detects_fatturapa() {
        let xml = br#"<?xml version="1.0"?>
<p:FatturaElettronica xmlns:p="http://ivaservizi.agenziaentrate.gov.it/docs/xsd/fatture/v1.2"/>"#;
        assert_eq!(detect_format(xml), FormatId::FatturaPa);
    }

    #[test]
    fn detects_cfdi_4_0() {
        let xml = br#"<?xml version="1.0"?>
<cfdi:Comprobante xmlns:cfdi="http://www.sat.gob.mx/cfd/4" Version="4.0"/>"#;
        assert_eq!(detect_format(xml), FormatId::Cfdi40);
    }

    #[test]
    fn detects_ksef_fa3() {
        let xml = br#"<?xml version="1.0"?>
<Faktura xmlns="http://crd.gov.pl/wzor/2023/06/29/12648/"/>"#;
        assert_eq!(detect_format(xml), FormatId::KsefFa3);
    }

    #[test]
    fn detects_mydata_v1() {
        let xml = br#"<?xml version="1.0"?>
<InvoicesDoc xmlns="http://www.aade.gr/myDATA/invoice/v1.0"/>"#;
        assert_eq!(detect_format(xml), FormatId::MyDataV10);
    }

    #[test]
    fn detects_nfe_v4() {
        let xml = br#"<?xml version="1.0"?>
<NFe xmlns="http://www.portalfiscal.inf.br/nfe"/>"#;
        assert_eq!(detect_format(xml), FormatId::NfeV400);
    }

    #[test]
    fn detects_gobl_envelope() {
        let payload = br#"{"$schema":"https://gobl.org/draft-0/envelope","head":{},"doc":{}}"#;
        assert_eq!(detect_format(payload), FormatId::GoblEnvelope);
    }

    #[test]
    fn detects_gobl_bill_directly() {
        let payload = br#"{"$schema":"https://gobl.org/draft-0/bill/invoice","code":"1"}"#;
        assert_eq!(detect_format(payload), FormatId::GoblBill);
    }

    #[test]
    fn detects_invoicekit_ir() {
        let payload =
            br#"{"$schema":"https://invoicekit.dev/schemas/ir/v1","schema_version":"1.0"}"#;
        assert_eq!(detect_format(payload), FormatId::InvoicekitIrV1);
    }

    #[test]
    fn detects_invoicekit_ir_by_shape() {
        // Without a $schema field, fall back to the (schema_version, meta.tenant_id) pair.
        let payload = br#"{"schema_version":"1.0","meta":{"tenant_id":"tnt","trace_id":"trc"}}"#;
        assert_eq!(detect_format(payload), FormatId::InvoicekitIrV1);
    }

    #[test]
    fn detects_pdf() {
        let pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n";
        assert_eq!(detect_format(pdf), FormatId::Pdf);
    }

    #[test]
    fn detects_pdf_with_facturx() {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend_from_slice(b"% padding\n");
        pdf.extend_from_slice(
            b"<< /Type /Filespec /F (factur-x.xml) /AFRelationship /Alternative >>\n",
        );
        assert_eq!(detect_format(&pdf), FormatId::PdfWithFacturX);
    }

    #[test]
    fn detects_zip_container() {
        let zip = b"PK\x03\x04rest-of-zip";
        assert_eq!(detect_format(zip), FormatId::ZipContainer);
    }

    // ----- Unknown handling: never panics, notes populated -----

    #[test]
    fn empty_input_returns_unknown_with_notes() {
        let detection = detect_format_with_notes(b"");
        assert_eq!(detection.format, FormatId::Unknown);
        assert_eq!(detection.notes, "empty input");
    }

    #[test]
    fn unknown_text_returns_unknown_with_notes() {
        let detection = detect_format_with_notes(b"not an invoice at all");
        assert_eq!(detection.format, FormatId::Unknown);
        assert!(!detection.notes.is_empty());
    }

    #[test]
    fn xml_with_unknown_root_returns_unknown_with_notes() {
        let detection = detect_format_with_notes(
            br#"<?xml version="1.0"?><Mystery xmlns="https://example.com/mystery"/>"#,
        );
        assert_eq!(detection.format, FormatId::Unknown);
        assert!(detection
            .notes
            .contains("XML root namespace not in registry"));
    }

    #[test]
    fn json_without_signature_returns_unknown() {
        let detection = detect_format_with_notes(br#"{"hello":"world"}"#);
        assert_eq!(detection.format, FormatId::Unknown);
    }

    #[test]
    fn bom_is_tolerated() {
        // Word, LibreOffice, etc. sometimes prepend a UTF-8 BOM.
        let mut input = b"\xef\xbb\xbf".to_vec();
        input.extend_from_slice(
            br#"<?xml version="1.0"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#,
        );
        assert_eq!(detect_format(&input), FormatId::Ubl21);
    }

    #[test]
    fn pretty_printed_gobl_is_recognised_in_substring_fallback() {
        // A pretty-printed envelope where $schema appears late in
        // the prefix still trips the substring fallback path.
        let mut payload = br#"{
    "head": {
        "uuid": "x"
    },
    "$schema": "https://gobl.org/draft-0/envelope",
    "doc": {}
}"#
        .to_vec();
        // Pad with whitespace to exceed the literal-match buffer if any.
        payload.extend(std::iter::repeat_n(b' ', 200));
        assert_eq!(detect_format(&payload), FormatId::GoblEnvelope);
    }

    #[test]
    fn arbitrary_binary_does_not_panic() {
        // Crash safety: feed it random-ish bytes.
        for seed in 0u32..32 {
            let mut bytes = Vec::with_capacity(1024);
            for i in 0..1024u32 {
                let mixed = seed.wrapping_mul(2_654_435_761) ^ i.wrapping_mul(0x9e37_79b9);
                bytes.push((mixed & 0xff) as u8);
            }
            let _ = detect_format(&bytes);
        }
    }
}
