// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Auditable UBL 2.1 Invoice/CreditNote element coverage.
//!
//! The rows in this module are derived from the OASIS UBL 2.1 OASIS Standard
//! maindoc schemas:
//!
//! - `xsd/maindoc/UBL-Invoice-2.1.xsd`
//! - `xsd/maindoc/UBL-CreditNote-2.1.xsd`
//!
//! They intentionally track the top-level document sequences. Reusable
//! aggregate internals, such as `cac:Party` and `cac:PostalAddress`, are mapped
//! by the parser and serializer helpers in this crate.

/// Official OASIS UBL 2.1 OASIS Standard HTML specification.
pub const UBL_2_1_OS_SPEC_URI: &str = "https://docs.oasis-open.org/ubl/os-UBL-2.1/UBL-2.1.html";

/// Official OASIS UBL 2.1 Invoice maindoc schema.
pub const UBL_2_1_INVOICE_SCHEMA_URI: &str =
    "https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-Invoice-2.1.xsd";

/// Official OASIS UBL 2.1 `CreditNote` maindoc schema.
pub const UBL_2_1_CREDIT_NOTE_SCHEMA_URI: &str =
    "https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-CreditNote-2.1.xsd";

/// InvoiceKit metadata extension URN used inside `ext:UBLExtensions`.
pub const INVOICEKIT_METADATA_EXTENSION_URN: &str = "urn:invoicekit:ubl:extension:metadata:v1";

/// Document-field extension URN used to preserve UBL fields without core IR homes.
pub const UBL_DOCUMENT_FIELDS_EXTENSION_URN: &str = "urn:invoicekit:ubl:2.1:document-fields";

/// UBL document kind covered by this matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UblDocumentKind {
    /// OASIS UBL 2.1 `Invoice`.
    Invoice,
    /// OASIS UBL 2.1 `CreditNote`.
    CreditNote,
}

/// How a UBL top-level element is represented by InvoiceKit today.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UblCoverageClass {
    /// The element maps to the current `invoicekit-ir` core commercial model.
    CoreIr,
    /// The element is represented by InvoiceKit's private UBL metadata extension.
    InvoiceKitMetadataExtension,
    /// The element is preserved as a UBL-specific `JurisdictionExtension`.
    UblDocumentFieldExtension,
    /// The element belongs to a profile or customization layer, not core IR.
    ProfileExtensionPayload,
    /// The element should be accounted for by the future lossiness-ledger pass.
    LossinessLedgerPreserved,
    /// The element has no implemented semantic or preservation strategy yet.
    UnsupportedGap,
}

/// One OASIS UBL 2.1 document-sequence element and its InvoiceKit strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UblElementCoverage {
    /// UBL document kind containing the element.
    pub document: UblDocumentKind,
    /// Namespace-qualified element reference from the maindoc XSD sequence.
    pub element: &'static str,
    /// Cardinality as published by the maindoc XSD.
    pub cardinality: &'static str,
    /// Current InvoiceKit representation class.
    pub class: UblCoverageClass,
    /// Human-readable mapping strategy or gap note.
    pub strategy: &'static str,
}

/// Returns the top-level element coverage rows for a UBL document kind.
#[must_use]
pub const fn top_level_coverage(document: UblDocumentKind) -> &'static [UblElementCoverage] {
    match document {
        UblDocumentKind::Invoice => INVOICE_ELEMENT_COVERAGE,
        UblDocumentKind::CreditNote => CREDIT_NOTE_ELEMENT_COVERAGE,
    }
}

/// Returns the coverage row for a namespace-qualified top-level element.
#[must_use]
pub fn coverage_for(
    document: UblDocumentKind,
    element: &str,
) -> Option<&'static UblElementCoverage> {
    top_level_coverage(document)
        .iter()
        .find(|row| row.element == element)
}

/// OASIS UBL 2.1 Invoice top-level sequence coverage.
pub const INVOICE_ELEMENT_COVERAGE: &[UblElementCoverage] = &[
    row_i("ext:UBLExtensions", "0..1", UblCoverageClass::InvoiceKitMetadataExtension, "InvoiceKit writes tenant, trace, and source metadata as ext:UBLExtensions/ik:DocumentMeta; other UBL extensions remain profile payloads."),
    row_i("cbc:UBLVersionID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Profile/version assertion; current core serializer omits and validators infer UBL 2.1 from the schema namespace."),
    row_i("cbc:CustomizationID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Serializer emits the core customization URN; future profile projections own non-core values."),
    row_i("cbc:ProfileID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Serializer emits the core profile URN; Peppol, PINT, and XRechnung projections own profile-specific values."),
    row_i("cbc:ProfileExecutionID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Collaboration-instance metadata; preserve in profile extension payload when needed."),
    row_i("cbc:ID", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.document_number."),
    row_i("cbc:CopyIndicator", "0..1", UblCoverageClass::UblDocumentFieldExtension, "No current IR field; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_i("cbc:UUID", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.id when present; parser falls back to cbc:ID."),
    row_i("cbc:IssueDate", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.issue_date."),
    row_i("cbc:IssueTime", "0..1", UblCoverageClass::UblDocumentFieldExtension, "No current date-time field in core IR; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_i("cbc:DueDate", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.due_date and payment_terms.due_date when terms exist."),
    row_i("cbc:InvoiceTypeCode", "0..1", UblCoverageClass::CoreIr, "Serializer emits code 380 for DocumentType::Invoice; richer code-list semantics belong to profile validation."),
    row_i("cbc:Note", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.notes."),
    row_i("cbc:TaxPointDate", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.tax_point_date."),
    row_i("cbc:DocumentCurrencyCode", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.currency."),
    row_i("cbc:TaxCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Multi-currency tax reporting is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_i("cbc:PricingCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Multi-currency pricing is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_i("cbc:PaymentCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Payment-currency separation is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_i("cbc:PaymentAlternativeCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Alternative payment currency is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_i("cbc:AccountingCostCode", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Business accounting allocation; preserve via future lossiness ledger until core/reference model grows a field."),
    row_i("cbc:AccountingCost", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Business accounting allocation text; preserved in urn:invoicekit:ubl:2.1:document-fields, never used as trace_id."),
    row_i("cbc:LineCountNumeric", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Derived check value; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_i("cbc:BuyerReference", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Buyer-assigned reference; preserved in urn:invoicekit:ubl:2.1:document-fields, never used as tenant_id."),
    row_i("cac:InvoicePeriod", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Invoice-period semantics need a period type before core support."),
    row_i("cac:OrderReference", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once reference-kind coverage is expanded."),
    row_i("cac:BillingReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once prior-invoice reference semantics are expanded."),
    row_i("cac:DespatchDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once logistics reference semantics are expanded."),
    row_i("cac:ReceiptDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once receipt reference semantics are expanded."),
    row_i("cac:StatementDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once statement reference semantics are expanded."),
    row_i("cac:OriginatorDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once originator reference semantics are expanded."),
    row_i("cac:ContractDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once contract reference semantics are expanded."),
    row_i("cac:AdditionalDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to attachments/references once document-reference payload coverage expands."),
    row_i("cac:ProjectReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Project references need a commercial project field or extension payload."),
    row_i("cac:Signature", "0..unbounded", UblCoverageClass::UblDocumentFieldExtension, "Signature payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields; signing semantics belong to evidence tasks."),
    row_i("cac:AccountingSupplierParty", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.supplier."),
    row_i("cac:AccountingCustomerParty", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.customer."),
    row_i("cac:PayeeParty", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payee."),
    row_i("cac:BuyerCustomerParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Buyer party distinct from accounting customer needs an expanded party-role model."),
    row_i("cac:SellerSupplierParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Seller party distinct from accounting supplier needs an expanded party-role model."),
    row_i("cac:TaxRepresentativeParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Tax representative needs an expanded party-role model."),
    row_i("cac:Delivery", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Delivery details need shipment/delivery structures before semantic support."),
    row_i("cac:DeliveryTerms", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Delivery terms need shipment/delivery structures before semantic support."),
    row_i("cac:PaymentMeans", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payment_instructions for current account/reference subset."),
    row_i("cac:PaymentTerms", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payment_terms for current note/due-date subset."),
    row_i("cac:PrepaidPayment", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Prepayment detail currently collapses only into LegalMonetaryTotal.prepaid_amount."),
    row_i("cac:AllowanceCharge", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Document-level allowance/charge detail currently collapses only into monetary totals."),
    row_i("cac:TaxExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_i("cac:PricingExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_i("cac:PaymentExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_i("cac:PaymentAlternativeExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_i("cac:TaxTotal", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.tax_summary for the current subtotal subset."),
    row_i("cac:WithholdingTaxTotal", "0..unbounded", UblCoverageClass::UblDocumentFieldExtension, "Withholding tax payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows withholding-tax semantics."),
    row_i("cac:LegalMonetaryTotal", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.monetary_total."),
    row_i("cac:InvoiceLine", "1..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.lines for the current line subset."),
];

/// OASIS UBL 2.1 `CreditNote` top-level sequence coverage.
pub const CREDIT_NOTE_ELEMENT_COVERAGE: &[UblElementCoverage] = &[
    row_c("ext:UBLExtensions", "0..1", UblCoverageClass::InvoiceKitMetadataExtension, "InvoiceKit writes tenant, trace, and source metadata as ext:UBLExtensions/ik:DocumentMeta; other UBL extensions remain profile payloads."),
    row_c("cbc:UBLVersionID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Profile/version assertion; current core serializer omits and validators infer UBL 2.1 from the schema namespace."),
    row_c("cbc:CustomizationID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Serializer emits the core customization URN; future profile projections own non-core values."),
    row_c("cbc:ProfileID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Serializer emits the core profile URN; Peppol, PINT, and XRechnung projections own profile-specific values."),
    row_c("cbc:ProfileExecutionID", "0..1", UblCoverageClass::ProfileExtensionPayload, "Collaboration-instance metadata; preserve in profile extension payload when needed."),
    row_c("cbc:ID", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.document_number."),
    row_c("cbc:CopyIndicator", "0..1", UblCoverageClass::UblDocumentFieldExtension, "No current IR field; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_c("cbc:UUID", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.id when present; parser falls back to cbc:ID."),
    row_c("cbc:IssueDate", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.issue_date."),
    row_c("cbc:IssueTime", "0..1", UblCoverageClass::UblDocumentFieldExtension, "No current date-time field in core IR; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_c("cbc:TaxPointDate", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.tax_point_date."),
    row_c("cbc:CreditNoteTypeCode", "0..1", UblCoverageClass::CoreIr, "Serializer emits code 381 for DocumentType::CreditNote; richer code-list semantics belong to profile validation."),
    row_c("cbc:Note", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.notes."),
    row_c("cbc:DocumentCurrencyCode", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.currency."),
    row_c("cbc:TaxCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Multi-currency tax reporting is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_c("cbc:PricingCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Multi-currency pricing is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_c("cbc:PaymentCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Payment-currency separation is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_c("cbc:PaymentAlternativeCurrencyCode", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Alternative payment currency is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows explicit currency roles."),
    row_c("cbc:AccountingCostCode", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Business accounting allocation; preserve via future lossiness ledger until core/reference model grows a field."),
    row_c("cbc:AccountingCost", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Business accounting allocation text; preserved in urn:invoicekit:ubl:2.1:document-fields, never used as trace_id."),
    row_c("cbc:LineCountNumeric", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Derived check value; preserved verbatim in urn:invoicekit:ubl:2.1:document-fields."),
    row_c("cbc:BuyerReference", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Buyer-assigned reference; preserved in urn:invoicekit:ubl:2.1:document-fields, never used as tenant_id."),
    row_c("cac:InvoicePeriod", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Invoice-period semantics need a period type before core support."),
    row_c("cac:DiscrepancyResponse", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Credit-note discrepancy response needs a correction/dispute model before semantic support."),
    row_c("cac:OrderReference", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once reference-kind coverage is expanded."),
    row_c("cac:BillingReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once prior-invoice reference semantics are expanded."),
    row_c("cac:DespatchDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once logistics reference semantics are expanded."),
    row_c("cac:ReceiptDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once receipt reference semantics are expanded."),
    row_c("cac:ContractDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once contract reference semantics are expanded."),
    row_c("cac:AdditionalDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to attachments/references once document-reference payload coverage expands."),
    row_c("cac:StatementDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once statement reference semantics are expanded."),
    row_c("cac:OriginatorDocumentReference", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Maps conceptually to DocumentReference once originator reference semantics are expanded."),
    row_c("cac:Signature", "0..unbounded", UblCoverageClass::UblDocumentFieldExtension, "Signature payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields; signing semantics belong to evidence tasks."),
    row_c("cac:AccountingSupplierParty", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.supplier."),
    row_c("cac:AccountingCustomerParty", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.customer."),
    row_c("cac:PayeeParty", "0..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payee."),
    row_c("cac:BuyerCustomerParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Buyer party distinct from accounting customer needs an expanded party-role model."),
    row_c("cac:SellerSupplierParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Seller party distinct from accounting supplier needs an expanded party-role model."),
    row_c("cac:TaxRepresentativeParty", "0..1", UblCoverageClass::LossinessLedgerPreserved, "Tax representative needs an expanded party-role model."),
    row_c("cac:Delivery", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Delivery details need shipment/delivery structures before semantic support."),
    row_c("cac:DeliveryTerms", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Delivery terms need shipment/delivery structures before semantic support."),
    row_c("cac:PaymentMeans", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payment_instructions for current account/reference subset."),
    row_c("cac:PaymentTerms", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.payment_terms for current note/due-date subset."),
    row_c("cac:TaxExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_c("cac:PricingExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_c("cac:PaymentExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_c("cac:PaymentAlternativeExchangeRate", "0..1", UblCoverageClass::UblDocumentFieldExtension, "Exchange-rate payload is preserved verbatim in urn:invoicekit:ubl:2.1:document-fields until core IR grows exchange-rate semantics."),
    row_c("cac:AllowanceCharge", "0..unbounded", UblCoverageClass::LossinessLedgerPreserved, "Document-level allowance/charge detail currently collapses only into monetary totals."),
    row_c("cac:TaxTotal", "0..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.tax_summary for the current subtotal subset."),
    row_c("cac:LegalMonetaryTotal", "1..1", UblCoverageClass::CoreIr, "Maps to CommercialDocument.monetary_total."),
    row_c("cac:CreditNoteLine", "1..unbounded", UblCoverageClass::CoreIr, "Maps to CommercialDocument.lines for the current line subset."),
];

const fn row_i(
    element: &'static str,
    cardinality: &'static str,
    class: UblCoverageClass,
    strategy: &'static str,
) -> UblElementCoverage {
    UblElementCoverage {
        document: UblDocumentKind::Invoice,
        element,
        cardinality,
        class,
        strategy,
    }
}

const fn row_c(
    element: &'static str,
    cardinality: &'static str,
    class: UblCoverageClass,
    strategy: &'static str,
) -> UblElementCoverage {
    UblElementCoverage {
        document: UblDocumentKind::CreditNote,
        element,
        cardinality,
        class,
        strategy,
    }
}
