# docs-generate

MDX page generator for the InvoiceKit documentation site (Nextra). Developer/CI tooling, not a shipped product.

`generate.py` reads existing repository data — the EN 16931 rule catalogue, the country report crates, and the operator runbooks — and writes one MDX page per item plus a Nextra `_meta.json` ordering manifest for each section. It does not lint, validate, or gate anything; it is a one-way fan-out from repo sources into site pages. Re-running it regenerates the pages.

## What it does

Reads:

- `crates/rulepack/data/en16931-br-co-coverage.json` — the EN 16931 business-rule catalogue (the `rules` array). Each rule may carry business terms, business groups, current IR paths, upstream Schematron `source_locations`, and a `rust_validator_testability` block (positive/negative flags, optional blocker string).
- `crates/report-*/Cargo.toml` — every country report adapter crate. Only crates whose `[package].name` matches `invoicekit-report-<cc>-...` are picked up; the country code is the first segment of the name suffix and the `description` field becomes the page summary. Non-matching crates are skipped.
- `docs/operators/*.md` — operator runbooks, copied verbatim into the site.

Writes (under `apps/docs-site/pages`, default; overridable with `--site`):

- `rules/<RULE-ID>.mdx` — one page per rule, rendering business terms/groups, the validator-testability flags and blocker, IR paths, and a table of upstream Schematron source locations (syntax, file, line, test). Plus `rules/index.mdx` and `rules/_meta.json`.
- `countries/<cc>-<adapter>.mdx` — one page per matched report crate (multiple per country if several report crates exist), with country name, country code, a source link, and the crate `description`. Plus `countries/index.mdx` and `countries/_meta.json`. The page text describes the Provider + MockProvider substrate as a fixed template, not as something derived from the crate.
- `operators/<name>.mdx` — each `docs/operators/*.md` runbook mirrored as MDX, plus `operators/index.mdx` and `operators/_meta.json`.

Country code → display name comes from a hardcoded `COUNTRY_NAMES` table in the script; an unknown code falls back to the raw code. On success the script prints page counts to stderr and exits 0.

What it is NOT: it is not a validator and does not check rule correctness, coverage, or crate completeness. It does not build or deploy the site (that is `next build` / `bun run build` in `apps/docs-site`). It does not verify that the input JSON is current or that every rule/crate is present — it renders whatever it reads.

## Usage / CI

Run directly from the repository root:

```
python3 tools/docs-generate/generate.py
```

Optional flags: `--rule-data <path>` (default `crates/rulepack/data/en16931-br-co-coverage.json`) and `--site <path>` (default `apps/docs-site/pages`).

Unit tests (5 tests covering rule rendering, crate parsing, and country rendering):

```
pytest tools/docs-generate/tests -q
```

In CI, the `docs-site` workflow (`.github/workflows/docs-site.yml`) runs the tests in the `generator-tests` job, then runs `generate.py` and `bun run build` in the `build-site` job. The workflow triggers on changes to `apps/docs-site/**`, `tools/docs-generate/**`, `crates/rulepack/data/**`, `docs/operators/**`, and the workflow file itself.

## License

Apache-2.0.
