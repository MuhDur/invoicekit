//! `invoicekit-ir` - layered invoice data model.
//!
//! The IR is the Rust source of truth for the InvoiceKit commercial document
//! model. It deliberately keeps global commercial invoice semantics separate
//! from profile or country-specific extension data.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical schema version carried by every serialized document.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SchemaVersion {
    /// Initial public IR version.
    #[serde(rename = "1.0")]
    #[default]
    V1_0,
}

/// Top-level document type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentType {
    /// Commercial invoice.
    Invoice,
    /// Credit note.
    CreditNote,
    /// Debit note.
    DebitNote,
    /// Pro forma invoice.
    ProForma,
    /// Self-billed invoice.
    SelfBilled,
}

/// Stable identifier for a commercial document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DocumentId(String);

impl DocumentId {
    /// Builds a non-empty document identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::MissingRequiredField`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(non_empty(value, "id")?))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.0, "id")
    }
}

/// Human or tenant-visible invoice number.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DocumentNumber(String);

impl DocumentNumber {
    /// Builds a non-empty document number.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::MissingRequiredField`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(non_empty(value, "document_number")?))
    }

    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.0, "document_number")
    }
}

/// ISO 8601 calendar date without a time component.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DateOnly(String);

impl DateOnly {
    /// Builds a validated `YYYY-MM-DD` date.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidDate`] when the value is not a valid calendar
    /// date in `YYYY-MM-DD` form.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        let value = value.into();
        if is_valid_date(&value) {
            Ok(Self(value))
        } else {
            Err(IrError::InvalidDate(value))
        }
    }

    /// Returns the date as serialized text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), IrError> {
        if is_valid_date(&self.0) {
            Ok(())
        } else {
            Err(IrError::InvalidDate(self.0.clone()))
        }
    }
}

/// ISO 4217 currency code.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Iso4217Code(String);

impl Iso4217Code {
    /// Builds a validated three-letter uppercase currency code.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidCurrency`] when the code is not three
    /// uppercase ASCII letters.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        let value = value.into();
        if is_upper_ascii_code(&value, 3) {
            Ok(Self(value))
        } else {
            Err(IrError::InvalidCurrency(value))
        }
    }

    fn validate(&self) -> Result<(), IrError> {
        if is_upper_ascii_code(&self.0, 3) {
            Ok(())
        } else {
            Err(IrError::InvalidCurrency(self.0.clone()))
        }
    }
}

/// ISO 3166-1 alpha-2 country code.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CountryCode(String);

impl CountryCode {
    /// Builds a validated two-letter uppercase country code.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidCountryCode`] when the code is not two
    /// uppercase ASCII letters.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        let value = value.into();
        if is_upper_ascii_code(&value, 2) {
            Ok(Self(value))
        } else {
            Err(IrError::InvalidCountryCode(value))
        }
    }

    fn validate(&self) -> Result<(), IrError> {
        if is_upper_ascii_code(&self.0, 2) {
            Ok(())
        } else {
            Err(IrError::InvalidCountryCode(self.0.clone()))
        }
    }
}

/// Decimal value serialized as a fixed decimal string at the API boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DecimalValue(#[serde(with = "rust_decimal::serde::str")] Decimal);

impl DecimalValue {
    /// Wraps a decimal value.
    #[must_use]
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// Returns the underlying decimal.
    #[must_use]
    pub const fn inner(&self) -> Decimal {
        self.0
    }
}

/// Monetary amount serialized as a decimal string.
pub type MoneyAmount = DecimalValue;

/// Quantity serialized as a decimal string.
pub type Quantity = DecimalValue;

/// Localized human text.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalizedString {
    /// BCP 47 language tag.
    pub language: String,
    /// Localized text value.
    pub text: String,
}

impl LocalizedString {
    fn validate(&self, field: &'static str) -> Result<(), IrError> {
        validate_non_empty(&self.language, field)?;
        validate_non_empty(&self.text, field)
    }
}

/// Party tax identifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PartyTaxId {
    /// Identifier scheme, such as `vat`.
    pub scheme: String,
    /// Identifier value.
    pub value: String,
}

impl PartyTaxId {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.scheme, "party.tax_ids.scheme")?;
        validate_non_empty(&self.value, "party.tax_ids.value")
    }
}

/// Postal address for a supplier, customer, payee, or other party.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PostalAddress {
    /// Address lines in display order.
    pub lines: Vec<String>,
    /// Locality or city.
    pub city: String,
    /// Optional subdivision, state, province, or region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subdivision: Option<String>,
    /// Postal code.
    pub postal_code: String,
    /// Country code.
    pub country: CountryCode,
}

impl PostalAddress {
    fn validate(&self) -> Result<(), IrError> {
        if self.lines.is_empty() {
            return Err(IrError::EmptyCollection("party.address.lines"));
        }
        for line in &self.lines {
            validate_non_empty(line, "party.address.lines")?;
        }
        validate_non_empty(&self.city, "party.address.city")?;
        validate_non_empty(&self.postal_code, "party.address.postal_code")?;
        self.country.validate()
    }
}

/// Contact details for a party.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Contact {
    /// Optional contact name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional email address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Optional telephone number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
}

/// Commercial party.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Party {
    /// Optional stable party identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Legal or trading name.
    pub name: String,
    /// Tax identifiers carried by the party.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tax_ids: Vec<PartyTaxId>,
    /// Postal address.
    pub address: PostalAddress,
    /// Optional contact details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Contact>,
}

impl Party {
    fn validate(&self, field: &'static str) -> Result<(), IrError> {
        validate_non_empty(&self.name, field)?;
        for tax_id in &self.tax_ids {
            tax_id.validate()?;
        }
        self.address.validate()
    }
}

/// Payment terms attached to the document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaymentTerms {
    /// Human-readable payment terms.
    pub description: String,
    /// Optional due date stated in the terms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<DateOnly>,
}

impl PaymentTerms {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.description, "payment_terms.description")?;
        if let Some(date) = &self.due_date {
            date.validate()?;
        }
        Ok(())
    }
}

/// Payment rail or instruction kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentInstructionKind {
    /// SEPA credit transfer.
    Sepa,
    /// IBAN or BIC transfer.
    IbanBic,
    /// Swiss QR payment.
    SwissQr,
    /// EPC QR payment.
    EpcQr,
    /// Saudi ZATCA QR payload.
    ZatcaQr,
    /// Other instruction kind described by the `account` or `reference`.
    Other,
}

/// Payment instruction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaymentInstruction {
    /// Instruction kind.
    pub kind: PaymentInstructionKind,
    /// Optional account or payment address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Optional payment reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// Polymorphic jurisdiction or profile extension payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JurisdictionExtension {
    /// Uniform resource name for the extension schema.
    pub urn: String,
    /// Extension payload validated by the country or profile registry.
    pub payload: Value,
}

impl JurisdictionExtension {
    /// Builds a polymorphic extension.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidExtensionUrn`] when `urn` is not a non-empty
    /// URN and [`IrError::InvalidExtensionPayload`] when `payload` is null.
    pub fn new(urn: impl Into<String>, payload: Value) -> Result<Self, IrError> {
        let extension = Self {
            urn: urn.into(),
            payload,
        };
        extension.validate()?;
        Ok(extension)
    }

    /// Validates the extension envelope, without validating the country schema.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] for invalid URNs or null payloads.
    pub fn validate(&self) -> Result<(), IrError> {
        if !self.urn.starts_with("urn:") || self.urn.trim().len() <= "urn:".len() {
            return Err(IrError::InvalidExtensionUrn(self.urn.clone()));
        }
        if self.payload.is_null() {
            return Err(IrError::InvalidExtensionPayload(self.urn.clone()));
        }
        Ok(())
    }
}

/// Document line.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentLine {
    /// Line identifier.
    pub id: String,
    /// Line description.
    pub description: String,
    /// Invoiced quantity.
    pub quantity: Quantity,
    /// Optional unit code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_code: Option<String>,
    /// Unit price amount.
    pub unit_price: MoneyAmount,
    /// Line extension amount.
    pub line_extension_amount: MoneyAmount,
    /// Optional tax category code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_category: Option<String>,
    /// Line-level jurisdiction extensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<JurisdictionExtension>,
}

impl DocumentLine {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.id, "lines.id")?;
        validate_non_empty(&self.description, "lines.description")?;
        for extension in &self.extensions {
            extension.validate()?;
        }
        Ok(())
    }
}

/// Tax category summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaxCategorySummary {
    /// Tax category code.
    pub category_code: String,
    /// Taxable amount.
    pub taxable_amount: MoneyAmount,
    /// Tax amount.
    pub tax_amount: MoneyAmount,
    /// Optional tax rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_rate: Option<DecimalValue>,
}

impl TaxCategorySummary {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.category_code, "tax_summary.category_code")
    }
}

/// Document monetary totals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
// The `_amount` suffix matches invoice business terms and serialized field names.
#[allow(clippy::struct_field_names)]
pub struct MonetaryTotal {
    /// Sum of line extension amounts.
    pub line_extension_amount: MoneyAmount,
    /// Tax-exclusive amount.
    pub tax_exclusive_amount: MoneyAmount,
    /// Tax-inclusive amount.
    pub tax_inclusive_amount: MoneyAmount,
    /// Optional allowance total.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowance_total_amount: Option<MoneyAmount>,
    /// Optional charge total.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_total_amount: Option<MoneyAmount>,
    /// Optional prepaid amount.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepaid_amount: Option<MoneyAmount>,
    /// Payable amount.
    pub payable_amount: MoneyAmount,
}

/// Content-addressed attachment reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attachment {
    /// Attachment role or semantic type.
    pub kind: String,
    /// Content digest.
    pub digest: String,
    /// Media type.
    pub media_type: String,
}

impl Attachment {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.kind, "attachments.kind")?;
        validate_non_empty(&self.digest, "attachments.digest")?;
        validate_non_empty(&self.media_type, "attachments.media_type")
    }
}

/// Reference to another commercial document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentReference {
    /// Reference type, such as purchase order.
    pub kind: String,
    /// Referenced identifier.
    pub id: String,
    /// Optional referenced issue date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_date: Option<DateOnly>,
}

impl DocumentReference {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.kind, "references.kind")?;
        validate_non_empty(&self.id, "references.id")?;
        if let Some(date) = &self.issue_date {
            date.validate()?;
        }
        Ok(())
    }
}

/// Operational metadata for a document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentMeta {
    /// Tenant identifier.
    pub tenant_id: String,
    /// Trace identifier for audit correlation.
    pub trace_id: String,
    /// Optional source system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_system: Option<String>,
}

impl DocumentMeta {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.tenant_id, "meta.tenant_id")?;
        validate_non_empty(&self.trace_id, "meta.trace_id")
    }
}

/// Input parts for constructing a [`CommercialDocument`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommercialDocumentParts {
    /// Schema version.
    #[serde(default)]
    pub schema_version: SchemaVersion,
    /// Stable document identifier.
    pub id: DocumentId,
    /// Document type.
    pub document_type: DocumentType,
    /// Issue date.
    pub issue_date: DateOnly,
    /// Optional tax point date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_point_date: Option<DateOnly>,
    /// Optional due date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<DateOnly>,
    /// Document number.
    pub document_number: DocumentNumber,
    /// Document currency.
    pub currency: Iso4217Code,
    /// Supplier party.
    pub supplier: Party,
    /// Customer party.
    pub customer: Party,
    /// Optional payee party.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<Party>,
    /// Optional payment terms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<PaymentTerms>,
    /// Payment instructions.
    #[serde(default)]
    pub payment_instructions: Vec<PaymentInstruction>,
    /// Document lines.
    pub lines: Vec<DocumentLine>,
    /// Tax summaries.
    #[serde(default)]
    pub tax_summary: Vec<TaxCategorySummary>,
    /// Monetary total.
    pub monetary_total: MonetaryTotal,
    /// Content-addressed attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Commercial document references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<DocumentReference>,
    /// Human-readable notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<LocalizedString>,
    /// Jurisdiction extension envelopes.
    #[serde(default)]
    pub extensions: Vec<JurisdictionExtension>,
    /// Operational metadata.
    pub meta: DocumentMeta,
}

/// Jurisdiction-agnostic commercial invoice or credit note semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommercialDocument {
    /// Schema version.
    #[serde(default)]
    pub schema_version: SchemaVersion,
    /// Stable document identifier.
    pub id: DocumentId,
    /// Document type.
    pub document_type: DocumentType,
    /// Issue date.
    pub issue_date: DateOnly,
    /// Optional tax point date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_point_date: Option<DateOnly>,
    /// Optional due date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<DateOnly>,
    /// Document number.
    pub document_number: DocumentNumber,
    /// Document currency.
    pub currency: Iso4217Code,
    /// Supplier party.
    pub supplier: Party,
    /// Customer party.
    pub customer: Party,
    /// Optional payee party.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<Party>,
    /// Optional payment terms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<PaymentTerms>,
    /// Payment instructions.
    #[serde(default)]
    pub payment_instructions: Vec<PaymentInstruction>,
    /// Document lines.
    pub lines: Vec<DocumentLine>,
    /// Tax summaries.
    #[serde(default)]
    pub tax_summary: Vec<TaxCategorySummary>,
    /// Monetary total.
    pub monetary_total: MonetaryTotal,
    /// Content-addressed attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Commercial document references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<DocumentReference>,
    /// Human-readable notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<LocalizedString>,
    /// Jurisdiction extension envelopes.
    #[serde(default)]
    pub extensions: Vec<JurisdictionExtension>,
    /// Operational metadata.
    pub meta: DocumentMeta,
}

impl CommercialDocument {
    /// Builds and validates a commercial document.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] when required fields are blank, shape checks fail,
    /// or nested extensions are invalid.
    pub fn new(parts: CommercialDocumentParts) -> Result<Self, IrError> {
        let document = Self {
            schema_version: parts.schema_version,
            id: parts.id,
            document_type: parts.document_type,
            issue_date: parts.issue_date,
            tax_point_date: parts.tax_point_date,
            due_date: parts.due_date,
            document_number: parts.document_number,
            currency: parts.currency,
            supplier: parts.supplier,
            customer: parts.customer,
            payee: parts.payee,
            payment_terms: parts.payment_terms,
            payment_instructions: parts.payment_instructions,
            lines: parts.lines,
            tax_summary: parts.tax_summary,
            monetary_total: parts.monetary_total,
            attachments: parts.attachments,
            references: parts.references,
            notes: parts.notes,
            extensions: parts.extensions,
            meta: parts.meta,
        };
        document.validate()?;
        Ok(document)
    }

    /// Deserializes a JSON value and validates the resulting document.
    ///
    /// # Errors
    ///
    /// Returns JSON decoding errors or validation errors.
    pub fn try_from_value(value: Value) -> Result<Self, IrError> {
        let document: Self = serde_json::from_value(value)?;
        document.validate()?;
        Ok(document)
    }

    /// Serializes the document into JSON.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization errors.
    pub fn to_value(&self) -> Result<Value, IrError> {
        Ok(serde_json::to_value(self)?)
    }

    /// Validates the document envelope and nested value objects.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] for invalid field shapes.
    pub fn validate(&self) -> Result<(), IrError> {
        self.id.validate()?;
        self.issue_date.validate()?;
        if let Some(date) = &self.tax_point_date {
            date.validate()?;
        }
        if let Some(date) = &self.due_date {
            date.validate()?;
        }
        self.document_number.validate()?;
        self.currency.validate()?;
        self.supplier.validate("supplier.name")?;
        self.customer.validate("customer.name")?;
        if let Some(payee) = &self.payee {
            payee.validate("payee.name")?;
        }
        if let Some(terms) = &self.payment_terms {
            terms.validate()?;
        }
        if self.lines.is_empty() {
            return Err(IrError::EmptyCollection("lines"));
        }
        for line in &self.lines {
            line.validate()?;
        }
        for summary in &self.tax_summary {
            summary.validate()?;
        }
        for attachment in &self.attachments {
            attachment.validate()?;
        }
        for reference in &self.references {
            reference.validate()?;
        }
        for note in &self.notes {
            note.validate("notes")?;
        }
        for extension in &self.extensions {
            extension.validate()?;
        }
        self.meta.validate()
    }
}

/// Profile identifier and version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProfileIdentifier {
    /// Profile URN.
    pub urn: String,
    /// Profile version.
    pub version: String,
}

impl ProfileIdentifier {
    fn validate(&self) -> Result<(), IrError> {
        if !self.urn.starts_with("urn:") {
            return Err(IrError::InvalidProfileUrn(self.urn.clone()));
        }
        validate_non_empty(&self.version, "profile.version")
    }
}

/// Projection of a commercial document onto a standard or country profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProfileView {
    /// Target profile.
    pub profile: ProfileIdentifier,
    /// Source commercial document identifier.
    pub source_document_id: DocumentId,
    /// Effective validation date.
    pub effective_date: DateOnly,
    /// Profile-specific extension envelopes.
    #[serde(default)]
    pub extensions: Vec<JurisdictionExtension>,
    /// Lossiness ledger produced by the projection.
    pub lossiness: LossinessLedger,
}

impl ProfileView {
    /// Builds and validates a profile view.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] for invalid profile, date, document id, extension,
    /// or lossiness data.
    pub fn new(
        profile: ProfileIdentifier,
        source_document_id: DocumentId,
        effective_date: DateOnly,
        extensions: Vec<JurisdictionExtension>,
        lossiness: LossinessLedger,
    ) -> Result<Self, IrError> {
        let view = Self {
            profile,
            source_document_id,
            effective_date,
            extensions,
            lossiness,
        };
        view.validate()?;
        Ok(view)
    }

    /// Validates the profile view envelope.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] for invalid nested values.
    pub fn validate(&self) -> Result<(), IrError> {
        self.profile.validate()?;
        self.source_document_id.validate()?;
        self.effective_date.validate()?;
        for extension in &self.extensions {
            extension.validate()?;
        }
        self.lossiness.validate()
    }
}

/// Field-level projection outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LossinessEntry {
    /// JSON Pointer-like source field path.
    pub path: String,
    /// Human-readable reason or note.
    pub reason: String,
}

impl LossinessEntry {
    fn validate(&self, field: &'static str) -> Result<(), IrError> {
        validate_non_empty(&self.path, field)?;
        validate_non_empty(&self.reason, field)
    }
}

/// Structured record of data preserved or lost during a profile projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LossinessLedger {
    /// Fields preserved by the projection.
    #[serde(default)]
    pub preserved: Vec<LossinessEntry>,
    /// Fields lost by the projection.
    #[serde(default)]
    pub lost: Vec<LossinessEntry>,
    /// Non-fatal projection warnings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl LossinessLedger {
    /// Builds a lossiness ledger.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] when an entry has a blank path or reason.
    pub fn new(preserved: Vec<LossinessEntry>, lost: Vec<LossinessEntry>) -> Result<Self, IrError> {
        let ledger = Self {
            preserved,
            lost,
            warnings: Vec::new(),
        };
        ledger.validate()?;
        Ok(ledger)
    }

    /// Validates ledger entries.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] when an entry has a blank path or reason.
    pub fn validate(&self) -> Result<(), IrError> {
        for entry in &self.preserved {
            entry.validate("lossiness.preserved")?;
        }
        for entry in &self.lost {
            entry.validate("lossiness.lost")?;
        }
        Ok(())
    }
}

/// Errors produced by the IR constructors and validators.
#[derive(Debug, Error)]
pub enum IrError {
    /// A required string field was blank.
    #[error("missing required field `{0}`")]
    MissingRequiredField(&'static str),
    /// A required collection was empty.
    #[error("collection `{0}` must not be empty")]
    EmptyCollection(&'static str),
    /// A date was not a valid `YYYY-MM-DD` calendar date.
    #[error("invalid date `{0}`; expected YYYY-MM-DD")]
    InvalidDate(String),
    /// A currency code was invalid.
    #[error("invalid ISO 4217 currency code `{0}`")]
    InvalidCurrency(String),
    /// A country code was invalid.
    #[error("invalid ISO 3166 country code `{0}`")]
    InvalidCountryCode(String),
    /// An extension URN was invalid.
    #[error("invalid jurisdiction extension URN `{0}`")]
    InvalidExtensionUrn(String),
    /// An extension payload was invalid.
    #[error("invalid jurisdiction extension payload for `{0}`")]
    InvalidExtensionPayload(String),
    /// A profile URN was invalid.
    #[error("invalid profile URN `{0}`")]
    InvalidProfileUrn(String),
    /// JSON encoding or decoding failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_ir::crate_name(), "invoicekit-ir");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-ir"
}

fn non_empty(value: impl Into<String>, field: &'static str) -> Result<String, IrError> {
    let value = value.into();
    validate_non_empty(&value, field)?;
    Ok(value)
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), IrError> {
    if value.trim().is_empty() {
        Err(IrError::MissingRequiredField(field))
    } else {
        Ok(())
    }
}

fn is_upper_ascii_code(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn is_valid_date(value: &str) -> bool {
    if value.len() != 10 {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 4 && *index != 7)
        .all(|(_, byte)| byte.is_ascii_digit())
    {
        return false;
    }

    let Some(year) = value.get(0..4).and_then(|part| part.parse::<u16>().ok()) else {
        return false;
    };
    let Some(month) = value.get(5..7).and_then(|part| part.parse::<u8>().ok()) else {
        return false;
    };
    let Some(day) = value.get(8..10).and_then(|part| part.parse::<u8>().ok()) else {
        return false;
    };

    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    (1..=days_in_month(year, month)).contains(&day)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::{
        crate_name, CommercialDocument, CommercialDocumentParts, DateOnly, DocumentId, IrError,
        JurisdictionExtension, LossinessEntry, LossinessLedger, ProfileIdentifier, ProfileView,
    };
    use serde_json::{json, Value};

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-ir");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn commercial_document_round_trips_through_json_value() {
        let input = synthetic_document_json();
        let document = CommercialDocument::try_from_value(input.clone()).unwrap();
        let output = document.to_value().unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn commercial_document_constructor_validates_typed_parts() {
        let parts: CommercialDocumentParts =
            serde_json::from_value(synthetic_document_json()).unwrap();
        let document = CommercialDocument::new(parts).unwrap();
        assert_eq!(document.id.as_str(), "doc_2026_0001");
    }

    #[test]
    fn amount_boundaries_are_decimal_strings() {
        let document = CommercialDocument::try_from_value(synthetic_document_json()).unwrap();
        let output = document.to_value().unwrap();
        assert_eq!(
            output.pointer("/monetary_total/payable_amount"),
            Some(&json!("119.00"))
        );
        assert_eq!(document.monetary_total.payable_amount.inner().scale(), 2);
    }

    #[test]
    fn invalid_date_is_rejected() {
        let err = DateOnly::new("2026-02-29").unwrap_err();
        assert!(matches!(err, IrError::InvalidDate(_)));

        let mut input = synthetic_document_json();
        input["issue_date"] = json!("2025-13-01");
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(err, IrError::InvalidDate(_)));
    }

    #[test]
    fn invalid_currency_is_rejected() {
        let mut input = synthetic_document_json();
        input["currency"] = json!("usd");
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(err, IrError::InvalidCurrency(_)));
    }

    #[test]
    fn empty_lines_are_rejected() {
        let mut input = synthetic_document_json();
        input["lines"] = json!([]);
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(err, IrError::EmptyCollection("lines")));
    }

    #[test]
    fn jurisdiction_extension_remains_polymorphic() {
        let extension =
            JurisdictionExtension::new("urn:invoicekit:ext:sa:zatca:2.0", json!({"qr": "encoded"}))
                .unwrap();
        assert_eq!(extension.urn, "urn:invoicekit:ext:sa:zatca:2.0");
        assert_eq!(extension.payload["qr"], "encoded");

        let err = JurisdictionExtension::new("sa-zatca", json!({})).unwrap_err();
        assert!(matches!(err, IrError::InvalidExtensionUrn(_)));
    }

    #[test]
    fn profile_view_carries_lossiness_ledger() {
        let ledger = LossinessLedger::new(
            vec![LossinessEntry {
                path: "/document_number".to_owned(),
                reason: "mapped exactly".to_owned(),
            }],
            vec![LossinessEntry {
                path: "/extensions/0/payload/internal_note".to_owned(),
                reason: "target profile has no equivalent field".to_owned(),
            }],
        )
        .unwrap();

        let view = ProfileView::new(
            ProfileIdentifier {
                urn: "urn:invoicekit:profile:peppol:bis:3".to_owned(),
                version: "3.0".to_owned(),
            },
            DocumentId::new("doc_2026_0001").unwrap(),
            DateOnly::new("2026-05-26").unwrap(),
            Vec::new(),
            ledger,
        )
        .unwrap();

        assert_eq!(view.lossiness.preserved.len(), 1);
        assert_eq!(view.lossiness.lost.len(), 1);
    }

    fn synthetic_document_json() -> Value {
        json!({
            "schema_version": "1.0",
            "id": "doc_2026_0001",
            "document_type": "invoice",
            "issue_date": "2026-05-26",
            "due_date": "2026-06-25",
            "document_number": "INV-2026-0001",
            "currency": "EUR",
            "supplier": party_json("supplier-1", "InvoiceKit GmbH", "DE"),
            "customer": party_json("customer-1", "ACME SAS", "FR"),
            "payment_terms": {
                "description": "30 days net",
                "due_date": "2026-06-25"
            },
            "payment_instructions": [{
                "kind": "iban_bic",
                "account": "DE02100100100006820101",
                "reference": "INV-2026-0001"
            }],
            "lines": [{
                "id": "1",
                "description": "Validation subscription",
                "quantity": "1",
                "unit_code": "EA",
                "unit_price": "100.00",
                "line_extension_amount": "100.00",
                "tax_category": "S",
                "extensions": [{
                    "urn": "urn:invoicekit:ext:line:test:1.0",
                    "payload": {"source": "synthetic"}
                }]
            }],
            "tax_summary": [{
                "category_code": "S",
                "taxable_amount": "100.00",
                "tax_amount": "19.00",
                "tax_rate": "19.00"
            }],
            "monetary_total": {
                "line_extension_amount": "100.00",
                "tax_exclusive_amount": "100.00",
                "tax_inclusive_amount": "119.00",
                "payable_amount": "119.00"
            },
            "references": [{
                "kind": "purchase_order",
                "id": "PO-42",
                "issue_date": "2026-05-01"
            }],
            "notes": [{
                "language": "en",
                "text": "Synthetic fixture."
            }],
            "extensions": [{
                "urn": "urn:invoicekit:ext:generic:test:1.0",
                "payload": {"profile_hint": "peppol-bis"}
            }],
            "meta": {
                "tenant_id": "tenant_123",
                "trace_id": "trace_abc",
                "source_system": "unit-test"
            }
        })
    }

    fn party_json(id: &str, name: &str, country: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "tax_ids": [{
                "scheme": "vat",
                "value": format!("{country}123456789")
            }],
            "address": {
                "lines": ["Main Street 1"],
                "city": "Sample City",
                "postal_code": "10115",
                "country": country
            },
            "contact": {
                "email": "billing@example.invalid"
            }
        })
    }
}
