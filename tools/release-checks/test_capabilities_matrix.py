"""T-006a CI gate: the bundled capability matrix validates against its JSON Schema.

Two assertions:

1. ``schemas/invoicekit-capabilities-v1.json`` parses as Draft 2020-12.
2. The matrix shipped at ``crates/cli/data/capabilities/matrix.json``
   (compiled into the ``invoicekit`` binary via ``include_str!``)
   validates against that schema.

This catches the case where the Rust source of truth and the JSON
Schema drift apart, or where the bundled matrix gains a row that
violates the schema and would otherwise only fail at runtime.
"""

from __future__ import annotations

import json
from pathlib import Path

import jsonschema

REPO = Path(__file__).resolve().parents[2]
SCHEMA = REPO / "schemas" / "invoicekit-capabilities-v1.json"
MATRIX = REPO / "crates" / "cli" / "data" / "capabilities" / "matrix.json"


def test_capabilities_schema_is_valid_draft_2020_12() -> None:
    schema = json.loads(SCHEMA.read_text())
    jsonschema.Draft202012Validator.check_schema(schema)


def test_bundled_matrix_validates_against_schema() -> None:
    schema = json.loads(SCHEMA.read_text())
    matrix = json.loads(MATRIX.read_text())
    jsonschema.Draft202012Validator(schema).validate(matrix)


def test_bundled_matrix_declares_supported_schema_version() -> None:
    matrix = json.loads(MATRIX.read_text())
    assert (
        matrix["schema_version"] == "1.0"
    ), "bundled matrix schema_version must match SUPPORTED_MATRIX_SCHEMA_VERSION in capabilities.rs"
