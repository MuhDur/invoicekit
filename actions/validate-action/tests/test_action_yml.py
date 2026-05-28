# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Schema-level tests for actions/validate-action/action.yml.

These tests run in CI before any release tag is cut so the
GitHub Marketplace manifest is guaranteed to parse and to
expose every input/output the README documents.
"""

from __future__ import annotations

import pathlib
import sys
import unittest

REPO = pathlib.Path(__file__).resolve().parents[3]

try:
    import yaml  # type: ignore
except ImportError:  # pragma: no cover - CI installs pyyaml
    print("pyyaml is required; pip install pyyaml", file=sys.stderr)
    raise


ACTION_PATH = REPO / "actions" / "validate-action" / "action.yml"


class ActionYmlTests(unittest.TestCase):
    def setUp(self) -> None:
        self.data = yaml.safe_load(ACTION_PATH.read_text(encoding="utf-8"))

    def test_required_marketplace_fields_present(self) -> None:
        for key in ("name", "description", "author", "branding", "runs"):
            self.assertIn(key, self.data, f"action.yml missing `{key}`")
        self.assertEqual(self.data["runs"]["using"], "composite")

    def test_inputs_match_readme_contract(self) -> None:
        inputs = self.data["inputs"]
        for name in (
            "path",
            "rule-pack",
            "fail-on",
            "invoicekit-version",
            "summary-artifact",
        ):
            self.assertIn(name, inputs, f"input `{name}` missing")
            self.assertIn("description", inputs[name])

    def test_outputs_expose_finding_counts_and_summary_path(self) -> None:
        outputs = self.data["outputs"]
        for name in (
            "fatal-count",
            "error-count",
            "warning-count",
            "files-validated",
            "summary-path",
        ):
            self.assertIn(name, outputs, f"output `{name}` missing")

    def test_branding_uses_marketplace_allowed_icon(self) -> None:
        # GitHub's Marketplace rejects unknown icons; sanity check
        # against the Feather-icons subset GitHub allows.
        self.assertIn(self.data["branding"]["icon"], _ALLOWED_BRANDING_ICONS)
        self.assertIn(self.data["branding"]["color"], _ALLOWED_BRANDING_COLORS)

    def test_validate_step_invokes_invoicekit_cli_with_expected_flags(self) -> None:
        steps = self.data["runs"]["steps"]
        validate_step = next(s for s in steps if s.get("id") == "validate")
        run = validate_step["run"]
        for flag in ("--glob", "--rule-pack", "--fail-on", "--json", "--annotations"):
            self.assertIn(flag, run, f"validate step missing flag `{flag}`")


_ALLOWED_BRANDING_ICONS = {
    "check-circle",
    "alert-circle",
    "shield",
    "file-text",
    "box",
}
_ALLOWED_BRANDING_COLORS = {
    "white",
    "yellow",
    "blue",
    "green",
    "orange",
    "red",
    "purple",
    "gray-dark",
}


if __name__ == "__main__":
    unittest.main()
