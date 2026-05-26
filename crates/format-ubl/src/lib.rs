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
    DocumentType, IrError, Iso4217Code, JurisdictionExtension, LocalizedString, MonetaryTotal,
    MoneyAmount, Party, PartyTaxId, PaymentInstruction, PaymentInstructionKind, PaymentTerms,
    PostalAddress, Quantity, SchemaVersion, TaxCategorySummary,
};
use quick_xml::events::{attributes::AttrError, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

const BEAD_ID: &str = "invoices-t-040-ubl-2-1-parser-serializer-1v2";
const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CREDIT_NOTE_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";
const UBL_CAC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
const UBL_CBC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";
const CORE_CUSTOMIZATION_ID: &str = "urn:invoicekit:ubl:2.1:core";
const CORE_PROFILE_ID: &str = "urn:invoicekit:profile:core";
const DEFAULT_LANGUAGE: &str = "und";

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
/// The parser extracts the current core IR surface. UBL elements that do not
/// have an IR field yet are accepted by the XML reader but are not represented
/// semantically in the returned [`CommercialDocument`].
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
/// let parsed = invoicekit_format_ubl::from_xml(xml).unwrap();
/// assert_eq!(parsed.document_type, invoicekit_ir::DocumentType::Invoice);
/// ```
pub fn from_xml(input: &str) -> Result<CommercialDocument, UblError> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut xml_version = XmlVersion::default();
    let mut stack = Vec::<String>::new();
    let mut state = ParseState::default();

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                state.start_element(&stack, &name, &attrs)?;
                stack.push(name);
            }
            Event::Empty(start) => {
                let name = decode_local_name(start.name().as_ref())?;
                let attrs = read_attrs(&reader, &start, xml_version)?;
                state.start_element(&stack, &name, &attrs)?;
                state.end_element(&name)?;
            }
            Event::End(end) => {
                let name = decode_local_name(end.name().as_ref())?;
                state.end_element(&name)?;
                let Some(opened) = stack.pop() else {
                    return Err(UblError::UnsupportedRoot(name));
                };
                if opened != name {
                    return Err(UblError::UnsupportedRoot(format!("{opened}/{name}")));
                }
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

    state.finish()
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
    tenant_id: Option<String>,
    trace_id: Option<String>,
    supplier: PartyBuilder,
    customer: PartyBuilder,
    payee: PartyBuilder,
    has_payee: bool,
    payment_terms_description: Option<String>,
    payment_instructions: Vec<PaymentInstruction>,
    current_payment: Option<PaymentBuilder>,
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
        stack: &[String],
        name: &str,
        attrs: &[XmlAttribute],
    ) -> Result<(), UblError> {
        if stack.is_empty() {
            self.document_type = Some(match name {
                "Invoice" => DocumentType::Invoice,
                "CreditNote" => DocumentType::CreditNote,
                other => return Err(UblError::UnsupportedRoot(other.to_owned())),
            });
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

    fn end_element(&mut self, name: &str) -> Result<(), UblError> {
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
            if let Some(payment) = self.current_payment.take().and_then(PaymentBuilder::build) {
                self.payment_instructions.push(payment);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn text(&mut self, stack: &[String], raw: &str) -> Result<(), UblError> {
        let value = raw.trim();
        if value.is_empty() {
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
            self.tenant_id = Some(value.to_owned());
        } else if is_root_child(stack, "AccountingCost") {
            self.trace_id = Some(value.to_owned());
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
        let tenant_id = self.tenant_id.unwrap_or_else(|| "ubl-import".to_owned());
        let trace_id = self
            .trace_id
            .unwrap_or_else(|| format!("{BEAD_ID}:{document_id}"));
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
            extensions: Vec::<JurisdictionExtension>::new(),
            meta: DocumentMeta {
                tenant_id,
                trace_id,
                source_system: None,
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

fn serialize_document(
    document: &CommercialDocument,
    root_name: &str,
    root_namespace: &str,
) -> Result<String, UblError> {
    let currency = string_value(&document.currency)?;
    let mut xml = String::new();
    write!(
        xml,
        r#"<{root_name} xmlns="{root_namespace}" xmlns:cac="{UBL_CAC_NAMESPACE_URI}" xmlns:cbc="{UBL_CBC_NAMESPACE_URI}">"#
    )
    .expect("writing to a String cannot fail");
    write_text_element(&mut xml, "cbc:CustomizationID", CORE_CUSTOMIZATION_ID);
    write_text_element(&mut xml, "cbc:ProfileID", CORE_PROFILE_ID);
    write_text_element(
        &mut xml,
        "cbc:ID",
        &string_value(&document.document_number)?,
    );
    write_text_element(&mut xml, "cbc:UUID", document.id.as_str());
    write_text_element(&mut xml, "cbc:IssueDate", document.issue_date.as_str());
    if let Some(date) = &document.tax_point_date {
        write_text_element(&mut xml, "cbc:TaxPointDate", date.as_str());
    }
    if document.document_type == DocumentType::Invoice {
        write_text_element(&mut xml, "cbc:InvoiceTypeCode", "380");
    } else {
        write_text_element(&mut xml, "cbc:CreditNoteTypeCode", "381");
    }
    write_text_element(&mut xml, "cbc:DocumentCurrencyCode", &currency);
    if let Some(date) = &document.due_date {
        write_text_element(&mut xml, "cbc:DueDate", date.as_str());
    }
    write_text_element(&mut xml, "cbc:BuyerReference", &document.meta.tenant_id);
    write_text_element(&mut xml, "cbc:AccountingCost", &document.meta.trace_id);
    for note in &document.notes {
        write_note(&mut xml, note);
    }
    write_party(
        &mut xml,
        "cac:AccountingSupplierParty",
        Some("cac:Party"),
        &document.supplier,
    )?;
    write_party(
        &mut xml,
        "cac:AccountingCustomerParty",
        Some("cac:Party"),
        &document.customer,
    )?;
    if let Some(payee) = &document.payee {
        write_party(&mut xml, "cac:PayeeParty", None, payee)?;
    }
    for instruction in &document.payment_instructions {
        write_payment_instruction(&mut xml, instruction);
    }
    if let Some(terms) = &document.payment_terms {
        xml.push_str("<cac:PaymentTerms>");
        write_text_element(&mut xml, "cbc:Note", &terms.description);
        xml.push_str("</cac:PaymentTerms>");
    }
    if !document.tax_summary.is_empty() {
        write_tax_total(&mut xml, &document.tax_summary, &currency);
    }
    write_monetary_total(&mut xml, &document.monetary_total, &currency);
    for line in &document.lines {
        write_line(&mut xml, document.document_type, line, &currency);
    }
    write!(xml, "</{root_name}>").expect("writing to a String cannot fail");
    Ok(xml)
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
    for tax_id in &party.tax_ids {
        xml.push_str("<cac:PartyTaxScheme>");
        write_text_element(xml, "cbc:CompanyID", &tax_id.value);
        xml.push_str("<cac:TaxScheme>");
        write_text_element(xml, "cbc:ID", &tax_id.scheme);
        xml.push_str("</cac:TaxScheme>");
        xml.push_str("</cac:PartyTaxScheme>");
    }
    write_address(xml, &party.address)?;
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
        xml.push_str("</cac:TaxCategory>");
        xml.push_str("</cac:TaxSubtotal>");
    }
    xml.push_str("</cac:TaxTotal>");
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

fn read_attrs(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
) -> Result<Vec<XmlAttribute>, UblError> {
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

fn decode_local_name(raw: &[u8]) -> Result<String, UblError> {
    let name = std::str::from_utf8(raw)
        .map_err(|_| UblError::InvalidName(String::from_utf8_lossy(raw).into_owned()))?;
    Ok(name
        .split_once(':')
        .map_or(name, |(_, local_name)| local_name)
        .to_owned())
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

fn is_root_child(stack: &[String], child: &str) -> bool {
    stack.len() == 2 && stack.last().is_some_and(|name| name == child)
}

fn party_role(stack: &[String]) -> Option<PartyRole> {
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{crate_name, from_xml, to_xml, UblError};
    use invoicekit_canonical::canonicalize_xml;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        LocalizedString, MonetaryTotal, Party, PartyTaxId, PaymentInstruction,
        PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion, TaxCategorySummary,
    };
    use rust_decimal::Decimal;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-ubl");
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
        let supplier = party("supplier", "Supplier GmbH", "DE123456789");
        let customer = party("customer", "Customer BV", "NL123456789B01");

        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(format!("doc-{seed}")).unwrap(),
            document_type,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: Some(DateOnly::new("2026-05-26").unwrap()),
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            document_number: DocumentNumber::new(format!("INV-{seed:04}")).unwrap(),
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
