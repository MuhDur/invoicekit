"""CI gate: verify schemas/validation-result.schema.json matches the live derivation.

The committed schema is the contract bindings target. A future PR that changes
the Rust source of truth without regenerating the schema must fail CI.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
COMMITTED = REPO / "schemas" / "validation-result.schema.json"


def test_committed_schema_matches_live() -> None:
    """`cargo run --example emit_schema` output must equal the committed file byte-for-byte after pretty-format."""
    committed = json.loads(COMMITTED.read_text())
    live_raw = subprocess.run(
        ["cargo", "run", "-p", "invoicekit-validate", "--example", "emit_schema", "--quiet"],
        check=True,
        capture_output=True,
        text=True,
        cwd=REPO,
    ).stdout
    live = json.loads(live_raw)
    assert live == committed, (
        "schemas/validation-result.schema.json is out of sync with the Rust source of "
        "truth in crates/validate/src/lib.rs. Re-run: "
        "`cargo run -p invoicekit-validate --example emit_schema --quiet > "
        "schemas/validation-result.schema.json`"
    )
