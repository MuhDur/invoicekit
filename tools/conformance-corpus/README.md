# conformance-corpus — fixture metadata validator

A continuous-integration check that validates the per-fixture `metadata.json`
files in the repository's `conformance-corpus/` data tree against a JSON Schema
and a set of policy rules, and verifies each declared artifact's bytes.

This is developer/CI tooling. It validates fixture *metadata and artifact
integrity*. It is not a format validator: it does not check that an invoice is
valid UBL, Cross Industry Invoice, EN 16931, etc. (the fixtures themselves
record any format-validation outcome in their `validation` field, which this
tool only reads as data).

## What it does

`validate_fixture_metadata.py` walks `conformance-corpus/`, finds every
`metadata.json`, and for each one:

- Loads it as JSON and validates it against
  `conformance-corpus/fixture-metadata.schema.json` using a small built-in
  schema checker (handles `const`, `enum`, `type`, `required`,
  `additionalProperties`, `minItems`, `uniqueItems`, `minLength`, `pattern`,
  `minimum`, and the `date`/`date-time`/`uri` formats — no external
  jsonschema dependency).
- Verifies the declared artifact: resolves `artifact.path` (rejecting absolute
  paths and `..` escapes), confirms the file exists, and checks its
  `size_bytes` and `sha256` against the actual bytes. The hash comparison uses
  a constant-time compare.
- Enforces policy semantics tied to `corpus_partition`,
  `publication`, `license`, `provenance`, and `pii`. For example: public
  fixtures must be `redistribution: public-ok` and must not contain personal
  data; `synthetic` fixtures must be public, carry a `synthetic-` style
  `fixture_id`, use CC0-1.0 or Apache-2.0, and have `generated` provenance;
  `licensed-real` fixtures must declare `license.evidence_path` and real-source
  provenance; `private-regression` fixtures must be private, use the
  `PRIVATE-REGRESSION` license, and be non-redistributable.
- Checks optional reference paths (`license.evidence_path`,
  `pii.redaction_report_path`) stay inside the fixture directory and point at
  real files.
- Confirms `maintenance.review_due` is after `maintenance.reviewed_at`.
- Enforces that every `fixture_id` is unique across the corpus.
- Runs a coverage check: every artifact file (`.json`, `.xml`, `.pdf`) under
  the corpus must have a sibling `metadata.json` whose `artifact.path` points
  at it. `README.md`, `metadata.json`, `scenario.json`, and declared reference
  files are exempt, as are the `generators/`, `fuzz/`, `gobl-upstream/`, and
  `pdf-snapshots/` directories (those carry their own provenance — see their
  own `README.md` / `MANIFEST.json`).

The corpus it validates lives at the repository root under
`conformance-corpus/`, partitioned into `synthetic/`, `licensed-real/`, and
`private-regression/` plus the exempt subtrees above; each fixture directory
holds an artifact (e.g. `fixture.xml`) and a `metadata.json` describing it.

## Usage / CI

Run directly (exits non-zero and prints the first failure on `stderr`,
otherwise prints the count of validated files):

```
python3 tools/conformance-corpus/validate_fixture_metadata.py
```

Optional flags: `--corpus-root <path>` and `--schema <path>` override the
defaults.

It runs in CI in `.github/workflows/license-header.yml` (step "Validate
conformance fixture metadata") and again in
`.github/workflows/adversarial-corpus-bless.yml` against the regenerated tree.

The unit tests in `tests/` are run with pytest, both standalone and from the
`license-header.yml` workflow:

```
pytest tools/conformance-corpus/tests -q
```

## License

Apache-2.0.
