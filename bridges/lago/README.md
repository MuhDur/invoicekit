# invoicekit-bridge-lago

Import adapter that parses a supplied [Lago](https://www.getlago.com/) invoice
webhook payload and lifts it into a typed `LagoInvoiceSummary`, recording every
field it keeps or drops in a `LossinessLedger`.

## Capabilities

- Parses a raw Lago webhook body (`parse_event`) into a typed
  `LagoWebhookEvent` (`webhook_type`, `object_type`, nested `invoice`).
- Classifies the `webhook_type` string into `LagoEventKind`. Three event
  strings are recognized: `invoice.created`, `invoice.finalized`,
  `invoice.payment_status_updated`. Anything else maps to
  `LagoEventKind::Unhandled`.
- Lifts a `LagoInvoice` into a `LagoInvoiceSummary` (`extract_invoice_summary`):
  - `lago_id`, `number` (falls back to `lago_id` when absent) → document number
  - `currency` (uppercased), `issuing_date`, `payment_due_date`
  - `fees_amount_cents`, `taxes_amount_cents`, `total_amount_cents` →
    decimal strings (subtotal / tax / total)
  - `customer` (passed through as `LagoCustomer`)
  - `fees[]` → `LagoLineSummary` per fee (`item_name`, `units`,
    `unit_amount_cents`, `amount_cents`)
- Converts integer minor units to decimal strings using a built-in
  currency-exponent table (0-decimal currencies like JPY, 3-decimal currencies
  like BHD, default 2). No floats.
- Records a `LossinessLedger` of `preserved` and `lost` entries — including
  `file_url` and each `metadata` key as dropped, and `currency`,
  `total_amount_cents`, `fees_amount_cents`, `fees`, `payment_status`, `lago_id`
  as preserved.
- `crate_name()` returns the canonical package name.

## Mode / Residuals

- **Import adapter only.** This crate transforms a webhook payload that the
  caller already holds. It does not open a network connection, authenticate, or
  call the Lago API; it does not verify webhook signatures. Delivering the
  payload is the caller's responsibility.
- **Produces a summary, not IR.** The output is `LagoInvoiceSummary` /
  `LagoLineSummary`, not an `invoicekit_ir::CommercialDocument`. Building a
  `CommercialDocument` (stitching in supplier config, monetary totals, lines) is
  left to downstream code; this crate does not do it. The lossiness entries name
  IR targets such as `monetary_total.payable_amount` and
  `commercial_document.lines` as descriptions of where a field would land — those
  IR structures are not constructed here.
- **Partial field coverage.** Only the fields modeled on `LagoInvoice`,
  `LagoCustomer`, and `LagoFee` are read. Lago `metadata` and `file_url` are
  dropped (recorded in the ledger). Per-fee tax detail, per-fee currency, charge
  identifiers, credit notes, and other Lago invoice fields are not modeled.
- **`status` / `payment_status` are strings.** They are deserialized and
  `payment_status` is noted in the ledger, but the crate does not map them to a
  reconcile state machine or any enum; the doc-comment's reference to "the
  reconcile state machine" describes intended downstream use, not behavior here.
- **No tax computation.** Amounts are carried through as-is; the crate computes
  nothing beyond minor-unit-to-decimal formatting.
- **Errors:** `BridgeError::Parse` (body is not valid Lago webhook JSON) and
  `BridgeError::MissingCurrency` (empty `currency`).

## References

- Lago webhook envelope shape (`{ webhook_type, object_type, <object>: { … } }`)
  and event names, as modeled in the source. No specification URLs are cited in
  the code.

## License

Apache-2.0
