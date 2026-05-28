# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Unit tests for tools/docs-generate/generate.py."""

from __future__ import annotations

import json
import pathlib
import sys
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "docs-generate"))

import generate  # noqa: E402


SAMPLE_RULE = {
    "id": "BR-CO-10",
    "business_terms": ["BT-106", "BT-110"],
    "business_groups": ["BG-22"],
    "current_ir_paths": ["Invoice.totals.tax_total"],
    "source_locations": [
        {
            "syntax": "UBL",
            "file": "ubl/xslt/EN16931-UBL-validation.xslt",
            "assert_line": 1234,
            "test": "$line-sum = $document-sum",
        }
    ],
    "rust_validator_testability": {
        "positive": True,
        "negative": False,
        "blocker": "needs tax-calculation closure",
    },
}


class GenerateRuleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="docs-gen-"))

    def test_render_rule_mdx_includes_business_terms_and_locations(self) -> None:
        body = generate.render_rule_mdx(SAMPLE_RULE)
        self.assertIn("# BR-CO-10", body)
        self.assertIn("**BT-106**", body)
        self.assertIn("**BT-110**", body)
        self.assertIn("**BG-22** (group)", body)
        self.assertIn("EN16931-UBL-validation.xslt", body)
        # blocker surfaces under testability
        self.assertIn("needs tax-calculation closure", body)

    def test_write_rule_pages_creates_one_mdx_per_rule_and_meta_json(self) -> None:
        rule_file = self.tmp / "rules.json"
        rule_file.write_text(
            json.dumps({"rules": [SAMPLE_RULE, {**SAMPLE_RULE, "id": "BR-02"}]}),
            encoding="utf-8",
        )
        site = self.tmp / "site"
        count = generate.write_rule_pages(rule_file, site)
        self.assertEqual(count, 2)
        self.assertTrue((site / "rules" / "BR-CO-10.mdx").exists())
        self.assertTrue((site / "rules" / "BR-02.mdx").exists())
        meta = json.loads((site / "rules" / "_meta.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["BR-CO-10"], "BR-CO-10")
        self.assertEqual(meta["BR-02"], "BR-02")
        self.assertEqual(meta["index"], "Overview")


class ParseReportCrateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="docs-gen-"))

    def test_parses_country_code_crate_name_and_description(self) -> None:
        cargo = self.tmp / "Cargo.toml"
        cargo.write_text(
            """
[package]
name = "invoicekit-report-cn-fapiao"
description = "China STA Golden Tax e-Fapiao adapter."
""",
            encoding="utf-8",
        )
        parsed = generate.parse_report_crate(cargo)
        self.assertIsNotNone(parsed)
        cc, crate, desc = parsed  # type: ignore[misc]
        self.assertEqual(cc, "CN")
        self.assertEqual(crate, "invoicekit-report-cn-fapiao")
        self.assertEqual(desc, "China STA Golden Tax e-Fapiao adapter.")

    def test_returns_none_for_non_report_crate(self) -> None:
        cargo = self.tmp / "Cargo.toml"
        cargo.write_text('[package]\nname = "invoicekit-money"\n', encoding="utf-8")
        self.assertIsNone(generate.parse_report_crate(cargo))

    def test_render_country_mdx_includes_country_name(self) -> None:
        body = generate.render_country_mdx(
            "CN", "invoicekit-report-cn-fapiao", "China STA Golden Tax e-Fapiao adapter."
        )
        self.assertIn("# China — `invoicekit-report-cn-fapiao`", body)
        self.assertIn("**Country code:** CN", body)
        self.assertIn("China STA Golden Tax e-Fapiao adapter.", body)


if __name__ == "__main__":
    unittest.main()
