// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! UBL 2.1 parser and serializer for InvoiceKit's core commercial IR.
//!
//! This crate is intentionally scoped to the [`invoicekit_ir::CommercialDocument`]
//! fields that exist today. It accepts UBL 2.1 `Invoice` and `CreditNote`
//! documents, extracts the core IR fields, and emits deterministic UBL XML that
//! is canonicalized by [`invoicekit_canonical::canonicalize_xml`].

use std::fmt::Write as _;
use std::str::FromStr as _;

use invoicekit_canonical::{canonicalize_xml, XmlCanonicalizeError};
use invoicekit_ir::{
    Attachment, CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly,
    DecimalValue, DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference,
    DocumentType, IrError, Iso4217Code, JurisdictionExtension, LocalizedString, LossinessEntry,
    LossinessLedger, MonetaryTotal, MoneyAmount, Party, PartyTaxId, PaymentInstruction,
    PaymentInstructionKind, PaymentTerms, PostalAddress, Quantity, SchemaVersion,
    TaxCategorySummary,
};
use quick_xml::events::{attributes::AttrError, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

pub mod mapping;
mod schema;

pub use mapping::{
    coverage_for, top_level_coverage, UblCoverageClass, UblDocumentKind, UblElementCoverage,
    CREDIT_NOTE_ELEMENT_COVERAGE, INVOICEKIT_METADATA_EXTENSION_URN, INVOICE_ELEMENT_COVERAGE,
    UBL_2_1_CREDIT_NOTE_SCHEMA_URI, UBL_2_1_INVOICE_SCHEMA_URI, UBL_2_1_OS_SPEC_URI,
    UBL_DOCUMENT_FIELDS_EXTENSION_URN,
};
pub use schema::{
    validate_oasis_ubl_2_1_schema, UblSchemaValidatedFixture, UblSchemaValidationFinding,
    UblSchemaValidationReport, OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES,
};

const BEAD_ID: &str = "invoices-t-040-ubl-2-1-parser-serializer-1v2";
const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CREDIT_NOTE_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";
const UBL_CAC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
const UBL_CBC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";
const UBL_EXT_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2";
const INVOICEKIT_EXTENSION_NAMESPACE_URI: &str = "urn:invoicekit:ubl:extension";
const CORE_CUSTOMIZATION_ID: &str = "urn:invoicekit:ubl:2.1:core";
const CORE_PROFILE_ID: &str = "urn:invoicekit:profile:core";
const DEFAULT_LANGUAGE: &str = "und";
const UBL_TOP_LEVEL_KEY: &str = "top_level";
const UBL_TOP_LEVEL_ELEMENT_KEY: &str = "element";
const UBL_TOP_LEVEL_XML_KEY: &str = "xml";

/// Errors returned by [`to_xml`] and [`from_xml`].
#[derive(Debug, Error)]
pub enum UblError {
    /// The XML input was not well formed.
    #[error("UBL XML is not well formed: {0}; hint: pass a complete UBL 2.1 Invoice or CreditNote document")]
    InvalidXml(#[from] quick_xml::Error),
    /// An XML attribute was malformed.
    #[error("UBL XML attribute is invalid: {0}; hint: check namespace and UBL basic-component attributes")]
    InvalidAttribute(#[from] AttrError),
    /// Text or attribute content could not be decoded.
    #[error(
        "UBL XML text encoding is invalid: {0}; hint: InvoiceKit expects UTF-8 compatible XML"
    )]
    InvalidEncoding(#[from] quick_xml::encoding::EncodingError),
    /// A tag or attribute name was not UTF-8.
    #[error("UBL XML name `{0}` is not valid UTF-8; hint: use UTF-8 element and attribute names")]
    InvalidName(String),
    /// The root element is not a UBL `Invoice` or `CreditNote`.
    #[error("unsupported UBL root `{0}`; hint: use Invoice or CreditNote")]
    UnsupportedRoot(String),
    /// The IR document type cannot be serialized as this UBL family member.
    #[error("document type `{0:?}` is not supported by the UBL serializer; hint: use Invoice or CreditNote")]
    UnsupportedDocumentType(DocumentType),
    /// An IR field has no schema-valid representation in the target UBL document type.
    #[error(
        "field `{field}` cannot be serialized for document type `{document_type:?}`; hint: {hint}"
    )]
    UnsupportedDocumentField {
        /// Document type being serialized.
        document_type: DocumentType,
        /// IR field path.
        field: &'static str,
        /// Remediation hint.
        hint: &'static str,
    },
    /// A required UBL element was missing.
    #[error("missing required UBL element `{0}`; hint: include the element needed to build InvoiceKit IR")]
    MissingElement(&'static str),
    /// A decimal field could not be parsed.
    #[error("invalid decimal `{value}` at `{path}`; hint: use a fixed-scale decimal string")]
    InvalidDecimal {
        /// UBL element path.
        path: &'static str,
        /// Invalid value.
        value: String,
    },
    /// IR validation failed after parsing.
    #[error("parsed UBL did not satisfy InvoiceKit IR validation: {0}")]
    InvalidIr(#[from] IrError),
    /// JSON conversion failed while reading opaque IR newtypes.
    #[error("could not serialize InvoiceKit IR helper value: {0}")]
    InvalidIrJson(#[from] serde_json::Error),
    /// Canonical XML output could not be produced.
    #[error("could not canonicalize UBL XML output: {0}")]
    Canonicalize(#[from] XmlCanonicalizeError),
    /// OASIS UBL 2.1 XSD validation harness failed before producing findings.
    #[error("OASIS UBL 2.1 schema harness failed during {operation}: {message}; hint: check the vendored schema corpus and XML well-formedness")]
    SchemaHarness {
        /// Operation that failed.
        operation: &'static str,
        /// Validator or I/O diagnostic.
        message: String,
    },
    /// A preserved UBL fragment in an extension payload is not safe to replay.
    #[error(
        "invalid preserved UBL top-level fragment for `{element}`: {message}; hint: preserve fragments produced by from_xml or remove the invalid UBL document-fields payload"
    )]
    InvalidPreservedTopLevel {
        /// Expected OASIS UBL top-level element slot.
        element: String,
        /// Validation diagnostic.
        message: String,
    },
}

/// Serialize an InvoiceKit commercial document into deterministic UBL 2.1 XML.
///
/// The returned XML has already passed through InvoiceKit XML canonicalization,
/// so serializing the same document twice on the same platform returns identical
/// bytes.
///
/// # Errors
///
/// Returns [`UblError::UnsupportedDocumentType`] for document types other than
/// [`DocumentType::Invoice`] and [`DocumentType::CreditNote`], or a canonical XML
/// error if the generated XML cannot be canonicalized.
///
/// # Examples
///
/// ```
/// # use invoicekit_format_ubl::to_xml;
/// # fn fixture() -> invoicekit_ir::CommercialDocument {
/// #     use invoicekit_ir::*;
/// #     use rust_decimal::Decimal;
/// #     let amount = DecimalValue::new(Decimal::new(10000, 2));
/// #     let party = Party {
/// #         id: Some("party-1".to_owned()),
/// #         name: "Example GmbH".to_owned(),
/// #         tax_ids: vec![PartyTaxId { scheme: "vat".to_owned(), value: "DE123456789".to_owned() }],
/// #         address: PostalAddress {
/// #             lines: vec!["Main Street 1".to_owned()],
/// #             city: "Berlin".to_owned(),
/// #             subdivision: None,
/// #             postal_code: "10115".to_owned(),
/// #             country: CountryCode::new("DE").unwrap(),
/// #         },
/// #         contact: None,
/// #     };
/// #     CommercialDocument::new(CommercialDocumentParts {
/// #         schema_version: SchemaVersion::V1_0,
/// #         id: DocumentId::new("doc-1").unwrap(),
/// #         document_type: DocumentType::Invoice,
/// #         issue_date: DateOnly::new("2026-05-26").unwrap(),
/// #         tax_point_date: None,
/// #         due_date: None,
/// #         document_number: DocumentNumber::new("INV-1").unwrap(),
/// #         currency: Iso4217Code::new("EUR").unwrap(),
/// #         supplier: party.clone(),
/// #         customer: party,
/// #         payee: None,
/// #         payment_terms: None,
/// #         payment_instructions: Vec::new(),
/// #         lines: vec![DocumentLine {
/// #             id: "1".to_owned(),
/// #             description: "Service".to_owned(),
/// #             quantity: DecimalValue::new(Decimal::ONE),
/// #             unit_code: Some("EA".to_owned()),
/// #             unit_price: amount.clone(),
/// #             line_extension_amount: amount.clone(),
/// #             tax_category: Some("S".to_owned()),
/// #             extensions: Vec::new(),
/// #         }],
/// #         tax_summary: vec![TaxCategorySummary {
/// #             category_code: "S".to_owned(),
/// #             taxable_amount: amount.clone(),
/// #             tax_amount: DecimalValue::new(Decimal::new(1900, 2)),
/// #             tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
/// #         }],
/// #         monetary_total: MonetaryTotal {
/// #             line_extension_amount: amount.clone(),
/// #             tax_exclusive_amount: amount.clone(),
/// #             tax_inclusive_amount: DecimalValue::new(Decimal::new(11900, 2)),
/// #             allowance_total_amount: None,
/// #             charge_total_amount: None,
/// #             prepaid_amount: None,
/// #             payable_amount: DecimalValue::new(Decimal::new(11900, 2)),
/// #         },
/// #         attachments: Vec::new(),
/// #         references: Vec::new(),
/// #         notes: Vec::new(),
/// #         extensions: Vec::new(),
/// #         meta: DocumentMeta {
/// #             tenant_id: "tenant".to_owned(),
/// #             trace_id: "trace".to_owned(),
/// #             source_system: None,
/// #         },
/// #     }).unwrap()
/// # }
/// let xml = to_xml(&fixture()).unwrap();
/// assert!(xml.contains("<Invoice"));
/// ```
pub fn to_xml(document: &CommercialDocument) -> Result<String, UblError> {
    document.validate()?;
    let raw = match document.document_type {
        DocumentType::Invoice => {
            serialize_document(document, "Invoice", UBL_INVOICE_NAMESPACE_URI)?
        }
        DocumentType::CreditNote => {
            serialize_document(document, "CreditNote", UBL_CREDIT_NOTE_NAMESPACE_URI)?
        }
        other => return Err(UblError::UnsupportedDocumentType(other)),
    };
    Ok(canonicalize_xml(&raw)?)
}

/// Parse a UBL 2.1 `Invoice` or `CreditNote` document into InvoiceKit IR.
///
/// The parser extracts the current core IR surface. Non-core top-level UBL
/// elements are preserved in a UBL-specific [`JurisdictionExtension`] so later
/// profile passes can round-trip them without silent loss.
///
/// # Errors
///
/// Returns a typed [`UblError`] when XML is malformed, the root is not
/// `Invoice` or `CreditNote`, required IR fields are absent, decimal values are
/// invalid, or the resulting IR does not validate.
///
/// # Examples
///
/// ```
/// # let xml = r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cbc:ID>INV-1</cbc:ID><cbc:UUID>doc-1</cbc:UUID><cbc:IssueDate>2026-05-26</cbc:IssueDate><cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode><cbc:BuyerReference>tenant</cbc:BuyerReference><cbc:AccountingCost>trace</cbc:AccountingCost><cac:AccountingSupplierParty><cac:Party><cac:PartyName><cbc:Name>Supplier</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress></cac:Party></cac:AccountingSupplierParty><cac:AccountingCustomerParty><cac:Party><cac:PartyName><cbc:Name>Customer</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress></cac:Party></cac:AccountingCustomerParty><cac:LegalMonetaryTotal><cbc:LineExtensionAmount currencyID="EUR">100.00</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount currencyID="EUR">100.00</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount currencyID="EUR">119.00</cbc:TaxInclusiveAmount><cbc:PayableAmount currencyID="EUR">119.00</cbc:PayableAmount></cac:LegalMonetaryTotal><cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:InvoicedQuantity unitCode="EA">1</cbc:InvoicedQuantity><cbc:LineExtensionAmount currencyID="EUR">100.00</cbc:LineExtensionAmount><cac:Item><cbc:Name>Service</cbc:Name></cac:Item><cac:Price><cbc:PriceAmount currencyID="EUR">100.00</cbc:PriceAmount></cac:Price></cac:InvoiceLine></Invoice>"#;
/// let (parsed, ledger) = invoicekit_format_ubl::from_xml(xml).unwrap();
/// assert_eq!(parsed.document_type, invoicekit_ir::DocumentType::Invoice);
/// assert!(ledger.lost.is_empty());
/// ```
pub fn from_xml(input: &str) -> Result<(CommercialDocument, LossinessLedger), UblError> {
    let (document, dropped_payment_means) = parse_xml_document(input)?;
    let serialized = to_xml(&document)?;
    let (reparsed, _) = parse_xml_document(&serialized)?;
    let mut ledger =
        LossinessLedger::from_roundtrip_comparison(&document, &reparsed, "format-ubl")?;
    if dropped_payment_means > 0 {
        ledger.lost.push(LossinessEntry {
            path: "/payment_instructions".to_owned(),
            reason: format!(
                "[format-ubl] dropped {dropped_payment_means} cac:PaymentMeans element(s) carrying \
                 only cbc:PaymentMeansCode (no cbc:PaymentID or cac:PayeeFinancialAccount); the core \
                 invoice model has no field for a bare payment-means code"
            ),
        });
    }
    Ok((document, ledger))
}

fn parse_xml_document(input: &str) -> Result<(CommercialDocument, usize), UblError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut xml_version = XmlVersion::default();
    let mut namespace_stack = vec![NamespaceFrame::default()];
    let mut stack = Vec::<ParsedElement>::new();
    let mut state = ParseState::default();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let (element, attrs, frame) =
                    read_element_start(&reader, &start, xml_version, namespace_stack.last())?;
                state.start_element(&stack, &element, &attrs)?;
                namespace_stack.push(frame);
                stack.push(element);
            }
            Event::Empty(start) => {
                let (element, attrs, _) =
                    read_element_start(&reader, &start, xml_version, namespace_stack.last())?;
                state.start_element(&stack, &element, &attrs)?;
                state.end_element(&element)?;
            }
            Event::End(end) => {
                let element = read_element_end(end.name().as_ref(), namespace_stack.last())?;
                state.end_element(&element)?;
                let Some(opened) = stack.pop() else {
                    return Err(UblError::UnsupportedRoot(element.local_name));
                };
                if opened != element {
                    return Err(UblError::UnsupportedRoot(format!(
                        "{}/{}",
                        opened.local_name, element.local_name
                    )));
                }
                namespace_stack.pop();
            }
            Event::Text(text) => {
                let text = text.xml_content(xml_version)?;
                state.text(&stack, text.as_ref())?;
            }
            Event::CData(cdata) => {
                let text = cdata.xml_content(xml_version)?;
                state.text(&stack, text.as_ref())?;
            }
            Event::GeneralRef(reference) => {
                let reference = reference.xml_content(xml_version)?;
                let text = resolve_xml_reference(&reference)?;
                state.text(&stack, &text)?;
            }
            Event::Decl(decl) => {
                let version = decl.version()?;
                xml_version = if version.as_ref() == b"1.1" {
                    XmlVersion::Explicit1_1
                } else {
                    XmlVersion::Explicit1_0
                };
            }
            Event::DocType(_) => {
                return Err(UblError::UnsupportedRoot("DOCTYPE".to_owned()));
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }

    let dropped_payment_means = state.dropped_payment_means;
    let document = state.finish()?;
    Ok((document, dropped_payment_means))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_format_ubl::crate_name(), "invoicekit-format-ubl");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-format-ubl"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PartyRole {
    Supplier,
    Customer,
    Payee,
}

#[derive(Default)]
struct ParseState {
    document_type: Option<DocumentType>,
    document_id: Option<String>,
    document_number: Option<String>,
    issue_date: Option<String>,
    tax_point_date: Option<String>,
    due_date: Option<String>,
    currency: Option<String>,
    metadata_tenant_id: Option<String>,
    metadata_trace_id: Option<String>,
    metadata_source_system: Option<String>,
    ubl_buyer_reference: Option<String>,
    ubl_accounting_cost: Option<String>,
    preserved_top_level: Vec<PreservedTopLevelXml>,
    capture: Option<XmlCapture>,
    current_extension_uri: Option<String>,
    supplier: PartyBuilder,
    customer: PartyBuilder,
    payee: PartyBuilder,
    has_payee: bool,
    payment_terms_description: Option<String>,
    payment_instructions: Vec<PaymentInstruction>,
    current_payment: Option<PaymentBuilder>,
    dropped_payment_means: usize,
    lines: Vec<DocumentLine>,
    current_line: Option<LineBuilder>,
    tax_summary: Vec<TaxCategorySummary>,
    current_tax: Option<TaxSummaryBuilder>,
    monetary_total: MonetaryTotalBuilder,
    notes: Vec<LocalizedString>,
    current_note_language: Option<String>,
}

impl ParseState {
    fn start_element(
        &mut self,
        stack: &[ParsedElement],
        element: &ParsedElement,
        attrs: &[XmlAttribute],
    ) -> Result<(), UblError> {
        let name = element.local_name.as_str();
        if stack.is_empty() {
            self.document_type = Some(match name {
                "Invoice" => DocumentType::Invoice,
                "CreditNote" => DocumentType::CreditNote,
                other => return Err(UblError::UnsupportedRoot(other.to_owned())),
            });
        }
        if let Some(capture) = self.capture.as_mut() {
            capture.start_element(element, attrs)?;
            return Ok(());
        }
        if let Some(document_kind) = self.document_type.and_then(document_kind_from_type) {
            if should_preserve_top_level(stack, element, document_kind) {
                self.capture = Some(XmlCapture::new(element, attrs)?);
                return Ok(());
            }
        }
        if is_element(element, "UBLExtension", UBL_EXT_NAMESPACE_URI)
            && is_root_ubl_extensions(stack)
        {
            self.current_extension_uri = None;
        }
        if name == "InvoiceLine" || name == "CreditNoteLine" {
            self.current_line = Some(LineBuilder::default());
        }
        if name == "TaxSubtotal" {
            self.current_tax = Some(TaxSummaryBuilder::default());
        }
        if name == "PaymentMeans" {
            self.current_payment = Some(PaymentBuilder::default());
        }
        if name == "Note" && stack.len() == 1 {
            self.current_note_language = attr_value(attrs, "languageID").map(ToOwned::to_owned);
        }
        if let Some(line) = self.current_line.as_mut() {
            if name == "InvoicedQuantity" || name == "CreditedQuantity" {
                line.unit_code = attr_value(attrs, "unitCode").map(ToOwned::to_owned);
            }
        }
        Ok(())
    }

    fn end_element(&mut self, element: &ParsedElement) -> Result<(), UblError> {
        if let Some(capture) = self.capture.as_mut() {
            capture.end_element(element)?;
            if capture.is_finished() {
                let capture = self
                    .capture
                    .take()
                    .ok_or(UblError::MissingElement("preserved top-level UBL element"))?;
                self.preserved_top_level.push(capture.finish());
            }
            return Ok(());
        }
        let name = element.local_name.as_str();
        if name == "InvoiceLine" || name == "CreditNoteLine" {
            let line = self
                .current_line
                .take()
                .ok_or(UblError::MissingElement("InvoiceLine"))?
                .build()?;
            self.lines.push(line);
        }
        if name == "TaxSubtotal" {
            let summary = self
                .current_tax
                .take()
                .ok_or(UblError::MissingElement("TaxSubtotal"))?
                .build()?;
            self.tax_summary.push(summary);
        }
        if name == "PaymentMeans" {
            if let Some(builder) = self.current_payment.take() {
                match builder.build() {
                    Some(payment) => self.payment_instructions.push(payment),
                    None => self.dropped_payment_means += 1,
                }
            }
        }
        if is_element(element, "UBLExtension", UBL_EXT_NAMESPACE_URI) {
            self.current_extension_uri = None;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn text(&mut self, stack: &[ParsedElement], raw: &str) -> Result<(), UblError> {
        if let Some(capture) = self.capture.as_mut() {
            capture.text(raw);
            return Ok(());
        }
        let value = raw.trim();
        if value.is_empty() {
            return Ok(());
        }

        if path_ends_ns(
            stack,
            &[
                ("UBLExtension", UBL_EXT_NAMESPACE_URI),
                ("ExtensionURI", UBL_EXT_NAMESPACE_URI),
            ],
        ) && in_top_level_ubl_extension(stack)
        {
            self.current_extension_uri = Some(value.to_owned());
            return Ok(());
        }

        if let Some(line) = self.current_line.as_mut() {
            if path_ends(stack, &["InvoiceLine", "ID"])
                || path_ends(stack, &["CreditNoteLine", "ID"])
            {
                line.id = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["InvoicedQuantity"]) || path_ends(stack, &["CreditedQuantity"]) {
                line.quantity = Some(decimal_value("line.quantity", value)?);
                return Ok(());
            }
            if path_ends(stack, &["LineExtensionAmount"]) {
                line.line_extension_amount =
                    Some(decimal_value("line.line_extension_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["Price", "PriceAmount"]) {
                line.unit_price = Some(decimal_value("line.unit_price", value)?);
                return Ok(());
            }
            if path_ends(stack, &["Item", "Description"]) || path_ends(stack, &["Item", "Name"]) {
                line.description = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["ClassifiedTaxCategory", "ID"]) {
                line.tax_category = Some(value.to_owned());
                return Ok(());
            }
        }

        if let Some(tax) = self.current_tax.as_mut() {
            if path_ends(stack, &["TaxableAmount"]) {
                tax.taxable_amount = Some(decimal_value("tax_summary.taxable_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["TaxAmount"]) {
                tax.tax_amount = Some(decimal_value("tax_summary.tax_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["TaxCategory", "ID"]) {
                tax.category_code = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["TaxCategory", "Percent"]) {
                tax.tax_rate = Some(decimal_value("tax_summary.tax_rate", value)?);
                return Ok(());
            }
        }

        if let Some(role) = party_role(stack) {
            let party = self.party_mut(role);
            if path_ends(stack, &["EndpointID"]) || path_ends(stack, &["PartyIdentification", "ID"])
            {
                if party.id.is_none() {
                    party.id = Some(value.to_owned());
                }
                return Ok(());
            }
            if path_ends(stack, &["PartyName", "Name"])
                || path_ends(stack, &["PartyLegalEntity", "RegistrationName"])
            {
                party.name = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PartyTaxScheme", "CompanyID"]) {
                party.tax_ids.push(PartyTaxId {
                    scheme: "vat".to_owned(),
                    value: value.to_owned(),
                });
                return Ok(());
            }
            if path_ends(stack, &["PostalAddress", "StreetName"])
                || path_ends(stack, &["PostalAddress", "AdditionalStreetName"])
                || path_ends(stack, &["PostalAddress", "Line"])
            {
                party.address_lines.push(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalAddress", "CityName"]) {
                party.city = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalAddress", "PostalZone"]) {
                party.postal_code = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalAddress", "CountrySubentity"]) {
                party.subdivision = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["Country", "IdentificationCode"]) {
                party.country = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["Contact", "Name"]) {
                party.contact_name = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["Contact", "ElectronicMail"]) {
                party.email = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["Contact", "Telephone"]) {
                party.phone = Some(value.to_owned());
                return Ok(());
            }
        }

        if let Some(payment) = self.current_payment.as_mut() {
            if path_ends(stack, &["PaymentID"]) {
                payment.reference = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PayeeFinancialAccount", "ID"]) {
                payment.account = Some(value.to_owned());
                return Ok(());
            }
        }

        if in_any(stack, &["LegalMonetaryTotal"]) {
            if path_ends(stack, &["LineExtensionAmount"]) {
                self.monetary_total.line_extension_amount = Some(decimal_value(
                    "monetary_total.line_extension_amount",
                    value,
                )?);
                return Ok(());
            }
            if path_ends(stack, &["TaxExclusiveAmount"]) {
                self.monetary_total.tax_exclusive_amount =
                    Some(decimal_value("monetary_total.tax_exclusive_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["TaxInclusiveAmount"]) {
                self.monetary_total.tax_inclusive_amount =
                    Some(decimal_value("monetary_total.tax_inclusive_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["AllowanceTotalAmount"]) {
                self.monetary_total.allowance_total_amount = Some(decimal_value(
                    "monetary_total.allowance_total_amount",
                    value,
                )?);
                return Ok(());
            }
            if path_ends(stack, &["ChargeTotalAmount"]) {
                self.monetary_total.charge_total_amount =
                    Some(decimal_value("monetary_total.charge_total_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["PrepaidAmount"]) {
                self.monetary_total.prepaid_amount =
                    Some(decimal_value("monetary_total.prepaid_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["PayableAmount"]) {
                self.monetary_total.payable_amount =
                    Some(decimal_value("monetary_total.payable_amount", value)?);
                return Ok(());
            }
        }

        if is_root_child(stack, "ID") {
            self.document_number = Some(value.to_owned());
        } else if is_root_child(stack, "UUID") {
            self.document_id = Some(value.to_owned());
        } else if is_root_child(stack, "IssueDate") {
            self.issue_date = Some(value.to_owned());
        } else if is_root_child(stack, "TaxPointDate") {
            self.tax_point_date = Some(value.to_owned());
        } else if is_root_child(stack, "DueDate") {
            self.due_date = Some(value.to_owned());
        } else if is_root_child(stack, "DocumentCurrencyCode") {
            self.currency = Some(value.to_owned());
        } else if is_root_child(stack, "BuyerReference") {
            self.ubl_buyer_reference = Some(value.to_owned());
        } else if is_root_child(stack, "AccountingCost") {
            self.ubl_accounting_cost = Some(value.to_owned());
        } else if is_root_child(stack, "Note") {
            self.notes.push(LocalizedString {
                language: self
                    .current_note_language
                    .clone()
                    .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
                text: value.to_owned(),
            });
        } else if path_ends(stack, &["PaymentTerms", "Note"]) {
            self.payment_terms_description = Some(value.to_owned());
        } else if self.is_invoicekit_metadata_field(stack, "TenantID") {
            self.metadata_tenant_id = Some(value.to_owned());
        } else if self.is_invoicekit_metadata_field(stack, "TraceID") {
            self.metadata_trace_id = Some(value.to_owned());
        } else if self.is_invoicekit_metadata_field(stack, "SourceSystem") {
            self.metadata_source_system = Some(value.to_owned());
        }

        Ok(())
    }

    fn finish(self) -> Result<CommercialDocument, UblError> {
        let document_type = self
            .document_type
            .ok_or(UblError::MissingElement("Invoice|CreditNote"))?;
        let document_number = self
            .document_number
            .ok_or(UblError::MissingElement("cbc:ID"))?;
        let document_id = self.document_id.unwrap_or_else(|| document_number.clone());
        let issue_date = self
            .issue_date
            .ok_or(UblError::MissingElement("cbc:IssueDate"))?;
        let currency = self
            .currency
            .ok_or(UblError::MissingElement("cbc:DocumentCurrencyCode"))?;
        let tenant_id = self
            .metadata_tenant_id
            .unwrap_or_else(|| "ubl-import".to_owned());
        let trace_id = self
            .metadata_trace_id
            .unwrap_or_else(|| format!("{BEAD_ID}:{document_id}"));
        let mut extensions = Vec::<JurisdictionExtension>::new();
        let mut document_fields = Map::new();
        if let Some(value) = self.ubl_buyer_reference {
            document_fields.insert("buyer_reference".to_owned(), Value::String(value));
        }
        if let Some(value) = self.ubl_accounting_cost {
            document_fields.insert("accounting_cost".to_owned(), Value::String(value));
        }
        if !self.preserved_top_level.is_empty() {
            let preserved: Vec<Value> = self
                .preserved_top_level
                .into_iter()
                .filter(|item| !is_default_profile_fragment(item))
                .map(PreservedTopLevelXml::into_value)
                .collect();
            if !preserved.is_empty() {
                document_fields.insert(UBL_TOP_LEVEL_KEY.to_owned(), Value::Array(preserved));
            }
        }
        if !document_fields.is_empty() {
            extensions.push(JurisdictionExtension::new(
                UBL_DOCUMENT_FIELDS_EXTENSION_URN,
                Value::Object(document_fields),
            )?);
        }
        let payment_terms = match self.payment_terms_description {
            Some(description) => Some(PaymentTerms {
                description,
                due_date: self
                    .due_date
                    .as_ref()
                    .map(|date| DateOnly::new(date.clone()))
                    .transpose()?,
            }),
            None => None,
        };

        let document = CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(document_id)?,
            document_type,
            issue_date: DateOnly::new(issue_date)?,
            tax_point_date: self.tax_point_date.map(DateOnly::new).transpose()?,
            due_date: self.due_date.map(DateOnly::new).transpose()?,
            document_number: DocumentNumber::new(document_number)?,
            currency: Iso4217Code::new(currency)?,
            supplier: self.supplier.build("AccountingSupplierParty")?,
            customer: self.customer.build("AccountingCustomerParty")?,
            payee: if self.has_payee {
                Some(self.payee.build("PayeeParty")?)
            } else {
                None
            },
            payment_terms,
            payment_instructions: self.payment_instructions,
            lines: self.lines,
            tax_summary: self.tax_summary,
            monetary_total: self.monetary_total.build()?,
            attachments: Vec::<Attachment>::new(),
            references: Vec::<DocumentReference>::new(),
            notes: self.notes,
            extensions,
            meta: DocumentMeta {
                tenant_id,
                trace_id,
                source_system: self.metadata_source_system,
            },
        })?;
        Ok(document)
    }

    fn party_mut(&mut self, role: PartyRole) -> &mut PartyBuilder {
        match role {
            PartyRole::Supplier => &mut self.supplier,
            PartyRole::Customer => &mut self.customer,
            PartyRole::Payee => {
                self.has_payee = true;
                &mut self.payee
            }
        }
    }

    fn is_invoicekit_metadata_field(&self, stack: &[ParsedElement], field: &str) -> bool {
        self.current_extension_uri.as_deref() == Some(INVOICEKIT_METADATA_EXTENSION_URN)
            && in_top_level_ubl_extension(stack)
            && path_ends_ns(
                stack,
                &[
                    ("ExtensionContent", UBL_EXT_NAMESPACE_URI),
                    ("DocumentMeta", INVOICEKIT_EXTENSION_NAMESPACE_URI),
                    (field, INVOICEKIT_EXTENSION_NAMESPACE_URI),
                ],
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreservedTopLevelXml {
    element: String,
    xml: String,
}

impl PreservedTopLevelXml {
    fn into_value(self) -> Value {
        let mut payload = Map::new();
        payload.insert(
            UBL_TOP_LEVEL_ELEMENT_KEY.to_owned(),
            Value::String(self.element),
        );
        payload.insert(UBL_TOP_LEVEL_XML_KEY.to_owned(), Value::String(self.xml));
        Value::Object(payload)
    }
}

fn is_default_profile_fragment(item: &PreservedTopLevelXml) -> bool {
    matches!(
        (item.element.as_str(), item.xml.as_str()),
        (
            "cbc:CustomizationID",
            "<cbc:CustomizationID>urn:invoicekit:ubl:2.1:core</cbc:CustomizationID>"
        ) | (
            "cbc:ProfileID",
            "<cbc:ProfileID>urn:invoicekit:profile:core</cbc:ProfileID>"
        )
    )
}

#[derive(Debug)]
struct XmlCapture {
    element: String,
    xml: String,
    depth: usize,
}

impl XmlCapture {
    fn new(element: &ParsedElement, attrs: &[XmlAttribute]) -> Result<Self, UblError> {
        let mut capture = Self {
            element: ubl_element_qname(element)?,
            xml: String::new(),
            depth: 0,
        };
        capture.start_element(element, attrs)?;
        Ok(capture)
    }

    fn start_element(
        &mut self,
        element: &ParsedElement,
        attrs: &[XmlAttribute],
    ) -> Result<(), UblError> {
        write_start_element(&mut self.xml, element, attrs)?;
        self.depth += 1;
        Ok(())
    }

    fn end_element(&mut self, element: &ParsedElement) -> Result<(), UblError> {
        let name = ubl_element_qname(element)?;
        write!(self.xml, "</{name}>").expect("writing to a String cannot fail");
        self.depth = self
            .depth
            .checked_sub(1)
            .ok_or(UblError::MissingElement("preserved top-level UBL element"))?;
        Ok(())
    }

    fn text(&mut self, raw: &str) {
        write_xml_text(raw, &mut self.xml);
    }

    const fn is_finished(&self) -> bool {
        self.depth == 0
    }

    fn finish(self) -> PreservedTopLevelXml {
        PreservedTopLevelXml {
            element: self.element,
            xml: self.xml,
        }
    }
}

#[derive(Default)]
struct PartyBuilder {
    id: Option<String>,
    name: Option<String>,
    tax_ids: Vec<PartyTaxId>,
    address_lines: Vec<String>,
    city: Option<String>,
    subdivision: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
    contact_name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
}

impl PartyBuilder {
    fn build(self, field: &'static str) -> Result<Party, UblError> {
        let contact = if self.contact_name.is_some() || self.email.is_some() || self.phone.is_some()
        {
            Some(Contact {
                name: self.contact_name,
                email: self.email,
                phone: self.phone,
            })
        } else {
            None
        };
        Ok(Party {
            id: self.id,
            name: self.name.ok_or(UblError::MissingElement(field))?,
            tax_ids: self.tax_ids,
            address: PostalAddress {
                lines: if self.address_lines.is_empty() {
                    return Err(UblError::MissingElement("cac:PostalAddress/cbc:StreetName"));
                } else {
                    self.address_lines
                },
                city: self
                    .city
                    .ok_or(UblError::MissingElement("cac:PostalAddress/cbc:CityName"))?,
                subdivision: self.subdivision,
                postal_code: self
                    .postal_code
                    .ok_or(UblError::MissingElement("cac:PostalAddress/cbc:PostalZone"))?,
                country: CountryCode::new(self.country.ok_or(UblError::MissingElement(
                    "cac:Country/cbc:IdentificationCode",
                ))?)?,
            },
            contact,
        })
    }
}

#[derive(Default)]
struct LineBuilder {
    id: Option<String>,
    description: Option<String>,
    quantity: Option<Quantity>,
    unit_code: Option<String>,
    unit_price: Option<MoneyAmount>,
    line_extension_amount: Option<MoneyAmount>,
    tax_category: Option<String>,
}

impl LineBuilder {
    fn build(self) -> Result<DocumentLine, UblError> {
        Ok(DocumentLine {
            id: self
                .id
                .ok_or(UblError::MissingElement("cac:InvoiceLine/cbc:ID"))?,
            description: self
                .description
                .ok_or(UblError::MissingElement("cac:Item/cbc:Name"))?,
            quantity: self
                .quantity
                .ok_or(UblError::MissingElement("cbc:InvoicedQuantity"))?,
            unit_code: self.unit_code,
            unit_price: self
                .unit_price
                .ok_or(UblError::MissingElement("cac:Price/cbc:PriceAmount"))?,
            line_extension_amount: self
                .line_extension_amount
                .ok_or(UblError::MissingElement("cbc:LineExtensionAmount"))?,
            tax_category: self.tax_category,
            extensions: Vec::new(),
        })
    }
}

#[derive(Default)]
struct TaxSummaryBuilder {
    category_code: Option<String>,
    taxable_amount: Option<MoneyAmount>,
    tax_amount: Option<MoneyAmount>,
    tax_rate: Option<DecimalValue>,
}

impl TaxSummaryBuilder {
    fn build(self) -> Result<TaxCategorySummary, UblError> {
        Ok(TaxCategorySummary {
            category_code: self
                .category_code
                .ok_or(UblError::MissingElement("cac:TaxCategory/cbc:ID"))?,
            taxable_amount: self
                .taxable_amount
                .ok_or(UblError::MissingElement("cbc:TaxableAmount"))?,
            tax_amount: self
                .tax_amount
                .ok_or(UblError::MissingElement("cbc:TaxAmount"))?,
            tax_rate: self.tax_rate,
        })
    }
}

#[derive(Default)]
// The `_amount` suffix matches invoice business terms and serialized field names.
#[allow(clippy::struct_field_names)]
struct MonetaryTotalBuilder {
    line_extension_amount: Option<MoneyAmount>,
    tax_exclusive_amount: Option<MoneyAmount>,
    tax_inclusive_amount: Option<MoneyAmount>,
    allowance_total_amount: Option<MoneyAmount>,
    charge_total_amount: Option<MoneyAmount>,
    prepaid_amount: Option<MoneyAmount>,
    payable_amount: Option<MoneyAmount>,
}

impl MonetaryTotalBuilder {
    fn build(self) -> Result<MonetaryTotal, UblError> {
        Ok(MonetaryTotal {
            line_extension_amount: self
                .line_extension_amount
                .ok_or(UblError::MissingElement("cbc:LineExtensionAmount"))?,
            tax_exclusive_amount: self
                .tax_exclusive_amount
                .ok_or(UblError::MissingElement("cbc:TaxExclusiveAmount"))?,
            tax_inclusive_amount: self
                .tax_inclusive_amount
                .ok_or(UblError::MissingElement("cbc:TaxInclusiveAmount"))?,
            allowance_total_amount: self.allowance_total_amount,
            charge_total_amount: self.charge_total_amount,
            prepaid_amount: self.prepaid_amount,
            payable_amount: self
                .payable_amount
                .ok_or(UblError::MissingElement("cbc:PayableAmount"))?,
        })
    }
}

#[derive(Default)]
struct PaymentBuilder {
    account: Option<String>,
    reference: Option<String>,
}

impl PaymentBuilder {
    fn build(self) -> Option<PaymentInstruction> {
        if self.account.is_none() && self.reference.is_none() {
            return None;
        }
        let kind = if self.account.is_some() {
            PaymentInstructionKind::IbanBic
        } else {
            PaymentInstructionKind::Other
        };
        Some(PaymentInstruction {
            kind,
            account: self.account,
            reference: self.reference,
        })
    }
}

#[derive(Debug)]
struct XmlAttribute {
    local_name: String,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedElement {
    local_name: String,
    namespace_uri: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct NamespaceFrame {
    bindings: Vec<(String, String)>,
}

impl NamespaceFrame {
    fn set(&mut self, prefix: &str, uri: String) {
        if let Some((_, value)) = self.bindings.iter_mut().find(|(item, _)| item == prefix) {
            *value = uri;
        } else {
            self.bindings.push((prefix.to_owned(), uri));
        }
    }

    fn lookup(&self, prefix: &str) -> Option<String> {
        self.bindings
            .iter()
            .rev()
            .find(|(item, _)| item == prefix)
            .map(|(_, uri)| uri.clone())
    }
}

fn serialize_document(
    document: &CommercialDocument,
    root_name: &str,
    root_namespace: &str,
) -> Result<String, UblError> {
    if document.document_type == DocumentType::CreditNote && document.due_date.is_some() {
        return Err(UblError::UnsupportedDocumentField {
            document_type: document.document_type,
            field: "due_date",
            hint: "UBL 2.1 CreditNote has no top-level cbc:DueDate; omit due_date or model the value in a profile-specific extension",
        });
    }

    let currency = string_value(&document.currency)?;
    let mut xml = String::new();
    write!(
        xml,
        r#"<{root_name} xmlns="{root_namespace}" xmlns:cac="{UBL_CAC_NAMESPACE_URI}" xmlns:cbc="{UBL_CBC_NAMESPACE_URI}" xmlns:ext="{UBL_EXT_NAMESPACE_URI}" xmlns:ik="{INVOICEKIT_EXTENSION_NAMESPACE_URI}">"#
    )
    .expect("writing to a String cannot fail");
    write_document_header(&mut xml, document, &currency)?;
    write_document_parties(&mut xml, document)?;
    write_document_settlement(&mut xml, document, &currency)?;
    for line in &document.lines {
        write_line(&mut xml, document.document_type, line, &currency);
    }
    write!(xml, "</{root_name}>").expect("writing to a String cannot fail");
    Ok(xml)
}

fn write_document_header(
    xml: &mut String,
    document: &CommercialDocument,
    currency: &str,
) -> Result<(), UblError> {
    write_invoicekit_metadata_extension(xml, &document.meta);
    write_preserved_top_level(xml, document, "cbc:UBLVersionID")?;
    write_preserved_or_default_text(xml, document, "cbc:CustomizationID", CORE_CUSTOMIZATION_ID)?;
    write_preserved_or_default_text(xml, document, "cbc:ProfileID", CORE_PROFILE_ID)?;
    write_preserved_top_level(xml, document, "cbc:ProfileExecutionID")?;
    write_text_element(xml, "cbc:ID", &string_value(&document.document_number)?);
    write_preserved_top_level(xml, document, "cbc:CopyIndicator")?;
    write_text_element(xml, "cbc:UUID", document.id.as_str());
    write_text_element(xml, "cbc:IssueDate", document.issue_date.as_str());
    write_preserved_top_level(xml, document, "cbc:IssueTime")?;
    if document.document_type == DocumentType::Invoice {
        if let Some(date) = &document.due_date {
            write_text_element(xml, "cbc:DueDate", date.as_str());
        }
    }
    if document.document_type == DocumentType::Invoice {
        write_text_element(xml, "cbc:InvoiceTypeCode", "380");
    } else {
        if let Some(date) = &document.tax_point_date {
            write_text_element(xml, "cbc:TaxPointDate", date.as_str());
        }
        write_text_element(xml, "cbc:CreditNoteTypeCode", "381");
    }
    for note in &document.notes {
        write_note(xml, note);
    }
    if document.document_type == DocumentType::Invoice {
        if let Some(date) = &document.tax_point_date {
            write_text_element(xml, "cbc:TaxPointDate", date.as_str());
        }
    }
    write_text_element(xml, "cbc:DocumentCurrencyCode", currency);
    write_preserved_top_level(xml, document, "cbc:TaxCurrencyCode")?;
    write_preserved_top_level(xml, document, "cbc:PricingCurrencyCode")?;
    write_preserved_top_level(xml, document, "cbc:PaymentCurrencyCode")?;
    write_preserved_top_level(xml, document, "cbc:PaymentAlternativeCurrencyCode")?;
    write_preserved_top_level(xml, document, "cbc:AccountingCostCode")?;
    write_ubl_document_field(xml, document, "accounting_cost", "cbc:AccountingCost");
    write_preserved_top_level(xml, document, "cbc:LineCountNumeric")?;
    write_ubl_document_field(xml, document, "buyer_reference", "cbc:BuyerReference");
    if document.document_type == DocumentType::Invoice {
        write_invoice_preserved_before_supplier(xml, document)?;
    } else {
        write_credit_note_preserved_before_supplier(xml, document)?;
    }
    Ok(())
}

fn write_document_parties(xml: &mut String, document: &CommercialDocument) -> Result<(), UblError> {
    write_party(
        xml,
        "cac:AccountingSupplierParty",
        Some("cac:Party"),
        &document.supplier,
    )?;
    write_party(
        xml,
        "cac:AccountingCustomerParty",
        Some("cac:Party"),
        &document.customer,
    )?;
    if let Some(payee) = &document.payee {
        write_party(xml, "cac:PayeeParty", None, payee)?;
    }
    write_preserved_top_level(xml, document, "cac:BuyerCustomerParty")?;
    write_preserved_top_level(xml, document, "cac:SellerSupplierParty")?;
    write_preserved_top_level(xml, document, "cac:TaxRepresentativeParty")?;
    write_preserved_top_level(xml, document, "cac:Delivery")?;
    write_preserved_top_level(xml, document, "cac:DeliveryTerms")?;
    Ok(())
}

fn write_document_settlement(
    xml: &mut String,
    document: &CommercialDocument,
    currency: &str,
) -> Result<(), UblError> {
    for instruction in &document.payment_instructions {
        write_payment_instruction(xml, instruction);
    }
    if let Some(terms) = &document.payment_terms {
        xml.push_str("<cac:PaymentTerms>");
        write_text_element(xml, "cbc:Note", &terms.description);
        xml.push_str("</cac:PaymentTerms>");
    }
    if document.document_type == DocumentType::Invoice {
        write_preserved_top_level(xml, document, "cac:PrepaidPayment")?;
        write_preserved_top_level(xml, document, "cac:AllowanceCharge")?;
        write_preserved_exchange_rates(xml, document)?;
    } else {
        write_preserved_exchange_rates(xml, document)?;
        write_preserved_top_level(xml, document, "cac:AllowanceCharge")?;
    }
    if !document.tax_summary.is_empty() {
        write_tax_total(xml, &document.tax_summary, currency);
    }
    write_preserved_top_level(xml, document, "cac:WithholdingTaxTotal")?;
    write_monetary_total(xml, &document.monetary_total, currency);
    Ok(())
}

fn write_invoicekit_metadata_extension(xml: &mut String, meta: &DocumentMeta) {
    xml.push_str("<ext:UBLExtensions><ext:UBLExtension>");
    write_text_element(xml, "ext:ExtensionURI", INVOICEKIT_METADATA_EXTENSION_URN);
    xml.push_str("<ext:ExtensionContent><ik:DocumentMeta>");
    write_text_element(xml, "ik:TenantID", &meta.tenant_id);
    write_text_element(xml, "ik:TraceID", &meta.trace_id);
    if let Some(source_system) = &meta.source_system {
        write_text_element(xml, "ik:SourceSystem", source_system);
    }
    xml.push_str(
        "</ik:DocumentMeta></ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>",
    );
}

fn write_invoice_preserved_before_supplier(
    xml: &mut String,
    document: &CommercialDocument,
) -> Result<(), UblError> {
    for element in [
        "cac:InvoicePeriod",
        "cac:OrderReference",
        "cac:BillingReference",
        "cac:DespatchDocumentReference",
        "cac:ReceiptDocumentReference",
        "cac:StatementDocumentReference",
        "cac:OriginatorDocumentReference",
        "cac:ContractDocumentReference",
        "cac:AdditionalDocumentReference",
        "cac:ProjectReference",
        "cac:Signature",
    ] {
        write_preserved_top_level(xml, document, element)?;
    }
    Ok(())
}

fn write_credit_note_preserved_before_supplier(
    xml: &mut String,
    document: &CommercialDocument,
) -> Result<(), UblError> {
    for element in [
        "cac:InvoicePeriod",
        "cac:DiscrepancyResponse",
        "cac:OrderReference",
        "cac:BillingReference",
        "cac:DespatchDocumentReference",
        "cac:ReceiptDocumentReference",
        "cac:ContractDocumentReference",
        "cac:AdditionalDocumentReference",
        "cac:StatementDocumentReference",
        "cac:OriginatorDocumentReference",
        "cac:Signature",
    ] {
        write_preserved_top_level(xml, document, element)?;
    }
    Ok(())
}

fn write_preserved_exchange_rates(
    xml: &mut String,
    document: &CommercialDocument,
) -> Result<(), UblError> {
    for element in [
        "cac:TaxExchangeRate",
        "cac:PricingExchangeRate",
        "cac:PaymentExchangeRate",
        "cac:PaymentAlternativeExchangeRate",
    ] {
        write_preserved_top_level(xml, document, element)?;
    }
    Ok(())
}

fn write_ubl_document_field(
    xml: &mut String,
    document: &CommercialDocument,
    key: &str,
    element_name: &str,
) {
    if let Some(value) = ubl_document_field(document, key) {
        write_text_element(xml, element_name, value);
    }
}

fn ubl_document_field<'a>(document: &'a CommercialDocument, key: &str) -> Option<&'a str> {
    document
        .extensions
        .iter()
        .find(|extension| extension.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|extension| extension.payload.get(key))
        .and_then(Value::as_str)
}

fn write_preserved_or_default_text(
    xml: &mut String,
    document: &CommercialDocument,
    element: &str,
    default: &str,
) -> Result<(), UblError> {
    if !write_preserved_top_level(xml, document, element)? {
        write_text_element(xml, element, default);
    }
    Ok(())
}

fn write_preserved_top_level(
    xml: &mut String,
    document: &CommercialDocument,
    element: &str,
) -> Result<bool, UblError> {
    let Some(items) = document
        .extensions
        .iter()
        .find(|extension| extension.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|extension| extension.payload.get(UBL_TOP_LEVEL_KEY))
        .and_then(Value::as_array)
    else {
        return Ok(false);
    };

    let mut wrote = false;
    for item in items {
        let matches_element =
            item.get(UBL_TOP_LEVEL_ELEMENT_KEY).and_then(Value::as_str) == Some(element);
        if matches_element {
            if let Some(fragment) = item.get(UBL_TOP_LEVEL_XML_KEY).and_then(Value::as_str) {
                validate_preserved_top_level_fragment(document, element, fragment)?;
                xml.push_str(fragment);
                wrote = true;
            }
        }
    }
    Ok(wrote)
}

fn validate_preserved_top_level_fragment(
    document: &CommercialDocument,
    expected_element: &str,
    fragment: &str,
) -> Result<(), UblError> {
    validate_preserved_top_level_slot(document, expected_element)?;
    validate_preserved_fragment_xml(expected_element, fragment)
}

fn validate_preserved_top_level_slot(
    document: &CommercialDocument,
    expected_element: &str,
) -> Result<(), UblError> {
    let Some(document_kind) = document_kind_from_type(document.document_type) else {
        return invalid_preserved_top_level(
            expected_element,
            "document type cannot contain UBL Invoice/CreditNote top-level fragments",
        );
    };
    if coverage_for(document_kind, expected_element).is_none() {
        return invalid_preserved_top_level(
            expected_element,
            "element is not valid for this UBL document type",
        );
    }
    Ok(())
}

fn validate_preserved_fragment_xml(expected_element: &str, fragment: &str) -> Result<(), UblError> {
    let wrapped = format!(
        r#"<Wrapper xmlns:cac="{UBL_CAC_NAMESPACE_URI}" xmlns:cbc="{UBL_CBC_NAMESPACE_URI}" xmlns:ext="{UBL_EXT_NAMESPACE_URI}" xmlns:ik="{INVOICEKIT_EXTENSION_NAMESPACE_URI}">{fragment}</Wrapper>"#
    );
    let mut reader = Reader::from_str(&wrapped);
    reader.config_mut().trim_text(false);
    let mut state = PreservedFragmentValidation::new(expected_element);

    loop {
        let event = reader
            .read_event()
            .map_err(|err| UblError::InvalidPreservedTopLevel {
                element: expected_element.to_owned(),
                message: format!("fragment XML is not well formed: {err}"),
            })?;
        match event {
            Event::Start(start) => {
                state.start(&reader, &start)?;
            }
            Event::Empty(start) => {
                state.empty(&reader, &start)?;
            }
            Event::End(end) => {
                state.end(end.name().as_ref())?;
            }
            Event::Text(text) => {
                state.outer_text(text.xml_content(state.xml_version)?.as_ref(), "text")?;
            }
            Event::CData(cdata) => {
                state.outer_text(cdata.xml_content(state.xml_version)?.as_ref(), "CDATA")?;
            }
            Event::GeneralRef(reference) => {
                state.outer_text(
                    reference.xml_content(state.xml_version)?.as_ref(),
                    "entity text",
                )?;
            }
            Event::Decl(decl) => {
                let version = decl.version()?;
                state.xml_version = if version.as_ref() == b"1.1" {
                    XmlVersion::Explicit1_1
                } else {
                    XmlVersion::Explicit1_0
                };
            }
            Event::DocType(_) => {
                return invalid_preserved_top_level(expected_element, "DOCTYPE is not allowed");
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    state.finish()
}

struct PreservedFragmentValidation<'a> {
    expected_element: &'a str,
    xml_version: XmlVersion,
    namespace_stack: Vec<NamespaceFrame>,
    element_stack: Vec<ParsedElement>,
    depth: usize,
    saw_wrapper: bool,
    saw_child: bool,
}

impl<'a> PreservedFragmentValidation<'a> {
    fn new(expected_element: &'a str) -> Self {
        Self {
            expected_element,
            xml_version: XmlVersion::default(),
            namespace_stack: vec![NamespaceFrame::default()],
            element_stack: Vec::new(),
            depth: 0,
            saw_wrapper: false,
            saw_child: false,
        }
    }

    fn start(&mut self, reader: &Reader<&[u8]>, start: &BytesStart<'_>) -> Result<(), UblError> {
        let (element, _, frame) =
            read_element_start(reader, start, self.xml_version, self.namespace_stack.last())?;
        if self.depth == 0 {
            if element.local_name != "Wrapper" {
                return invalid_preserved_top_level(
                    self.expected_element,
                    "validation wrapper did not parse",
                );
            }
            self.saw_wrapper = true;
        } else if self.depth == 1 {
            validate_preserved_child(self.expected_element, &element, self.saw_child)?;
            self.saw_child = true;
        }
        self.element_stack.push(element);
        self.namespace_stack.push(frame);
        self.depth += 1;
        Ok(())
    }

    fn empty(&mut self, reader: &Reader<&[u8]>, start: &BytesStart<'_>) -> Result<(), UblError> {
        let (element, _, _) =
            read_element_start(reader, start, self.xml_version, self.namespace_stack.last())?;
        if self.depth == 1 {
            validate_preserved_child(self.expected_element, &element, self.saw_child)?;
            self.saw_child = true;
        } else if self.depth == 0 {
            return invalid_preserved_top_level(
                self.expected_element,
                "fragment wrapper cannot be empty",
            );
        }
        Ok(())
    }

    fn end(&mut self, raw: &[u8]) -> Result<(), UblError> {
        if self.depth == 0 {
            return invalid_preserved_top_level(
                self.expected_element,
                "fragment has more closing tags than opening tags",
            );
        }
        let element = read_element_end(raw, self.namespace_stack.last())?;
        let opened =
            self.element_stack
                .pop()
                .ok_or_else(|| UblError::InvalidPreservedTopLevel {
                    element: self.expected_element.to_owned(),
                    message: "fragment has more closing tags than opening tags".to_owned(),
                })?;
        if element != opened {
            return invalid_preserved_top_level(
                self.expected_element,
                format!(
                    "closing tag `{}` does not match opening tag `{}`",
                    element.local_name, opened.local_name
                ),
            );
        }
        self.depth -= 1;
        self.namespace_stack.pop();
        Ok(())
    }

    fn outer_text(&self, text: &str, kind: &str) -> Result<(), UblError> {
        if self.depth <= 1 && !text.trim().is_empty() {
            return invalid_preserved_top_level(
                self.expected_element,
                format!("fragment has {kind} outside its root element"),
            );
        }
        Ok(())
    }

    fn finish(self) -> Result<(), UblError> {
        if !self.saw_wrapper {
            return invalid_preserved_top_level(self.expected_element, "fragment did not parse");
        }
        if !self.saw_child {
            return invalid_preserved_top_level(self.expected_element, "fragment is empty");
        }
        if self.depth != 0 {
            return invalid_preserved_top_level(self.expected_element, "fragment is not balanced");
        }
        if !self.element_stack.is_empty() {
            return invalid_preserved_top_level(self.expected_element, "fragment is not balanced");
        }
        Ok(())
    }
}

fn validate_preserved_child(
    expected_element: &str,
    element: &ParsedElement,
    already_saw_child: bool,
) -> Result<(), UblError> {
    if already_saw_child {
        return invalid_preserved_top_level(
            expected_element,
            "fragment has multiple root elements",
        );
    }
    let actual = ubl_element_qname(element)?;
    if actual != expected_element {
        return invalid_preserved_top_level(
            expected_element,
            format!("fragment root `{actual}` does not match expected slot"),
        );
    }
    Ok(())
}

fn invalid_preserved_top_level<T>(
    element: &str,
    message: impl Into<String>,
) -> Result<T, UblError> {
    Err(UblError::InvalidPreservedTopLevel {
        element: element.to_owned(),
        message: message.into(),
    })
}

fn write_party(
    xml: &mut String,
    container: &str,
    nested_party: Option<&str>,
    party: &Party,
) -> Result<(), UblError> {
    write!(xml, "<{container}>").expect("writing to a String cannot fail");
    if let Some(wrapper) = nested_party {
        write!(xml, "<{wrapper}>").expect("writing to a String cannot fail");
    }
    if let Some(id) = &party.id {
        write_text_element(xml, "cbc:EndpointID", id);
        xml.push_str("<cac:PartyIdentification>");
        write_text_element(xml, "cbc:ID", id);
        xml.push_str("</cac:PartyIdentification>");
    }
    xml.push_str("<cac:PartyName>");
    write_text_element(xml, "cbc:Name", &party.name);
    xml.push_str("</cac:PartyName>");
    write_address(xml, &party.address)?;
    for tax_id in &party.tax_ids {
        xml.push_str("<cac:PartyTaxScheme>");
        write_text_element(xml, "cbc:CompanyID", &tax_id.value);
        xml.push_str("<cac:TaxScheme>");
        write_text_element(xml, "cbc:ID", &tax_id.scheme);
        xml.push_str("</cac:TaxScheme>");
        xml.push_str("</cac:PartyTaxScheme>");
    }
    xml.push_str("<cac:PartyLegalEntity>");
    write_text_element(xml, "cbc:RegistrationName", &party.name);
    xml.push_str("</cac:PartyLegalEntity>");
    if let Some(contact) = &party.contact {
        if contact.name.is_some() || contact.email.is_some() || contact.phone.is_some() {
            xml.push_str("<cac:Contact>");
            if let Some(name) = &contact.name {
                write_text_element(xml, "cbc:Name", name);
            }
            if let Some(phone) = &contact.phone {
                write_text_element(xml, "cbc:Telephone", phone);
            }
            if let Some(email) = &contact.email {
                write_text_element(xml, "cbc:ElectronicMail", email);
            }
            xml.push_str("</cac:Contact>");
        }
    }
    if let Some(wrapper) = nested_party {
        write!(xml, "</{wrapper}>").expect("writing to a String cannot fail");
    }
    write!(xml, "</{container}>").expect("writing to a String cannot fail");
    Ok(())
}

fn write_address(xml: &mut String, address: &PostalAddress) -> Result<(), UblError> {
    xml.push_str("<cac:PostalAddress>");
    if let Some(first) = address.lines.first() {
        write_text_element(xml, "cbc:StreetName", first);
    }
    for line in address.lines.iter().skip(1) {
        write_text_element(xml, "cbc:AdditionalStreetName", line);
    }
    write_text_element(xml, "cbc:CityName", &address.city);
    write_text_element(xml, "cbc:PostalZone", &address.postal_code);
    if let Some(subdivision) = &address.subdivision {
        write_text_element(xml, "cbc:CountrySubentity", subdivision);
    }
    xml.push_str("<cac:Country>");
    write_text_element(
        xml,
        "cbc:IdentificationCode",
        &string_value(&address.country)?,
    );
    xml.push_str("</cac:Country>");
    xml.push_str("</cac:PostalAddress>");
    Ok(())
}

fn write_payment_instruction(xml: &mut String, instruction: &PaymentInstruction) {
    xml.push_str("<cac:PaymentMeans>");
    let code = match instruction.kind {
        PaymentInstructionKind::Sepa | PaymentInstructionKind::IbanBic => "30",
        PaymentInstructionKind::SwissQr
        | PaymentInstructionKind::EpcQr
        | PaymentInstructionKind::ZatcaQr
        | PaymentInstructionKind::Other => "1",
    };
    write_text_element(xml, "cbc:PaymentMeansCode", code);
    if let Some(reference) = &instruction.reference {
        write_text_element(xml, "cbc:PaymentID", reference);
    }
    if let Some(account) = &instruction.account {
        xml.push_str("<cac:PayeeFinancialAccount>");
        write_text_element(xml, "cbc:ID", account);
        xml.push_str("</cac:PayeeFinancialAccount>");
    }
    xml.push_str("</cac:PaymentMeans>");
}

fn write_tax_total(xml: &mut String, summaries: &[TaxCategorySummary], currency: &str) {
    let total = summaries.iter().fold(Decimal::ZERO, |acc, summary| {
        acc + summary.tax_amount.inner()
    });
    xml.push_str("<cac:TaxTotal>");
    write_amount_element(xml, "cbc:TaxAmount", total, currency);
    for summary in summaries {
        xml.push_str("<cac:TaxSubtotal>");
        write_amount_element(
            xml,
            "cbc:TaxableAmount",
            summary.taxable_amount.inner(),
            currency,
        );
        write_amount_element(xml, "cbc:TaxAmount", summary.tax_amount.inner(), currency);
        xml.push_str("<cac:TaxCategory>");
        write_text_element(xml, "cbc:ID", &summary.category_code);
        if let Some(rate) = &summary.tax_rate {
            write_text_element(xml, "cbc:Percent", &rate.inner().to_string());
        }
        write_tax_scheme(xml);
        xml.push_str("</cac:TaxCategory>");
        xml.push_str("</cac:TaxSubtotal>");
    }
    xml.push_str("</cac:TaxTotal>");
}

fn write_tax_scheme(xml: &mut String) {
    xml.push_str("<cac:TaxScheme>");
    write_text_element(xml, "cbc:ID", "VAT");
    xml.push_str("</cac:TaxScheme>");
}

fn write_monetary_total(xml: &mut String, total: &MonetaryTotal, currency: &str) {
    xml.push_str("<cac:LegalMonetaryTotal>");
    write_amount_element(
        xml,
        "cbc:LineExtensionAmount",
        total.line_extension_amount.inner(),
        currency,
    );
    write_amount_element(
        xml,
        "cbc:TaxExclusiveAmount",
        total.tax_exclusive_amount.inner(),
        currency,
    );
    write_amount_element(
        xml,
        "cbc:TaxInclusiveAmount",
        total.tax_inclusive_amount.inner(),
        currency,
    );
    if let Some(value) = &total.allowance_total_amount {
        write_amount_element(xml, "cbc:AllowanceTotalAmount", value.inner(), currency);
    }
    if let Some(value) = &total.charge_total_amount {
        write_amount_element(xml, "cbc:ChargeTotalAmount", value.inner(), currency);
    }
    if let Some(value) = &total.prepaid_amount {
        write_amount_element(xml, "cbc:PrepaidAmount", value.inner(), currency);
    }
    write_amount_element(
        xml,
        "cbc:PayableAmount",
        total.payable_amount.inner(),
        currency,
    );
    xml.push_str("</cac:LegalMonetaryTotal>");
}

fn write_line(xml: &mut String, document_type: DocumentType, line: &DocumentLine, currency: &str) {
    let (container, quantity_element) = if document_type == DocumentType::CreditNote {
        ("cac:CreditNoteLine", "cbc:CreditedQuantity")
    } else {
        ("cac:InvoiceLine", "cbc:InvoicedQuantity")
    };
    write!(xml, "<{container}>").expect("writing to a String cannot fail");
    write_text_element(xml, "cbc:ID", &line.id);
    write!(xml, "<{quantity_element}").expect("writing to a String cannot fail");
    if let Some(unit_code) = &line.unit_code {
        xml.push_str(r#" unitCode=""#);
        write_xml_attr(unit_code, xml);
        xml.push('"');
    }
    xml.push('>');
    write_xml_text(&line.quantity.inner().to_string(), xml);
    write!(xml, "</{quantity_element}>").expect("writing to a String cannot fail");
    write_amount_element(
        xml,
        "cbc:LineExtensionAmount",
        line.line_extension_amount.inner(),
        currency,
    );
    xml.push_str("<cac:Item>");
    write_text_element(xml, "cbc:Name", &line.description);
    if let Some(category) = &line.tax_category {
        xml.push_str("<cac:ClassifiedTaxCategory>");
        write_text_element(xml, "cbc:ID", category);
        write_tax_scheme(xml);
        xml.push_str("</cac:ClassifiedTaxCategory>");
    }
    xml.push_str("</cac:Item>");
    xml.push_str("<cac:Price>");
    write_amount_element(xml, "cbc:PriceAmount", line.unit_price.inner(), currency);
    xml.push_str("</cac:Price>");
    write!(xml, "</{container}>").expect("writing to a String cannot fail");
}

fn write_note(xml: &mut String, note: &LocalizedString) {
    xml.push_str(r#"<cbc:Note languageID=""#);
    write_xml_attr(&note.language, xml);
    xml.push_str(r#"">"#);
    write_xml_text(&note.text, xml);
    xml.push_str("</cbc:Note>");
}

fn write_text_element(xml: &mut String, name: &str, value: &str) {
    write!(xml, "<{name}>").expect("writing to a String cannot fail");
    write_xml_text(value, xml);
    write!(xml, "</{name}>").expect("writing to a String cannot fail");
}

fn write_amount_element(xml: &mut String, name: &str, amount: Decimal, currency: &str) {
    write!(xml, r#"<{name} currencyID=""#).expect("writing to a String cannot fail");
    write_xml_attr(currency, xml);
    xml.push_str(r#"">"#);
    write_xml_text(&amount.to_string(), xml);
    write!(xml, "</{name}>").expect("writing to a String cannot fail");
}

fn write_xml_text(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(character),
        }
    }
}

fn write_xml_attr(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(character),
        }
    }
}

fn string_value<T: Serialize>(value: &T) -> Result<String, UblError> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or(UblError::MissingElement("serialized IR newtype string"))
}

fn decimal_value(path: &'static str, value: &str) -> Result<DecimalValue, UblError> {
    Decimal::from_str(value)
        .map(DecimalValue::new)
        .map_err(|_| UblError::InvalidDecimal {
            path,
            value: value.to_owned(),
        })
}

fn read_element_start(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
    current: Option<&NamespaceFrame>,
) -> Result<(ParsedElement, Vec<XmlAttribute>, NamespaceFrame), UblError> {
    let mut frame = current.cloned().unwrap_or_default();
    let mut attrs = Vec::new();
    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        let key = decode_xml_name(attr.key.as_ref())?;
        let value = attr
            .decoded_and_normalized_value(xml_version, reader.decoder())?
            .into_owned();
        if key == "xmlns" {
            frame.set("", value);
            continue;
        }
        if let Some(prefix) = key.strip_prefix("xmlns:") {
            frame.set(prefix, value);
            continue;
        }
        let (_, local_name) = split_qname(&key);
        attrs.push(XmlAttribute {
            local_name: local_name.to_owned(),
            value,
        });
    }
    let element = read_element_name(start.name().as_ref(), &frame)?;
    Ok((element, attrs, frame))
}

fn read_element_end(
    raw: &[u8],
    current: Option<&NamespaceFrame>,
) -> Result<ParsedElement, UblError> {
    let frame = current.ok_or_else(|| UblError::UnsupportedRoot("missing namespace".to_owned()))?;
    read_element_name(raw, frame)
}

fn read_element_name(raw: &[u8], frame: &NamespaceFrame) -> Result<ParsedElement, UblError> {
    let name = decode_xml_name(raw)?;
    let (prefix, local_name) = split_qname(&name);
    let namespace_uri =
        if prefix.is_empty() {
            frame.lookup("")
        } else {
            Some(frame.lookup(prefix).ok_or_else(|| {
                UblError::InvalidName(format!("unbound namespace prefix `{prefix}`"))
            })?)
        };
    Ok(ParsedElement {
        local_name: local_name.to_owned(),
        namespace_uri,
    })
}

fn write_start_element(
    xml: &mut String,
    element: &ParsedElement,
    attrs: &[XmlAttribute],
) -> Result<(), UblError> {
    let name = ubl_element_qname(element)?;
    write!(xml, "<{name}").expect("writing to a String cannot fail");
    for attr in attrs {
        write!(xml, " {}", attr.local_name).expect("writing to a String cannot fail");
        xml.push_str("=\"");
        write_xml_attr(&attr.value, xml);
        xml.push('"');
    }
    xml.push('>');
    Ok(())
}

fn ubl_element_qname(element: &ParsedElement) -> Result<String, UblError> {
    let Some(namespace) = element.namespace_uri.as_deref() else {
        return Ok(element.local_name.clone());
    };
    let prefix = match namespace {
        UBL_CBC_NAMESPACE_URI => "cbc",
        UBL_CAC_NAMESPACE_URI => "cac",
        UBL_EXT_NAMESPACE_URI => "ext",
        INVOICEKIT_EXTENSION_NAMESPACE_URI => "ik",
        UBL_INVOICE_NAMESPACE_URI | UBL_CREDIT_NOTE_NAMESPACE_URI => "",
        other => {
            return Err(UblError::InvalidName(format!(
                "unsupported preserved namespace `{other}`"
            )))
        }
    };
    if prefix.is_empty() {
        Ok(element.local_name.clone())
    } else {
        Ok(format!("{prefix}:{}", element.local_name))
    }
}

fn attr_value<'a>(attrs: &'a [XmlAttribute], local_name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.local_name == local_name)
        .map(|attr| attr.value.as_str())
}

fn decode_xml_name(raw: &[u8]) -> Result<String, UblError> {
    std::str::from_utf8(raw)
        .map(ToOwned::to_owned)
        .map_err(|_| UblError::InvalidName(String::from_utf8_lossy(raw).into_owned()))
}

fn split_qname(name: &str) -> (&str, &str) {
    name.split_once(':')
        .map_or(("", name), |(prefix, local_name)| (prefix, local_name))
}

fn resolve_xml_reference(reference: &str) -> Result<String, UblError> {
    match reference {
        "amp" => Ok("&".to_owned()),
        "lt" => Ok("<".to_owned()),
        "gt" => Ok(">".to_owned()),
        "apos" => Ok("'".to_owned()),
        "quot" => Ok("\"".to_owned()),
        other => Err(UblError::UnsupportedRoot(format!("entity:{other}"))),
    }
}

fn path_ends(stack: &[ParsedElement], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack
            .iter()
            .rev()
            .take(suffix.len())
            .zip(suffix.iter().rev())
            .all(|(left, right)| left.local_name == *right)
}

fn path_ends_ns(stack: &[ParsedElement], suffix: &[(&str, &str)]) -> bool {
    stack.len() >= suffix.len()
        && stack
            .iter()
            .rev()
            .take(suffix.len())
            .zip(suffix.iter().rev())
            .all(|(left, (name, namespace))| is_element(left, name, namespace))
}

fn in_any(stack: &[ParsedElement], names: &[&str]) -> bool {
    stack
        .iter()
        .any(|item| names.iter().any(|name| item.local_name == *name))
}

fn is_root_ubl_extensions(stack: &[ParsedElement]) -> bool {
    stack.len() == 2
        && stack
            .last()
            .is_some_and(|item| is_element(item, "UBLExtensions", UBL_EXT_NAMESPACE_URI))
}

fn in_top_level_ubl_extension(stack: &[ParsedElement]) -> bool {
    stack.len() >= 3
        && stack
            .get(1)
            .is_some_and(|item| is_element(item, "UBLExtensions", UBL_EXT_NAMESPACE_URI))
        && stack
            .get(2)
            .is_some_and(|item| is_element(item, "UBLExtension", UBL_EXT_NAMESPACE_URI))
}

fn is_root_child(stack: &[ParsedElement], child: &str) -> bool {
    stack.len() == 2 && stack.last().is_some_and(|name| name.local_name == child)
}

fn is_element(element: &ParsedElement, local_name: &str, namespace: &str) -> bool {
    element.local_name == local_name && element.namespace_uri.as_deref() == Some(namespace)
}

fn party_role(stack: &[ParsedElement]) -> Option<PartyRole> {
    if in_any(stack, &["AccountingSupplierParty"]) {
        Some(PartyRole::Supplier)
    } else if in_any(stack, &["AccountingCustomerParty"]) {
        Some(PartyRole::Customer)
    } else if in_any(stack, &["PayeeParty"]) {
        Some(PartyRole::Payee)
    } else {
        None
    }
}

const fn document_kind_from_type(document_type: DocumentType) -> Option<UblDocumentKind> {
    match document_type {
        DocumentType::Invoice => Some(UblDocumentKind::Invoice),
        DocumentType::CreditNote => Some(UblDocumentKind::CreditNote),
        DocumentType::DebitNote | DocumentType::ProForma | DocumentType::SelfBilled => None,
    }
}

fn should_preserve_top_level(
    stack: &[ParsedElement],
    element: &ParsedElement,
    document_kind: UblDocumentKind,
) -> bool {
    if stack.len() != 1 {
        return false;
    }
    let Ok(qname) = ubl_element_qname(element) else {
        return false;
    };
    if qname == "cbc:AccountingCost" || qname == "cbc:BuyerReference" {
        return false;
    }
    coverage_for(document_kind, &qname).is_some_and(|row| {
        matches!(
            row.class,
            UblCoverageClass::ProfileExtensionPayload
                | UblCoverageClass::LossinessLedgerPreserved
                | UblCoverageClass::UblDocumentFieldExtension
                | UblCoverageClass::UnsupportedGap
        )
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        coverage_for, crate_name, from_xml, to_xml, top_level_coverage,
        validate_oasis_ubl_2_1_schema, UblCoverageClass, UblDocumentKind, UblError, BEAD_ID,
        CORE_CUSTOMIZATION_ID, CORE_PROFILE_ID, INVOICEKIT_EXTENSION_NAMESPACE_URI,
        INVOICEKIT_METADATA_EXTENSION_URN, OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES,
        UBL_CAC_NAMESPACE_URI, UBL_CBC_NAMESPACE_URI, UBL_DOCUMENT_FIELDS_EXTENSION_URN,
        UBL_EXT_NAMESPACE_URI, UBL_TOP_LEVEL_ELEMENT_KEY, UBL_TOP_LEVEL_KEY,
    };
    use invoicekit_canonical::canonicalize_xml;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        JurisdictionExtension, LocalizedString, MonetaryTotal, Party, PartyTaxId,
        PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion,
        TaxCategorySummary,
    };
    use rust_decimal::Decimal;
    use serde_json::{json, Value};

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-ubl");
    }

    fn parse_document(xml: &str) -> CommercialDocument {
        let (document, ledger) = from_xml(xml).unwrap();
        assert!(
            ledger.lost.is_empty(),
            "successful UBL fixture parse should not report lost IR fields"
        );
        document
    }

    #[test]
    fn invoice_round_trip_preserves_core_ir() {
        let document = fixture(DocumentType::Invoice, 1);
        let xml = to_xml(&document).unwrap();
        let parsed = parse_document(&xml);
        assert_eq!(parsed, document);
    }

    #[test]
    fn credit_note_round_trip_preserves_core_ir() {
        let document = fixture(DocumentType::CreditNote, 2);
        let xml = to_xml(&document).unwrap();
        let parsed = parse_document(&xml);
        assert_eq!(parsed, document);
    }

    #[test]
    fn serializer_is_canonical_and_byte_stable() {
        let document = fixture(DocumentType::Invoice, 3);
        let first = to_xml(&document).unwrap();
        let second = to_xml(&document).unwrap();
        assert_eq!(first, second);
        assert_eq!(canonicalize_xml(&first).unwrap(), first);
    }

    #[test]
    fn coverage_matrix_pins_official_top_level_counts() {
        assert_eq!(top_level_coverage(UblDocumentKind::Invoice).len(), 54);
        assert_eq!(top_level_coverage(UblDocumentKind::CreditNote).len(), 51);

        assert_eq!(
            coverage_for(UblDocumentKind::Invoice, "cbc:BuyerReference")
                .unwrap()
                .class,
            UblCoverageClass::UblDocumentFieldExtension
        );
        assert_eq!(
            coverage_for(UblDocumentKind::Invoice, "cbc:AccountingCost")
                .unwrap()
                .class,
            UblCoverageClass::UblDocumentFieldExtension
        );
        assert_eq!(
            coverage_for(UblDocumentKind::CreditNote, "cac:DiscrepancyResponse")
                .unwrap()
                .class,
            UblCoverageClass::LossinessLedgerPreserved
        );
    }

    #[test]
    fn coverage_matrix_has_no_unsupported_top_level_gaps() {
        let unsupported = [UblDocumentKind::Invoice, UblDocumentKind::CreditNote]
            .into_iter()
            .flat_map(top_level_coverage)
            .filter(|row| row.class == UblCoverageClass::UnsupportedGap)
            .map(|row| row.element)
            .collect::<Vec<_>>();

        assert!(
            unsupported.is_empty(),
            "unsupported UBL top-level rows must be preserved or mapped: {unsupported:?}"
        );
    }

    #[test]
    fn metadata_round_trip_uses_ubl_extension_not_business_fields() {
        let document = fixture(DocumentType::Invoice, 6);
        let xml = to_xml(&document).unwrap();

        assert!(xml.contains("<ext:UBLExtensions"));
        assert!(xml.contains("<ik:DocumentMeta"));
        assert!(xml.contains("TenantID"));
        assert!(xml.contains("tenant-6"));
        assert!(xml.contains("TraceID"));
        assert!(xml.contains("trace-6"));
        assert!(!xml.contains("BuyerReference"));
        assert!(!xml.contains("AccountingCost"));

        let parsed = parse_document(&xml);
        assert_eq!(parsed.meta, document.meta);
    }

    #[test]
    fn incoming_business_fields_are_preserved_as_ubl_extension() {
        let document = fixture(DocumentType::Invoice, 7);
        let injected = format!(
            r#"<cbc:AccountingCost xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">COST-42</cbc:AccountingCost><cbc:BuyerReference xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">BUYER-PO-7</cbc:BuyerReference><cac:AccountingSupplierParty"#
        );
        let xml = to_xml(&document)
            .unwrap()
            .replace("<cac:AccountingSupplierParty", &injected);

        let parsed = parse_document(&xml);
        assert_eq!(parsed.meta.tenant_id, "tenant-7");
        assert_eq!(parsed.meta.trace_id, "trace-7");

        let payload = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
            .map(|extension| &extension.payload)
            .unwrap();
        assert_eq!(payload["accounting_cost"], "COST-42");
        assert_eq!(payload["buyer_reference"], "BUYER-PO-7");
    }

    #[test]
    fn bare_payment_means_code_drop_is_recorded_in_lossiness_ledger() {
        // A schema-valid cac:PaymentMeans that carries only cbc:PaymentMeansCode
        // (no cbc:PaymentID and no cac:PayeeFinancialAccount) has no home in the
        // core invoice model, so the parser drops it. That drop must be visible in
        // the lossiness ledger rather than vanishing silently.
        let document = fixture(DocumentType::Invoice, 21);
        let bare_payment_means = format!(
            r#"<cac:PaymentMeans xmlns:cac="{UBL_CAC_NAMESPACE_URI}" xmlns:cbc="{UBL_CBC_NAMESPACE_URI}"><cbc:PaymentMeansCode>10</cbc:PaymentMeansCode></cac:PaymentMeans>"#
        );
        // Inject the bare element ahead of the fixture's real payment means,
        // keeping the original element intact. The canonical serializer pins a
        // namespace declaration on the original opening tag, so match the prefix.
        let serialized = to_xml(&document).unwrap();
        let anchor = "<cac:PaymentMeans";
        assert!(
            serialized.contains(anchor),
            "fixture must serialize a payment means to anchor the injection"
        );
        let xml = serialized.replacen(anchor, &format!("{bare_payment_means}{anchor}"), 1);
        assert_eq!(
            xml.matches(anchor).count(),
            2,
            "injection should leave the original payment means in place"
        );

        let (parsed, ledger) = from_xml(&xml).expect("bare payment means should still parse");

        // The bare element is dropped: only the original payment instruction survives.
        assert_eq!(parsed.payment_instructions, document.payment_instructions);

        let entry = ledger
            .lost
            .iter()
            .find(|entry| entry.path == "/payment_instructions")
            .expect("dropped bare PaymentMeans must produce a /payment_instructions lost entry");
        assert!(
            entry.reason.contains("PaymentMeans"),
            "ledger reason should name the dropped element: {}",
            entry.reason
        );
    }

    #[test]
    fn payment_means_with_payee_account_does_not_record_loss() {
        // Control: a real payment means (with a payee financial account) must
        // survive parsing and must NOT add a spurious lossiness entry.
        let document = fixture(DocumentType::Invoice, 22);
        let xml = to_xml(&document).unwrap();
        let (parsed, ledger) = from_xml(&xml).expect("fixture invoice should parse");
        assert_eq!(parsed.payment_instructions, document.payment_instructions);
        assert!(
            !ledger
                .lost
                .iter()
                .any(|entry| entry.path == "/payment_instructions"),
            "a preserved payment means must not report a lost /payment_instructions entry"
        );
    }

    #[test]
    fn ubl_document_field_extension_round_trips_through_business_fields() {
        let mut document = fixture(DocumentType::Invoice, 8);
        document.extensions.push(
            JurisdictionExtension::new(
                UBL_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "accounting_cost": "COST-8",
                    "buyer_reference": "BUYER-8"
                }),
            )
            .unwrap(),
        );

        let xml = to_xml(&document).unwrap();
        assert!(xml.contains("AccountingCost"));
        assert!(xml.contains("BuyerReference"));
        assert_eq!(parse_document(&xml), document);
    }

    #[test]
    fn non_core_top_level_fields_are_preserved_and_replayed() {
        let document = fixture(DocumentType::Invoice, 14);
        let ubl_version = format!(
            r#"<cbc:UBLVersionID xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">2.1</cbc:UBLVersionID>"#
        );
        let profile_execution = format!(
            r#"<cbc:ProfileExecutionID xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">exec-14</cbc:ProfileExecutionID>"#
        );
        let copy_indicator = format!(
            r#"<cbc:CopyIndicator xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">false</cbc:CopyIndicator>"#
        );
        let issue_time = format!(
            r#"<cbc:IssueTime xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">12:00:00</cbc:IssueTime>"#
        );
        let currency_fields = format!(
            r#"<cbc:TaxCurrencyCode xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">USD</cbc:TaxCurrencyCode><cbc:AccountingCostCode xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">DEPT-14</cbc:AccountingCostCode><cbc:LineCountNumeric xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">1</cbc:LineCountNumeric>"#
        );
        let order_reference = format!(
            r#"<cac:OrderReference xmlns:cac="{UBL_CAC_NAMESPACE_URI}" xmlns:cbc="{UBL_CBC_NAMESPACE_URI}"><cbc:ID>ORDER-14</cbc:ID></cac:OrderReference>"#
        );
        let xml = to_xml(&document)
            .unwrap()
            .replacen(
                "<cbc:CustomizationID",
                &format!("{ubl_version}<cbc:CustomizationID"),
                1,
            )
            .replace(CORE_CUSTOMIZATION_ID, "urn:example:customization")
            .replace(CORE_PROFILE_ID, "urn:example:profile")
            .replacen(
                "</cbc:ProfileID><cbc:ID",
                &format!("</cbc:ProfileID>{profile_execution}<cbc:ID"),
                1,
            )
            .replacen("<cbc:UUID", &format!("{copy_indicator}<cbc:UUID"), 1)
            .replacen(
                "</cbc:IssueDate><cbc:DueDate",
                &format!("</cbc:IssueDate>{issue_time}<cbc:DueDate"),
                1,
            )
            .replacen(
                "</cbc:DocumentCurrencyCode>",
                &format!("</cbc:DocumentCurrencyCode>{currency_fields}"),
                1,
            )
            .replacen(
                "<cac:AccountingSupplierParty",
                &format!("{order_reference}<cac:AccountingSupplierParty"),
                1,
            );

        let parsed = parse_document(&xml);
        let payload = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
            .map(|extension| &extension.payload)
            .unwrap();
        let preserved = payload
            .get(UBL_TOP_LEVEL_KEY)
            .and_then(Value::as_array)
            .unwrap();

        for element in [
            "cbc:UBLVersionID",
            "cbc:CustomizationID",
            "cbc:ProfileID",
            "cbc:ProfileExecutionID",
            "cbc:CopyIndicator",
            "cbc:IssueTime",
            "cbc:TaxCurrencyCode",
            "cbc:AccountingCostCode",
            "cbc:LineCountNumeric",
            "cac:OrderReference",
        ] {
            assert!(
                preserved.iter().any(|item| {
                    item.get(UBL_TOP_LEVEL_ELEMENT_KEY).and_then(Value::as_str) == Some(element)
                }),
                "missing preserved top-level element {element}"
            );
        }

        let serialized = to_xml(&parsed).unwrap();
        assert!(serialized.contains("urn:example:customization"));
        assert!(serialized.contains("urn:example:profile"));
        assert!(serialized.contains("TaxCurrencyCode"));
        assert!(serialized.contains(">USD<"));
        assert!(serialized.contains("OrderReference"));
        assert_eq!(parse_document(&serialized), parsed);

        let schema_report = validate_oasis_ubl_2_1_schema(&serialized).unwrap();
        assert!(
            schema_report.is_valid(),
            "expected preserved-field output to be schema valid, findings: {:?}",
            schema_report.findings
        );
    }

    #[test]
    fn rejects_mismatched_preserved_top_level_fragment() {
        let mut document = fixture(DocumentType::Invoice, 15);
        document.extensions.push(
            JurisdictionExtension::new(
                UBL_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    UBL_TOP_LEVEL_KEY: [{
                        UBL_TOP_LEVEL_ELEMENT_KEY: "cbc:UBLVersionID",
                        "xml": "<cbc:ProfileID>not-a-version</cbc:ProfileID>"
                    }]
                }),
            )
            .unwrap(),
        );

        let err = to_xml(&document).unwrap_err();
        assert!(matches!(
            err,
            UblError::InvalidPreservedTopLevel { element, .. }
                if element == "cbc:UBLVersionID"
        ));
    }

    #[test]
    fn rejects_mismatched_preserved_top_level_closing_tag() {
        let mut document = fixture(DocumentType::Invoice, 16);
        document.extensions.push(
            JurisdictionExtension::new(
                UBL_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    UBL_TOP_LEVEL_KEY: [{
                        UBL_TOP_LEVEL_ELEMENT_KEY: "cbc:UBLVersionID",
                        "xml": "<cbc:UBLVersionID>2.1</cbc:ProfileID>"
                    }]
                }),
            )
            .unwrap(),
        );

        let err = to_xml(&document).unwrap_err();
        assert!(matches!(
            err,
            UblError::InvalidPreservedTopLevel { element, .. }
                if element == "cbc:UBLVersionID"
        ));
    }

    #[test]
    fn credit_note_serializer_does_not_emit_due_date() {
        let document = fixture(DocumentType::CreditNote, 9);
        let xml = to_xml(&document).unwrap();
        assert!(!xml.contains("DueDate"));
    }

    #[test]
    fn credit_note_serializer_rejects_root_due_date() {
        let mut document = fixture(DocumentType::CreditNote, 10);
        document.due_date = Some(DateOnly::new("2026-06-25").unwrap());

        let err = to_xml(&document).unwrap_err();
        assert!(matches!(
            err,
            UblError::UnsupportedDocumentField {
                document_type: DocumentType::CreditNote,
                field: "due_date",
                ..
            }
        ));
    }

    #[test]
    fn metadata_extension_requires_matching_extension_uri() {
        let document = fixture(DocumentType::Invoice, 11);
        let xml = to_xml(&document)
            .unwrap()
            .replace(INVOICEKIT_METADATA_EXTENSION_URN, "urn:foreign:metadata");

        let parsed = parse_document(&xml);
        assert_eq!(parsed.meta.tenant_id, "ubl-import");
        assert!(parsed.meta.trace_id.starts_with(BEAD_ID));
        assert_ne!(parsed.meta, document.meta);
    }

    #[test]
    fn metadata_extension_requires_invoicekit_namespace() {
        let document = fixture(DocumentType::Invoice, 12);
        let xml = to_xml(&document)
            .unwrap()
            .replace(INVOICEKIT_EXTENSION_NAMESPACE_URI, "urn:foreign:invoicekit");

        let parsed = parse_document(&xml);
        assert_eq!(parsed.meta.tenant_id, "ubl-import");
        assert!(parsed.meta.trace_id.starts_with(BEAD_ID));
        assert_ne!(parsed.meta, document.meta);
    }

    #[test]
    fn metadata_extension_requires_top_level_ubl_extensions_container() {
        let document = fixture(DocumentType::Invoice, 13);
        let fake_extension = format!(
            r#"<ext:UBLExtension xmlns:ext="{UBL_EXT_NAMESPACE_URI}"><ext:ExtensionURI>{INVOICEKIT_METADATA_EXTENSION_URN}</ext:ExtensionURI><ext:ExtensionContent><ik:DocumentMeta xmlns:ik="{INVOICEKIT_EXTENSION_NAMESPACE_URI}"><ik:TenantID>foreign-tenant</ik:TenantID><ik:TraceID>foreign-trace</ik:TraceID></ik:DocumentMeta></ext:ExtensionContent></ext:UBLExtension><cac:Party>"#
        );
        let xml = to_xml(&document)
            .unwrap()
            .replacen("<cac:Party>", &fake_extension, 1);

        let parsed = parse_document(&xml);
        assert_eq!(parsed.meta, document.meta);
    }

    #[test]
    fn generated_fifty_fixture_round_trips_pass() {
        for seed in 0..50 {
            let document = fixture(
                if seed % 2 == 0 {
                    DocumentType::Invoice
                } else {
                    DocumentType::CreditNote
                },
                seed,
            );
            let xml = to_xml(&document).unwrap();
            assert_eq!(parse_document(&xml), document);
        }
    }

    #[test]
    fn oasis_ubl_2_1_schema_validated_fixtures_are_documented() {
        assert_eq!(OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES.len(), 2);
        assert_eq!(
            OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES[0].document_kind,
            UblDocumentKind::Invoice
        );
        assert_eq!(
            OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES[1].document_kind,
            UblDocumentKind::CreditNote
        );
    }

    #[test]
    fn serialized_invoice_fixture_passes_oasis_ubl_2_1_xsd() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 20)).unwrap();
        let report = validate_oasis_ubl_2_1_schema(&xml).unwrap();

        assert_eq!(report.document_kind, UblDocumentKind::Invoice);
        assert_eq!(report.schema_file, "xsd/maindoc/UBL-Invoice-2.1.xsd");
        assert!(
            report.is_valid(),
            "expected schema-valid Invoice fixture, findings: {:?}",
            report.findings
        );
    }

    #[test]
    fn serialized_credit_note_fixture_passes_oasis_ubl_2_1_xsd() {
        let xml = to_xml(&fixture(DocumentType::CreditNote, 21)).unwrap();
        let report = validate_oasis_ubl_2_1_schema(&xml).unwrap();

        assert_eq!(report.document_kind, UblDocumentKind::CreditNote);
        assert_eq!(report.schema_file, "xsd/maindoc/UBL-CreditNote-2.1.xsd");
        assert!(
            report.is_valid(),
            "expected schema-valid CreditNote fixture, findings: {:?}",
            report.findings
        );
    }

    #[test]
    fn party_serializer_uses_schema_order_and_legal_name() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 23)).unwrap();
        let party_start = xml.find("<cac:AccountingSupplierParty").unwrap();
        let party_end = xml.find("</cac:AccountingSupplierParty>").unwrap();
        let party_xml = xml
            .get(party_start..party_end)
            .expect("supplier party XML tag offsets must be byte aligned");
        let postal = party_xml.find("<cac:PostalAddress>").unwrap();
        let tax = party_xml.find("<cac:PartyTaxScheme>").unwrap();
        let legal = party_xml.find("<cac:PartyLegalEntity>").unwrap();
        let contact = party_xml.find("<cac:Contact>").unwrap();

        assert!(
            postal < tax && tax < legal && legal < contact,
            "cac:Party children must preserve UBL schema order: {party_xml}"
        );
        assert!(party_xml.contains(">Supplier GmbH</cbc:RegistrationName>"));
    }

    #[test]
    fn schema_harness_rejects_invalid_credit_note_due_date() {
        let xml = to_xml(&fixture(DocumentType::CreditNote, 22))
            .unwrap()
            .replacen(
                "<cbc:DocumentCurrencyCode",
                "<cbc:DueDate xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">2026-06-25</cbc:DueDate><cbc:DocumentCurrencyCode",
                1,
            );
        let report = validate_oasis_ubl_2_1_schema(&xml).unwrap();

        assert!(!report.is_valid());
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.message.contains("DueDate")),
            "expected DueDate validation finding, got {:?}",
            report.findings
        );
    }

    #[test]
    fn rejects_unsupported_root() {
        let err = from_xml("<Order/>").unwrap_err();
        assert!(matches!(err, UblError::UnsupportedRoot(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        let xml = r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#;
        let err = from_xml(xml).unwrap_err();
        assert!(matches!(err, UblError::MissingElement(_)));
    }

    #[test]
    fn rejects_invalid_decimal() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 4))
            .unwrap()
            .replacen(">100.04<", ">not-decimal<", 1);
        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(err, UblError::InvalidDecimal { .. }));
    }

    #[test]
    fn rejects_unsupported_document_type_on_serialize() {
        let mut document = fixture(DocumentType::Invoice, 5);
        document.document_type = DocumentType::DebitNote;
        let err = to_xml(&document).unwrap_err();
        assert!(matches!(err, UblError::UnsupportedDocumentType(_)));
    }

    proptest! {
        #[test]
        fn parse_serialize_parse_is_stable(seed in 0_u32..128) {
            let document = fixture(DocumentType::Invoice, seed);
            let xml = to_xml(&document).unwrap();
            let parsed = parse_document(&xml);
            let reparsed = parse_document(&to_xml(&parsed).unwrap());
            prop_assert_eq!(parsed, reparsed);
        }
    }

    fn fixture(document_type: DocumentType, seed: u32) -> CommercialDocument {
        let base = Decimal::new(10000 + i64::from(seed), 2);
        let tax = Decimal::new(1900, 2);
        let total = base + tax;
        let amount = DecimalValue::new(base);
        let supplier = party("supplier", "Supplier GmbH", "DE123456789");
        let customer = party("customer", "Customer BV", "NL123456789B01");
        let due_date =
            (document_type == DocumentType::Invoice).then(|| DateOnly::new("2026-06-25").unwrap());

        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(format!("doc-{seed}")).unwrap(),
            document_type,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: Some(DateOnly::new("2026-05-26").unwrap()),
            due_date: due_date.clone(),
            document_number: DocumentNumber::new(format!("INV-{seed:04}")).unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier,
            customer,
            payee: Some(party("payee", "Payee GmbH", "DE987654321")),
            payment_terms: Some(PaymentTerms {
                description: "Payable within 30 days".to_owned(),
                due_date,
            }),
            payment_instructions: vec![PaymentInstruction {
                kind: PaymentInstructionKind::IbanBic,
                account: Some(format!("DE893704004405320130{seed:02}")),
                reference: Some(format!("RF{seed:04}")),
            }],
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: format!("Implementation service {seed}"),
                quantity: DecimalValue::new(Decimal::ONE),
                unit_code: Some("EA".to_owned()),
                unit_price: amount.clone(),
                line_extension_amount: amount.clone(),
                tax_category: Some("S".to_owned()),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amount.clone(),
                tax_amount: DecimalValue::new(tax),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amount.clone(),
                tax_exclusive_amount: amount,
                tax_inclusive_amount: DecimalValue::new(total),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: DecimalValue::new(total),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: vec![LocalizedString {
                language: "en".to_owned(),
                text: format!("Fixture note {seed}"),
            }],
            extensions: Vec::new(),
            meta: DocumentMeta {
                tenant_id: format!("tenant-{seed}"),
                trace_id: format!("trace-{seed}"),
                source_system: None,
            },
        })
        .unwrap()
    }

    fn party(id: &str, name: &str, vat: &str) -> Party {
        Party {
            id: Some(id.to_owned()),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: vat.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["Main Street 1".to_owned(), "Suite 2".to_owned()],
                city: "Berlin".to_owned(),
                subdivision: Some("BE".to_owned()),
                postal_code: "10115".to_owned(),
                country: CountryCode::new("DE").unwrap(),
            },
            contact: Some(Contact {
                name: Some(format!("{name} Contact")),
                email: Some(format!("{}@example.test", id.replace('-', ""))),
                phone: Some("+49-30-000000".to_owned()),
            }),
        }
    }
}
