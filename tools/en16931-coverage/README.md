# EN 16931 coverage matrix generator

`generate_coverage.py` builds the checked EN 16931 BR/BR-CO coverage matrix that
maps each business rule onto the InvoiceKit intermediate representation (IR). It
is a developer/CI data-generation tool, not a validator and not part of the
shipped product.

## What it does

The script reads the two generated Schematron-derived XSLT files from a local
checkout of `ConnectingEurope/eInvoicing-EN16931`:

- `ubl/xslt/EN16931-UBL-validation.xslt`
- `cii/xslt/EN16931-CII-validation.xslt`

For each `BR-*` and `BR-CO-*` assertion it parses, line by line, the rule id,
severity flag, assertion test, template context, and the human-readable
`svrl:text`. It extracts the referenced business terms and groups (`BT-*`,
`BG-*`) and a best-effort label per term. It does not run the XSLT or validate
any invoice.

The parsed rule set is checked against a hard-coded expected list of rule ids
(`EXPECTED_RULE_IDS`); a mismatch raises an error naming the missing and extra
ids. The rule filter is BR-* and BR-CO-* only — it excludes BR-CL, BR-DEC,
BR-S, BR-E, syntax-only, and VAT-category-specific rules.

Each rule is then annotated from two hand-maintained mapping tables in the
script:

- `CURRENT_IR_PATHS` — business terms with a path in the current IR.
- `REQUIRED_EXTENSION_FIELDS` — terms that are only reachable through the
  profile extension URN `urn:invoicekit:profile:en16931:2017`.

A term with neither mapping is recorded as an `unmapped.*` extension field. A
rule is marked validator-testable (positive and negative) only when every
referenced term has a current IR path and no required extension field; otherwise
it carries a blocker note flagging the IR gap. The mapping tables encode the
project's view of coverage — they are not derived from the source XSLT.

The emitted JSON artifact records: schema version, a fixed `generated_at` date,
source provenance (repository, the `validation-1.3.16` tag, the resolved source
git commit via `git rev-parse`, and a SHA-256 over each source XSLT file), the
IR-gap policy, summary counts (total / BR / BR-CO / source assertions /
validator-testable-now / blocked-by-IR-gaps), and the per-rule records with text
variants, term mappings, source locations, and testability flags. Output is
written deterministically (`indent=2`, `sort_keys=True`).

`TERM_OVERRIDES` corrects one known source-text discrepancy (BR-51 → BT-87).

## Usage / CI

Invoked manually against a local source checkout; it is not wired into any CI
workflow in this repository.

```bash
python3 tools/en16931-coverage/generate_coverage.py \
  --source-root /path/to/eInvoicing-EN16931  # checked out at validation-1.3.16
```

`--source-root` is required. `--output` defaults to
`crates/rulepack/data/en16931-br-co-coverage.json`. That committed artifact is
embedded by the `validate-ubl-cii` crate via `include_str!`; regenerate and
commit it when the source tag or the IR mappings change.

## License

Apache-2.0.
