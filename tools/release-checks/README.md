# release-checks — CI gate tests and the supply-chain wiring guard

A directory of developer/CI tooling that guards InvoiceKit's release surface. It holds two kinds of thing: pytest gate tests that assert committed artifacts stay in sync with their source of truth, and one standalone meta-guard (`verify_release_checks.py`) that asserts the CI workflows themselves still wire in the supply-chain steps. These are checks, not shipped product, and not part of any published crate.

## What it does

Each file is a focused, independent check. None of them generate artifacts or remediate; they read committed files (and in some cases run `cargo`) and assert.

`verify_release_checks.py` — standalone script (not pytest). Reads the workflow YAML under `.github/workflows/` and the dependency-policy configs, and asserts the supply-chain surface is still wired in:
- `ci.yml` contains steps that run `cargo audit`, `cargo deny`, and the `cassette_corpus_has_no_unscrubbed_pii` cassette PII scan.
- `release.yml` contains a `cargo cyclonedx` (CycloneDX SBOM) step and a `cosign sign-blob` step.
- `license-header.yml` runs `tools/license-header/check_headers.py`.
- The three T-050 Typst RustSec advisory waivers (`RUSTSEC-2024-0320`, `RUSTSEC-2024-0436`, `RUSTSEC-2025-0141`) are present in both `deny.toml` and `.cargo/audit.toml`. It then shells out to `cargo tree --locked --workspace -i <crate>` for each waived crate (`yaml-rust`, `paste`, `bincode`) and asserts the advisory only reaches an allowed workspace crate through the documented Typst dependency path. It does a string `in` check on YAML text — it does not parse the workflow, run the underlying jobs, or verify the advisory signatures. Exit codes: `0` all wired, `1` a required check is missing or a waiver drifted out of scope, `2` a workflow file is missing. `--skip-advisory-waiver-scope-check` runs only the workflow-wiring assertions (skips the `cargo tree` calls); `--workflows-dir` overrides the directory.

`test_capabilities_matrix.py` (T-006a) — asserts `schemas/invoicekit-capabilities-v1.json` is a valid Draft 2020-12 schema, that the bundled `crates/cli/data/capabilities/matrix.json` validates against it, and that the matrix declares `schema_version == "1.0"`.

`test_ir_schema_match.py` (T-011) — runs `cargo run -p invoicekit-cli --bin gen-schema` and asserts its output equals the committed `schemas/invoicekit-ir-v1.json` (JSON-parsed comparison), then validates a hand-written synthetic `CommercialDocument` against the committed schema.

`test_schemas_match.py` — runs the `invoicekit-validate` `emit_schema` and `emit_explain_plan_schema` examples and asserts their output equals the committed `schemas/validation-result.schema.json` and `schemas/validation-explain-plan.schema.json`.

`test_typescript_types_match.py` (T-012) — a fast file-existence pre-flight: asserts the `@invoicekit/types` package exists, every schema under `schemas/` has a matching generated `.d.ts`, every generated `.d.ts` is re-exported by `src/index.ts`, and no orphan `.d.ts` files remain. It does not run the TypeScript generator or `tsc`; the real generation/type check lives in a separate `typescript-types.yml` workflow.

`test_country_manifests.py` (T-770+) — loads every `data/country-manifests/*.toml`, asserts the required top-level fields, the `[sandbox]`/`[trust]`/`[fiscal_rep]`/`[validator]` blocks, enum values, the `INVOICEKIT_SANDBOX_<CC>_*` env-var shape, a `blake3:identity` `signature_alg` with a non-empty `signature`, at least one source entry, and an ISO 3166-1 alpha-2 country code. It checks field shape only — it does not recompute or verify the signature.

`test_en16931_coverage.py` — reads `crates/rulepack/data/en16931-br-co-coverage.json`, asserts the BR/BR-CO rule-id set and counts, the pinned official ConnectingEurope source (repository, tag `validation-1.3.16`, commit, UBL/CII XSLT file set), that every rule carries non-silent term mappings and source locations, and that known IR gaps are named as explicitly blocking. It cross-checks the artifact against `tools/en16931-coverage/generate_coverage.py` (via `runpy`) but does not regenerate the artifact.

`test_cii_coverage.py` — reads `crates/format-cii/data/cii-d16b-element-coverage.json`, asserts the coverage class set, element counts, pinned CII D16B schema source plus per-file SHA-256 hashes, that every schema edge is classified non-silently, and that named metadata/overload and gap-family boundaries are encoded as expected. Cross-checks the source constants against `tools/cii-coverage/generate_coverage.py`.

`test_ubl_conformance_corpus.py` / `test_cii_conformance_corpus.py` — release checks over the synthetic conformance corpus under `conformance-corpus/synthetic/`. They import the sibling `tools/conformance-corpus/validate_fixture_metadata.py` to validate fixture metadata and hashes, then assert each corpus has the expected fixture count (50), unique fixture ids, the required coverage scenarios, document types, and profile set. The CII check also asserts the legacy `cii-d16b` corpus is marked retired regression data and that profile claims are encoded as the expected guideline-context ids in `fixture.xml`.

`tests/test_verify_release_checks.py` — unit tests for `verify_release_checks.py`, using synthetic workflow files and a fake `cargo tree` runner to exercise the pass/fail paths and exit codes.

## Usage / CI

The `.github/workflows/license-header.yml` job installs `pytest` and `jsonschema`, then runs (among other steps):

```
pytest tools/release-checks/tests -q
pytest tools/release-checks/test_ir_schema_match.py -q
pytest tools/release-checks/test_capabilities_matrix.py -q
pytest tools/release-checks/test_typescript_types_match.py -q
pytest tools/release-checks/test_country_manifests.py -q
pytest tools/release-checks/test_cii_coverage.py -q
pytest tools/release-checks/test_cii_conformance_corpus.py -q
python3 tools/release-checks/verify_release_checks.py
```

`test_capabilities_matrix.py` also runs from `.github/workflows/capabilities.yml`. The other gate files (`test_schemas_match.py`, `test_en16931_coverage.py`, `test_ubl_conformance_corpus.py`) are pytest tests in the same shape and run with `pytest tools/release-checks/<file>.py -q`.

Run any single check locally with `pytest tools/release-checks/<file>.py -q`, or the meta-guard with `python3 tools/release-checks/verify_release_checks.py`. The schema/coverage gates that shell out to `cargo` require the Rust workspace to build.

## License

Apache-2.0.
