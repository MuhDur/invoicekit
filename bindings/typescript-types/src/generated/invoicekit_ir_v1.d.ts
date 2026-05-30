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
 * Optional invoice billing period (EN 16931 BG-14).
 */
invoice_period?: (InvoicePeriod | null)
/**
 * Optional actual delivery date (EN 16931 BT-72).
 */
delivery_date?: (string | null)
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
/**
 * Document-level allowances (EN 16931 BG-20) and charges (BG-21).
 */
allowance_charges?: DocumentAllowanceCharge[]
/**
 * Optional deliver-to information (EN 16931 BG-13 / BG-15).
 */
deliver_to?: (DeliverToParty | null)
meta: DocumentMeta
}
/**
 * Invoice billing period (EN 16931 BG-14).
 *
 *  The date range the invoice covers (e.g. a periodic or summary invoice). At
 *  least one of `start_date` (BT-73) or `end_date` (BT-74) must be present when
 *  the group is used.
 *
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "InvoicePeriod".
 */
export interface InvoicePeriod {
/**
 * Period start date (BT-73).
 */
start_date?: (string | null)
/**
 * Period end date (BT-74).
 */
end_date?: (string | null)
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
 * Commodity/service classifications (EN 16931 BG-? / BT-158; national
 *  schemes such as HSN/SAC, NCM, `ClaveProdServ`). Empty when unclassified.
 */
classifications?: ItemClassification[]
/**
 * Line-level jurisdiction extensions.
 */
extensions?: JurisdictionExtension[]
/**
 * Line-level allowances (EN 16931 BG-27) and charges (BG-28). Reuses the
 *  document-level [`DocumentAllowanceCharge`] shape; the VAT category fields
 *  are normally unused at line level (a line allowance/charge inherits the
 *  line's tax category), but are accepted when a producer supplies them.
 */
allowance_charges?: DocumentAllowanceCharge[]
}
/**
 * Commodity or service classification of a line item.
 *
 *  Models EN 16931 BT-158 *Item classification identifier* together with its
 *  scheme identifier (BT-158-1) and optional scheme version (BT-158-2). The
 *  `scheme_id` is an open list (e.g. UNTDID 7143 codes such as `HS`/`SRV`, or a
 *  national scheme name like `HSN`, `SAC`, `NCM`, `ClaveProdServ`, `UNSPSC`),
 *  mirroring the open `scheme` on [`PartyTaxId`]. A line may carry several
 *  classifications, so consumers select the scheme they need (a UBL/CII
 *  serializer emits all of them; a national report picks its own scheme).
 *
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "ItemClassification".
 */
export interface ItemClassification {
/**
 * Classification code value (EN 16931 BT-158).
 */
code: string
/**
 * Scheme/list identifier the code belongs to (EN 16931 BT-158-1 `listID`).
 */
scheme_id: string
/**
 * Optional scheme version (EN 16931 BT-158-2 `listVersionID`).
 */
scheme_version?: (string | null)
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
 * Document-level allowance (EN 16931 BG-20) or charge (BG-21).
 *
 *  Both groups share an identical structure distinguished by `is_charge`; only
 *  the business-term numbers differ (allowance BT-92..98 / charge BT-99..105).
 *  The detail is carried verbatim from the producer — InvoiceKit serializes the
 *  supplied amounts/codes faithfully and does not recompute or reconcile them
 *  against the document totals (that is the reference validator's responsibility).
 *
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DocumentAllowanceCharge".
 */
export interface DocumentAllowanceCharge {
/**
 * `true` = charge (BG-21), `false` = allowance (BG-20).
 */
is_charge: boolean
/**
 * Allowance/charge amount (BT-92 / BT-99).
 */
amount: string
/**
 * Optional base amount the percentage applies to (BT-93 / BT-100).
 */
base_amount?: (string | null)
/**
 * Optional percentage of the base amount (BT-94 / BT-101).
 */
percentage?: (string | null)
/**
 * Optional VAT category code for the allowance/charge (BT-95 / BT-102).
 */
tax_category?: (string | null)
/**
 * Optional VAT rate (BT-96 / BT-103).
 */
tax_rate?: (string | null)
/**
 * Optional reason text (BT-97 / BT-104).
 */
reason?: (string | null)
/**
 * Optional reason code (BT-98 / BT-105), from UNCL 5189 (allowances) or
 *  UNCL 7161 (charges); carried verbatim.
 */
reason_code?: (string | null)
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
/**
 * Optional VAT exemption reason text (EN 16931 BT-120), for an exempt /
 *  zero-rated / reverse-charge category. Carried verbatim from the producer.
 */
exemption_reason?: (string | null)
/**
 * Optional VAT exemption reason code (EN 16931 BT-121), from a controlled
 *  list such as CEF `VATEX` or IT `Natura`. Carried verbatim from the
 *  producer — InvoiceKit serializes whatever code is supplied and does not
 *  invent one (the national code-list mapping is a separate concern).
 */
exemption_reason_code?: (string | null)
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
 * Deliver-to information (EN 16931 BG-13): where the goods or services are
 *  delivered, when that differs from the buyer.
 *
 *  The actual delivery date (BT-72) is the separate top-level `delivery_date`
 *  field; this group carries the deliver-to party name (BT-70), location
 *  identifier (BT-71), and address (EN 16931 BG-15).
 *
 * This interface was referenced by `CommercialDocument`'s JSON-Schema
 * via the `definition` "DeliverToParty".
 */
export interface DeliverToParty {
/**
 * Deliver-to party name (BT-70).
 */
name?: (string | null)
/**
 * Deliver-to location identifier (BT-71).
 */
location_id?: (string | null)
/**
 * Deliver-to address (EN 16931 BG-15).
 */
address?: (PostalAddress1 | null)
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
