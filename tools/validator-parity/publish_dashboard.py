#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Publish the EN 16931 parity dashboard for parity.invoicekit.org.

Runs the differential harness against the JVM sidecars, appends a
new row to ``docs/parity/history.jsonl``, and renders a static HTML
dashboard at ``docs/parity/index.html``. The dashboard is a single
self-contained file (inline CSS + inline JS, no external CDN) so it
can be served from any static host without extra plumbing.

Usage::

    python3 tools/validator-parity/publish_dashboard.py \\
        --history docs/parity/history.jsonl \\
        --html    docs/parity/index.html

The harness inputs (corpus fixtures, sidecar URLs) come from the
existing ``en16931_parity.py`` driver; this script delegates to it
via subprocess and aggregates the per-fixture verdicts into a
single time-series row.
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
PARITY_DRIVER = REPO / "tools" / "validator-parity" / "en16931_parity.py"


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--history",
        type=pathlib.Path,
        default=REPO / "docs" / "parity" / "history.jsonl",
        help="JSONL history file (appended to)",
    )
    p.add_argument(
        "--html",
        type=pathlib.Path,
        default=REPO / "docs" / "parity" / "index.html",
        help="Output static HTML dashboard",
    )
    p.add_argument(
        "--summary-only",
        action="store_true",
        help="Skip running the parity driver; rebuild HTML from existing history",
    )
    return p.parse_args(argv)


def run_parity_driver() -> dict:
    """Invoke the existing driver, return a summary row.

    The driver's exit code is meaningful (0 = parity, non-zero =
    divergence); this wrapper captures both. Failure does not stop
    the dashboard from being published — it surfaces in the row.
    """
    proc = subprocess.run(
        [sys.executable, str(PARITY_DRIVER), "--json"],
        capture_output=True,
        text=True,
        cwd=REPO,
    )
    fixtures: list[dict] = []
    if proc.stdout.strip():
        # The driver emits one JSON object per fixture (NDJSON).
        for line in proc.stdout.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                fixtures.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    passed = sum(1 for f in fixtures if f.get("status") == "parity")
    diverged = sum(1 for f in fixtures if f.get("status") == "diverged")
    unavailable = sum(1 for f in fixtures if f.get("status") == "unavailable")
    total = len(fixtures)
    return {
        "timestamp": _dt.datetime.now(tz=_dt.timezone.utc).isoformat(),
        "git_sha": os.environ.get("GITHUB_SHA")
        or _git_sha()
        or "unknown",
        "exit_code": proc.returncode,
        "total_fixtures": total,
        "parity_count": passed,
        "diverged_count": diverged,
        "unavailable_count": unavailable,
        "parity_ratio": (passed / total) if total else 0.0,
    }


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
        "parity_count": 0,
        "diverged_count": 0,
        "unavailable_count": 0,
        "total_fixtures": 0,
        "parity_ratio": 0.0,
        "git_sha": "—",
    }
    parity_pct = f"{latest['parity_ratio'] * 100:.2f}%"
    html.parent.mkdir(parents=True, exist_ok=True)
    html.write_text(
        DASHBOARD_TEMPLATE.format(
            series_json=series_json,
            parity_pct=parity_pct,
            parity_count=latest["parity_count"],
            diverged_count=latest["diverged_count"],
            unavailable_count=latest["unavailable_count"],
            total_fixtures=latest["total_fixtures"],
            git_sha=latest["git_sha"],
            timestamp=latest["timestamp"],
        ),
        encoding="utf-8",
    )


DASHBOARD_TEMPLATE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>InvoiceKit EN 16931 parity dashboard</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  body {{ font: 14px/1.5 system-ui, sans-serif; margin: 2rem auto; max-width: 960px; color: #111; padding: 0 1rem; }}
  h1 {{ margin-bottom: 0.25rem; }}
  .subtitle {{ color: #666; margin-top: 0; }}
  .cards {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 1rem; margin: 2rem 0; }}
  .card {{ border: 1px solid #ddd; border-radius: 8px; padding: 1rem; background: #fafafa; }}
  .card h2 {{ font-size: 2rem; margin: 0; color: #2a4d9b; }}
  .card p {{ margin: 0; color: #555; font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.05em; }}
  table {{ border-collapse: collapse; width: 100%; margin-top: 1.5rem; }}
  th, td {{ border-bottom: 1px solid #eee; padding: 0.5rem; text-align: right; font-variant-numeric: tabular-nums; }}
  th:first-child, td:first-child {{ text-align: left; }}
  footer {{ color: #888; margin-top: 2rem; font-size: 0.85rem; }}
  code {{ background: #f0f0f0; padding: 0 4px; border-radius: 3px; }}
</style>
</head>
<body>
<h1>EN 16931 parity dashboard</h1>
<p class="subtitle">
  Pure-Rust validator vs. JVM reference sidecars (KOSIT validator, PHIVE) over the synthetic UBL 2.1 conformance corpus.
  Updated automatically on each main-branch run.
</p>

<div class="cards">
  <div class="card"><h2>{parity_pct}</h2><p>Latest parity</p></div>
  <div class="card"><h2>{parity_count}</h2><p>Fixtures in agreement</p></div>
  <div class="card"><h2>{diverged_count}</h2><p>Diverged</p></div>
  <div class="card"><h2>{unavailable_count}</h2><p>Oracle unavailable</p></div>
  <div class="card"><h2>{total_fixtures}</h2><p>Total fixtures</p></div>
</div>

<h3>Parity over time</h3>
<table>
  <thead><tr><th>Timestamp (UTC)</th><th>Parity %</th><th>Parity</th><th>Diverged</th><th>Unavailable</th><th>Commit</th></tr></thead>
  <tbody id="rows"></tbody>
</table>

<footer>
  Latest run <code>{timestamp}</code> at commit <code>{git_sha}</code>.
  History JSONL: <code>docs/parity/history.jsonl</code>. Source:
  <code>tools/validator-parity/publish_dashboard.py</code>.
</footer>

<script>
  const rows = {series_json};
  const tbody = document.getElementById("rows");
  for (const r of rows.slice().reverse()) {{
    const tr = document.createElement("tr");
    tr.innerHTML = ""
      + "<td>" + r.timestamp + "</td>"
      + "<td>" + (r.parity_ratio * 100).toFixed(2) + "%</td>"
      + "<td>" + r.parity_count + "</td>"
      + "<td>" + r.diverged_count + "</td>"
      + "<td>" + r.unavailable_count + "</td>"
      + "<td><code>" + (r.git_sha || "—").slice(0, 7) + "</code></td>";
    tbody.appendChild(tr);
  }}
</script>
</body>
</html>
"""


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)

    if not args.summary_only:
        row = run_parity_driver()
        append_row(args.history, row)

    rows = load_rows(args.history)
    render_dashboard(rows, args.html)
    print(f"published dashboard with {len(rows)} historical row(s) -> {args.html}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
