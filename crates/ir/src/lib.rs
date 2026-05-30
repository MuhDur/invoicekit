// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-ir` - layered invoice data model.
//!
//! The IR is the Rust source of truth for the InvoiceKit commercial document
//! model. It deliberately keeps global commercial invoice semantics separate
//! from profile or country-specific extension data.

use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical schema version carried by every serialized document.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub enum SchemaVersion {
    /// Initial public IR version.
    #[serde(rename = "1.0")]
    #[default]
    V1_0,
}

/// Top-level document type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct DocumentId(String);

impl DocumentId {
    /// Builds a non-empty document identifier.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::MissingRequiredField`] when `value` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{DocumentId, IrError};
    ///
    /// let id = DocumentId::new("doc-2026-0001")?;
    /// assert_eq!(id.as_str(), "doc-2026-0001");
    ///
    /// assert!(matches!(
    ///     DocumentId::new(""),
    ///     Err(IrError::MissingRequiredField("id"))
    /// ));
    /// # Ok::<(), IrError>(())
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(non_empty(value, "id")?))
    }

    /// Returns the identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// let id = invoicekit_ir::DocumentId::new("doc-2026-0001").unwrap();
    /// assert_eq!(id.as_str(), "doc-2026-0001");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.0, "id")
    }
}

/// Human or tenant-visible invoice number.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct DocumentNumber(String);

impl DocumentNumber {
    /// Builds a non-empty document number.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::MissingRequiredField`] when `value` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{DocumentNumber, IrError};
    ///
    /// let n = DocumentNumber::new("INV-2026-0001")?;
    /// assert_eq!(n.as_str(), "INV-2026-0001");
    /// # Ok::<(), IrError>(())
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(non_empty(value, "document_number")?))
    }

    /// Returns the document number as serialized text.
    ///
    /// # Examples
    ///
    /// ```
    /// let number = invoicekit_ir::DocumentNumber::new("INV-2026-0001").unwrap();
    /// assert_eq!(number.as_str(), "INV-2026-0001");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.0, "document_number")
    }
}

/// ISO 8601 calendar date without a time component.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct DateOnly(String);

impl DateOnly {
    /// Builds a validated `YYYY-MM-DD` date.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidDate`] when the value is not a valid calendar
    /// date in `YYYY-MM-DD` form.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{DateOnly, IrError};
    ///
    /// let d = DateOnly::new("2026-05-27")?;
    /// assert_eq!(d.as_str(), "2026-05-27");
    ///
    /// assert!(matches!(DateOnly::new("2026-13-01"), Err(IrError::InvalidDate(_))));
    /// assert!(matches!(DateOnly::new("not-a-date"), Err(IrError::InvalidDate(_))));
    /// # Ok::<(), IrError>(())
    /// ```
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Iso4217Code(String);

impl Iso4217Code {
    /// Builds a validated three-letter uppercase currency code.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidCurrency`] when the code is not three
    /// uppercase ASCII letters.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{Iso4217Code, IrError};
    ///
    /// let eur = Iso4217Code::new("EUR")?;
    /// assert_eq!(eur.as_str(), "EUR");
    ///
    /// assert!(matches!(Iso4217Code::new("eur"), Err(IrError::InvalidCurrency(_))));
    /// assert!(matches!(Iso4217Code::new("EURO"), Err(IrError::InvalidCurrency(_))));
    /// # Ok::<(), IrError>(())
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        let value = value.into();
        if is_upper_ascii_code(&value, 3) {
            Ok(Self(value))
        } else {
            Err(IrError::InvalidCurrency(value))
        }
    }

    /// Returns the currency code as serialized text.
    ///
    /// # Examples
    ///
    /// ```
    /// let currency = invoicekit_ir::Iso4217Code::new("EUR").unwrap();
    /// assert_eq!(currency.as_str(), "EUR");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct CountryCode(String);

impl CountryCode {
    /// Builds a validated two-letter uppercase country code.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidCountryCode`] when the code is not two
    /// uppercase ASCII letters.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{CountryCode, IrError};
    ///
    /// assert!(CountryCode::new("DE").is_ok());
    /// assert!(matches!(CountryCode::new("de"), Err(IrError::InvalidCountryCode(_))));
    /// assert!(matches!(CountryCode::new("DEU"), Err(IrError::InvalidCountryCode(_))));
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        let value = value.into();
        if is_upper_ascii_code(&value, 2) {
            Ok(Self(value))
        } else {
            Err(IrError::InvalidCountryCode(value))
        }
    }

    /// Returns the country code as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct DecimalValue(
    #[serde(with = "rust_decimal::serde::str")]
    #[schemars(with = "String")]
    Decimal,
);

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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
///
/// # URN scheme casing (ebaq)
///
/// RFC 8141 declares the URN scheme name case-insensitive. Real producers
/// in the wild (notably some legacy XML exporters) emit `URN:` or `Urn:`.
/// Both [`JurisdictionExtension::new`] and the [`Deserialize`] implementation
/// accept any casing of the scheme prefix and normalise it to the canonical
/// lowercase `urn:` so equality checks remain stable.
///
/// The namespace identifier and namespace-specific string are preserved
/// verbatim. Rule-pack URN registries stay as shipped — Peppol's UNCL1001
/// codes are lowercase by definition, ZUGFeRD profile URNs are mixed case,
/// and changing those bytes would break canonical signing payloads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct JurisdictionExtension {
    /// Uniform resource name for the extension schema (canonical lowercase
    /// `urn:` prefix).
    pub urn: String,
    /// Extension payload validated by the country or profile registry.
    pub payload: Value,
}

impl<'de> Deserialize<'de> for JurisdictionExtension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            urn: String,
            payload: Value,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            urn: Self::canonicalise_urn(raw.urn),
            payload: raw.payload,
        })
    }
}

impl JurisdictionExtension {
    /// Builds a polymorphic extension. Normalises the URN scheme prefix to
    /// the canonical lowercase `urn:` per RFC 8141.
    ///
    /// # Errors
    ///
    /// Returns [`IrError::InvalidExtensionUrn`] when `urn` is not a non-empty
    /// URN and [`IrError::InvalidExtensionPayload`] when `payload` is null.
    pub fn new(urn: impl Into<String>, payload: Value) -> Result<Self, IrError> {
        let extension = Self {
            urn: Self::canonicalise_urn(urn.into()),
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
        let trimmed = self.urn.trim();
        if !Self::has_urn_scheme(trimmed) || trimmed.len() <= "urn:".len() {
            return Err(IrError::InvalidExtensionUrn(self.urn.clone()));
        }
        if self.payload.is_null() {
            return Err(IrError::InvalidExtensionPayload(self.urn.clone()));
        }
        Ok(())
    }

    /// True when `value` starts with the URN scheme prefix in any casing.
    fn has_urn_scheme(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() >= 4
            && bytes[0].eq_ignore_ascii_case(&b'u')
            && bytes[1].eq_ignore_ascii_case(&b'r')
            && bytes[2].eq_ignore_ascii_case(&b'n')
            && bytes[3] == b':'
    }

    /// Trims surrounding whitespace, lower-cases the `urn:` scheme prefix, and
    /// leaves everything after the first colon untouched. Returns the trimmed
    /// `urn` unchanged when it does not begin with the URN scheme (validation
    /// handles rejection downstream).
    ///
    /// Trimming happens first so the scheme check sees the same bytes
    /// [`Self::validate`] does; otherwise a whitespace-padded uppercase scheme
    /// would be stored un-canonicalised and break the [`Eq`] invariant.
    fn canonicalise_urn(urn: String) -> String {
        let urn = if urn.trim().len() == urn.len() {
            urn
        } else {
            urn.trim().to_owned()
        };
        if Self::has_urn_scheme(&urn) && urn.as_bytes()[..3].iter().any(u8::is_ascii_uppercase) {
            let mut canonical = String::with_capacity(urn.len());
            canonical.push_str("urn:");
            canonical.push_str(&urn[4..]);
            canonical
        } else {
            urn
        }
    }
}

/// Commodity or service classification of a line item.
///
/// Models EN 16931 BT-158 *Item classification identifier* together with its
/// scheme identifier (BT-158-1) and optional scheme version (BT-158-2). The
/// `scheme_id` is an open list (e.g. UNTDID 7143 codes such as `HS`/`SRV`, or a
/// national scheme name like `HSN`, `SAC`, `NCM`, `ClaveProdServ`, `UNSPSC`),
/// mirroring the open `scheme` on [`PartyTaxId`]. A line may carry several
/// classifications, so consumers select the scheme they need (a UBL/CII
/// serializer emits all of them; a national report picks its own scheme).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct ItemClassification {
    /// Classification code value (EN 16931 BT-158).
    pub code: String,
    /// Scheme/list identifier the code belongs to (EN 16931 BT-158-1 `listID`).
    pub scheme_id: String,
    /// Optional scheme version (EN 16931 BT-158-2 `listVersionID`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme_version: Option<String>,
}

impl ItemClassification {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.code, "lines.classifications.code")?;
        // EN 16931 BR-65: when a classification code is present its scheme
        // identifier is mandatory.
        validate_non_empty(&self.scheme_id, "lines.classifications.scheme_id")
    }
}

/// Document line.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
    /// Commodity/service classifications (EN 16931 BG-? / BT-158; national
    /// schemes such as HSN/SAC, NCM, `ClaveProdServ`). Empty when unclassified.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classifications: Vec<ItemClassification>,
    /// Line-level jurisdiction extensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<JurisdictionExtension>,
}

impl DocumentLine {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.id, "lines.id")?;
        validate_non_empty(&self.description, "lines.description")?;
        for classification in &self.classifications {
            classification.validate()?;
        }
        for extension in &self.extensions {
            extension.validate()?;
        }
        Ok(())
    }
}

/// Tax category summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
    /// Optional VAT exemption reason text (EN 16931 BT-120), for an exempt /
    /// zero-rated / reverse-charge category. Carried verbatim from the producer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption_reason: Option<String>,
    /// Optional VAT exemption reason code (EN 16931 BT-121), from a controlled
    /// list such as CEF `VATEX` or IT `Natura`. Carried verbatim from the
    /// producer — InvoiceKit serializes whatever code is supplied and does not
    /// invent one (the national code-list mapping is a separate concern).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption_reason_code: Option<String>,
}

impl TaxCategorySummary {
    fn validate(&self) -> Result<(), IrError> {
        validate_non_empty(&self.category_code, "tax_summary.category_code")
    }
}

/// Document monetary totals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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

/// Document-level allowance (EN 16931 BG-20) or charge (BG-21).
///
/// Both groups share an identical structure distinguished by `is_charge`; only
/// the business-term numbers differ (allowance BT-92..98 / charge BT-99..105).
/// The detail is carried verbatim from the producer — InvoiceKit serializes the
/// supplied amounts/codes faithfully and does not recompute or reconcile them
/// against the document totals (that is the reference validator's responsibility).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct DocumentAllowanceCharge {
    /// `true` = charge (BG-21), `false` = allowance (BG-20).
    pub is_charge: bool,
    /// Allowance/charge amount (BT-92 / BT-99).
    pub amount: MoneyAmount,
    /// Optional base amount the percentage applies to (BT-93 / BT-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_amount: Option<MoneyAmount>,
    /// Optional percentage of the base amount (BT-94 / BT-101).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<DecimalValue>,
    /// Optional VAT category code for the allowance/charge (BT-95 / BT-102).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_category: Option<String>,
    /// Optional VAT rate (BT-96 / BT-103).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_rate: Option<DecimalValue>,
    /// Optional reason text (BT-97 / BT-104).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Optional reason code (BT-98 / BT-105), from UNCL 5189 (allowances) or
    /// UNCL 7161 (charges); carried verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

impl DocumentAllowanceCharge {
    fn validate(&self) -> Result<(), IrError> {
        // EN 16931 BR-33 (allowance) / BR-38 (charge): a document-level
        // allowance or charge must carry a reason text or a reason code.
        if self.reason.is_none() && self.reason_code.is_none() {
            return Err(IrError::MissingRequiredField("allowance_charges.reason"));
        }
        Ok(())
    }
}

/// Content-addressed attachment reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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

/// Semantic class of a [`DocumentReference`].
///
/// Maps the open-vocabulary `kind` string onto the EN 16931 reference business
/// terms so a serializer can route each reference to the right element without
/// re-deriving the mapping.
///
/// The `kind` field is deliberately open (national producers use diverse
/// strings); [`DocumentReference::kind_class`] folds it onto this closed set,
/// and an unrecognized kind classifies as [`ReferenceKindClass::Other`] — a
/// serializer should not emit a typed reference element for those.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceKindClass {
    /// Purchase/sales order reference (EN 16931 BT-13 / BT-14;
    /// UBL `cac:OrderReference`, CII `BuyerOrderReferencedDocument`).
    Order,
    /// Preceding-invoice reference (EN 16931 BG-3 / BT-25; the original invoice
    /// a credit note, debit note, or correction refers to;
    /// UBL `cac:BillingReference`, CII `InvoiceReferencedDocument`).
    PrecedingInvoice,
    /// Contract reference (EN 16931 BT-12; UBL `cac:ContractDocumentReference`).
    Contract,
    /// Despatch-advice reference (EN 16931 BT-16; UBL `cac:DespatchDocumentReference`).
    DespatchAdvice,
    /// Receiving-advice reference (EN 16931 BT-15; UBL `cac:ReceiptDocumentReference`).
    ReceivingAdvice,
    /// A reference whose `kind` does not map to a known EN 16931 reference term.
    Other,
}

/// Reference to another commercial document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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

    /// Classify the open-vocabulary [`Self::kind`] onto an EN 16931 reference
    /// business term. Matching is case-insensitive and tolerant of `-`/`_`/space
    /// punctuation variants seen across national producers. An unrecognized kind
    /// returns [`ReferenceKindClass::Other`] (do not emit a typed element for it).
    #[must_use]
    pub fn kind_class(&self) -> ReferenceKindClass {
        let normalized = self
            .kind
            .to_ascii_lowercase()
            .replace(['_', ' '], "-");
        let k = normalized.as_str();
        // Preceding invoice: the original document a credit/debit note or
        // correction points back to. Covers the diverse national vocabulary
        // (e.g. "original-invoice", "credit-note-original-invoice",
        // "rectified-invoice", "corrected-ecf", "cfdi-relacion-01", "factura").
        if k.contains("preceding")
            || k.contains("original")
            || k.contains("rectified")
            || k.contains("corrected")
            || k.contains("credit-note")
            || k.contains("relacion")
            || k == "invoice"
            || k == "factura"
        {
            return ReferenceKindClass::PrecedingInvoice;
        }
        if k.contains("order") || k == "po" {
            return ReferenceKindClass::Order;
        }
        if k.contains("contract") {
            return ReferenceKindClass::Contract;
        }
        if k.contains("despatch") || k.contains("dispatch") {
            return ReferenceKindClass::DespatchAdvice;
        }
        if k.contains("receipt") || k.contains("receiv") {
            return ReferenceKindClass::ReceivingAdvice;
        }
        ReferenceKindClass::Other
    }
}

/// Operational metadata for a document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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

/// Invoice billing period (EN 16931 BG-14).
///
/// The date range the invoice covers (e.g. a periodic or summary invoice). At
/// least one of `start_date` (BT-73) or `end_date` (BT-74) must be present when
/// the group is used.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct InvoicePeriod {
    /// Period start date (BT-73).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<DateOnly>,
    /// Period end date (BT-74).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<DateOnly>,
}

impl InvoicePeriod {
    fn validate(&self) -> Result<(), IrError> {
        if let Some(date) = &self.start_date {
            date.validate()?;
        }
        if let Some(date) = &self.end_date {
            date.validate()?;
        }
        if self.start_date.is_none() && self.end_date.is_none() {
            return Err(IrError::MissingRequiredField("invoice_period"));
        }
        Ok(())
    }
}

/// Input parts for constructing a [`CommercialDocument`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
    /// Optional invoice billing period (EN 16931 BG-14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_period: Option<InvoicePeriod>,
    /// Optional actual delivery date (EN 16931 BT-72).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_date: Option<DateOnly>,
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
    /// Document-level allowances (EN 16931 BG-20) and charges (BG-21).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowance_charges: Vec<DocumentAllowanceCharge>,
    /// Operational metadata.
    pub meta: DocumentMeta,
}

/// Jurisdiction-agnostic commercial invoice or credit note semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
    /// Optional invoice billing period (EN 16931 BG-14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice_period: Option<InvoicePeriod>,
    /// Optional actual delivery date (EN 16931 BT-72).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_date: Option<DateOnly>,
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
    /// Document-level allowances (EN 16931 BG-20) and charges (BG-21).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowance_charges: Vec<DocumentAllowanceCharge>,
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
            invoice_period: parts.invoice_period,
            delivery_date: parts.delivery_date,
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
            allowance_charges: parts.allowance_charges,
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
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{CommercialDocument, IrError};
    /// use serde_json::json;
    ///
    /// // An empty object fails the {id, document_type, ...} shape check.
    /// let result = CommercialDocument::try_from_value(json!({}));
    /// assert!(result.is_err());
    /// # Ok::<(), IrError>(())
    /// ```
    pub fn try_from_value(value: Value) -> Result<Self, IrError> {
        let document: Self = serde_json::from_value(value)?;
        document.validate()?;
        Ok(document)
    }

    /// Serializes the document into JSON.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization errors. The round-trip
    /// `to_value(...)` -> `try_from_value(...)` -> `to_value(...)`
    /// is byte-stable.
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
        if let Some(period) = &self.invoice_period {
            period.validate()?;
        }
        if let Some(date) = &self.delivery_date {
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
        for allowance_charge in &self.allowance_charges {
            allowance_charge.validate()?;
        }
        self.meta.validate()
    }
}

/// Profile identifier and version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
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
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_ir::{IrError, LossinessLedger};
    ///
    /// let ledger = LossinessLedger::new(vec![], vec![])?;
    /// assert!(ledger.preserved.is_empty());
    /// assert!(ledger.lost.is_empty());
    /// assert!(ledger.warnings.is_empty());
    /// # Ok::<(), IrError>(())
    /// ```
    pub fn new(preserved: Vec<LossinessEntry>, lost: Vec<LossinessEntry>) -> Result<Self, IrError> {
        let ledger = Self {
            preserved,
            lost,
            warnings: Vec::new(),
        };
        ledger.validate()?;
        Ok(ledger)
    }

    /// Builds a field-level ledger by comparing a source document with
    /// the document produced after a format projection round trip.
    ///
    /// # Errors
    ///
    /// Returns an [`IrError`] when a generated ledger entry is invalid.
    pub fn from_roundtrip_comparison(
        source: &CommercialDocument,
        reparsed: &CommercialDocument,
        adapter: &'static str,
    ) -> Result<Self, IrError> {
        let mut preserved: Vec<LossinessEntry> = Vec::new();
        let mut lost: Vec<LossinessEntry> = Vec::new();

        record_identity_lossiness(&mut preserved, &mut lost, source, reparsed, adapter);
        record_payload_lossiness(&mut preserved, &mut lost, source, reparsed, adapter);

        Self::new(preserved, lost)
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

fn record_identity_lossiness(
    preserved: &mut Vec<LossinessEntry>,
    lost: &mut Vec<LossinessEntry>,
    source: &CommercialDocument,
    reparsed: &CommercialDocument,
    adapter: &'static str,
) {
    record_field_lossiness(
        preserved,
        lost,
        "/id",
        source.id.as_str() == reparsed.id.as_str(),
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/schema_version",
        source.schema_version == reparsed.schema_version,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/document_type",
        source.document_type == reparsed.document_type,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/issue_date",
        source.issue_date == reparsed.issue_date,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/tax_point_date",
        source.tax_point_date == reparsed.tax_point_date,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/due_date",
        source.due_date == reparsed.due_date,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/invoice_period",
        source.invoice_period == reparsed.invoice_period,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/delivery_date",
        source.delivery_date == reparsed.delivery_date,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/document_number",
        source.document_number.as_str() == reparsed.document_number.as_str(),
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/currency",
        source.currency.as_str() == reparsed.currency.as_str(),
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/meta",
        source.meta == reparsed.meta,
        adapter,
    );
}

fn record_payload_lossiness(
    preserved: &mut Vec<LossinessEntry>,
    lost: &mut Vec<LossinessEntry>,
    source: &CommercialDocument,
    reparsed: &CommercialDocument,
    adapter: &'static str,
) {
    record_field_lossiness(
        preserved,
        lost,
        "/supplier",
        source.supplier == reparsed.supplier,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/customer",
        source.customer == reparsed.customer,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/payee",
        source.payee == reparsed.payee,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/payment_terms",
        source.payment_terms == reparsed.payment_terms,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/payment_instructions",
        source.payment_instructions == reparsed.payment_instructions,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/lines",
        source.lines == reparsed.lines,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/tax_summary",
        source.tax_summary == reparsed.tax_summary,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/monetary_total",
        source.monetary_total == reparsed.monetary_total,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/attachments",
        source.attachments == reparsed.attachments,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/references",
        source.references == reparsed.references,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/notes",
        source.notes == reparsed.notes,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/extensions",
        source.extensions == reparsed.extensions,
        adapter,
    );
    record_field_lossiness(
        preserved,
        lost,
        "/allowance_charges",
        source.allowance_charges == reparsed.allowance_charges,
        adapter,
    );
}

fn record_field_lossiness(
    preserved: &mut Vec<LossinessEntry>,
    lost: &mut Vec<LossinessEntry>,
    path: &'static str,
    survived: bool,
    adapter: &'static str,
) {
    record_lossiness(
        preserved,
        lost,
        path,
        survived,
        || format!("{adapter} round-trips {path}"),
        || format!("{adapter} drift at {path}"),
    );
}

fn record_lossiness(
    preserved: &mut Vec<LossinessEntry>,
    lost: &mut Vec<LossinessEntry>,
    path: &'static str,
    survived: bool,
    on_preserved: impl FnOnce() -> String,
    on_lost: impl FnOnce() -> String,
) {
    if survived {
        preserved.push(LossinessEntry {
            path: path.to_owned(),
            reason: on_preserved(),
        });
    } else {
        lost.push(LossinessEntry {
            path: path.to_owned(),
            reason: on_lost(),
        });
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

/// Generate the canonical JSON Schema (Draft 2020-12) for [`CommercialDocument`].
///
/// The returned value is the schema the InvoiceKit binding generators
/// (TypeScript, Python, Java, .NET) consume; CI re-derives it on every PR
/// and asserts byte-equality against the committed
/// `schemas/invoicekit-ir-v1.json`, so a future PR that edits the Rust
/// source of truth without regenerating the schema cannot ship.
///
/// # Panics
///
/// Panics only if `schemars` produces a schema that fails to serialize
/// back to a `serde_json::Value`, which the `schemars` documentation rules
/// out for any type that derives `JsonSchema` (and `CommercialDocument`
/// does). In practice this function is total.
///
/// # Examples
///
/// ```
/// let schema = invoicekit_ir::commercial_document_schema();
/// assert!(schema.get("$schema").is_some());
/// assert_eq!(
///     schema.get("title").and_then(|v| v.as_str()),
///     Some("CommercialDocument"),
/// );
/// ```
#[must_use]
pub fn commercial_document_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(CommercialDocument);
    serde_json::to_value(schema).expect("schemars output is always serializable")
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
        crate_name, CommercialDocument, CommercialDocumentParts, DateOnly, DocumentId,
        DocumentReference, IrError, JurisdictionExtension, LossinessEntry, LossinessLedger,
        ProfileIdentifier, ProfileView, ReferenceKindClass,
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
    fn classification_round_trips_and_defaults_to_empty() {
        // Absent `classifications` deserializes to an empty vec (additive,
        // backward-compatible); a present one round-trips fully.
        let baseline = CommercialDocument::try_from_value(synthetic_document_json()).unwrap();
        assert!(baseline.lines[0].classifications.is_empty());

        let mut input = synthetic_document_json();
        input["lines"][0]["classifications"] =
            json!([{ "code": "0901", "scheme_id": "HSN", "scheme_version": "2017" }]);
        let doc = CommercialDocument::try_from_value(input).unwrap();
        let classification = &doc.lines[0].classifications[0];
        assert_eq!(classification.code, "0901");
        assert_eq!(classification.scheme_id, "HSN");
        assert_eq!(classification.scheme_version.as_deref(), Some("2017"));
    }

    #[test]
    fn tax_exemption_reason_round_trips_and_defaults_to_none() {
        // Absent exemption fields deserialize to None (additive, backward-compatible).
        let baseline = CommercialDocument::try_from_value(synthetic_document_json()).unwrap();
        assert!(baseline.tax_summary[0].exemption_reason.is_none());
        assert!(baseline.tax_summary[0].exemption_reason_code.is_none());

        // A present reason + code round-trips verbatim (EN 16931 BT-120 / BT-121).
        let mut input = synthetic_document_json();
        input["tax_summary"][0]["exemption_reason"] = json!("Reverse charge");
        input["tax_summary"][0]["exemption_reason_code"] = json!("VATEX-EU-AE");
        let doc = CommercialDocument::try_from_value(input).unwrap();
        assert_eq!(
            doc.tax_summary[0].exemption_reason.as_deref(),
            Some("Reverse charge")
        );
        assert_eq!(
            doc.tax_summary[0].exemption_reason_code.as_deref(),
            Some("VATEX-EU-AE")
        );
    }

    #[test]
    fn invoice_period_and_delivery_date_round_trip_and_default_to_none() {
        // Absent BG-14 period + BT-72 delivery date deserialize to None
        // (additive, backward-compatible).
        let baseline = CommercialDocument::try_from_value(synthetic_document_json()).unwrap();
        assert!(baseline.invoice_period.is_none());
        assert!(baseline.delivery_date.is_none());

        // A present period (BT-73/74) + delivery date (BT-72) survives a full
        // from_value -> to_value round-trip verbatim.
        let mut input = synthetic_document_json();
        input["invoice_period"] = json!({ "start_date": "2026-05-01", "end_date": "2026-05-31" });
        input["delivery_date"] = json!("2026-05-28");
        let doc = CommercialDocument::try_from_value(input).unwrap();
        let period = doc.invoice_period.as_ref().unwrap();
        assert_eq!(period.start_date.as_ref().map(DateOnly::as_str), Some("2026-05-01"));
        assert_eq!(period.end_date.as_ref().map(DateOnly::as_str), Some("2026-05-31"));
        assert_eq!(doc.delivery_date.as_ref().map(DateOnly::as_str), Some("2026-05-28"));

        let out = doc.to_value().unwrap();
        assert_eq!(out["invoice_period"]["start_date"], json!("2026-05-01"));
        assert_eq!(out["invoice_period"]["end_date"], json!("2026-05-31"));
        assert_eq!(out["delivery_date"], json!("2026-05-28"));
    }

    #[test]
    fn allowance_charges_round_trip_and_default_to_empty() {
        // Absent allowance_charges deserializes to an empty vec (additive,
        // backward-compatible).
        let baseline = CommercialDocument::try_from_value(synthetic_document_json()).unwrap();
        assert!(baseline.allowance_charges.is_empty());

        // A present allowance (BG-20) + charge (BG-21) round-trips verbatim
        // through from_value -> to_value.
        let mut input = synthetic_document_json();
        input["allowance_charges"] = json!([
            {
                "is_charge": false,
                "amount": "10.00",
                "base_amount": "100.00",
                "percentage": "10.00",
                "tax_category": "S",
                "tax_rate": "19.00",
                "reason": "Loyalty discount",
                "reason_code": "95"
            },
            {
                "is_charge": true,
                "amount": "5.00",
                "tax_category": "S",
                "tax_rate": "19.00",
                "reason_code": "FC"
            }
        ]);
        let doc = CommercialDocument::try_from_value(input).unwrap();
        assert_eq!(doc.allowance_charges.len(), 2);
        let allowance = &doc.allowance_charges[0];
        assert!(!allowance.is_charge);
        assert_eq!(allowance.amount.inner().to_string(), "10.00");
        assert_eq!(allowance.reason.as_deref(), Some("Loyalty discount"));
        let charge = &doc.allowance_charges[1];
        assert!(charge.is_charge);
        assert_eq!(charge.reason_code.as_deref(), Some("FC"));

        let out = doc.to_value().unwrap();
        assert_eq!(out["allowance_charges"][0]["amount"], json!("10.00"));
        assert_eq!(out["allowance_charges"][1]["is_charge"], json!(true));
    }

    #[test]
    fn allowance_charge_requires_a_reason_or_code() {
        // EN 16931 BR-33 / BR-38: a document-level allowance/charge must carry a
        // reason or a reason code.
        let mut input = synthetic_document_json();
        input["allowance_charges"] = json!([{ "is_charge": false, "amount": "10.00" }]);
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(
            err,
            IrError::MissingRequiredField("allowance_charges.reason")
        ));
    }

    #[test]
    fn invoice_period_rejects_an_empty_group() {
        // BG-14, when present, requires at least one of BT-73 / BT-74 — pin the
        // exact variant so the test keeps exercising InvoicePeriod::validate's
        // at-least-one rule (not some unrelated decode error).
        let mut input = synthetic_document_json();
        input["invoice_period"] = json!({});
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(err, IrError::MissingRequiredField("invoice_period")));
    }

    #[test]
    fn reference_kind_class_maps_the_national_vocabulary() {
        let preceding = |k: &str| DocumentReference {
            kind: k.to_owned(),
            id: "X".to_owned(),
            issue_date: None,
        }
        .kind_class();
        // Every kind string currently produced across the report crates is a
        // preceding-invoice / correction reference.
        for k in [
            "rectified-invoice",
            "original-invoice",
            "original-cu-invoice",
            "invoice",
            "factura",
            "eta-original-uuid",
            "credit-note-original-invoice",
            "credit-note-original",
            "credit-note-of",
            "corrected-ecf",
            "cfdi-relacion-01",
        ] {
            assert_eq!(
                preceding(k),
                ReferenceKindClass::PrecedingInvoice,
                "kind {k:?} should classify as PrecedingInvoice"
            );
        }
        assert_eq!(preceding("purchase order"), ReferenceKindClass::Order);
        assert_eq!(preceding("Order_Ref"), ReferenceKindClass::Order);
        assert_eq!(preceding("contract"), ReferenceKindClass::Contract);
        assert_eq!(preceding("despatch-advice"), ReferenceKindClass::DespatchAdvice);
        assert_eq!(preceding("receipt-advice"), ReferenceKindClass::ReceivingAdvice);
        // An unrecognized kind must not pretend to be a typed reference.
        assert_eq!(preceding("something-bespoke"), ReferenceKindClass::Other);
    }

    #[test]
    fn classification_without_scheme_id_is_rejected() {
        // EN 16931 BR-65: a classification code requires a scheme identifier.
        let mut input = synthetic_document_json();
        input["lines"][0]["classifications"] = json!([{ "code": "9983", "scheme_id": "" }]);
        let err = CommercialDocument::try_from_value(input).unwrap_err();
        assert!(matches!(
            err,
            IrError::MissingRequiredField("lines.classifications.scheme_id")
        ));
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

    // ebaq: RFC 8141 case-insensitive scheme handling
    //
    // The URN scheme name is case-insensitive per RFC 8141. We accept any
    // casing of the scheme prefix on construction *and* on deserialisation,
    // canonicalise to lowercase `urn:` so equality holds, and leave the
    // namespace identifier and namespace-specific string untouched.

    #[test]
    fn ebaq_extension_new_accepts_uppercase_scheme() {
        let extension =
            JurisdictionExtension::new("URN:invoicekit:ext:sa:zatca:2.0", json!({"qr": "x"}))
                .unwrap();
        assert_eq!(extension.urn, "urn:invoicekit:ext:sa:zatca:2.0");
    }

    #[test]
    fn ebaq_extension_new_accepts_mixed_case_scheme() {
        let extension =
            JurisdictionExtension::new("Urn:invoicekit:ext:de:xrechnung:3.0", json!({"id": 1}))
                .unwrap();
        assert_eq!(extension.urn, "urn:invoicekit:ext:de:xrechnung:3.0");

        let extension =
            JurisdictionExtension::new("uRN:peppol:bis:billing:3.0", json!({"id": 2})).unwrap();
        assert_eq!(extension.urn, "urn:peppol:bis:billing:3.0");
    }

    #[test]
    fn ebaq_extension_new_preserves_nid_nss_casing() {
        // Registries below the scheme are case-sensitive in practice.
        // Mixed-case ZUGFeRD profile identifiers must round-trip byte-for-byte.
        let extension = JurisdictionExtension::new(
            "URN:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:BASICWL",
            json!({"profile": "BASIC WL"}),
        )
        .unwrap();
        assert_eq!(
            extension.urn,
            "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:BASICWL"
        );
    }

    #[test]
    fn ebaq_extension_deserialize_canonicalises_scheme() {
        let raw = json!({
            "urn": "URN:invoicekit:ext:fr:ctc:1.0",
            "payload": {"k": "v"}
        });
        let extension: JurisdictionExtension = serde_json::from_value(raw).unwrap();
        assert_eq!(extension.urn, "urn:invoicekit:ext:fr:ctc:1.0");
        extension.validate().unwrap();
    }

    #[test]
    fn ebaq_extension_lowercase_scheme_is_left_alone() {
        // The fast path: an already-lowercase URN must not allocate a new
        // string. Functional check: bytes-equal to the input.
        let input = "urn:invoicekit:ext:sa:zatca:2.0";
        let extension = JurisdictionExtension::new(input, json!({})).unwrap();
        assert_eq!(extension.urn, input);
    }

    #[test]
    fn ebaq_extension_canonicalises_whitespace_padded_scheme() {
        // Regression: `canonicalise_urn` used to inspect the untrimmed input,
        // so a whitespace-padded uppercase scheme skipped canonicalisation and
        // was stored verbatim, while `validate` trimmed before checking. That
        // broke the `Eq` invariant against an unpadded lowercase equivalent and
        // poisoned the canonical signing payload. Both `new` and the
        // `Deserialize` path must trim and canonicalise to lowercase `urn:`.
        let padded = JurisdictionExtension::new(
            "  URN:invoicekit:ext:fr:ctc:1.0  ",
            json!({"k": "v"}),
        )
        .expect("whitespace-padded uppercase scheme is valid");
        let canonical = JurisdictionExtension::new(
            "urn:invoicekit:ext:fr:ctc:1.0",
            json!({"k": "v"}),
        )
        .expect("canonical form is valid");
        assert_eq!(padded.urn, "urn:invoicekit:ext:fr:ctc:1.0");
        assert_eq!(padded, canonical, "Eq invariant must hold after trimming");

        let raw = json!({
            "urn": "  URN:invoicekit:ext:fr:ctc:1.0  ",
            "payload": {"k": "v"}
        });
        let deserialized: JurisdictionExtension =
            serde_json::from_value(raw).expect("deserialize whitespace-padded scheme");
        assert_eq!(deserialized.urn, "urn:invoicekit:ext:fr:ctc:1.0");
        assert_eq!(deserialized, canonical);
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
