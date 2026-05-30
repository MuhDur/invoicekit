# validator-parity — EN 16931 differential parity harness

Developer and continuous-integration tooling that compares the pure-Rust EN 16931
validator against live Java reference validator sidecars over the committed XML
conformance corpus. This is a test/CI tool, not a shipped product and not a
validator in its own right.

## What it does

Two Python scripts.

### `en16931_parity.py`

A differential harness. For each fixture it:

- Reads XML fixtures matched by a repo-relative glob. Default glob:
  `conformance-corpus/synthetic/ubl-2-1/*/fixture.xml`.
- Projects each fixture onto a target profile by rewriting `cbc:CustomizationID`
  and `cbc:ProfileID` (Peppol BIS or KOSIT XRechnung 3.0) and filling a missing
  `schemeID` on `cbc:EndpointID` with `0204`.
- Optionally normalizes the projected UBL XML through the Rust
  `invoicekit-ubl-normalize` binary before sending it on (skippable with
  `--no-ubl-normalize`).
- Runs the Rust findings probe (`invoicekit-en16931-findings`, invoked via
  `cargo run` by default) on the same XML and collects its rule identifiers.
- Sends the XML to each configured Java validator sidecar over JSON-RPC
  (`POST <url>/rpc`, method `validator.validate`) and collects the sidecar's
  rule identifiers.
- Compares only EN 16931 core business-rule identifiers — `BR-*` and `BR-CO-*`,
  matched by a regular expression. Other identifiers (for example
  `PEPPOL-EN16931-*`) are ignored.
- Records a mismatch when the Rust rule-id set differs from the sidecar's
  rule-id set, reporting `rust_only` and `oracle_only` differences per fixture.

It fails closed:

- If a sidecar response carries a configuration/library marker
  (`CONFIGURATION-MISSING`, `LIBRARY-ERROR`, `NO-MATCHING-SET`), the backend is
  reported as `configuration_error` (exit code 2).
- If a sidecar reports a schema / well-formedness precondition failure
  (e.g. `PHIVE-UNNAMED`, `KOSIT-XML-WELLFORMED`, or `[SAX]`/"schema" messages),
  that backend is also `configuration_error`.
- Otherwise, per backend, parity below `--min-parity` (default `0.999`) yields
  status `fail` (exit code 1).

Output is a single aggregated JSON summary object printed to stdout, keyed by
backend, with `compared`, `parity`, `mismatch_count`, and up to the first 20
`mismatches`. Backends are configured via `--kosit-url` / `--phive-url` or the
`INVOICEKIT_VALIDATOR_KOSIT_URL` / `INVOICEKIT_VALIDATOR_PHIVE_URL` environment
variables; with no sidecar configured and no fixtures found it exits 2.

This harness is a point-in-time diff run on demand. It is not a live monitor and
does not poll.

### `publish_dashboard.py`

A static dashboard publisher for the parity time series:

- Invokes `en16931_parity.py` as a subprocess (unless `--summary-only`),
  aggregates the run into one summary row (timestamp, git SHA, fixture counts,
  parity ratio, driver exit code).
- Appends that row to a JSONL history file (default `docs/parity/history.jsonl`).
- Renders a single self-contained static HTML file (inline CSS and JS, no
  external CDN) at `docs/parity/index.html` showing the latest figures and the
  full history table.

The publisher always renders the dashboard even when the harness exits non-zero;
the driver's exit code is captured in the row rather than aborting the run. This
is a reporting/visualization tool, not a gate.

## Usage / CI

Unit tests for both scripts live in `tests/` and run under pytest / unittest;
they exercise the pure functions with monkeypatched probe and RPC calls (no live
sidecar required).

The `.github/workflows/parity-dashboard.yml` workflow runs the publisher on every
push to `main` that touches `crates/**`, `tools/validator-parity/**`, or
`conformance-corpus/**`, on a nightly schedule, and on manual dispatch. It starts
KOSIT and PHIVE validator sidecar service containers, builds the Rust validator,
then runs:

```
python3 tools/validator-parity/publish_dashboard.py \
  --history docs/parity/history.jsonl \
  --html    docs/parity/index.html
```

and commits any refreshed `docs/parity/` artefacts back to `main`. Operator
notes for the published dashboard are in `docs/operators/PARITY-DASHBOARD.md`.

The harness can also be run directly for local diffing, for example:

```
python3 tools/validator-parity/en16931_parity.py \
  --kosit-url http://127.0.0.1:8080 \
  --phive-url http://127.0.0.1:8081
```

## License

Apache-2.0.
