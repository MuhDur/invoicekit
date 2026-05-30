// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-067 PII/GDPR redactor for support bundles.
//!
//! Customers ship support bundles when something goes wrong; if the
//! bundle contains a real `CommercialDocument` the supplier and
//! customer names, addresses, contact details, and bank account
//! identifiers are personal data under GDPR Article 4(1). This module
//! produces a redacted copy of the document that preserves the *shape*
//! (so a debugger can reproduce the validator path, the canonicalizer,
//! the renderer) while replacing every PII field with the literal
//! placeholder `<REDACTED>`.
//!
//! ## API
//!
//! ```
//! use invoicekit_reconcile::redact::{redact_for_support, RedactedBundle};
//! use invoicekit_ir::*;
//! # use rust_decimal::Decimal;
//! # fn fixture() -> CommercialDocument { /* … built below in tests */
//! #   CommercialDocument::new(CommercialDocumentParts {
//! #     schema_version: SchemaVersion::V1_0,
//! #     id: DocumentId::new("doc-1").unwrap(),
//! #     document_type: DocumentType::Invoice,
//! #     issue_date: DateOnly::new("2026-05-26").unwrap(),
//! #     tax_point_date: None,
//! #     due_date: None,
//! #     invoice_period: None,
//! #     delivery_date: None,
//! #     document_number: DocumentNumber::new("INV-1").unwrap(),
//! #     currency: Iso4217Code::new("EUR").unwrap(),
//! #     supplier: Party {
//! #       id: Some("sup-1".to_owned()),
//! #       name: "ACME GmbH".to_owned(),
//! #       tax_ids: vec![PartyTaxId { scheme: "vat".to_owned(), value: "DE123456789".to_owned() }],
//! #       address: PostalAddress {
//! #         lines: vec!["Real Street 1".to_owned()],
//! #         city: "Berlin".to_owned(),
//! #         subdivision: None,
//! #         postal_code: "10115".to_owned(),
//! #         country: CountryCode::new("DE").unwrap(),
//! #       },
//! #       contact: Some(Contact {
//! #         name: Some("Real Person".to_owned()),
//! #         email: Some("real@example.test".to_owned()),
//! #         phone: Some("+49-30-000000".to_owned()),
//! #       }),
//! #     },
//! #     customer: Party {
//! #       id: Some("cus-1".to_owned()),
//! #       name: "Other Real GmbH".to_owned(),
//! #       tax_ids: vec![],
//! #       address: PostalAddress {
//! #         lines: vec!["Other Street 1".to_owned()],
//! #         city: "Munich".to_owned(),
//! #         subdivision: None,
//! #         postal_code: "80331".to_owned(),
//! #         country: CountryCode::new("DE").unwrap(),
//! #       },
//! #       contact: None,
//! #     },
//! #     payee: None,
//! #     payment_terms: None,
//! #     payment_instructions: vec![
//! #       PaymentInstruction { kind: PaymentInstructionKind::IbanBic,
//! #         account: Some("DE89370400440532013000".to_owned()),
//! #         reference: Some("INV-1".to_owned()) }],
//! #     lines: vec![DocumentLine {
//! #       id: "1".to_owned(),
//! #       description: "Service".to_owned(),
//! #       quantity: DecimalValue::new(Decimal::ONE),
//! #       unit_code: Some("EA".to_owned()),
//! #       unit_price: DecimalValue::new(Decimal::new(10000, 2)),
//! #       line_extension_amount: DecimalValue::new(Decimal::new(10000, 2)),
//! #       tax_category: Some("S".to_owned()),
//! #       classifications: vec![],
//! #       extensions: vec![],
//! #     }],
//! #     tax_summary: vec![],
//! #     monetary_total: MonetaryTotal {
//! #       line_extension_amount: DecimalValue::new(Decimal::new(10000, 2)),
//! #       tax_exclusive_amount: DecimalValue::new(Decimal::new(10000, 2)),
//! #       tax_inclusive_amount: DecimalValue::new(Decimal::new(10000, 2)),
//! #       allowance_total_amount: None, charge_total_amount: None, prepaid_amount: None,
//! #       payable_amount: DecimalValue::new(Decimal::new(10000, 2)),
//! #     },
//! #     attachments: vec![], references: vec![], notes: vec![],
//! #     extensions: vec![],
//! #     meta: DocumentMeta { tenant_id: "tenant".to_owned(), trace_id: "trace".to_owned(),
//! #       source_system: None },
//! #   }).unwrap()
//! # }
//! let bundle: RedactedBundle = redact_for_support(&fixture()).unwrap();
//! assert_eq!(bundle.document.supplier.name, "<REDACTED>");
//! assert!(!bundle.report.fields_redacted.is_empty());
//! ```
//!
//! ## What stays vs. what is redacted
//!
//! Redacted (replaced with `<REDACTED>` placeholders): supplier/customer/
//! payee party names; postal address lines / city / subdivision / postal
//! code (country stays); contact name / email / phone; payment-instruction
//! `account` (typical IBAN/SEPA target) and `reference`; party tax-id
//! `value` (the number itself; scheme like `"vat"` stays).
//!
//! Kept verbatim because they are reproducibility-critical and the
//! redaction policy explicitly excludes them: document id / number,
//! issue date, currency, line descriptions, monetary totals, tax category
//! codes, extensions, meta tenant/trace ids, party ids (opaque stable
//! identifiers, not personal data per GDPR Recital 26 when not tied
//! back to a natural person).
//!
//! ## Reversibility
//!
//! v1 of this redactor is one-way only. Per the bead "reversible only
//! with explicit unredaction key" is out of scope; the architecture
//! supports adding a sealed-envelope unredaction key in a follow-up
//! without changing the public `redact_for_support` signature.

use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, Contact, IrError, Party, PartyTaxId,
    PaymentInstruction, PostalAddress,
};
use serde::{Deserialize, Serialize};

/// Bead identifier carried alongside emitted log events for diagnostic correlation.
pub const REDACT_BEAD_ID: &str = "invoices-t-067-pii-gdpr-redactor-c93";

/// Placeholder string that replaces every redacted field value.
///
/// Chosen so the value is recognizably a placeholder in support-bundle
/// diffs and never collides with a real field shape (the leading `<`
/// fails any country-code / VAT-id / IBAN regex).
pub const REDACTED_PLACEHOLDER: &str = "<REDACTED>";

/// Result of [`redact_for_support`]: the redacted document plus a
/// structured report of which fields were replaced.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RedactedBundle {
    /// The redacted copy of the input commercial document.
    pub document: CommercialDocument,
    /// Field-level audit log of what was redacted.
    pub report: RedactionReport,
}

/// Audit report listing every field path replaced by [`REDACTED_PLACEHOLDER`].
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RedactionReport {
    /// JSON-Pointer-style paths to the fields that were redacted.
    pub fields_redacted: Vec<String>,
}

impl RedactionReport {
    /// Number of fields that were redacted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields_redacted.len()
    }

    /// True when nothing was redacted (input had no PII to begin with).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields_redacted.is_empty()
    }
}

/// Redact every PII field on `document` and return the redacted copy
/// plus an audit report.
///
/// # Errors
///
/// Returns [`IrError`] only if the underlying IR rejects the redacted
/// shape (which it shouldn't — the placeholder string is a valid
/// non-empty UTF-8 value for every field). Any error here is an
/// `invoicekit-ir` invariant bug, not a runtime PII issue.
pub fn redact_for_support(document: &CommercialDocument) -> Result<RedactedBundle, IrError> {
    let mut report = RedactionReport::default();
    let supplier = redact_party(&document.supplier, "supplier", &mut report);
    let customer = redact_party(&document.customer, "customer", &mut report);
    let payee = document
        .payee
        .as_ref()
        .map(|p| redact_party(p, "payee", &mut report));
    let payment_instructions = document
        .payment_instructions
        .iter()
        .enumerate()
        .map(|(idx, inst)| redact_payment_instruction(inst, idx, &mut report))
        .collect();

    let redacted = CommercialDocument::new(CommercialDocumentParts {
        schema_version: document.schema_version,
        id: document.id.clone(),
        document_type: document.document_type,
        issue_date: document.issue_date.clone(),
        tax_point_date: document.tax_point_date.clone(),
        due_date: document.due_date.clone(),
        invoice_period: None,
        delivery_date: None,
        document_number: document.document_number.clone(),
        currency: document.currency.clone(),
        supplier,
        customer,
        payee,
        payment_terms: document.payment_terms.clone(),
        payment_instructions,
        lines: document.lines.clone(),
        tax_summary: document.tax_summary.clone(),
        monetary_total: document.monetary_total.clone(),
        attachments: document.attachments.clone(),
        references: document.references.clone(),
        notes: document.notes.clone(),
        extensions: document.extensions.clone(),
        meta: document.meta.clone(),
    })?;

    Ok(RedactedBundle {
        document: redacted,
        report,
    })
}

fn redact_party(party: &Party, role: &str, report: &mut RedactionReport) -> Party {
    let mut redacted_tax_ids: Vec<PartyTaxId> = Vec::with_capacity(party.tax_ids.len());
    for (idx, tax_id) in party.tax_ids.iter().enumerate() {
        report
            .fields_redacted
            .push(format!("/{role}/tax_ids/{idx}/value"));
        redacted_tax_ids.push(PartyTaxId {
            scheme: tax_id.scheme.clone(),
            value: REDACTED_PLACEHOLDER.to_owned(),
        });
    }
    report.fields_redacted.push(format!("/{role}/name"));
    let address = redact_address(&party.address, role, report);
    let contact = party
        .contact
        .as_ref()
        .map(|c| redact_contact(c, role, report));
    Party {
        id: party.id.clone(),
        name: REDACTED_PLACEHOLDER.to_owned(),
        tax_ids: redacted_tax_ids,
        address,
        contact,
    }
}

fn redact_address(
    address: &PostalAddress,
    role: &str,
    report: &mut RedactionReport,
) -> PostalAddress {
    let lines = address
        .lines
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            report
                .fields_redacted
                .push(format!("/{role}/address/lines/{idx}"));
            REDACTED_PLACEHOLDER.to_owned()
        })
        .collect();
    report.fields_redacted.push(format!("/{role}/address/city"));
    report
        .fields_redacted
        .push(format!("/{role}/address/postal_code"));
    let subdivision = address.subdivision.as_ref().map(|_| {
        report
            .fields_redacted
            .push(format!("/{role}/address/subdivision"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    PostalAddress {
        lines,
        city: REDACTED_PLACEHOLDER.to_owned(),
        subdivision,
        postal_code: REDACTED_PLACEHOLDER.to_owned(),
        country: address.country.clone(),
    }
}

fn redact_contact(contact: &Contact, role: &str, report: &mut RedactionReport) -> Contact {
    let name = contact.name.as_ref().map(|_| {
        report.fields_redacted.push(format!("/{role}/contact/name"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    let email = contact.email.as_ref().map(|_| {
        report
            .fields_redacted
            .push(format!("/{role}/contact/email"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    let phone = contact.phone.as_ref().map(|_| {
        report
            .fields_redacted
            .push(format!("/{role}/contact/phone"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    Contact { name, email, phone }
}

fn redact_payment_instruction(
    inst: &PaymentInstruction,
    idx: usize,
    report: &mut RedactionReport,
) -> PaymentInstruction {
    let account = inst.account.as_ref().map(|_| {
        report
            .fields_redacted
            .push(format!("/payment_instructions/{idx}/account"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    let reference = inst.reference.as_ref().map(|_| {
        report
            .fields_redacted
            .push(format!("/payment_instructions/{idx}/reference"));
        REDACTED_PLACEHOLDER.to_owned()
    });
    PaymentInstruction {
        kind: inst.kind,
        account,
        reference,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocumentParts, CountryCode, DateOnly, DecimalValue, DocumentId, DocumentLine,
        DocumentMeta, DocumentNumber, DocumentType, Iso4217Code, MonetaryTotal,
        PaymentInstructionKind, SchemaVersion,
    };
    use rust_decimal::Decimal;

    fn fixture() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new("doc-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: None,
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("INV-1").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: Party {
                id: Some("sup-1".to_owned()),
                name: "ACME GmbH".to_owned(),
                tax_ids: vec![PartyTaxId {
                    scheme: "vat".to_owned(),
                    value: "DE123456789".to_owned(),
                }],
                address: PostalAddress {
                    lines: vec!["Real Street 1".to_owned(), "Suite 2".to_owned()],
                    city: "Berlin".to_owned(),
                    subdivision: Some("BE".to_owned()),
                    postal_code: "10115".to_owned(),
                    country: CountryCode::new("DE").unwrap(),
                },
                contact: Some(Contact {
                    name: Some("Real Person".to_owned()),
                    email: Some("real@example.test".to_owned()),
                    phone: Some("+49-30-000000".to_owned()),
                }),
            },
            customer: Party {
                id: Some("cus-1".to_owned()),
                name: "Other Real GmbH".to_owned(),
                tax_ids: vec![],
                address: PostalAddress {
                    lines: vec!["Other Street 1".to_owned()],
                    city: "Munich".to_owned(),
                    subdivision: None,
                    postal_code: "80331".to_owned(),
                    country: CountryCode::new("DE").unwrap(),
                },
                contact: None,
            },
            payee: None,
            payment_terms: None,
            payment_instructions: vec![PaymentInstruction {
                kind: PaymentInstructionKind::IbanBic,
                account: Some("DE89370400440532013000".to_owned()),
                reference: Some("INV-1".to_owned()),
            }],
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Service description".to_owned(),
                quantity: DecimalValue::new(Decimal::ONE),
                unit_code: Some("EA".to_owned()),
                unit_price: DecimalValue::new(Decimal::new(10000, 2)),
                line_extension_amount: DecimalValue::new(Decimal::new(10000, 2)),
                tax_category: Some("S".to_owned()),
                classifications: vec![],
                extensions: vec![],
            }],
            tax_summary: vec![],
            monetary_total: MonetaryTotal {
                line_extension_amount: DecimalValue::new(Decimal::new(10000, 2)),
                tax_exclusive_amount: DecimalValue::new(Decimal::new(10000, 2)),
                tax_inclusive_amount: DecimalValue::new(Decimal::new(10000, 2)),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: DecimalValue::new(Decimal::new(10000, 2)),
            },
            attachments: vec![],
            references: vec![],
            notes: vec![],
            extensions: vec![],
            meta: DocumentMeta {
                tenant_id: "tenant".to_owned(),
                trace_id: "trace".to_owned(),
                source_system: None,
            },
        })
        .unwrap()
    }

    #[test]
    fn supplier_and_customer_names_are_redacted() {
        let bundle = redact_for_support(&fixture()).unwrap();
        assert_eq!(bundle.document.supplier.name, REDACTED_PLACEHOLDER);
        assert_eq!(bundle.document.customer.name, REDACTED_PLACEHOLDER);
    }

    #[test]
    fn address_fields_are_redacted_country_kept() {
        let bundle = redact_for_support(&fixture()).unwrap();
        let sup = &bundle.document.supplier.address;
        for line in &sup.lines {
            assert_eq!(line, REDACTED_PLACEHOLDER);
        }
        assert_eq!(sup.city, REDACTED_PLACEHOLDER);
        assert_eq!(sup.postal_code, REDACTED_PLACEHOLDER);
        assert_eq!(sup.subdivision.as_deref(), Some(REDACTED_PLACEHOLDER));
        // Country code stays — required by IR validation and not personal data.
        assert_eq!(sup.country, CountryCode::new("DE").unwrap());
    }

    #[test]
    fn contact_fields_are_redacted_when_present() {
        let bundle = redact_for_support(&fixture()).unwrap();
        let contact = bundle.document.supplier.contact.as_ref().unwrap();
        assert_eq!(contact.name.as_deref(), Some(REDACTED_PLACEHOLDER));
        assert_eq!(contact.email.as_deref(), Some(REDACTED_PLACEHOLDER));
        assert_eq!(contact.phone.as_deref(), Some(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn missing_contact_stays_missing() {
        let bundle = redact_for_support(&fixture()).unwrap();
        // Customer in the fixture has no contact, so the redacted copy
        // must keep contact = None (we don't fabricate empty contacts).
        assert!(bundle.document.customer.contact.is_none());
    }

    #[test]
    fn payment_instruction_account_and_reference_are_redacted() {
        let bundle = redact_for_support(&fixture()).unwrap();
        let inst = &bundle.document.payment_instructions[0];
        assert_eq!(inst.account.as_deref(), Some(REDACTED_PLACEHOLDER));
        assert_eq!(inst.reference.as_deref(), Some(REDACTED_PLACEHOLDER));
        assert_eq!(inst.kind, PaymentInstructionKind::IbanBic);
    }

    #[test]
    fn tax_id_value_redacted_scheme_kept() {
        let bundle = redact_for_support(&fixture()).unwrap();
        let tax = &bundle.document.supplier.tax_ids[0];
        assert_eq!(tax.scheme, "vat");
        assert_eq!(tax.value, REDACTED_PLACEHOLDER);
    }

    #[test]
    fn reproducibility_fields_are_preserved() {
        let original = fixture();
        let bundle = redact_for_support(&original).unwrap();
        assert_eq!(bundle.document.id, original.id);
        assert_eq!(bundle.document.document_number, original.document_number);
        assert_eq!(bundle.document.issue_date, original.issue_date);
        assert_eq!(bundle.document.currency, original.currency);
        assert_eq!(bundle.document.lines, original.lines);
        assert_eq!(bundle.document.monetary_total, original.monetary_total);
        assert_eq!(bundle.document.tax_summary, original.tax_summary);
        assert_eq!(bundle.document.meta, original.meta);
    }

    #[test]
    fn report_lists_every_redacted_field() {
        let bundle = redact_for_support(&fixture()).unwrap();
        let paths: Vec<&str> = bundle
            .report
            .fields_redacted
            .iter()
            .map(String::as_str)
            .collect();
        // Spot-check: supplier name, customer name, supplier first address
        // line, supplier city, supplier contact email, payment account.
        assert!(paths.contains(&"/supplier/name"));
        assert!(paths.contains(&"/customer/name"));
        assert!(paths.contains(&"/supplier/address/lines/0"));
        assert!(paths.contains(&"/supplier/address/city"));
        assert!(paths.contains(&"/supplier/contact/email"));
        assert!(paths.contains(&"/payment_instructions/0/account"));
        // Report length matches the count of redacted fields above plus
        // every other field in the fixture (subdivision, postal_code,
        // tax-id value, etc.).
        assert!(bundle.report.len() >= 12);
        assert!(!bundle.report.is_empty());
    }

    #[test]
    fn redacted_document_validates_against_ir() {
        // The output of redact_for_support must satisfy CommercialDocument::validate
        // (we did not break the invoice shape by replacing field values).
        let bundle = redact_for_support(&fixture()).unwrap();
        bundle.document.validate().expect("redacted IR validates");
    }

    #[test]
    fn redaction_is_idempotent() {
        // Redacting a redacted bundle does not change the document.
        let once = redact_for_support(&fixture()).unwrap().document;
        let twice = redact_for_support(&once).unwrap().document;
        assert_eq!(once, twice);
    }
}
