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
    DocumentType, IrError, Iso4217Code, JurisdictionExtension, LocalizedString, MonetaryTotal,
    MoneyAmount, Party, PartyTaxId, PaymentInstruction, PaymentInstructionKind, PaymentTerms,
    PostalAddress, Quantity, SchemaVersion, TaxCategorySummary,
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
/// have an IR field yet are accepted by the XML reader but are not represented
/// semantically in the returned [`CommercialDocument`].
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
/// let parsed = invoicekit_format_cii::from_xml(xml).unwrap();
/// assert_eq!(parsed.document_type, invoicekit_ir::DocumentType::Invoice);
/// ```
#[allow(clippy::too_many_lines)]
pub fn from_xml(input: &str) -> Result<CommercialDocument, CiiError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut xml_version = XmlVersion::default();
    let mut stack = Vec::<String>::new();
    let mut text_stack = Vec::<String>::new();
    let mut state = ParseState::default();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                state.start_element(&stack, &name, &attrs)?;
                stack.push(name);
                text_stack.push(String::new());
            }
            Event::Empty(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                state.start_element(&stack, &name, &attrs)?;
                state.end_element(&name)?;
            }
            Event::End(end) => {
                let name = decode_local_name(end.name().as_ref())?;
                let Some(opened) = stack.last() else {
                    return Err(CiiError::UnsupportedRoot(name));
                };
                if opened != &name {
                    return Err(CiiError::UnsupportedRoot(format!("{opened}/{name}")));
                }
                let text = text_stack
                    .pop()
                    .ok_or_else(|| CiiError::UnsupportedRoot(format!("{name}:text")))?;
                if !text.is_empty() {
                    state.text(&stack, &text)?;
                }
                state.end_element(&name)?;
                stack.pop();
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
        if name == "IncludedSupplyChainTradeLineItem" {
            let line = self
                .current_line
                .take()
                .ok_or(CiiError::MissingElement("IncludedSupplyChainTradeLineItem"))?
                .build()?;
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
            if path_ends(stack, &["ApplicableTradeTax", "RateApplicablePercent"]) {
                tax.tax_rate = Some(decimal_value("tax_summary.tax_rate", value)?);
                return Ok(());
            }
        }

        if let Some(role) = party_role(stack) {
            let party = self.party_mut(role);
            if path_ends(stack, &["SpecifiedLegalOrganization", "ID"])
                || path_ends(stack, &["GlobalID"])
                || path_ends(stack, &["SellerTradeParty", "ID"])
                || path_ends(stack, &["BuyerTradeParty", "ID"])
                || path_ends(stack, &["PayeeTradeParty", "ID"])
            {
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
                }
            }
        }
        Ok(())
    }

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
        if !cii_document_fields.is_empty() {
            extensions.push(JurisdictionExtension::new(
                mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
                Value::Object(cii_document_fields),
            )?);
        }
        if !self.cii_guideline_context_ids.is_empty() {
            extensions.push(JurisdictionExtension::new(
                mapping::CII_PROFILE_CONTEXT_EXTENSION_URN,
                json_object_array("guideline_context_ids", self.cii_guideline_context_ids),
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
    write_document_context(&mut xml, document)?;

    xml.push_str("<rsm:ExchangedDocument>");
    write_text_element(
        &mut xml,
        "ram:ID",
        &string_value(&document.document_number)?,
    );
    write_text_element(&mut xml, "ram:TypeCode", document_type_code(document)?);
    xml.push_str("<ram:IssueDateTime>");
    write_date_time(&mut xml, &document.issue_date)?;
    xml.push_str("</ram:IssueDateTime>");
    for note in &document.notes {
        write_note(&mut xml, note);
    }
    xml.push_str("</rsm:ExchangedDocument>");

    xml.push_str("<rsm:SupplyChainTradeTransaction>");
    for line in &document.lines {
        write_line(&mut xml, line, &currency);
    }
    xml.push_str("<ram:ApplicableHeaderTradeAgreement>");
    if let Some(value) = cii_document_field_value(document, "buyer_reference") {
        write_text_element(&mut xml, "ram:BuyerReference", value);
    }
    write_party(&mut xml, "ram:SellerTradeParty", &document.supplier)?;
    write_party(&mut xml, "ram:BuyerTradeParty", &document.customer)?;
    xml.push_str("</ram:ApplicableHeaderTradeAgreement>");

    xml.push_str("<ram:ApplicableHeaderTradeDelivery>");
    if let Some(date) = &document.tax_point_date {
        xml.push_str("<ram:ActualDeliverySupplyChainEvent><ram:OccurrenceDateTime>");
        write_date_time(&mut xml, date)?;
        xml.push_str("</ram:OccurrenceDateTime></ram:ActualDeliverySupplyChainEvent>");
    }
    xml.push_str("</ram:ApplicableHeaderTradeDelivery>");

    xml.push_str("<ram:ApplicableHeaderTradeSettlement>");
    if let Some(payee) = &document.payee {
        write_party(&mut xml, "ram:PayeeTradeParty", payee)?;
    }
    if let Some(first) = document
        .payment_instructions
        .iter()
        .find(|instruction| instruction.reference.is_some())
        .and_then(|instruction| instruction.reference.as_ref())
    {
        write_text_element(&mut xml, "ram:PaymentReference", first);
    }
    write_text_element(&mut xml, "ram:InvoiceCurrencyCode", &currency);
    for instruction in &document.payment_instructions {
        write_payment_instruction(&mut xml, instruction);
    }
    for summary in &document.tax_summary {
        write_tax_summary(&mut xml, summary);
    }
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
    write_monetary_total(&mut xml, &document.monetary_total, &currency);
    xml.push_str("</ram:ApplicableHeaderTradeSettlement>");
    xml.push_str("</rsm:SupplyChainTradeTransaction>");
    xml.push_str("</rsm:CrossIndustryInvoice>");
    Ok(xml)
}

fn write_document_context(xml: &mut String, document: &CommercialDocument) -> Result<(), CiiError> {
    xml.push_str("<rsm:ExchangedDocumentContext>");
    for value in cii_document_field_values(document, "business_process_context_ids") {
        write_context_parameter(
            xml,
            "ram:BusinessProcessSpecifiedDocumentContextParameter",
            value,
            None,
        );
    }
    write_invoicekit_metadata_context_parameter(xml, &document.meta)?;
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

fn cii_document_field_value<'a>(document: &'a CommercialDocument, key: &str) -> Option<&'a str> {
    document
        .extensions
        .iter()
        .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|extension| extension.payload.get(key))
        .and_then(Value::as_str)
}

fn cii_document_field_values<'a>(document: &'a CommercialDocument, key: &str) -> Vec<&'a str> {
    document
        .extensions
        .iter()
        .find(|extension| extension.urn == mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|extension| extension.payload.get(key))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn profile_context_values<'a>(document: &'a CommercialDocument, key: &str) -> Vec<&'a str> {
    document
        .extensions
        .iter()
        .find(|extension| extension.urn == mapping::CII_PROFILE_CONTEXT_EXTENSION_URN)
        .and_then(|extension| extension.payload.get(key))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn json_object_array(key: &str, values: Vec<String>) -> Value {
    let mut payload = Map::new();
    payload.insert(
        key.to_owned(),
        Value::Array(values.into_iter().map(Value::String).collect()),
    );
    Value::Object(payload)
}

fn write_party(xml: &mut String, container: &str, party: &Party) -> Result<(), CiiError> {
    xml.push('<');
    xml.push_str(container);
    xml.push('>');
    if let Some(id) = &party.id {
        write_text_element(xml, "ram:ID", id);
    }
    write_text_element(xml, "ram:Name", &party.name);
    if let Some(id) = &party.id {
        xml.push_str("<ram:SpecifiedLegalOrganization>");
        write_text_element(xml, "ram:ID", id);
        xml.push_str("</ram:SpecifiedLegalOrganization>");
    }
    if let Some(contact) = &party.contact {
        if contact.name.is_some() || contact.email.is_some() || contact.phone.is_some() {
            xml.push_str("<ram:DefinedTradeContact>");
            if let Some(name) = &contact.name {
                write_text_element(xml, "ram:PersonName", name);
            }
            if let Some(phone) = &contact.phone {
                xml.push_str("<ram:TelephoneUniversalCommunication>");
                write_text_element(xml, "ram:CompleteNumber", phone);
                xml.push_str("</ram:TelephoneUniversalCommunication>");
            }
            if let Some(email) = &contact.email {
                xml.push_str("<ram:EmailURIUniversalCommunication>");
                write_text_element(xml, "ram:URIID", email);
                xml.push_str("</ram:EmailURIUniversalCommunication>");
            }
            xml.push_str("</ram:DefinedTradeContact>");
        }
    }
    write_address(xml, &party.address)?;
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
        xml.push_str("</ram:ID></ram:SpecifiedTaxRegistration>");
    }
    xml.push_str("</");
    xml.push_str(container);
    xml.push('>');
    Ok(())
}

fn write_address(xml: &mut String, address: &PostalAddress) -> Result<(), CiiError> {
    xml.push_str("<ram:PostalTradeAddress>");
    write_text_element(xml, "ram:PostcodeCode", &address.postal_code);
    if let Some(first) = address.lines.first() {
        write_text_element(xml, "ram:LineOne", first);
    }
    if let Some(second) = address.lines.get(1) {
        write_text_element(xml, "ram:LineTwo", second);
    }
    if let Some(extra_lines) = address.lines.get(2..) {
        write_text_element(xml, "ram:LineThree", &extra_lines.join(" "));
    }
    write_text_element(xml, "ram:CityName", &address.city);
    if let Some(subdivision) = &address.subdivision {
        write_text_element(xml, "ram:CountrySubDivisionName", subdivision);
    }
    write_text_element(xml, "ram:CountryID", &string_value(&address.country)?);
    xml.push_str("</ram:PostalTradeAddress>");
    Ok(())
}

fn write_payment_instruction(xml: &mut String, instruction: &PaymentInstruction) {
    xml.push_str("<ram:SpecifiedTradeSettlementPaymentMeans>");
    let code = match instruction.kind {
        PaymentInstructionKind::Sepa | PaymentInstructionKind::IbanBic => "30",
        PaymentInstructionKind::SwissQr
        | PaymentInstructionKind::EpcQr
        | PaymentInstructionKind::ZatcaQr
        | PaymentInstructionKind::Other => "1",
    };
    write_text_element(xml, "ram:TypeCode", code);
    if let Some(account) = &instruction.account {
        xml.push_str("<ram:PayeePartyCreditorFinancialAccount>");
        write_text_element(xml, "ram:IBANID", account);
        xml.push_str("</ram:PayeePartyCreditorFinancialAccount>");
    }
    xml.push_str("</ram:SpecifiedTradeSettlementPaymentMeans>");
}

fn write_tax_summary(xml: &mut String, summary: &TaxCategorySummary) {
    xml.push_str("<ram:ApplicableTradeTax>");
    write_amount_text_element(xml, "ram:CalculatedAmount", summary.tax_amount.inner());
    write_text_element(xml, "ram:TypeCode", "VAT");
    write_amount_text_element(xml, "ram:BasisAmount", summary.taxable_amount.inner());
    write_text_element(xml, "ram:CategoryCode", &summary.category_code);
    if let Some(rate) = &summary.tax_rate {
        write_text_element(xml, "ram:RateApplicablePercent", &rate.inner().to_string());
    }
    xml.push_str("</ram:ApplicableTradeTax>");
}

fn write_monetary_total(xml: &mut String, total: &MonetaryTotal, currency: &str) {
    xml.push_str("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
    write_amount_text_element(
        xml,
        "ram:LineTotalAmount",
        total.line_extension_amount.inner(),
    );
    if let Some(value) = &total.allowance_total_amount {
        write_amount_text_element(xml, "ram:AllowanceTotalAmount", value.inner());
    }
    if let Some(value) = &total.charge_total_amount {
        write_amount_text_element(xml, "ram:ChargeTotalAmount", value.inner());
    }
    write_amount_text_element(
        xml,
        "ram:TaxBasisTotalAmount",
        total.tax_exclusive_amount.inner(),
    );
    xml.push_str(r#"<ram:TaxTotalAmount currencyID=""#);
    write_xml_attr(currency, xml);
    xml.push_str(r#"">"#);
    let tax_total = total.tax_inclusive_amount.inner() - total.tax_exclusive_amount.inner();
    write_xml_text(&tax_total.to_string(), xml);
    xml.push_str("</ram:TaxTotalAmount>");
    write_amount_text_element(
        xml,
        "ram:GrandTotalAmount",
        total.tax_inclusive_amount.inner(),
    );
    if let Some(value) = &total.prepaid_amount {
        write_amount_text_element(xml, "ram:TotalPrepaidAmount", value.inner());
    }
    write_amount_text_element(xml, "ram:DuePayableAmount", total.payable_amount.inner());
    xml.push_str("</ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
}

fn write_line(xml: &mut String, line: &DocumentLine, _currency: &str) {
    xml.push_str("<ram:IncludedSupplyChainTradeLineItem>");
    xml.push_str("<ram:AssociatedDocumentLineDocument>");
    write_text_element(xml, "ram:LineID", &line.id);
    xml.push_str("</ram:AssociatedDocumentLineDocument>");
    xml.push_str("<ram:SpecifiedTradeProduct>");
    write_text_element(xml, "ram:Name", &line.description);
    xml.push_str("</ram:SpecifiedTradeProduct>");
    xml.push_str("<ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice>");
    write_amount_text_element(xml, "ram:ChargeAmount", line.unit_price.inner());
    xml.push_str("</ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement>");
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
    xml.push_str("</ram:SpecifiedLineTradeSettlement>");
    xml.push_str("</ram:IncludedSupplyChainTradeLineItem>");
}

fn write_note(xml: &mut String, note: &LocalizedString) {
    xml.push_str("<ram:IncludedNote>");
    write_text_element(xml, "ram:Content", &note.text);
    xml.push_str("</ram:IncludedNote>");
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
        let key = decode_local_name(attr.key.as_ref())?;
        if key == "xmlns" {
            continue;
        }
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

fn decode_local_name(raw: &[u8]) -> Result<String, CiiError> {
    let name = std::str::from_utf8(raw)
        .map_err(|_| CiiError::InvalidName(String::from_utf8_lossy(raw).into_owned()))?;
    Ok(name
        .split_once(':')
        .map_or(name, |(_, local_name)| local_name)
        .to_owned())
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
    use serde_json::json;

    use super::{crate_name, from_xml, mapping, to_xml, CiiError};
    use invoicekit_canonical::canonicalize_xml;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        JurisdictionExtension, LocalizedString, MonetaryTotal, Party, PartyTaxId,
        PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion,
        TaxCategorySummary,
    };
    use rust_decimal::Decimal;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-cii");
    }

    #[test]
    fn invoice_round_trip_preserves_core_ir() {
        let document = fixture(DocumentType::Invoice, 1);
        let xml = to_xml(&document).unwrap();
        let parsed = from_xml(&xml).unwrap();
        assert_eq!(parsed, document);
    }

    #[test]
    fn credit_note_round_trip_preserves_core_ir() {
        let document = fixture(DocumentType::CreditNote, 2);
        let xml = to_xml(&document).unwrap();
        let parsed = from_xml(&xml).unwrap();
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
        assert_eq!(from_xml(&xml).unwrap(), document);
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
            "<ram:BusinessProcessSpecifiedDocumentContextParameter><ram:ID>PROCESS-42</ram:ID></ram:BusinessProcessSpecifiedDocumentContextParameter><ram:BusinessProcessSpecifiedDocumentContextParameter><ram:ID>PROCESS-43</ram:ID></ram:BusinessProcessSpecifiedDocumentContextParameter>",
            &xml[application_context..],
        );
        assert!(with_business_process.contains("PROCESS-42"));
        let with_buyer_reference = with_business_process.replace(
            "<ram:SellerTradeParty>",
            "<ram:BuyerReference>BUYER-PO-7</ram:BuyerReference><ram:SellerTradeParty>",
        );

        let parsed = from_xml(&with_buyer_reference).unwrap();
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
        assert_eq!(from_xml(&xml).unwrap(), document);
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
        assert_eq!(from_xml(&xml).unwrap(), document);
    }

    #[test]
    fn mapping_decisions_name_standard_field_boundaries() {
        assert_eq!(mapping::NAMED_MAPPING_DECISIONS.len(), 4);
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
            assert_eq!(from_xml(&xml).unwrap(), document);
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
        assert_eq!(from_xml(&xml).unwrap(), document);

        let numeric_reference_xml = xml.replace("Supplier &amp; Sons", "Supplier &#x26; Sons");
        assert_eq!(from_xml(&numeric_reference_xml).unwrap(), document);
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
            let parsed = from_xml(&xml).unwrap();
            let reparsed = from_xml(&to_xml(&parsed).unwrap()).unwrap();
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
                language: "und".to_owned(),
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
