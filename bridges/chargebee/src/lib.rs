// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-bridge-chargebee` — Chargebee bridge.
//!
//! Parses Chargebee webhook events, extracts the invoice
//! payload, and surfaces dropped fields via [`LossinessLedger`].
//! Same shape as the Stripe / Lago / Maxio bridges.
//!
//! Supported event types today:
//!
//! * `invoice_generated` — invoice created + ready to send.
//! * `payment_succeeded` — payment captured.
//! * `payment_failed` — payment attempt failed.
//! * `invoice_voided` — invoice voided / credited.
//!
//! Chargebee's webhook envelope is
//! `{ id, event_type, content: { invoice: {...} } }`.
//! Monetary amounts come as integer cents.

use std::collections::BTreeMap;

use invoicekit_ir::{LossinessEntry, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Subset of Chargebee event types this bridge recognises.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargebeeEventKind {
    /// `invoice_generated` — invoice created + sendable.
    InvoiceGenerated,
    /// `payment_succeeded` — payment captured.
    PaymentSucceeded,
    /// `payment_failed` — payment attempt failed.
    PaymentFailed,
    /// `invoice_voided` — invoice voided.
    InvoiceVoided,
    /// Any other Chargebee event type.
    Unhandled,
}

impl ChargebeeEventKind {
    /// Parse from Chargebee's `event_type` string.
    #[must_use]
    pub fn from_event_type(value: &str) -> Self {
        match value {
            "invoice_generated" => Self::InvoiceGenerated,
            "payment_succeeded" => Self::PaymentSucceeded,
            "payment_failed" => Self::PaymentFailed,
            "invoice_voided" => Self::InvoiceVoided,
            _ => Self::Unhandled,
        }
    }
}

/// Typed envelope for a Chargebee webhook event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ChargebeeWebhookEvent {
    /// Chargebee event id (`ev_...`).
    pub id: String,
    /// Chargebee event type (`invoice_generated`, ...).
    pub event_type: String,
    /// Webhook content envelope.
    pub content: ChargebeeContent,
}

impl ChargebeeWebhookEvent {
    /// Convenience: parsed `event_type` mapped to the typed
    /// [`ChargebeeEventKind`].
    #[must_use]
    pub fn kind(&self) -> ChargebeeEventKind {
        ChargebeeEventKind::from_event_type(&self.event_type)
    }
}

/// `content` envelope.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ChargebeeContent {
    /// Invoice resource.
    pub invoice: ChargebeeInvoice,
    /// Customer block (Chargebee nests it next to invoice).
    pub customer: ChargebeeCustomer,
}

/// Subset of Chargebee's `Invoice` object we read.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ChargebeeInvoice {
    /// Chargebee invoice id (`INV_...`).
    pub id: String,
    /// Customer id (`cus_...`).
    pub customer_id: String,
    /// Currency code (lowercase per Chargebee, e.g. `usd`).
    pub currency_code: String,
    /// Status (`paid`, `posted`, `payment_due`, `not_paid`,
    /// `voided`, `pending`).
    pub status: String,
    /// Tax-exclusive subtotal in the smallest currency unit.
    pub sub_total: i64,
    /// Total amount in the smallest currency unit.
    pub total: i64,
    /// Tax amount in the smallest currency unit.
    #[serde(default)]
    pub tax: i64,
    /// Issue epoch (Unix seconds).
    pub date: i64,
    /// Due epoch.
    #[serde(default)]
    pub due_date: Option<i64>,
    /// Line items.
    #[serde(default)]
    pub line_items: Vec<ChargebeeLineItem>,
    /// Hosted invoice URL.
    #[serde(default)]
    pub hosted_invoice_url: Option<String>,
    /// Arbitrary metadata.
    #[serde(default)]
    pub meta_data: BTreeMap<String, String>,
}

/// Chargebee customer record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ChargebeeCustomer {
    /// Customer id.
    pub id: String,
    /// Display first name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Display last name.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Company name.
    #[serde(default)]
    pub company: Option<String>,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// Tax ID (VAT / GST / EIN).
    #[serde(default)]
    pub vat_number: Option<String>,
    /// Billing address.
    #[serde(default)]
    pub billing_address: Option<ChargebeeAddress>,
}

/// Chargebee address record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ChargebeeAddress {
    /// Address line 1.
    #[serde(default)]
    pub line1: Option<String>,
    /// City.
    #[serde(default)]
    pub city: Option<String>,
    /// State / province.
    #[serde(default)]
    pub state: Option<String>,
    /// Zip / postal code.
    #[serde(default)]
    pub zip: Option<String>,
    /// ISO 3166-1 alpha-2 country code.
    #[serde(default)]
    pub country: Option<String>,
}

/// Chargebee line item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct ChargebeeLineItem {
    /// Line id.
    pub id: String,
    /// Display description.
    pub description: String,
    /// Quantity (Chargebee uses integers).
    #[serde(default = "one")]
    pub quantity: i64,
    /// Unit amount in smallest currency unit.
    #[serde(default)]
    pub unit_amount: Option<i64>,
    /// Line subtotal in smallest currency unit.
    pub amount: i64,
}

const fn one() -> i64 {
    1
}

/// One-line summary of the Chargebee invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChargebeeInvoiceSummary {
    /// Chargebee invoice id.
    pub chargebee_invoice_id: String,
    /// Document number (uses the Chargebee id as the display
    /// number — Chargebee does not separate them).
    pub document_number: String,
    /// Uppercased currency code.
    pub currency: String,
    /// Subtotal decimal.
    pub subtotal_decimal: String,
    /// Tax decimal.
    pub tax_decimal: String,
    /// Total decimal.
    pub total_decimal: String,
    /// Issue date (`YYYY-MM-DD` UTC).
    pub issue_date: String,
    /// Optional due date.
    pub due_date: Option<String>,
    /// Customer summary.
    pub customer: ChargebeeCustomer,
    /// Per-line summaries.
    pub lines: Vec<ChargebeeLineSummary>,
}

/// Per-line summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChargebeeLineSummary {
    /// Chargebee line id.
    pub chargebee_line_id: String,
    /// Display description.
    pub description: String,
    /// Quantity.
    pub quantity: i64,
    /// Unit price decimal.
    pub unit_price_decimal: String,
    /// Line total decimal.
    pub line_total_decimal: String,
}

/// Outcome of [`extract_invoice_summary`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranslationOutcome {
    /// Extracted summary.
    pub summary: ChargebeeInvoiceSummary,
    /// Lossiness ledger.
    pub lossiness: LossinessLedger,
}

/// Errors raised by the bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Webhook body could not be parsed as JSON.
    #[error("chargebee webhook body parse failed: {0}")]
    Parse(String),
    /// Chargebee's `currency_code` field was missing or empty.
    #[error("chargebee invoice missing currency_code")]
    MissingCurrency,
}

/// Parse a raw Chargebee webhook body.
///
/// # Errors
///
/// Returns [`BridgeError::Parse`] when the body is not a valid
/// Chargebee `webhook` shape.
pub fn parse_event(body: &str) -> Result<ChargebeeWebhookEvent, BridgeError> {
    serde_json::from_str(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

/// Lift a Chargebee invoice into the typed
/// [`ChargebeeInvoiceSummary`] + [`LossinessLedger`].
///
/// # Errors
///
/// Returns [`BridgeError::MissingCurrency`] when the Chargebee
/// invoice carries an empty currency code.
pub fn extract_invoice_summary(
    invoice: &ChargebeeInvoice,
    customer: &ChargebeeCustomer,
) -> Result<TranslationOutcome, BridgeError> {
    if invoice.currency_code.is_empty() {
        return Err(BridgeError::MissingCurrency);
    }

    let currency = invoice.currency_code.to_uppercase();
    let exponent = currency_minor_unit_exponent(&currency);
    let subtotal_decimal = minor_units_to_decimal(invoice.sub_total, exponent);
    let tax_decimal = minor_units_to_decimal(invoice.tax, exponent);
    let total_decimal = minor_units_to_decimal(invoice.total, exponent);
    let issue_date = unix_to_yyyy_mm_dd_utc(invoice.date);
    let due_date = invoice.due_date.map(unix_to_yyyy_mm_dd_utc);

    let mut preserved: Vec<LossinessEntry> = Vec::new();
    let mut lost: Vec<LossinessEntry> = Vec::new();

    preserved.push(LossinessEntry {
        path: "/id".to_owned(),
        reason: "Chargebee invoice id preserved as document_number".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/currency_code".to_owned(),
        reason: "currency_code lifted to InvoiceKit currency".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/total".to_owned(),
        reason: "total lifted to monetary_total.payable_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/sub_total".to_owned(),
        reason: "sub_total lifted to monetary_total.tax_exclusive_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/line_items".to_owned(),
        reason: format!(
            "{} line item(s) lifted to commercial_document.lines",
            invoice.line_items.len()
        ),
    });

    if invoice.hosted_invoice_url.is_some() {
        lost.push(LossinessEntry {
            path: "/hosted_invoice_url".to_owned(),
            reason: "Chargebee hosted-invoice URL is not part of the InvoiceKit IR".to_owned(),
        });
    }
    for key in invoice.meta_data.keys() {
        lost.push(LossinessEntry {
            path: format!("/meta_data/{key}"),
            reason: "Chargebee meta_data key not part of the IR".to_owned(),
        });
    }

    let lines: Vec<ChargebeeLineSummary> = invoice
        .line_items
        .iter()
        .map(|line| extract_line(line, exponent))
        .collect();

    Ok(TranslationOutcome {
        summary: ChargebeeInvoiceSummary {
            chargebee_invoice_id: invoice.id.clone(),
            document_number: invoice.id.clone(),
            currency,
            subtotal_decimal,
            tax_decimal,
            total_decimal,
            issue_date,
            due_date,
            customer: customer.clone(),
            lines,
        },
        lossiness: LossinessLedger {
            preserved,
            lost,
            ..Default::default()
        },
    })
}

fn extract_line(line: &ChargebeeLineItem, exponent: u32) -> ChargebeeLineSummary {
    let quantity = line.quantity.max(1);
    let unit_price_decimal = line.unit_amount.map_or_else(
        || minor_units_to_decimal(line.amount / quantity, exponent),
        |a| minor_units_to_decimal(a, exponent),
    );
    ChargebeeLineSummary {
        chargebee_line_id: line.id.clone(),
        description: line.description.clone(),
        quantity,
        unit_price_decimal,
        line_total_decimal: minor_units_to_decimal(line.amount, exponent),
    }
}

/// Currency minor-unit exponent — same table as Stripe/Lago.
#[must_use]
pub fn currency_minor_unit_exponent(currency: &str) -> u32 {
    match currency {
        "BIF" | "CLP" | "DJF" | "GNF" | "JPY" | "KMF" | "KRW" | "MGA" | "PYG" | "RWF" | "UGX"
        | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "JOD" | "KWD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

fn minor_units_to_decimal(minor_units: i64, exponent: u32) -> String {
    let pow = 10_i64.pow(exponent);
    let negative = minor_units < 0;
    let abs = minor_units.unsigned_abs();
    let pow_u = pow.unsigned_abs();
    let whole = abs / pow_u;
    let fractional = abs % pow_u;
    let sign = if negative { "-" } else { "" };
    if exponent == 0 {
        format!("{sign}{whole}")
    } else {
        format!(
            "{sign}{whole}.{fractional:0width$}",
            width = exponent as usize
        )
    }
}

fn unix_to_yyyy_mm_dd_utc(epoch: i64) -> String {
    let mut days = epoch.div_euclid(86_400);
    let mut year: i64 = 1970;
    loop {
        let year_days: i64 = if is_leap_year(year) { 366 } else { 365 };
        if days >= year_days {
            days -= year_days;
            year += 1;
        } else if days < 0 {
            year -= 1;
            let prev_year_days: i64 = if is_leap_year(year) { 366 } else { 365 };
            days += prev_year_days;
        } else {
            break;
        }
    }
    let months: [u8; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month: usize = 0;
    while month < 12 && days >= i64::from(months[month]) {
        days -= i64::from(months[month]);
        month += 1;
    }
    let day = days + 1;
    format!("{year:04}-{:02}-{day:02}", month + 1)
}

const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_bridge_chargebee::crate_name(),
///     "invoicekit-bridge-chargebee"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bridge-chargebee"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_body() -> &'static str {
        r#"{
            "id": "ev_chargebee_abc",
            "event_type": "invoice_generated",
            "content": {
              "invoice": {
                "id": "INV_CB_42",
                "customer_id": "cus_acme",
                "currency_code": "eur",
                "status": "posted",
                "sub_total": 100000,
                "total": 119000,
                "tax": 19000,
                "date": 1748275200,
                "due_date": 1750867200,
                "line_items": [
                  {
                    "id": "li_a",
                    "description": "Monthly subscription",
                    "quantity": 1,
                    "unit_amount": 90000,
                    "amount": 90000
                  },
                  {
                    "id": "li_b",
                    "description": "Usage overage",
                    "quantity": 200,
                    "unit_amount": 50,
                    "amount": 10000
                  }
                ],
                "hosted_invoice_url": "https://acme.chargebee.example/invoices/abc",
                "meta_data": {"order_id": "ord_42"}
              },
              "customer": {
                "id": "cus_acme",
                "first_name": "Acme",
                "last_name": "Buyer",
                "company": "Acme GmbH",
                "email": "billing@acme.example",
                "vat_number": "DE123456789",
                "billing_address": {
                  "line1": "1 Acme Way",
                  "city": "Berlin",
                  "zip": "10115",
                  "country": "DE"
                }
              }
            }
        }"#
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-bridge-chargebee");
    }

    #[test]
    fn event_kind_round_trips() {
        assert_eq!(
            ChargebeeEventKind::from_event_type("invoice_generated"),
            ChargebeeEventKind::InvoiceGenerated
        );
        assert_eq!(
            ChargebeeEventKind::from_event_type("payment_succeeded"),
            ChargebeeEventKind::PaymentSucceeded
        );
        assert_eq!(
            ChargebeeEventKind::from_event_type("payment_failed"),
            ChargebeeEventKind::PaymentFailed
        );
        assert_eq!(
            ChargebeeEventKind::from_event_type("invoice_voided"),
            ChargebeeEventKind::InvoiceVoided
        );
        assert_eq!(
            ChargebeeEventKind::from_event_type("customer_created"),
            ChargebeeEventKind::Unhandled
        );
    }

    #[test]
    fn parse_event_extracts_typed_envelope() {
        let event = parse_event(webhook_body()).unwrap();
        assert_eq!(event.id, "ev_chargebee_abc");
        assert_eq!(event.event_type, "invoice_generated");
        assert_eq!(event.kind(), ChargebeeEventKind::InvoiceGenerated);
        assert_eq!(event.content.invoice.id, "INV_CB_42");
    }

    #[test]
    fn parse_event_rejects_malformed_body() {
        let err = parse_event("not json").unwrap_err();
        assert!(matches!(err, BridgeError::Parse(_)));
    }

    #[test]
    fn extract_invoice_summary_happy_path() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome =
            extract_invoice_summary(&event.content.invoice, &event.content.customer).unwrap();
        let s = &outcome.summary;
        assert_eq!(s.chargebee_invoice_id, "INV_CB_42");
        assert_eq!(s.document_number, "INV_CB_42");
        assert_eq!(s.currency, "EUR");
        assert_eq!(s.subtotal_decimal, "1000.00");
        assert_eq!(s.tax_decimal, "190.00");
        assert_eq!(s.total_decimal, "1190.00");
        assert_eq!(s.issue_date, "2025-05-26");
        assert_eq!(s.due_date.as_deref(), Some("2025-06-25"));
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[1].quantity, 200);
        assert_eq!(s.lines[1].unit_price_decimal, "0.50");
        assert_eq!(s.lines[1].line_total_decimal, "100.00");
        assert_eq!(s.customer.vat_number.as_deref(), Some("DE123456789"));
    }

    #[test]
    fn extract_invoice_summary_records_lossiness() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome =
            extract_invoice_summary(&event.content.invoice, &event.content.customer).unwrap();
        let has_lost = |p: &str| outcome.lossiness.lost.iter().any(|e| e.path == p);
        let has_preserved = |p: &str| outcome.lossiness.preserved.iter().any(|e| e.path == p);
        assert!(has_lost("/hosted_invoice_url"));
        assert!(has_lost("/meta_data/order_id"));
        assert!(has_preserved("/currency_code"));
        assert!(has_preserved("/total"));
        assert!(has_preserved("/sub_total"));
        assert!(has_preserved("/line_items"));
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_currency() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.content.invoice.currency_code = String::new();
        let err =
            extract_invoice_summary(&event.content.invoice, &event.content.customer).unwrap_err();
        assert!(matches!(err, BridgeError::MissingCurrency));
    }

    #[test]
    fn currency_minor_unit_exponent_handles_zero_and_three_decimal_currencies() {
        assert_eq!(currency_minor_unit_exponent("JPY"), 0);
        assert_eq!(currency_minor_unit_exponent("BHD"), 3);
        assert_eq!(currency_minor_unit_exponent("EUR"), 2);
    }
}
