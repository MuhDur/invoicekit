# invoicekit-bridge-chargebee

Import adapter that parses a supplied Chargebee webhook body and lifts the invoice it carries into a typed InvoiceKit summary, recording which fields were preserved and which were dropped.

## Capabilities

- `parse_event(body: &str)` deserializes a raw Chargebee webhook JSON body of the shape `{ id, event_type, content: { invoice, customer } }` into a typed `ChargebeeWebhookEvent`. Returns `BridgeError::Parse` on malformed JSON.
- `ChargebeeEventKind::from_event_type` / `ChargebeeWebhookEvent::kind` classify the `event_type` string into `InvoiceGenerated`, `PaymentSucceeded`, `PaymentFailed`, `InvoiceVoided`, or `Unhandled`.
- `extract_invoice_summary(&ChargebeeInvoice, &ChargebeeCustomer)` produces a `TranslationOutcome` holding a `ChargebeeInvoiceSummary` plus a `LossinessLedger` (from `invoicekit-ir`). Mapped fields:
  - invoice `id` → both `chargebee_invoice_id` and `document_number` (Chargebee does not separate the two);
  - `currency_code` → uppercased `currency`;
  - `sub_total`, `tax`, `total` (integer minor units) → decimal strings, scaled by a built-in currency minor-unit exponent table;
  - `date` and optional `due_date` (Unix seconds) → `YYYY-MM-DD` UTC strings via an internal date conversion;
  - `line_items` → `ChargebeeLineSummary` entries (id, description, quantity, unit-price decimal, line-total decimal). When `unit_amount` is absent, unit price is derived as `amount / quantity`.
  - the supplied `customer` record is cloned into the summary verbatim.
- `currency_minor_unit_exponent(currency: &str)` exposes the minor-unit table (0 for zero-decimal currencies such as JPY/KRW, 3 for BHD/JOD/KWD/OMR/TND, 2 otherwise).
- `crate_name()` returns the package name string.

## Mode / Residuals

- This crate is a pure payload transformer. It does NOT call the Chargebee API, fetch resources, verify webhook signatures, or perform any network or HTTP work — the caller must supply the webhook body as a string.
- The `event_type` classification is informational only. `extract_invoice_summary` ignores the event kind; it always reads the invoice and customer from `content`. There is no payment-state, void, or credit-specific handling — `PaymentSucceeded`, `PaymentFailed`, and `InvoiceVoided` are recognized labels but drive no distinct behavior. The crate-level doc-comment framing these as "Supported event types today" with per-event semantics (e.g. "payment captured", "invoice voided / credited") overstates what the code does.
- The output is a flat `ChargebeeInvoiceSummary`, not a full InvoiceKit intermediate representation document. The lossiness ledger's `preserved` reasons reference IR paths (`monetary_total.payable_amount`, `commercial_document.lines`, etc.), but this crate does not construct those IR objects — it only emits the summary and the ledger.
- Explicitly dropped (recorded in `LossinessLedger.lost`): `hosted_invoice_url` and every `meta_data` key. Invoice `status` and `customer_id` are parsed but not carried into the summary. The error variant `BridgeError::MissingCurrency` is raised when `currency_code` is empty.
- The Unix-to-date conversion is a hand-rolled UTC calendar walk (proleptic Gregorian leap-year rules); it produces a calendar date with no time-of-day or timezone component.

## References

No external specifications or URLs are cited in the source. The Chargebee field names (`invoice_generated`, `currency_code`, `sub_total`, `meta_data`, etc.) and the webhook envelope shape are referenced only as inline documentation of the JSON structure being parsed.

## License

Apache-2.0
