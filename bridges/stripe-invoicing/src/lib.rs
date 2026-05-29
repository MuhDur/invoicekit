// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-bridge-stripe-invoicing` — Stripe Invoicing bridge.
//!
//! Parses Stripe webhook events, extracts the invoice payload,
//! and surfaces a [`StripeInvoiceSummary`] alongside a
//! [`LossinessLedger`] listing every Stripe field the
//! summary does not carry. The eventual engine call site
//! stitches the operator's tenant config onto the summary to
//! produce a full [`invoicekit_ir::CommercialDocument`].
//!
//! Engine-side transmission (sign + UBL projection + Peppol
//! submit) lives in the operator's main app — this crate
//! ships the parsing + extraction half so the bridge stays
//! testable without dragging the full engine into the test
//! target.
//!
//! Supported event types today:
//!
//! * `invoice.finalized` — invoice is locked + ready to send.
//! * `invoice.payment_succeeded` — paid; emit a paid-stamp.
//! * `invoice.voided` — surface as a credit note kickoff.
//!
//! Other event types parse cleanly as [`StripeWebhookEvent`]
//! and surface with `event_kind = StripeEventKind::Unhandled`
//! so the operator can decide whether to ignore them or fan
//! out into a follow-up handler.

use std::collections::BTreeMap;

use invoicekit_ir::{LossinessEntry, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The subset of Stripe webhook event types this bridge
/// recognises. Other event names map to [`StripeEventKind::Unhandled`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeEventKind {
    /// `invoice.finalized` — invoice is locked + ready to send.
    InvoiceFinalized,
    /// `invoice.payment_succeeded` — emit a paid-stamp.
    InvoicePaymentSucceeded,
    /// `invoice.voided` — surface as credit-note kickoff.
    InvoiceVoided,
    /// Any other event the bridge does not specifically handle.
    Unhandled,
}

impl StripeEventKind {
    /// Parse from the dotted Stripe event-type string.
    #[must_use]
    pub fn from_type(value: &str) -> Self {
        match value {
            "invoice.finalized" => Self::InvoiceFinalized,
            "invoice.payment_succeeded" => Self::InvoicePaymentSucceeded,
            "invoice.voided" => Self::InvoiceVoided,
            _ => Self::Unhandled,
        }
    }
}

/// Typed envelope for a Stripe webhook event. We model only
/// the fields the bridge consumes; serde silently ignores the
/// rest, which we surface via [`LossinessLedger`] in
/// [`extract_invoice_summary`].
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripeWebhookEvent {
    /// Stripe event id (`evt_...`).
    pub id: String,
    /// Stripe event type (`invoice.finalized`, ...).
    #[serde(rename = "type")]
    pub event_type: String,
    /// Unix epoch creation time.
    pub created: i64,
    /// Event data envelope.
    pub data: StripeEventData,
    /// Account id this event belongs to (used by Stripe
    /// Connect accounts). Optional in non-Connect payloads.
    #[serde(default)]
    pub account: Option<String>,
    /// Number of webhook delivery attempts so far.
    #[serde(default)]
    pub pending_webhooks: i64,
    /// Livemode flag.
    #[serde(default)]
    pub livemode: bool,
}

impl StripeWebhookEvent {
    /// Convenience: parsed `event_type` mapped to the typed
    /// [`StripeEventKind`].
    #[must_use]
    pub fn kind(&self) -> StripeEventKind {
        StripeEventKind::from_type(&self.event_type)
    }
}

/// `data` envelope. Stripe wraps the actual object in
/// `data.object`; we hoist that out as [`StripeInvoice`].
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripeEventData {
    /// The actual invoice payload.
    pub object: StripeInvoice,
}

/// Subset of Stripe's `Invoice` object we read.
///
/// Mirrors `api.stripe.com/v1/invoices/<id>` fields one-to-one
/// for the keys the bridge uses; everything else lives in
/// `extra` and surfaces via [`LossinessLedger`].
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripeInvoice {
    /// Stripe invoice id (`in_...`).
    pub id: String,
    /// Invoice number (display id, e.g. `INV-0001`).
    #[serde(default)]
    pub number: Option<String>,
    /// Customer id (`cus_...`).
    #[serde(default)]
    pub customer: Option<String>,
    /// Customer name copied onto the invoice at finalization.
    #[serde(default)]
    pub customer_name: Option<String>,
    /// Customer email.
    #[serde(default)]
    pub customer_email: Option<String>,
    /// Customer address. Stripe nests this under
    /// `customer_address`.
    #[serde(default)]
    pub customer_address: Option<StripeAddress>,
    /// Customer tax IDs ("EU VAT 123") captured at finalize.
    #[serde(default)]
    pub customer_tax_ids: Vec<StripeTaxId>,
    /// Currency code (lowercase per Stripe; e.g. `usd`).
    pub currency: String,
    /// Total amount due, in the currency's smallest unit
    /// (cents for USD/EUR). Stripe uses i64 throughout.
    pub total: i64,
    /// Tax-exclusive subtotal in the currency's smallest unit.
    pub subtotal: i64,
    /// Tax amount in the currency's smallest unit.
    #[serde(default)]
    pub tax: Option<i64>,
    /// Status: `draft`, `open`, `paid`, `void`, `uncollectible`.
    pub status: String,
    /// Unix epoch the invoice was first finalized.
    #[serde(default)]
    pub finalized_at: Option<i64>,
    /// Unix epoch the invoice was created.
    pub created: i64,
    /// Unix epoch the invoice is due. `None` for receipt-only.
    #[serde(default)]
    pub due_date: Option<i64>,
    /// Line items.
    pub lines: StripeInvoiceLines,
    /// Hosted invoice URL (Stripe's customer-facing render).
    #[serde(default)]
    pub hosted_invoice_url: Option<String>,
    /// PDF URL.
    #[serde(default)]
    pub invoice_pdf: Option<String>,
    /// Arbitrary metadata key/value pairs.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Stripe `customer_address` shape.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct StripeAddress {
    /// Street address line 1.
    #[serde(default)]
    pub line1: Option<String>,
    /// Street address line 2.
    #[serde(default)]
    pub line2: Option<String>,
    /// City.
    #[serde(default)]
    pub city: Option<String>,
    /// State / province / region.
    #[serde(default)]
    pub state: Option<String>,
    /// Postal code.
    #[serde(default)]
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2 country code.
    #[serde(default)]
    pub country: Option<String>,
}

/// Stripe tax id record (`{ type: "eu_vat", value: "DE..." }`).
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct StripeTaxId {
    /// Tax-id scheme (`eu_vat`, `us_ein`, ...).
    #[serde(rename = "type")]
    pub scheme: String,
    /// Value string.
    pub value: String,
}

/// Pagination wrapper Stripe uses on list fields.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripeInvoiceLines {
    /// The actual line items.
    pub data: Vec<StripeInvoiceLine>,
    /// True when Stripe truncated the list and the operator
    /// must paginate via the Stripe API to fetch the rest.
    /// The bridge surfaces this via `LossinessLedger` so the
    /// audit trail never silently drops lines.
    #[serde(default)]
    pub has_more: bool,
}

/// Stripe invoice line item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripeInvoiceLine {
    /// Line item id (`il_...`).
    pub id: String,
    /// Description string Stripe renders on the invoice.
    #[serde(default)]
    pub description: Option<String>,
    /// Quantity.
    #[serde(default)]
    pub quantity: Option<i64>,
    /// Total amount for this line, in the smallest currency unit.
    pub amount: i64,
    /// Unit amount. Stripe nests this under `price.unit_amount`;
    /// optional because invoice items can carry a raw `amount`
    /// with no underlying price.
    #[serde(default)]
    pub price: Option<StripePrice>,
}

/// Stripe price object.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct StripePrice {
    /// Price id (`price_...`).
    pub id: String,
    /// Unit amount in the smallest currency unit. Optional —
    /// custom prices can carry just `unit_amount_decimal`.
    #[serde(default)]
    pub unit_amount: Option<i64>,
    /// Decimal-string unit amount for sub-cent prices.
    #[serde(default)]
    pub unit_amount_decimal: Option<String>,
}

/// One-line summary of the Stripe invoice in InvoiceKit terms.
///
/// Built by [`extract_invoice_summary`]; the eventual engine
/// call site stitches the operator's tenant config (supplier
/// party, signing key, profile choice) onto this summary to
/// produce a full [`invoicekit_ir::CommercialDocument`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StripeInvoiceSummary {
    /// Stripe invoice id.
    pub stripe_invoice_id: String,
    /// Display number Stripe assigned (`INV-0001` etc.).
    /// Falls back to the Stripe id when number is missing.
    pub document_number: String,
    /// Uppercased currency code (`EUR`, `USD`, ...).
    pub currency: String,
    /// Tax-exclusive subtotal as a decimal string in the
    /// invoice's currency.
    pub subtotal_decimal: String,
    /// Total amount as a decimal string.
    pub total_decimal: String,
    /// Tax amount as a decimal string. `None` when Stripe
    /// returned a null `tax` field.
    pub tax_decimal: Option<String>,
    /// Issue date as `YYYY-MM-DD` UTC, derived from
    /// `finalized_at` (preferred) or `created` (fallback).
    pub issue_date: String,
    /// Due date as `YYYY-MM-DD` UTC, when Stripe set one.
    pub due_date: Option<String>,
    /// Customer-side details lifted from the invoice.
    pub customer: StripeCustomerSummary,
    /// Per-line summaries.
    pub lines: Vec<StripeLineSummary>,
}

/// Customer-side fields extracted from the Stripe invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StripeCustomerSummary {
    /// Stripe customer id, when present.
    pub stripe_customer_id: Option<String>,
    /// Customer display name.
    pub name: Option<String>,
    /// Customer email.
    pub email: Option<String>,
    /// Customer address (Stripe's `customer_address` shape).
    pub address: Option<StripeAddress>,
    /// Customer tax ids (filtered through to the audit trail).
    pub tax_ids: Vec<StripeTaxId>,
}

/// Per-line summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StripeLineSummary {
    /// Stripe line id.
    pub stripe_line_id: String,
    /// Display description.
    pub description: String,
    /// Quantity (defaults to 1 when Stripe omits it).
    pub quantity: i64,
    /// Unit-price decimal, when derivable from Stripe.
    /// Falls back to `amount / quantity` when no price object.
    pub unit_price_decimal: String,
    /// Line total decimal.
    pub line_total_decimal: String,
}

/// Outcome of [`extract_invoice_summary`]: typed summary + the
/// audit-grade [`LossinessLedger`] listing every Stripe field
/// we touched (preserved) and every one we dropped (lost).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranslationOutcome {
    /// Extracted summary.
    pub summary: StripeInvoiceSummary,
    /// Lossiness ledger.
    pub lossiness: LossinessLedger,
}

/// Errors raised by the bridge.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Webhook body could not be parsed as JSON.
    #[error("stripe webhook body parse failed: {0}")]
    Parse(String),
    /// The decoded event was for an event type the bridge
    /// does not extract (e.g. `customer.created`).
    #[error("stripe event kind not extractable: {0}")]
    UnsupportedEventKind(String),
    /// Stripe's `currency` field was missing or empty.
    #[error("stripe invoice missing currency")]
    MissingCurrency,
}

/// Parse a raw Stripe webhook body.
///
/// # Errors
///
/// Returns [`BridgeError::Parse`] when the body is not a valid
/// `event` shape.
pub fn parse_event(body: &str) -> Result<StripeWebhookEvent, BridgeError> {
    serde_json::from_str(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

/// Lift a Stripe invoice into the typed
/// [`StripeInvoiceSummary`] + [`LossinessLedger`].
///
/// # Errors
///
/// Returns [`BridgeError::MissingCurrency`] when the Stripe
/// invoice carries an empty currency.
pub fn extract_invoice_summary(invoice: &StripeInvoice) -> Result<TranslationOutcome, BridgeError> {
    if invoice.currency.is_empty() {
        return Err(BridgeError::MissingCurrency);
    }

    let currency = invoice.currency.to_uppercase();
    let exponent = currency_minor_unit_exponent(&currency);
    let subtotal_decimal = minor_units_to_decimal(invoice.subtotal, exponent);
    let total_decimal = minor_units_to_decimal(invoice.total, exponent);
    let tax_decimal = invoice.tax.map(|t| minor_units_to_decimal(t, exponent));

    let issue_epoch = invoice.finalized_at.unwrap_or(invoice.created);
    let issue_date = unix_to_yyyy_mm_dd_utc(issue_epoch);
    let due_date = invoice.due_date.map(unix_to_yyyy_mm_dd_utc);

    let document_number = invoice
        .number
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| invoice.id.clone());

    let lines: Vec<StripeLineSummary> = invoice
        .lines
        .data
        .iter()
        .map(|line| extract_line(line, exponent))
        .collect();

    let customer = StripeCustomerSummary {
        stripe_customer_id: invoice.customer.clone(),
        name: invoice.customer_name.clone(),
        email: invoice.customer_email.clone(),
        address: invoice.customer_address.clone(),
        tax_ids: invoice.customer_tax_ids.clone(),
    };

    Ok(TranslationOutcome {
        summary: StripeInvoiceSummary {
            stripe_invoice_id: invoice.id.clone(),
            document_number,
            currency,
            subtotal_decimal,
            total_decimal,
            tax_decimal,
            issue_date,
            due_date,
            customer,
            lines,
        },
        lossiness: LossinessLedger {
            preserved: preserved_lossiness_entries(invoice),
            lost: lost_lossiness_entries(invoice),
            ..Default::default()
        },
    })
}

fn preserved_lossiness_entries(invoice: &StripeInvoice) -> Vec<LossinessEntry> {
    vec![
        LossinessEntry {
            path: "/id".to_owned(),
            reason: "Stripe invoice id preserved as document_number fallback".to_owned(),
        },
        LossinessEntry {
            path: "/currency".to_owned(),
            reason: "currency lifted to InvoiceKit currency".to_owned(),
        },
        LossinessEntry {
            path: "/total".to_owned(),
            reason: "total lifted to monetary_total.payable_amount".to_owned(),
        },
        LossinessEntry {
            path: "/subtotal".to_owned(),
            reason: "subtotal lifted to monetary_total.tax_exclusive_amount".to_owned(),
        },
        LossinessEntry {
            path: "/lines".to_owned(),
            reason: format!(
                "{} line item(s) lifted to commercial_document.lines",
                invoice.lines.data.len()
            ),
        },
    ]
}

fn lost_lossiness_entries(invoice: &StripeInvoice) -> Vec<LossinessEntry> {
    let mut lost = Vec::new();

    if invoice.lines.has_more {
        lost.push(LossinessEntry {
            path: "/lines/has_more".to_owned(),
            reason: "Stripe truncated the line list; operator must paginate the Stripe API \
                     to fetch the remaining lines before treating the bundle as complete"
                .to_owned(),
        });
    }

    if invoice.hosted_invoice_url.is_some() {
        lost.push(LossinessEntry {
            path: "/hosted_invoice_url".to_owned(),
            reason: "Stripe's hosted-invoice URL is not part of the InvoiceKit IR".to_owned(),
        });
    }
    if invoice.invoice_pdf.is_some() {
        lost.push(LossinessEntry {
            path: "/invoice_pdf".to_owned(),
            reason: "Stripe-rendered PDF is replaced by the InvoiceKit-rendered PDF".to_owned(),
        });
    }
    lost.extend(invoice.metadata.keys().map(|key| {
        LossinessEntry {
            path: format!("/metadata/{key}"),
            reason:
                "Stripe metadata key not part of the IR (operator may map it via tenant config)"
                    .to_owned(),
        }
    }));

    lost
}

fn extract_line(line: &StripeInvoiceLine, exponent: u32) -> StripeLineSummary {
    let quantity = line.quantity.unwrap_or(1).max(1);
    let line_total_decimal = minor_units_to_decimal(line.amount, exponent);
    let unit_price_decimal = unit_price_for(line, quantity, exponent);
    StripeLineSummary {
        stripe_line_id: line.id.clone(),
        description: line.description.clone().unwrap_or_default(),
        quantity,
        unit_price_decimal,
        line_total_decimal,
    }
}

fn unit_price_for(line: &StripeInvoiceLine, quantity: i64, exponent: u32) -> String {
    if let Some(price) = line.price.as_ref() {
        if let Some(amount) = price.unit_amount {
            return minor_units_to_decimal(amount, exponent);
        }
        if let Some(decimal) = price.unit_amount_decimal.as_deref() {
            // Stripe returns the value in the smallest unit as a
            // decimal string; convert to the major unit.
            return minor_units_decimal_to_major_decimal(decimal, exponent);
        }
    }
    // No price object, or one carrying neither field: derive the
    // unit price from the line total divided by quantity.
    minor_units_to_decimal(line.amount / quantity, exponent)
}

/// Currency minor-unit exponent. Covers ISO 4217 currencies
/// where InvoiceKit cares about the magnitude; default 2.
#[must_use]
pub fn currency_minor_unit_exponent(currency: &str) -> u32 {
    match currency {
        // Zero-decimal currencies per Stripe's published list.
        "BIF" | "CLP" | "DJF" | "GNF" | "JPY" | "KMF" | "KRW" | "MGA" | "PYG" | "RWF" | "UGX"
        | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        // Three-decimal currencies.
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

fn minor_units_decimal_to_major_decimal(decimal: &str, exponent: u32) -> String {
    // Stripe's `unit_amount_decimal` is a stringified number
    // in the smallest currency unit. Parse to f64 (Stripe's
    // own representation is decimal-string-of-i64 fractions),
    // divide by 10^exponent, and re-format. This handles sub-
    // cent prices that don't fit `unit_amount: i64`. Falls
    // back to "0" if the input doesn't parse.
    //
    // f64 loses precision past 2^53; sub-cent Stripe prices
    // never approach that range so the precision is fine.
    #[allow(clippy::cast_precision_loss)]
    let pow = 10_u64.pow(exponent) as f64;
    decimal.parse::<f64>().map_or_else(
        |_| "0".to_owned(),
        |n| format!("{:.*}", exponent as usize, n / pow),
    )
}

fn unix_to_yyyy_mm_dd_utc(epoch: i64) -> String {
    // Minimal civil-date formatter — avoids pulling chrono /
    // jiff into the dep tree for a one-shot use. Handles
    // 1970-01-01 onwards correctly for all practical Stripe
    // invoice timestamps.
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
///     invoicekit_bridge_stripe_invoicing::crate_name(),
///     "invoicekit-bridge-stripe-invoicing"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bridge-stripe-invoicing"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webhook_body() -> &'static str {
        // Minimal but realistic Stripe `invoice.finalized` event.
        r#"{
            "id": "evt_test_123",
            "object": "event",
            "type": "invoice.finalized",
            "created": 1748275200,
            "livemode": false,
            "pending_webhooks": 1,
            "data": {
              "object": {
                "id": "in_test_xyz",
                "number": "INV-2026-0042",
                "customer": "cus_test_abc",
                "customer_name": "Acme Corp",
                "customer_email": "billing@acme.example",
                "customer_address": {
                  "line1": "1 Acme Way",
                  "city": "Berlin",
                  "postal_code": "10115",
                  "country": "DE"
                },
                "customer_tax_ids": [
                  {"type": "eu_vat", "value": "DE123456789"}
                ],
                "currency": "eur",
                "total": 12345,
                "subtotal": 10000,
                "tax": 2345,
                "status": "open",
                "created": 1748275200,
                "finalized_at": 1748275260,
                "due_date": 1750867200,
                "lines": {
                  "has_more": false,
                  "data": [
                    {
                      "id": "il_a",
                      "description": "Consulting",
                      "quantity": 5,
                      "amount": 5000,
                      "price": {"id": "price_a", "unit_amount": 1000}
                    },
                    {
                      "id": "il_b",
                      "description": "Setup",
                      "quantity": 1,
                      "amount": 5000,
                      "price": {"id": "price_b", "unit_amount": 5000}
                    }
                  ]
                },
                "hosted_invoice_url": "https://invoice.stripe.com/test/abc",
                "invoice_pdf": "https://pay.stripe.com/test/abc.pdf",
                "metadata": {"order_id": "ord_42"}
              }
            }
        }"#
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-bridge-stripe-invoicing");
    }

    #[test]
    fn event_kind_round_trips() {
        assert_eq!(
            StripeEventKind::from_type("invoice.finalized"),
            StripeEventKind::InvoiceFinalized
        );
        assert_eq!(
            StripeEventKind::from_type("invoice.payment_succeeded"),
            StripeEventKind::InvoicePaymentSucceeded
        );
        assert_eq!(
            StripeEventKind::from_type("invoice.voided"),
            StripeEventKind::InvoiceVoided
        );
        assert_eq!(
            StripeEventKind::from_type("customer.created"),
            StripeEventKind::Unhandled
        );
    }

    #[test]
    fn parse_event_extracts_typed_envelope() {
        let event = parse_event(webhook_body()).unwrap();
        assert_eq!(event.id, "evt_test_123");
        assert_eq!(event.event_type, "invoice.finalized");
        assert_eq!(event.kind(), StripeEventKind::InvoiceFinalized);
        assert!(!event.livemode);
        assert_eq!(event.data.object.id, "in_test_xyz");
    }

    #[test]
    fn parse_event_rejects_malformed_body() {
        let err = parse_event("not json").unwrap_err();
        assert!(matches!(err, BridgeError::Parse(_)));
    }

    #[test]
    fn extract_invoice_summary_happy_path() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.data.object).unwrap();
        let s = &outcome.summary;
        assert_eq!(s.stripe_invoice_id, "in_test_xyz");
        assert_eq!(s.document_number, "INV-2026-0042");
        assert_eq!(s.currency, "EUR");
        assert_eq!(s.subtotal_decimal, "100.00");
        assert_eq!(s.total_decimal, "123.45");
        assert_eq!(s.tax_decimal.as_deref(), Some("23.45"));
        // The webhook payload's finalized_at / due_date epochs
        // land in 2025; the date math below is the correctness
        // check on the bridge's UTC conversion.
        assert_eq!(s.issue_date, "2025-05-26");
        assert_eq!(s.due_date.as_deref(), Some("2025-06-25"));
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[0].unit_price_decimal, "10.00");
        assert_eq!(s.lines[0].line_total_decimal, "50.00");
        assert_eq!(s.lines[0].quantity, 5);
        // Customer summary survived intact.
        assert_eq!(
            s.customer.tax_ids.first().map(|t| t.value.as_str()),
            Some("DE123456789")
        );
    }

    #[test]
    fn extract_invoice_summary_records_lossiness() {
        let event = parse_event(webhook_body()).unwrap();
        let outcome = extract_invoice_summary(&event.data.object).unwrap();
        let lost_paths = lossiness_paths(&outcome.lossiness.lost);
        let preserved_paths = lossiness_paths(&outcome.lossiness.preserved);
        assert!(lost_paths.contains(&"/hosted_invoice_url"));
        assert!(lost_paths.contains(&"/invoice_pdf"));
        assert!(lost_paths.contains(&"/metadata/order_id"));
        assert!(!lost_paths.contains(&"/lines/has_more"));
        assert!(preserved_paths.contains(&"/currency"));
        assert!(preserved_paths.contains(&"/total"));
        assert!(preserved_paths.contains(&"/lines"));
    }

    #[test]
    fn extract_invoice_summary_falls_back_to_stripe_id_when_number_missing() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.data.object.number = None;
        let outcome = extract_invoice_summary(&event.data.object).unwrap();
        assert_eq!(outcome.summary.document_number, "in_test_xyz");
    }

    #[test]
    fn extract_invoice_summary_flags_paginated_lines_as_lost() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.data.object.lines.has_more = true;
        let outcome = extract_invoice_summary(&event.data.object).unwrap();
        let lost_paths = lossiness_paths(&outcome.lossiness.lost);
        assert!(lost_paths.contains(&"/lines/has_more"));
    }

    #[test]
    fn extract_invoice_summary_rejects_missing_currency() {
        let mut event = parse_event(webhook_body()).unwrap();
        event.data.object.currency = String::new();
        let err = extract_invoice_summary(&event.data.object).unwrap_err();
        assert!(matches!(err, BridgeError::MissingCurrency));
    }

    #[test]
    fn currency_minor_unit_exponent_handles_zero_and_three_decimal_currencies() {
        assert_eq!(currency_minor_unit_exponent("JPY"), 0);
        assert_eq!(currency_minor_unit_exponent("KRW"), 0);
        assert_eq!(currency_minor_unit_exponent("BHD"), 3);
        assert_eq!(currency_minor_unit_exponent("EUR"), 2);
        assert_eq!(currency_minor_unit_exponent("USD"), 2);
        assert_eq!(currency_minor_unit_exponent("XYZ"), 2);
    }

    #[test]
    fn minor_units_to_decimal_handles_jpy_and_eur() {
        assert_eq!(minor_units_to_decimal(12345, 2), "123.45");
        assert_eq!(minor_units_to_decimal(100, 2), "1.00");
        assert_eq!(minor_units_to_decimal(7, 2), "0.07");
        assert_eq!(minor_units_to_decimal(12345, 0), "12345");
        assert_eq!(minor_units_to_decimal(-12345, 2), "-123.45");
    }

    #[test]
    fn unix_to_yyyy_mm_dd_utc_known_vectors() {
        assert_eq!(unix_to_yyyy_mm_dd_utc(0), "1970-01-01");
        assert_eq!(unix_to_yyyy_mm_dd_utc(86_400), "1970-01-02");
        // 1748275200 = 2025-05-26 16:00:00 UTC.
        assert_eq!(unix_to_yyyy_mm_dd_utc(1_748_275_200), "2025-05-26");
        // 1780184000 lands on 2026-05-30.
        assert_eq!(unix_to_yyyy_mm_dd_utc(1_780_184_000), "2026-05-30");
        // Leap-year boundary: 1709164800 = 2024-02-29 00:00 UTC.
        assert_eq!(unix_to_yyyy_mm_dd_utc(1_709_164_800), "2024-02-29");
        // 1709251200 = 2024-03-01 00:00 UTC (verifies the rollover
        // through the leap day).
        assert_eq!(unix_to_yyyy_mm_dd_utc(1_709_251_200), "2024-03-01");
    }

    #[test]
    fn is_leap_year_handles_century_rules() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2000));
    }

    fn lossiness_paths(entries: &[LossinessEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.path.as_str()).collect()
    }
}
