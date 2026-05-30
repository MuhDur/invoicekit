# invoicekit-bridge-recurly

A Recurly import adapter: it parses a Recurly webhook JSON body that you supply, pulls the invoice and account blocks out of it, and produces a typed invoice summary plus a ledger of which fields were kept and which were dropped.

This crate transforms a payload you hand it. It does **not** talk to the Recurly API, fetch invoices, or verify webhook signatures.

## Capabilities

- `parse_event(body: &str)` — deserializes a Recurly webhook envelope `{ event_type, invoice, account }` into the typed `RecurlyWebhookEvent`. Returns `BridgeError::Parse` on malformed JSON.
- `RecurlyEventKind::from_event_type(&str)` / `RecurlyWebhookEvent::kind()` — classifies the `event_type` string into one of `NewInvoice`, `PaidChargeInvoice`, `FailedChargeInvoice`, `VoidChargeInvoice`, or `Unhandled`. This is classification only; extraction behavior does not branch on it.
- `extract_invoice_summary(invoice, account)` — produces a `TranslationOutcome { summary: RecurlyInvoiceSummary, lossiness: LossinessLedger }`.
  - Fields read from the invoice: `uuid`, `invoice_number`, `currency`, `subtotal`, `tax`, `total`, `created_at`, `due_on`, `line_items` (Recurly "adjustments"), `invoice_pdf_url`, `custom_fields`.
  - Mapping performed: `currency` is uppercased; `invoice_number` becomes `document_number` (string); `created_at` and `due_on` are truncated to a `YYYY-MM-DD` prefix; each line item becomes a `RecurlyLineSummary` with `quantity` floored to a minimum of 1.
  - Monetary amounts (`subtotal`, `tax`, `total`, line `unit_amount`/`subtotal`) are carried through unchanged as decimal strings in the major unit. Recurly's `*_in_cents` integer fields are not read.
  - The `account` block (including `address` and `vat_number`) is cloned into the summary verbatim.
  - Returns `BridgeError::MissingCurrency` when `currency` is empty.
- `crate_name()` — returns the canonical package name.

The lossiness ledger records preserved fields (`/uuid`, `/invoice_number`, `/currency`, `/total`, `/subtotal`, `/line_items`) and dropped fields (`/invoice_pdf_url` when present, and each `/custom_fields/<key>`).

## Mode / Residuals

- **Import adapter only.** Input is a webhook body string; output is a Rust summary struct. No network I/O, no Recurly client, no signature verification.
- **Output is a summary, not the InvoiceKit IR.** `extract_invoice_summary` returns `RecurlyInvoiceSummary`, a flat typed view. It does not construct an `invoicekit-ir` invoice document. The only `invoicekit-ir` types used are `LossinessEntry` and `LossinessLedger`. The lossiness `reason` strings name IR target paths (e.g. `monetary_total.payable_amount`, `commercial_document.lines`), but no such IR mapping is executed here — those strings describe intended downstream placement, not work this crate does.
- **Event kind does not drive behavior.** `state` is parsed but unused. Extraction is identical for created, paid, failed, and voided events.
- **No tax logic.** `tax` is passed through as a string; no tax categories, rates, or breakdowns are derived.
- **No amount validation.** Decimal strings are copied as-is; the crate does not parse them into money types, check minor units, or verify that lines sum to the total.
- **Date handling is prefix-only.** `take_date_prefix` keeps the first 10 characters when they look like `YYYY-MM-DD`, otherwise returns the input string unchanged. Time zone is not converted.

## References

The source names these Recurly webhook event types: `new_invoice_notification`, `paid_charge_invoice_notification`, `failed_charge_invoice_notification`, `void_charge_invoice_notification`. The country field is documented as ISO 3166-1 alpha-2 and timestamps as RFC 3339. No external specification URLs appear in the source.

## License

Apache-2.0
