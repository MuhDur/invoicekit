// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! qovx property-based round-trip tests for `CommercialDocument`'s
//! JSON representation.
//!
//! For every generated valid input shape:
//!
//!   1. Build a `CommercialDocument` via `CommercialDocument::new`.
//!   2. Serialise it via `to_value`.
//!   3. Round-trip via `try_from_value`.
//!   4. Assert the re-parsed document equals the original.
//!   5. Assert the second serialisation is byte-equal (idempotent).
//!
//! Plus focused negative cases that exercise the validation
//! envelope: empty `id`, invalid date, invalid currency, empty
//! `lines` collection.

use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
    DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, IrError, Iso4217Code,
    ItemClassification,
    LocalizedString, MonetaryTotal, Party, PartyTaxId, PaymentInstruction, PaymentInstructionKind,
    PaymentTerms, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use serde_json::json;

const VALID_CURRENCIES: &[&str] = &["EUR", "USD", "GBP", "NOK", "SEK", "DKK"];
const VALID_COUNTRIES: &[&str] = &["DE", "FR", "ES", "IT", "NL", "NO", "SE", "DK", "GB", "US"];
const TAX_CATEGORIES: &[&str] = &["S", "AA", "Z", "E", "AE", "K", "L"];

#[derive(Clone, Debug)]
struct DocConfig {
    document_number: String,
    currency: String,
    supplier_country: &'static str,
    customer_country: &'static str,
    line_count: u32,
    line_quantities: Vec<u32>,
    line_prices_cents: Vec<u64>,
    line_tax_categories: Vec<&'static str>,
    with_payee: bool,
}

fn arb_config() -> impl Strategy<Value = DocConfig> {
    (
        "INV-[0-9]{4}",
        prop::sample::select(VALID_CURRENCIES),
        prop::sample::select(VALID_COUNTRIES),
        prop::sample::select(VALID_COUNTRIES),
        1u32..=3,
        any::<bool>(),
    )
        .prop_flat_map(|(num, currency, sup_c, cust_c, line_count, with_payee)| {
            let count_usize = usize::try_from(line_count).unwrap_or(1);
            (
                Just(num),
                Just(currency),
                Just(sup_c),
                Just(cust_c),
                Just(line_count),
                prop::collection::vec(1u32..=50, count_usize),
                prop::collection::vec(50u64..=50_000, count_usize),
                prop::collection::vec(prop::sample::select(TAX_CATEGORIES), count_usize),
                Just(with_payee),
            )
        })
        .prop_map(
            |(num, currency, sup_c, cust_c, line_count, qtys, prices, taxes, with_payee)| {
                DocConfig {
                    document_number: num,
                    currency: currency.to_owned(),
                    supplier_country: sup_c,
                    customer_country: cust_c,
                    line_count,
                    line_quantities: qtys,
                    line_prices_cents: prices,
                    line_tax_categories: taxes,
                    with_payee,
                }
            },
        )
}

fn party(role: &str, country: &str, idx: u32) -> Party {
    Party {
        id: Some(format!("{role}-{idx}")),
        name: format!("{role} {idx} GmbH"),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: format!("DE{idx:09}"),
        }],
        address: PostalAddress {
            lines: vec![format!("{role} Street {idx}")],
            city: "Berlin".to_owned(),
            subdivision: None,
            postal_code: format!("{:05}", 10_000 + idx),
            country: CountryCode::new(country).unwrap(),
        },
        contact: Some(Contact {
            name: Some(format!("{role} Contact")),
            email: None,
            phone: None,
        }),
    }
}

fn build_document(config: &DocConfig) -> CommercialDocument {
    let mut lines = Vec::new();
    let mut line_total = Decimal::ZERO;
    for i in 0..(config.line_count as usize) {
        let qty = config.line_quantities[i];
        let price_cents = config.line_prices_cents[i];
        let tax = config.line_tax_categories[i];
        let qty_dec = Decimal::new(i64::from(qty), 0);
        let price_dec = Decimal::new(i64::try_from(price_cents).unwrap_or(0), 2);
        let amt = price_dec * qty_dec;
        line_total += amt;
        lines.push(DocumentLine {
            id: format!("L{}", i + 1),
            description: format!("Line item {} qty {qty}", i + 1),
            quantity: DecimalValue::new(qty_dec),
            unit_code: Some("EA".to_owned()),
            unit_price: DecimalValue::new(price_dec),
            line_extension_amount: DecimalValue::new(amt),
            tax_category: Some(tax.to_owned()),
            // Exercise the classification round-trip on every generated line:
            // both scheme ids and a Some/None scheme_version across lines.
            classifications: vec![ItemClassification {
                code: format!("{:04}", 1000 + i),
                scheme_id: if tax == "S" { "SAC".to_owned() } else { "HSN".to_owned() },
                scheme_version: if i % 2 == 0 {
                    Some("2017".to_owned())
                } else {
                    None
                },
            }],
            extensions: Vec::new(),
        });
    }
    let tax_amount = (line_total * Decimal::new(19, 2)).round_dp(2);
    let tax_inclusive = line_total + tax_amount;
    let payee = if config.with_payee {
        Some(party("payee", "DE", 999))
    } else {
        None
    };
    CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,
        id: DocumentId::new(config.document_number.clone()).unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        invoice_period: None,
        delivery_date: None,
        document_number: DocumentNumber::new(config.document_number.clone()).unwrap(),
        currency: Iso4217Code::new(config.currency.clone()).unwrap(),
        supplier: party("supplier", config.supplier_country, 1),
        customer: party("customer", config.customer_country, 2),
        payee,
        payment_terms: Some(PaymentTerms {
            description: "Payable within 30 days.".to_owned(),
            due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        }),
        payment_instructions: vec![PaymentInstruction {
            kind: PaymentInstructionKind::IbanBic,
            account: Some("DE89370400440532013000".to_owned()),
            reference: Some("RF0001".to_owned()),
        }],
        lines,
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: DecimalValue::new(line_total),
            tax_amount: DecimalValue::new(tax_amount),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            // Exercise the exemption fields' round-trip on every generated doc.
            exemption_reason: Some("Exempt under Article 132".to_owned()),
            exemption_reason_code: Some("VATEX-EU-132".to_owned()),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: DecimalValue::new(line_total),
            tax_exclusive_amount: DecimalValue::new(line_total),
            tax_inclusive_amount: DecimalValue::new(tax_inclusive),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: DecimalValue::new(tax_inclusive),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: vec![LocalizedString {
            language: "en".to_owned(),
            text: "Property-test generated.".to_owned(),
        }],
        extensions: Vec::new(),
        allowance_charges: Vec::new(),
        deliver_to: None,
        meta: DocumentMeta {
            tenant_id: "tenant-proptest".to_owned(),
            trace_id: "trace-proptest".to_owned(),
            source_system: None,
        },
    })
    .expect("build_document over valid inputs never fails")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn commercial_document_round_trips_through_json(config in arb_config()) {
        let document = build_document(&config);
        let value = document.to_value().expect("to_value never fails for built docs");
        let parsed = CommercialDocument::try_from_value(value.clone())
            .expect("try_from_value must accept our own output");
        prop_assert_eq!(&parsed, &document);

        let value2 = parsed.to_value().expect("second to_value");
        prop_assert_eq!(value, value2);
    }
}

// --- focused invalid-envelope negatives -----------------------------

#[test]
fn try_from_value_rejects_blank_id() {
    let bad = json!({
        "schema_version": "1.0",
        "id": "",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "document_number": "INV-1",
        "currency": "EUR",
        "supplier": minimal_party("Supplier"),
        "customer": minimal_party("Customer"),
        "payment_instructions": [],
        "lines": [minimal_line()],
        "tax_summary": [],
        "monetary_total": minimal_total(),
        "extensions": [],
        "meta": minimal_meta(),
    });
    let err = CommercialDocument::try_from_value(bad).unwrap_err();
    assert!(
        matches!(err, IrError::MissingRequiredField("id")),
        "unexpected: {err}"
    );
}

#[test]
fn try_from_value_rejects_invalid_currency() {
    let bad = json!({
        "schema_version": "1.0",
        "id": "doc-1",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "document_number": "INV-1",
        "currency": "eur",
        "supplier": minimal_party("Supplier"),
        "customer": minimal_party("Customer"),
        "payment_instructions": [],
        "lines": [minimal_line()],
        "tax_summary": [],
        "monetary_total": minimal_total(),
        "extensions": [],
        "meta": minimal_meta(),
    });
    let err = CommercialDocument::try_from_value(bad).unwrap_err();
    assert!(
        matches!(err, IrError::InvalidCurrency(_)),
        "unexpected: {err}"
    );
}

#[test]
fn try_from_value_rejects_invalid_date() {
    let bad = json!({
        "schema_version": "1.0",
        "id": "doc-1",
        "document_type": "invoice",
        "issue_date": "2026-13-40",
        "document_number": "INV-1",
        "currency": "EUR",
        "supplier": minimal_party("Supplier"),
        "customer": minimal_party("Customer"),
        "payment_instructions": [],
        "lines": [minimal_line()],
        "tax_summary": [],
        "monetary_total": minimal_total(),
        "extensions": [],
        "meta": minimal_meta(),
    });
    let err = CommercialDocument::try_from_value(bad).unwrap_err();
    assert!(matches!(err, IrError::InvalidDate(_)), "unexpected: {err}");
}

#[test]
fn try_from_value_rejects_empty_lines() {
    let bad = json!({
        "schema_version": "1.0",
        "id": "doc-1",
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "document_number": "INV-1",
        "currency": "EUR",
        "supplier": minimal_party("Supplier"),
        "customer": minimal_party("Customer"),
        "payment_instructions": [],
        "lines": [],
        "tax_summary": [],
        "monetary_total": minimal_total(),
        "extensions": [],
        "meta": minimal_meta(),
    });
    let err = CommercialDocument::try_from_value(bad).unwrap_err();
    assert!(
        matches!(err, IrError::EmptyCollection("lines")),
        "unexpected: {err}"
    );
}

fn minimal_party(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "tax_ids": [{"scheme": "vat", "value": "DE123456789"}],
        "address": {
            "lines": ["1 Main"],
            "city": "Berlin",
            "postal_code": "10115",
            "country": "DE",
        },
    })
}

fn minimal_line() -> serde_json::Value {
    json!({
        "id": "L1",
        "description": "Test line",
        "quantity": "1",
        "unit_price": "100.00",
        "line_extension_amount": "100.00",
        "extensions": [],
    })
}

fn minimal_total() -> serde_json::Value {
    json!({
        "line_extension_amount": "100.00",
        "tax_exclusive_amount": "100.00",
        "tax_inclusive_amount": "100.00",
        "payable_amount": "100.00",
    })
}

fn minimal_meta() -> serde_json::Value {
    json!({"tenant_id": "tenant-x", "trace_id": "trace-x"})
}
