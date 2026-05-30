// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! UN/CEFACT Cross Industry Invoice parser and serializer for InvoiceKit IR.
//!
//! This crate maps the CII D16B `CrossIndustryInvoice` syntax used by
//! Factur-X/ZUGFeRD into the core [`invoicekit_ir::CommercialDocument`] fields
//! that exist today. CII standard fields without core IR homes are preserved as
//! CII document-field extensions instead of being overloaded as InvoiceKit
//! operational metadata. The serializer emits deterministic CII XML and then
//! canonicalizes it with [`invoicekit_canonical::canonicalize_xml`].

use std::str::FromStr as _;

use invoicekit_canonical::{canonicalize_xml, XmlCanonicalizeError};
use invoicekit_ir::{
    Attachment, CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly,
    DecimalValue, DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference,
    DocumentType, IrError, Iso4217Code, ItemClassification, JurisdictionExtension, LocalizedString,
    LossinessLedger, MonetaryTotal, MoneyAmount, Party, PartyTaxId, PaymentInstruction,
    PaymentInstructionKind, PaymentTerms, PostalAddress, Quantity, ReferenceKindClass,
    SchemaVersion, TaxCategorySummary,
};
use quick_xml::events::{attributes::AttrError, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

pub mod mapping;

const BEAD_ID: &str = "invoices-t-041-cii-parser-serializer-gyl";
const CII_RSM_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
const CII_RAM_NAMESPACE_URI: &str =
    "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
const CII_UDT_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100";
const CII_QDT_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:QualifiedDataType:100";
const CORE_GUIDELINE_ID: &str = "urn:cen.eu:en16931:2017";
const DEFAULT_LANGUAGE: &str = "und";
const CII_PRESERVED_XML_KEY: &str = "preserved_xml";
const CII_PRESERVED_CONTAINER_KEY: &str = "container";
const CII_PRESERVED_ELEMENT_KEY: &str = "element";
const CII_PRESERVED_XML_FRAGMENT_KEY: &str = "xml";

/// Errors returned by [`to_xml`] and [`from_xml`].
#[derive(Debug, Error)]
pub enum CiiError {
    /// The XML input was not well formed.
    #[error(
        "CII XML is not well formed: {0}; hint: pass a complete CrossIndustryInvoice document"
    )]
    InvalidXml(#[from] quick_xml::Error),
    /// An XML attribute was malformed.
    #[error("CII XML attribute is invalid: {0}; hint: check CII namespace and typed attributes")]
    InvalidAttribute(#[from] AttrError),
    /// Text or attribute content could not be decoded.
    #[error(
        "CII XML text encoding is invalid: {0}; hint: InvoiceKit expects UTF-8 compatible XML"
    )]
    InvalidEncoding(#[from] quick_xml::encoding::EncodingError),
    /// A tag or attribute name was not UTF-8.
    #[error("CII XML name `{0}` is not valid UTF-8; hint: use UTF-8 element and attribute names")]
    InvalidName(String),
    /// The root element is not CII `CrossIndustryInvoice`.
    #[error("unsupported CII root `{0}`; hint: use rsm:CrossIndustryInvoice")]
    UnsupportedRoot(String),
    /// A CII element used the wrong namespace URI for its local name.
    #[error(
        "invalid CII namespace for `{element}`: expected `{expected}`, got `{actual}`; hint: use the UN/CEFACT CII D16B rsm/ram/udt namespaces"
    )]
    InvalidNamespace {
        /// CII local element name.
        element: String,
        /// Expected namespace URI.
        expected: &'static str,
        /// Resolved namespace diagnostic.
        actual: String,
    },
    /// The CII type code is not mapped to the current InvoiceKit IR.
    #[error("unsupported CII document type code `{0}`; hint: use 380 for invoice or 381 for credit note")]
    UnsupportedTypeCode(String),
    /// The IR document type cannot be serialized as this CII family member.
    #[error("document type `{0:?}` is not supported by the CII serializer; hint: use Invoice or CreditNote")]
    UnsupportedDocumentType(DocumentType),
    /// A required CII element was missing.
    #[error("missing required CII element `{0}`; hint: include the element needed to build InvoiceKit IR")]
    MissingElement(&'static str),
    /// A decimal field could not be parsed.
    #[error("invalid decimal `{value}` at `{path}`; hint: use a fixed-scale decimal string")]
    InvalidDecimal {
        /// CII element path.
        path: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A CII date field could not be parsed.
    #[error("invalid CII date `{value}` at `{path}`; hint: use format=102 YYYYMMDD dates")]
    InvalidDate {
        /// CII element path.
        path: &'static str,
        /// Invalid value.
        value: String,
    },
    /// IR validation failed after parsing.
    #[error("parsed CII did not satisfy InvoiceKit IR validation: {0}")]
    InvalidIr(#[from] IrError),
    /// JSON conversion failed while reading opaque IR newtypes or metadata.
    #[error("could not convert InvoiceKit IR helper value: {0}")]
    InvalidIrJson(#[from] serde_json::Error),
    /// Canonical XML output could not be produced.
    #[error("could not canonicalize CII XML output: {0}")]
    Canonicalize(#[from] XmlCanonicalizeError),
    /// A preserved CII fragment in an extension payload is not safe to replay.
    #[error(
        "invalid preserved CII fragment for `{element}` in `{container}`: {message}; hint: preserve fragments produced by from_xml or remove the invalid CII document-fields payload"
    )]
    InvalidPreservedXml {
        /// Expected CII container path.
        container: String,
        /// Expected CII element name.
        element: String,
        /// Validation diagnostic.
        message: String,
    },
}

/// Serialize an InvoiceKit commercial document into deterministic CII XML.
///
/// The returned XML has passed through InvoiceKit XML canonicalization, so
/// serializing the same document twice on the same platform returns identical
/// bytes.
///
/// # Errors
///
/// Returns [`CiiError::UnsupportedDocumentType`] for document types other than
/// [`DocumentType::Invoice`] and [`DocumentType::CreditNote`], or a canonical
/// XML error if the generated XML cannot be canonicalized.
///
/// # Examples
///
/// ```
/// # use invoicekit_format_cii::to_xml;
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
/// #         id: DocumentId::new("INV-1").unwrap(),
/// #         document_type: DocumentType::Invoice,
/// #         issue_date: DateOnly::new("2026-05-26").unwrap(),
/// #         tax_point_date: None,
/// #         due_date: None,
/// #         invoice_period: None,
/// #         delivery_date: None,
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
/// #             unit_code: Some("C62".to_owned()),
/// #             unit_price: amount.clone(),
/// #             line_extension_amount: amount.clone(),
/// #             tax_category: Some("S".to_owned()),
/// #             classifications: Vec::new(),
/// #             extensions: Vec::new(),
/// #         }],
/// #         tax_summary: vec![TaxCategorySummary {
/// #             category_code: "S".to_owned(),
/// #             taxable_amount: amount.clone(),
/// #             tax_amount: DecimalValue::new(Decimal::new(1900, 2)),
/// #             tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
/// #             exemption_reason: None,
/// #             exemption_reason_code: None,
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
/// #         allowance_charges: Vec::new(),
/// #         meta: DocumentMeta {
/// #             tenant_id: "tenant".to_owned(),
/// #             trace_id: "trace".to_owned(),
/// #             source_system: None,
/// #         },
/// #     }).unwrap()
/// # }
/// let xml = to_xml(&fixture()).unwrap();
/// assert!(xml.contains("<rsm:CrossIndustryInvoice"));
/// ```
pub fn to_xml(document: &CommercialDocument) -> Result<String, CiiError> {
    document.validate()?;
    let raw = serialize_document(document)?;
    Ok(canonicalize_xml(&raw)?)
}

/// Parse a CII D16B `CrossIndustryInvoice` document into InvoiceKit IR.
///
/// The parser extracts the current core IR surface. CII elements that do not
/// have an IR field yet are preserved as canonical raw XML fragments in the CII
/// document-fields extension so parse/serialize operations do not silently
/// discard standard CII document data.
///
/// # Errors
///
/// Returns a typed [`CiiError`] when XML is malformed, the root is not
/// `CrossIndustryInvoice`, required IR fields are absent, decimal/date values
/// are invalid, or the resulting IR does not validate.
///
/// # Examples
///
/// ```
/// # let xml = r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100" xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100" xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100"><rsm:ExchangedDocument><ram:ID>INV-1</ram:ID><ram:TypeCode>380</ram:TypeCode><ram:IssueDateTime><udt:DateTimeString format="102">20260526</udt:DateTimeString></ram:IssueDateTime></rsm:ExchangedDocument><rsm:SupplyChainTradeTransaction><ram:IncludedSupplyChainTradeLineItem><ram:AssociatedDocumentLineDocument><ram:LineID>1</ram:LineID></ram:AssociatedDocumentLineDocument><ram:SpecifiedTradeProduct><ram:Name>Service</ram:Name></ram:SpecifiedTradeProduct><ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice><ram:ChargeAmount>100.00</ram:ChargeAmount></ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement><ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery><ram:SpecifiedLineTradeSettlement><ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation></ram:SpecifiedLineTradeSettlement></ram:IncludedSupplyChainTradeLineItem><ram:ApplicableHeaderTradeAgreement><ram:SellerTradeParty><ram:Name>Supplier</ram:Name><ram:PostalTradeAddress><ram:LineOne>Main</ram:LineOne><ram:CityName>Berlin</ram:CityName><ram:PostcodeCode>10115</ram:PostcodeCode><ram:CountryID>DE</ram:CountryID></ram:PostalTradeAddress></ram:SellerTradeParty><ram:BuyerTradeParty><ram:Name>Customer</ram:Name><ram:PostalTradeAddress><ram:LineOne>Main</ram:LineOne><ram:CityName>Berlin</ram:CityName><ram:PostcodeCode>10115</ram:PostcodeCode><ram:CountryID>DE</ram:CountryID></ram:PostalTradeAddress></ram:BuyerTradeParty></ram:ApplicableHeaderTradeAgreement><ram:ApplicableHeaderTradeSettlement><ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode><ram:SpecifiedTradeSettlementHeaderMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount><ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount><ram:GrandTotalAmount>119.00</ram:GrandTotalAmount><ram:DuePayableAmount>119.00</ram:DuePayableAmount></ram:SpecifiedTradeSettlementHeaderMonetarySummation></ram:ApplicableHeaderTradeSettlement></rsm:SupplyChainTradeTransaction></rsm:CrossIndustryInvoice>"#;
/// let (parsed, ledger) = invoicekit_format_cii::from_xml(xml).unwrap();
/// assert_eq!(parsed.document_type, invoicekit_ir::DocumentType::Invoice);
/// assert!(ledger.lost.is_empty());
/// ```
#[allow(clippy::too_many_lines)]
pub fn from_xml(input: &str) -> Result<(CommercialDocument, LossinessLedger), CiiError> {
    let document = parse_xml_document(input)?;
    let serialized = to_xml(&document)?;
    let reparsed = parse_xml_document(&serialized)?;
    let ledger = LossinessLedger::from_roundtrip_comparison(&document, &reparsed, "format-cii")?;
    Ok((document, ledger))
}

#[allow(clippy::too_many_lines)]
fn parse_xml_document(input: &str) -> Result<CommercialDocument, CiiError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut xml_version = XmlVersion::default();
    let mut stack = Vec::<String>::new();
    let mut text_stack = Vec::<String>::new();
    let mut namespace_stack = Vec::<Vec<XmlNamespaceBinding>>::new();
    let mut raw_capture = None::<RawXmlCapture>;
    let mut state = ParseState::default();

    loop {
        let event_start = reader_position(reader.buffer_position())?;
        let event = reader.read_event()?;
        let event_end = reader_position(reader.buffer_position())?;

        if let Some(capture) = raw_capture.as_mut() {
            match &event {
                Event::Start(_) => {
                    capture.depth += 1;
                }
                Event::Empty(_)
                | Event::Text(_)
                | Event::CData(_)
                | Event::GeneralRef(_)
                | Event::Decl(_)
                | Event::DocType(_)
                | Event::PI(_)
                | Event::Comment(_) => {}
                Event::End(_) => {
                    capture.depth = capture.depth.checked_sub(1).ok_or_else(|| {
                        CiiError::UnsupportedRoot(format!("preserved:{}", capture.element))
                    })?;
                    if capture.depth == 0 {
                        let capture = raw_capture
                            .take()
                            .ok_or_else(|| CiiError::UnsupportedRoot("preserved".to_owned()))?;
                        let start = capture.start;
                        state.push_preserved_xml(capture, input_slice(input, start, event_end)?)?;
                    }
                }
                Event::Eof => return Err(CiiError::UnsupportedRoot("preserved:Eof".to_owned())),
            }
            continue;
        }

        match event {
            Event::Start(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let namespace_declarations =
                    namespace_declarations_for_start(&reader, &start, xml_version)?;
                let namespace = resolve_element_namespace(
                    start.name().as_ref(),
                    &namespace_stack,
                    Some(&namespace_declarations),
                )?;
                validate_cii_namespace(&namespace, &stack, &name)?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                if should_preserve_raw_xml(&stack, &name) {
                    raw_capture = Some(RawXmlCapture::new(
                        &stack,
                        name,
                        event_start,
                        state.active_line_id(),
                        effective_namespace_bindings(&namespace_stack, &namespace_declarations),
                    ));
                    continue;
                }
                state.start_element(&stack, &name, &attrs)?;
                stack.push(name);
                text_stack.push(String::new());
                namespace_stack.push(namespace_declarations);
            }
            Event::Empty(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let namespace_declarations =
                    namespace_declarations_for_start(&reader, &start, xml_version)?;
                let namespace = resolve_element_namespace(
                    start.name().as_ref(),
                    &namespace_stack,
                    Some(&namespace_declarations),
                )?;
                validate_cii_namespace(&namespace, &stack, &name)?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                if should_preserve_raw_xml(&stack, &name) {
                    state.push_preserved_xml(
                        RawXmlCapture::new(
                            &stack,
                            name,
                            event_start,
                            state.active_line_id(),
                            effective_namespace_bindings(&namespace_stack, &namespace_declarations),
                        ),
                        input_slice(input, event_start, event_end)?,
                    )?;
                    continue;
                }
                state.start_element(&stack, &name, &attrs)?;
                state.end_element(&name)?;
            }
            Event::End(end) => {
                let name = decode_local_name(end.name().as_ref())?;
                let Some((opened, parent_stack)) = stack.split_last() else {
                    return Err(CiiError::UnsupportedRoot(name));
                };
                if opened != &name {
                    return Err(CiiError::UnsupportedRoot(format!("{opened}/{name}")));
                }
                let namespace =
                    resolve_element_namespace(end.name().as_ref(), &namespace_stack, None)?;
                validate_cii_namespace(&namespace, parent_stack, &name)?;
                let text = text_stack
                    .pop()
                    .ok_or_else(|| CiiError::UnsupportedRoot(format!("{name}:text")))?;
                if !text.is_empty() {
                    state.text(&stack, &text)?;
                }
                state.end_element(&name)?;
                stack.pop();
                namespace_stack.pop();
            }
            Event::Text(text) => {
                let text = text.xml_content(xml_version)?;
                append_text(&mut text_stack, text.as_ref())?;
            }
            Event::CData(cdata) => {
                let text = cdata.xml_content(xml_version)?;
                append_text(&mut text_stack, text.as_ref())?;
            }
            Event::GeneralRef(reference) => {
                let reference = reference.xml_content(xml_version)?;
                let text = resolve_xml_reference(&reference, xml_version)?;
                append_text(&mut text_stack, &text)?;
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
                return Err(CiiError::UnsupportedRoot("DOCTYPE".to_owned()));
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }

    state.finish()
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_format_cii::crate_name(), "invoicekit-format-cii");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-format-cii"
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
    document_number: Option<String>,
    issue_date: Option<String>,
    tax_point_date: Option<String>,
    due_date: Option<String>,
    currency: Option<String>,
    metadata_tenant_id: Option<String>,
    metadata_trace_id: Option<String>,
    metadata_source_system: Option<String>,
    cii_buyer_reference: Option<String>,
    cii_business_process_context_ids: Vec<String>,
    cii_guideline_context_ids: Vec<String>,
    cii_transaction_ids: Vec<String>,
    cii_test_indicators: Vec<String>,
    cii_application_contexts: Vec<CiiApplicationContext>,
    current_context_parameter: Option<DocumentContextParameterBuilder>,
    supplier: PartyBuilder,
    customer: PartyBuilder,
    payee: PartyBuilder,
    has_payee: bool,
    payment_terms_description: Option<String>,
    payment_reference: Option<String>,
    payment_instructions: Vec<PaymentInstruction>,
    current_payment: Option<PaymentBuilder>,
    lines: Vec<DocumentLine>,
    current_line: Option<LineBuilder>,
    tax_summary: Vec<TaxCategorySummary>,
    current_tax: Option<TaxSummaryBuilder>,
    monetary_total: MonetaryTotalBuilder,
    notes: Vec<LocalizedString>,
    preserved_xml: Vec<CiiPreservedXml>,
    current_line_preserved_start: Option<usize>,
}

impl ParseState {
    fn start_element(
        &mut self,
        stack: &[String],
        name: &str,
        attrs: &[XmlAttribute],
    ) -> Result<(), CiiError> {
        if stack.is_empty() && name != "CrossIndustryInvoice" {
            return Err(CiiError::UnsupportedRoot(name.to_owned()));
        }
        if let Some(kind) = DocumentContextKind::from_element(name) {
            self.current_context_parameter = Some(DocumentContextParameterBuilder::new(kind));
        }
        if name == "IncludedSupplyChainTradeLineItem" {
            self.current_line = Some(LineBuilder::default());
            self.current_line_preserved_start = Some(self.preserved_xml.len());
        }
        if name == "ApplicableTradeTax"
            && self.current_line.is_none()
            && in_any(stack, &["ApplicableHeaderTradeSettlement"])
        {
            self.current_tax = Some(TaxSummaryBuilder::default());
        }
        if name == "SpecifiedTradeSettlementPaymentMeans" {
            self.current_payment = Some(PaymentBuilder::default());
        }
        if let Some(line) = self.current_line.as_mut() {
            if name == "BilledQuantity" || name == "CreditedQuantity" {
                line.unit_code = attr_value(attrs, "unitCode").map(ToOwned::to_owned);
            }
            // EN 16931 BT-158 bindings: ram:DesignatedProductClassification wraps a
            // single ram:ClassCode whose text is the code (BT-158), @listID the
            // scheme (BT-158-1) and @listVersionID the version (BT-158-2).
            if name == "DesignatedProductClassification"
                && in_any(stack, &["SpecifiedTradeProduct"])
            {
                line.current_classification = Some(ClassificationBuilder::default());
            }
            if name == "ClassCode" {
                if let Some(classification) = line.current_classification.as_mut() {
                    classification.scheme_id = attr_value(attrs, "listID").map(ToOwned::to_owned);
                    classification.scheme_version =
                        attr_value(attrs, "listVersionID").map(ToOwned::to_owned);
                }
            }
        }
        Ok(())
    }

    fn end_element(&mut self, name: &str) -> Result<(), CiiError> {
        if self
            .current_context_parameter
            .as_ref()
            .is_some_and(|parameter| parameter.kind.element_name() == name)
        {
            let parameter = self
                .current_context_parameter
                .take()
                .ok_or(CiiError::MissingElement("ram:DocumentContextParameter"))?;
            self.apply_context_parameter(parameter)?;
        }
        if name == "DesignatedProductClassification" {
            if let Some(line) = self.current_line.as_mut() {
                if let Some(classification) =
                    line.current_classification.take().and_then(ClassificationBuilder::build)
                {
                    line.classifications.push(classification);
                }
            }
        }
        if name == "IncludedSupplyChainTradeLineItem" {
            let line = self
                .current_line
                .take()
                .ok_or(CiiError::MissingElement("IncludedSupplyChainTradeLineItem"))?
                .build()?;
            self.assign_current_line_preserved_xml(&line.id);
            self.lines.push(line);
        }
        if name == "ApplicableTradeTax" {
            if let Some(tax) = self.current_tax.take() {
                self.tax_summary.push(tax.build()?);
            }
        }
        if name == "SpecifiedTradeSettlementPaymentMeans" {
            if let Some(payment) = self.current_payment.take().and_then(PaymentBuilder::build) {
                self.payment_instructions.push(payment);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn text(&mut self, stack: &[String], raw: &str) -> Result<(), CiiError> {
        // EN 16931 BT-120 / BT-121: accumulate the exemption reason text + code
        // from RAW fragments BEFORE the trim / empty early-return below. Entity-
        // bearing text (e.g. "A & B") arrives as several Text/GeneralRef events;
        // overwriting kept only the last fragment and silently lost the rest.
        // Free text is preserved exactly (no trim) so a serialize -> parse ->
        // serialize round-trip stays byte-stable.
        if let Some(tax) = self.current_tax.as_mut() {
            if path_ends(stack, &["ApplicableTradeTax", "ExemptionReason"]) {
                tax.exemption_reason
                    .get_or_insert_with(String::new)
                    .push_str(raw);
                return Ok(());
            }
            if path_ends(stack, &["ApplicableTradeTax", "ExemptionReasonCode"]) {
                tax.exemption_reason_code
                    .get_or_insert_with(String::new)
                    .push_str(raw);
                return Ok(());
            }
        }
        let value = raw.trim();
        if value.is_empty() {
            return Ok(());
        }

        if let Some(parameter) = self.current_context_parameter.as_mut() {
            let container = parameter.kind.element_name();
            if path_ends(stack, &[container, "ID"]) {
                parameter.id = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &[container, "Value"]) {
                parameter.value = Some(value.to_owned());
                return Ok(());
            }
        }

        if path_ends(
            stack,
            &["ExchangedDocumentContext", "SpecifiedTransactionID"],
        ) {
            self.cii_transaction_ids.push(value.to_owned());
            return Ok(());
        }
        if path_ends(stack, &["ExchangedDocumentContext", "TestIndicator"]) {
            self.cii_test_indicators.push(value.to_owned());
            return Ok(());
        }

        if let Some(line) = self.current_line.as_mut() {
            if path_ends(stack, &["AssociatedDocumentLineDocument", "LineID"]) {
                line.id = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["BilledQuantity"]) || path_ends(stack, &["CreditedQuantity"]) {
                line.quantity = Some(decimal_value("line.quantity", value)?);
                return Ok(());
            }
            if path_ends(
                stack,
                &[
                    "SpecifiedTradeSettlementLineMonetarySummation",
                    "LineTotalAmount",
                ],
            ) {
                line.line_extension_amount =
                    Some(decimal_value("line.line_extension_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["NetPriceProductTradePrice", "ChargeAmount"]) {
                line.unit_price = Some(decimal_value("line.unit_price", value)?);
                return Ok(());
            }
            if path_ends(stack, &["SpecifiedTradeProduct", "Name"]) {
                line.description = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["SpecifiedTradeProduct", "Description"]) {
                if line.description.is_none() {
                    line.description = Some(value.to_owned());
                }
                return Ok(());
            }
            if path_ends(stack, &["DesignatedProductClassification", "ClassCode"]) {
                if let Some(classification) = line.current_classification.as_mut() {
                    classification.code = Some(value.to_owned());
                }
                return Ok(());
            }
            if path_ends(stack, &["ApplicableTradeTax", "CategoryCode"]) {
                line.tax_category = Some(value.to_owned());
                return Ok(());
            }
        }

        if let Some(tax) = self.current_tax.as_mut() {
            if path_ends(stack, &["ApplicableTradeTax", "BasisAmount"]) {
                tax.taxable_amount = Some(decimal_value("tax_summary.taxable_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["ApplicableTradeTax", "CalculatedAmount"]) {
                tax.tax_amount = Some(decimal_value("tax_summary.tax_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["ApplicableTradeTax", "CategoryCode"]) {
                tax.category_code = Some(value.to_owned());
                return Ok(());
            }
            // (BT-120 / BT-121 exemption reason text + code are accumulated
            // above, in the raw-fragment region, so entity-bearing text is not
            // truncated.)
            if path_ends(stack, &["ApplicableTradeTax", "RateApplicablePercent"]) {
                tax.tax_rate = Some(decimal_value("tax_summary.tax_rate", value)?);
                return Ok(());
            }
        }

        if let Some(role) = party_role(stack) {
            let party = self.party_mut(role);
            if path_ends(stack, &["SpecifiedLegalOrganization", "ID"]) {
                if party.id.is_none() {
                    party.id = Some(value.to_owned());
                }
                return Ok(());
            }
            if path_ends(stack, &["SellerTradeParty", "Name"])
                || path_ends(stack, &["BuyerTradeParty", "Name"])
                || path_ends(stack, &["PayeeTradeParty", "Name"])
            {
                party.name = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["SpecifiedTaxRegistration", "ID"]) {
                party.tax_ids.push(PartyTaxId {
                    scheme: "vat".to_owned(),
                    value: value.to_owned(),
                });
                return Ok(());
            }
            if path_ends(stack, &["PostalTradeAddress", "LineOne"])
                || path_ends(stack, &["PostalTradeAddress", "LineTwo"])
                || path_ends(stack, &["PostalTradeAddress", "LineThree"])
            {
                party.address_lines.push(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalTradeAddress", "CityName"]) {
                party.city = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalTradeAddress", "PostcodeCode"]) {
                party.postal_code = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalTradeAddress", "CountrySubDivisionName"]) {
                party.subdivision = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["PostalTradeAddress", "CountryID"]) {
                party.country = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["DefinedTradeContact", "PersonName"]) {
                party.contact_name = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(stack, &["EmailURIUniversalCommunication", "URIID"]) {
                party.email = Some(value.to_owned());
                return Ok(());
            }
            if path_ends(
                stack,
                &["TelephoneUniversalCommunication", "CompleteNumber"],
            ) {
                party.phone = Some(value.to_owned());
                return Ok(());
            }
        }

        if let Some(payment) = self.current_payment.as_mut() {
            if path_ends(stack, &["SpecifiedTradeSettlementPaymentMeans", "TypeCode"]) {
                payment.kind = Some(payment_kind(value));
                return Ok(());
            }
            if path_ends(stack, &["PayeePartyCreditorFinancialAccount", "IBANID"])
                || path_ends(
                    stack,
                    &["PayeePartyCreditorFinancialAccount", "ProprietaryID"],
                )
            {
                payment.account = Some(value.to_owned());
                return Ok(());
            }
        }

        if in_any(stack, &["SpecifiedTradeSettlementHeaderMonetarySummation"]) {
            if path_ends(stack, &["LineTotalAmount"]) {
                self.monetary_total.line_extension_amount = Some(decimal_value(
                    "monetary_total.line_extension_amount",
                    value,
                )?);
                return Ok(());
            }
            if path_ends(stack, &["TaxBasisTotalAmount"]) {
                self.monetary_total.tax_exclusive_amount =
                    Some(decimal_value("monetary_total.tax_exclusive_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["GrandTotalAmount"]) {
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
            if path_ends(stack, &["TotalPrepaidAmount"]) {
                self.monetary_total.prepaid_amount =
                    Some(decimal_value("monetary_total.prepaid_amount", value)?);
                return Ok(());
            }
            if path_ends(stack, &["DuePayableAmount"]) {
                self.monetary_total.payable_amount =
                    Some(decimal_value("monetary_total.payable_amount", value)?);
                return Ok(());
            }
        }

        if path_ends(stack, &["ExchangedDocument", "ID"]) {
            self.document_number = Some(value.to_owned());
        } else if path_ends(stack, &["ExchangedDocument", "TypeCode"]) {
            self.document_type = Some(document_type(value)?);
        } else if path_ends(stack, &["IssueDateTime", "DateTimeString"]) {
            self.issue_date = Some(cii_date_to_iso("ExchangedDocument/IssueDateTime", value)?);
        } else if path_ends(stack, &["TaxPointDate", "DateTimeString"]) {
            self.tax_point_date = Some(cii_date_to_iso(
                "ApplicableHeaderTradeSettlement/TaxPointDate",
                value,
            )?);
        } else if path_ends(
            stack,
            &[
                "ActualDeliverySupplyChainEvent",
                "OccurrenceDateTime",
                "DateTimeString",
            ],
        ) {
            self.tax_point_date = Some(cii_date_to_iso(
                "ApplicableHeaderTradeDelivery/ActualDeliverySupplyChainEvent",
                value,
            )?);
        } else if path_ends(stack, &["DueDateDateTime", "DateTimeString"]) {
            self.due_date = Some(cii_date_to_iso(
                "SpecifiedTradePaymentTerms/DueDate",
                value,
            )?);
        } else if path_ends(
            stack,
            &["ApplicableHeaderTradeSettlement", "InvoiceCurrencyCode"],
        ) {
            self.currency = Some(value.to_owned());
        } else if path_ends(stack, &["ApplicableHeaderTradeAgreement", "BuyerReference"]) {
            self.cii_buyer_reference = Some(value.to_owned());
        } else if path_ends(stack, &["ExchangedDocument", "IncludedNote", "Content"]) {
            self.notes.push(LocalizedString {
                language: DEFAULT_LANGUAGE.to_owned(),
                text: value.to_owned(),
            });
        } else if path_ends(stack, &["SpecifiedTradePaymentTerms", "Description"]) {
            self.payment_terms_description = Some(value.to_owned());
        } else if path_ends(
            stack,
            &["ApplicableHeaderTradeSettlement", "PaymentReference"],
        ) {
            self.payment_reference = Some(value.to_owned());
        }

        Ok(())
    }

    fn apply_context_parameter(
        &mut self,
        parameter: DocumentContextParameterBuilder,
    ) -> Result<(), CiiError> {
        match parameter.kind {
            DocumentContextKind::BusinessProcess => {
                if let Some(id) = parameter.id {
                    self.cii_business_process_context_ids.push(id);
                }
            }
            DocumentContextKind::Guideline => {
                if let Some(id) = parameter.id.filter(|id| id != CORE_GUIDELINE_ID) {
                    self.cii_guideline_context_ids.push(id);
                }
            }
            DocumentContextKind::Application => {
                if parameter.id.as_deref() == Some(mapping::INVOICEKIT_CII_METADATA_EXTENSION_URN) {
                    if let Some(value) = parameter.value {
                        let metadata: CiiMetadataPayload = serde_json::from_str(&value)?;
                        self.metadata_tenant_id = Some(metadata.tenant_id);
                        self.metadata_trace_id = Some(metadata.trace_id);
                        self.metadata_source_system = metadata.source_system;
                    }
                } else if let Some(id) = parameter.id {
                    self.cii_application_contexts.push(CiiApplicationContext {
                        id,
                        value: parameter.value,
                    });
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn finish(self) -> Result<CommercialDocument, CiiError> {
        let document_type = self
            .document_type
            .ok_or(CiiError::MissingElement("ram:TypeCode"))?;
        let document_number = self
            .document_number
            .ok_or(CiiError::MissingElement("ram:ExchangedDocument/ram:ID"))?;
        let issue_date = self
            .issue_date
            .ok_or(CiiError::MissingElement("ram:IssueDateTime"))?;
        let currency = self
            .currency
            .ok_or(CiiError::MissingElement("ram:InvoiceCurrencyCode"))?;
        let tenant_id = self
            .metadata_tenant_id
            .unwrap_or_else(|| "cii-import".to_owned());
        let trace_id = self
            .metadata_trace_id
            .unwrap_or_else(|| format!("{BEAD_ID}:{document_number}"));
        let mut extensions = Vec::<JurisdictionExtension>::new();
        let mut cii_document_fields = Map::new();
        if let Some(value) = self.cii_buyer_reference {
            cii_document_fields.insert("buyer_reference".to_owned(), Value::String(value));
        }
        if !self.cii_business_process_context_ids.is_empty() {
            cii_document_fields.insert(
                "business_process_context_ids".to_owned(),
                Value::Array(
                    self.cii_business_process_context_ids
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if !self.preserved_xml.is_empty() {
            cii_document_fields.insert(
                CII_PRESERVED_XML_KEY.to_owned(),
                Value::Array(
                    self.preserved_xml
                        .into_iter()
                        .map(CiiPreservedXml::into_value)
                        .collect(),
                ),
            );
        }
        if !cii_document_fields.is_empty() {
            extensions.push(JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                Value::Object(cii_document_fields),
            )?);
        }
        let mut cii_profile_context = Map::new();
        insert_string_array(
            &mut cii_profile_context,
            "guideline_context_ids",
            self.cii_guideline_context_ids,
        );
        insert_string_array(
            &mut cii_profile_context,
            "transaction_ids",
            self.cii_transaction_ids,
        );
        insert_string_array(
            &mut cii_profile_context,
            "test_indicators",
            self.cii_test_indicators,
        );
        if !self.cii_application_contexts.is_empty() {
            cii_profile_context.insert(
                "application_contexts".to_owned(),
                Value::Array(
                    self.cii_application_contexts
                        .into_iter()
                        .map(CiiApplicationContext::into_value)
                        .collect(),
                ),
            );
        }
        if !cii_profile_context.is_empty() {
            extensions.push(JurisdictionExtension::new(
                mapping::CII_PROFILE_CONTEXT_EXTENSION_URN,
                Value::Object(cii_profile_context),
            )?);
        }
        let due_date = self.due_date.map(DateOnly::new).transpose()?;
        let payment_terms = self
            .payment_terms_description
            .map(|description| PaymentTerms {
                description,
                due_date: due_date.clone(),
            });
        let mut payment_instructions = self.payment_instructions;
        if let Some(reference) = self.payment_reference {
            if let Some(first) = payment_instructions.first_mut() {
                if first.reference.is_none() {
                    first.reference = Some(reference);
                }
            } else {
                payment_instructions.push(PaymentInstruction {
                    kind: PaymentInstructionKind::Other,
                    account: None,
                    reference: Some(reference),
                });
            }
        }

        let document = CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(document_number.clone())?,
            document_type,
            issue_date: DateOnly::new(issue_date)?,
            tax_point_date: self.tax_point_date.map(DateOnly::new).transpose()?,
            due_date,
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new(document_number)?,
            currency: Iso4217Code::new(currency)?,
            supplier: self.supplier.build("SellerTradeParty")?,
            customer: self.customer.build("BuyerTradeParty")?,
            payee: if self.has_payee {
                Some(self.payee.build("PayeeTradeParty")?)
            } else {
                None
            },
            payment_terms,
            payment_instructions,
            lines: self.lines,
            tax_summary: self.tax_summary,
            monetary_total: self.monetary_total.build()?,
            attachments: Vec::<Attachment>::new(),
            references: Vec::<DocumentReference>::new(),
            notes: self.notes,
            extensions,
            allowance_charges: Vec::new(),
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

    fn active_line_id(&self) -> Option<String> {
        self.current_line
            .as_ref()
            .and_then(|line| line.id.as_ref())
            .cloned()
    }

    fn push_preserved_xml(&mut self, capture: RawXmlCapture, xml: &str) -> Result<(), CiiError> {
        let preserved = CiiPreservedXml {
            container: capture.container,
            element: capture.element,
            xml: canonicalize_preserved_xml_fragment(xml, &capture.namespaces)?,
            line_id: capture.line_id,
        };
        validate_preserved_xml_fragment(&preserved)?;
        self.preserved_xml.push(preserved);
        Ok(())
    }

    fn assign_current_line_preserved_xml(&mut self, line_id: &str) {
        let start = self.current_line_preserved_start.take().unwrap_or(0);
        for preserved in self.preserved_xml.iter_mut().skip(start) {
            if preserved.line_id.is_none() && is_line_scoped_container_path(&preserved.container) {
                preserved.line_id = Some(line_id.to_owned());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentContextKind {
    BusinessProcess,
    Guideline,
    Application,
}

impl DocumentContextKind {
    fn from_element(name: &str) -> Option<Self> {
        match name {
            "BusinessProcessSpecifiedDocumentContextParameter" => Some(Self::BusinessProcess),
            "GuidelineSpecifiedDocumentContextParameter" => Some(Self::Guideline),
            "ApplicationSpecifiedDocumentContextParameter" => Some(Self::Application),
            _ => None,
        }
    }

    const fn element_name(self) -> &'static str {
        match self {
            Self::BusinessProcess => "BusinessProcessSpecifiedDocumentContextParameter",
            Self::Guideline => "GuidelineSpecifiedDocumentContextParameter",
            Self::Application => "ApplicationSpecifiedDocumentContextParameter",
        }
    }
}

#[derive(Debug)]
struct DocumentContextParameterBuilder {
    kind: DocumentContextKind,
    id: Option<String>,
    value: Option<String>,
}

impl DocumentContextParameterBuilder {
    const fn new(kind: DocumentContextKind) -> Self {
        Self {
            kind,
            id: None,
            value: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct CiiMetadataPayload {
    tenant_id: String,
    trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_system: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CiiApplicationContext {
    id: String,
    value: Option<String>,
}

impl CiiApplicationContext {
    fn into_value(self) -> Value {
        let mut payload = Map::new();
        payload.insert("id".to_owned(), Value::String(self.id));
        if let Some(value) = self.value {
            payload.insert("value".to_owned(), Value::String(value));
        }
        Value::Object(payload)
    }

    fn from_value(value: &Value) -> Result<Self, CiiError> {
        let Some(payload) = value.as_object() else {
            return Err(CiiError::InvalidPreservedXml {
                container: "ExchangedDocumentContext".to_owned(),
                element: "ApplicationSpecifiedDocumentContextParameter".to_owned(),
                message: "application context payload must be an object".to_owned(),
            });
        };
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| CiiError::InvalidPreservedXml {
                container: "ExchangedDocumentContext".to_owned(),
                element: "ApplicationSpecifiedDocumentContextParameter".to_owned(),
                message: "application context id must be a string".to_owned(),
            })?;
        let value = payload
            .get("value")
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| CiiError::InvalidPreservedXml {
                        container: "ExchangedDocumentContext".to_owned(),
                        element: "ApplicationSpecifiedDocumentContextParameter".to_owned(),
                        message: "application context value must be a string".to_owned(),
                    })
            })
            .transpose()?;
        Ok(Self { id, value })
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
    fn build(self, field: &'static str) -> Result<Party, CiiError> {
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
            name: self.name.ok_or(CiiError::MissingElement(field))?,
            tax_ids: self.tax_ids,
            address: PostalAddress {
                lines: if self.address_lines.is_empty() {
                    return Err(CiiError::MissingElement(
                        "ram:PostalTradeAddress/ram:LineOne",
                    ));
                } else {
                    self.address_lines
                },
                city: self.city.ok_or(CiiError::MissingElement(
                    "ram:PostalTradeAddress/ram:CityName",
                ))?,
                subdivision: self.subdivision,
                postal_code: self.postal_code.ok_or(CiiError::MissingElement(
                    "ram:PostalTradeAddress/ram:PostcodeCode",
                ))?,
                country: CountryCode::new(self.country.ok_or(CiiError::MissingElement(
                    "ram:PostalTradeAddress/ram:CountryID",
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
    classifications: Vec<ItemClassification>,
    current_classification: Option<ClassificationBuilder>,
}

/// Accumulates a single EN 16931 BT-158 commodity classification while the
/// parser walks `ram:DesignatedProductClassification/ram:ClassCode`.
#[derive(Default)]
struct ClassificationBuilder {
    /// BT-158: the classification code text.
    code: Option<String>,
    /// BT-158-1: the `@listID` scheme identifier.
    scheme_id: Option<String>,
    /// BT-158-2: the optional `@listVersionID`.
    scheme_version: Option<String>,
}

impl ClassificationBuilder {
    fn build(self) -> Option<ItemClassification> {
        let code = self.code?;
        Some(ItemClassification {
            code,
            scheme_id: self.scheme_id.unwrap_or_default(),
            scheme_version: self.scheme_version,
        })
    }
}

impl LineBuilder {
    fn build(self) -> Result<DocumentLine, CiiError> {
        Ok(DocumentLine {
            id: self.id.ok_or(CiiError::MissingElement(
                "ram:AssociatedDocumentLineDocument/ram:LineID",
            ))?,
            description: self.description.ok_or(CiiError::MissingElement(
                "ram:SpecifiedTradeProduct/ram:Name",
            ))?,
            quantity: self
                .quantity
                .ok_or(CiiError::MissingElement("ram:BilledQuantity"))?,
            unit_code: self.unit_code,
            unit_price: self.unit_price.ok_or(CiiError::MissingElement(
                "ram:NetPriceProductTradePrice/ram:ChargeAmount",
            ))?,
            line_extension_amount: self.line_extension_amount.ok_or(CiiError::MissingElement(
                "ram:SpecifiedTradeSettlementLineMonetarySummation/ram:LineTotalAmount",
            ))?,
            tax_category: self.tax_category,
            classifications: self.classifications,
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
    exemption_reason: Option<String>,
    exemption_reason_code: Option<String>,
}

impl TaxSummaryBuilder {
    fn build(self) -> Result<TaxCategorySummary, CiiError> {
        Ok(TaxCategorySummary {
            category_code: self.category_code.ok_or(CiiError::MissingElement(
                "ram:ApplicableTradeTax/ram:CategoryCode",
            ))?,
            taxable_amount: self.taxable_amount.ok_or(CiiError::MissingElement(
                "ram:ApplicableTradeTax/ram:BasisAmount",
            ))?,
            tax_amount: self.tax_amount.ok_or(CiiError::MissingElement(
                "ram:ApplicableTradeTax/ram:CalculatedAmount",
            ))?,
            tax_rate: self.tax_rate,
            exemption_reason: self.exemption_reason,
            exemption_reason_code: self.exemption_reason_code,
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
    fn build(self) -> Result<MonetaryTotal, CiiError> {
        Ok(MonetaryTotal {
            line_extension_amount: self
                .line_extension_amount
                .ok_or(CiiError::MissingElement("ram:LineTotalAmount"))?,
            tax_exclusive_amount: self
                .tax_exclusive_amount
                .ok_or(CiiError::MissingElement("ram:TaxBasisTotalAmount"))?,
            tax_inclusive_amount: self
                .tax_inclusive_amount
                .ok_or(CiiError::MissingElement("ram:GrandTotalAmount"))?,
            allowance_total_amount: self.allowance_total_amount,
            charge_total_amount: self.charge_total_amount,
            prepaid_amount: self.prepaid_amount,
            payable_amount: self
                .payable_amount
                .ok_or(CiiError::MissingElement("ram:DuePayableAmount"))?,
        })
    }
}

#[derive(Default)]
struct PaymentBuilder {
    kind: Option<PaymentInstructionKind>,
    account: Option<String>,
}

impl PaymentBuilder {
    fn build(self) -> Option<PaymentInstruction> {
        self.account.map(|account| PaymentInstruction {
            kind: self.kind.unwrap_or(PaymentInstructionKind::IbanBic),
            account: Some(account),
            reference: None,
        })
    }
}

#[derive(Debug)]
struct XmlAttribute {
    local_name: String,
    value: String,
}

#[derive(Debug)]
struct RawXmlCapture {
    container: String,
    element: String,
    start: usize,
    depth: usize,
    line_id: Option<String>,
    namespaces: Vec<XmlNamespaceBinding>,
}

impl RawXmlCapture {
    fn new(
        stack: &[String],
        element: String,
        start: usize,
        line_id: Option<String>,
        namespaces: Vec<XmlNamespaceBinding>,
    ) -> Self {
        Self {
            container: stack.join("/"),
            element,
            start,
            depth: 1,
            line_id,
            namespaces,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct XmlNamespaceBinding {
    prefix: Option<String>,
    uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CiiPreservedXml {
    container: String,
    element: String,
    xml: String,
    line_id: Option<String>,
}

impl CiiPreservedXml {
    fn into_value(self) -> Value {
        let mut payload = Map::new();
        payload.insert(
            CII_PRESERVED_CONTAINER_KEY.to_owned(),
            Value::String(self.container),
        );
        payload.insert(
            CII_PRESERVED_ELEMENT_KEY.to_owned(),
            Value::String(self.element),
        );
        payload.insert(
            CII_PRESERVED_XML_FRAGMENT_KEY.to_owned(),
            Value::String(self.xml),
        );
        if let Some(line_id) = self.line_id {
            payload.insert("line_id".to_owned(), Value::String(line_id));
        }
        Value::Object(payload)
    }

    fn from_value(value: &Value) -> Result<Self, CiiError> {
        let Some(payload) = value.as_object() else {
            return Err(CiiError::InvalidPreservedXml {
                container: String::new(),
                element: String::new(),
                message: "payload item must be an object".to_owned(),
            });
        };
        let container = preserved_string_field(payload, CII_PRESERVED_CONTAINER_KEY)?;
        let element = preserved_string_field(payload, CII_PRESERVED_ELEMENT_KEY)?;
        let xml = preserved_string_field(payload, CII_PRESERVED_XML_FRAGMENT_KEY)?;
        let line_id = payload
            .get("line_id")
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| CiiError::InvalidPreservedXml {
                        container: container.clone(),
                        element: element.clone(),
                        message: "line_id must be a string".to_owned(),
                    })
            })
            .transpose()?;
        Ok(Self {
            container,
            element,
            xml,
            line_id,
        })
    }
}

#[allow(clippy::too_many_lines)]
fn serialize_document(document: &CommercialDocument) -> Result<String, CiiError> {
    let currency = string_value(&document.currency)?;
    let mut xml = String::new();
    xml.push_str(r#"<rsm:CrossIndustryInvoice xmlns:qdt=""#);
    write_xml_attr(CII_QDT_NAMESPACE_URI, &mut xml);
    xml.push_str(r#"" xmlns:udt=""#);
    write_xml_attr(CII_UDT_NAMESPACE_URI, &mut xml);
    xml.push_str(r#"" xmlns:rsm=""#);
    write_xml_attr(CII_RSM_NAMESPACE_URI, &mut xml);
    xml.push_str(r#"" xmlns:ram=""#);
    write_xml_attr(CII_RAM_NAMESPACE_URI, &mut xml);
    xml.push_str(r#"">"#);
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice",
        None,
        "ExchangedDocumentContext",
    )?;
    write_document_context(&mut xml, document)?;

    xml.push_str("<rsm:ExchangedDocument>");
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument",
        None,
        "ID",
    )?;
    write_text_element(
        &mut xml,
        "ram:ID",
        &string_value(&document.document_number)?,
    );
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument",
        None,
        "TypeCode",
    )?;
    write_text_element(&mut xml, "ram:TypeCode", document_type_code(document)?);
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument",
        None,
        "IssueDateTime",
    )?;
    xml.push_str("<ram:IssueDateTime>");
    write_date_time(&mut xml, &document.issue_date)?;
    xml.push_str("</ram:IssueDateTime>");
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument",
        None,
        "IncludedNote",
    )?;
    for note in &document.notes {
        write_note(&mut xml, note, document)?;
    }
    write_preserved_xml_after_all(
        &mut xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument",
        None,
    )?;
    xml.push_str("</rsm:ExchangedDocument>");

    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice",
        None,
        "SupplyChainTradeTransaction",
    )?;
    xml.push_str("<rsm:SupplyChainTradeTransaction>");
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction",
        None,
        "IncludedSupplyChainTradeLineItem",
    )?;
    for line in &document.lines {
        write_line(&mut xml, line, &currency, document)?;
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction",
        None,
        "ApplicableHeaderTradeAgreement",
    )?;
    xml.push_str("<ram:ApplicableHeaderTradeAgreement>");
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement",
        None,
        "BuyerReference",
    )?;
    if let Some(value) = cii_document_field_value(document, "buyer_reference") {
        write_text_element(&mut xml, "ram:BuyerReference", value);
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement",
        None,
        "SellerTradeParty",
    )?;
    write_party(
        &mut xml,
        "ram:SellerTradeParty",
        &document.supplier,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty",
    )?;
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement",
        None,
        "BuyerTradeParty",
    )?;
    write_party(
        &mut xml,
        "ram:BuyerTradeParty",
        &document.customer,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty",
    )?;
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement",
        None,
        "BuyerOrderReferencedDocument",
    )?;
    let emitted_buyer_order = write_buyer_order_referenced_document(&mut xml, document);
    write_preserved_xml_after_child(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement",
        None,
        "BuyerOrderReferencedDocument",
        emitted_buyer_order,
    )?;
    xml.push_str("</ram:ApplicableHeaderTradeAgreement>");

    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction",
        None,
        "ApplicableHeaderTradeDelivery",
    )?;
    xml.push_str("<ram:ApplicableHeaderTradeDelivery>");
    if let Some(date) = &document.tax_point_date {
        xml.push_str("<ram:ActualDeliverySupplyChainEvent><ram:OccurrenceDateTime>");
        write_date_time(&mut xml, date)?;
        xml.push_str("</ram:OccurrenceDateTime></ram:ActualDeliverySupplyChainEvent>");
    }
    write_preserved_xml_after_all(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeDelivery",
        None,
    )?;
    xml.push_str("</ram:ApplicableHeaderTradeDelivery>");

    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction",
        None,
        "ApplicableHeaderTradeSettlement",
    )?;
    xml.push_str("<ram:ApplicableHeaderTradeSettlement>");
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "PayeeTradeParty",
    )?;
    if let Some(payee) = &document.payee {
        write_party(
            &mut xml,
            "ram:PayeeTradeParty",
            payee,
            document,
            "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty",
        )?;
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "PaymentReference",
    )?;
    if let Some(first) = document
        .payment_instructions
        .iter()
        .find(|instruction| instruction.reference.is_some())
        .and_then(|instruction| instruction.reference.as_ref())
    {
        write_text_element(&mut xml, "ram:PaymentReference", first);
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "InvoiceCurrencyCode",
    )?;
    write_text_element(&mut xml, "ram:InvoiceCurrencyCode", &currency);
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "SpecifiedTradeSettlementPaymentMeans",
    )?;
    for instruction in &document.payment_instructions {
        write_payment_instruction(&mut xml, instruction, document)?;
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "ApplicableTradeTax",
    )?;
    for summary in &document.tax_summary {
        write_tax_summary(&mut xml, summary, document)?;
    }
    // BG-14: native invoice period at its schema slot (after ApplicableTradeTax),
    // UNLESS a preserved BillingSpecifiedPeriod already exists for this container
    // (a parse-then-enrich document). Preserved fragment wins: it is flushed by
    // the SpecifiedTradePaymentTerms preserve replay below, so emitting natively
    // too would produce two BillingSpecifiedPeriod elements (BG-14 is 0..1).
    let settlement_path =
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement";
    let billing_period_preserved = cii_preserved_xml_values(document, settlement_path, None)?
        .iter()
        .any(|preserved| preserved.element == "BillingSpecifiedPeriod");
    if !billing_period_preserved {
        write_billing_specified_period(&mut xml, document)?;
    }
    // Flush preserved settlement siblings ordered BEFORE SpecifiedTradeAllowanceCharge
    // (notably a preserved BillingSpecifiedPeriod) before the native allowances, so a
    // parse-then-enrich document keeps CII child order (BillingSpecifiedPeriod precedes
    // SpecifiedTradeAllowanceCharge). Canonical idempotence alone does NOT catch a
    // mis-ordered-but-well-formed emission.
    write_preserved_xml_before(
        &mut xml,
        document,
        settlement_path,
        None,
        "SpecifiedTradeAllowanceCharge",
    )?;
    // BG-20/21: native document-level allowances/charges (SpecifiedTradeAllowanceCharge,
    // 0..n + lossiness-ledger-preserved, so native entries and any preserved ones
    // legitimately coexist — emitted unconditionally).
    write_cii_allowance_charges(&mut xml, document);
    // Flush the remaining preserved settlement siblings (preserved allowance charges,
    // SubtotalCalculatedTradeTax, SpecifiedLogisticsServiceCharge) at/above
    // SpecifiedTradeAllowanceCharge but STRICTLY ABOVE BillingSpecifiedPeriod, so a
    // BillingSpecifiedPeriod already flushed above is not re-emitted. The window is
    // explicit because the convenience wrapper's lower bound (the last known child)
    // would re-cover BillingSpecifiedPeriod, and write_preserved_xml does not consume.
    write_preserved_xml(
        &mut xml,
        document,
        settlement_path,
        None,
        cii_child_order("ApplicableHeaderTradeSettlement", "BillingSpecifiedPeriod"),
        cii_child_order("ApplicableHeaderTradeSettlement", "SpecifiedTradePaymentTerms"),
    )?;
    if let Some(terms) = &document.payment_terms {
        xml.push_str("<ram:SpecifiedTradePaymentTerms>");
        write_text_element(&mut xml, "ram:Description", &terms.description);
        if let Some(date) = &terms.due_date {
            xml.push_str("<ram:DueDateDateTime>");
            write_date_time(&mut xml, date)?;
            xml.push_str("</ram:DueDateDateTime>");
        }
        xml.push_str("</ram:SpecifiedTradePaymentTerms>");
    } else if let Some(date) = &document.due_date {
        xml.push_str("<ram:SpecifiedTradePaymentTerms><ram:DueDateDateTime>");
        write_date_time(&mut xml, date)?;
        xml.push_str("</ram:DueDateDateTime></ram:SpecifiedTradePaymentTerms>");
    }
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "SpecifiedTradeSettlementHeaderMonetarySummation",
    )?;
    write_monetary_total(&mut xml, &document.monetary_total, &currency, document)?;
    write_preserved_xml_before(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "InvoiceReferencedDocument",
    )?;
    let emitted_invoice_ref = write_invoice_referenced_documents(&mut xml, document)?;
    write_preserved_xml_after_child(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement",
        None,
        "InvoiceReferencedDocument",
        emitted_invoice_ref,
    )?;
    xml.push_str("</ram:ApplicableHeaderTradeSettlement>");
    write_preserved_xml_after_all(
        &mut xml,
        document,
        "CrossIndustryInvoice/SupplyChainTradeTransaction",
        None,
    )?;
    xml.push_str("</rsm:SupplyChainTradeTransaction>");
    write_preserved_xml_after_all(&mut xml, document, "CrossIndustryInvoice", None)?;
    xml.push_str("</rsm:CrossIndustryInvoice>");
    Ok(xml)
}

fn write_document_context(xml: &mut String, document: &CommercialDocument) -> Result<(), CiiError> {
    xml.push_str("<rsm:ExchangedDocumentContext>");
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
        "SpecifiedTransactionID",
    )?;
    for value in profile_context_values(document, "transaction_ids") {
        write_text_element(xml, "ram:SpecifiedTransactionID", value);
    }
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
        "TestIndicator",
    )?;
    for value in profile_context_values(document, "test_indicators") {
        write_text_element(xml, "ram:TestIndicator", value);
    }
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
        "BusinessProcessSpecifiedDocumentContextParameter",
    )?;
    for value in cii_document_field_values(document, "business_process_context_ids") {
        write_context_parameter(
            xml,
            "ram:BusinessProcessSpecifiedDocumentContextParameter",
            value,
            None,
        );
    }
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
        "ApplicationSpecifiedDocumentContextParameter",
    )?;
    for context in profile_application_context_values(document)? {
        write_context_parameter(
            xml,
            "ram:ApplicationSpecifiedDocumentContextParameter",
            &context.id,
            context.value.as_deref(),
        );
    }
    write_invoicekit_metadata_context_parameter(xml, &document.meta)?;
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
        "GuidelineSpecifiedDocumentContextParameter",
    )?;
    let guideline_context_ids = profile_context_values(document, "guideline_context_ids");
    if guideline_context_ids.is_empty() {
        write_context_parameter(
            xml,
            "ram:GuidelineSpecifiedDocumentContextParameter",
            CORE_GUIDELINE_ID,
            None,
        );
    } else {
        for value in guideline_context_ids {
            write_context_parameter(
                xml,
                "ram:GuidelineSpecifiedDocumentContextParameter",
                value,
                None,
            );
        }
    }
    write_preserved_xml_after_all(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocumentContext",
        None,
    )?;
    xml.push_str("</rsm:ExchangedDocumentContext>");
    Ok(())
}

fn write_context_parameter(xml: &mut String, container: &str, id: &str, value: Option<&str>) {
    xml.push('<');
    xml.push_str(container);
    xml.push('>');
    write_text_element(xml, "ram:ID", id);
    if let Some(value) = value {
        write_text_element(xml, "ram:Value", value);
    }
    xml.push_str("</");
    xml.push_str(container);
    xml.push('>');
}

fn write_invoicekit_metadata_context_parameter(
    xml: &mut String,
    meta: &DocumentMeta,
) -> Result<(), CiiError> {
    let payload = CiiMetadataPayload {
        tenant_id: meta.tenant_id.clone(),
        trace_id: meta.trace_id.clone(),
        source_system: meta.source_system.clone(),
    };
    let value = serde_json::to_string(&payload)?;
    write_context_parameter(
        xml,
        "ram:ApplicationSpecifiedDocumentContextParameter",
        mapping::INVOICEKIT_CII_METADATA_EXTENSION_URN,
        Some(&value),
    );
    Ok(())
}

/// Look up a single extension payload value by URN and key.
fn extension_payload_value<'a>(
    document: &'a CommercialDocument,
    urn: &str,
    key: &str,
) -> Option<&'a Value> {
    document
        .extensions
        .iter()
        .find(|extension| extension.urn == urn)
        .and_then(|extension| extension.payload.get(key))
}

fn cii_document_field_value<'a>(document: &'a CommercialDocument, key: &str) -> Option<&'a str> {
    extension_payload_value(document, mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN, key)
        .and_then(Value::as_str)
}

fn cii_document_field_values<'a>(document: &'a CommercialDocument, key: &str) -> Vec<&'a str> {
    extension_payload_value(document, mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN, key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn profile_context_values<'a>(document: &'a CommercialDocument, key: &str) -> Vec<&'a str> {
    extension_payload_value(document, mapping::CII_PROFILE_CONTEXT_EXTENSION_URN, key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn profile_application_context_values(
    document: &CommercialDocument,
) -> Result<Vec<CiiApplicationContext>, CiiError> {
    let Some(values) = extension_payload_value(
        document,
        mapping::CII_PROFILE_CONTEXT_EXTENSION_URN,
        "application_contexts",
    ) else {
        return Ok(Vec::new());
    };
    let Some(items) = values.as_array() else {
        return Err(CiiError::InvalidPreservedXml {
            container: "ExchangedDocumentContext".to_owned(),
            element: "ApplicationSpecifiedDocumentContextParameter".to_owned(),
            message: "application_contexts must be an array".to_owned(),
        });
    };
    items
        .iter()
        .map(CiiApplicationContext::from_value)
        .collect()
}

fn insert_string_array(payload: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    if values.is_empty() {
        return;
    }
    payload.insert(
        key.to_owned(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
}

fn write_preserved_xml(
    xml: &mut String,
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
    lower_bound: Option<u16>,
    upper_bound: Option<u16>,
) -> Result<(), CiiError> {
    let parent = container_name(container);
    let mut values = cii_preserved_xml_values(document, container, line_id)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, preserved)| {
            let order = cii_child_order(parent, &preserved.element)?;
            if lower_bound.is_none_or(|lower| order > lower)
                && upper_bound.is_none_or(|upper| order < upper)
            {
                Some((order, index, preserved))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|(order, index, _)| (*order, *index));
    for (_, _, preserved) in values {
        xml.push_str(&preserved.xml);
    }
    Ok(())
}

fn write_preserved_xml_before(
    xml: &mut String,
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
    before_child: &str,
) -> Result<(), CiiError> {
    let parent = container_name(container);
    write_preserved_xml(
        xml,
        document,
        container,
        line_id,
        previous_known_child_order(parent, before_child),
        cii_child_order(parent, before_child),
    )
}

fn write_preserved_xml_after_all(
    xml: &mut String,
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
) -> Result<(), CiiError> {
    let parent = container_name(container);
    write_preserved_xml(
        xml,
        document,
        container,
        line_id,
        max_known_child_order(parent),
        None,
    )
}

/// Replay every remaining preserved sibling whose schema order is strictly
/// greater than `child`'s order (and greater than the highest known anchor).
///
/// Used after emitting a native element that the *parser* deliberately keeps as
/// preserved raw XML — `ram:BuyerOrderReferencedDocument` (BT-13) and
/// `ram:InvoiceReferencedDocument` (BT-25). Those elements are intentionally
/// absent from [`known_cii_children`] (so the parser preserves them rather than
/// dropping them), which means [`write_preserved_xml_after_all`] alone would
/// re-replay every sibling above the last *known* anchor and double-emit the
/// fragments that the matching `write_preserved_xml_before` already wrote. Lower-
/// bounding the trailing replay on `child`'s own order keeps each fragment to a
/// single emission and preserves schema order across the native + preserved mix.
///
/// `emitted_native` selects whether the trailing replay *includes* a preserved
/// fragment that shares `child`'s own schema order. When the native element was
/// emitted from `document.references` (a fresh-IR doc), a preserved fragment at
/// the same order is suppressed so the slot is not double-filled. When no native
/// element was emitted (the common parsed-doc case, whose `references` are empty
/// because the parser preserves the element as raw XML), the preserved fragment
/// at `child`'s order is replayed so the round-trip stays lossless.
fn write_preserved_xml_after_child(
    xml: &mut String,
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
    child: &str,
    emitted_native: bool,
) -> Result<(), CiiError> {
    let parent = container_name(container);
    let child_lower = cii_child_order(parent, child).map(|child_order| {
        // EXCLUSIVE lower bound: `order > lower`. To *include* the child's own
        // order, drop the bound by one; to *exclude* it, keep the child's order.
        if emitted_native {
            child_order
        } else {
            child_order.saturating_sub(1)
        }
    });
    let lower_bound = match (max_known_child_order(parent), child_lower) {
        (Some(anchor), Some(child_order)) => Some(anchor.max(child_order)),
        (anchor, child_order) => anchor.or(child_order),
    };
    write_preserved_xml(xml, document, container, line_id, lower_bound, None)
}

fn max_known_child_order(parent: &str) -> Option<u16> {
    known_cii_children(parent).and_then(|children| {
        children
            .iter()
            .filter_map(|child| cii_child_order(parent, child))
            .max()
    })
}

fn cii_preserved_xml_values(
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
) -> Result<Vec<CiiPreservedXml>, CiiError> {
    let Some(values) = extension_payload_value(
        document,
        mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
        CII_PRESERVED_XML_KEY,
    ) else {
        return Ok(Vec::new());
    };
    let Some(items) = values.as_array() else {
        return Err(CiiError::InvalidPreservedXml {
            container: container.to_owned(),
            element: CII_PRESERVED_XML_KEY.to_owned(),
            message: "preserved_xml must be an array".to_owned(),
        });
    };

    let mut matching = Vec::new();
    for item in items {
        let preserved = CiiPreservedXml::from_value(item)?;
        validate_preserved_xml_entry(document, &preserved)?;
        if preserved.container == container && preserved.line_id.as_deref() == line_id {
            matching.push(preserved);
        }
    }
    Ok(matching)
}

fn has_preserved_xml(
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
) -> Result<bool, CiiError> {
    Ok(!cii_preserved_xml_values(document, container, line_id)?.is_empty())
}

fn has_preserved_xml_at_or_below(
    document: &CommercialDocument,
    container: &str,
    line_id: Option<&str>,
) -> Result<bool, CiiError> {
    let Some(values) = extension_payload_value(
        document,
        mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
        CII_PRESERVED_XML_KEY,
    ) else {
        return Ok(false);
    };
    let Some(items) = values.as_array() else {
        return Err(CiiError::InvalidPreservedXml {
            container: container.to_owned(),
            element: CII_PRESERVED_XML_KEY.to_owned(),
            message: "preserved_xml must be an array".to_owned(),
        });
    };

    for item in items {
        let preserved = CiiPreservedXml::from_value(item)?;
        validate_preserved_xml_entry(document, &preserved)?;
        let in_subtree = preserved.container == container
            || preserved
                .container
                .strip_prefix(container)
                .is_some_and(|suffix| suffix.starts_with('/'));
        if in_subtree && preserved.line_id.as_deref() == line_id {
            return Ok(true);
        }
    }
    Ok(false)
}

#[allow(clippy::too_many_lines)]
fn write_party(
    xml: &mut String,
    container: &str,
    party: &Party,
    document: &CommercialDocument,
    container_path: &str,
) -> Result<(), CiiError> {
    xml.push('<');
    xml.push_str(container);
    xml.push('>');
    write_preserved_xml_before(xml, document, container_path, None, "Name")?;
    write_text_element(xml, "ram:Name", &party.name);
    write_preserved_xml_before(
        xml,
        document,
        container_path,
        None,
        "SpecifiedLegalOrganization",
    )?;
    let legal_organization_path = format!("{container_path}/SpecifiedLegalOrganization");
    if party.id.is_some() || has_preserved_xml(document, &legal_organization_path, None)? {
        xml.push_str("<ram:SpecifiedLegalOrganization>");
        write_preserved_xml_before(xml, document, &legal_organization_path, None, "ID")?;
        if let Some(id) = &party.id {
            write_text_element(xml, "ram:ID", id);
        }
        write_preserved_xml_after_all(xml, document, &legal_organization_path, None)?;
        xml.push_str("</ram:SpecifiedLegalOrganization>");
    }
    write_preserved_xml_before(xml, document, container_path, None, "DefinedTradeContact")?;
    let contact_path = format!("{container_path}/DefinedTradeContact");
    let contact = party.contact.as_ref();
    let contact_has_known_fields = contact.is_some_and(|contact| {
        contact.name.is_some() || contact.email.is_some() || contact.phone.is_some()
    });
    let contact_has_preserved_xml = has_preserved_xml_at_or_below(document, &contact_path, None)?;
    if contact_has_known_fields || contact_has_preserved_xml {
        xml.push_str("<ram:DefinedTradeContact>");
        write_preserved_xml_before(xml, document, &contact_path, None, "PersonName")?;
        if let Some(name) = contact.and_then(|contact| contact.name.as_ref()) {
            write_text_element(xml, "ram:PersonName", name);
        }
        write_preserved_xml_before(
            xml,
            document,
            &contact_path,
            None,
            "TelephoneUniversalCommunication",
        )?;
        let telephone_path = format!("{contact_path}/TelephoneUniversalCommunication");
        if contact.and_then(|contact| contact.phone.as_ref()).is_some()
            || has_preserved_xml(document, &telephone_path, None)?
        {
            xml.push_str("<ram:TelephoneUniversalCommunication>");
            let phone = contact.and_then(|contact| contact.phone.as_ref());
            if let Some(phone) = phone {
                write_preserved_xml_before(xml, document, &telephone_path, None, "CompleteNumber")?;
                write_text_element(xml, "ram:CompleteNumber", phone);
                write_preserved_xml_after_all(xml, document, &telephone_path, None)?;
            } else {
                write_preserved_xml(xml, document, &telephone_path, None, None, None)?;
            }
            xml.push_str("</ram:TelephoneUniversalCommunication>");
        }
        write_preserved_xml_before(
            xml,
            document,
            &contact_path,
            None,
            "EmailURIUniversalCommunication",
        )?;
        let email_path = format!("{contact_path}/EmailURIUniversalCommunication");
        if contact.and_then(|contact| contact.email.as_ref()).is_some()
            || has_preserved_xml(document, &email_path, None)?
        {
            xml.push_str("<ram:EmailURIUniversalCommunication>");
            let email = contact.and_then(|contact| contact.email.as_ref());
            if let Some(email) = email {
                write_preserved_xml_before(xml, document, &email_path, None, "URIID")?;
                write_text_element(xml, "ram:URIID", email);
                write_preserved_xml_after_all(xml, document, &email_path, None)?;
            } else {
                write_preserved_xml(xml, document, &email_path, None, None, None)?;
            }
            xml.push_str("</ram:EmailURIUniversalCommunication>");
        }
        write_preserved_xml_after_all(xml, document, &contact_path, None)?;
        xml.push_str("</ram:DefinedTradeContact>");
    }
    write_preserved_xml_before(xml, document, container_path, None, "PostalTradeAddress")?;
    write_address(
        xml,
        &party.address,
        document,
        &format!("{container_path}/PostalTradeAddress"),
    )?;
    write_preserved_xml_before(
        xml,
        document,
        container_path,
        None,
        "SpecifiedTaxRegistration",
    )?;
    for tax_id in &party.tax_ids {
        xml.push_str("<ram:SpecifiedTaxRegistration>");
        xml.push_str(r#"<ram:ID schemeID=""#);
        let scheme = if tax_id.scheme.eq_ignore_ascii_case("vat") {
            "VA"
        } else {
            tax_id.scheme.as_str()
        };
        write_xml_attr(scheme, xml);
        xml.push_str(r#"">"#);
        write_xml_text(&tax_id.value, xml);
        xml.push_str("</ram:ID>");
        write_preserved_xml_after_all(
            xml,
            document,
            &format!("{container_path}/SpecifiedTaxRegistration"),
            None,
        )?;
        xml.push_str("</ram:SpecifiedTaxRegistration>");
    }
    write_preserved_xml_after_all(xml, document, container_path, None)?;
    xml.push_str("</");
    xml.push_str(container);
    xml.push('>');
    Ok(())
}

fn write_address(
    xml: &mut String,
    address: &PostalAddress,
    document: &CommercialDocument,
    container_path: &str,
) -> Result<(), CiiError> {
    xml.push_str("<ram:PostalTradeAddress>");
    write_preserved_xml_before(xml, document, container_path, None, "PostcodeCode")?;
    write_text_element(xml, "ram:PostcodeCode", &address.postal_code);
    write_preserved_xml_before(xml, document, container_path, None, "LineOne")?;
    if let Some(first) = address.lines.first() {
        write_text_element(xml, "ram:LineOne", first);
    }
    write_preserved_xml_before(xml, document, container_path, None, "LineTwo")?;
    if let Some(second) = address.lines.get(1) {
        write_text_element(xml, "ram:LineTwo", second);
    }
    write_preserved_xml_before(xml, document, container_path, None, "LineThree")?;
    if let Some(extra_lines) = address.lines.get(2..) {
        write_text_element(xml, "ram:LineThree", &extra_lines.join(" "));
    }
    write_preserved_xml_before(xml, document, container_path, None, "CityName")?;
    write_text_element(xml, "ram:CityName", &address.city);
    write_preserved_xml_before(
        xml,
        document,
        container_path,
        None,
        "CountrySubDivisionName",
    )?;
    if let Some(subdivision) = &address.subdivision {
        write_text_element(xml, "ram:CountrySubDivisionName", subdivision);
    }
    write_preserved_xml_before(xml, document, container_path, None, "CountryID")?;
    write_text_element(xml, "ram:CountryID", &string_value(&address.country)?);
    write_preserved_xml_after_all(xml, document, container_path, None)?;
    xml.push_str("</ram:PostalTradeAddress>");
    Ok(())
}

fn write_payment_instruction(
    xml: &mut String,
    instruction: &PaymentInstruction,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    let container_path =
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradeSettlementPaymentMeans";
    xml.push_str("<ram:SpecifiedTradeSettlementPaymentMeans>");
    write_preserved_xml_before(xml, document, container_path, None, "TypeCode")?;
    let code = match instruction.kind {
        PaymentInstructionKind::Sepa | PaymentInstructionKind::IbanBic => "30",
        PaymentInstructionKind::SwissQr
        | PaymentInstructionKind::EpcQr
        | PaymentInstructionKind::ZatcaQr
        | PaymentInstructionKind::Other => "1",
    };
    write_text_element(xml, "ram:TypeCode", code);
    write_preserved_xml_before(
        xml,
        document,
        container_path,
        None,
        "PayeePartyCreditorFinancialAccount",
    )?;
    if let Some(account) = &instruction.account {
        xml.push_str("<ram:PayeePartyCreditorFinancialAccount>");
        write_text_element(xml, "ram:IBANID", account);
        write_preserved_xml_after_all(
            xml,
            document,
            &format!("{container_path}/PayeePartyCreditorFinancialAccount"),
            None,
        )?;
        xml.push_str("</ram:PayeePartyCreditorFinancialAccount>");
    }
    write_preserved_xml_after_all(xml, document, container_path, None)?;
    xml.push_str("</ram:SpecifiedTradeSettlementPaymentMeans>");
    Ok(())
}

fn write_tax_summary(
    xml: &mut String,
    summary: &TaxCategorySummary,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    let container_path =
        "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/ApplicableTradeTax";
    xml.push_str("<ram:ApplicableTradeTax>");
    write_preserved_xml_before(xml, document, container_path, None, "CalculatedAmount")?;
    write_amount_text_element(xml, "ram:CalculatedAmount", summary.tax_amount.inner());
    write_preserved_xml_before(xml, document, container_path, None, "TypeCode")?;
    write_text_element(xml, "ram:TypeCode", "VAT");
    // EN 16931 BT-120 (free-text exemption reason): ram:ExemptionReason sits
    // after ram:TypeCode / before ram:BasisAmount in the CII child order. The
    // native emission is bracketed within the preserved-XML replay (same
    // machinery as DesignatedProductClassification): replay siblings up to the
    // ExemptionReason slot first, then emit the native element. Emit nothing
    // when absent so documents without exemption text serialize byte-identically.
    write_preserved_xml_before(xml, document, container_path, None, "ExemptionReason")?;
    if let Some(exemption_reason) = &summary.exemption_reason {
        write_text_element(xml, "ram:ExemptionReason", exemption_reason);
    }
    write_preserved_xml_before(xml, document, container_path, None, "BasisAmount")?;
    write_amount_text_element(xml, "ram:BasisAmount", summary.taxable_amount.inner());
    write_preserved_xml_before(xml, document, container_path, None, "CategoryCode")?;
    write_text_element(xml, "ram:CategoryCode", &summary.category_code);
    // EN 16931 BT-121 (controlled-list exemption code, e.g. VATEX-EU-AE or an IT
    // Natura code): ram:ExemptionReasonCode sits after ram:CategoryCode / before
    // ram:RateApplicablePercent. Serialized verbatim — InvoiceKit does not map,
    // translate, or invent codes. Bracketed within the preserved-XML replay.
    write_preserved_xml_before(xml, document, container_path, None, "ExemptionReasonCode")?;
    if let Some(exemption_reason_code) = &summary.exemption_reason_code {
        write_text_element(xml, "ram:ExemptionReasonCode", exemption_reason_code);
    }
    write_preserved_xml_before(xml, document, container_path, None, "RateApplicablePercent")?;
    if let Some(rate) = &summary.tax_rate {
        write_text_element(xml, "ram:RateApplicablePercent", &rate.inner().to_string());
    }
    write_preserved_xml_after_all(xml, document, container_path, None)?;
    xml.push_str("</ram:ApplicableTradeTax>");
    Ok(())
}

fn write_monetary_total(
    xml: &mut String,
    total: &MonetaryTotal,
    currency: &str,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    let container_path = "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradeSettlementHeaderMonetarySummation";
    xml.push_str("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
    write_preserved_xml_before(xml, document, container_path, None, "LineTotalAmount")?;
    write_amount_text_element(
        xml,
        "ram:LineTotalAmount",
        total.line_extension_amount.inner(),
    );
    write_preserved_xml_before(xml, document, container_path, None, "AllowanceTotalAmount")?;
    if let Some(value) = &total.allowance_total_amount {
        write_amount_text_element(xml, "ram:AllowanceTotalAmount", value.inner());
    }
    write_preserved_xml_before(xml, document, container_path, None, "ChargeTotalAmount")?;
    if let Some(value) = &total.charge_total_amount {
        write_amount_text_element(xml, "ram:ChargeTotalAmount", value.inner());
    }
    write_preserved_xml_before(xml, document, container_path, None, "TaxBasisTotalAmount")?;
    write_amount_text_element(
        xml,
        "ram:TaxBasisTotalAmount",
        total.tax_exclusive_amount.inner(),
    );
    write_preserved_xml_before(xml, document, container_path, None, "TaxTotalAmount")?;
    xml.push_str(r#"<ram:TaxTotalAmount currencyID=""#);
    write_xml_attr(currency, xml);
    xml.push_str(r#"">"#);
    // Attacker-controlled amounts can push this difference past Decimal::MAX /
    // MIN; `Decimal`'s `Sub` panics on overflow. Use `checked_sub` and emit an
    // empty (still well-formed) element on overflow rather than panicking. For
    // every real invoice the result is identical to the prior subtraction.
    if let Some(tax_total) = total
        .tax_inclusive_amount
        .inner()
        .checked_sub(total.tax_exclusive_amount.inner())
    {
        write_xml_text(&tax_total.to_string(), xml);
    }
    xml.push_str("</ram:TaxTotalAmount>");
    write_preserved_xml_before(xml, document, container_path, None, "GrandTotalAmount")?;
    write_amount_text_element(
        xml,
        "ram:GrandTotalAmount",
        total.tax_inclusive_amount.inner(),
    );
    write_preserved_xml_before(xml, document, container_path, None, "TotalPrepaidAmount")?;
    if let Some(value) = &total.prepaid_amount {
        write_amount_text_element(xml, "ram:TotalPrepaidAmount", value.inner());
    }
    write_preserved_xml_before(xml, document, container_path, None, "DuePayableAmount")?;
    write_amount_text_element(xml, "ram:DuePayableAmount", total.payable_amount.inner());
    write_preserved_xml_after_all(xml, document, container_path, None)?;
    xml.push_str("</ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
    Ok(())
}

fn write_line(
    xml: &mut String,
    line: &DocumentLine,
    _currency: &str,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    let line_path =
        "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem";
    xml.push_str("<ram:IncludedSupplyChainTradeLineItem>");
    write_preserved_xml_before(
        xml,
        document,
        line_path,
        Some(&line.id),
        "AssociatedDocumentLineDocument",
    )?;
    xml.push_str("<ram:AssociatedDocumentLineDocument>");
    write_text_element(xml, "ram:LineID", &line.id);
    write_preserved_xml_after_all(
        xml,
        document,
        &format!("{line_path}/AssociatedDocumentLineDocument"),
        Some(&line.id),
    )?;
    xml.push_str("</ram:AssociatedDocumentLineDocument>");
    write_preserved_xml_before(
        xml,
        document,
        line_path,
        Some(&line.id),
        "SpecifiedTradeProduct",
    )?;
    write_specified_trade_product(xml, line, document, line_path)?;
    write_preserved_xml_before(
        xml,
        document,
        line_path,
        Some(&line.id),
        "SpecifiedLineTradeAgreement",
    )?;
    xml.push_str("<ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice>");
    write_amount_text_element(xml, "ram:ChargeAmount", line.unit_price.inner());
    write_preserved_xml_after_all(
        xml,
        document,
        &format!("{line_path}/SpecifiedLineTradeAgreement/NetPriceProductTradePrice"),
        Some(&line.id),
    )?;
    xml.push_str("</ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement>");
    write_preserved_xml_before(
        xml,
        document,
        line_path,
        Some(&line.id),
        "SpecifiedLineTradeDelivery",
    )?;
    xml.push_str("<ram:SpecifiedLineTradeDelivery>");
    xml.push_str(r"<ram:BilledQuantity");
    if let Some(unit_code) = &line.unit_code {
        xml.push_str(r#" unitCode=""#);
        write_xml_attr(unit_code, xml);
        xml.push('"');
    }
    xml.push('>');
    write_xml_text(&line.quantity.inner().to_string(), xml);
    xml.push_str("</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery>");
    write_preserved_xml_before(
        xml,
        document,
        line_path,
        Some(&line.id),
        "SpecifiedLineTradeSettlement",
    )?;
    xml.push_str("<ram:SpecifiedLineTradeSettlement>");
    if let Some(category) = &line.tax_category {
        xml.push_str("<ram:ApplicableTradeTax>");
        write_text_element(xml, "ram:TypeCode", "VAT");
        write_text_element(xml, "ram:CategoryCode", category);
        xml.push_str("</ram:ApplicableTradeTax>");
    }
    xml.push_str("<ram:SpecifiedTradeSettlementLineMonetarySummation>");
    write_amount_text_element(
        xml,
        "ram:LineTotalAmount",
        line.line_extension_amount.inner(),
    );
    xml.push_str("</ram:SpecifiedTradeSettlementLineMonetarySummation>");
    write_preserved_xml_after_all(
        xml,
        document,
        &format!("{line_path}/SpecifiedLineTradeSettlement"),
        Some(&line.id),
    )?;
    xml.push_str("</ram:SpecifiedLineTradeSettlement>");
    write_preserved_xml_after_all(xml, document, line_path, Some(&line.id))?;
    xml.push_str("</ram:IncludedSupplyChainTradeLineItem>");
    Ok(())
}

/// Emit `<ram:SpecifiedTradeProduct>` with its `ram:Name`, the native EN 16931
/// BT-158 classifications, and any preserved siblings replayed in schema order.
///
/// The native `ram:DesignatedProductClassification` emission is bracketed by the
/// preserved replay: children that precede it (in CII schema order) are replayed
/// first via `write_preserved_xml_before`, then the native classifications land
/// at their slot, then `write_preserved_xml_after_all` replays the trailing
/// children. This keeps element order valid even when a line also carries a
/// lower-order preserved `SpecifiedTradeProduct` sibling.
fn write_specified_trade_product(
    xml: &mut String,
    line: &DocumentLine,
    document: &CommercialDocument,
    line_path: &str,
) -> Result<(), CiiError> {
    xml.push_str("<ram:SpecifiedTradeProduct>");
    write_text_element(xml, "ram:Name", &line.description);
    let product_path = format!("{line_path}/SpecifiedTradeProduct");
    write_preserved_xml_before(
        xml,
        document,
        &product_path,
        Some(&line.id),
        "DesignatedProductClassification",
    )?;
    write_designated_product_classifications(xml, &line.classifications);
    write_preserved_xml_after_all(xml, document, &product_path, Some(&line.id))?;
    xml.push_str("</ram:SpecifiedTradeProduct>");
    Ok(())
}

/// Emit the EN 16931 BT-158 commodity classifications as
/// `ram:DesignatedProductClassification/ram:ClassCode[@listID,@listVersionID]`.
/// Empty when the line carries no classifications, so unclassified lines
/// serialize byte-identically to before this binding existed.
fn write_designated_product_classifications(
    xml: &mut String,
    classifications: &[ItemClassification],
) {
    for classification in classifications {
        xml.push_str("<ram:DesignatedProductClassification><ram:ClassCode");
        if !classification.scheme_id.is_empty() {
            xml.push_str(r#" listID=""#);
            write_xml_attr(&classification.scheme_id, xml);
            xml.push('"');
        }
        if let Some(scheme_version) = &classification.scheme_version {
            xml.push_str(r#" listVersionID=""#);
            write_xml_attr(scheme_version, xml);
            xml.push('"');
        }
        xml.push('>');
        write_xml_text(&classification.code, xml);
        xml.push_str("</ram:ClassCode></ram:DesignatedProductClassification>");
    }
}

/// Emit the EN 16931 BT-13 purchase order reference as
/// `ram:BuyerOrderReferencedDocument/ram:IssuerAssignedID` under
/// `ram:ApplicableHeaderTradeAgreement`, sourced from the first
/// [`ReferenceKindClass::Order`] entry in `document.references`.
///
/// BT-13 is `0..1`: only the first Order-class reference is emitted. The
/// element is omitted entirely when no Order-class reference exists, so
/// documents without one serialize byte-identically to before this binding.
///
/// Returns `true` when a native element was emitted, so the caller can suppress
/// a same-order preserved fragment and avoid double-filling the BT-13 slot.
fn write_buyer_order_referenced_document(xml: &mut String, document: &CommercialDocument) -> bool {
    let Some(reference) = document
        .references
        .iter()
        .find(|reference| reference.kind_class() == ReferenceKindClass::Order)
    else {
        return false;
    };
    xml.push_str("<ram:BuyerOrderReferencedDocument>");
    write_text_element(xml, "ram:IssuerAssignedID", &reference.id);
    xml.push_str("</ram:BuyerOrderReferencedDocument>");
    true
}

/// Emit the EN 16931 BT-25 preceding-invoice references as
/// `ram:InvoiceReferencedDocument/ram:IssuerAssignedID` (plus an optional
/// `ram:FormattedIssueDateTime` carrying BT-26 when the reference issue date is
/// present) under `ram:ApplicableHeaderTradeSettlement`, one element per
/// [`ReferenceKindClass::PrecedingInvoice`] entry in `document.references`.
///
/// BG-3 is `0..n`: every preceding-invoice reference is emitted in IR order.
/// Empty when the document carries no preceding-invoice reference, so such
/// documents serialize byte-identically to before this binding existed.
///
/// Returns `true` when at least one native element was emitted, so the caller
/// can suppress a same-order preserved fragment and avoid double-filling the
/// BT-25 slot.
fn write_invoice_referenced_documents(
    xml: &mut String,
    document: &CommercialDocument,
) -> Result<bool, CiiError> {
    let mut emitted = false;
    for reference in document
        .references
        .iter()
        .filter(|reference| reference.kind_class() == ReferenceKindClass::PrecedingInvoice)
    {
        xml.push_str("<ram:InvoiceReferencedDocument>");
        write_text_element(xml, "ram:IssuerAssignedID", &reference.id);
        if let Some(issue_date) = &reference.issue_date {
            xml.push_str(r#"<ram:FormattedIssueDateTime><qdt:DateTimeString format="102">"#);
            write_xml_text(&iso_date_to_cii(issue_date)?, xml);
            xml.push_str("</qdt:DateTimeString></ram:FormattedIssueDateTime>");
        }
        xml.push_str("</ram:InvoiceReferencedDocument>");
        emitted = true;
    }
    Ok(emitted)
}

/// Emit the EN 16931 BG-14 invoice period (BT-73 start / BT-74 end) as
/// `ram:BillingSpecifiedPeriod/ram:StartDateTime|ram:EndDateTime` (each a
/// `udt:DateTimeString format="102"`) under `ram:ApplicableHeaderTradeSettlement`,
/// from the native IR.
///
/// `ram:BillingSpecifiedPeriod` (`ram:SpecifiedPeriodType`, verified against the
/// vendored CII D16B element catalog) is `lossiness_ledger_preserved` on parse
/// and never populates [`CommercialDocument::invoice_period`], so a parsed
/// document replays its preserved fragment and a fresh IR document emits here.
/// Emitted textually right after the `ram:ApplicableTradeTax` loop — its schema
/// slot — so a preserved fragment is flushed by the following
/// `write_preserved_xml_before(.., "SpecifiedTradePaymentTerms")`. The caller
/// gates this call on the absence of a preserved `BillingSpecifiedPeriod`
/// (preserved wins), so even a parse-then-enrich document that carries BOTH a
/// preserved fragment and a caller-set `invoice_period` emits the element
/// exactly once (BG-14 is 0..1) — never a malformed duplicate.
fn write_billing_specified_period(
    xml: &mut String,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    let Some(period) = &document.invoice_period else {
        return Ok(());
    };
    xml.push_str("<ram:BillingSpecifiedPeriod>");
    if let Some(start) = &period.start_date {
        xml.push_str("<ram:StartDateTime>");
        write_date_time(xml, start)?;
        xml.push_str("</ram:StartDateTime>");
    }
    if let Some(end) = &period.end_date {
        xml.push_str("<ram:EndDateTime>");
        write_date_time(xml, end)?;
        xml.push_str("</ram:EndDateTime>");
    }
    xml.push_str("</ram:BillingSpecifiedPeriod>");
    Ok(())
}

/// Emit EN 16931 document-level allowances (BG-20) and charges (BG-21) from the
/// native IR as `ram:SpecifiedTradeAllowanceCharge` elements, in CII
/// `TradeAllowanceChargeType` child order (`ChargeIndicator`,
/// `CalculationPercent`, `BasisAmount`, `ActualAmount`, `ReasonCode`, `Reason`,
/// `CategoryTradeTax`). Element names verified against the vendored CII D16B
/// element catalog.
///
/// `ram:SpecifiedTradeAllowanceCharge` is `0..n` and `lossiness_ledger_preserved`
/// on parse, so a preserved fragment (flushed by the `SpecifiedTradePaymentTerms`
/// replay) and native entries legitimately coexist — emitted unconditionally,
/// not gated (the repeatable analogue of UBL `cac:AllowanceCharge`).
fn write_cii_allowance_charges(xml: &mut String, document: &CommercialDocument) {
    for allowance_charge in &document.allowance_charges {
        xml.push_str("<ram:SpecifiedTradeAllowanceCharge><ram:ChargeIndicator><udt:Indicator>");
        xml.push_str(if allowance_charge.is_charge {
            "true"
        } else {
            "false"
        });
        xml.push_str("</udt:Indicator></ram:ChargeIndicator>");
        if let Some(percentage) = &allowance_charge.percentage {
            write_text_element(xml, "ram:CalculationPercent", &percentage.inner().to_string());
        }
        if let Some(base) = &allowance_charge.base_amount {
            write_amount_text_element(xml, "ram:BasisAmount", base.inner());
        }
        write_amount_text_element(xml, "ram:ActualAmount", allowance_charge.amount.inner());
        if let Some(code) = &allowance_charge.reason_code {
            write_text_element(xml, "ram:ReasonCode", code);
        }
        if let Some(reason) = &allowance_charge.reason {
            write_text_element(xml, "ram:Reason", reason);
        }
        // CategoryTradeTax (a TradeTaxType): TypeCode, CategoryCode,
        // RateApplicablePercent. Emitted only when a category code is present.
        if let Some(category) = &allowance_charge.tax_category {
            xml.push_str("<ram:CategoryTradeTax>");
            write_text_element(xml, "ram:TypeCode", "VAT");
            write_text_element(xml, "ram:CategoryCode", category);
            if let Some(rate) = &allowance_charge.tax_rate {
                write_text_element(xml, "ram:RateApplicablePercent", &rate.inner().to_string());
            }
            xml.push_str("</ram:CategoryTradeTax>");
        }
        xml.push_str("</ram:SpecifiedTradeAllowanceCharge>");
    }
}

fn write_note(
    xml: &mut String,
    note: &LocalizedString,
    document: &CommercialDocument,
) -> Result<(), CiiError> {
    xml.push_str("<ram:IncludedNote>");
    write_preserved_xml_before(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument/IncludedNote",
        None,
        "Content",
    )?;
    write_text_element(xml, "ram:Content", &note.text);
    write_preserved_xml_after_all(
        xml,
        document,
        "CrossIndustryInvoice/ExchangedDocument/IncludedNote",
        None,
    )?;
    xml.push_str("</ram:IncludedNote>");
    Ok(())
}

fn write_date_time(xml: &mut String, date: &DateOnly) -> Result<(), CiiError> {
    xml.push_str(r#"<udt:DateTimeString format="102">"#);
    write_xml_text(&iso_date_to_cii(date)?, xml);
    xml.push_str("</udt:DateTimeString>");
    Ok(())
}

fn write_text_element(xml: &mut String, name: &str, value: &str) {
    xml.push('<');
    xml.push_str(name);
    xml.push('>');
    write_xml_text(value, xml);
    xml.push_str("</");
    xml.push_str(name);
    xml.push('>');
}

fn write_amount_text_element(xml: &mut String, name: &str, amount: Decimal) {
    write_text_element(xml, name, &amount.to_string());
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

fn string_value<T: Serialize>(value: &T) -> Result<String, CiiError> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or(CiiError::MissingElement("serialized IR newtype string"))
}

fn decimal_value(path: &'static str, value: &str) -> Result<DecimalValue, CiiError> {
    Decimal::from_str(value)
        .map(DecimalValue::new)
        .map_err(|_| CiiError::InvalidDecimal {
            path,
            value: value.to_owned(),
        })
}

fn cii_date_to_iso(path: &'static str, value: &str) -> Result<String, CiiError> {
    if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let iso = format!("{}-{}-{}", &value[0..4], &value[4..6], &value[6..8]);
        DateOnly::new(iso.clone()).map_err(|_| CiiError::InvalidDate {
            path,
            value: value.to_owned(),
        })?;
        Ok(iso)
    } else if DateOnly::new(value.to_owned()).is_ok() {
        Ok(value.to_owned())
    } else {
        Err(CiiError::InvalidDate {
            path,
            value: value.to_owned(),
        })
    }
}

fn iso_date_to_cii(date: &DateOnly) -> Result<String, CiiError> {
    let value = date.as_str();
    if value.len() == 10 {
        Ok(format!("{}{}{}", &value[0..4], &value[5..7], &value[8..10]))
    } else {
        Err(CiiError::InvalidDate {
            path: "DateOnly",
            value: value.to_owned(),
        })
    }
}

fn document_type(code: &str) -> Result<DocumentType, CiiError> {
    match code {
        "380" => Ok(DocumentType::Invoice),
        "381" => Ok(DocumentType::CreditNote),
        other => Err(CiiError::UnsupportedTypeCode(other.to_owned())),
    }
}

fn document_type_code(document: &CommercialDocument) -> Result<&'static str, CiiError> {
    match document.document_type {
        DocumentType::Invoice => Ok("380"),
        DocumentType::CreditNote => Ok("381"),
        other => Err(CiiError::UnsupportedDocumentType(other)),
    }
}

fn payment_kind(code: &str) -> PaymentInstructionKind {
    match code {
        "30" | "58" => PaymentInstructionKind::IbanBic,
        _ => PaymentInstructionKind::Other,
    }
}

fn read_attrs(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
) -> Result<Vec<XmlAttribute>, CiiError> {
    let mut attrs = Vec::new();
    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        let raw_key = decode_name(attr.key.as_ref())?;
        if raw_key == "xmlns" || raw_key.starts_with("xmlns:") {
            continue;
        }
        let key = local_xml_name(raw_key).to_owned();
        let value = attr
            .decoded_and_normalized_value(xml_version, reader.decoder())?
            .into_owned();
        attrs.push(XmlAttribute {
            local_name: key,
            value,
        });
    }
    Ok(attrs)
}

fn attr_value<'a>(attrs: &'a [XmlAttribute], local_name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.local_name == local_name)
        .map(|attr| attr.value.as_str())
}

fn reader_position(position: u64) -> Result<usize, CiiError> {
    usize::try_from(position).map_err(|_| CiiError::UnsupportedRoot("xml-position".to_owned()))
}

fn input_slice(input: &str, start: usize, end: usize) -> Result<&str, CiiError> {
    input
        .get(start..end)
        .ok_or_else(|| CiiError::UnsupportedRoot("xml-position-range".to_owned()))
}

fn preserved_string_field(payload: &Map<String, Value>, key: &str) -> Result<String, CiiError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CiiError::InvalidPreservedXml {
            container: payload
                .get(CII_PRESERVED_CONTAINER_KEY)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            element: payload
                .get(CII_PRESERVED_ELEMENT_KEY)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            message: format!("{key} must be a string"),
        })
}

fn namespace_declarations_for_start(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
) -> Result<Vec<XmlNamespaceBinding>, CiiError> {
    let mut declarations = Vec::new();
    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        let key = decode_name(attr.key.as_ref())?;
        let prefix = if key == "xmlns" {
            None
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            if prefix == "xml" || prefix == "xmlns" {
                continue;
            }
            Some(prefix.to_owned())
        } else {
            continue;
        };
        let uri = attr
            .decoded_and_normalized_value(xml_version, reader.decoder())?
            .into_owned();
        declarations.push(XmlNamespaceBinding { prefix, uri });
    }
    Ok(declarations)
}

fn effective_namespace_bindings(
    stack: &[Vec<XmlNamespaceBinding>],
    declarations: &[XmlNamespaceBinding],
) -> Vec<XmlNamespaceBinding> {
    let mut frame = Vec::<XmlNamespaceBinding>::new();
    for scope in stack
        .iter()
        .flat_map(|scope| scope.iter())
        .chain(declarations)
    {
        frame.retain(|binding| binding.prefix != scope.prefix);
        frame.push(scope.clone());
    }
    frame
}

fn resolve_element_namespace(
    raw_name: &[u8],
    stack: &[Vec<XmlNamespaceBinding>],
    declarations: Option<&[XmlNamespaceBinding]>,
) -> Result<String, CiiError> {
    let name = decode_name(raw_name)?;
    let (prefix, _local_name) = split_xml_name(name);
    if prefix == "xml" {
        return Ok("http://www.w3.org/XML/1998/namespace".to_owned());
    }
    for binding in declarations
        .into_iter()
        .flatten()
        .rev()
        .chain(stack.iter().rev().flat_map(|scope| scope.iter().rev()))
    {
        if binding.prefix.as_deref().unwrap_or_default() == prefix {
            return Ok(binding.uri.clone());
        }
    }
    Ok(if prefix.is_empty() {
        "unbound".to_owned()
    } else {
        format!("unknown prefix `{prefix}`")
    })
}

fn validate_cii_namespace(
    namespace: &str,
    parent_stack: &[String],
    name: &str,
) -> Result<(), CiiError> {
    if parent_stack.is_empty() && name != "CrossIndustryInvoice" {
        return Ok(());
    }
    let expected = expected_cii_namespace(parent_stack, name);
    if namespace != expected {
        return Err(CiiError::InvalidNamespace {
            element: name.to_owned(),
            expected,
            actual: namespace.to_owned(),
        });
    }
    Ok(())
}

fn expected_cii_namespace(parent_stack: &[String], name: &str) -> &'static str {
    if parent_stack.is_empty() && name == "CrossIndustryInvoice"
        || parent_stack
            .last()
            .is_some_and(|parent| parent == "CrossIndustryInvoice")
    {
        CII_RSM_NAMESPACE_URI
    } else if name == "DateTimeString" {
        // `DateTimeString` is `udt:` under a `udt:DateTimeType` parent
        // (`IssueDateTime`, `TaxPointDate`, `DueDateDateTime`, …) but `qdt:`
        // under a `qdt:FormattedDateTimeType` parent — the `Formatted*DateTime`
        // elements such as `FormattedIssueDateTime` (BT-26 on a referenced
        // document). Without this distinction the parser rejects its own valid
        // output for a dated reference.
        if parent_stack
            .last()
            .is_some_and(|parent| parent.starts_with("Formatted") && parent.ends_with("DateTime"))
        {
            CII_QDT_NAMESPACE_URI
        } else {
            CII_UDT_NAMESPACE_URI
        }
    } else if name == "Indicator" {
        // `udt:Indicator` is the content of a `udt:IndicatorType` element such as
        // `ram:ChargeIndicator` (the allowance-vs-charge flag on a
        // `ram:SpecifiedTradeAllowanceCharge`). Without this the parser rejects
        // its own valid allowance/charge output.
        CII_UDT_NAMESPACE_URI
    } else {
        CII_RAM_NAMESPACE_URI
    }
}

fn canonicalize_preserved_xml_fragment(
    xml: &str,
    namespaces: &[XmlNamespaceBinding],
) -> Result<String, CiiError> {
    let wrapper_prefix = preserved_wrapper_prefix(namespaces);
    let wrapper_name = format!("{wrapper_prefix}:wrapper");
    let mut wrapped = String::new();
    wrapped.push('<');
    wrapped.push_str(&wrapper_name);
    wrapped.push_str(" xmlns:");
    wrapped.push_str(&wrapper_prefix);
    wrapped.push_str("=\"");
    write_xml_attr("urn:invoicekit:preserved-fragment-wrapper", &mut wrapped);
    wrapped.push('"');
    for namespace in namespaces {
        if namespace
            .prefix
            .as_deref()
            .is_some_and(|prefix| prefix == "xml" || prefix == "xmlns" || prefix == wrapper_prefix)
        {
            continue;
        }
        match namespace.prefix.as_deref() {
            Some(prefix) => {
                wrapped.push_str(" xmlns:");
                wrapped.push_str(prefix);
            }
            None => wrapped.push_str(" xmlns"),
        }
        wrapped.push_str("=\"");
        write_xml_attr(&namespace.uri, &mut wrapped);
        wrapped.push('"');
    }
    wrapped.push('>');
    wrapped.push_str(xml);
    wrapped.push_str("</");
    wrapped.push_str(&wrapper_name);
    wrapped.push('>');

    let canonical = canonicalize_xml(&wrapped)?;
    let start_end = canonical
        .find('>')
        .ok_or_else(|| CiiError::UnsupportedRoot("preserved-wrapper".to_owned()))?;
    let end_tag = format!("</{wrapper_name}>");
    if !canonical.ends_with(&end_tag) {
        return Err(CiiError::UnsupportedRoot("preserved-wrapper".to_owned()));
    }
    let body_end = canonical.len() - end_tag.len();
    canonical
        .get(start_end + 1..body_end)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CiiError::UnsupportedRoot("preserved-wrapper".to_owned()))
}

fn preserved_wrapper_prefix(namespaces: &[XmlNamespaceBinding]) -> String {
    let base = "ikp";
    if namespaces
        .iter()
        .all(|namespace| namespace.prefix.as_deref() != Some(base))
    {
        return base.to_owned();
    }
    for index in 0..100 {
        let candidate = format!("{base}{index}");
        if namespaces
            .iter()
            .all(|namespace| namespace.prefix.as_deref() != Some(candidate.as_str()))
        {
            return candidate;
        }
    }
    format!("{base}Preserved")
}

fn should_preserve_raw_xml(stack: &[String], name: &str) -> bool {
    stack
        .last()
        .and_then(|parent| known_cii_children(parent))
        .is_some_and(|known| !known.contains(&name))
}

#[allow(clippy::too_many_lines)]
fn known_cii_children(parent: &str) -> Option<&'static [&'static str]> {
    match parent {
        "CrossIndustryInvoice" => Some(&[
            "ExchangedDocumentContext",
            "ExchangedDocument",
            "SupplyChainTradeTransaction",
        ]),
        "ExchangedDocumentContext" => Some(&[
            "SpecifiedTransactionID",
            "TestIndicator",
            "BusinessProcessSpecifiedDocumentContextParameter",
            "ApplicationSpecifiedDocumentContextParameter",
            "GuidelineSpecifiedDocumentContextParameter",
        ]),
        "BusinessProcessSpecifiedDocumentContextParameter"
        | "GuidelineSpecifiedDocumentContextParameter"
        | "ApplicationSpecifiedDocumentContextParameter" => Some(&["ID", "Value"]),
        "ExchangedDocument" => Some(&["ID", "TypeCode", "IssueDateTime", "IncludedNote"]),
        "IncludedNote" => Some(&["Content"]),
        "SupplyChainTradeTransaction" => Some(&[
            "IncludedSupplyChainTradeLineItem",
            "ApplicableHeaderTradeAgreement",
            "ApplicableHeaderTradeDelivery",
            "ApplicableHeaderTradeSettlement",
        ]),
        "IncludedSupplyChainTradeLineItem" => Some(&[
            "AssociatedDocumentLineDocument",
            "SpecifiedTradeProduct",
            "SpecifiedLineTradeAgreement",
            "SpecifiedLineTradeDelivery",
            "SpecifiedLineTradeSettlement",
        ]),
        "AssociatedDocumentLineDocument" => Some(&["LineID"]),
        "SpecifiedTradeProduct" => Some(&["Name", "Description", "DesignatedProductClassification"]),
        "SpecifiedLineTradeAgreement" => Some(&["NetPriceProductTradePrice"]),
        "NetPriceProductTradePrice" => Some(&["ChargeAmount"]),
        "SpecifiedLineTradeDelivery" => Some(&["BilledQuantity", "CreditedQuantity"]),
        "SpecifiedLineTradeSettlement" => Some(&[
            "ApplicableTradeTax",
            "SpecifiedTradeSettlementLineMonetarySummation",
        ]),
        "SpecifiedTradeSettlementLineMonetarySummation" => Some(&["LineTotalAmount"]),
        "ApplicableHeaderTradeAgreement" => {
            Some(&["BuyerReference", "SellerTradeParty", "BuyerTradeParty"])
        }
        "ApplicableHeaderTradeDelivery" => Some(&["ActualDeliverySupplyChainEvent"]),
        "ActualDeliverySupplyChainEvent" => Some(&["OccurrenceDateTime"]),
        "ApplicableHeaderTradeSettlement" => Some(&[
            "PayeeTradeParty",
            "PaymentReference",
            "InvoiceCurrencyCode",
            "SpecifiedTradeSettlementPaymentMeans",
            "ApplicableTradeTax",
            "SpecifiedTradePaymentTerms",
            "SpecifiedTradeSettlementHeaderMonetarySummation",
            "TaxPointDate",
        ]),
        "SpecifiedTradeSettlementPaymentMeans" => {
            Some(&["TypeCode", "PayeePartyCreditorFinancialAccount"])
        }
        "PayeePartyCreditorFinancialAccount" => Some(&["IBANID", "ProprietaryID"]),
        "SpecifiedTradePaymentTerms" => Some(&["Description", "DueDateDateTime"]),
        "ApplicableTradeTax" => Some(&[
            "CalculatedAmount",
            "TypeCode",
            "ExemptionReason",
            "BasisAmount",
            "CategoryCode",
            "ExemptionReasonCode",
            "RateApplicablePercent",
        ]),
        "SpecifiedTradeSettlementHeaderMonetarySummation" => Some(&[
            "LineTotalAmount",
            "AllowanceTotalAmount",
            "ChargeTotalAmount",
            "TaxBasisTotalAmount",
            "TaxTotalAmount",
            "GrandTotalAmount",
            "TotalPrepaidAmount",
            "DuePayableAmount",
        ]),
        "SellerTradeParty" | "BuyerTradeParty" | "PayeeTradeParty" => Some(&[
            "Name",
            "SpecifiedLegalOrganization",
            "DefinedTradeContact",
            "PostalTradeAddress",
            "SpecifiedTaxRegistration",
        ]),
        "SpecifiedLegalOrganization" | "SpecifiedTaxRegistration" => Some(&["ID"]),
        "DefinedTradeContact" => Some(&[
            "PersonName",
            "TelephoneUniversalCommunication",
            "EmailURIUniversalCommunication",
        ]),
        "TelephoneUniversalCommunication" => Some(&["CompleteNumber"]),
        "EmailURIUniversalCommunication" => Some(&["URIID"]),
        "PostalTradeAddress" => Some(&[
            "PostcodeCode",
            "LineOne",
            "LineTwo",
            "LineThree",
            "CityName",
            "CountrySubDivisionName",
            "CountryID",
        ]),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn cii_child_order(parent: &str, child: &str) -> Option<u16> {
    match parent {
        "CrossIndustryInvoice" => child_order(
            child,
            &[
                "ExchangedDocumentContext",
                "ExchangedDocument",
                "SupplyChainTradeTransaction",
                "ValuationBreakdownStatement",
            ],
        ),
        "ExchangedDocumentContext" => child_order(
            child,
            &[
                "SpecifiedTransactionID",
                "TestIndicator",
                "BusinessProcessSpecifiedDocumentContextParameter",
                "BIMSpecifiedDocumentContextParameter",
                "ScenarioSpecifiedDocumentContextParameter",
                "ApplicationSpecifiedDocumentContextParameter",
                "GuidelineSpecifiedDocumentContextParameter",
                "SubsetSpecifiedDocumentContextParameter",
                "MessageStandardSpecifiedDocumentContextParameter",
            ],
        ),
        "BusinessProcessSpecifiedDocumentContextParameter"
        | "GuidelineSpecifiedDocumentContextParameter"
        | "ApplicationSpecifiedDocumentContextParameter"
        | "BIMSpecifiedDocumentContextParameter"
        | "ScenarioSpecifiedDocumentContextParameter"
        | "SubsetSpecifiedDocumentContextParameter"
        | "MessageStandardSpecifiedDocumentContextParameter" => {
            child_order(child, &["ID", "Value", "SpecifiedDocumentVersion"])
        }
        "ExchangedDocument" => child_order(
            child,
            &[
                "ID",
                "Name",
                "TypeCode",
                "IssueDateTime",
                "CopyIndicator",
                "Purpose",
                "ControlRequirementIndicator",
                "LanguageID",
                "PurposeCode",
                "RevisionDateTime",
                "VersionID",
                "GlobalID",
                "RevisionID",
                "PreviousRevisionID",
                "CategoryCode",
                "IncludedNote",
                "EffectiveSpecifiedPeriod",
                "IssuerTradeParty",
            ],
        ),
        "IncludedNote" => child_order(
            child,
            &["Subject", "ContentCode", "Content", "SubjectCode", "ID"],
        ),
        "SupplyChainTradeTransaction" => child_order(
            child,
            &[
                "IncludedSupplyChainTradeLineItem",
                "ApplicableHeaderTradeAgreement",
                "ApplicableHeaderTradeDelivery",
                "ApplicableHeaderTradeSettlement",
            ],
        ),
        "IncludedSupplyChainTradeLineItem" => child_order(
            child,
            &[
                "DescriptionCode",
                "AssociatedDocumentLineDocument",
                "SpecifiedTradeProduct",
                "SpecifiedLineTradeAgreement",
                "SpecifiedLineTradeDelivery",
                "SpecifiedLineTradeSettlement",
                "IncludedSubordinateTradeLineItem",
            ],
        ),
        "AssociatedDocumentLineDocument" => child_order(
            child,
            &[
                "LineID",
                "ParentLineID",
                "LineStatusCode",
                "LineStatusReasonCode",
                "IncludedNote",
            ],
        ),
        "SpecifiedTradeProduct" => child_order(
            child,
            &[
                "ID",
                "GlobalID",
                "SellerAssignedID",
                "BuyerAssignedID",
                "ManufacturerAssignedID",
                "Name",
                "TradeName",
                "Description",
                "TypeCode",
                "NetWeightMeasure",
                "GrossWeightMeasure",
                "ProductGroupID",
                "EndItemTypeCode",
                "EndItemName",
                "AreaDensityMeasure",
                "UseDescription",
                "BrandName",
                "SubBrandName",
                "DrainedNetWeightMeasure",
                "VariableMeasureIndicator",
                "ColourCode",
                "ColourDescription",
                "Designation",
                "FormattedCancellationAnnouncedLaunchDateTime",
                "FormattedLatestProductDataChangeDateTime",
                "ApplicableProductCharacteristic",
                "ApplicableMaterialGoodsCharacteristic",
                "DesignatedProductClassification",
                "IndividualTradeProductInstance",
                "CertificationEvidenceReferenceReferencedDocument",
                "InspectionReferenceReferencedDocument",
                "OriginTradeCountry",
                "LinearSpatialDimension",
                "MinimumLinearSpatialDimension",
                "MaximumLinearSpatialDimension",
                "ManufacturerTradeParty",
                "PresentationSpecifiedBinaryFile",
                "MSDSReferenceReferencedDocument",
                "AdditionalReferenceReferencedDocument",
                "LegalRightsOwnerTradeParty",
                "BrandOwnerTradeParty",
                "IncludedReferencedProduct",
                "InformationNote",
            ],
        ),
        "SpecifiedLineTradeAgreement" => child_order(
            child,
            &[
                "BuyerReference",
                "BuyerRequisitionerTradeParty",
                "ApplicableTradeDeliveryTerms",
                "SellerOrderReferencedDocument",
                "BuyerOrderReferencedDocument",
                "QuotationReferencedDocument",
                "ContractReferencedDocument",
                "DemandForecastReferencedDocument",
                "PromotionalDealReferencedDocument",
                "AdditionalReferencedDocument",
                "GrossPriceProductTradePrice",
                "NetPriceProductTradePrice",
                "RequisitionerReferencedDocument",
                "ItemSellerTradeParty",
                "ItemBuyerTradeParty",
                "IncludedSpecifiedMarketplace",
                "UltimateCustomerOrderReferencedDocument",
            ],
        ),
        "NetPriceProductTradePrice" => child_order(
            child,
            &[
                "TypeCode",
                "ChargeAmount",
                "BasisQuantity",
                "MinimumQuantity",
                "MaximumQuantity",
                "ChangeReason",
                "OrderUnitConversionFactorNumeric",
                "AppliedTradeAllowanceCharge",
                "ValiditySpecifiedPeriod",
                "IncludedTradeTax",
                "DeliveryTradeLocation",
                "TradeComparisonReferencePrice",
                "AssociatedReferencedDocument",
            ],
        ),
        "SpecifiedLineTradeDelivery" => child_order(
            child,
            &[
                "RequestedQuantity",
                "ReceivedQuantity",
                "BilledQuantity",
                "ChargeFreeQuantity",
                "PackageQuantity",
                "ProductUnitQuantity",
                "PerPackageUnitQuantity",
                "NetWeightMeasure",
                "GrossWeightMeasure",
                "TheoreticalWeightMeasure",
                "DespatchedQuantity",
                "SpecifiedDeliveryAdjustment",
                "IncludedSupplyChainPackaging",
                "RelatedSupplyChainConsignment",
                "ShipToTradeParty",
                "UltimateShipToTradeParty",
                "ShipFromTradeParty",
                "ActualDespatchSupplyChainEvent",
                "ActualPickUpSupplyChainEvent",
                "RequestedDeliverySupplyChainEvent",
                "ActualDeliverySupplyChainEvent",
                "ActualReceiptSupplyChainEvent",
                "AdditionalReferencedDocument",
                "DespatchAdviceReferencedDocument",
                "ReceivingAdviceReferencedDocument",
                "DeliveryNoteReferencedDocument",
                "ConsumptionReportReferencedDocument",
                "PackingListReferencedDocument",
            ],
        ),
        "SpecifiedLineTradeSettlement" => child_order(
            child,
            &[
                "PaymentReference",
                "InvoiceIssuerReference",
                "TotalAdjustmentAmount",
                "DiscountIndicator",
                "ApplicableTradeTax",
                "BillingSpecifiedPeriod",
                "SpecifiedTradeAllowanceCharge",
                "SubtotalCalculatedTradeTax",
                "SpecifiedLogisticsServiceCharge",
                "SpecifiedTradePaymentTerms",
                "SpecifiedTradeSettlementLineMonetarySummation",
                "SpecifiedFinancialAdjustment",
                "InvoiceReferencedDocument",
                "AdditionalReferencedDocument",
                "PayableSpecifiedTradeAccountingAccount",
                "ReceivableSpecifiedTradeAccountingAccount",
                "PurchaseSpecifiedTradeAccountingAccount",
                "SalesSpecifiedTradeAccountingAccount",
                "SpecifiedTradeSettlementFinancialCard",
            ],
        ),
        "SpecifiedTradeSettlementLineMonetarySummation" => child_order(
            child,
            &[
                "LineTotalAmount",
                "ChargeTotalAmount",
                "AllowanceTotalAmount",
                "TaxBasisTotalAmount",
                "TaxTotalAmount",
                "GrandTotalAmount",
                "InformationAmount",
                "TotalAllowanceChargeAmount",
                "TotalRetailValueInformationAmount",
                "GrossLineTotalAmount",
                "NetLineTotalAmount",
                "NetIncludingTaxesLineTotalAmount",
                "ProductWeightLossInformationAmount",
            ],
        ),
        "ApplicableHeaderTradeAgreement" => child_order(
            child,
            &[
                "Reference",
                "BuyerReference",
                "SellerTradeParty",
                "BuyerTradeParty",
                "SalesAgentTradeParty",
                "BuyerRequisitionerTradeParty",
                "BuyerAssignedAccountantTradeParty",
                "SellerAssignedAccountantTradeParty",
                "BuyerTaxRepresentativeTradeParty",
                "SellerTaxRepresentativeTradeParty",
                "ProductEndUserTradeParty",
                "ApplicableTradeDeliveryTerms",
                "SellerOrderReferencedDocument",
                "BuyerOrderReferencedDocument",
                "QuotationReferencedDocument",
                "OrderResponseReferencedDocument",
                "ContractReferencedDocument",
                "DemandForecastReferencedDocument",
                "SupplyInstructionReferencedDocument",
                "PromotionalDealReferencedDocument",
                "PriceListReferencedDocument",
                "AdditionalReferencedDocument",
                "RequisitionerReferencedDocument",
                "BuyerAgentTradeParty",
                "PurchaseConditionsReferencedDocument",
                "SpecifiedProcuringProject",
                "UltimateCustomerOrderReferencedDocument",
            ],
        ),
        "ApplicableHeaderTradeDelivery" => child_order(
            child,
            &[
                "RelatedSupplyChainConsignment",
                "ShipToTradeParty",
                "UltimateShipToTradeParty",
                "ShipFromTradeParty",
                "ActualDespatchSupplyChainEvent",
                "ActualPickUpSupplyChainEvent",
                "ActualDeliverySupplyChainEvent",
                "ActualReceiptSupplyChainEvent",
                "AdditionalReferencedDocument",
                "DespatchAdviceReferencedDocument",
                "ReceivingAdviceReferencedDocument",
                "DeliveryNoteReferencedDocument",
                "ConsumptionReportReferencedDocument",
                "PreviousDeliverySupplyChainEvent",
                "PackingListReferencedDocument",
            ],
        ),
        "ActualDeliverySupplyChainEvent" => child_order(
            child,
            &[
                "ID",
                "OccurrenceDateTime",
                "TypeCode",
                "Description",
                "DescriptionBinaryObject",
                "UnitQuantity",
                "LatestOccurrenceDateTime",
                "EarliestOccurrenceDateTime",
                "OccurrenceSpecifiedPeriod",
                "OccurrenceLogisticsLocation",
            ],
        ),
        "ApplicableHeaderTradeSettlement" => child_order(
            child,
            &[
                "DuePayableAmount",
                "CreditorReferenceTypeCode",
                "CreditorReferenceType",
                "CreditorReferenceIssuerID",
                "CreditorReferenceID",
                "PaymentReference",
                "TaxCurrencyCode",
                "InvoiceCurrencyCode",
                "PaymentCurrencyCode",
                "InvoiceIssuerReference",
                "InvoiceDateTime",
                "NextInvoiceDateTime",
                "CreditReasonCode",
                "CreditReason",
                "InvoicerTradeParty",
                "InvoiceeTradeParty",
                "PayeeTradeParty",
                "PayerTradeParty",
                "TaxApplicableTradeCurrencyExchange",
                "InvoiceApplicableTradeCurrencyExchange",
                "PaymentApplicableTradeCurrencyExchange",
                "SpecifiedTradeSettlementPaymentMeans",
                "ApplicableTradeTax",
                "BillingSpecifiedPeriod",
                "SpecifiedTradeAllowanceCharge",
                "SubtotalCalculatedTradeTax",
                "SpecifiedLogisticsServiceCharge",
                "SpecifiedTradePaymentTerms",
                "SpecifiedTradeSettlementHeaderMonetarySummation",
                "SpecifiedFinancialAdjustment",
                "InvoiceReferencedDocument",
                "ProFormaInvoiceReferencedDocument",
                "LetterOfCreditReferencedDocument",
                "FactoringAgreementReferencedDocument",
                "FactoringListReferencedDocument",
                "PayableSpecifiedTradeAccountingAccount",
                "ReceivableSpecifiedTradeAccountingAccount",
                "PurchaseSpecifiedTradeAccountingAccount",
                "SalesSpecifiedTradeAccountingAccount",
                "SpecifiedTradeSettlementFinancialCard",
                "SpecifiedAdvancePayment",
                "UltimatePayeeTradeParty",
            ],
        ),
        "SpecifiedTradeSettlementPaymentMeans" => child_order(
            child,
            &[
                "PaymentChannelCode",
                "TypeCode",
                "GuaranteeMethodCode",
                "PaymentMethodCode",
                "Information",
                "ID",
                "ApplicableTradeSettlementFinancialCard",
                "PayerPartyDebtorFinancialAccount",
                "PayeePartyCreditorFinancialAccount",
                "PayerSpecifiedDebtorFinancialInstitution",
                "PayeeSpecifiedCreditorFinancialInstitution",
            ],
        ),
        "PayeePartyCreditorFinancialAccount" => {
            child_order(child, &["IBANID", "AccountName", "ProprietaryID"])
        }
        "SpecifiedTradePaymentTerms" => child_order(
            child,
            &[
                "ID",
                "FromEventCode",
                "SettlementPeriodMeasure",
                "Description",
                "DueDateDateTime",
                "TypeCode",
                "InstructionTypeCode",
                "DirectDebitMandateID",
                "PartialPaymentPercent",
                "PaymentMeansID",
                "PartialPaymentAmount",
                "ApplicableTradePaymentPenaltyTerms",
                "ApplicableTradePaymentDiscountTerms",
                "PayeeTradeParty",
            ],
        ),
        "ApplicableTradeTax" => child_order(
            child,
            &[
                "CalculatedAmount",
                "TypeCode",
                "ExemptionReason",
                "CalculatedRate",
                "CalculationSequenceNumeric",
                "BasisQuantity",
                "BasisAmount",
                "UnitBasisAmount",
                "LineTotalBasisAmount",
                "AllowanceChargeBasisAmount",
                "CategoryCode",
                "CurrencyCode",
                "Jurisdiction",
                "CustomsDutyIndicator",
                "ExemptionReasonCode",
                "TaxBasisAllowanceRate",
                "TaxPointDate",
                "Type",
                "InformationAmount",
                "CategoryName",
                "DueDateTypeCode",
                "RateApplicablePercent",
                "SpecifiedTradeAccountingAccount",
                "ServiceSupplyTradeCountry",
                "BuyerRepayableTaxSpecifiedTradeAccountingAccount",
                "SellerPayableTaxSpecifiedTradeAccountingAccount",
                "SellerRefundableTaxSpecifiedTradeAccountingAccount",
                "BuyerDeductibleTaxSpecifiedTradeAccountingAccount",
                "BuyerNonDeductibleTaxSpecifiedTradeAccountingAccount",
                "PlaceApplicableTradeLocation",
            ],
        ),
        "SpecifiedTradeSettlementHeaderMonetarySummation" => child_order(
            child,
            &[
                "LineTotalAmount",
                "ChargeTotalAmount",
                "AllowanceTotalAmount",
                "TaxBasisTotalAmount",
                "TaxTotalAmount",
                "RoundingAmount",
                "GrandTotalAmount",
                "InformationAmount",
                "TotalPrepaidAmount",
                "TotalDiscountAmount",
                "TotalAllowanceChargeAmount",
                "DuePayableAmount",
                "RetailValueExcludingTaxInformationAmount",
                "TotalDepositFeeInformationAmount",
                "ProductValueExcludingTobaccoTaxInformationAmount",
                "TotalRetailValueInformationAmount",
                "GrossLineTotalAmount",
                "NetLineTotalAmount",
                "NetIncludingTaxesLineTotalAmount",
            ],
        ),
        "SellerTradeParty" | "BuyerTradeParty" | "PayeeTradeParty" => child_order(
            child,
            &[
                "ID",
                "GlobalID",
                "Name",
                "RoleCode",
                "Description",
                "SpecifiedLegalOrganization",
                "DefinedTradeContact",
                "PostalTradeAddress",
                "URIUniversalCommunication",
                "SpecifiedTaxRegistration",
                "EndPointURIUniversalCommunication",
                "LogoAssociatedSpecifiedBinaryFile",
            ],
        ),
        "SpecifiedLegalOrganization" => child_order(
            child,
            &[
                "LegalClassificationCode",
                "Name",
                "ID",
                "TradingBusinessName",
                "PostalTradeAddress",
                "AuthorizedLegalRegistration",
            ],
        ),
        "SpecifiedTaxRegistration" => child_order(child, &["ID", "AssociatedRegisteredTax"]),
        "PostalTradeAddress" => child_order(
            child,
            &[
                "ID",
                "PostcodeCode",
                "PostOfficeBox",
                "BuildingName",
                "LineOne",
                "LineTwo",
                "LineThree",
                "LineFour",
                "LineFive",
                "StreetName",
                "CityName",
                "CitySubDivisionName",
                "CountryID",
                "CountryName",
                "CountrySubDivisionID",
                "CountrySubDivisionName",
                "AttentionOf",
                "CareOf",
                "BuildingNumber",
                "DepartmentName",
                "AdditionalStreetName",
            ],
        ),
        "DefinedTradeContact" => child_order(
            child,
            &[
                "ID",
                "PersonName",
                "DepartmentName",
                "TypeCode",
                "JobTitle",
                "Responsibility",
                "PersonID",
                "TelephoneUniversalCommunication",
                "DirectTelephoneUniversalCommunication",
                "MobileTelephoneUniversalCommunication",
                "FaxUniversalCommunication",
                "EmailURIUniversalCommunication",
                "TelexUniversalCommunication",
                "VOIPUniversalCommunication",
                "InstantMessagingUniversalCommunication",
                "SpecifiedNote",
                "SpecifiedContactPerson",
            ],
        ),
        "TelephoneUniversalCommunication" | "EmailURIUniversalCommunication" => {
            child_order(child, &["URIID", "ChannelCode", "CompleteNumber"])
        }
        _ => known_cii_children(parent)
            .and_then(|children| children.iter().position(|value| *value == child))
            .and_then(|position| u16::try_from(position + 1).ok()),
    }
}

fn child_order(child: &str, children: &[&str]) -> Option<u16> {
    children
        .iter()
        .position(|value| *value == child)
        .and_then(|position| u16::try_from(position + 1).ok())
}

fn previous_known_child_order(parent: &str, before_child: &str) -> Option<u16> {
    let before = cii_child_order(parent, before_child)?;
    known_cii_children(parent).and_then(|children| {
        children
            .iter()
            .filter_map(|child| cii_child_order(parent, child))
            .filter(|order| *order < before)
            .max()
    })
}

fn container_name(container: &str) -> &str {
    container
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(container)
}

fn validate_preserved_xml_entry(
    document: &CommercialDocument,
    preserved: &CiiPreservedXml,
) -> Result<(), CiiError> {
    validate_preserved_xml_fragment(preserved)?;
    if !is_replayable_container_path(&preserved.container) {
        return Err(preserved_xml_error(
            preserved,
            "container path is not replayable by the CII serializer",
        ));
    }
    let line_scoped = is_line_scoped_container_path(&preserved.container);
    match (line_scoped, preserved.line_id.as_deref()) {
        (true, None) => {
            return Err(preserved_xml_error(
                preserved,
                "line-scoped preserved XML requires line_id",
            ));
        }
        (false, Some(_)) => {
            return Err(preserved_xml_error(
                preserved,
                "line_id is only valid for line-scoped preserved XML",
            ));
        }
        (true, Some(line_id)) if !document.lines.iter().any(|line| line.id == line_id) => {
            return Err(preserved_xml_error(
                preserved,
                "line_id does not match any document line",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn is_line_scoped_container_path(container: &str) -> bool {
    container.starts_with(
        "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem",
    )
}

fn is_replayable_container_path(container: &str) -> bool {
    matches!(
        container,
        "CrossIndustryInvoice"
            | "CrossIndustryInvoice/ExchangedDocumentContext"
            | "CrossIndustryInvoice/ExchangedDocumentContext/BusinessProcessSpecifiedDocumentContextParameter"
            | "CrossIndustryInvoice/ExchangedDocumentContext/ApplicationSpecifiedDocumentContextParameter"
            | "CrossIndustryInvoice/ExchangedDocumentContext/GuidelineSpecifiedDocumentContextParameter"
            | "CrossIndustryInvoice/ExchangedDocument"
            | "CrossIndustryInvoice/ExchangedDocument/IncludedNote"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/AssociatedDocumentLineDocument"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedTradeProduct"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeAgreement"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeAgreement/NetPriceProductTradePrice"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeDelivery"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeSettlement"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeSettlement/ApplicableTradeTax"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedLineTradeSettlement/SpecifiedTradeSettlementLineMonetarySummation"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/SpecifiedLegalOrganization"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact/TelephoneUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact/EmailURIUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/PostalTradeAddress"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/SpecifiedTaxRegistration"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/SpecifiedLegalOrganization"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/DefinedTradeContact"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/DefinedTradeContact/TelephoneUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/DefinedTradeContact/EmailURIUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/PostalTradeAddress"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/BuyerTradeParty/SpecifiedTaxRegistration"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeDelivery"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeDelivery/ActualDeliverySupplyChainEvent"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/SpecifiedLegalOrganization"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/DefinedTradeContact"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/DefinedTradeContact/TelephoneUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/DefinedTradeContact/EmailURIUniversalCommunication"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/PostalTradeAddress"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/PayeeTradeParty/SpecifiedTaxRegistration"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradeSettlementPaymentMeans"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradeSettlementPaymentMeans/PayeePartyCreditorFinancialAccount"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/ApplicableTradeTax"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradePaymentTerms"
            | "CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeSettlement/SpecifiedTradeSettlementHeaderMonetarySummation"
    )
}

#[allow(clippy::too_many_lines)]
fn validate_preserved_xml_fragment(preserved: &CiiPreservedXml) -> Result<(), CiiError> {
    let container_name = container_name(&preserved.container);
    if known_cii_children(container_name).is_none() {
        return Err(preserved_xml_error(
            preserved,
            "container is not a replayable CII container",
        ));
    }
    let Some(order) = cii_child_order(container_name, &preserved.element) else {
        return Err(preserved_xml_error(
            preserved,
            "root element is not ordered for this CII container",
        ));
    };
    if order == 0 {
        return Err(preserved_xml_error(preserved, "invalid schema order"));
    }

    let mut reader = Reader::from_str(&preserved.xml);
    reader.config_mut().trim_text(false);
    let mut root_count = 0_usize;
    let mut depth = 0_usize;
    let root_parent_stack = preserved
        .container
        .split('/')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut element_stack = Vec::<String>::new();
    let mut namespace_stack = Vec::<Vec<XmlNamespaceBinding>>::new();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let namespace_declarations =
                    namespace_declarations_for_start(&reader, &start, XmlVersion::default())?;
                let namespace = resolve_element_namespace(
                    start.name().as_ref(),
                    &namespace_stack,
                    Some(&namespace_declarations),
                )?;
                if depth == 0 {
                    root_count += 1;
                    if root_count > 1 {
                        return Err(preserved_xml_error(
                            preserved,
                            "fragment must contain exactly one root element",
                        ));
                    }
                    if name != preserved.element {
                        return Err(preserved_xml_error(
                            preserved,
                            "root element does not match preserved element",
                        ));
                    }
                }
                let parent_stack = if depth == 0 {
                    root_parent_stack.as_slice()
                } else {
                    element_stack.as_slice()
                };
                validate_cii_namespace(&namespace, parent_stack, &name)?;
                depth += 1;
                element_stack.push(name);
                namespace_stack.push(namespace_declarations);
            }
            Event::Empty(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let namespace_declarations =
                    namespace_declarations_for_start(&reader, &start, XmlVersion::default())?;
                let namespace = resolve_element_namespace(
                    start.name().as_ref(),
                    &namespace_stack,
                    Some(&namespace_declarations),
                )?;
                if depth == 0 {
                    root_count += 1;
                    if root_count > 1 {
                        return Err(preserved_xml_error(
                            preserved,
                            "fragment must contain exactly one root element",
                        ));
                    }
                    if name != preserved.element {
                        return Err(preserved_xml_error(
                            preserved,
                            "root element does not match preserved element",
                        ));
                    }
                }
                let parent_stack = if depth == 0 {
                    root_parent_stack.as_slice()
                } else {
                    element_stack.as_slice()
                };
                validate_cii_namespace(&namespace, parent_stack, &name)?;
            }
            Event::End(end) => {
                if depth == 0 {
                    return Err(preserved_xml_error(preserved, "unexpected closing element"));
                }
                let name = decode_local_name(end.name().as_ref())?;
                let Some((opened, parent_stack)) = element_stack.split_last() else {
                    return Err(preserved_xml_error(preserved, "unexpected closing element"));
                };
                if opened != &name {
                    return Err(preserved_xml_error(
                        preserved,
                        "start and end elements do not match",
                    ));
                }
                let namespace =
                    resolve_element_namespace(end.name().as_ref(), &namespace_stack, None)?;
                validate_cii_namespace(&namespace, parent_stack, &name)?;
                depth -= 1;
                element_stack.pop();
                namespace_stack.pop();
            }
            Event::Text(text) => {
                if depth == 0 && !text.xml_content(XmlVersion::default())?.trim().is_empty() {
                    return Err(preserved_xml_error(
                        preserved,
                        "text outside the root element is not allowed",
                    ));
                }
            }
            Event::CData(cdata) => {
                if depth == 0 && !cdata.xml_content(XmlVersion::default())?.trim().is_empty() {
                    return Err(preserved_xml_error(
                        preserved,
                        "CDATA outside the root element is not allowed",
                    ));
                }
            }
            Event::GeneralRef(reference) => {
                if depth == 0
                    && !reference
                        .xml_content(XmlVersion::default())?
                        .trim()
                        .is_empty()
                {
                    return Err(preserved_xml_error(
                        preserved,
                        "entity reference outside the root element is not allowed",
                    ));
                }
            }
            Event::DocType(_) => {
                return Err(preserved_xml_error(preserved, "DOCTYPE is not allowed"));
            }
            Event::Decl(_) => {
                return Err(preserved_xml_error(
                    preserved,
                    "XML declaration is not allowed in a fragment",
                ));
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => {
                if root_count != 1 {
                    return Err(preserved_xml_error(
                        preserved,
                        "fragment must contain exactly one root element",
                    ));
                }
                if depth != 0 {
                    return Err(preserved_xml_error(
                        preserved,
                        "fragment ended before closing the root element",
                    ));
                }
                return Ok(());
            }
        }
    }
}

fn preserved_xml_error(preserved: &CiiPreservedXml, message: &str) -> CiiError {
    CiiError::InvalidPreservedXml {
        container: preserved.container.clone(),
        element: preserved.element.clone(),
        message: message.to_owned(),
    }
}

fn decode_local_name(raw: &[u8]) -> Result<String, CiiError> {
    Ok(local_xml_name(decode_name(raw)?).to_owned())
}

fn decode_name(raw: &[u8]) -> Result<&str, CiiError> {
    std::str::from_utf8(raw)
        .map_err(|_| CiiError::InvalidName(String::from_utf8_lossy(raw).into_owned()))
}

fn local_xml_name(name: &str) -> &str {
    split_xml_name(name).1
}

fn split_xml_name(name: &str) -> (&str, &str) {
    name.split_once(':')
        .map_or(("", name), |(prefix, local_name)| (prefix, local_name))
}

fn append_text(text_stack: &mut [String], text: &str) -> Result<(), CiiError> {
    let Some(current) = text_stack.last_mut() else {
        return if text.trim().is_empty() {
            Ok(())
        } else {
            Err(CiiError::UnsupportedRoot("#text".to_owned()))
        };
    };
    current.push_str(text);
    Ok(())
}

fn resolve_xml_reference(reference: &str, xml_version: XmlVersion) -> Result<String, CiiError> {
    if let Some(hex) = reference
        .strip_prefix("#x")
        .or_else(|| reference.strip_prefix("#X"))
    {
        return char_from_reference(reference, u32::from_str_radix(hex, 16), xml_version);
    }
    if let Some(decimal) = reference.strip_prefix('#') {
        return char_from_reference(reference, decimal.parse::<u32>(), xml_version);
    }

    match reference {
        "amp" => Ok("&".to_owned()),
        "lt" => Ok("<".to_owned()),
        "gt" => Ok(">".to_owned()),
        "apos" => Ok("'".to_owned()),
        "quot" => Ok("\"".to_owned()),
        other => Err(CiiError::UnsupportedRoot(format!("entity:{other}"))),
    }
}

fn char_from_reference(
    reference: &str,
    parsed: Result<u32, impl std::error::Error>,
    xml_version: XmlVersion,
) -> Result<String, CiiError> {
    parsed
        .ok()
        .filter(|codepoint| is_valid_xml_char(*codepoint, xml_version))
        .and_then(char::from_u32)
        .map(|character| character.to_string())
        .ok_or_else(|| CiiError::UnsupportedRoot(format!("entity:{reference}")))
}

fn is_valid_xml_char(codepoint: u32, xml_version: XmlVersion) -> bool {
    match xml_version {
        XmlVersion::Explicit1_1 => matches!(
            codepoint,
            0x1..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF
        ),
        XmlVersion::Implicit1_0 | XmlVersion::Explicit1_0 => matches!(
            codepoint,
            0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF
        ),
    }
}

fn path_ends(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack
            .iter()
            .rev()
            .take(suffix.len())
            .zip(suffix.iter().rev())
            .all(|(left, right)| left == right)
}

fn in_any(stack: &[String], names: &[&str]) -> bool {
    stack
        .iter()
        .any(|item| names.iter().any(|name| item == name))
}

fn party_role(stack: &[String]) -> Option<PartyRole> {
    if in_any(stack, &["SellerTradeParty"]) {
        Some(PartyRole::Supplier)
    } else if in_any(stack, &["BuyerTradeParty"]) {
        Some(PartyRole::Customer)
    } else if in_any(stack, &["PayeeTradeParty"]) {
        Some(PartyRole::Payee)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::{json, Value};

    use super::{
        crate_name, from_xml, mapping, to_xml, CiiError, CII_QDT_NAMESPACE_URI,
        CII_RAM_NAMESPACE_URI, CII_RSM_NAMESPACE_URI,
    };
    use invoicekit_canonical::canonicalize_xml;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType,
        Iso4217Code, ItemClassification, JurisdictionExtension, LocalizedString, MonetaryTotal,
        Party, PartyTaxId, PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress,
        SchemaVersion, TaxCategorySummary,
    };
    use rust_decimal::Decimal;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-cii");
    }

    /// `write_monetary_total` must not panic when the attacker-controlled
    /// `tax_inclusive_amount - tax_exclusive_amount` difference overflows
    /// `Decimal`. Pre-fix the unguarded subtraction panicked the serializer
    /// (a denial of service through the public `to_xml` entry point).
    #[test]
    fn monetary_total_does_not_panic_on_decimal_difference_overflow() {
        let mut document = fixture(DocumentType::Invoice, 0);
        // Decimal::MIN minus a positive exclusive amount overflows the Sub impl.
        document.monetary_total.tax_inclusive_amount = DecimalValue::new(Decimal::MIN);
        document.monetary_total.tax_exclusive_amount =
            DecimalValue::new(Decimal::ONE_HUNDRED);

        let xml = to_xml(&document).expect("serializer must not panic on overflowing totals");
        // Element is still emitted and well-formed (empty value on overflow).
        assert!(xml.contains("<ram:TaxTotalAmount"));
        assert!(xml.contains("</ram:TaxTotalAmount>"));
    }

    /// Normal (non-overflowing) totals must serialize the tax difference exactly
    /// as the prior subtraction did: this guards the behavior-preserving fix.
    #[test]
    fn monetary_total_emits_exact_tax_difference_for_normal_amounts() {
        let mut document = fixture(DocumentType::Invoice, 0);
        document.monetary_total.tax_exclusive_amount = DecimalValue::new(Decimal::new(10000, 2));
        document.monetary_total.tax_inclusive_amount = DecimalValue::new(Decimal::new(11900, 2));

        let xml = to_xml(&document).expect("serializer must succeed on normal amounts");
        // 119.00 - 100.00 = 19.00
        assert!(xml.contains(">19.00</ram:TaxTotalAmount>"));
    }

    fn parse_document(xml: &str) -> CommercialDocument {
        let (document, ledger) = from_xml(xml).unwrap();
        assert!(
            ledger.lost.is_empty(),
            "successful CII fixture parse should not report lost IR fields"
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
    fn metadata_uses_application_context_without_overloading_cii_fields() {
        let document = fixture(DocumentType::Invoice, 21);
        let xml = to_xml(&document).unwrap();

        assert!(xml.contains("ApplicationSpecifiedDocumentContextParameter"));
        assert!(xml.contains(mapping::INVOICEKIT_CII_METADATA_EXTENSION_URN));
        assert!(!xml.contains("BusinessProcessSpecifiedDocumentContextParameter"));
        assert!(!xml.contains("BuyerReference"));
        assert_eq!(parse_document(&xml), document);
    }

    #[test]
    fn parser_preserves_standard_cii_fields_as_document_extension() {
        let document = fixture(DocumentType::Invoice, 22);
        let xml = to_xml(&document).unwrap();
        let application_context = xml
            .find("<ram:ApplicationSpecifiedDocumentContextParameter")
            .unwrap();
        let with_business_process = format!(
            "{}{}{}",
            &xml[..application_context],
            &format!(
                r#"<ram:BusinessProcessSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>PROCESS-42</ram:ID></ram:BusinessProcessSpecifiedDocumentContextParameter><ram:BusinessProcessSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>PROCESS-43</ram:ID></ram:BusinessProcessSpecifiedDocumentContextParameter>"#
            ),
            &xml[application_context..],
        );
        assert!(with_business_process.contains("PROCESS-42"));
        let with_buyer_reference = with_business_process.replace(
            "<ram:SellerTradeParty>",
            &format!(
                r#"<ram:BuyerReference xmlns:ram="{CII_RAM_NAMESPACE_URI}">BUYER-PO-7</ram:BuyerReference><ram:SellerTradeParty>"#
            ),
        );

        let parsed = parse_document(&with_buyer_reference);
        assert_eq!(parsed.meta, document.meta);
        let extension = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
            .unwrap();
        assert_eq!(
            extension
                .payload
                .get("buyer_reference")
                .and_then(|value| value.as_str()),
            Some("BUYER-PO-7")
        );
        assert_eq!(
            extension
                .payload
                .get("business_process_context_ids")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice),
            Some([json!("PROCESS-42"), json!("PROCESS-43")].as_slice())
        );
    }

    #[test]
    fn serializer_emits_preserved_cii_document_fields() {
        let mut document = fixture(DocumentType::Invoice, 23);
        document.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "buyer_reference": "BUYER-PO-8",
                    "business_process_context_ids": ["PROCESS-43", "PROCESS-44"]
                }),
            )
            .unwrap(),
        );

        let xml = to_xml(&document).unwrap();
        assert!(xml.contains("BusinessProcessSpecifiedDocumentContextParameter"));
        assert!(xml.contains("<ram:ID>PROCESS-43</ram:ID>"));
        assert!(xml.contains("<ram:ID>PROCESS-44</ram:ID>"));
        assert!(xml.contains("<ram:BuyerReference>BUYER-PO-8</ram:BuyerReference>"));
        assert_eq!(parse_document(&xml), document);
    }

    #[test]
    fn serializer_emits_profile_guideline_context() {
        let mut document = fixture(DocumentType::Invoice, 24);
        document.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_PROFILE_CONTEXT_EXTENSION_URN,
                json!({
                    "guideline_context_ids": [
                        "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic"
                    ]
                }),
            )
            .unwrap(),
        );

        let xml = to_xml(&document).unwrap();
        assert!(xml.contains("GuidelineSpecifiedDocumentContextParameter"));
        assert!(xml.contains("urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic"));
        assert_eq!(parse_document(&xml), document);
    }

    #[test]
    fn parser_preserves_unmapped_cii_fragments_as_document_extension() {
        let document = fixture(DocumentType::Invoice, 25);
        let xml = to_xml(&document).unwrap();
        let document_name = format!(
            r#"<ram:Name xmlns:ram="{CII_RAM_NAMESPACE_URI}">Commercial invoice</ram:Name>"#
        );
        let tax_representative = format!(
            r#"<ram:SellerTaxRepresentativeTradeParty xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:Name>Tax Representative</ram:Name></ram:SellerTaxRepresentativeTradeParty>"#
        );
        let with_document_name = insert_before_tag_after(
            &xml,
            "<rsm:ExchangedDocument>",
            "<ram:TypeCode",
            &document_name,
        );
        let with_tax_representative = with_document_name.replacen(
            "</ram:ApplicableHeaderTradeAgreement>",
            &format!("{tax_representative}</ram:ApplicableHeaderTradeAgreement>"),
            1,
        );

        let parsed = parse_document(&with_tax_representative);
        let preserved = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
            .and_then(|extension| extension.payload.get("preserved_xml"))
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(preserved.iter().any(|item| {
            item.get("container").and_then(|value| value.as_str())
                == Some("CrossIndustryInvoice/ExchangedDocument")
                && item.get("element").and_then(|value| value.as_str()) == Some("Name")
                && item
                    .get("xml")
                    .and_then(|value| value.as_str())
                    .is_some_and(|xml| xml.contains("Commercial invoice"))
        }));
        assert!(preserved.iter().any(|item| {
            item.get("element").and_then(|value| value.as_str())
                == Some("SellerTaxRepresentativeTradeParty")
                && item
                    .get("xml")
                    .and_then(|value| value.as_str())
                    .is_some_and(|xml| xml.contains("Tax Representative"))
        }));
        assert_eq!(parse_document(&to_xml(&parsed).unwrap()), parsed);
    }

    #[test]
    fn parser_accepts_alternate_cii_prefixes_with_stable_preserved_xml() {
        let document = fixture(DocumentType::Invoice, 26);
        let xml = to_xml(&document).unwrap();
        let document_name = format!(
            r#"<alt:Name xmlns:alt="{CII_RAM_NAMESPACE_URI}" beta="2" alpha="1">Alt Prefix</alt:Name>"#
        );
        let with_document_name = insert_before_tag_after(
            &xml,
            "<rsm:ExchangedDocument>",
            "<ram:TypeCode",
            &document_name,
        );

        let parsed = parse_document(&with_document_name);
        let preserved_xml = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
            .and_then(|extension| extension.payload.get("preserved_xml"))
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("xml"))
            .and_then(|value| value.as_str())
            .unwrap();
        assert!(preserved_xml.contains(r#"alpha="1" beta="2""#));
        assert!(preserved_xml.contains(CII_RAM_NAMESPACE_URI));
        assert_eq!(parse_document(&to_xml(&parsed).unwrap()), parsed);
    }

    #[test]
    fn parser_rejects_wrong_cii_namespace_for_known_elements() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 27)).unwrap();
        let document_start = xml.find("<rsm:ExchangedDocument>").unwrap();
        let type_start = document_start
            + xml[document_start..]
                .find("<ram:TypeCode")
                .expect("document TypeCode start");
        let open_end = type_start + xml[type_start..].find('>').unwrap() + 1;
        let close_start = open_end + xml[open_end..].find("</ram:TypeCode>").unwrap();
        let close_end = close_start + "</ram:TypeCode>".len();
        let xml = format!(
            r#"{}<bad:TypeCode xmlns:bad="urn:wrong">{}</bad:TypeCode>{}"#,
            &xml[..type_start],
            &xml[open_end..close_start],
            &xml[close_end..],
        );

        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidNamespace {
                element,
                expected,
                ..
            } if element == "TypeCode" && expected == CII_RAM_NAMESPACE_URI
        ));
    }

    #[test]
    fn parser_preserves_repeatable_party_identifiers_as_cii_xml() {
        let document = fixture(DocumentType::Invoice, 28);
        let xml = to_xml(&document).unwrap();
        let party_identifiers = format!(
            r#"<ram:ID xmlns:ram="{CII_RAM_NAMESPACE_URI}" schemeID="GLN">4000001123452</ram:ID><ram:GlobalID xmlns:ram="{CII_RAM_NAMESPACE_URI}" schemeID="0088">4000001123452</ram:GlobalID>"#
        );
        let with_party_identifiers = xml.replacen(
            "<ram:Name>Supplier GmbH</ram:Name>",
            &format!("{party_identifiers}<ram:Name>Supplier GmbH</ram:Name>"),
            1,
        );

        let parsed = parse_document(&with_party_identifiers);
        assert_eq!(parsed.supplier.id, document.supplier.id);
        let serialized = to_xml(&parsed).unwrap();
        let id_pos = serialized.find("4000001123452").unwrap();
        let name_pos = serialized
            .find("<ram:Name>Supplier GmbH</ram:Name>")
            .unwrap();
        assert!(id_pos < name_pos);
        assert_eq!(parse_document(&serialized), parsed);
    }

    #[test]
    fn parser_assigns_line_id_to_pre_lineid_preserved_cii_fragments() {
        let document = fixture(DocumentType::Invoice, 38);
        let xml = to_xml(&document).unwrap();
        let description_code = format!(
            r#"<ram:DescriptionCode xmlns:ram="{CII_RAM_NAMESPACE_URI}">AAA</ram:DescriptionCode>"#
        );
        let with_description_code = xml.replacen(
            "<ram:AssociatedDocumentLineDocument>",
            &format!("{description_code}<ram:AssociatedDocumentLineDocument>"),
            1,
        );

        let parsed = parse_document(&with_description_code);
        let preserved = preserved_xml_items(&parsed);
        assert!(preserved.iter().any(|item| {
            item.get("container").and_then(|value| value.as_str())
                == Some("CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem")
                && item.get("element").and_then(|value| value.as_str())
                    == Some("DescriptionCode")
                && item.get("line_id").and_then(|value| value.as_str()) == Some("1")
        }));

        let serialized = to_xml(&parsed).unwrap();
        let description_pos = serialized.find("<ram:DescriptionCode").unwrap();
        let line_id_container_pos = serialized
            .find("<ram:AssociatedDocumentLineDocument>")
            .unwrap();
        assert!(description_pos < line_id_container_pos);
        assert_eq!(parse_document(&serialized), parsed);
    }

    #[test]
    fn parser_replays_preserved_only_defined_trade_contact() {
        let mut document = fixture(DocumentType::Invoice, 39);
        document.supplier.contact = None;
        let xml = to_xml(&document).unwrap();
        let contact = format!(
            r#"<ram:DefinedTradeContact xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>CONTACT-ONLY</ram:ID></ram:DefinedTradeContact>"#
        );
        let with_contact = xml.replacen(
            "<ram:PostalTradeAddress>",
            &format!("{contact}<ram:PostalTradeAddress>"),
            1,
        );

        let parsed = parse_document(&with_contact);
        assert_eq!(parsed.supplier.contact, None);
        let preserved = preserved_xml_items(&parsed);
        assert!(preserved.iter().any(|item| {
            item.get("container").and_then(|value| value.as_str())
                == Some("CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact")
                && item.get("element").and_then(|value| value.as_str()) == Some("ID")
                && item
                    .get("xml")
                    .and_then(|value| value.as_str())
                    .is_some_and(|xml| xml.contains("CONTACT-ONLY"))
        }));

        let serialized = to_xml(&parsed).unwrap();
        assert!(serialized.contains("<ram:DefinedTradeContact><ram:ID"));
        assert!(serialized.contains("CONTACT-ONLY"));
        assert_eq!(parse_document(&serialized), parsed);
    }

    #[test]
    fn parser_replays_preserved_only_nested_contact_communication() {
        let mut document = fixture(DocumentType::Invoice, 40);
        document.supplier.contact = None;
        let xml = to_xml(&document).unwrap();
        let contact = format!(
            r#"<ram:DefinedTradeContact xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:TelephoneUniversalCommunication><ram:URIID>tel:+4930000000</ram:URIID><ram:ChannelCode>TELEPHONE</ram:ChannelCode></ram:TelephoneUniversalCommunication></ram:DefinedTradeContact>"#
        );
        let with_contact = xml.replacen(
            "<ram:PostalTradeAddress>",
            &format!("{contact}<ram:PostalTradeAddress>"),
            1,
        );

        let parsed = parse_document(&with_contact);
        assert_eq!(parsed.supplier.contact, None);
        let preserved = preserved_xml_items(&parsed);
        assert!(preserved.iter().any(|item| {
            item.get("container").and_then(|value| value.as_str())
                == Some("CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact/TelephoneUniversalCommunication")
                && item.get("element").and_then(|value| value.as_str()) == Some("ChannelCode")
                && item
                    .get("xml")
                    .and_then(|value| value.as_str())
                    .is_some_and(|xml| xml.contains("TELEPHONE"))
        }));
        assert!(preserved.iter().any(|item| {
            item.get("container").and_then(|value| value.as_str())
                == Some("CrossIndustryInvoice/SupplyChainTradeTransaction/ApplicableHeaderTradeAgreement/SellerTradeParty/DefinedTradeContact/TelephoneUniversalCommunication")
                && item.get("element").and_then(|value| value.as_str()) == Some("URIID")
                && item
                    .get("xml")
                    .and_then(|value| value.as_str())
                    .is_some_and(|xml| xml.contains("tel:+4930000000"))
        }));

        let serialized = to_xml(&parsed).unwrap();
        assert!(
            serialized.contains("<ram:DefinedTradeContact><ram:TelephoneUniversalCommunication>")
        );
        assert!(serialized.contains("<ram:URIID"));
        assert!(serialized.contains("<ram:ChannelCode"));
        assert!(serialized.contains("TELEPHONE"));
        assert_eq!(parse_document(&serialized), parsed);
    }

    #[test]
    fn parser_preserves_root_level_cii_extension_elements() {
        let document = fixture(DocumentType::Invoice, 33);
        let xml = to_xml(&document).unwrap();
        let valuation = format!(
            r#"<rsm:ValuationBreakdownStatement xmlns:rsm="{CII_RSM_NAMESPACE_URI}" xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>VAL-1</ram:ID></rsm:ValuationBreakdownStatement>"#
        );
        let xml = xml.replacen(
            "</rsm:CrossIndustryInvoice>",
            &format!("{valuation}</rsm:CrossIndustryInvoice>"),
            1,
        );

        let parsed = parse_document(&xml);
        let serialized = to_xml(&parsed).unwrap();
        let transaction_pos = serialized
            .find("</rsm:SupplyChainTradeTransaction>")
            .unwrap();
        let valuation_pos = serialized.find("<rsm:ValuationBreakdownStatement").unwrap();
        assert!(transaction_pos < valuation_pos);
        assert_eq!(parse_document(&serialized), parsed);
    }

    #[test]
    fn parser_round_trips_profile_context_payloads() {
        let document = fixture(DocumentType::Invoice, 29);
        let xml = to_xml(&document).unwrap();
        let profile_context = format!(
            r#"<ram:SpecifiedTransactionID xmlns:ram="{CII_RAM_NAMESPACE_URI}">TX-42</ram:SpecifiedTransactionID><ram:TestIndicator xmlns:ram="{CII_RAM_NAMESPACE_URI}">true</ram:TestIndicator><ram:ApplicationSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>urn:example:profile-app</ram:ID><ram:Value>profile-value</ram:Value></ram:ApplicationSpecifiedDocumentContextParameter>"#
        );
        let with_profile_context = xml.replacen(
            "<ram:ApplicationSpecifiedDocumentContextParameter",
            &format!("{profile_context}<ram:ApplicationSpecifiedDocumentContextParameter"),
            1,
        );

        let parsed = parse_document(&with_profile_context);
        let extension = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_PROFILE_CONTEXT_EXTENSION_URN)
            .unwrap();
        assert_eq!(
            extension
                .payload
                .get("transaction_ids")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice),
            Some([json!("TX-42")].as_slice())
        );
        assert_eq!(
            extension
                .payload
                .get("test_indicators")
                .and_then(|value| value.as_array())
                .map(Vec::as_slice),
            Some([json!("true")].as_slice())
        );
        assert_eq!(parse_document(&to_xml(&parsed).unwrap()), parsed);
    }

    #[test]
    fn serializer_rejects_wrong_namespace_preserved_cii_fragment() {
        let mut document = fixture(DocumentType::Invoice, 30);
        document.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [{
                        "container": "CrossIndustryInvoice/ExchangedDocument",
                        "element": "Name",
                        "xml": r#"<evil:Name xmlns:evil="urn:wrong">Bad</evil:Name>"#
                    }]
                }),
            )
            .unwrap(),
        );

        let err = to_xml(&document).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidNamespace { element, .. } if element == "Name"
        ));
    }

    #[test]
    fn serializer_rejects_unmatched_preserved_cii_payloads() {
        let mut typoed_container = fixture(DocumentType::Invoice, 34);
        typoed_container.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [{
                        "container": "CrossIndustryInvoice/Typo/ExchangedDocument",
                        "element": "Name",
                        "xml": format!(r#"<ram:Name xmlns:ram="{CII_RAM_NAMESPACE_URI}">Bad</ram:Name>"#)
                    }]
                }),
            )
            .unwrap(),
        );
        let err = to_xml(&typoed_container).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidPreservedXml { message, .. }
                if message.contains("container path")
        ));

        let mut missing_line = fixture(DocumentType::Invoice, 35);
        missing_line.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [{
                        "container": "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem",
                        "element": "DescriptionCode",
                        "line_id": "missing-line",
                        "xml": format!(r#"<ram:DescriptionCode xmlns:ram="{CII_RAM_NAMESPACE_URI}">A</ram:DescriptionCode>"#)
                    }]
                }),
            )
            .unwrap(),
        );
        let err = to_xml(&missing_line).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidPreservedXml { message, .. }
                if message.contains("line_id")
        ));

        let mut stray_line_id = fixture(DocumentType::Invoice, 37);
        let line_id = stray_line_id.lines.first().unwrap().id.clone();
        stray_line_id.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [{
                        "container": "CrossIndustryInvoice/ExchangedDocument",
                        "element": "Name",
                        "line_id": line_id,
                        "xml": format!(r#"<ram:Name xmlns:ram="{CII_RAM_NAMESPACE_URI}">Bad</ram:Name>"#)
                    }]
                }),
            )
            .unwrap(),
        );
        let err = to_xml(&stray_line_id).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidPreservedXml { message, .. }
                if message.contains("line_id")
        ));
    }

    #[test]
    fn parser_rejects_wrong_namespace_inside_preserved_cii_fragment() {
        let document = fixture(DocumentType::Invoice, 32);
        let xml = to_xml(&document).unwrap();
        let tax_representative = format!(
            r#"<ram:SellerTaxRepresentativeTradeParty xmlns:ram="{CII_RAM_NAMESPACE_URI}"><bad:Name xmlns:bad="urn:wrong">Tax Representative</bad:Name></ram:SellerTaxRepresentativeTradeParty>"#
        );
        let xml = xml.replacen(
            "</ram:ApplicableHeaderTradeAgreement>",
            &format!("{tax_representative}</ram:ApplicableHeaderTradeAgreement>"),
            1,
        );

        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(
            err,
            CiiError::InvalidNamespace { element, .. } if element == "Name"
        ));
    }

    #[test]
    fn serializer_replays_preserved_cii_fragments_in_schema_order() {
        let document = fixture(DocumentType::Invoice, 31);
        let xml = to_xml(&document).unwrap();
        let document_name =
            format!(r#"<ram:Name xmlns:ram="{CII_RAM_NAMESPACE_URI}">Ordered Name</ram:Name>"#);
        let context_fragments = format!(
            r#"<ram:BIMSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>BIM-1</ram:ID></ram:BIMSpecifiedDocumentContextParameter><ram:ScenarioSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>SCENARIO-1</ram:ID></ram:ScenarioSpecifiedDocumentContextParameter>"#
        );
        let with_document_name = insert_before_tag_after(
            &xml,
            "<rsm:ExchangedDocument>",
            "<ram:TypeCode",
            &document_name,
        );
        let with_context_fragments = with_document_name.replacen(
            "<ram:ApplicationSpecifiedDocumentContextParameter",
            &format!("{context_fragments}<ram:ApplicationSpecifiedDocumentContextParameter"),
            1,
        );

        let parsed = parse_document(&with_context_fragments);
        let serialized = to_xml(&parsed).unwrap();
        let exchanged_document_start = serialized.find("<rsm:ExchangedDocument>").unwrap();
        let id_pos = exchanged_document_start
            + serialized[exchanged_document_start..]
                .find("CII-0031")
                .unwrap();
        let name_pos = serialized.find("Ordered Name").unwrap();
        let type_pos = exchanged_document_start
            + serialized[exchanged_document_start..]
                .find("<ram:TypeCode")
                .unwrap();
        assert!(id_pos < name_pos);
        assert!(name_pos < type_pos);

        let bim_pos = serialized
            .find("<ram:BIMSpecifiedDocumentContextParameter")
            .unwrap();
        let scenario_pos = serialized
            .find("<ram:ScenarioSpecifiedDocumentContextParameter")
            .unwrap();
        let application_pos = serialized
            .find("<ram:ApplicationSpecifiedDocumentContextParameter")
            .unwrap();
        let guideline_pos = serialized
            .find("<ram:GuidelineSpecifiedDocumentContextParameter")
            .unwrap();
        assert!(bim_pos < scenario_pos);
        assert!(scenario_pos < application_pos);
        assert!(application_pos < guideline_pos);
        assert_eq!(parse_document(&serialized), parsed);
    }

    /// BT-158 round-trip: a CII line carrying a
    /// `ram:DesignatedProductClassification/ram:ClassCode[@listID,@listVersionID]`
    /// must parse into `line.classifications`, serialize back identically, and
    /// appear exactly once (mapped, never also preserved-raw).
    #[test]
    fn designated_product_classification_round_trips_once() {
        let document = fixture(DocumentType::Invoice, 41);
        let xml = to_xml(&document).unwrap();
        let classification = format!(
            r#"<ram:DesignatedProductClassification xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ClassCode listID="HS" listVersionID="2023">8471.30</ram:ClassCode></ram:DesignatedProductClassification>"#
        );
        // Insert the classification inside SpecifiedTradeProduct, after ram:Name.
        let with_classification = xml.replacen(
            "</ram:SpecifiedTradeProduct>",
            &format!("{classification}</ram:SpecifiedTradeProduct>"),
            1,
        );

        let parsed = parse_document(&with_classification);
        let line = parsed.lines.first().expect("line");
        assert_eq!(line.classifications.len(), 1);
        let mapped = &line.classifications[0];
        assert_eq!(mapped.code, "8471.30");
        assert_eq!(mapped.scheme_id, "HS");
        assert_eq!(mapped.scheme_version.as_deref(), Some("2023"));

        let serialized = to_xml(&parsed).unwrap();
        // Mapped, not also preserved-raw: appears exactly once.
        assert_eq!(serialized.matches("<ram:DesignatedProductClassification>").count(), 1);
        assert_eq!(serialized.matches("<ram:ClassCode").count(), 1);
        assert!(serialized.contains(
            r#"<ram:DesignatedProductClassification><ram:ClassCode listID="HS" listVersionID="2023">8471.30</ram:ClassCode></ram:DesignatedProductClassification>"#
        ));
        // No preserved_xml fragment captured the classification.
        let preserved_has_classification = parsed
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
            .and_then(|extension| extension.payload.get("preserved_xml"))
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("element").and_then(Value::as_str)
                        == Some("DesignatedProductClassification")
                })
            });
        assert!(
            !preserved_has_classification,
            "BT-158 must be mapped, never also preserved-raw"
        );

        // Idempotent canonical serialization (schema order holds) and stable parse.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
        assert_eq!(parse_document(&serialized), parsed);
    }

    /// MIXED case: a line with BOTH a native BT-158 classification AND preserved
    /// `SpecifiedTradeProduct` siblings at a lower schema order
    /// (`ram:ApplicableProductCharacteristic`, order 24 < 26) and a higher one
    /// (`ram:OriginTradeCountry`, order 31 > 26). The native classification must
    /// land in its correct schema slot so the emitted document is canonical and
    /// no preserved element is dropped.
    #[test]
    fn designated_product_classification_emits_in_schema_order_with_preserved_siblings() {
        let mut document = fixture(DocumentType::Invoice, 42);
        document.lines.first_mut().unwrap().classifications = vec![ItemClassification {
            code: "8471.30".to_owned(),
            scheme_id: "HS".to_owned(),
            scheme_version: Some("2023".to_owned()),
        }];
        let product_container = "CrossIndustryInvoice/SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem/SpecifiedTradeProduct";
        let line_id = document.lines.first().unwrap().id.clone();
        document.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [
                        {
                            "container": product_container,
                            "element": "OriginTradeCountry",
                            "line_id": line_id,
                            "xml": format!(r#"<ram:OriginTradeCountry xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>DE</ram:ID></ram:OriginTradeCountry>"#)
                        },
                        {
                            "container": product_container,
                            "element": "ApplicableProductCharacteristic",
                            "line_id": line_id,
                            "xml": format!(r#"<ram:ApplicableProductCharacteristic xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:Description>Colour</ram:Description><ram:Value>Blue</ram:Value></ram:ApplicableProductCharacteristic>"#)
                        }
                    ]
                }),
            )
            .unwrap(),
        );

        let serialized = to_xml(&document).unwrap();

        // Canonical idempotence: schema order holds across the native + preserved mix.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);

        // Correct schema order: ApplicableProductCharacteristic (24) precedes
        // DesignatedProductClassification (26) precedes OriginTradeCountry (31).
        let characteristic_pos = serialized
            .find("<ram:ApplicableProductCharacteristic")
            .expect("preserved lower-order sibling present");
        let classification_pos = serialized
            .find("<ram:DesignatedProductClassification>")
            .expect("native classification present");
        let origin_pos = serialized
            .find("<ram:OriginTradeCountry")
            .expect("preserved higher-order sibling present");
        assert!(characteristic_pos < classification_pos);
        assert!(classification_pos < origin_pos);

        // No element dropped: each appears exactly once, plus a stable round-trip.
        assert_eq!(serialized.matches("<ram:DesignatedProductClassification>").count(), 1);
        assert_eq!(serialized.matches("<ram:ApplicableProductCharacteristic").count(), 1);
        assert_eq!(serialized.matches("<ram:OriginTradeCountry").count(), 1);

        let parsed = parse_document(&serialized);
        assert_eq!(parsed.lines[0].classifications, document.lines[0].classifications);
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
    }

    /// GATING TEST (1) for EN 16931 BT-120 / BT-121: a fresh document whose tax
    /// category carries both `exemption_reason` (BT-120, free text) and
    /// `exemption_reason_code` (BT-121, a controlled-list code) emits both
    /// elements at their correct CII slots, and a full
    /// serialize -> PARSE -> serialize round-trip is byte-stable. The parse step
    /// is essential: a `to_xml`-only test would miss a parser rejection (the
    /// lesson from the `FormattedIssueDateTime` parser bug).
    #[test]
    fn tax_exemption_reason_and_code_emit_in_schema_order_and_round_trip() {
        let mut document = fixture(DocumentType::Invoice, 70);
        let summary = document.tax_summary.first_mut().unwrap();
        // Reverse-charge category with a CEF VATEX code, verbatim.
        summary.category_code = "AE".to_owned();
        summary.tax_rate = Some(DecimalValue::new(Decimal::ZERO));
        summary.exemption_reason = Some("Reverse charge".to_owned());
        summary.exemption_reason_code = Some("VATEX-EU-AE".to_owned());
        document.lines.first_mut().unwrap().tax_category = Some("AE".to_owned());

        let serialized = to_xml(&document).unwrap();

        // GATING (2): canonical idempotence — schema order holds, output stable.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);

        // Both EN 16931 bindings are present, serialized verbatim (no mapping).
        assert!(serialized.contains("<ram:ExemptionReason>Reverse charge</ram:ExemptionReason>"));
        assert!(serialized
            .contains("<ram:ExemptionReasonCode>VATEX-EU-AE</ram:ExemptionReasonCode>"));

        // The exemption bindings live in the header-summary ApplicableTradeTax,
        // not the line-level one. Scope position checks to the header block,
        // which is the only ApplicableTradeTax that emits ram:CalculatedAmount.
        let header_tax = &serialized[serialized.find("<ram:CalculatedAmount").unwrap()..];

        // Exact CII child order within the header ApplicableTradeTax:
        // TypeCode < ExemptionReason < BasisAmount < CategoryCode
        //   < ExemptionReasonCode < RateApplicablePercent.
        let type_code = header_tax.find("<ram:TypeCode>VAT</ram:TypeCode>").unwrap();
        let exemption_reason = header_tax.find("<ram:ExemptionReason>").unwrap();
        let basis_amount = header_tax.find("<ram:BasisAmount").unwrap();
        let category_code = header_tax.find("<ram:CategoryCode>AE</ram:CategoryCode>").unwrap();
        let exemption_code = header_tax.find("<ram:ExemptionReasonCode>").unwrap();
        let rate = header_tax.find("<ram:RateApplicablePercent>").unwrap();
        assert!(type_code < exemption_reason);
        assert!(exemption_reason < basis_amount);
        assert!(basis_amount < category_code);
        assert!(category_code < exemption_code);
        assert!(exemption_code < rate);

        // Each binding appears exactly once in the whole document.
        assert_eq!(serialized.matches("<ram:ExemptionReason>").count(), 1);
        assert_eq!(serialized.matches("<ram:ExemptionReasonCode>").count(), 1);

        // serialize -> PARSE -> serialize is byte-stable (not just to_xml idempotence).
        let parsed = parse_document(&serialized);
        assert_eq!(
            parsed.tax_summary[0].exemption_reason.as_deref(),
            Some("Reverse charge")
        );
        assert_eq!(
            parsed.tax_summary[0].exemption_reason_code.as_deref(),
            Some("VATEX-EU-AE")
        );
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
    }

    #[test]
    fn invoice_period_emits_billing_specified_period_in_schema_order() {
        use invoicekit_ir::InvoicePeriod;

        let mut document = fixture(DocumentType::Invoice, 71);
        document.invoice_period = Some(InvoicePeriod {
            start_date: Some(DateOnly::new("2026-05-01").unwrap()),
            end_date: Some(DateOnly::new("2026-05-31").unwrap()),
        });

        let serialized = to_xml(&document).unwrap();

        // GATING: canonical idempotence — schema order holds, output stable.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);

        // BG-14: ram:BillingSpecifiedPeriod with StartDateTime (BT-73) and
        // EndDateTime (BT-74), each a udt:DateTimeString format="102". The
        // canonicalizer pins an inline xmlns:udt on each DateTimeString, so
        // assert structure + the format="102" date payloads rather than the
        // raw open tags.
        assert!(
            serialized.contains("<ram:BillingSpecifiedPeriod><ram:StartDateTime>"),
            "BG-14 period must open with StartDateTime:\n{serialized}"
        );
        assert!(
            serialized.contains(
                r#"format="102">20260501</udt:DateTimeString></ram:StartDateTime><ram:EndDateTime>"#
            ),
            "BT-73 start date must serialize as a format=102 DateTimeString:\n{serialized}"
        );
        assert!(
            serialized.contains(
                r#"format="102">20260531</udt:DateTimeString></ram:EndDateTime></ram:BillingSpecifiedPeriod>"#
            ),
            "BT-74 end date must close the BillingSpecifiedPeriod:\n{serialized}"
        );

        // CII child order within ApplicableHeaderTradeSettlement:
        // ApplicableTradeTax < BillingSpecifiedPeriod < MonetarySummation.
        let tax = serialized.find("<ram:ApplicableTradeTax>").unwrap();
        let period = serialized.find("<ram:BillingSpecifiedPeriod>").unwrap();
        let summation = serialized
            .find("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>")
            .unwrap();
        assert!(
            tax < period && period < summation,
            "BillingSpecifiedPeriod must sit between ApplicableTradeTax and the monetary summation:\n{serialized}"
        );

        // serialize -> PARSE -> serialize is byte-stable (not just to_xml
        // idempotence). The parser preserves BillingSpecifiedPeriod as raw XML
        // (invoice_period stays None), and the preserved replay re-emits it
        // EXACTLY ONCE — proving no native + preserved double-emit.
        let parsed = parse_document(&serialized);
        assert!(
            parsed.invoice_period.is_none(),
            "parser preserves BillingSpecifiedPeriod as raw XML, does not populate invoice_period"
        );
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
        assert_eq!(serialized.matches("<ram:BillingSpecifiedPeriod>").count(), 1);
    }

    #[test]
    fn absent_invoice_period_emits_no_billing_specified_period() {
        let document = fixture(DocumentType::Invoice, 72);
        assert!(document.invoice_period.is_none());
        let serialized = to_xml(&document).unwrap();
        assert!(
            !serialized.contains("<ram:BillingSpecifiedPeriod>"),
            "a None invoice_period must emit no ram:BillingSpecifiedPeriod:\n{serialized}"
        );
    }

    /// Gating test: a parse-then-enrich document carrying BOTH a preserved
    /// `ram:BillingSpecifiedPeriod` AND a caller-set `invoice_period` must emit
    /// the element EXACTLY ONCE (preserved fragment wins) — never a malformed
    /// duplicate of the 0..1 BG-14 group.
    #[test]
    fn both_native_and_preserved_billing_period_emit_once_preserved_wins() {
        use invoicekit_ir::InvoicePeriod;

        // Seed a document whose serialized form carries a BillingSpecifiedPeriod,
        // then parse it: the element becomes preserved raw XML, invoice_period None.
        let mut seed = fixture(DocumentType::Invoice, 73);
        seed.invoice_period = Some(InvoicePeriod {
            start_date: Some(DateOnly::new("2026-05-01").unwrap()),
            end_date: Some(DateOnly::new("2026-05-31").unwrap()),
        });
        let seeded = to_xml(&seed).unwrap();
        let mut parsed = parse_document(&seeded);
        assert!(parsed.invoice_period.is_none());

        // A consumer ALSO sets the typed field (the parse-then-enrich edge).
        parsed.invoice_period = Some(InvoicePeriod {
            start_date: Some(DateOnly::new("2099-09-09").unwrap()),
            end_date: None,
        });

        let out = to_xml(&parsed).unwrap();
        assert_eq!(
            out.matches("<ram:BillingSpecifiedPeriod>").count(),
            1,
            "exactly one ram:BillingSpecifiedPeriod (preserved wins):\n{out}"
        );
        // Preserved fragment wins: the original period survives, not the override.
        assert!(out.contains("format=\"102\">20260501</udt:DateTimeString>"));
        assert!(!out.contains("20990909"), "native override must not appear:\n{out}");
        assert_eq!(canonicalize_xml(&out).unwrap(), out);
    }

    #[test]
    fn allowance_charges_emit_specified_trade_allowance_charge_in_schema_order() {
        use invoicekit_ir::DocumentAllowanceCharge;

        let mut document = fixture(DocumentType::Invoice, 74);
        document.allowance_charges = vec![
            DocumentAllowanceCharge {
                is_charge: false,
                amount: DecimalValue::new(Decimal::new(1000, 2)),
                base_amount: Some(DecimalValue::new(Decimal::new(10000, 2))),
                percentage: Some(DecimalValue::new(Decimal::new(1000, 2))),
                tax_category: Some("S".to_owned()),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
                reason: Some("Loyalty & volume discount".to_owned()),
                reason_code: Some("95".to_owned()),
            },
            DocumentAllowanceCharge {
                is_charge: true,
                amount: DecimalValue::new(Decimal::new(500, 2)),
                base_amount: None,
                percentage: None,
                tax_category: None,
                tax_rate: None,
                reason: Some("Freight".to_owned()),
                reason_code: None,
            },
        ];

        let serialized = to_xml(&document).unwrap();

        // GATING: canonical idempotence.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);

        // Two SpecifiedTradeAllowanceCharge, allowance (false) then charge (true).
        assert_eq!(
            serialized
                .matches("<ram:SpecifiedTradeAllowanceCharge>")
                .count(),
            2,
            "expected two ram:SpecifiedTradeAllowanceCharge:\n{serialized}"
        );
        // ChargeIndicator carries a udt:Indicator (canonicalizer pins xmlns:udt
        // on it, so match the content + close).
        assert!(serialized.contains(">false</udt:Indicator></ram:ChargeIndicator>"));
        assert!(serialized.contains(">true</udt:Indicator></ram:ChargeIndicator>"));
        // BT-92 amount, BT-93 base, BT-94 percentage, BT-97 reason (escaped), BT-98 code.
        assert!(serialized.contains("<ram:ActualAmount>10.00</ram:ActualAmount>"));
        assert!(serialized.contains("<ram:ActualAmount>5.00</ram:ActualAmount>"));
        assert!(serialized.contains("<ram:BasisAmount>100.00</ram:BasisAmount>"));
        assert!(serialized.contains("<ram:CalculationPercent>10.00</ram:CalculationPercent>"));
        assert!(
            serialized.contains("<ram:Reason>Loyalty &amp; volume discount</ram:Reason>"),
            "BT-97 reason must be XML-escaped:\n{serialized}"
        );
        assert!(serialized.contains("<ram:ReasonCode>95</ram:ReasonCode>"));
        assert!(serialized.contains("<ram:CategoryTradeTax>"));

        // Placement: ApplicableTradeTax < SpecifiedTradeAllowanceCharge <
        // SpecifiedTradePaymentTerms.
        let tax = serialized.find("<ram:ApplicableTradeTax>").unwrap();
        let allowance = serialized
            .find("<ram:SpecifiedTradeAllowanceCharge>")
            .unwrap();
        let terms = serialized
            .find("<ram:SpecifiedTradePaymentTerms>")
            .unwrap();
        assert!(
            tax < allowance && allowance < terms,
            "SpecifiedTradeAllowanceCharge must sit after ApplicableTradeTax and before payment terms:\n{serialized}"
        );

        // serialize -> PARSE -> serialize byte-stable: the parser preserves the
        // elements as raw XML (allowance_charges stays empty) and the preserved
        // replay re-emits them exactly.
        let parsed = parse_document(&serialized);
        assert!(parsed.allowance_charges.is_empty());
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
    }

    #[test]
    fn absent_allowance_charges_emit_no_specified_trade_allowance_charge() {
        let document = fixture(DocumentType::Invoice, 75);
        assert!(document.allowance_charges.is_empty());
        let serialized = to_xml(&document).unwrap();
        assert!(
            !serialized.contains("<ram:SpecifiedTradeAllowanceCharge>"),
            "an empty allowance_charges must emit no ram:SpecifiedTradeAllowanceCharge:\n{serialized}"
        );
    }

    /// Gating test for the CII child-order fix: a parse-then-enrich document
    /// carrying BOTH a preserved `BillingSpecifiedPeriod` (BG-14, order 25) AND
    /// caller-set native allowances (BG-20/21, order 26) must emit the period
    /// BEFORE the allowances. The native allowances are emitted before the
    /// preserve replay, so without the explicit reordering the preserved period
    /// would (incorrectly) follow them — schema-out-of-order yet
    /// canonically-idempotent.
    #[test]
    fn preserved_billing_period_precedes_native_allowance_charges() {
        use invoicekit_ir::{DocumentAllowanceCharge, InvoicePeriod};

        // Seed a doc whose serialized form carries a BillingSpecifiedPeriod, then
        // parse it: the period becomes preserved raw XML, invoice_period None.
        let mut seed = fixture(DocumentType::Invoice, 76);
        seed.invoice_period = Some(InvoicePeriod {
            start_date: Some(DateOnly::new("2026-05-01").unwrap()),
            end_date: Some(DateOnly::new("2026-05-31").unwrap()),
        });
        let mut parsed = parse_document(&to_xml(&seed).unwrap());
        assert!(parsed.invoice_period.is_none());

        // A consumer adds native allowances/charges.
        parsed.allowance_charges = vec![DocumentAllowanceCharge {
            is_charge: false,
            amount: DecimalValue::new(Decimal::new(1000, 2)),
            base_amount: None,
            percentage: None,
            tax_category: None,
            tax_rate: None,
            reason: Some("Discount".to_owned()),
            reason_code: None,
        }];

        let out = to_xml(&parsed).unwrap();

        // Each element appears exactly once, and the period precedes the allowance.
        assert_eq!(out.matches("<ram:BillingSpecifiedPeriod>").count(), 1);
        assert_eq!(out.matches("<ram:SpecifiedTradeAllowanceCharge>").count(), 1);
        let period = out.find("<ram:BillingSpecifiedPeriod>").unwrap();
        let allowance = out.find("<ram:SpecifiedTradeAllowanceCharge>").unwrap();
        assert!(
            period < allowance,
            "preserved BillingSpecifiedPeriod must precede native allowances (CII child order):\n{out}"
        );
        // Output is canonical, and round-trips byte-stably.
        assert_eq!(canonicalize_xml(&out).unwrap(), out);
        assert_eq!(to_xml(&parse_document(&out)).unwrap(), out);
    }

    #[test]
    fn tax_exemption_reason_with_xml_entities_round_trips_losslessly() {
        // BT-120 free text frequently contains `&`/`<`/`>`. quick-xml splits
        // entity-interrupted text into several events; the parser must
        // ACCUMULATE them, not overwrite (which kept only the last fragment).
        let mut document = fixture(DocumentType::Invoice, 71);
        let summary = document.tax_summary.first_mut().unwrap();
        summary.category_code = "AE".to_owned();
        summary.tax_rate = Some(DecimalValue::new(Decimal::ZERO));
        summary.exemption_reason = Some("Exempt: Art 132 A & B < C, D > E".to_owned());
        summary.exemption_reason_code = Some("VATEX-A & B".to_owned());
        document.lines.first_mut().unwrap().tax_category = Some("AE".to_owned());

        let serialized = to_xml(&document).unwrap();
        let parsed = parse_document(&serialized);
        assert_eq!(
            parsed.tax_summary[0].exemption_reason.as_deref(),
            Some("Exempt: Art 132 A & B < C, D > E"),
            "entity-bearing BT-120 text must survive parse verbatim (no fragment loss)"
        );
        assert_eq!(
            parsed.tax_summary[0].exemption_reason_code.as_deref(),
            Some("VATEX-A & B"),
            "entity-bearing BT-121 code must survive parse verbatim"
        );
        assert_eq!(
            to_xml(&parsed).unwrap(),
            serialized,
            "entity-bearing exemption round-trip must be byte-stable"
        );
    }

    /// Behavior-preserving guard: a tax category with neither exemption field
    /// emits no `ram:ExemptionReason*` element, so documents that predate this
    /// binding serialize byte-identically. Round-trips through parse unchanged.
    #[test]
    fn tax_summary_without_exemption_omits_both_elements() {
        let document = fixture(DocumentType::Invoice, 71);
        assert!(document.tax_summary[0].exemption_reason.is_none());
        assert!(document.tax_summary[0].exemption_reason_code.is_none());

        let serialized = to_xml(&document).unwrap();
        assert!(!serialized.contains("ram:ExemptionReason"));
        assert!(!serialized.contains("ram:ExemptionReasonCode"));
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);

        let parsed = parse_document(&serialized);
        assert!(parsed.tax_summary[0].exemption_reason.is_none());
        assert!(parsed.tax_summary[0].exemption_reason_code.is_none());
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
    }

    /// Only `exemption_reason_code` (BT-121) without the free-text BT-120: the
    /// code must still land at its own slot (after `CategoryCode`), confirming the
    /// two bindings are emitted independently at their distinct positions.
    #[test]
    fn tax_exemption_code_without_reason_emits_at_its_own_slot() {
        let mut document = fixture(DocumentType::Invoice, 72);
        let summary = document.tax_summary.first_mut().unwrap();
        summary.category_code = "E".to_owned();
        summary.exemption_reason_code = Some("VATEX-EU-132-1A".to_owned());
        document.lines.first_mut().unwrap().tax_category = Some("E".to_owned());

        let serialized = to_xml(&document).unwrap();
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
        assert!(!serialized.contains("<ram:ExemptionReason>"));
        assert!(serialized
            .contains("<ram:ExemptionReasonCode>VATEX-EU-132-1A</ram:ExemptionReasonCode>"));

        let header_tax = &serialized[serialized.find("<ram:CalculatedAmount").unwrap()..];
        let category_code = header_tax.find("<ram:CategoryCode>E</ram:CategoryCode>").unwrap();
        let exemption_code = header_tax.find("<ram:ExemptionReasonCode>").unwrap();
        let rate = header_tax.find("<ram:RateApplicablePercent>").unwrap();
        assert!(category_code < exemption_code);
        assert!(exemption_code < rate);

        let parsed = parse_document(&serialized);
        assert_eq!(parsed.tax_summary[0].exemption_reason, None);
        assert_eq!(
            parsed.tax_summary[0].exemption_reason_code.as_deref(),
            Some("VATEX-EU-132-1A")
        );
        assert_eq!(to_xml(&parsed).unwrap(), serialized);
    }

    /// GATING TEST (1): a fresh `CommercialDocument` carrying one Order-class
    /// reference (BT-13) AND one `PrecedingInvoice`-class reference (BT-25, with an
    /// issue date for BT-26) emits BOTH typed elements at their correct schema
    /// slots, and the serialized output is canonical (idempotent).
    #[test]
    fn document_references_emit_order_and_preceding_invoice_in_schema_order() {
        let mut document = fixture(DocumentType::CreditNote, 60);
        document.references = vec![
            DocumentReference {
                kind: "purchase-order".to_owned(),
                id: "PO-2026-991".to_owned(),
                issue_date: None,
            },
            DocumentReference {
                kind: "original-invoice".to_owned(),
                id: "INV-2026-100".to_owned(),
                issue_date: Some(DateOnly::new("2026-04-15").unwrap()),
            },
        ];

        let serialized = to_xml(&document).unwrap();

        // BT-13: BuyerOrderReferencedDocument under ApplicableHeaderTradeAgreement.
        assert!(serialized.contains(
            "<ram:BuyerOrderReferencedDocument><ram:IssuerAssignedID>PO-2026-991</ram:IssuerAssignedID></ram:BuyerOrderReferencedDocument>"
        ));
        // BT-25: InvoiceReferencedDocument carries the preceding-invoice id, and
        // BT-26 emits as ram:FormattedIssueDateTime/qdt:DateTimeString format="102".
        // (to_xml runs the canonicalizer, which scopes the qdt namespace decl onto
        // the first element that uses the qdt prefix; assert on the stable pieces.)
        assert!(serialized.contains(
            "<ram:InvoiceReferencedDocument><ram:IssuerAssignedID>INV-2026-100</ram:IssuerAssignedID><ram:FormattedIssueDateTime><qdt:DateTimeString"
        ));
        assert!(serialized.contains(r#"format="102">20260415</qdt:DateTimeString></ram:FormattedIssueDateTime></ram:InvoiceReferencedDocument>"#));
        // Each typed element emitted exactly once.
        assert_eq!(serialized.matches("<ram:BuyerOrderReferencedDocument>").count(), 1);
        assert_eq!(serialized.matches("<ram:InvoiceReferencedDocument>").count(), 1);

        // Schema slots: BT-13 sits inside the agreement (after BuyerTradeParty);
        // BT-25 sits inside the settlement (after the monetary summation). The
        // canonicalizer may add an xmlns decl to the container open tag, so match
        // the open tag without its closing bracket.
        let agreement_open = serialized
            .find("<ram:ApplicableHeaderTradeAgreement")
            .expect("agreement present");
        let agreement_close = serialized
            .find("</ram:ApplicableHeaderTradeAgreement>")
            .expect("agreement closes");
        let buyer_order_pos = serialized
            .find("<ram:BuyerOrderReferencedDocument>")
            .expect("BT-13 present");
        assert!(agreement_open < buyer_order_pos && buyer_order_pos < agreement_close);

        let settlement_open = serialized
            .find("<ram:ApplicableHeaderTradeSettlement")
            .expect("settlement present");
        let monetary_pos = serialized
            .find("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>")
            .expect("monetary summation present");
        let invoice_ref_pos = serialized
            .find("<ram:InvoiceReferencedDocument>")
            .expect("BT-25 present");
        assert!(settlement_open < monetary_pos && monetary_pos < invoice_ref_pos);

        // The qdt namespace used by FormattedIssueDateTime is declared at the root.
        assert!(serialized.contains(CII_QDT_NAMESPACE_URI));

        // Canonical idempotence: schema order holds.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
    }

    /// GATING TEST (round-trip): a dated `PrecedingInvoice` reference (BT-25 +
    /// BT-26) must survive serialize -> parse -> serialize. Before the
    /// `expected_cii_namespace` fix, `from_xml` rejected the serializer's own
    /// `qdt:DateTimeString` under `FormattedIssueDateTime` with
    /// `InvalidNamespace` (it hard-coded `DateTimeString -> udt`), so a dated
    /// reference produced output the crate could not re-parse.
    #[test]
    fn dated_preceding_invoice_reference_round_trips_through_parse() {
        let mut document = fixture(DocumentType::CreditNote, 63);
        document.references = vec![DocumentReference {
            kind: "original-invoice".to_owned(),
            id: "INV-2026-100".to_owned(),
            issue_date: Some(DateOnly::new("2026-04-15").unwrap()),
        }];

        let serialized = to_xml(&document).unwrap();
        // The crate must accept its OWN dated-reference output.
        let (reparsed, _ledger) =
            from_xml(&serialized).expect("dated reference output must re-parse");
        let reserialized = to_xml(&reparsed).unwrap();
        assert_eq!(
            reserialized, serialized,
            "dated preceding-invoice reference round-trip is not byte-stable"
        );
        // BT-26 survives exactly once.
        assert_eq!(
            reserialized.matches("<ram:FormattedIssueDateTime>").count(),
            1
        );
        assert!(reserialized.contains(">20260415</qdt:DateTimeString>"));
    }

    /// A document with no references serializes byte-identically to one whose
    /// references vector is empty — proving the binding is behavior-preserving
    /// (existing goldens/corpus unchanged for reference-free documents).
    #[test]
    fn empty_references_serialize_byte_identically() {
        let document = fixture(DocumentType::Invoice, 61);
        assert!(document.references.is_empty());
        let serialized = to_xml(&document).unwrap();
        assert!(!serialized.contains("BuyerOrderReferencedDocument"));
        assert!(!serialized.contains("InvoiceReferencedDocument"));
        // Canonical and stable just as before the binding existed.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
    }

    /// Multiple `PrecedingInvoice`-class references emit one
    /// `InvoiceReferencedDocument` each (BG-3 is `0..n`); a reference without an
    /// issue date omits the optional `FormattedIssueDateTime`. Idempotence holds.
    #[test]
    fn multiple_preceding_invoice_references_emit_one_element_each() {
        let mut document = fixture(DocumentType::CreditNote, 62);
        document.references = vec![
            DocumentReference {
                kind: "preceding-invoice".to_owned(),
                id: "INV-A".to_owned(),
                issue_date: Some(DateOnly::new("2026-03-01").unwrap()),
            },
            DocumentReference {
                kind: "rectified-invoice".to_owned(),
                id: "INV-B".to_owned(),
                issue_date: None,
            },
        ];

        let serialized = to_xml(&document).unwrap();
        assert_eq!(serialized.matches("<ram:InvoiceReferencedDocument>").count(), 2);
        assert!(serialized.contains("<ram:IssuerAssignedID>INV-A</ram:IssuerAssignedID>"));
        assert!(serialized.contains("<ram:IssuerAssignedID>INV-B</ram:IssuerAssignedID>"));
        // INV-A keeps order: its element appears before INV-B's.
        let a_pos = serialized.find("INV-A").unwrap();
        let b_pos = serialized.find("INV-B").unwrap();
        assert!(a_pos < b_pos);
        // Only the dated reference carries FormattedIssueDateTime.
        assert_eq!(serialized.matches("<ram:FormattedIssueDateTime>").count(), 1);
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
    }

    /// Other/Contract-class references are NOT emitted in this task: they neither
    /// produce `BuyerOrderReferencedDocument` nor `InvoiceReferencedDocument` and
    /// the document stays byte-stable/canonical.
    #[test]
    fn non_order_non_preceding_references_are_skipped() {
        let mut document = fixture(DocumentType::Invoice, 63);
        document.references = vec![
            DocumentReference {
                kind: "contract".to_owned(),
                id: "C-1".to_owned(),
                issue_date: None,
            },
            DocumentReference {
                kind: "some-unrecognized-kind".to_owned(),
                id: "X-1".to_owned(),
                issue_date: None,
            },
            DocumentReference {
                kind: "despatch-advice".to_owned(),
                id: "D-1".to_owned(),
                issue_date: None,
            },
        ];
        let serialized = to_xml(&document).unwrap();
        assert!(!serialized.contains("BuyerOrderReferencedDocument"));
        assert!(!serialized.contains("InvoiceReferencedDocument"));
        assert!(!serialized.contains("C-1"));
        assert!(!serialized.contains("X-1"));
        assert!(!serialized.contains("D-1"));
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
    }

    /// GATING TEST (2): NO DOUBLE-EMIT through parse. The parser never populates
    /// `document.references` (it preserves typed reference elements as raw
    /// round-trip XML), so a document that carried a `BuyerOrderReferencedDocument`
    /// and an `InvoiceReferencedDocument` through parse must re-emit each EXACTLY
    /// ONCE — once from the preserved-XML replay, never also from the native
    /// `references` path (which stays empty for parsed docs).
    #[test]
    fn parsed_reference_elements_re_emit_exactly_once_no_double() {
        let document = fixture(DocumentType::CreditNote, 64);
        let xml = to_xml(&document).unwrap();

        // Inject a BT-13 element into the agreement and a BT-25 element into the
        // settlement, mirroring a producer that carried these natively.
        let buyer_order = "<ram:BuyerOrderReferencedDocument><ram:IssuerAssignedID>PO-CARRIED</ram:IssuerAssignedID></ram:BuyerOrderReferencedDocument>";
        let with_order = xml.replacen(
            "</ram:ApplicableHeaderTradeAgreement>",
            &format!("{buyer_order}</ram:ApplicableHeaderTradeAgreement>"),
            1,
        );
        let invoice_ref = "<ram:InvoiceReferencedDocument><ram:IssuerAssignedID>INV-CARRIED</ram:IssuerAssignedID></ram:InvoiceReferencedDocument>";
        let carried = with_order.replacen(
            "</ram:ApplicableHeaderTradeSettlement>",
            &format!("{invoice_ref}</ram:ApplicableHeaderTradeSettlement>"),
            1,
        );

        let parsed = parse_document(&carried);
        // Parser keeps these as preserved raw XML; native references stay empty.
        assert!(
            parsed.references.is_empty(),
            "parser must not populate references (preserve-only path)"
        );

        let reserialized = to_xml(&parsed).unwrap();
        // Each carried element re-emits EXACTLY once — no double from a second path.
        assert_eq!(
            reserialized.matches("<ram:BuyerOrderReferencedDocument>").count(),
            1,
            "BT-13 must re-emit exactly once (preserved replay, not also native)"
        );
        assert_eq!(
            reserialized.matches("<ram:InvoiceReferencedDocument>").count(),
            1,
            "BT-25 must re-emit exactly once (preserved replay, not also native)"
        );
        assert!(reserialized.contains("PO-CARRIED"));
        assert!(reserialized.contains("INV-CARRIED"));

        // Stable, canonical, and re-parses identically (no regression / no drift).
        assert_eq!(canonicalize_xml(&reserialized).unwrap(), reserialized);
        assert_eq!(parse_document(&reserialized), parsed);
    }

    /// MIXED case: a parsed doc carrying a preserved BT-25 sibling, then re-emitted
    /// with a NATIVE Order-class reference added to the IR. Both must land in
    /// schema order (BT-13 in the agreement, preserved BT-25 in the settlement),
    /// each exactly once, and the result stays canonical.
    #[test]
    fn native_order_reference_coexists_with_preserved_preceding_invoice() {
        let document = fixture(DocumentType::CreditNote, 65);
        let xml = to_xml(&document).unwrap();
        let invoice_ref = "<ram:InvoiceReferencedDocument><ram:IssuerAssignedID>INV-PRESERVED</ram:IssuerAssignedID></ram:InvoiceReferencedDocument>";
        let carried = xml.replacen(
            "</ram:ApplicableHeaderTradeSettlement>",
            &format!("{invoice_ref}</ram:ApplicableHeaderTradeSettlement>"),
            1,
        );

        let mut parsed = parse_document(&carried);
        // Add a NATIVE order reference to the IR (fresh BT-13).
        parsed.references.push(DocumentReference {
            kind: "order".to_owned(),
            id: "PO-NATIVE".to_owned(),
            issue_date: None,
        });

        let serialized = to_xml(&parsed).unwrap();
        // Native BT-13 emitted once; preserved BT-25 replayed once.
        assert_eq!(serialized.matches("<ram:BuyerOrderReferencedDocument>").count(), 1);
        assert_eq!(serialized.matches("<ram:InvoiceReferencedDocument>").count(), 1);
        assert!(serialized.contains("PO-NATIVE"));
        assert!(serialized.contains("INV-PRESERVED"));
        // Schema order holds across native + preserved mix.
        assert_eq!(canonicalize_xml(&serialized).unwrap(), serialized);
    }

    #[test]
    fn serializer_sorts_hand_authored_preserved_cii_fragments() {
        let mut document = fixture(DocumentType::Invoice, 36);
        document.extensions.push(
            JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                json!({
                    "preserved_xml": [
                        {
                            "container": "CrossIndustryInvoice/ExchangedDocumentContext",
                            "element": "ScenarioSpecifiedDocumentContextParameter",
                            "xml": format!(r#"<ram:ScenarioSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>SCENARIO-1</ram:ID></ram:ScenarioSpecifiedDocumentContextParameter>"#)
                        },
                        {
                            "container": "CrossIndustryInvoice/ExchangedDocumentContext",
                            "element": "BIMSpecifiedDocumentContextParameter",
                            "xml": format!(r#"<ram:BIMSpecifiedDocumentContextParameter xmlns:ram="{CII_RAM_NAMESPACE_URI}"><ram:ID>BIM-1</ram:ID></ram:BIMSpecifiedDocumentContextParameter>"#)
                        }
                    ]
                }),
            )
            .unwrap(),
        );

        let serialized = to_xml(&document).unwrap();
        let bim_pos = serialized
            .find("<ram:BIMSpecifiedDocumentContextParameter")
            .unwrap();
        let scenario_pos = serialized
            .find("<ram:ScenarioSpecifiedDocumentContextParameter")
            .unwrap();
        assert!(bim_pos < scenario_pos);
    }

    #[test]
    fn mapping_decisions_name_standard_field_boundaries() {
        assert_eq!(mapping::NAMED_MAPPING_DECISIONS.len(), 7);
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element == "HeaderTradeAgreementType/BuyerReference"
                && decision.class == "cii_document_field_extension"
                && decision.rationale.contains("never tenant_id")
        }));
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element
                == "ExchangedDocumentContextType/BusinessProcessSpecifiedDocumentContextParameter"
                && decision.class == "cii_document_field_extension"
                && decision.rationale.contains("never trace_id")
                && decision
                    .representation
                    .contains("business_process_context_ids[]")
        }));
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element
                == "ExchangedDocumentContextType/GuidelineSpecifiedDocumentContextParameter"
                && decision.class == "profile_extension_payload"
                && decision.representation.contains("guideline_context_ids[]")
                && decision
                    .rationale
                    .contains("never a business-process context")
        }));
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element == "ExchangedDocumentContextType/SpecifiedTransactionID"
                && decision.class == "profile_extension_payload"
                && decision.representation.contains("transaction_ids[]")
        }));
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element == "ExchangedDocumentContextType/TestIndicator"
                && decision.class == "profile_extension_payload"
                && decision.representation.contains("test_indicators[]")
        }));
        assert!(mapping::NAMED_MAPPING_DECISIONS.iter().any(|decision| {
            decision.element
                == "ExchangedDocumentContextType/ApplicationSpecifiedDocumentContextParameter"
                && decision.class == "profile_extension_payload"
                && decision.representation.contains("application_contexts[]")
        }));
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
    fn escaped_text_and_numeric_references_round_trip() {
        let mut document = fixture(DocumentType::Invoice, 8);
        document.supplier.name = "Supplier & Sons".to_owned();
        document.customer.name = "Customer <EU>".to_owned();
        document.lines.first_mut().unwrap().description =
            "Research & implementation <core>".to_owned();
        document.notes.first_mut().unwrap().text = "Line break & Co".to_owned();

        let xml = to_xml(&document).unwrap();
        assert!(xml.contains("Supplier &amp; Sons"));
        assert!(xml.contains("Customer &lt;EU&gt;"));
        assert_eq!(parse_document(&xml), document);

        let numeric_reference_xml = xml.replace("Supplier &amp; Sons", "Supplier &#x26; Sons");
        assert_eq!(parse_document(&numeric_reference_xml), document);
    }

    #[test]
    fn rejects_invalid_numeric_character_reference() {
        let document = fixture(DocumentType::Invoice, 9);
        let xml = to_xml(&document).unwrap();
        let invalid_reference_xml = xml.replace(&document.supplier.name, "Supplier &#0; Sons");

        let err = from_xml(&invalid_reference_xml).unwrap_err();
        assert!(matches!(err, CiiError::UnsupportedRoot(_)));
    }

    #[test]
    fn rejects_unsupported_root() {
        let err = from_xml("<Invoice/>").unwrap_err();
        assert!(matches!(err, CiiError::UnsupportedRoot(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        let xml = r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"/>"#;
        let err = from_xml(xml).unwrap_err();
        assert!(matches!(err, CiiError::MissingElement(_)));
    }

    #[test]
    fn rejects_invalid_decimal() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 4))
            .unwrap()
            .replacen(">100.04<", ">not-decimal<", 1);
        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(err, CiiError::InvalidDecimal { .. }));
    }

    #[test]
    fn rejects_invalid_cii_date() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 5))
            .unwrap()
            .replacen(">20260526<", ">20261340<", 1);
        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(err, CiiError::InvalidDate { .. }));
    }

    #[test]
    fn rejects_unsupported_type_code() {
        let xml = to_xml(&fixture(DocumentType::Invoice, 6))
            .unwrap()
            .replacen(">380<", ">999<", 1);
        let err = from_xml(&xml).unwrap_err();
        assert!(matches!(err, CiiError::UnsupportedTypeCode(_)));
    }

    #[test]
    fn rejects_unsupported_document_type_on_serialize() {
        let mut document = fixture(DocumentType::Invoice, 7);
        document.document_type = DocumentType::DebitNote;
        let err = to_xml(&document).unwrap_err();
        assert!(matches!(err, CiiError::UnsupportedDocumentType(_)));
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
        let number = format!("CII-{seed:04}");
        let supplier = party("supplier", "Supplier GmbH", "DE123456789");
        let customer = party("customer", "Customer BV", "NL123456789B01");

        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(number.clone()).unwrap(),
            document_type,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: Some(DateOnly::new("2026-05-26").unwrap()),
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new(number).unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier,
            customer,
            payee: Some(party("payee", "Payee GmbH", "DE987654321")),
            payment_terms: Some(PaymentTerms {
                description: "Payable within 30 days".to_owned(),
                due_date: Some(DateOnly::new("2026-06-25").unwrap()),
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
                unit_code: Some("C62".to_owned()),
                unit_price: amount.clone(),
                line_extension_amount: amount.clone(),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amount.clone(),
                tax_amount: DecimalValue::new(tax),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
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
                language: "und".to_owned(),
                text: format!("Fixture note {seed}"),
            }],
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
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

    fn preserved_xml_items(document: &CommercialDocument) -> &[Value] {
        document
            .extensions
            .iter()
            .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
            .and_then(|extension| extension.payload.get("preserved_xml"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .expect("preserved_xml extension")
    }

    fn insert_before_tag_after(
        input: &str,
        start_marker: &str,
        tag_start: &str,
        insertion: &str,
    ) -> String {
        let start = input.find(start_marker).expect("start marker");
        let position = start + input[start..].find(tag_start).expect("tag start");
        format!("{}{}{}", &input[..position], insertion, &input[position..])
    }
}
