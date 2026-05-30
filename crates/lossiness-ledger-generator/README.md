# invoicekit-lossiness-ledger-generator

Computes a populated `LossinessLedger` for a single cross-format invoice projection: which top-level IR fields survive the projection and which drift or vanish.

## What it does

Given a source `CommercialDocument` and a `TargetFormat`, `compute_ledger` returns a `LossinessLedger` (the `preserved` / `lost` / `warnings` record defined in `invoicekit-ir`). It first runs `source.validate()`, then takes one of two paths depending on the target:

- **Format projections** (`TargetFormat::Ubl`, `TargetFormat::Cii`) — serialize the source IR through the target adapter (`invoicekit-format-ubl` / `invoicekit-format-cii`), reparse the emitted XML back into IR, and call `LossinessLedger::from_roundtrip_comparison` to diff the two trees. Fields that compare equal across the round trip are `preserved`; fields that differ are `lost`.
- **Profile projections** (`TargetFormat::FacturX(profile)`, six profiles) — delegate to `invoicekit_profile_factur_x::project`, which already produces the ledger; this crate forwards it unchanged.

The point is the evidence trail: the resulting ledger is the artifact an evidence bundle attaches verbatim to show what a projection kept and dropped.

## Capabilities

- `compute_ledger(source, target) -> Result<LossinessLedger, LossinessGeneratorError>` — the one entry point.
- `TargetFormat` — `Ubl`, `Cii`, or `FacturX(FacturXProfile)`; `TargetFormat::name()` returns a stable operator-readable identifier per variant (e.g. `format-ubl`, `factur-x-minimum`).
- `LossinessGeneratorError` — typed errors for source-IR validation failure, UBL/CII adapter failure, Factur-X projection failure, and the ledger's own envelope check failing.
- `crate_name()` — the canonical package name constant.

## Mode / Residuals

This crate is a thin orchestrator. It owns no diff logic and no projection logic of its own — both are borrowed:

- The round-trip diff lives in `invoicekit-ir` (`LossinessLedger::from_roundtrip_comparison`). It is a **field-level equality comparison** over a fixed set of top-level IR fields (id, schema version, document type, dates, document number, currency, meta, supplier, customer, payee, payment terms and instructions, lines, tax summary, monetary total, attachments, references, notes, extensions, and the rest). It compares whole-field contents via `==`, so a value drift inside a collection whose element count is unchanged is still recorded as lost. It is **not** a per-leaf JSON Pointer diff: a `lost` entry names a top-level field path (e.g. `/lines`), not the specific element or attribute that drifted, and the entry `reason` is a generic note rather than an explanation of the exact change.
- A field is only judged `preserved`/`lost` by whether the source value equals the value recovered after writing and reparsing through the adapter. So the ledger measures the round-trip fidelity of *this crate's adapters*, not conformance to any external profile definition of what a format may carry.
- Profile (Factur-X / ZUGFeRD) ledgers are whatever `invoicekit-profile-factur-x::project` returns; this crate does not inspect or augment them.

No cryptography, signing, or extraction is performed here.

## References

None. The crate source cites no specification, standard, or URL. Format and profile semantics are defined by the dependency crates (`invoicekit-format-ubl`, `invoicekit-format-cii`, `invoicekit-profile-factur-x`), not here.

## License

Apache-2.0. Part of the InvoiceKit workspace.
