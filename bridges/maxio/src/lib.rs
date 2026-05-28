// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-bridge-maxio` — Maxio (Chargify + `SaaSOptics`)
//! bridge.
//!
//! Parses Maxio webhook events, extracts the invoice payload,
//! and surfaces dropped fields via [`LossinessLedger`]. Same
//! shape as the Stripe + Lago bridges so engine-side code can
//! dispatch on a common surface.
//!
//! Supported event types today:
//!
//! * `invoice_issued` — finalized invoice ready for delivery.
//! * `payment_success` — successful payment captured.
//! * `payment_failure` — payment attempt failed.
//! * `invoice_voided` — refund / credit-note kickoff.
//!
//! Maxio's webhook envelope is
//! `{ event, payload: { invoice: {...} } }`; the invoice payload
//! lives under `payload.invoice`. Monetary amounts come as
//! decimal strings in the major unit (Maxio does not use
//! integer cents).

use std::collections::BTreeMap;

use invoicekit_ir::{LossinessEntry, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Subset of Maxio event types this bridge recognises.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxioEventKind {
    /// `invoice_issued` — invoice finalized + ready to send.
    InvoiceIssued,
    /// `payment_success` — payment captured.
    PaymentSuccess,
    /// `payment_failure` — payment attempt failed.
    PaymentFailure,
    /// `invoice_voided` — invoice voided / credited.
    InvoiceVoided,
    /// Any other Maxio event type.
    Unhandled,
}

impl MaxioEventKind {
    /// Parse from Maxio's `event` string.
    #[must_use]
    pub fn from_event(value: &str) -> Self {
        match value {
            "invoice_issued" => Self::InvoiceIssued,
            "payment_success" => Self::PaymentSuccess,
            "payment_failure" => Self::PaymentFailure,
            "invoice_voided" => Self::InvoiceVoided,
            _ => Self::Unhandled,
        }
    }
}

/// Typed envelope for a Maxio webhook event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct MaxioWebhookEvent {
    /// Maxio event name (`invoice_issued`, ...).
    pub event: String,
    /// Webhook payload.
    pub payload: MaxioWebhookPayload,
}

impl MaxioWebhookEvent {
    /// Convenience: parsed `event` mapped to the typed
    /// [`MaxioEventKind`].
    #[must_use]
    pub fn kind(&self) -> MaxioEventKind {
        MaxioEventKind::from_event(&self.event)
    }
}

/// Maxio wraps the resource under `payload.invoice` for
/// invoice events.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct MaxioWebhookPayload {
    /// Invoice resource.
    pub invoice: MaxioInvoice,
}

/// Subset of Maxio's `Invoice` object we read.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct MaxioInvoice {
    /// Maxio's stable invoice UID (`inv_...`).
    pub uid: String,
    /// Display number Maxio assigns (`1042`, `INV-2026-0042`).
    pub number: String,
    /// Issue date (`YYYY-MM-DD`).
    pub issue_date: String,
    /// Optional due date.
    #[serde(default)]
    pub due_date: Option<String>,
    /// Currency code (uppercased per Maxio, e.g. `USD`).
    pub currency: String,
    /// Status (`open`, `paid`, `voided`, `pending`).
    pub status: String,
    /// Subtotal as a decimal string in the major unit.
    pub subtotal_amount: String,
    /// Tax amount as a decimal string.
    pub tax_amount: String,
    /// Total amount as a decimal string.
    pub total_amount: String,
    /// Customer block (Maxio nests this).
    pub customer: MaxioCustomer,
    /// Line items.
    #[serde(default)]
    pub line_items: Vec<MaxioLineItem>,
    /// Public link Maxio renders the invoice at.
    #[serde(default)]
    pub public_url: Option<String>,
    /// PDF download URL.
    #[serde(default)]
    pub pdf_url: Option<String>,
    /// Arbitrary metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Maxio customer record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct MaxioCustomer {
    /// Customer id (`cus_...`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// Address line 1.
    #[serde(default)]
    pub address_line1: Option<String>,
    /// City.
    #[serde(default)]
    pub city: Option<String>,
    /// State / province.
    #[serde(default)]
    pub state: Option<String>,
    /// Postal / zip code.
    #[serde(default)]
    pub zip: Option<String>,
    /// ISO 3166-1 alpha-2 country code.
    #[serde(default)]
    pub country: Option<String>,
    /// VAT / EIN / GST tax number, when collected.
    #[serde(default)]
    pub tax_number: Option<String>,
}

/// Maxio line item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct MaxioLineItem {
    /// Line item uid.
    pub uid: String,
    /// Display title.
    pub title: String,
    /// Optional product description.
    #[serde(default)]
    pub description: Option<String>,
    /// Quantity (Maxio uses decimal strings).
    #[serde(default = "default_quantity")]
    pub quantity: String,
    /// Unit price decimal string.
    pub unit_price: String,
    /// Line subtotal decimal string.
    pub subtotal_amount: String,
}

fn default_quantity() -> String {
    "1".to_owned()
}

/// One-line summary of the Maxio invoice in InvoiceKit terms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MaxioInvoiceSummary {
    /// Maxio invoice uid.
    pub maxio_invoice_uid: String,
    /// Display number.
    pub document_number: String,
    /// Uppercased currency code.
    pub currency: String,
    /// Subtotal decimal.
    pub subtotal_decimal: String,
    /// Tax decimal.
    pub tax_decimal: String,
    /// Total decimal.
    pub total_decimal: String,
    /// Issue date.
    pub issue_date: String,
    /// Optional due date.
    pub due_date: Option<String>,
    /// Customer summary (passed through).
    pub customer: MaxioCustomer,
    /// Per-line summaries.
    pub lines: Vec<MaxioLineSummary>,
}

/// Per-line summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MaxioLineSummary {
    /// Maxio line uid.
    pub maxio_line_uid: String,
    /// Display description.
    pub description: String,
    /// Quantity decimal string.
    pub quantity: String,
    /// Unit price decimal.
    pub unit_price_decimal: String,
    /// Line total decimal.
    pub line_total_decimal: String,
}

/// Outcome of [`extract_invoice_summary`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranslationOutcome {
    /// Extracted summary.
    pub summary: MaxioInvoiceSummary,
    /// Lossiness ledger.
    pub lossiness: LossinessLedger,
}

/// Errors raised by the bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Webhook body could not be parsed as JSON.
    #[error("maxio webhook body parse failed: {0}")]
    Parse(String),
    /// Maxio's `currency` field was missing or empty.
    #[error("maxio invoice missing currency")]
    MissingCurrency,
    /// Maxio's `number` field was missing.
    #[error("maxio invoice missing display number")]
    MissingNumber,
}

/// Parse a raw Maxio webhook body.
///
/// # Errors
///
/// Returns [`BridgeError::Parse`] when the body is not a valid
/// Maxio `webhook` shape.
pub fn parse_event(body: &str) -> Result<MaxioWebhookEvent, BridgeError> {
    serde_json::from_str(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

/// Lift a Maxio invoice into the typed
/// [`MaxioInvoiceSummary`] + [`LossinessLedger`].
///
/// # Errors
///
/// Returns [`BridgeError::MissingCurrency`] when the Maxio
/// invoice carries an empty currency, or
/// [`BridgeError::MissingNumber`] when the display number is
/// empty.
pub fn extract_invoice_summary(invoice: &MaxioInvoice) -> Result<TranslationOutcome, BridgeError> {
    if invoice.currency.is_empty() {
        return Err(BridgeError::MissingCurrency);
    }
    if invoice.number.is_empty() {
        return Err(BridgeError::MissingNumber);
    }

    let currency = invoice.currency.to_uppercase();
    let mut preserved: Vec<LossinessEntry> = Vec::new();
    let mut lost: Vec<LossinessEntry> = Vec::new();

    preserved.push(LossinessEntry {
        path: "/uid".to_owned(),
        reason: "Maxio invoice uid preserved as audit ref".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/number".to_owned(),
        reason: "display number lifted to document_number".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/currency".to_owned(),
        reason: "currency lifted to InvoiceKit currency".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/total_amount".to_owned(),
        reason: "total lifted to monetary_total.payable_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/subtotal_amount".to_owned(),
        reason: "subtotal lifted to monetary_total.tax_exclusive_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/line_items".to_owned(),
        reason: format!(
            "{} line item(s) lifted to commercial_document.lines",
            invoice.line_items.len()
        ),
    });

    if invoice.public_url.is_some() {
        lost.push(LossinessEntry {
            path: "/public_url".to_owned(),
            reason: "Maxio public invoice URL is not part of the InvoiceKit IR".to_owned(),
        });
    }
    if invoice.pdf_url.is_some() {
        lost.push(LossinessEntry {
            path: "/pdf_url".to_owned(),
            reason: "Maxio-rendered PDF is replaced by the InvoiceKit-rendered PDF".to_owned(),
        });
    }
    for key in invoice.metadata.keys() {
        lost.push(LossinessEntry {
            path: format!("/metadata/{key}"),
            reason: "Maxio metadata key not part of the IR (operator may map it via tenant config)"
                .to_owned(),
        });
    }

    let lines: Vec<MaxioLineSummary> = invoice.line_items.iter().map(extract_line).collect();

    Ok(TranslationOutcome {
        summary: MaxioInvoiceSummary {
            maxio_invoice_uid: invoice.uid.clone(),
            document_number: invoice.number.clone(),
            currency,
            subtotal_decimal: invoice.subtotal_amount.clone(),
            tax_decimal: invoice.tax_amount.clone(),
            total_decimal: invoice.total_amount.clone(),
            issue_date: invoice.issue_date.clone(),
            due_date: invoice.due_date.clone(),
            customer: invoice.customer.clone(),
            lines,
        },
        lossiness: LossinessLedger {
            preserved,
            lost,
            ..Default::default()
        },
    })
}

fn extract_line(line: &MaxioLineItem) -> MaxioLineSummary {
    MaxioLineSummary {
        maxio_line_uid: line.uid.clone(),
        description: line
            .description
            .clone()
            .unwrap_or_else(|| line.title.clone()),
        quantity: line.quantity.clone(),
        unit_price_decimal: line.unit_price.clone(),
        line_total_decimal: line.subtotal_amount.clone(),
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_bridge_maxio::crate_name(),
///     "invoicekit-bridge-maxio"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bridge-maxio"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_body() -> &'static str {
        r#"{
            "event": "invoice_issued",
            "payload": {
              "invoice": {
                "uid": "inv_maxio_abc",
                "number": "MX-2026-0042",
                "issue_date": "2026-05-28",
                "due_date": "2026-06-27",
                "currency": "USD",
                "status": "open",
                "subtotal_amount": "1000.00",
                "tax_amount": "85.00",
                "total_amount": "1085.00",
                "customer": {
                  "id": "cus_maxio_xyz",
                  "name": "Acme Corp",
                  "email": "billing@acme.example",
                  "address_line1": "1 Acme Way",
                  "city": "Berlin",
                  "zip": "10115",
                  "country": "DE",
                  "tax_number": "DE123456789"
                },
                "line_items": [
                  {
                    "uid": "li_a",
                    "title": "Monthly subscription",
                    "description": "Premium tier",
                    "quantity": "1",
                    "unit_price": "900.00",
                    "subtotal_amount": "900.00"
                  },
                  {
                    "uid": "li_b",
                    "title": "Usage overage",
                    "quantity": "200",
                    "unit_price": "0.50",
                    "subtotal_amount": "100.00"
                  }
                ],
                "public_url": "https://billing.maxio.example/invoices/abc",
                "pdf_url": "https://billing.maxio.example/invoices/abc.pdf",
                "metadata": {"order_id": "ord_42"}
              }
            }
        }"#
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-bridge-maxio");
    }

    #[test]
    fn event_kind_round_trips() {
        assert_eq!(
            MaxioEventKind::from_event("invoice_issued"),
            MaxioEventKind::InvoiceIssued
        );
        assert_eq!(
            MaxioEventKind::from_event("payment_success"),
            MaxioEventKind::PaymentSuccess
        );
        assert_eq!(
            MaxioEventKind::from_event("payment_failure"),
            MaxioEventKind::PaymentFailure
        );
        assert_eq!(
            MaxioEventKind::from_event("invoice_voided"),
            MaxioEventKind::InvoiceVoided
        );
        assert_eq!(
            MaxioEventKind::from_event("subscription_created"),
            MaxioEventKind::Unhandled
        );
    }

    #[test]
    fn parse_event_extracts_typed_envelope() {
        let event = parse_event(webhook_body()).unwrap();
        assert_eq!(event.event, "invoice_issued");
        assert_eq!(event.kind(), MaxioEventKind::InvoiceIssued);
        assert_eq!(event.payload.invoice.uid, "inv_maxio_abc");
    }

    #[test]
    fn parse_event_rejects_malformed_body() {
        let err = parse_event("not json").unwrap_err();
        assert!(matches!(err, BridgeError::Parse(_)));
    }

    #[test]
    fn extract_invoice_summary_happy_path() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.payload.invoice).unwrap();
        let s = &outcome.summary;
        assert_eq!(s.maxio_invoice_uid, "inv_maxio_abc");
        assert_eq!(s.document_number, "MX-2026-0042");
        assert_eq!(s.currency, "USD");
        assert_eq!(s.subtotal_decimal, "1000.00");
        assert_eq!(s.tax_decimal, "85.00");
        assert_eq!(s.total_decimal, "1085.00");
        assert_eq!(s.issue_date, "2026-05-28");
        assert_eq!(s.due_date.as_deref(), Some("2026-06-27"));
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[0].description, "Premium tier");
        assert_eq!(s.lines[0].quantity, "1");
        assert_eq!(s.lines[1].description, "Usage overage");
        assert_eq!(s.lines[1].quantity, "200");
        assert_eq!(s.lines[1].unit_price_decimal, "0.50");
        assert_eq!(s.customer.tax_number.as_deref(), Some("DE123456789"));
    }

    #[test]
    fn extract_invoice_summary_records_lossiness() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.payload.invoice).unwrap();
        let has_lost = |p: &str| outcome.lossiness.lost.iter().any(|e| e.path == p);
        let has_preserved = |p: &str| outcome.lossiness.preserved.iter().any(|e| e.path == p);
        assert!(has_lost("/public_url"));
        assert!(has_lost("/pdf_url"));
        assert!(has_lost("/metadata/order_id"));
        assert!(has_preserved("/uid"));
        assert!(has_preserved("/currency"));
        assert!(has_preserved("/total_amount"));
        assert!(has_preserved("/line_items"));
    }

    #[test]
    fn extract_invoice_summary_uses_title_when_description_missing() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.payload.invoice.line_items[0].description = None;
        let outcome = extract_invoice_summary(&event.payload.invoice).unwrap();
        assert_eq!(outcome.summary.lines[0].description, "Monthly subscription");
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_currency() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.payload.invoice.currency = String::new();
        let err = extract_invoice_summary(&event.payload.invoice).unwrap_err();
        assert!(matches!(err, BridgeError::MissingCurrency));
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_number() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.payload.invoice.number = String::new();
        let err = extract_invoice_summary(&event.payload.invoice).unwrap_err();
        assert!(matches!(err, BridgeError::MissingNumber));
    }
}
