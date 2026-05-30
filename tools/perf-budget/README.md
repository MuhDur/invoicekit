# perf-budget — criterion benchmark regression gate and dashboard publisher

Developer/CI tooling that gates pull requests on benchmark regressions and renders a static benchmark history page. Not a profiler, not a benchmark runner: it consumes the output `cargo bench` already left on disk.

## What it does

Two Python scripts and one configuration file.

### `budget.toml`

The list of tracked operations and their per-operation regression ceilings. Each `[operations.<name>]` table sets a `max_regression_pct`; `<name>` must match the criterion `bench_function` id written under `target/criterion/<name>/`. A top-level `default_max_regression_pct` (currently `10.0`) supplies the ceiling for any operation that omits its own. The TOML comments name the owning `crates/bench-harness/benches/*.rs` file for each operation; this README does not duplicate that list.

### `perf_budget.py` — the gate

Reads criterion's on-disk output, applies the budget, and exits with a status code:

- Loads `budget.toml` (the tracked-operation list).
- For each tracked operation, reads `mean.point_estimate` (nanoseconds) from `<current>/<name>/new/estimates.json` and, if a baseline directory is given, from `<baseline>/<name>/new/estimates.json`.
- Computes the percentage delta against the baseline and marks each operation `ok`, `regression`, `new` (no baseline yet — seeds rather than fails), `missing-current` (no current measurement), or `baseline-missing`.
- Prints a markdown summary table to stdout and, when `--summary-out` is given, writes it to a file.

Exit codes: `0` all within budget; `1` at least one operation regressed beyond its threshold **or** is missing a current measurement; `2` invalid input (missing/unreadable files, malformed JSON, malformed budget, unwritable summary path).

Operations present in the criterion output but absent from `budget.toml` are not checked and never fail the build. A new benchmark with no baseline seeds the baseline rather than failing the PR that introduced it.

### `publish_dashboard.py` — the dashboard publisher

Reads the same criterion output and `budget.toml`, appends one row (UTC timestamp + git SHA + per-operation mean and configured `max_regression_pct`) to `docs/bench/history.jsonl`, and renders a self-contained static page at `docs/bench/index.html` (inline CSS/JS, no external assets). The page shows the latest run's mean per operation, the configured ceiling, and drift versus the previous run, coloured green/amber/red. `--summary-only` rebuilds the HTML from existing history without reading criterion output.

This is a publisher, not a live monitor: it records and renders whatever the most recent run produced. It imports `perf_budget` for budget parsing and estimate loading.

## Usage / CI

Run the gate locally or in CI:

```
python tools/perf-budget/perf_budget.py \
    --current target/criterion \
    --baseline baseline/criterion \
    --budget tools/perf-budget/budget.toml \
    --summary-out perf-summary.md
```

`--baseline` and `--summary-out` are optional; `--budget` defaults to the `budget.toml` next to the script. The two `import` fallbacks mean it runs on Python 3.11+ (stdlib `tomllib`) or 3.10 with `tomli` installed.

Publish the dashboard:

```
python3 tools/perf-budget/publish_dashboard.py \
    --criterion target/criterion \
    --budget    tools/perf-budget/budget.toml \
    --history   docs/bench/history.jsonl \
    --html      docs/bench/index.html
```

In CI:

- `.github/workflows/bench.yml` runs the unit tests (`pytest tools/perf-budget/tests -q`), runs the criterion benches, restores the baseline cached from the default branch, then runs `perf_budget.py` with `--summary-out perf-summary.md` and surfaces the summary as a sticky pull-request comment (header `perf-budget`).
- `.github/workflows/bench-dashboard.yml` runs `publish_dashboard.py` on push to `main`.

Unit tests live in `tests/test_perf_budget.py` and `tests/test_publish_dashboard.py`.

## License

Apache-2.0
