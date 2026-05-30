# invoicekit-bridge-maxio

An import adapter that parses a supplied Maxio (Chargify + SaaSOptics) webhook
event body and lifts the embedded invoice into a typed summary, recording which
fields are preserved and which are dropped. It transforms a JSON payload you
provide; it does not talk to the Maxio API.

## Capabilities

- `parse_event(body: &str)` deserializes a Maxio webhook envelope
  (`{ event, payload: { invoice: {...} } }`) into a typed `MaxioWebhookEvent`,
  returning `BridgeError::Parse` on malformed JSON.
- `MaxioEventKind::from_event` classifies the `event` string into one of
  `InvoiceIssued`, `PaymentSuccess`, `PaymentFailure`, `InvoiceVoided`, or
  `Unhandled`. `MaxioWebhookEvent::kind()` is a convenience wrapper.
- `extract_invoice_summary(&MaxioInvoice)` reads a fixed subset of Maxio's
  invoice object and returns a `TranslationOutcome` holding:
  - a `MaxioInvoiceSummary` with: `maxio_invoice_uid` (from `uid`),
    `document_number` (from `number`), uppercased `currency`, the
    `subtotal_decimal` / `tax_decimal` / `total_decimal` amounts copied as-is
    (decimal strings, major unit), `issue_date`, optional `due_date`, the
    passed-through `MaxioCustomer`, and per-line `MaxioLineSummary` entries.
  - a `LossinessLedger` recording preserved paths (`/uid`, `/number`,
    `/currency`, `/total_amount`, `/subtotal_amount`, `/line_items`) and lost
    paths (`/public_url`, `/pdf_url`, and each `/metadata/<key>`).
- Line extraction falls back to the line `title` when `description` is absent;
  line quantity defaults to `"1"` when omitted.
- Validates that `currency` and `number` are non-empty, returning
  `BridgeError::MissingCurrency` / `BridgeError::MissingNumber` otherwise.
- `crate_name()` returns the canonical package name `"invoicekit-bridge-maxio"`.

## Mode / Residuals

This crate is a parse-and-summarize adapter only. What it does NOT do:

- **No network.** It does not call Maxio's API, fetch resources, or send
  anything. It operates solely on a webhook body string handed to it.
- **No webhook authentication.** Signature / HMAC verification is not
  implemented; the caller must authenticate the webhook before parsing.
- **No InvoiceKit IR construction.** Despite the lossiness `reason` strings
  naming IR targets (e.g. "total lifted to `monetary_total.payable_amount`",
  "line item(s) lifted to `commercial_document.lines`"), the code never builds
  those IR fields. The output is the flat `MaxioInvoiceSummary` plus the ledger;
  no `invoicekit_ir` document type is produced. Only `LossinessEntry` /
  `LossinessLedger` from `invoicekit-ir` are used.
- **No amount parsing.** Monetary fields are carried as raw decimal strings.
  There is no decimal/money parsing, rounding, or arithmetic validation
  (e.g. subtotal + tax == total is not checked).
- **Event kind does not branch behavior.** `MaxioEventKind` is parsed and
  exposed, but `extract_invoice_summary` always performs the same invoice
  extraction regardless of kind. `PaymentSuccess`, `PaymentFailure`, and
  `InvoiceVoided` have no distinct handling beyond enum classification; there is
  no payment, refund, or credit-note processing.
- **Fixed field subset.** Only the fields declared on `MaxioInvoice`,
  `MaxioCustomer`, and `MaxioLineItem` are read. Other Maxio invoice fields are
  ignored and not recorded in the ledger.

The four "supported event types" listed in the source doc-comment refer to
which `event` strings `MaxioEventKind` recognizes, not to distinct processing
paths.

## References

The module documentation describes the Maxio webhook envelope shape
(`{ event, payload: { invoice: {...} } }`) and that monetary amounts arrive as
major-unit decimal strings. No external specification documents or URLs are
cited in the source.

## License

Apache-2.0
