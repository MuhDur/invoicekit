# Benchmark dashboard — operator runbook (T-124)

`crates/bench-harness` houses the criterion benches that the
performance regression budget gate (T-007) compares against the
rolling baseline. The publisher wraps the same artefacts into a
static HTML dashboard for `benchmark.invoicekit.org`.

## Files

- `crates/bench-harness/benches/*.rs` — the criterion workloads.
- `tools/perf-budget/budget.toml` — per-operation
  `max_regression_pct` configuration; the dashboard reuses it as
  the budget column.
- `tools/perf-budget/publish_dashboard.py` — wraps the criterion
  output, appends one summary row to `docs/bench/history.jsonl`,
  and rebuilds `docs/bench/index.html`.
- `tools/perf-budget/tests/test_publish_dashboard.py` — 4 unit
  tests (run via `pytest tools/perf-budget/tests -q`).
- `.github/workflows/bench-dashboard.yml` — GitHub Actions job
  that runs the publisher on every main-branch push and nightly
  06:37 UTC, then commits the refreshed dashboard artefacts back
  to `main` from `invoicekit-bench-bot`.
- `docs/bench/history.jsonl` — append-only JSONL time series.
- `docs/bench/index.html` — self-contained HTML dashboard (no
  CDN, inline CSS and JS).

## Local refresh

```bash
cargo bench -p invoicekit-bench-harness --bench '*' -- \
    --warm-up-time 1 --measurement-time 3

python3 tools/perf-budget/publish_dashboard.py \
  --criterion target/criterion \
  --budget    tools/perf-budget/budget.toml \
  --history   docs/bench/history.jsonl \
  --html      docs/bench/index.html
```

## Deployment to benchmark.invoicekit.org

Use either topology:

1. **GitHub Pages mirror** — point `benchmark.invoicekit.org`
   at the `docs/bench/` directory served from a `gh-pages`
   branch. Add a post-bench workflow step that copies the
   files into `gh-pages` and pushes.
2. **Object-store mirror** — sync `docs/bench/` to
   `s3://benchmark.invoicekit.org/` with `aws s3 sync` or
   `rclone`. Put CloudFront / Bunny in front for TLS.

## Reading the dashboard

- **Mean** column shows the latest criterion mean (auto-scaled
  ns / µs / ms).
- **Max regression** column shows the per-op
  `max_regression_pct` from `budget.toml` — this is the CI gate.
- **Drift vs. previous run** column compares the latest mean
  against the previous mean in the JSONL history. Green = under
  budget, amber = within 80% of the budget, red = over budget.
  A red row matches a `bench` CI failure for the same op.

## Tests

```bash
pytest tools/perf-budget/tests -q
```

4 publisher tests + 15 existing perf-budget tests, all green on
a clean checkout.
