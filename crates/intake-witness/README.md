# invoicekit-intake-witness

Deterministic cross-examination of AI-extracted invoice fields. Re-checks the arithmetic and identifier shapes that an AI intake layer reported, and blocks AI-only emission when a rule disagrees.

## What it does

The AI intake layers (PaddleOCR, SmolDocling, Qwen2.5-VL) produce candidate field values. This crate does not run any of those models and does not read PDFs, run optical character recognition, or call a vision-language model. It takes an already-extracted `ExtractedDocument` — decimal amounts and VAT identifier strings — and re-derives the relationships those values must satisfy. Where the AI's reported value disagrees with the re-derived value, the witness emits a `WitnessFailure` carrying a stable rule id and the JSON-pointer-style citation paths of the offending fields, so an audit UI can highlight exactly what to review.

All checks are deterministic decimal arithmetic and string-shape tests. There is no statistical model, no confidence score, and no network call in this crate.

## Capabilities

Three rules ship today, each with a stable id under the `rules` module:

- `witness.line_total.reconciles` — for each line, checks that `quantity * unit_price - line_discount + line_charge` equals the reported `line_net_amount` within the currency-rounding tolerance.
- `witness.vat.subtotals_close` — checks that the sum of per-line `vat_amount` values equals the reported `document_vat_total` within tolerance.
- `witness.vat_id.validates` — checks that each party's VAT identifier is well-formed under the EU country-prefix taxonomy.

Public surface:

- `cross_examine(&ExtractedDocument) -> Result<WitnessOutcome, WitnessError>` — runs all three rules. Returns `WitnessOutcome::Passed` when every rule agrees, `WitnessOutcome::Failed(Vec<WitnessFailure>)` otherwise. Returns `WitnessError::InvalidTolerance` only when `rounding_tolerance` is negative.
- `validate_eu_vat_id_shape(&str) -> Result<&'static str, &'static str>` — the standalone VAT-id shape check; returns the canonical country code on success.
- `group_failures_by_rule(&[WitnessFailure]) -> BTreeMap<&str, Vec<&WitnessFailure>>` — groups failures by rule id for the audit dashboard.
- Types: `ExtractedDocument`, `ExtractedLine`, `ExtractedParty`, `WitnessFailure`, `WitnessOutcome`, `WitnessError`.
- `WitnessOutcome::is_passed()` and `WitnessOutcome::failures()`.
- `crate_name()` — returns `"invoicekit-intake-witness"`.

The rounding tolerance defaults to `0.01` (one minor unit) when the document leaves it at zero. All decimal arithmetic is overflow-checked: an overflowing line product, net difference, or VAT sum is reported as a reconciliation failure that blocks emission, never a panic.

## Mode / limitations

- This crate validates extracted values; it does not extract them. PDF parsing, OCR, and VLM inference live in sibling intake crates. The model names in the source doc-comment refer to the upstream producers, not to anything this crate runs.
- The VAT-id rule is a shape check only. It verifies the country prefix against a fixed EU table, requires a body length in `[2, 12]`, and restricts body characters to `[A-Z0-9+*]`. It does **not** verify per-country check digits and does **not** confirm that the number is registered. The doc-comment names this as the deterministic precondition that runs before a live VIES round-trip; that VIES lookup is described as a follow-up `intake-witness-vies` crate and is not present here.
- The prefix table is uppercase-only by design. A lowercase prefix is rejected even though the live VIES endpoint would accept it — the witness re-validates the AI extraction, which must commit to a single canonical form.
- The country-prefix table has 28 entries: the 27 EU member states (Greece carried under its `EL` VAT prefix rather than `GR`) plus `XI` (Northern Ireland). It does not cover non-EU VAT schemes.
- VAT-id length and character checks operate on ASCII bytes; non-ASCII input is rejected as an illegal body before any byte-boundary split.
- `serde` derives let `WitnessOutcome` and the input types round-trip through JSON.

## Where it sits

In the pipeline `engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence`, this crate is part of the intake stage: the gate between AI extraction and committing a document to the canonical intermediate representation.

## References

- EN 16931 BR-CO rounding — the source cites this as the basis for the one-minor-unit default tolerance.
- VIES (the EU VAT Information Exchange System) — named in the source as the live round-trip a follow-up crate performs after this shape check; not invoked here.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
