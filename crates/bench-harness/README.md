# invoicekit-bench-harness

The cross-crate [criterion](https://github.com/bheisler/criterion.rs) benchmark harness that backs InvoiceKit's performance regression budget (T-007).

## What it is

This crate is a thin shell. The only public item in the library is `crate_name() -> &'static str`, the workspace-identity helper every InvoiceKit crate carries. The actual work lives in the `[[bench]]` targets under `benches/`, each of which exercises one named operation drawn from a sibling crate and reports its timing under a fixed criterion `bench_function` id.

The crate is not published (`publish = false`). It exists so the continuous-integration bench workflow can run a stable set of microbenchmarks and compare them against a rolling baseline.

## Capabilities

- `crate_name()` — `const fn` returning `"invoicekit-bench-harness"`.
- A set of criterion benches, each named to match an entry in `tools/perf-budget/budget.toml`. The CI workflow parses `target/criterion/<name>/new/estimates.json`, takes `mean.point_estimate`, and fails the build when the regression versus the cached `main` baseline exceeds that operation's `max_regression_pct`.

Tracked operations and their owning bench files:

| `bench_function` id | Bench file | What it times |
| --- | --- | --- |
| `ir-round-trip` | `ir_round_trip.rs` | Intermediate-representation encode + validate + decode round-trip on one representative document. |
| `xml-canonicalization` | `xml_canonicalization.rs` | XML canonicalization on a synthetic ~1 MiB UBL invoice. |
| `ubl-parse` | `ubl_parse.rs` | UBL 2.1 Invoice parse on a synthetic ~1 MiB invoice. |
| `cii-parse` | `cii_parse.rs` | UN/CEFACT CII D16B parse on a synthetic ~1 MiB invoice. |
| `validate-ubl-small`, `validate-ubl`, `validate-cii` | `validate.rs` | EN 16931 BR / BR-CO rule engine on a 1-line UBL, a 200-line UBL, and a 200-line CII invoice. |
| `render-pdf` | `render_pdf.rs` | Typst PDF render of one commercial document. |
| `tax-line-extensions`, `tax-payable-amount`, `tax-trace-canonical-json` | `tax_calc.rs` | Per-line decimal line-extension arithmetic, document payable rollup, and canonical-JSON serialization of an arithmetic trace, over a 200-line invoice. |
| `codelist-lookup` | `codelist_lookup.rs` | Code-list membership lookups across a per-invoice field mix. |
| `evidence-pack`, `evidence-unpack`, `evidence-verify` | `evidence.rs` | Pack, unpack, and BLAKE3 re-hash of a realistic `.ikb` evidence bundle. |
| `intake-factur-x-extract`, `format-detect` | `intake_pdf.rs` | Factur-X / ZUGFeRD embedded-CII extraction from a PDF/A-3 container, and the byte-prefix format sniff at the front of intake. |

## Mode / Residuals

- No measured numbers ship here. This crate defines the workloads and their regression thresholds; the absolute timings are produced at run time by criterion on the CI host and live outside the source tree.
- Bench fixtures are synthetic and built inline in each bench file (for example, `intake_pdf.rs` constructs a minimal PDF/A-3 Factur-X container in-process rather than reading a real-world sample). The benches measure the InvoiceKit code paths, not representative production documents or hardware.
- The `[lib]` target sets `bench = false`. This stops `cargo bench -p invoicekit-bench-harness` from running the library unit tests as a benchmark; the real benches set `harness = false` and use criterion's own `main`.
- The set of tracked operations is the set of `[operations.*]` tables in `tools/perf-budget/budget.toml`. Adding a bench without a matching budget entry (or vice versa) is a configuration mismatch the workflow does not auto-reconcile.

## References

Specs and standards exercised by the benches (named in the bench and budget sources):

- EN 16931 — semantic data model and business rules for the European core invoice.
- UBL 2.1 — OASIS Universal Business Language Invoice.
- UN/CEFACT Cross Industry Invoice (CII) D16B.
- Factur-X / ZUGFeRD — CII XML embedded in a PDF/A-3 container.

## License

Apache-2.0. Part of the InvoiceKit workspace.
