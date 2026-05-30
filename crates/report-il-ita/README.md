# invoicekit-report-il-ita

Israel — Israel Tax Authority (ITA / רשות המסים) e-Invoicing clearance adapter. Requests a per-invoice **Allocation Number** from the Israel Invoicing gateway.

Israel runs a clearance regime: above the legal turnover threshold an issuer must obtain an Allocation Number for each business-to-business invoice and print it on the document, and the buyer cannot claim a VAT input credit without it. This crate is the typed surface for that request.

## Capabilities

- **Transmit (clearance request)** — `ItaProvider::request_allocation` takes an `ItaAllocationRequest` and returns an `ItaAllocationEnvelope` carrying the ITA-issued 9-digit allocation number, the per-invoice status, the ITA-recorded timestamp, and a rejection reason when refused.
- **Local validation** — `validate_id` checks issuer and buyer Tax Authority ids against the 9-ASCII-digit shape before the wire; an empty payload is rejected as `ItaError::BadPayload`.
- **Status surfacing** — an ITA `Rejected` verdict is not an `Err`; it is returned inside the envelope as `ItaStatus::Rejected` so the engine can persist the refusal alongside its audit trail. `ItaError` is reserved for local validation failures and transport failures.

It does **not** serialize a national format. The invoice payload is passed in as an opaque `Vec<u8>` (`ItaAllocationRequest.payload`, documented as "UBL or ITA-defined JSON"); this crate does not build, parse, or inspect it. It does not sign and does not produce evidence bundles.

## Coverage

Opaque-payload / bring-your-own. The crate models the ITA allocation request-and-response contract with typed Rust surfaces, but the only implementation shipped is `MockItaProvider` — a deterministic provider with fixed timestamps and incrementing serials, for tests and wiring. There is no live network transport here.

Documented residuals from the module doc-comment:

- The live ITA REST integration lands in a follow-up `report-il-ita-http` crate. `ItaError::Transport` exists for that wire; no HTTP/TLS/DNS code is present in this crate.
- `gross_basis_points` is carried as integer basis points (1 NIS = 10,000 bp) so currency and decimal handling stays upstream.
- `ItaEnvironment` selects `Sandbox` or `Production`; the mock ignores the distinction.

## References

Sourced from the module doc-comment. The Israel Tax Authority (רשות המסים) e-Invoicing clearance regime and the Israel Invoicing gateway Allocation Number requirement. No specification or regulator URLs are cited in the source.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
