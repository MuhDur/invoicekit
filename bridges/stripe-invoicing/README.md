# invoicekit-bridge-stripe-invoicing

An import adapter that parses a supplied Stripe webhook JSON body, lifts the embedded invoice object into an InvoiceKit-shaped `StripeInvoiceSummary`, and records every Stripe field that survived or was dropped in a `LossinessLedger`. It transforms a payload you hand it; it does not talk to the Stripe API.

## Capabilities

- `parse_event(body: &str)` — deserialize a raw Stripe webhook body into a typed `StripeWebhookEvent`. Models only the fields the bridge consumes; serde ignores the rest.
- `StripeEventKind::from_type` / `StripeWebhookEvent::kind` — classify the dotted event type into `InvoiceFinalized` (`invoice.finalized`), `InvoicePaymentSucceeded` (`invoice.payment_succeeded`), `InvoiceVoided` (`invoice.voided`), or `Unhandled` for anything else.
- `extract_invoice_summary(invoice: &StripeInvoice)` — map the Stripe invoice object into a `StripeInvoiceSummary` plus a `LossinessLedger`, returned as a `TranslationOutcome`.
- Field mapping into the summary:
  - `id` → `stripe_invoice_id`; `number` → `document_number` (falls back to `id` when `number` is missing or empty).
  - `currency` → uppercased `currency`.
  - `subtotal`, `total`, `tax` minor-unit integers → decimal strings via `currency_minor_unit_exponent`.
  - `finalized_at` (preferred) or `created` (fallback) → `issue_date` as `YYYY-MM-DD` UTC; `due_date` → `due_date` when present.
  - `customer`, `customer_name`, `customer_email`, `customer_address`, `customer_tax_ids` → `StripeCustomerSummary`.
  - `lines.data[]` → `StripeLineSummary` per line (`id`, `description`, `quantity` defaulting to 1, `line_total_decimal` from `amount`, `unit_price_decimal` from `price.unit_amount`, else `price.unit_amount_decimal`, else `amount / quantity`).
- `currency_minor_unit_exponent(currency: &str)` — minor-unit exponent for a currency code: 0 for the listed zero-decimal currencies (BIF, CLP, DJF, GNF, JPY, KMF, KRW, MGA, PYG, RWF, UGX, VND, VUV, XAF, XOF, XPF), 3 for BHD/JOD/KWD/OMR/TND, 2 otherwise.
- Lossiness ledger:
  - `preserved` — fixed entries noting that `/id`, `/currency`, `/total`, `/subtotal`, and `/lines` were carried into InvoiceKit terms.
  - `lost` — entries for `/lines/has_more` when Stripe truncated the line list, `/hosted_invoice_url`, `/invoice_pdf`, and each `/metadata/<key>`, with the reason each was not carried.
- `crate_name()` — returns the package name string.

## Mode / Residuals

This is the parsing and extraction half of the bridge only. Real behavior versus what is out of scope:

- Real: webhook JSON parsing, invoice field extraction, integer-minor-unit-to-decimal conversion, a dependency-free UTC `YYYY-MM-DD` date formatter, and lossiness accounting.
- Not handled — no Stripe API calls. The crate transforms a payload you supply; it never fetches anything. When `lines.has_more` is true it records that the operator must paginate the Stripe API themselves; it does not paginate.
- Not handled — no webhook signature verification. There is no Stripe-Signature check; the caller is responsible for authenticating the webhook before parsing.
- Not handled — no `CommercialDocument` is produced. `extract_invoice_summary` returns a `StripeInvoiceSummary`, not an `invoicekit_ir::CommercialDocument`. The doc-comment states that the eventual engine call site stitches the operator's tenant config (supplier party, signing key, profile choice) onto the summary to build the full document; that step lives in the operator's app and is not in this crate.
- Not handled — no signing, no Universal Business Language projection, no Peppol submission. Transmission is explicitly out of scope.
- Event types `invoice.payment_succeeded` and `invoice.voided` are classified by `StripeEventKind` but receive no kind-specific extraction logic; `extract_invoice_summary` reads any invoice object regardless of event kind. The doc-comment's "emit a paid-stamp" and "credit-note kickoff" are descriptions of intended downstream handling, not behavior in this crate.
- Decimal conversion of `price.unit_amount_decimal` parses through `f64`; the code notes sub-cent Stripe prices stay well within `f64` precision. Integer minor-unit fields (`total`, `subtotal`, `tax`, line `amount`) are converted with exact integer arithmetic.
- The `lines.has_more` lost-entry depends on the supplied payload's flag; the bridge cannot detect truncation Stripe did not signal.

## References

The crate references Stripe's invoice object endpoint `api.stripe.com/v1/invoices/<id>` and Stripe's published zero-decimal currency list in source comments. ISO 4217 minor units and ISO 3166-1 alpha-2 country codes are named in field doc-comments. No specification URLs are embedded in the source.

## License

Apache-2.0
