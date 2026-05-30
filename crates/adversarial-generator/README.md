# invoicekit-adversarial-generator

A fixed catalogue of pathological `invoicekit_ir::CommercialDocument` instances and a helper that emits each one through every shipped InvoiceKit serializer, for differential testing.

## What it does

This crate builds a small, hand-curated set of edge-case invoice documents and runs them through every format adapter and profile serializer InvoiceKit ships. The differential harness consumes the same catalogue, so "what counts as pathological" lives in one place.

The catalogue is a closed enumeration, not a random or coverage-guided fuzzer. Each scenario is a deterministic, hand-written document shape. There is no input mutation, no randomization, and no fuzzing engine in this crate.

The seven scenarios (`AdversarialScenario`):

- `ZeroAmountLine` — single line, zero unit price and zero totals (`Z` tax category).
- `NegativeAmountLine` — single line with a negative unit price (refund-style); the doc-comment notes this typically trips EN 16931 rule BR-CO-14.
- `AllowanceGreaterThanTotals` — allowance total exceeds the line-extension sum, producing a negative tax-exclusive amount; the doc-comment notes EN 16931 rule BR-CO-15.
- `MixedVatRates` — two lines under different VAT categories (`S` standard, `Z` zero), so the tax summary carries two buckets.
- `SingleLine` — minimum valid single-line invoice.
- `HighLineCount` — 50 lines, to stress per-line serializer allocations.
- `UnicodeStress` — full-width CJK, Arabic, and a zero-width-joiner emoji in the supplier name and a line description.

## Capabilities

- `AdversarialScenario` — the stable scenario enum, with `all()` (stable iteration order) and `name()` (kebab-case operator-readable name).
- `build_scenario(scenario) -> Result<CommercialDocument, AdversarialError>` — construct one scenario document. IR construction errors are surfaced, not swallowed.
- `generate_adversarial_corpus() -> Result<Vec<(AdversarialScenario, CommercialDocument)>, AdversarialError>` — build every scenario in stable order; returns the first IR error encountered.
- `emit_through_every_serializer(document) -> Vec<SerializerOutcome>` — serialize one document through every shipped serializer. Each `SerializerOutcome` carries the serializer's stable name and exactly one of `output` (the serialized string) or `error` (the stringified typed error). A typed serializer error is recorded as signal, not dropped.
- `crate_name() -> &'static str`.

Serializers exercised by `emit_through_every_serializer`: `format-ubl`, `format-cii`, `profile-xrechnung` (with `XRechnungOptions::default()`), `profile-peppol-bis`, `profile-peppol-pint` for five PINT countries (`au-nz`, `sg`, `jp`, `ae`, `my`), and `format-gobl` (JSON, re-serialized via `serde_json`). Ten or more outcomes per scenario.

## Binary: `gen-corpus-v0-5`

`cargo run --bin gen-corpus-v0-5` writes the v0.5 synthetic public corpus under `conformance-corpus/synthetic/adversarial-v0-5/`. It materializes each scenario's serializer outcomes once, then stamps 12 envelope variations per (scenario, serializer) to reach 840 fixtures, each in its own `fixture-NNNN/` directory with a sibling `metadata.json` and a `CORPUS-MANIFEST.md`. Output is byte-deterministic for a given toolchain, so it is safe to re-run and diff against the committed corpus.

Each fixture's `metadata.json` records a `sha256` of the fixture body, computed with `sha2::Sha256`. This is a content hash for fixture identity and diffing — not a signature, MAC, or any security claim.

## Mode / Residuals

- This is a test-fixture generator, not a fuzzer. The corpus is a fixed enumeration; adding a scenario is an explicit code change, and removing one is a breaking change for the differential harness.
- The generated fixture metadata marks `validation.expected_outcome` as `not-yet-validated` with a known gap of `full-en16931-validation-pending`. The generator emits documents and serializer outputs; it does not run EN 16931 or schema validation over them.
- The EN 16931 business-rule references (BR-CO-14, BR-CO-15) come from the source doc-comments describing intended validator behavior. This crate does not itself check those rules.
- `publish = false`: this is an internal workspace tool, not a published crate.

## Where it sits in the pipeline

It depends on `invoicekit-ir` plus the serializer crates it emits through (`format-ubl`, `format-cii`, `format-gobl`, `profile-xrechnung`, `profile-peppol-bis`, `profile-peppol-pint`). It is downstream tooling for the differential and conformance-corpus work, not part of the production document pipeline.

## References

- EN 16931 business rules BR-CO-14 and BR-CO-15 (named in the scenario doc-comments).
- The generated fixtures target the InvoiceKit `fixture-metadata.schema.json` contract.

## License

Apache-2.0.
