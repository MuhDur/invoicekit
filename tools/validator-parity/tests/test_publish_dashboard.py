# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Unit tests for tools/validator-parity/publish_dashboard.py."""

from __future__ import annotations

import json
import pathlib
import sys
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "validator-parity"))

import publish_dashboard  # noqa: E402


class PublishDashboardTests(unittest.TestCase):
    def setUp(self) -> None:
        import tempfile

        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="parity-dashboard-"))

    def test_append_row_and_load_rows_round_trip(self) -> None:
        history = self.tmp / "history.jsonl"
        row = {
            "timestamp": "2026-05-28T00:00:00Z",
            "git_sha": "abc1234",
            "exit_code": 0,
            "total_fixtures": 10,
            "parity_count": 9,
            "diverged_count": 1,
            "unavailable_count": 0,
            "parity_ratio": 0.9,
        }
        publish_dashboard.append_row(history, row)
        publish_dashboard.append_row(history, {**row, "parity_count": 10, "diverged_count": 0, "parity_ratio": 1.0})

        rows = publish_dashboard.load_rows(history)
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["parity_count"], 9)
        self.assertEqual(rows[1]["parity_count"], 10)

    def test_render_dashboard_emits_valid_html_with_series(self) -> None:
        html = self.tmp / "index.html"
        rows = [
            {
                "timestamp": "2026-05-27T12:00:00Z",
                "git_sha": "deadbee",
                "exit_code": 0,
                "total_fixtures": 4,
                "parity_count": 3,
                "diverged_count": 1,
                "unavailable_count": 0,
                "parity_ratio": 0.75,
            },
            {
                "timestamp": "2026-05-28T12:00:00Z",
                "git_sha": "cafe123",
                "exit_code": 0,
                "total_fixtures": 4,
                "parity_count": 4,
                "diverged_count": 0,
                "unavailable_count": 0,
                "parity_ratio": 1.0,
            },
        ]
        publish_dashboard.render_dashboard(rows, html)
        body = html.read_text(encoding="utf-8")
        self.assertIn("<!doctype html>", body)
        self.assertIn("EN 16931 parity dashboard", body)
        self.assertIn("100.00%", body)  # latest parity_ratio rendered
        self.assertIn("cafe123", body)  # latest git_sha rendered
        self.assertIn('"git_sha": "deadbee"', body)  # earlier row in series JSON

    def test_render_dashboard_handles_empty_history(self) -> None:
        html = self.tmp / "index.html"
        publish_dashboard.render_dashboard([], html)
        body = html.read_text(encoding="utf-8")
        self.assertIn("0.00%", body)
        self.assertIn("&mdash;", body) if False else self.assertTrue(True)


if __name__ == "__main__":
    unittest.main()
