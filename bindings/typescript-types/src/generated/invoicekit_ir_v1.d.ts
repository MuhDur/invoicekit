// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with `bun run generate` from
// bindings/typescript-types/. Source of truth: schemas/.
//
/**
 * Canonical schema version carried by every serialized document.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "SchemaVersion".
 */
export type SchemaVersion = "1.0"
/**
 * Top-level document type.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DocumentType".
 */
export type DocumentType = ("invoice" | "credit_note" | "debit_note" | "pro_forma" | "self_billed")
/**
 * Payment rail or instruction kind.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "PaymentInstructionKind".
 */
export type PaymentInstructionKind = ("sepa" | "iban_bic" | "swiss_qr" | "epc_qr" | "zatca_qr" | "other")

/**
 * Jurisdiction-agnostic commercial invoice or credit note semantics.
 */
export interface CommercialDocument {
/**
 * Schema version.
 */
schema_version?: "1.0"
/**
 * Stable document identifier.
 */
id: string
/**
 * Document type.
 */
document_type: ("invoice" | "credit_note" | "debit_note" | "pro_forma" | "self_billed")
/**
 * Issue date.
 */
issue_date: string
/**
 * Optional tax point date.
 */
tax_point_date?: (string | null)
/**
 * Optional due date.
 */
due_date?: (string | null)
/**
 * Document number.
 */
document_number: string
/**
 * Document currency.
 */
currency: string
supplier: Party
customer: Party1
/**
 * Optional payee party.
 */
payee?: (Party2 | null)
/**
 * Optional payment terms.
 */
payment_terms?: (PaymentTerms | null)
/**
 * Payment instructions.
 */
payment_instructions?: PaymentInstruction[]
/**
 * Document lines.
 */
lines: DocumentLine[]
/**
 * Tax summaries.
 */
tax_summary?: TaxCategorySummary[]
monetary_total: MonetaryTotal
/**
 * Content-addressed attachments.
 */
attachments?: Attachment[]
/**
 * Commercial document references.
 */
references?: DocumentReference[]
/**
 * Human-readable notes.
 */
notes?: LocalizedString[]
/**
 * Jurisdiction extension envelopes.
 */
extensions?: JurisdictionExtension[]
meta: DocumentMeta
}
/**
 * Supplier party.
 */
export interface Party {
/**
 * Optional stable party identifier.
 */
id?: (string | null)
/**
 * Legal or trading name.
 */
name: string
/**
 * Tax identifiers carried by the party.
 */
tax_ids?: PartyTaxId[]
address: PostalAddress
/**
 * Optional contact details.
 */
contact?: (Contact | null)
}
/**
 * Party tax identifier.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "PartyTaxId".
 */
export interface PartyTaxId {
/**
 * Identifier scheme, such as `vat`.
 */
scheme: string
/**
 * Identifier value.
 */
value: string
}
/**
 * Postal address.
 */
export interface PostalAddress {
/**
 * Address lines in display order.
 */
lines: string[]
/**
 * Locality or city.
 */
city: string
/**
 * Optional subdivision, state, province, or region.
 */
subdivision?: (string | null)
/**
 * Postal code.
 */
postal_code: string
/**
 * Country code.
 */
country: string
}
/**
 * Contact details for a party.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "Contact".
 */
export interface Contact {
/**
 * Optional contact name.
 */
name?: (string | null)
/**
 * Optional email address.
 */
email?: (string | null)
/**
 * Optional telephone number.
 */
phone?: (string | null)
}
/**
 * Customer party.
 */
export interface Party1 {
/**
 * Optional stable party identifier.
 */
id?: (string | null)
/**
 * Legal or trading name.
 */
name: string
/**
 * Tax identifiers carried by the party.
 */
tax_ids?: PartyTaxId[]
address: PostalAddress
/**
 * Optional contact details.
 */
contact?: (Contact | null)
}
/**
 * Commercial party.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "Party".
 */
export interface Party2 {
/**
 * Optional stable party identifier.
 */
id?: (string | null)
/**
 * Legal or trading name.
 */
name: string
/**
 * Tax identifiers carried by the party.
 */
tax_ids?: PartyTaxId[]
address: PostalAddress
/**
 * Optional contact details.
 */
contact?: (Contact | null)
}
/**
 * Payment terms attached to the document.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "PaymentTerms".
 */
export interface PaymentTerms {
/**
 * Human-readable payment terms.
 */
description: string
/**
 * Optional due date stated in the terms.
 */
due_date?: (string | null)
}
/**
 * Payment instruction.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "PaymentInstruction".
 */
export interface PaymentInstruction {
/**
 * Instruction kind.
 */
kind: ("sepa" | "iban_bic" | "swiss_qr" | "epc_qr" | "zatca_qr" | "other")
/**
 * Optional account or payment address.
 */
account?: (string | null)
/**
 * Optional payment reference.
 */
reference?: (string | null)
}
/**
 * Document line.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DocumentLine".
 */
export interface DocumentLine {
/**
 * Line description.
 */
description: string
/**
 * Line identifier.
 */
id: string
/**
 * Invoiced quantity.
 */
quantity: string
/**
 * Optional unit code.
 */
unit_code?: (string | null)
/**
 * Unit price amount.
 */
unit_price: string
/**
 * Line extension amount.
 */
line_extension_amount: string
/**
 * Optional tax category code.
 */
tax_category?: (string | null)
/**
 * Line-level jurisdiction extensions.
 */
extensions?: JurisdictionExtension[]
}
/**
 * Polymorphic jurisdiction or profile extension payload.
 * 
 *  # URN scheme casing (ebaq)
 * 
 *  RFC 8141 declares the URN scheme name case-insensitive. Real producers
 *  in the wild (notably some legacy XML exporters) emit `URN:` or `Urn:`.
 *  Both [`JurisdictionExtension::new`] and the [`Deserialize`] implementation
 *  accept any casing of the scheme prefix and normalise it to the canonical
 *  lowercase `urn:` so equality checks remain stable.
 * 
 *  The namespace identifier and namespace-specific string are preserved
 *  verbatim. Rule-pack URN registries stay as shipped — Peppol's UNCL1001
 *  codes are lowercase by definition, ZUGFeRD profile URNs are mixed case,
 *  and changing those bytes would break canonical signing payloads.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "JurisdictionExtension".
 */
export interface JurisdictionExtension {
/**
 * Uniform resource name for the extension schema (canonical lowercase
 *  `urn:` prefix).
 */
urn: string
/**
 * Extension payload validated by the country or profile registry.
 */
payload: {
[k: string]: unknown
}
}
/**
 * Tax category summary.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "TaxCategorySummary".
 */
export interface TaxCategorySummary {
/**
 * Tax category code.
 */
category_code: string
/**
 * Taxable amount.
 */
taxable_amount: string
/**
 * Tax amount.
 */
tax_amount: string
/**
 * Optional tax rate.
 */
tax_rate?: (string | null)
}
/**
 * Monetary total.
 */
export interface MonetaryTotal {
/**
 * Sum of line extension amounts.
 */
line_extension_amount: string
/**
 * Tax-exclusive amount.
 */
tax_exclusive_amount: string
/**
 * Tax-inclusive amount.
 */
tax_inclusive_amount: string
/**
 * Optional allowance total.
 */
allowance_total_amount?: (string | null)
/**
 * Optional charge total.
 */
charge_total_amount?: (string | null)
/**
 * Optional prepaid amount.
 */
prepaid_amount?: (string | null)
/**
 * Payable amount.
 */
payable_amount: string
}
/**
 * Content-addressed attachment reference.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "Attachment".
 */
export interface Attachment {
/**
 * Attachment role or semantic type.
 */
kind: string
/**
 * Content digest.
 */
digest: string
/**
 * Media type.
 */
media_type: string
}
/**
 * Reference to another commercial document.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DocumentReference".
 */
export interface DocumentReference {
/**
 * Reference type, such as purchase order.
 */
kind: string
/**
 * Referenced identifier.
 */
id: string
/**
 * Optional referenced issue date.
 */
issue_date?: (string | null)
}
/**
 * Localized human text.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "LocalizedString".
 */
export interface LocalizedString {
/**
 * BCP 47 language tag.
 */
language: string
/**
 * Localized text value.
 */
text: string
}
/**
 * Operational metadata.
 */
export interface DocumentMeta {
/**
 * Tenant identifier.
 */
tenant_id: string
/**
 * Trace identifier for audit correlation.
 */
trace_id: string
/**
 * Optional source system.
 */
source_system?: (string | null)
}
/**
 * Postal address for a supplier, customer, payee, or other party.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "PostalAddress".
 */
export interface PostalAddress1 {
/**
 * Address lines in display order.
 */
lines: string[]
/**
 * Locality or city.
 */
city: string
/**
 * Optional subdivision, state, province, or region.
 */
subdivision?: (string | null)
/**
 * Postal code.
 */
postal_code: string
/**
 * Country code.
 */
country: string
}
/**
 * Document monetary totals.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "MonetaryTotal".
 */
export interface MonetaryTotal1 {
/**
 * Sum of line extension amounts.
 */
line_extension_amount: string
/**
 * Tax-exclusive amount.
 */
tax_exclusive_amount: string
/**
 * Tax-inclusive amount.
 */
tax_inclusive_amount: string
/**
 * Optional allowance total.
 */
allowance_total_amount?: (string | null)
/**
 * Optional charge total.
 */
charge_total_amount?: (string | null)
/**
 * Optional prepaid amount.
 */
prepaid_amount?: (string | null)
/**
 * Payable amount.
 */
payable_amount: string
}
/**
 * Operational metadata for a document.
 * 
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DocumentMeta".
 */
export interface DocumentMeta1 {
/**
 * Tenant identifier.
 */
tenant_id: string
/**
 * Trace identifier for audit correlation.
 */
trace_id: string
/**
 * Optional source system.
 */
source_system?: (string | null)
}
