// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-bridge-lago` — Lago bridge.
//!
//! Parses Lago webhook events, lifts the invoice payload into
//! a [`LagoInvoiceSummary`], and surfaces dropped fields via
//! [`LossinessLedger`]. Same shape as the Stripe Invoicing
//! bridge so engine-side code can dispatch on a common
//! surface; downstream callers can build a
//! [`invoicekit_ir::CommercialDocument`] by stitching the
//! tenant's supplier config onto the summary.
//!
//! Supported event types today:
//!
//! * `invoice.created` — a draft invoice was created.
//! * `invoice.finalized` — invoice is locked + ready to send.
//! * `invoice.payment_status_updated` — `payment_status` flipped
//!   (paid / failed / refunded).
//!
//! Lago's webhook envelope is `{ webhook_type, object_type, <object>: { … } }`;
//! the actual invoice payload lives under the `invoice` key.

use std::collections::BTreeMap;

use invoicekit_ir::{LossinessEntry, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Subset of Lago webhook types this bridge recognises.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LagoEventKind {
    /// `invoice.created` — draft invoice created.
    InvoiceCreated,
    /// `invoice.finalized` — invoice locked + sendable.
    InvoiceFinalized,
    /// `invoice.payment_status_updated` — `payment_status` changed.
    InvoicePaymentStatusUpdated,
    /// Any other Lago webhook type.
    Unhandled,
}

impl LagoEventKind {
    /// Parse from the dotted Lago `webhook_type` string.
    #[must_use]
    pub fn from_type(value: &str) -> Self {
        match value {
            "invoice.created" => Self::InvoiceCreated,
            "invoice.finalized" => Self::InvoiceFinalized,
            "invoice.payment_status_updated" => Self::InvoicePaymentStatusUpdated,
            _ => Self::Unhandled,
        }
    }
}

/// Typed envelope for a Lago webhook event. We model only the
/// fields we read; everything else surfaces via the lossiness
/// ledger.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct LagoWebhookEvent {
    /// Lago webhook type (`invoice.finalized`, ...).
    pub webhook_type: String,
    /// Resource type Lago tags this event with (e.g. `invoice`).
    pub object_type: String,
    /// Invoice payload.
    pub invoice: LagoInvoice,
}

impl LagoWebhookEvent {
    /// Convenience: parsed `webhook_type` mapped to the typed
    /// [`LagoEventKind`].
    #[must_use]
    pub fn kind(&self) -> LagoEventKind {
        LagoEventKind::from_type(&self.webhook_type)
    }
}

/// Subset of Lago's `Invoice` object we read.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct LagoInvoice {
    /// Lago invoice id (UUID).
    pub lago_id: String,
    /// Display sequential number Lago assigns at finalize.
    #[serde(default)]
    pub number: Option<String>,
    /// `issuing_date` (`YYYY-MM-DD`).
    pub issuing_date: String,
    /// Optional `payment_due_date` (`YYYY-MM-DD`).
    #[serde(default)]
    pub payment_due_date: Option<String>,
    /// Currency code (uppercase per Lago, e.g. `EUR`).
    pub currency: String,
    /// `status`: `draft`, `finalized`, `voided`.
    pub status: String,
    /// `payment_status`: `pending`, `succeeded`, `failed`, etc.
    #[serde(default)]
    pub payment_status: Option<String>,
    /// Tax-exclusive subtotal as a decimal-string Lago renders.
    pub fees_amount_cents: i64,
    /// `taxes_amount_cents`.
    pub taxes_amount_cents: i64,
    /// `total_amount_cents`.
    pub total_amount_cents: i64,
    /// Customer payload Lago nests under the invoice.
    pub customer: LagoCustomer,
    /// Fee line items (Lago's analogue of invoice lines).
    #[serde(default)]
    pub fees: Vec<LagoFee>,
    /// `file_url` Lago renders the PDF at.
    #[serde(default)]
    pub file_url: Option<String>,
    /// Arbitrary metadata key/value pairs.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Lago customer record.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LagoCustomer {
    /// Lago customer id (UUID).
    pub lago_id: String,
    /// External customer id (operator-side id).
    #[serde(default)]
    pub external_id: Option<String>,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// Country code (ISO 3166-1 alpha-2).
    #[serde(default)]
    pub country: Option<String>,
    /// City.
    #[serde(default)]
    pub city: Option<String>,
    /// Zipcode.
    #[serde(default)]
    pub zipcode: Option<String>,
    /// Street address line 1.
    #[serde(default)]
    pub address_line1: Option<String>,
    /// Street address line 2.
    #[serde(default)]
    pub address_line2: Option<String>,
    /// State / province.
    #[serde(default)]
    pub state: Option<String>,
    /// Customer VAT / tax id.
    #[serde(default)]
    pub tax_identification_number: Option<String>,
}

/// Lago fee line item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct LagoFee {
    /// Fee id (UUID).
    pub lago_id: String,
    /// Display name Lago renders on the invoice.
    pub item_name: String,
    /// Units billed (`charge` fees) or `1` for one-shot fees.
    #[serde(default = "default_units")]
    pub units: String,
    /// Unit amount as decimal string.
    pub unit_amount_cents: i64,
    /// Total amount as decimal string.
    pub amount_cents: i64,
}

fn default_units() -> String {
    "1".to_owned()
}

/// One-line summary of the Lago invoice in InvoiceKit terms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LagoInvoiceSummary {
    /// Lago invoice id.
    pub lago_invoice_id: String,
    /// Display number (falls back to `lago_id` when absent).
    pub document_number: String,
    /// Uppercased currency code.
    pub currency: String,
    /// Tax-exclusive subtotal as decimal string.
    pub subtotal_decimal: String,
    /// Tax amount as decimal string.
    pub tax_decimal: String,
    /// Total amount as decimal string.
    pub total_decimal: String,
    /// Issue date (`YYYY-MM-DD`).
    pub issue_date: String,
    /// Optional payment due date.
    pub due_date: Option<String>,
    /// Customer summary (passed through).
    pub customer: LagoCustomer,
    /// Per-fee summaries.
    pub lines: Vec<LagoLineSummary>,
}

/// Per-line summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LagoLineSummary {
    /// Lago fee id.
    pub lago_fee_id: String,
    /// Display description.
    pub description: String,
    /// Units string Lago carried.
    pub units: String,
    /// Unit price decimal.
    pub unit_price_decimal: String,
    /// Line total decimal.
    pub line_total_decimal: String,
}

/// Outcome of [`extract_invoice_summary`]: typed summary + the
/// audit-grade [`LossinessLedger`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranslationOutcome {
    /// Extracted summary.
    pub summary: LagoInvoiceSummary,
    /// Lossiness ledger.
    pub lossiness: LossinessLedger,
}

/// Errors raised by the bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Webhook body could not be parsed as JSON.
    #[error("lago webhook body parse failed: {0}")]
    Parse(String),
    /// Lago's `currency` field was missing or empty.
    #[error("lago invoice missing currency")]
    MissingCurrency,
}

/// Parse a raw Lago webhook body.
///
/// # Errors
///
/// Returns [`BridgeError::Parse`] when the body is not a valid
/// Lago `webhook` shape.
pub fn parse_event(body: &str) -> Result<LagoWebhookEvent, BridgeError> {
    serde_json::from_str(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

/// Lift a Lago invoice into the typed
/// [`LagoInvoiceSummary`] + [`LossinessLedger`].
///
/// # Errors
///
/// Returns [`BridgeError::MissingCurrency`] when the Lago
/// invoice carries an empty currency.
pub fn extract_invoice_summary(invoice: &LagoInvoice) -> Result<TranslationOutcome, BridgeError> {
    if invoice.currency.is_empty() {
        return Err(BridgeError::MissingCurrency);
    }

    let currency = invoice.currency.to_uppercase();
    let exponent = currency_minor_unit_exponent(&currency);
    let subtotal_decimal = minor_units_to_decimal(invoice.fees_amount_cents, exponent);
    let tax_decimal = minor_units_to_decimal(invoice.taxes_amount_cents, exponent);
    let total_decimal = minor_units_to_decimal(invoice.total_amount_cents, exponent);
    let document_number = invoice
        .number
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| invoice.lago_id.clone());

    let mut preserved: Vec<LossinessEntry> = Vec::new();
    let mut lost: Vec<LossinessEntry> = Vec::new();

    preserved.push(LossinessEntry {
        path: "/lago_id".to_owned(),
        reason: "Lago invoice id preserved as document_number fallback".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/currency".to_owned(),
        reason: "currency lifted to InvoiceKit currency".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/total_amount_cents".to_owned(),
        reason: "total lifted to monetary_total.payable_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/fees_amount_cents".to_owned(),
        reason: "subtotal lifted to monetary_total.tax_exclusive_amount".to_owned(),
    });
    preserved.push(LossinessEntry {
        path: "/fees".to_owned(),
        reason: format!(
            "{} fee(s) lifted to commercial_document.lines",
            invoice.fees.len()
        ),
    });

    if invoice.file_url.is_some() {
        lost.push(LossinessEntry {
            path: "/file_url".to_owned(),
            reason: "Lago-rendered PDF is replaced by the InvoiceKit-rendered PDF".to_owned(),
        });
    }
    if let Some(status) = &invoice.payment_status {
        preserved.push(LossinessEntry {
            path: "/payment_status".to_owned(),
            reason: format!("payment_status `{status}` carried into the reconcile state machine"),
        });
    }
    for key in invoice.metadata.keys() {
        lost.push(LossinessEntry {
            path: format!("/metadata/{key}"),
            reason: "Lago metadata key not part of the IR (operator may map it via tenant config)"
                .to_owned(),
        });
    }

    let lines: Vec<LagoLineSummary> = invoice
        .fees
        .iter()
        .map(|fee| extract_fee(fee, exponent))
        .collect();

    Ok(TranslationOutcome {
        summary: LagoInvoiceSummary {
            lago_invoice_id: invoice.lago_id.clone(),
            document_number,
            currency,
            subtotal_decimal,
            tax_decimal,
            total_decimal,
            issue_date: invoice.issuing_date.clone(),
            due_date: invoice.payment_due_date.clone(),
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

fn extract_fee(fee: &LagoFee, exponent: u32) -> LagoLineSummary {
    LagoLineSummary {
        lago_fee_id: fee.lago_id.clone(),
        description: fee.item_name.clone(),
        units: fee.units.clone(),
        unit_price_decimal: minor_units_to_decimal(fee.unit_amount_cents, exponent),
        line_total_decimal: minor_units_to_decimal(fee.amount_cents, exponent),
    }
}

/// Currency minor-unit exponent — same table as the Stripe bridge.
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

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_bridge_lago::crate_name(),
///     "invoicekit-bridge-lago"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bridge-lago"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_body() -> &'static str {
        r#"{
            "webhook_type": "invoice.finalized",
            "object_type": "invoice",
            "invoice": {
              "lago_id": "11111111-2222-3333-4444-555555555555",
              "number": "ACME-2026-0007",
              "issuing_date": "2026-05-28",
              "payment_due_date": "2026-06-27",
              "currency": "EUR",
              "status": "finalized",
              "payment_status": "pending",
              "fees_amount_cents": 100000,
              "taxes_amount_cents": 19000,
              "total_amount_cents": 119000,
              "customer": {
                "lago_id": "cust-uuid-aaaa",
                "external_id": "acme-internal-7",
                "name": "Acme GmbH",
                "email": "billing@acme.example",
                "country": "DE",
                "city": "Berlin",
                "zipcode": "10115",
                "address_line1": "1 Acme Way",
                "tax_identification_number": "DE123456789"
              },
              "fees": [
                {
                  "lago_id": "fee-1",
                  "item_name": "Monthly subscription",
                  "units": "1",
                  "unit_amount_cents": 90000,
                  "amount_cents": 90000
                },
                {
                  "lago_id": "fee-2",
                  "item_name": "Usage overage",
                  "units": "250",
                  "unit_amount_cents": 40,
                  "amount_cents": 10000
                }
              ],
              "file_url": "https://api.lago.example/invoices/abc.pdf",
              "metadata": {"order_id": "ord-42"}
            }
        }"#
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-bridge-lago");
    }

    #[test]
    fn event_kind_round_trips() {
        assert_eq!(
            LagoEventKind::from_type("invoice.created"),
            LagoEventKind::InvoiceCreated
        );
        assert_eq!(
            LagoEventKind::from_type("invoice.finalized"),
            LagoEventKind::InvoiceFinalized
        );
        assert_eq!(
            LagoEventKind::from_type("invoice.payment_status_updated"),
            LagoEventKind::InvoicePaymentStatusUpdated
        );
        assert_eq!(
            LagoEventKind::from_type("customer.created"),
            LagoEventKind::Unhandled
        );
    }

    #[test]
    fn parse_event_extracts_typed_envelope() {
        let event = parse_event(webhook_body()).unwrap();
        assert_eq!(event.webhook_type, "invoice.finalized");
        assert_eq!(event.object_type, "invoice");
        assert_eq!(event.kind(), LagoEventKind::InvoiceFinalized);
        assert_eq!(
            event.invoice.lago_id,
            "11111111-2222-3333-4444-555555555555"
        );
    }

    #[test]
    fn parse_event_rejects_malformed_body() {
        let err = parse_event("not json").unwrap_err();
        assert!(matches!(err, BridgeError::Parse(_)));
    }

    #[test]
    fn extract_invoice_summary_happy_path() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.invoice).unwrap();
        let s = &outcome.summary;
        assert_eq!(s.lago_invoice_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(s.document_number, "ACME-2026-0007");
        assert_eq!(s.currency, "EUR");
        assert_eq!(s.subtotal_decimal, "1000.00");
        assert_eq!(s.tax_decimal, "190.00");
        assert_eq!(s.total_decimal, "1190.00");
        assert_eq!(s.issue_date, "2026-05-28");
        assert_eq!(s.due_date.as_deref(), Some("2026-06-27"));
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[1].units, "250");
        assert_eq!(s.lines[1].unit_price_decimal, "0.40");
        assert_eq!(s.lines[1].line_total_decimal, "100.00");
        // Customer survived intact.
        assert_eq!(
            s.customer.tax_identification_number.as_deref(),
            Some("DE123456789")
        );
    }

    #[test]
    fn extract_invoice_summary_records_lossiness() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.invoice).unwrap();
        let has_lost = |p: &str| outcome.lossiness.lost.iter().any(|e| e.path == p);
        let has_preserved = |p: &str| outcome.lossiness.preserved.iter().any(|e| e.path == p);
        assert!(has_lost("/file_url"));
        assert!(has_lost("/metadata/order_id"));
        assert!(has_preserved("/currency"));
        assert!(has_preserved("/total_amount_cents"));
        assert!(has_preserved("/payment_status"));
        assert!(has_preserved("/fees"));
    }

    #[test]
    fn extract_invoice_summary_falls_back_to_lago_id_when_number_missing() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.invoice.number = None;
        let outcome = extract_invoice_summary(&event.invoice).unwrap();
        assert_eq!(
            outcome.summary.document_number,
            "11111111-2222-3333-4444-555555555555"
        );
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_currency() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.invoice.currency = String::new();
        let err = extract_invoice_summary(&event.invoice).unwrap_err();
        assert!(matches!(err, BridgeError::MissingCurrency));
    }

    #[test]
    fn currency_minor_unit_exponent_handles_zero_and_three_decimal_currencies() {
        assert_eq!(currency_minor_unit_exponent("JPY"), 0);
        assert_eq!(currency_minor_unit_exponent("BHD"), 3);
        assert_eq!(currency_minor_unit_exponent("EUR"), 2);
    }

    #[test]
    fn minor_units_to_decimal_handles_known_vectors() {
        assert_eq!(minor_units_to_decimal(12345, 2), "123.45");
        assert_eq!(minor_units_to_decimal(0, 2), "0.00");
        assert_eq!(minor_units_to_decimal(-9990, 2), "-99.90");
        assert_eq!(minor_units_to_decimal(12345, 0), "12345");
    }
}
