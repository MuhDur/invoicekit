# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Unit tests for tools/perf-budget/publish_dashboard.py."""

from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "perf-budget"))

import publish_dashboard  # noqa: E402


def _criterion_op(root: pathlib.Path, op: str, mean_ns: float) -> None:
    (root / op / "new").mkdir(parents=True, exist_ok=True)
    (root / op / "new" / "estimates.json").write_text(
        json.dumps({"mean": {"point_estimate": mean_ns}}),
        encoding="utf-8",
    )


def _budget(path: pathlib.Path, ops: dict[str, float]) -> None:
    """Write a budget.toml matching the production schema.

    `ops` maps each operation name to its max_regression_pct.
    """
    lines = ["default_max_regression_pct = 10.0"]
    for op, regression_pct in ops.items():
        lines.append(f"[operations.{op!s}]")
        lines.append(f"max_regression_pct = {regression_pct}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


class PublishBenchDashboardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="bench-dashboard-"))

    def test_collect_row_includes_each_budgeted_op(self) -> None:
        criterion = self.tmp / "criterion"
        budget = self.tmp / "budget.toml"
        _criterion_op(criterion, "validate-ubl", 12_345.0)
        _criterion_op(criterion, "render-pdf", 678_910.0)
        _budget(budget, {'"validate-ubl"': 10.0, '"render-pdf"': 5.0})

        row = publish_dashboard.collect_row(criterion, budget)
        self.assertIn("ops", row)
        self.assertEqual(row["ops"]["validate-ubl"]["mean_ns"], 12_345.0)
        self.assertEqual(row["ops"]["validate-ubl"]["max_regression_pct"], 10.0)
        self.assertEqual(row["ops"]["render-pdf"]["mean_ns"], 678_910.0)
        self.assertEqual(row["ops"]["render-pdf"]["max_regression_pct"], 5.0)

    def test_collect_row_records_null_for_missing_criterion_output(self) -> None:
        criterion = self.tmp / "criterion"
        criterion.mkdir()
        budget = self.tmp / "budget.toml"
        _budget(budget, {'"absent-op"': 10.0})

        row = publish_dashboard.collect_row(criterion, budget)
        self.assertIsNone(row["ops"]["absent-op"]["mean_ns"])
        self.assertEqual(row["ops"]["absent-op"]["max_regression_pct"], 10.0)

    def test_render_dashboard_emits_drift_versus_previous_run(self) -> None:
        html = self.tmp / "index.html"
        rows = [
            {
                "timestamp": "2026-05-27T00:00:00+00:00",
                "git_sha": "deadbee",
                "ops": {
                    "validate-ubl": {"mean_ns": 100.0, "max_regression_pct": 10.0},
                    "render-pdf": {"mean_ns": 1000.0, "max_regression_pct": 10.0},
                },
            },
            {
                "timestamp": "2026-05-28T00:00:00+00:00",
                "git_sha": "cafe123",
                "ops": {
                    # 5% drift, within budget -> ok class
                    "validate-ubl": {"mean_ns": 105.0, "max_regression_pct": 10.0},
                    # 15% drift, over the 10% ceiling -> over class
                    "render-pdf": {"mean_ns": 1150.0, "max_regression_pct": 10.0},
                },
            },
        ]
        publish_dashboard.render_dashboard(rows, html)
        body = html.read_text(encoding="utf-8")
        self.assertIn("InvoiceKit benchmark dashboard", body)
        self.assertIn("validate-ubl", body)
        self.assertIn("render-pdf", body)
        # Drift columns rendered with sign
        self.assertIn("+5.00%", body)
        self.assertIn("+15.00%", body)
        # The over-budget op picks up the red class
        self.assertIn('class="over"', body)
        # __benchHistory inline JSON round-trip preserves both rows
        self.assertIn('"git_sha": "deadbee"', body)
        self.assertIn('"git_sha": "cafe123"', body)

    def test_render_dashboard_handles_empty_history(self) -> None:
        html = self.tmp / "index.html"
        publish_dashboard.render_dashboard([], html)
        body = html.read_text(encoding="utf-8")
        self.assertIn("No data yet", body)


if __name__ == "__main__":
    unittest.main()
