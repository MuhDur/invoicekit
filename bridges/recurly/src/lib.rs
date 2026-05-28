// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-bridge-recurly` — Recurly bridge.
//!
//! Parses Recurly webhook events, extracts the invoice
//! payload, and surfaces dropped fields via [`LossinessLedger`].
//! Same shape as the Stripe / Lago / Maxio / Chargebee bridges.
//!
//! Supported event types today:
//!
//! * `new_invoice_notification` — invoice created.
//! * `paid_charge_invoice_notification` — payment captured.
//! * `failed_charge_invoice_notification` — payment failed.
//! * `void_charge_invoice_notification` — invoice voided.
//!
//! Recurly webhook bodies are JSON envelopes
//! `{ event_type, invoice: { ... }, account: { ... } }`.
//! Monetary amounts come as decimal strings in the major
//! unit (Recurly's `*_in_cents` integer fields shadow them,
//! but the bridge uses the decimal strings to avoid currency-
//! specific minor-unit conversion).

use std::collections::BTreeMap;

use invoicekit_ir::{LossinessEntry, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Subset of Recurly event types this bridge recognises.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurlyEventKind {
    /// `new_invoice_notification` — invoice created.
    NewInvoice,
    /// `paid_charge_invoice_notification` — payment captured.
    PaidChargeInvoice,
    /// `failed_charge_invoice_notification` — payment failed.
    FailedChargeInvoice,
    /// `void_charge_invoice_notification` — invoice voided.
    VoidChargeInvoice,
    /// Any other Recurly event type.
    Unhandled,
}

impl RecurlyEventKind {
    /// Parse from Recurly's `event_type` string.
    #[must_use]
    pub fn from_event_type(value: &str) -> Self {
        match value {
            "new_invoice_notification" => Self::NewInvoice,
            "paid_charge_invoice_notification" => Self::PaidChargeInvoice,
            "failed_charge_invoice_notification" => Self::FailedChargeInvoice,
            "void_charge_invoice_notification" => Self::VoidChargeInvoice,
            _ => Self::Unhandled,
        }
    }
}

/// Typed envelope for a Recurly webhook event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct RecurlyWebhookEvent {
    /// Recurly event type (`new_invoice_notification`, ...).
    pub event_type: String,
    /// Invoice resource.
    pub invoice: RecurlyInvoice,
    /// Account resource (Recurly's customer block).
    pub account: RecurlyAccount,
}

impl RecurlyWebhookEvent {
    /// Convenience: parsed `event_type` mapped to the typed
    /// [`RecurlyEventKind`].
    #[must_use]
    pub fn kind(&self) -> RecurlyEventKind {
        RecurlyEventKind::from_event_type(&self.event_type)
    }
}

/// Subset of Recurly's `Invoice` object we read.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct RecurlyInvoice {
    /// Recurly invoice uuid.
    pub uuid: String,
    /// Sequential invoice number Recurly assigns at finalize.
    pub invoice_number: i64,
    /// Currency code (uppercase per Recurly, e.g. `USD`).
    pub currency: String,
    /// State (`open`, `paid`, `failed`, `past_due`, `voided`).
    pub state: String,
    /// Subtotal as a decimal string in the major unit.
    pub subtotal: String,
    /// Tax amount as a decimal string.
    pub tax: String,
    /// Total amount as a decimal string.
    pub total: String,
    /// Created-at timestamp (Recurly emits RFC 3339).
    pub created_at: String,
    /// Optional due-on date (RFC 3339).
    #[serde(default)]
    pub due_on: Option<String>,
    /// Line items (Recurly calls them "adjustments").
    #[serde(default)]
    pub line_items: Vec<RecurlyLineItem>,
    /// PDF URL.
    #[serde(default)]
    pub invoice_pdf_url: Option<String>,
    /// Arbitrary custom fields the operator attached.
    #[serde(default)]
    pub custom_fields: BTreeMap<String, String>,
}

/// Recurly account record (customer-side block).
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RecurlyAccount {
    /// Operator-side account code.
    pub account_code: String,
    /// Display first name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Display last name.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Company name.
    #[serde(default)]
    pub company_name: Option<String>,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// VAT / EIN / GST number.
    #[serde(default)]
    pub vat_number: Option<String>,
    /// Billing address.
    #[serde(default)]
    pub address: Option<RecurlyAddress>,
}

/// Recurly address record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RecurlyAddress {
    /// Street address line 1.
    #[serde(default)]
    pub street1: Option<String>,
    /// Street address line 2.
    #[serde(default)]
    pub street2: Option<String>,
    /// City.
    #[serde(default)]
    pub city: Option<String>,
    /// State / region.
    #[serde(default)]
    pub region: Option<String>,
    /// Postal code.
    #[serde(default)]
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2 country code.
    #[serde(default)]
    pub country: Option<String>,
}

/// Recurly line item (adjustment).
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct RecurlyLineItem {
    /// Adjustment uuid.
    pub uuid: String,
    /// Display description.
    pub description: String,
    /// Quantity (Recurly uses integers).
    #[serde(default = "one")]
    pub quantity: i64,
    /// Unit amount as decimal string in the major unit.
    pub unit_amount: String,
    /// Subtotal as decimal string.
    pub subtotal: String,
}

const fn one() -> i64 {
    1
}

/// One-line summary of the Recurly invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecurlyInvoiceSummary {
    /// Recurly invoice uuid.
    pub recurly_invoice_uuid: String,
    /// Document number (the human-readable Recurly invoice
    /// number, formatted as a decimal string).
    pub document_number: String,
    /// Uppercased currency code.
    pub currency: String,
    /// Subtotal decimal.
    pub subtotal_decimal: String,
    /// Tax decimal.
    pub tax_decimal: String,
    /// Total decimal.
    pub total_decimal: String,
    /// Issue date (`YYYY-MM-DD` UTC, extracted from
    /// `created_at`).
    pub issue_date: String,
    /// Optional due date (`YYYY-MM-DD` UTC).
    pub due_date: Option<String>,
    /// Customer / account summary.
    pub account: RecurlyAccount,
    /// Per-line summaries.
    pub lines: Vec<RecurlyLineSummary>,
}

/// Per-line summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecurlyLineSummary {
    /// Adjustment uuid.
    pub recurly_line_uuid: String,
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
    pub summary: RecurlyInvoiceSummary,
    /// Lossiness ledger.
    pub lossiness: LossinessLedger,
}

/// Errors raised by the bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Webhook body could not be parsed as JSON.
    #[error("recurly webhook body parse failed: {0}")]
    Parse(String),
    /// Recurly's `currency` field was missing or empty.
    #[error("recurly invoice missing currency")]
    MissingCurrency,
}

/// Parse a raw Recurly webhook body.
///
/// # Errors
///
/// Returns [`BridgeError::Parse`] when the body is not a valid
/// Recurly `webhook` shape.
pub fn parse_event(body: &str) -> Result<RecurlyWebhookEvent, BridgeError> {
    serde_json::from_str(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

/// Lift a Recurly invoice into the typed
/// [`RecurlyInvoiceSummary`] + [`LossinessLedger`].
///
/// # Errors
///
/// Returns [`BridgeError::MissingCurrency`] when the Recurly
/// invoice carries an empty currency.
pub fn extract_invoice_summary(
    invoice: &RecurlyInvoice,
    account: &RecurlyAccount,
) -> Result<TranslationOutcome, BridgeError> {
    if invoice.currency.is_empty() {
        return Err(BridgeError::MissingCurrency);
    }

    let currency = invoice.currency.to_uppercase();
    let issue_date = take_date_prefix(&invoice.created_at);
    let due_date = invoice.due_on.as_deref().map(take_date_prefix);

    let mut preserved: Vec<LossinessEntry> = Vec::new();
    let mut lost: Vec<LossinessEntry> = Vec::new();

    preserved.push(LossinessEntry {
        path: "/uuid".to_owned(),
        reason: "Recurly invoice uuid preserved as audit ref".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/invoice_number".to_owned(),
        reason: "invoice_number lifted to document_number".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/currency".to_owned(),
        reason: "currency lifted to InvoiceKit currency".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/total".to_owned(),
        reason: "total lifted to monetary_total.payable_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/subtotal".to_owned(),
        reason: "subtotal lifted to monetary_total.tax_exclusive_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/line_items".to_owned(),
        reason: format!(
            "{} adjustment(s) lifted to commercial_document.lines",
            invoice.line_items.len()
        ),
    });

    if invoice.invoice_pdf_url.is_some() {
        lost.push(LossinessEntry {
            path: "/invoice_pdf_url".to_owned(),
            reason: "Recurly-rendered PDF is replaced by the InvoiceKit-rendered PDF".to_owned(),
        });
    }
    for key in invoice.custom_fields.keys() {
        lost.push(LossinessEntry {
            path: format!("/custom_fields/{key}"),
            reason: "Recurly custom field not part of the IR".to_owned(),
        });
    }

    let lines: Vec<RecurlyLineSummary> = invoice.line_items.iter().map(extract_line).collect();

    Ok(TranslationOutcome {
        summary: RecurlyInvoiceSummary {
            recurly_invoice_uuid: invoice.uuid.clone(),
            document_number: invoice.invoice_number.to_string(),
            currency,
            subtotal_decimal: invoice.subtotal.clone(),
            tax_decimal: invoice.tax.clone(),
            total_decimal: invoice.total.clone(),
            issue_date,
            due_date,
            account: account.clone(),
            lines,
        },
        lossiness: LossinessLedger {
            preserved,
            lost,
            ..Default::default()
        },
    })
}

fn extract_line(line: &RecurlyLineItem) -> RecurlyLineSummary {
    RecurlyLineSummary {
        recurly_line_uuid: line.uuid.clone(),
        description: line.description.clone(),
        quantity: line.quantity.max(1),
        unit_price_decimal: line.unit_amount.clone(),
        line_total_decimal: line.subtotal.clone(),
    }
}

/// Take the `YYYY-MM-DD` prefix from a Recurly timestamp like
/// `2026-05-28T10:30:00Z`. Falls back to the full string when
/// it doesn't start with a date-shaped prefix.
fn take_date_prefix(timestamp: &str) -> String {
    if timestamp.len() >= 10
        && timestamp
            .chars()
            .take(10)
            .enumerate()
            .all(|(i, c)| match i {
                4 | 7 => c == '-',
                _ => c.is_ascii_digit(),
            })
    {
        timestamp[..10].to_owned()
    } else {
        timestamp.to_owned()
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_bridge_recurly::crate_name(),
///     "invoicekit-bridge-recurly"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bridge-recurly"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_body() -> &'static str {
        r#"{
            "event_type": "new_invoice_notification",
            "invoice": {
              "uuid": "inv-recurly-abc",
              "invoice_number": 1042,
              "currency": "USD",
              "state": "open",
              "subtotal": "1000.00",
              "tax": "85.00",
              "total": "1085.00",
              "created_at": "2026-05-28T10:30:00Z",
              "due_on": "2026-06-27T10:30:00Z",
              "line_items": [
                {
                  "uuid": "li-a",
                  "description": "Monthly subscription",
                  "quantity": 1,
                  "unit_amount": "900.00",
                  "subtotal": "900.00"
                },
                {
                  "uuid": "li-b",
                  "description": "Usage overage",
                  "quantity": 200,
                  "unit_amount": "0.50",
                  "subtotal": "100.00"
                }
              ],
              "invoice_pdf_url": "https://acme.recurly.example/invoices/abc.pdf",
              "custom_fields": {"order_id": "ord-42"}
            },
            "account": {
              "account_code": "acme-acct-7",
              "first_name": "Acme",
              "last_name": "Buyer",
              "company_name": "Acme GmbH",
              "email": "billing@acme.example",
              "vat_number": "DE123456789",
              "address": {
                "street1": "1 Acme Way",
                "city": "Berlin",
                "postal_code": "10115",
                "country": "DE"
              }
            }
        }"#
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-bridge-recurly");
    }

    #[test]
    fn event_kind_round_trips() {
        assert_eq!(
            RecurlyEventKind::from_event_type("new_invoice_notification"),
            RecurlyEventKind::NewInvoice
        );
        assert_eq!(
            RecurlyEventKind::from_event_type("paid_charge_invoice_notification"),
            RecurlyEventKind::PaidChargeInvoice
        );
        assert_eq!(
            RecurlyEventKind::from_event_type("failed_charge_invoice_notification"),
            RecurlyEventKind::FailedChargeInvoice
        );
        assert_eq!(
            RecurlyEventKind::from_event_type("void_charge_invoice_notification"),
            RecurlyEventKind::VoidChargeInvoice
        );
        assert_eq!(
            RecurlyEventKind::from_event_type("subscription_created"),
            RecurlyEventKind::Unhandled
        );
    }

    #[test]
    fn parse_event_extracts_typed_envelope() {
        let event = parse_event(webhook_body()).unwrap();
        assert_eq!(event.event_type, "new_invoice_notification");
        assert_eq!(event.kind(), RecurlyEventKind::NewInvoice);
        assert_eq!(event.invoice.uuid, "inv-recurly-abc");
        assert_eq!(event.account.account_code, "acme-acct-7");
    }

    #[test]
    fn parse_event_rejects_malformed_body() {
        let err = parse_event("not json").unwrap_err();
        assert!(matches!(err, BridgeError::Parse(_)));
    }

    #[test]
    fn extract_invoice_summary_happy_path() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.invoice, &event.account).unwrap();
        let s = &outcome.summary;
        assert_eq!(s.recurly_invoice_uuid, "inv-recurly-abc");
        assert_eq!(s.document_number, "1042");
        assert_eq!(s.currency, "USD");
        assert_eq!(s.subtotal_decimal, "1000.00");
        assert_eq!(s.tax_decimal, "85.00");
        assert_eq!(s.total_decimal, "1085.00");
        assert_eq!(s.issue_date, "2026-05-28");
        assert_eq!(s.due_date.as_deref(), Some("2026-06-27"));
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[1].quantity, 200);
        assert_eq!(s.lines[1].unit_price_decimal, "0.50");
        assert_eq!(s.account.vat_number.as_deref(), Some("DE123456789"));
    }

    #[test]
    fn extract_invoice_summary_records_lossiness() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.invoice, &event.account).unwrap();
        let has_lost = |p: &str| outcome.lossiness.lost.iter().any(|e| e.path == p);
        let has_preserved = |p: &str| outcome.lossiness.preserved.iter().any(|e| e.path == p);
        assert!(has_lost("/invoice_pdf_url"));
        assert!(has_lost("/custom_fields/order_id"));
        assert!(has_preserved("/uuid"));
        assert!(has_preserved("/currency"));
        assert!(has_preserved("/total"));
        assert!(has_preserved("/line_items"));
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_currency() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.invoice.currency = String::new();
        let err = extract_invoice_summary(&event.invoice, &event.account).unwrap_err();
        assert!(matches!(err, BridgeError::MissingCurrency));
    }

    #[test]
    fn take_date_prefix_extracts_yyyy_mm_dd() {
        assert_eq!(take_date_prefix("2026-05-28T10:30:00Z"), "2026-05-28");
        assert_eq!(take_date_prefix("2026-12-31"), "2026-12-31");
        // Falls back when the input doesn't look like a date.
        assert_eq!(take_date_prefix("not-a-date"), "not-a-date");
    }
}
