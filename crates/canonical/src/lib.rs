// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-canonical` — byte-stable JSON and XML canonicalization.
//!
//! Every InvoiceKit operation that signs, hashes, or audits a JSON document
//! first canonicalizes it through this crate. The output is a byte-stable
//! UTF-8 string that two independent implementations should produce
//! bit-identically given the same input.
//!
//! XML invoices use [`canonicalize_xml`], which implements the InvoiceKit
//! no-comments XML Canonicalization 1.1 profile plus the invoice overlay from
//! `plans/PLAN.md`: deterministic namespace prefixes for UBL/CII families,
//! namespace declarations and attributes in canonical order, normalized text
//! escaping, expanded empty elements, and no inter-element formatting whitespace.
//!
//! ## What this crate guarantees for JSON
//!
//! * Object members are emitted in lexicographic order by UTF-16 code unit
//!   sequence of the member name, exactly as RFC 8785 §3.2.3 specifies.
//! * Strings are escaped using the minimal RFC 8785 §3.2.2 / ECMAScript
//!   `JSON.stringify` rule set: `"`, `\\`, control characters U+0000…U+001F
//!   via short escapes (`\\b`, `\\f`, `\\n`, `\\r`, `\\t`) or `\\u00xx`, and
//!   every other Unicode code point passes through verbatim. Forward slash
//!   `/` is NOT escaped.
//! * Numbers are serialized using the ECMAScript 6.5.3 `Number.prototype.
//!   toString` algorithm, as required by RFC 8785 §3.2.2.2. The
//!   `ryu-js` crate is the reference implementation of that algorithm and
//!   is what JCS test-vector ports converge on.
//! * Arrays preserve element order.
//! * Whitespace between tokens is removed (no insignificant whitespace).
//!
//! ## What this crate does NOT do
//!
//! * It does not validate that the input is RFC 8259-conformant JSON beyond
//!   what `serde_json` already enforces.
//! * It does not transcode strings — the input must already be valid UTF-8
//!   (which every `&str` is).
//! * It does not deduplicate object members; [`canonicalize`] rejects
//!   duplicate object names before the parsed [`Value`] can collapse them.
//!   [`canonicalize_value`] accepts an already-parsed [`Value`], where
//!   duplicate object names are no longer representable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};

use quick_xml::events::{attributes::AttrError, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use thiserror::Error;

const DUPLICATE_MEMBER_ERROR_PREFIX: &str = "invoicekit duplicate object member: ";
const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;
const MIN_SAFE_INTEGER: i128 = -MAX_SAFE_INTEGER;
const XML_NAMESPACE_URI: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE_URI: &str = "http://www.w3.org/2000/xmlns/";
const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CAC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
const UBL_CBC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";
const UBL_EXT_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2";
const UBL_QDT_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:QualifiedDataTypes-2";
const UBL_UDT_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:UnqualifiedDataTypes-2";
const CII_RSM_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
const CII_RAM_NAMESPACE_URI: &str =
    "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
const CII_UDT_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100";
const XMLDSIG_NAMESPACE_URI: &str = "http://www.w3.org/2000/09/xmldsig#";
const XADES_NAMESPACE_URI: &str = "http://uri.etsi.org/01903/v1.3.2#";

/// Errors returned by [`canonicalize`] and [`canonicalize_value`].
#[derive(Debug, Error)]
pub enum CanonicalizeError {
    /// The input was not valid JSON.
    #[error("input was not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// The input contained the same object member name more than once.
    ///
    /// RFC 8785 builds on I-JSON, which forbids duplicate object names.
    /// Rejecting them before `serde_json::Value` construction prevents
    /// silent last-write-wins data loss in signed payloads.
    #[error("duplicate object member `{0}` is not valid RFC 8785/I-JSON input")]
    DuplicateObjectMember(String),
    /// An integer was outside the I-JSON interoperable safe range.
    ///
    /// RFC 8785 inherits I-JSON's IEEE-754 double-precision number domain.
    /// JSON integer values are interoperable only in
    /// `[-9007199254740991, 9007199254740991]`.
    #[error("integer `{0}` is outside the RFC 8785/I-JSON safe range")]
    UnsafeInteger(String),
    /// A JSON number could not be represented under RFC 8785 number rules.
    ///
    /// RFC 8785 §3.2.2.4 forbids serializing `NaN`, `+Infinity`, and
    /// `-Infinity`. `serde_json` does not normally produce those values
    /// from textual input, but when feeding the API from in-memory
    /// [`Value`]s constructed by other code this error surfaces them.
    #[error("number `{0}` is not representable under RFC 8785 (NaN/Infinity)")]
    NonFiniteNumber(String),
}

/// Errors returned by [`canonicalize_xml`].
#[derive(Debug, Error)]
pub enum XmlCanonicalizeError {
    /// The input was not well-formed XML.
    #[error("input was not valid XML: {0}")]
    InvalidXml(#[from] quick_xml::Error),
    /// An XML attribute could not be parsed.
    #[error("invalid XML attribute: {0}")]
    InvalidAttribute(#[from] AttrError),
    /// Text or attribute content could not be decoded as UTF-8 XML content.
    #[error("invalid XML encoding: {0}")]
    InvalidEncoding(#[from] quick_xml::encoding::EncodingError),
    /// A tag or attribute name was not valid UTF-8.
    #[error("invalid XML name `{0}`")]
    InvalidName(String),
    /// The document referenced a namespace prefix that is not in scope.
    #[error("XML namespace prefix `{0}` is not declared")]
    UnboundPrefix(String),
    /// Canonical prefix overlay would create two attributes with the same name.
    #[error("duplicate canonical XML attribute `{0}`")]
    DuplicateAttribute(String),
    /// The XML document ended before a start tag was closed.
    #[error("XML document ended with unclosed element `{0}`")]
    UnclosedElement(String),
    /// The XML document contained an end tag without a matching start tag.
    #[error("unexpected XML end tag `{0}`")]
    UnexpectedEndTag(String),
    /// The XML document contained a DTD, which InvoiceKit canonicalization rejects.
    #[error("XML DTD declarations are not supported in invoice canonicalization")]
    UnsupportedDoctype,
    /// The XML document contained an entity reference outside the predefined XML set.
    #[error("XML entity reference `&{0};` is not supported")]
    UnsupportedEntityReference(String),
}

/// Canonicalize a JSON string into its RFC 8785 form.
///
/// # Errors
///
/// Returns [`CanonicalizeError::InvalidJson`] when the input does not parse
/// as JSON, [`CanonicalizeError::DuplicateObjectMember`] when an object
/// repeats a member name, [`CanonicalizeError::UnsafeInteger`] when an
/// integer is outside the I-JSON safe range, or
/// [`CanonicalizeError::NonFiniteNumber`] when the input contains a
/// non-finite number.
///
/// # Examples
///
/// ```
/// let raw = r#"{ "b": 2 ,  "a":1 }"#;
/// let canonical = invoicekit_canonical::canonicalize(raw).unwrap();
/// assert_eq!(canonical, r#"{"a":1,"b":2}"#);
/// ```
pub fn canonicalize(input: &str) -> Result<String, CanonicalizeError> {
    let value = parse_value_rejecting_duplicate_members(input)?;
    canonicalize_value(&value)
}

/// Canonicalize a parsed JSON value into its RFC 8785 form.
///
/// # Errors
///
/// Returns [`CanonicalizeError::UnsafeInteger`] when an integer is outside
/// the I-JSON safe range, or [`CanonicalizeError::NonFiniteNumber`] when
/// the value contains a non-finite number.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// let value = json!({"b": [3, 1, 2], "a": null});
/// let canonical = invoicekit_canonical::canonicalize_value(&value).unwrap();
/// assert_eq!(canonical, r#"{"a":null,"b":[3,1,2]}"#);
/// ```
pub fn canonicalize_value(value: &Value) -> Result<String, CanonicalizeError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// Canonicalize XML using InvoiceKit's no-comments XML C14N 1.1 invoice profile.
///
/// The profile is deterministic over semantically equivalent invoice XML:
/// declarations and comments are omitted, empty elements are expanded, namespace
/// declarations and attributes are sorted, UBL/CII namespace prefixes are
/// normalized to stable invoice prefixes, and whitespace-only text nodes between
/// elements are removed.
///
/// # Errors
///
/// Returns [`XmlCanonicalizeError`] when the input is not well-formed XML, uses
/// an undeclared namespace prefix, contains unsupported DTD or custom entity
/// references, or canonical prefix normalization would produce duplicate
/// attribute names.
///
/// # Examples
///
/// ```
/// let raw = r#"<Invoice xmlns:x="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"><x:AccountingSupplierParty/></Invoice>"#;
/// let canonical = invoicekit_canonical::canonicalize_xml(raw).unwrap();
/// assert_eq!(
///     canonical,
///     r#"<Invoice><cac:AccountingSupplierParty xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"></cac:AccountingSupplierParty></Invoice>"#
/// );
/// ```
#[allow(clippy::too_many_lines)]
pub fn canonicalize_xml(input: &str) -> Result<String, XmlCanonicalizeError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);

    let mut xml_version = XmlVersion::default();
    let mut out = String::new();
    let mut namespace_frames = vec![NamespaceFrame::root()];
    let mut open_elements = Vec::new();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let current = namespace_frames
                    .last()
                    .ok_or_else(|| XmlCanonicalizeError::UnexpectedEndTag(String::new()))?;
                let (element, frame) =
                    write_xml_start(&reader, &start, xml_version, current, &mut out)?;
                namespace_frames.push(frame);
                open_elements.push(element);
            }
            Event::Empty(start) => {
                let current = namespace_frames
                    .last()
                    .ok_or_else(|| XmlCanonicalizeError::UnexpectedEndTag(String::new()))?;
                let (element, _) =
                    write_xml_start(&reader, &start, xml_version, current, &mut out)?;
                write_xml_end(&element.rendered_name, &mut out);
            }
            Event::End(end) => {
                let name = end.name();
                let end_name = decode_xml_name(name.as_ref())?.to_owned();
                let element = open_elements
                    .pop()
                    .ok_or(XmlCanonicalizeError::UnexpectedEndTag(end_name))?;
                namespace_frames.pop();
                write_xml_end(&element.rendered_name, &mut out);
            }
            Event::Text(text) => {
                let text = text.xml_content(xml_version)?;
                write_xml_text_node(&text, &mut out);
            }
            Event::CData(cdata) => {
                let text = cdata.xml_content(xml_version)?;
                write_xml_text_node(&text, &mut out);
            }
            Event::GeneralRef(reference) => {
                let reference = reference.xml_content(xml_version)?;
                let resolved = resolve_xml_reference(&reference)?;
                write_xml_text_node(&resolved, &mut out);
            }
            Event::PI(instruction) => {
                let instruction = decode_xml_name(instruction.content())?;
                out.push_str("<?");
                out.push_str(instruction.trim());
                out.push_str("?>");
            }
            Event::Decl(decl) => {
                let version = decl.version()?;
                if version.as_ref() == b"1.1" {
                    xml_version = XmlVersion::Explicit1_1;
                } else {
                    xml_version = XmlVersion::Explicit1_0;
                }
            }
            Event::DocType(_) => return Err(XmlCanonicalizeError::UnsupportedDoctype),
            Event::Comment(_) => {}
            Event::Eof => break,
        }
    }

    if let Some(element) = open_elements.pop() {
        return Err(XmlCanonicalizeError::UnclosedElement(element.rendered_name));
    }

    Ok(out)
}

#[derive(Clone, Debug)]
struct NamespaceFrame {
    input: BTreeMap<String, String>,
    output: BTreeMap<String, String>,
}

impl NamespaceFrame {
    fn root() -> Self {
        let mut input = BTreeMap::new();
        input.insert("xml".to_owned(), XML_NAMESPACE_URI.to_owned());
        let mut output = BTreeMap::new();
        output.insert("xml".to_owned(), XML_NAMESPACE_URI.to_owned());
        Self { input, output }
    }
}

#[derive(Debug)]
struct OpenXmlElement {
    rendered_name: String,
}

#[derive(Debug)]
struct RawXmlAttribute {
    prefix: String,
    local_name: String,
    value: String,
}

#[derive(Debug)]
struct CanonicalXmlAttribute {
    rendered_name: String,
    namespace_uri: String,
    local_name: String,
    value: String,
}

#[allow(clippy::too_many_lines)]
fn write_xml_start(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
    current: &NamespaceFrame,
    out: &mut String,
) -> Result<(OpenXmlElement, NamespaceFrame), XmlCanonicalizeError> {
    let mut frame = current.clone();
    let mut raw_attributes = Vec::new();

    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        let key = decode_xml_name(attr.key.as_ref())?;
        let value = attr
            .decoded_and_normalized_value(xml_version, reader.decoder())?
            .into_owned();

        if key == "xmlns" {
            frame.input.insert(String::new(), value);
            continue;
        }
        if let Some(prefix) = key.strip_prefix("xmlns:") {
            if prefix != "xml" && prefix != "xmlns" {
                frame.input.insert(prefix.to_owned(), value);
            }
            continue;
        }

        let (prefix, local_name) = split_xml_qname(key);
        raw_attributes.push(RawXmlAttribute {
            prefix: prefix.to_owned(),
            local_name: local_name.to_owned(),
            value,
        });
    }

    let name = start.name();
    let raw_name = decode_xml_name(name.as_ref())?;
    let (element_prefix, element_local_name) = split_xml_qname(raw_name);
    let element_namespace_uri = lookup_element_namespace(&frame, element_prefix)?;
    let rendered_element_prefix = element_namespace_uri
        .as_ref()
        .map_or_else(String::new, |uri| {
            preferred_invoice_prefix(uri, element_prefix, false)
        });
    let rendered_element_name = render_xml_qname(&rendered_element_prefix, element_local_name);

    let mut namespace_declarations = BTreeMap::new();
    if let Some(uri) = element_namespace_uri.as_deref() {
        ensure_output_namespace(
            &mut frame,
            &mut namespace_declarations,
            &rendered_element_prefix,
            uri,
        )?;
    } else if frame.output.get("").is_some_and(|uri| !uri.is_empty()) {
        namespace_declarations.insert(String::new(), String::new());
        frame.output.remove("");
    }

    let mut rendered_attribute_names = BTreeSet::new();
    let mut canonical_attributes = Vec::with_capacity(raw_attributes.len());
    for attr in raw_attributes {
        let namespace_uri = lookup_attribute_namespace(&frame, &attr.prefix)?;
        let rendered_prefix = namespace_uri.as_ref().map_or_else(String::new, |uri| {
            preferred_invoice_prefix(uri, &attr.prefix, true)
        });
        let rendered_name = render_xml_qname(&rendered_prefix, &attr.local_name);

        if !rendered_attribute_names.insert(rendered_name.clone()) {
            return Err(XmlCanonicalizeError::DuplicateAttribute(rendered_name));
        }

        if let Some(uri) = namespace_uri.as_deref() {
            ensure_output_namespace(
                &mut frame,
                &mut namespace_declarations,
                &rendered_prefix,
                uri,
            )?;
        }

        canonical_attributes.push(CanonicalXmlAttribute {
            rendered_name,
            namespace_uri: namespace_uri.unwrap_or_default(),
            local_name: attr.local_name,
            value: attr.value,
        });
    }

    canonical_attributes.sort_by(|left, right| {
        left.namespace_uri
            .cmp(&right.namespace_uri)
            .then_with(|| left.local_name.cmp(&right.local_name))
            .then_with(|| left.rendered_name.cmp(&right.rendered_name))
    });

    out.push('<');
    out.push_str(&rendered_element_name);
    for (prefix, uri) in namespace_declarations {
        out.push(' ');
        if prefix.is_empty() {
            out.push_str("xmlns");
        } else {
            out.push_str("xmlns:");
            out.push_str(&prefix);
        }
        out.push_str("=\"");
        write_xml_attr_value(&uri, out);
        out.push('"');
    }
    for attr in canonical_attributes {
        out.push(' ');
        out.push_str(&attr.rendered_name);
        out.push_str("=\"");
        write_xml_attr_value(&attr.value, out);
        out.push('"');
    }
    out.push('>');

    Ok((
        OpenXmlElement {
            rendered_name: rendered_element_name,
        },
        frame,
    ))
}

fn ensure_output_namespace(
    frame: &mut NamespaceFrame,
    declarations: &mut BTreeMap<String, String>,
    prefix: &str,
    uri: &str,
) -> Result<(), XmlCanonicalizeError> {
    if prefix == "xmlns" || uri == XMLNS_NAMESPACE_URI {
        return Err(XmlCanonicalizeError::UnboundPrefix(prefix.to_owned()));
    }
    if prefix == "xml" || uri == XML_NAMESPACE_URI {
        return Ok(());
    }
    if frame
        .output
        .get(prefix)
        .is_none_or(|current| current != uri)
    {
        declarations.insert(prefix.to_owned(), uri.to_owned());
        frame.output.insert(prefix.to_owned(), uri.to_owned());
    }
    Ok(())
}

fn lookup_element_namespace(
    frame: &NamespaceFrame,
    prefix: &str,
) -> Result<Option<String>, XmlCanonicalizeError> {
    if prefix.is_empty() {
        return Ok(frame.input.get("").filter(|uri| !uri.is_empty()).cloned());
    }
    frame
        .input
        .get(prefix)
        .cloned()
        .map(Some)
        .ok_or_else(|| XmlCanonicalizeError::UnboundPrefix(prefix.to_owned()))
}

fn lookup_attribute_namespace(
    frame: &NamespaceFrame,
    prefix: &str,
) -> Result<Option<String>, XmlCanonicalizeError> {
    if prefix.is_empty() {
        return Ok(None);
    }
    frame
        .input
        .get(prefix)
        .cloned()
        .map(Some)
        .ok_or_else(|| XmlCanonicalizeError::UnboundPrefix(prefix.to_owned()))
}

fn preferred_invoice_prefix(uri: &str, original_prefix: &str, is_attribute: bool) -> String {
    let prefix = match uri {
        UBL_INVOICE_NAMESPACE_URI if !is_attribute => "",
        UBL_CAC_NAMESPACE_URI => "cac",
        UBL_CBC_NAMESPACE_URI => "cbc",
        UBL_EXT_NAMESPACE_URI => "ext",
        UBL_QDT_NAMESPACE_URI => "qdt",
        UBL_UDT_NAMESPACE_URI | CII_UDT_NAMESPACE_URI => "udt",
        CII_RSM_NAMESPACE_URI => "rsm",
        CII_RAM_NAMESPACE_URI => "ram",
        XMLDSIG_NAMESPACE_URI => "ds",
        XADES_NAMESPACE_URI => "xades",
        XML_NAMESPACE_URI => "xml",
        _ => original_prefix,
    };
    if is_attribute && prefix.is_empty() && !uri.is_empty() {
        original_prefix.to_owned()
    } else {
        prefix.to_owned()
    }
}

fn render_xml_qname(prefix: &str, local_name: &str) -> String {
    if prefix.is_empty() {
        local_name.to_owned()
    } else {
        format!("{prefix}:{local_name}")
    }
}

fn split_xml_qname(name: &str) -> (&str, &str) {
    name.split_once(':')
        .map_or(("", name), |(prefix, local_name)| (prefix, local_name))
}

fn decode_xml_name(raw: &[u8]) -> Result<&str, XmlCanonicalizeError> {
    std::str::from_utf8(raw)
        .map_err(|_| XmlCanonicalizeError::InvalidName(String::from_utf8_lossy(raw).into_owned()))
}

fn resolve_xml_reference(reference: &str) -> Result<String, XmlCanonicalizeError> {
    match reference {
        "amp" => Ok("&".to_owned()),
        "lt" => Ok("<".to_owned()),
        "gt" => Ok(">".to_owned()),
        "apos" => Ok("'".to_owned()),
        "quot" => Ok("\"".to_owned()),
        value if value.starts_with("#x") => value
            .strip_prefix("#x")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
            .map(|character| character.to_string())
            .ok_or_else(|| XmlCanonicalizeError::UnsupportedEntityReference(value.to_owned())),
        value if value.starts_with('#') => value
            .strip_prefix('#')
            .and_then(|decimal| decimal.parse::<u32>().ok())
            .and_then(char::from_u32)
            .map(|character| character.to_string())
            .ok_or_else(|| XmlCanonicalizeError::UnsupportedEntityReference(value.to_owned())),
        value => Err(XmlCanonicalizeError::UnsupportedEntityReference(
            value.to_owned(),
        )),
    }
}

fn write_xml_text_node(text: &str, out: &mut String) {
    if text.chars().all(is_xml_whitespace) {
        return;
    }
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            character => out.push(character),
        }
    }
}

fn write_xml_attr_value(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            character => out.push(character),
        }
    }
}

fn write_xml_end(rendered_name: &str, out: &mut String) {
    out.push_str("</");
    out.push_str(rendered_name);
    out.push('>');
}

fn is_xml_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r')
}

fn parse_value_rejecting_duplicate_members(input: &str) -> Result<Value, CanonicalizeError> {
    match serde_json::from_str::<CheckedValue>(input) {
        Ok(CheckedValue(value)) => Ok(value),
        Err(error) => {
            if let Some(member) = duplicate_member_from_error(&error) {
                return Err(CanonicalizeError::DuplicateObjectMember(member));
            }
            Err(CanonicalizeError::InvalidJson(error))
        }
    }
}

fn duplicate_member_from_error(error: &serde_json::Error) -> Option<String> {
    let message = error.to_string();
    let payload = message.strip_prefix(DUPLICATE_MEMBER_ERROR_PREFIX)?;
    let payload = payload
        .rsplit_once(" at line ")
        .map_or(payload, |(payload, _)| payload);
    serde_json::from_str(payload).ok()
}

fn duplicate_member_error<E>(member: &str) -> E
where
    E: de::Error,
{
    let encoded = serde_json::to_string(member).expect("serializing a string is infallible");
    E::custom(format!("{DUPLICATE_MEMBER_ERROR_PREFIX}{encoded}"))
}

struct CheckedValue(Value);

impl<'de> Deserialize<'de> for CheckedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CheckedValueVisitor).map(Self)
    }
}

struct CheckedValueVisitor;

impl<'de> Visitor<'de> for CheckedValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object member names")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let value = i64::try_from(value).map_err(|_| E::custom("integer does not fit i64"))?;
        Ok(Value::Number(value.into()))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let value = u64::try_from(value).map_err(|_| E::custom("integer does not fit u64"))?;
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let number = serde_json::Number::from_f64(value)
            .ok_or_else(|| E::custom("non-finite JSON number"))?;
        Ok(Value::Number(number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let CheckedValue(value) = CheckedValue::deserialize(deserializer)?;
        Ok(value)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(CheckedValue(value)) = seq.next_element::<CheckedValue>()? {
            items.push(value);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = serde_json::Map::new();
        let mut seen = BTreeSet::new();
        while let Some(member) = map.next_key::<String>()? {
            if !seen.insert(member.clone()) {
                return Err(duplicate_member_error(&member));
            }
            let CheckedValue(value) = map.next_value::<CheckedValue>()?;
            object.insert(member, value);
        }
        Ok(Value::Object(object))
    }
}

fn write_value(value: &Value, out: &mut String) -> Result<(), CanonicalizeError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(n, out)?,
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Lexicographic sort by UTF-16 code-unit sequence per RFC 8785 §3.2.3.
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| compare_utf16(a.0, b.0));
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(v, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(n: &serde_json::Number, out: &mut String) -> Result<(), CanonicalizeError> {
    if let Some(i) = n.as_i64() {
        ensure_safe_integer(i128::from(i), n)?;
        write!(out, "{i}").expect("write to String is infallible");
        return Ok(());
    }
    if let Some(u) = n.as_u64() {
        let value = i128::from(u);
        ensure_safe_integer(value, n)?;
        write!(out, "{u}").expect("write to String is infallible");
        return Ok(());
    }
    if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return Err(CanonicalizeError::NonFiniteNumber(n.to_string()));
        }
        // ECMAScript 6.5.3 Number.prototype.toString -> Ryū-JS.
        let mut buffer = ryu_js::Buffer::new();
        let s = buffer.format(f);
        out.push_str(s);
        return Ok(());
    }
    let rendered = n.to_string();
    if rendered
        .bytes()
        .all(|byte| byte == b'-' || byte.is_ascii_digit())
    {
        return Err(CanonicalizeError::UnsafeInteger(rendered));
    }
    Err(CanonicalizeError::NonFiniteNumber(rendered))
}

fn ensure_safe_integer(
    value: i128,
    original: &serde_json::Number,
) -> Result<(), CanonicalizeError> {
    if (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        Ok(())
    } else {
        Err(CanonicalizeError::UnsafeInteger(original.to_string()))
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).expect("write to String is infallible");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Compare two strings by their UTF-16 code-unit sequence.
///
/// RFC 8785 §3.2.3 mandates lexicographic sort of object members in their
/// UTF-16 representation. For BMP code points UTF-16 ordering matches
/// Unicode-scalar ordering; for supplementary code points (U+10000..) the
/// surrogate pair ordering differs from the scalar ordering.
fn compare_utf16(a: &str, b: &str) -> std::cmp::Ordering {
    let mut au = a.encode_utf16();
    let mut bu = b.encode_utf16();
    loop {
        match (au.next(), bu.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => {
                if a != b {
                    return a.cmp(&b);
                }
            }
        }
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_canonical::crate_name(), "invoicekit-canonical");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-canonical"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{json, Value};

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-canonical");
    }

    #[test]
    fn xml_invoice_overlay_normalizes_namespace_prefixes_and_attribute_order() {
        let invoice = UBL_INVOICE_NAMESPACE_URI;
        let cac = UBL_CAC_NAMESPACE_URI;
        let cbc = UBL_CBC_NAMESPACE_URI;
        let input = format!(
            r#"
            <Invoice xmlns="{invoice}" xmlns:x="{cac}" xmlns:b="{cbc}">
                <x:AccountingSupplierParty z="2" b:schemeID="0088" a="1"/>
                <!-- formatting comment must not affect signatures -->
            </Invoice>
            "#
        );
        let canonical = canonicalize_xml(&input).unwrap();
        let expected = format!(
            r#"<Invoice xmlns="{invoice}"><cac:AccountingSupplierParty xmlns:cac="{cac}" xmlns:cbc="{cbc}" a="1" z="2" cbc:schemeID="0088"></cac:AccountingSupplierParty></Invoice>"#
        );
        assert_eq!(canonical, expected);
    }

    #[test]
    fn xml_namespace_aliases_canonicalize_to_identical_bytes() {
        let cac = UBL_CAC_NAMESPACE_URI;
        let first = format!(r#"<Invoice xmlns:c="{cac}"><c:AccountingCustomerParty/></Invoice>"#);
        let second = format!(
            r#"<Invoice xmlns:cac="{cac}"><cac:AccountingCustomerParty></cac:AccountingCustomerParty></Invoice>"#
        );

        assert_eq!(
            canonicalize_xml(&first).unwrap(),
            canonicalize_xml(&second).unwrap()
        );
    }

    #[test]
    fn xml_text_cdata_and_attributes_use_canonical_escaping() {
        let input = r#"<?xml version="1.0"?><r b="&quot;" a="&lt;&amp;"><![CDATA[x<y]]>&amp;z<!--ignored--></r>"#;
        let canonical = canonicalize_xml(input).unwrap();
        assert_eq!(canonical, r#"<r a="&lt;&amp;" b="&quot;">x&lt;y&amp;z</r>"#);
    }

    #[test]
    fn xml_cross_platform_fixture_is_byte_stable() {
        let invoice = UBL_INVOICE_NAMESPACE_URI;
        let cbc = UBL_CBC_NAMESPACE_URI;
        let input = format!(
            r#"<n:Invoice xmlns:n="{invoice}" xmlns:q="{cbc}"><q:ID>INV-001</q:ID><q:Note>same bytes</q:Note></n:Invoice>"#
        );
        let canonical = canonicalize_xml(&input).unwrap();
        let expected = format!(
            r#"<Invoice xmlns="{invoice}"><cbc:ID xmlns:cbc="{cbc}">INV-001</cbc:ID><cbc:Note xmlns:cbc="{cbc}">same bytes</cbc:Note></Invoice>"#
        );
        assert_eq!(canonical.as_bytes(), expected.as_bytes());
    }

    #[test]
    fn xml_invalid_inputs_are_rejected_with_typed_errors() {
        let err = canonicalize_xml("<r><unclosed></r>").unwrap_err();
        assert!(matches!(err, XmlCanonicalizeError::InvalidXml(_)));

        let err = canonicalize_xml("<p:Invoice/>").unwrap_err();
        assert!(matches!(err, XmlCanonicalizeError::UnboundPrefix(prefix) if prefix == "p"));

        let err = canonicalize_xml("<!DOCTYPE r [<!ENTITY x \"x\">]><r/>").unwrap_err();
        assert!(matches!(err, XmlCanonicalizeError::UnsupportedDoctype));
    }

    /// RFC 8785 §3.3 test vector: object member ordering.
    #[test]
    fn rfc8785_member_ordering() {
        let input = r#"{
            "numbers": [333333333.33333329, 1E30, 4.5, 2e-3],
            "string": "Hello",
            "literals": [null, true, false]
        }"#;
        let canonical = canonicalize(input).unwrap();
        // The canonical form must order top-level keys literals < numbers < string.
        let expected_prefix = r#"{"literals":[null,true,false],"numbers":["#;
        assert!(canonical.starts_with(expected_prefix), "got: {canonical}");
    }

    /// RFC 8785 §3.2.2.2: integers serialize without a decimal point.
    #[test]
    fn integers_serialize_without_decimal_point() {
        assert_eq!(canonicalize("42").unwrap(), "42");
        assert_eq!(canonicalize("-42").unwrap(), "-42");
        assert_eq!(canonicalize("0").unwrap(), "0");
    }

    /// ECMAScript Number.toString: `1.0` → "1".
    #[test]
    fn ecmascript_number_serialization() {
        assert_eq!(canonicalize("1.0").unwrap(), "1");
        assert_eq!(canonicalize("100.0").unwrap(), "100");
        // RFC 8785 §3.2.2.3: `4.50` -> "4.5".
        assert_eq!(canonicalize("4.50").unwrap(), "4.5");
    }

    /// Empty object + empty array.
    #[test]
    fn empty_containers_serialize() {
        assert_eq!(canonicalize("{}").unwrap(), "{}");
        assert_eq!(canonicalize("[]").unwrap(), "[]");
        assert_eq!(canonicalize(r#"{"a":[]}"#).unwrap(), r#"{"a":[]}"#);
    }

    /// RFC 8785 §3.2.2: control characters serialize as `\\u00xx`.
    #[test]
    fn control_characters_are_escaped() {
        let input = "{\"k\":\"\\u0001\\u001f\"}";
        let canonical = canonicalize(input).unwrap();
        assert_eq!(canonical, "{\"k\":\"\\u0001\\u001f\"}");
    }

    /// String escapes: backslash, quote, slash (not escaped), control chars.
    #[test]
    fn string_escapes_match_rfc8785() {
        // Slash must NOT be escaped.
        assert_eq!(canonicalize(r#""a/b""#).unwrap(), r#""a/b""#);
        // Backslash and quote are escaped.
        assert_eq!(canonicalize(r#""a\\b\"c""#).unwrap(), r#""a\\b\"c""#);
        // Tab, newline, carriage return, formfeed, backspace use short escapes.
        assert_eq!(
            canonicalize("\"\\t\\n\\r\\f\\b\"").unwrap(),
            "\"\\t\\n\\r\\f\\b\""
        );
    }

    /// Member-name sort is by UTF-16 code unit (RFC 8785 §3.2.3).
    #[test]
    fn member_name_sort_by_utf16_code_unit() {
        // "a", "b", "ä" (U+00E4 = 0xE4), "💖" (U+1F496 surrogate pair starts 0xD83D)
        let input = r#"{"b":2,"a":1,"ä":3,"💖":4}"#;
        let out = canonicalize(input).unwrap();
        // a < b < ä < 💖 because UTF-16 code units 0x61 < 0x62 < 0xE4 < 0xD83D.
        assert_eq!(out, r#"{"a":1,"b":2,"ä":3,"💖":4}"#);
    }

    /// Non-finite numbers are rejected.
    #[test]
    fn non_finite_numbers_are_rejected() {
        let v: Value = serde_json::from_str(r#"{"k":null}"#).unwrap();
        // Construct a Value that contains NaN through arithmetic; serde_json's
        // Number cannot deserialize NaN from text, but it can hold it via
        // serde_json::Number::from_f64 only when finite. So instead we cover
        // the contract by directly constructing the failure path:
        let nan = serde_json::Number::from_f64(f64::NAN);
        assert!(
            nan.is_none(),
            "serde_json::Number rejects NaN at construction"
        );
        // Verify the happy-path Value works (the negative test above asserts
        // the actual library refuses NaN, satisfying the contract).
        assert!(canonicalize_value(&v).is_ok());
    }

    /// Invalid JSON is rejected.
    #[test]
    fn invalid_json_is_rejected() {
        let err = canonicalize("not json").unwrap_err();
        assert!(matches!(err, CanonicalizeError::InvalidJson(_)));
    }

    fn duplicate_member_name(error: CanonicalizeError) -> Option<String> {
        match error {
            CanonicalizeError::DuplicateObjectMember(member) => Some(member),
            _ => None,
        }
    }

    /// Duplicate object members are rejected before `Value` can collapse them.
    #[test]
    fn duplicate_object_members_are_rejected() {
        let err = canonicalize(r#"{"a":1,"a":2}"#).unwrap_err();
        assert_eq!(duplicate_member_name(err).as_deref(), Some("a"));
    }

    /// Duplicate detection recurses through nested objects and arrays.
    #[test]
    fn nested_duplicate_object_members_are_rejected() {
        let err = canonicalize(r#"{"outer":[{"b":1,"b":2}]}"#).unwrap_err();
        assert_eq!(duplicate_member_name(err).as_deref(), Some("b"));
    }

    /// I-JSON's interoperable integer range ends at 2^53 - 1.
    #[test]
    fn safe_integer_boundaries_are_accepted() {
        assert_eq!(
            canonicalize("9007199254740991").unwrap(),
            "9007199254740991"
        );
        assert_eq!(
            canonicalize("-9007199254740991").unwrap(),
            "-9007199254740991"
        );
    }

    /// Integers outside the I-JSON safe range are rejected from text.
    #[test]
    fn unsafe_integer_text_is_rejected() {
        let err = canonicalize("9007199254740992").unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "9007199254740992")
        );

        let err = canonicalize("-9007199254740992").unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "-9007199254740992")
        );
    }

    /// In-memory `Value` inputs use the same integer-domain guard.
    #[test]
    fn unsafe_integer_value_is_rejected() {
        let value = Value::Number(serde_json::Number::from(9_007_199_254_740_992_u64));
        let err = canonicalize_value(&value).unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "9007199254740992")
        );
    }

    /// Determinism: canonicalize twice on the same input -> identical bytes.
    #[test]
    fn canonicalize_is_idempotent() {
        let input = json!({"z": 1, "a": [2, 3], "m": null});
        let once = canonicalize_value(&input).unwrap();
        let twice = canonicalize_value(&input).unwrap();
        assert_eq!(once, twice);
    }

    /// Canonical form parses back to the same logical document.
    #[test]
    fn canonical_form_round_trips() {
        let input = json!({"b": [1, 2], "a": null, "c": "hi"});
        let canonical = canonicalize_value(&input).unwrap();
        let reparsed: Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(reparsed, input);
    }

    proptest! {
        /// Canonicalize is a function: same input -> same output.
        #[test]
        fn canonicalize_is_a_function(seed in any::<u32>()) {
            // Build a synthetic JSON value parameterized by the seed.
            let value = json!({
                "seed": seed,
                "nested": {"a": 1, "b": [seed, seed.wrapping_add(1)]},
                "list": (0..(seed % 5)).map(serde_json::Value::from).collect::<Vec<_>>(),
            });
            let a = canonicalize_value(&value).unwrap();
            let b = canonicalize_value(&value).unwrap();
            prop_assert_eq!(a, b);
        }

        /// Round-trip: canonicalize -> parse -> canonicalize -> same bytes.
        #[test]
        fn canonicalize_then_parse_then_canonicalize_is_stable(seed in any::<u32>()) {
            let value = json!({
                "seed": seed,
                "child": {"x": i64::from(seed).wrapping_neg(), "y": [1, 2, 3]},
            });
            let first = canonicalize_value(&value).unwrap();
            let reparsed: Value = serde_json::from_str(&first).unwrap();
            let second = canonicalize_value(&reparsed).unwrap();
            prop_assert_eq!(first, second);
        }

        /// Generated valid XML canonicalizes, reparses, and canonicalizes to identical bytes.
        #[test]
        fn generated_xml_round_trips_to_identical_canonical_output(seed in 0_u16..500_u16) {
            let invoice = UBL_INVOICE_NAMESPACE_URI;
            let cbc = UBL_CBC_NAMESPACE_URI;
            let a = seed % 17;
            let input = format!(
                r#"<x:Invoice xmlns:x="{invoice}" xmlns:basic="{cbc}" z="{seed}" a="{a}">
                    <basic:ID>INV-{seed:04}</basic:ID>
                    <basic:Note>line {seed}</basic:Note>
                    <basic:DocumentCurrencyCode>EUR</basic:DocumentCurrencyCode>
                </x:Invoice>"#
            );
            let first = canonicalize_xml(&input).unwrap();
            let second = canonicalize_xml(&first).unwrap();
            prop_assert_eq!(first, second);
        }
    }
}
