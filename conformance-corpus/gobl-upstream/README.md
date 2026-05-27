# GOBL upstream conformance corpus

This directory holds 20 invoice fixtures lifted unmodified from the
[invopop/gobl](https://github.com/invopop/gobl) project's own example
corpus. They drive the `crates/format-gobl` round-trip integration
test (`tests/upstream_corpus.rs`) and exist to satisfy the strict-
acceptance gate of bead `invoices-t-013` ("20 invoices from GOBL's
own test corpus").

## Provenance

- Upstream repository: <https://github.com/invopop/gobl>
- Pinned commit: `042daa715bf3b4a39bcad6991697103e7bcdc1bd` (May 2026)
- Source paths: `examples/<country>/out/invoice-*.json`
- Selection: 20 fixtures across 18 countries — AE, AR, BE, BR, DE,
  ES, FR, GB, GR, MX, NL, PL, PT, SE, SG, US — chosen for diversity
  of profile, syntax, and edge case (Peppol, Factur-X, reverse-
  charge, freelance, NFSe, peppol-1, ARCA, etc.).

## License

GOBL itself is licensed under Apache 2.0. The example fixtures
inherit that licence as part of the upstream repository. These files
are redistributed under the same Apache 2.0 terms with attribution
to the GOBL authors at the top of the upstream repository.

## File naming

`{country-alpha2}__{upstream-filename}.json` so the directory listing
is its own bilingual index — sortable by country, greppable by GOBL
fixture name. Underscores between country and filename keep the
double-underscore as the splittable token.

## Why this directory is exempt from per-fixture metadata.json

`conformance-corpus/` normally requires a sibling `metadata.json`
per artefact, enforced by
`tools/conformance-corpus/validate_fixture_metadata.py`. This
corpus is excluded from that check:

- These fixtures are not InvoiceKit-authored. They are upstream
  test data carried for **interop verification**, not for
  primary-source conformance evidence.
- Their licence and provenance are uniform across all 20 files —
  declared in this README — so per-file metadata would be 20×
  the same boilerplate.
- The round-trip test that consumes them is the source of truth
  for what each fixture is expected to do (see
  `coverage-matrix.json`).

The exclusion is implemented by adding `gobl-upstream` to
`IGNORED_ARTIFACT_DIRS` in the validator script, with a comment
pointing back here.

## Updating the corpus

Bump the pinned SHA in this README and in
`crates/format-gobl/tests/upstream_corpus.rs`, then re-fetch the
files and re-run the bless step from the test docs to refresh
`coverage-matrix.json`.
