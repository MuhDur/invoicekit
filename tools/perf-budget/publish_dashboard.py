#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Publish the InvoiceKit benchmark dashboard for benchmark.invoicekit.org.

Reads the criterion benchmark output produced by `cargo bench` and the
per-operation budget at `tools/perf-budget/budget.toml`, appends a new
row to ``docs/bench/history.jsonl`` keyed by git commit + UTC timestamp,
and renders a self-contained static dashboard at
``docs/bench/index.html``.

Layout mirrors the parity dashboard publisher (T-123):

* one append-only JSONL time series under ``docs/bench/``
* one static HTML page with inline CSS + JS (no external CDN)
* per-operation history is the headline view, with the budget line
  rendered alongside the most recent run's mean for each op

Usage::

    python3 tools/perf-budget/publish_dashboard.py \\
        --criterion target/criterion \\
        --budget    tools/perf-budget/budget.toml \\
        --history   docs/bench/history.jsonl \\
        --html      docs/bench/index.html
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import pathlib
import subprocess
import sys
from typing import Iterable

REPO = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tools" / "perf-budget"))

import perf_budget  # noqa: E402


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--criterion",
        type=pathlib.Path,
        default=REPO / "target" / "criterion",
        help="Root criterion output directory (one subdir per op)",
    )
    p.add_argument(
        "--budget",
        type=pathlib.Path,
        default=REPO / "tools" / "perf-budget" / "budget.toml",
        help="Per-operation budget TOML",
    )
    p.add_argument(
        "--history",
        type=pathlib.Path,
        default=REPO / "docs" / "bench" / "history.jsonl",
        help="JSONL history file (appended to)",
    )
    p.add_argument(
        "--html",
        type=pathlib.Path,
        default=REPO / "docs" / "bench" / "index.html",
        help="Output static HTML dashboard",
    )
    p.add_argument(
        "--summary-only",
        action="store_true",
        help="Rebuild HTML from existing history without reading criterion output",
    )
    return p.parse_args(argv)


def _git_sha() -> str | None:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


def collect_row(criterion: pathlib.Path, budget: pathlib.Path) -> dict:
    """Read criterion + budget into a single dashboard row.

    Each operation gets its mean (ns) and the configured
    `max_regression_pct` from the budget. Missing criterion output
    is recorded as `null` so the dashboard can render "no data"
    rather than silently dropping the op.
    """
    _default_pct, ops = perf_budget.load_budget(budget)
    ops_data: dict[str, dict[str, float | None]] = {}
    for op_name, regression_pct in ops.items():
        try:
            mean = perf_budget.load_estimate(criterion, op_name)
        except perf_budget.InvalidInputError:
            mean = None
        ops_data[op_name] = {
            "mean_ns": mean,
            "max_regression_pct": regression_pct,
        }
    return {
        "timestamp": _dt.datetime.now(tz=_dt.timezone.utc).isoformat(),
        "git_sha": os.environ.get("GITHUB_SHA") or _git_sha() or "unknown",
        "ops": ops_data,
    }


def append_row(history: pathlib.Path, row: dict) -> None:
    history.parent.mkdir(parents=True, exist_ok=True)
    with history.open("a", encoding="utf-8") as f:
        f.write(json.dumps(row, sort_keys=True))
        f.write("\n")


def load_rows(history: pathlib.Path) -> list[dict]:
    if not history.exists():
        return []
    rows: list[dict] = []
    for line in history.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def render_dashboard(rows: Iterable[dict], html: pathlib.Path) -> None:
    rows_list = list(rows)
    series_json = json.dumps(rows_list, sort_keys=True)
    latest = rows_list[-1] if rows_list else {
        "timestamp": "—",
        "git_sha": "—",
        "ops": {},
    }
    # Build per-op history (most recent N runs) so the table can
    # show drift relative to the previous run, not just an absolute
    # mean. This is the dashboard's headline view per the bead's
    # "per-operation history visible" acceptance gate.
    history_by_op: dict[str, list[tuple[str, float]]] = {}
    for r in rows_list:
        for op_name, op_data in r.get("ops", {}).items():
            mean = op_data.get("mean_ns")
            if mean is None:
                continue
            history_by_op.setdefault(op_name, []).append((r["timestamp"], mean))

    ops_rows = ""
    for op_name, op_data in sorted(latest["ops"].items()):
        mean = op_data.get("mean_ns")
        regression_pct = op_data.get("max_regression_pct")
        mean_str = _fmt_ns(mean) if mean is not None else "—"
        budget_str = f"{regression_pct:.1f}%" if regression_pct is not None else "—"
        drift_str = "—"
        cls = ""
        op_history = history_by_op.get(op_name, [])
        if mean is not None and len(op_history) >= 2:
            previous = op_history[-2][1]
            if previous > 0:
                delta = ((mean - previous) / previous) * 100
                drift_str = f"{delta:+.2f}%"
                if regression_pct is not None and delta > regression_pct:
                    cls = "over"
                elif regression_pct is not None and delta > regression_pct * 0.8:
                    cls = "close"
                else:
                    cls = "ok"
        ops_rows += (
            f'<tr class="{cls}"><td>{op_name}</td>'
            f"<td>{mean_str}</td><td>{budget_str}</td><td>{drift_str}</td></tr>\n"
        )
    html.parent.mkdir(parents=True, exist_ok=True)
    html.write_text(
        DASHBOARD_TEMPLATE.format(
            series_json=series_json,
            ops_rows=ops_rows or '<tr><td colspan="4">No data yet</td></tr>',
            timestamp=latest["timestamp"],
            git_sha=latest["git_sha"],
            run_count=len(rows_list),
        ),
        encoding="utf-8",
    )


def _fmt_ns(ns: float | None) -> str:
    if ns is None:
        return "—"
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.2f} µs"
    return f"{ns:.0f} ns"


DASHBOARD_TEMPLATE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>InvoiceKit benchmark dashboard</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  body {{ font: 14px/1.5 system-ui, sans-serif; margin: 2rem auto; max-width: 1100px; color: #111; padding: 0 1rem; }}
  h1 {{ margin-bottom: 0.25rem; }}
  .subtitle {{ color: #666; margin-top: 0; }}
  table {{ border-collapse: collapse; width: 100%; margin-top: 1rem; }}
  th, td {{ border-bottom: 1px solid #eee; padding: 0.5rem; text-align: right; font-variant-numeric: tabular-nums; }}
  th:first-child, td:first-child {{ text-align: left; }}
  tr.ok td {{ background: #f4fff4; }}
  tr.close td {{ background: #fff8e0; }}
  tr.over td {{ background: #ffe4e1; }}
  footer {{ color: #888; margin-top: 2rem; font-size: 0.85rem; }}
  code {{ background: #f0f0f0; padding: 0 4px; border-radius: 3px; }}
  details {{ margin-top: 1rem; }}
  summary {{ cursor: pointer; font-weight: 600; }}
</style>
</head>
<body>
<h1>InvoiceKit benchmark dashboard</h1>
<p class="subtitle">
  Criterion benchmark means per engine operation, compared against the per-op
  ceiling in <code>tools/perf-budget/budget.toml</code>. Rows in green are well
  inside budget; amber within 80% of ceiling; red over budget. Refreshed on
  every push to <code>main</code> by the bench workflow.
</p>

<h3>Latest run</h3>
<table>
  <thead><tr><th>Operation</th><th>Mean</th><th>Max regression</th><th>Drift vs. previous run</th></tr></thead>
  <tbody>
{ops_rows}  </tbody>
</table>

<details>
  <summary>Per-operation history (JSON)</summary>
  <p>The dashboard reads <code>docs/bench/history.jsonl</code>; download the
     file to plot in your favourite tool.</p>
</details>

<footer>
  Latest run <code>{timestamp}</code> at commit <code>{git_sha}</code>; total runs in history: {run_count}.
  Source: <code>tools/perf-budget/publish_dashboard.py</code>.
</footer>

<script>
  // History data is kept inline so future dashboards can render
  // a sparkline per op without an extra fetch.
  window.__benchHistory = {series_json};
</script>
</body>
</html>
"""


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)

    if not args.summary_only:
        row = collect_row(args.criterion, args.budget)
        append_row(args.history, row)

    rows = load_rows(args.history)
    render_dashboard(rows, args.html)
    print(f"published bench dashboard with {len(rows)} historical row(s) -> {args.html}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
