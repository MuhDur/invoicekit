<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-tw-mof

Taiwan (Ministry of Finance / 財政部, MOF) — electronic uniform invoice (電子統一發票) clearance adapter for the e-Invoice platform at `einvoice.nat.gov.tw`.

A typed surface for submitting a business-to-business or business-to-consumer e-Invoice to the MOF platform and recording what it returns: an invoice number from a pre-allocated number book (發票字軌) and the 4-digit random number used in the periodic uniform invoice lottery (統一發票兌獎).

## Capabilities

- **Transmit (clearance submission)** — `MofProvider::submit` takes a `MofSubmitRequest` and returns a `MofSubmitEnvelope` carrying the MOF invoice number (`AA-12345678` — two-letter track plus 8-digit serial), the 4-digit lottery random number, the per-invoice status, the MOF-recorded RFC-3339 issuance timestamp, and a rejection reason when refused.
- **Local validation** — `validate_uniform_number` checks the issuer 統一編號 (uniform number / VAT id) against the 8-ASCII-digit shape before the wire; an empty payload is rejected as `MofError::BadPayload`.
- **Status surfacing** — a MOF 上傳失敗 verdict is not an `Err`; it is returned inside the envelope as `MofStatus::Rejected` (versus `Accepted` / 上傳成功) so the engine persists the refusal alongside its audit trail. `MofError` is reserved for local validation failures and transport failures.

It does **not** serialize a national format. The invoice payload is passed in as an opaque `Vec<u8>` (`MofSubmitRequest.payload`, documented as "Canonical signed payload (MIG 3.2 XML / X.501 JSON)"); this crate does not build, parse, sign, or inspect it. It does not produce evidence bundles.

## Coverage

Opaque-payload / bring-your-own. The crate models the MOF e-Invoice submission contract with typed Rust surfaces, but the only implementation shipped is `MockMofProvider` — a deterministic provider with a fixed timestamp and incrementing serials, for tests and wiring. There is no live network transport here.

Documented residuals from the module doc-comment:

- The live MOF integration lands in a follow-up `report-tw-mof-http` crate. `MofError::Transport` exists for that wire; no HTTP/TLS/DNS code is present in this crate.
- `MofEnvironment` selects `Test` (`wwwtest.einvoice.nat.gov.tw`) or `Production` (`einvoice.nat.gov.tw`); the mock ignores the distinction.
- `MofInvoiceKind` carries the document kind — `B2b` (三聯式 / triplicate), `B2c` (二聯式 / duplicate), `Allowance` (折讓單), `Void` (作廢) — but the mock does not vary its behaviour by kind.
- `MockMofProvider::with_forced_status` is an opt-in test hook to drive the 上傳失敗 (`Rejected`) branch deterministically and exercise the rejection audit trail offline; the default constructor yields `Accepted`.

## References

Sourced from the module doc-comment. Taiwan's Ministry of Finance (財政部) electronic uniform invoice (電子統一發票) regime, operated through the e-Invoice platform at `einvoice.nat.gov.tw` (`wwwtest.einvoice.nat.gov.tw` for test). No specification URLs are cited in the source.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
