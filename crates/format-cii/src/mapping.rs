// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Auditable CII D16B coverage and mapping decisions.
//!
//! The full generated element-edge matrix lives in
//! `crates/format-cii/data/cii-d16b-element-coverage.json`. This module keeps
//! the constants used by the parser and serializer close to the CII
//! implementation.

/// CEN EN16931 validation repository that carries the UN/CEFACT CII D16B XSD
/// bundle used for the coverage artifact.
pub const CII_D16B_SCHEMA_REPOSITORY: &str =
    "https://github.com/ConnectingEurope/eInvoicing-EN16931";

/// Validation tag used as the pinned source snapshot for CII D16B coverage.
pub const CII_D16B_SCHEMA_TAG: &str = "validation-1.3.16";

/// Exact commit behind [`CII_D16B_SCHEMA_TAG`] at the time the matrix was
/// generated.
pub const CII_D16B_SCHEMA_COMMIT: &str = "b6c9e06a59812fb1a83585da40923b3678a649ad";

/// CII document-field extension URN for standard fields that do not belong in
/// InvoiceKit operational metadata.
pub const CII_DOCUMENT_FIELDS_EXTENSION_URN: &str = "urn:invoicekit:cii:d16b:document-fields";

/// CII profile-context extension URN for profile/application context values
/// carried by CII document-context parameters.
pub const CII_PROFILE_CONTEXT_EXTENSION_URN: &str = "urn:invoicekit:cii:d16b:profile-context";

/// Application context parameter ID used to carry InvoiceKit operational
/// metadata inside CII without overloading CII business fields.
pub const INVOICEKIT_CII_METADATA_EXTENSION_URN: &str = "urn:invoicekit:cii:extension:metadata:v1";

/// One named CII mapping decision that would otherwise be easy to overload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CiiMappingDecision {
    /// Schema element path or element-edge identifier.
    pub element: &'static str,
    /// Current InvoiceKit representation class.
    pub class: &'static str,
    /// Concrete IR path or extension field used today.
    pub representation: &'static str,
    /// Short rationale for the mapping.
    pub rationale: &'static str,
}

/// Named decisions for the CII fields that are deliberately not used as
/// InvoiceKit operational metadata.
pub const NAMED_MAPPING_DECISIONS: &[CiiMappingDecision] = &[
    CiiMappingDecision {
        element: "HeaderTradeAgreementType/BuyerReference",
        class: "cii_document_field_extension",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:document-fields].buyer_reference",
        rationale: "BuyerReference is the buyer-assigned business reference; it is never tenant_id.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/BusinessProcessSpecifiedDocumentContextParameter",
        class: "cii_document_field_extension",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:document-fields].business_process_context_ids[]",
        rationale: "Business-process context is repeatable and identifies business processes; it is never trace_id.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/SpecifiedTransactionID",
        class: "profile_extension_payload",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:profile-context].transaction_ids[]",
        rationale: "Transaction context is repeatable CII profile data and is not an invoice document number.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/TestIndicator",
        class: "profile_extension_payload",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:profile-context].test_indicators[]",
        rationale: "CII test indicators describe profile/test context and are not InvoiceKit runtime flags.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/GuidelineSpecifiedDocumentContextParameter",
        class: "profile_extension_payload",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:profile-context].guideline_context_ids[]",
        rationale: "Guideline context declares the CII profile or CIUS; it is never a business-process context.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/ApplicationSpecifiedDocumentContextParameter",
        class: "profile_extension_payload",
        representation: "CommercialDocument.extensions[urn:invoicekit:cii:d16b:profile-context].application_contexts[]",
        rationale: "Third-party application context parameters are preserved separately from InvoiceKit-owned metadata.",
    },
    CiiMappingDecision {
        element: "ExchangedDocumentContextType/ApplicationSpecifiedDocumentContextParameter[ID=urn:invoicekit:cii:extension:metadata:v1]",
        class: "invoicekit_metadata_extension",
        representation: "CommercialDocument.meta via urn:invoicekit:cii:extension:metadata:v1",
        rationale: "InvoiceKit operational metadata is carried in an InvoiceKit-owned application context parameter.",
    },
];
