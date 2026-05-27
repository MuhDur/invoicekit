// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! OASIS UBL 2.1 XSD validation harness.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use fastxml::error::StructuredError;
use fastxml::schema::{CompiledSchema, FileFetcher, XmlSchemaValidationContext};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use crate::{
    decode_xml_name, read_element_end, read_element_start, split_qname, top_level_coverage,
    NamespaceFrame, ParsedElement, UblDocumentKind, UblError, UBL_2_1_CREDIT_NOTE_SCHEMA_URI,
    UBL_2_1_INVOICE_SCHEMA_URI, UBL_CAC_NAMESPACE_URI, UBL_CBC_NAMESPACE_URI,
    UBL_CREDIT_NOTE_NAMESPACE_URI, UBL_EXT_NAMESPACE_URI, UBL_INVOICE_NAMESPACE_URI,
};

const SCHEMA_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/ubl-2.1");
const INVOICE_SCHEMA_FILE: &str = "xsd/maindoc/UBL-Invoice-2.1.xsd";
const CREDIT_NOTE_SCHEMA_FILE: &str = "xsd/maindoc/UBL-CreditNote-2.1.xsd";
const INVOICEKIT_EXTENSION_SCHEMA_FILE: &str = "invoicekit-extension-v1.xsd";

static INVOICE_SCHEMA: OnceLock<Result<Arc<CompiledSchema>, String>> = OnceLock::new();
static CREDIT_NOTE_SCHEMA: OnceLock<Result<Arc<CompiledSchema>, String>> = OnceLock::new();

/// One serializer fixture pinned by the OASIS UBL 2.1 schema harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UblSchemaValidatedFixture {
    /// Stable test fixture name.
    pub name: &'static str,
    /// UBL document kind validated by the fixture.
    pub document_kind: UblDocumentKind,
    /// Vendored OASIS schema used for the validation.
    pub schema_uri: &'static str,
}

/// Serializer fixtures that currently pass the vendored OASIS UBL 2.1 XSDs.
pub const OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES: &[UblSchemaValidatedFixture] = &[
    UblSchemaValidatedFixture {
        name: "format-ubl serializer invoice fixture seed=20",
        document_kind: UblDocumentKind::Invoice,
        schema_uri: UBL_2_1_INVOICE_SCHEMA_URI,
    },
    UblSchemaValidatedFixture {
        name: "format-ubl serializer credit-note fixture seed=21",
        document_kind: UblDocumentKind::CreditNote,
        schema_uri: UBL_2_1_CREDIT_NOTE_SCHEMA_URI,
    },
];

/// A single finding returned by the OASIS UBL 2.1 schema validator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UblSchemaValidationFinding {
    /// Severity as reported by the schema validator.
    pub level: String,
    /// Human-readable validation message.
    pub message: String,
    /// XPath-like element path, when the validator can provide one.
    pub element_path: Option<String>,
    /// One-indexed line number, when available.
    pub line: Option<usize>,
    /// One-indexed column number, when available.
    pub column: Option<usize>,
}

/// Result of checking one UBL XML document against the vendored OASIS schemas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UblSchemaValidationReport {
    /// UBL document kind inferred from the root element.
    pub document_kind: UblDocumentKind,
    /// Official OASIS schema URI represented by the vendored schema.
    pub schema_uri: &'static str,
    /// Local vendored schema path, relative to `crates/format-ubl/schemas/ubl-2.1`.
    pub schema_file: &'static str,
    /// Schema validation findings. Empty means the XML is schema-valid.
    pub findings: Vec<UblSchemaValidationFinding>,
}

impl UblSchemaValidationReport {
    /// Returns true when the schema validator found no issues.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Validate UBL XML against the vendored OASIS UBL 2.1 Invoice/CreditNote XSDs.
///
/// This harness is intentionally offline: it resolves imports from
/// `crates/format-ubl/schemas/ubl-2.1` and never fetches schemas over the
/// network at test or runtime. It also applies the OASIS maindoc top-level
/// sequence order because the pure-Rust XSD validator accepts some globally
/// declared UBL elements in the wrong document sequence.
///
/// # Errors
///
/// Returns [`UblError`] when the XML is not well formed, the root is not a UBL
/// `Invoice` or `CreditNote`, or the vendored schema corpus cannot be loaded.
///
/// # Examples
///
/// ```
/// # use invoicekit_format_ubl::validate_oasis_ubl_2_1_schema;
/// let report = validate_oasis_ubl_2_1_schema("<Order/>").unwrap_err();
/// assert!(report.to_string().contains("unsupported UBL root"));
/// ```
pub fn validate_oasis_ubl_2_1_schema(xml: &str) -> Result<UblSchemaValidationReport, UblError> {
    let document_kind = detect_document_kind(xml)?;
    let doc = fastxml::parse(xml.as_bytes()).map_err(|err| UblError::SchemaHarness {
        operation: "parse XML for OASIS UBL 2.1 schema validation",
        message: err.to_string(),
    })?;
    let ctx = schema_context(document_kind)?;
    let mut findings: Vec<_> = ctx
        .validate(&doc)
        .map_err(|err| UblError::SchemaHarness {
            operation: "validate XML against OASIS UBL 2.1 schema",
            message: err.to_string(),
        })?
        .iter()
        .map(finding_from_fastxml)
        .collect();
    findings.extend(top_level_sequence_findings(xml, document_kind)?);

    Ok(UblSchemaValidationReport {
        document_kind,
        schema_uri: schema_uri(document_kind),
        schema_file: schema_file(document_kind),
        findings,
    })
}

fn schema_context(document_kind: UblDocumentKind) -> Result<XmlSchemaValidationContext, UblError> {
    let schema = match document_kind {
        UblDocumentKind::Invoice => INVOICE_SCHEMA.get_or_init(|| compile_schema(document_kind)),
        UblDocumentKind::CreditNote => {
            CREDIT_NOTE_SCHEMA.get_or_init(|| compile_schema(document_kind))
        }
    }
    .clone()
    .map_err(|message| UblError::SchemaHarness {
        operation: "compile vendored OASIS UBL 2.1 schema",
        message,
    })?;
    Ok(XmlSchemaValidationContext::from_arc(schema))
}

fn compile_schema(document_kind: UblDocumentKind) -> Result<Arc<CompiledSchema>, String> {
    let path = schema_path(document_kind);
    let content = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let extension_path = Path::new(SCHEMA_ROOT).join(INVOICEKIT_EXTENSION_SCHEMA_FILE);
    let extension_content = std::fs::read(&extension_path)
        .map_err(|err| format!("{}: {err}", extension_path.display()))?;
    let base_uri = format!("file://{}", path.display());
    let fetcher = FileFetcher::new();
    let extension_uri = format!("file://{}", extension_path.display());
    let schema = fastxml::schema::parse_xsd_with_imports_multiple(
        &[(&base_uri, &content), (&extension_uri, &extension_content)],
        &fetcher,
    )
    .map_err(|err| err.to_string())?;
    Ok(Arc::new(schema))
}

fn schema_path(document_kind: UblDocumentKind) -> PathBuf {
    Path::new(SCHEMA_ROOT).join(schema_file(document_kind))
}

const fn schema_file(document_kind: UblDocumentKind) -> &'static str {
    match document_kind {
        UblDocumentKind::Invoice => INVOICE_SCHEMA_FILE,
        UblDocumentKind::CreditNote => CREDIT_NOTE_SCHEMA_FILE,
    }
}

const fn schema_uri(document_kind: UblDocumentKind) -> &'static str {
    match document_kind {
        UblDocumentKind::Invoice => UBL_2_1_INVOICE_SCHEMA_URI,
        UblDocumentKind::CreditNote => UBL_2_1_CREDIT_NOTE_SCHEMA_URI,
    }
}

fn detect_document_kind(xml: &str) -> Result<UblDocumentKind, UblError> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event()? {
            Event::Start(start) | Event::Empty(start) => {
                let name = decode_xml_name(start.name().as_ref())?;
                let (_, local_name) = split_qname(&name);
                return match local_name {
                    "Invoice" => Ok(UblDocumentKind::Invoice),
                    "CreditNote" => Ok(UblDocumentKind::CreditNote),
                    other => Err(UblError::UnsupportedRoot(other.to_owned())),
                };
            }
            Event::Eof => return Err(UblError::MissingElement("Invoice|CreditNote")),
            Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::Comment(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::GeneralRef(_)
            | Event::End(_) => {}
        }
    }
}

fn finding_from_fastxml(error: &StructuredError) -> UblSchemaValidationFinding {
    UblSchemaValidationFinding {
        level: error.level.to_string(),
        message: error.message.clone(),
        element_path: error.element_path().map(ToOwned::to_owned),
        line: error.line(),
        column: error.column(),
    }
}

fn top_level_sequence_findings(
    xml: &str,
    document_kind: UblDocumentKind,
) -> Result<Vec<UblSchemaValidationFinding>, UblError> {
    let children = top_level_children(xml)?;
    let coverage = top_level_coverage(document_kind);
    let mut findings = Vec::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut previous_index = 0;

    for child in &children {
        let Some(index) = coverage.iter().position(|row| row.element == child) else {
            findings.push(sequence_finding(format!(
                "top-level element '{child}' is not allowed by the OASIS UBL 2.1 {document_kind:?} sequence"
            )));
            continue;
        };
        if index < previous_index {
            findings.push(sequence_finding(format!(
                "top-level element '{child}' is out of order in the OASIS UBL 2.1 {document_kind:?} sequence"
            )));
        }
        previous_index = index;
        *counts.entry(child.as_str()).or_insert(0) += 1;
    }

    for row in coverage {
        let count = counts.get(row.element).copied().unwrap_or(0);
        if row.cardinality.starts_with("1..") && count == 0 {
            findings.push(sequence_finding(format!(
                "required top-level element '{}' is missing from the OASIS UBL 2.1 {:?} sequence",
                row.element, document_kind
            )));
        }
        if row.cardinality.ends_with("..1") && count > 1 {
            findings.push(sequence_finding(format!(
                "top-level element '{}' appears {count} times but OASIS UBL 2.1 {:?} allows {}",
                row.element, document_kind, row.cardinality
            )));
        }
    }

    Ok(findings)
}

fn top_level_children(xml: &str) -> Result<Vec<String>, UblError> {
    let mut reader = Reader::from_str(xml);
    let mut xml_version = XmlVersion::default();
    let mut namespace_stack = vec![NamespaceFrame::default()];
    let mut depth = 0_usize;
    let mut children = Vec::new();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let (element, _, frame) =
                    read_element_start(&reader, &start, xml_version, namespace_stack.last())?;
                if depth == 1 {
                    children.push(qualified_top_level_name(&element));
                }
                namespace_stack.push(frame);
                depth += 1;
            }
            Event::Empty(start) => {
                let (element, _, _) =
                    read_element_start(&reader, &start, xml_version, namespace_stack.last())?;
                if depth == 1 {
                    children.push(qualified_top_level_name(&element));
                }
            }
            Event::End(end) => {
                let _ = read_element_end(end.name().as_ref(), namespace_stack.last())?;
                depth = depth.saturating_sub(1);
                namespace_stack.pop();
            }
            Event::Decl(decl) => {
                let version = decl.version()?;
                xml_version = if version.as_ref() == b"1.1" {
                    XmlVersion::Explicit1_1
                } else {
                    XmlVersion::Explicit1_0
                };
            }
            Event::DocType(_) => return Err(UblError::UnsupportedRoot("DOCTYPE".to_owned())),
            Event::Text(_)
            | Event::CData(_)
            | Event::GeneralRef(_)
            | Event::PI(_)
            | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }

    Ok(children)
}

fn qualified_top_level_name(element: &ParsedElement) -> String {
    let prefix = match element.namespace_uri.as_deref() {
        Some(UBL_EXT_NAMESPACE_URI) => Some("ext"),
        Some(UBL_CBC_NAMESPACE_URI) => Some("cbc"),
        Some(UBL_CAC_NAMESPACE_URI) => Some("cac"),
        Some(UBL_INVOICE_NAMESPACE_URI | UBL_CREDIT_NOTE_NAMESPACE_URI) | None => None,
        Some(_) => Some("foreign"),
    };
    prefix.map_or_else(
        || element.local_name.clone(),
        |prefix| format!("{prefix}:{}", element.local_name),
    )
}

fn sequence_finding(message: String) -> UblSchemaValidationFinding {
    UblSchemaValidationFinding {
        level: "error".to_owned(),
        message,
        element_path: None,
        line: None,
        column: None,
    }
}
