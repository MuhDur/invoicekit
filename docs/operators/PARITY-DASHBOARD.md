# Parity dashboard — operator runbook (T-123)

The differential test harness (`tools/validator-parity/en16931_parity.py`)
runs the pure-Rust EN 16931 validator and the JVM reference sidecars over
every fixture in `conformance-corpus/synthetic/ubl-2-1/**` and reports
per-fixture parity. A wrapper script publishes the time series and a
static HTML dashboard.

## Files

- `tools/validator-parity/en16931_parity.py` — the differential driver.
  Emits one JSON object per fixture on stdout when called with `--json`.
- `tools/validator-parity/publish_dashboard.py` — wraps the driver,
  appends one summary row to `docs/parity/history.jsonl`, and rebuilds
  `docs/parity/index.html`.
- `tools/validator-parity/tests/test_publish_dashboard.py` — unit tests.
- `.github/workflows/parity-dashboard.yml` — GitHub Actions job that
  runs the publisher on every main-branch push and nightly at 05:17 UTC,
  then commits the refreshed dashboard artefacts back to `main`.
- `docs/parity/history.jsonl` — append-only JSONL time series.
- `docs/parity/index.html` — self-contained HTML dashboard (inline CSS
  and JS, no external CDN).

## Local refresh

```bash
python3 tools/validator-parity/publish_dashboard.py \
  --history docs/parity/history.jsonl \
  --html    docs/parity/index.html
```

Requires the JVM sidecars to be reachable at
`INVOICEKIT_VALIDATOR_KOSIT_URL` and `INVOICEKIT_VALIDATOR_PHIVE_URL` —
the same env vars the differential driver reads. Bring them up via
`deploy/docker-compose.yml`.

## Deployment to parity.invoicekit.org

Two supported topologies; pick one and document the choice in the
ops runbook for the host:

1. **GitHub Pages mirror** — point `parity.invoicekit.org` at the
   `docs/parity/` directory served from the `gh-pages` branch.
   The default workflow commits to `main`; add a follow-up step
   that copies `docs/parity/` into `gh-pages` and pushes.
2. **Object-store mirror** — sync `docs/parity/` to an S3-compatible
   bucket (`s3://parity.invoicekit.org/`) using `aws s3 sync` or
   `rclone` from a post-build CI step. CloudFront / Bunny in front
   for TLS.

The dashboard is fully static; no DB, no API. Just serve the two
files.

## Reading the dashboard

- The **latest parity** card shows the most recent run's
  `parity_count / total_fixtures` as a percent.
- The **parity over time** table renders the JSONL history in
  reverse chronological order. Watch for sustained drops; a single
  spike usually means a sidecar was unavailable.
- `unavailable_count > 0` does not mean divergence — it means the
  JVM oracle refused a fixture (typically because the rule pack
  wasn't loaded). The driver's `oracle_unavailable` markers handle
  this case so the row counts diverged fixtures separately.

## Tests

```bash
python3 -m unittest discover -s tools/validator-parity/tests
```

3 tests pass on a clean checkout — exercises append/load round-trip,
HTML rendering with a populated series, and the empty-history path.
