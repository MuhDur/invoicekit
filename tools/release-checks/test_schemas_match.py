"""CI gate: verify validator schemas match their live derivations.

The committed schema is the contract bindings target. A future PR that changes
the Rust source of truth without regenerating the matching schema must fail CI.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
VALIDATION_RESULT = REPO / "schemas" / "validation-result.schema.json"
VALIDATION_EXPLAIN_PLAN = REPO / "schemas" / "validation-explain-plan.schema.json"


def _live_schema(example: str) -> dict:
    live_raw = subprocess.run(
        ["cargo", "run", "-p", "invoicekit-validate", "--example", example, "--quiet"],
        check=True,
        capture_output=True,
        text=True,
        cwd=REPO,
    ).stdout
    return json.loads(live_raw)


def test_validation_result_schema_matches_live() -> None:
    """`emit_schema` output must equal the committed validation-result schema."""
    committed = json.loads(VALIDATION_RESULT.read_text())
    live = _live_schema("emit_schema")
    assert live == committed, (
        "schemas/validation-result.schema.json is out of sync with the Rust source of "
        "truth in crates/validate/src/lib.rs. Re-run: "
        "`cargo run -p invoicekit-validate --example emit_schema --quiet > "
        "schemas/validation-result.schema.json`"
    )


def test_validation_explain_plan_schema_matches_live() -> None:
    """`emit_explain_plan_schema` output must equal the committed explain-plan schema."""
    committed = json.loads(VALIDATION_EXPLAIN_PLAN.read_text())
    live = _live_schema("emit_explain_plan_schema")
    assert live == committed, (
        "schemas/validation-explain-plan.schema.json is out of sync with the Rust "
        "source of truth in crates/validate/src/lib.rs. Re-run: "
        "`cargo run -p invoicekit-validate --example emit_explain_plan_schema --quiet > "
        "schemas/validation-explain-plan.schema.json`"
    )
