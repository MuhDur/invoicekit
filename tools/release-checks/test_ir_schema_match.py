"""T-011 CI gates: the committed IR JSON Schema is in sync, and a known-good
synthetic CommercialDocument validates against it.

Two assertions:

1. `cargo run --bin gen-schema -p invoicekit-cli` produces the schema we
   committed at `schemas/invoicekit-ir-v1.json` byte-for-byte after JSON
   parse — any drift between the Rust source of truth in `crates/ir` and
   the committed schema fails the build.
2. A representative synthetic `CommercialDocument` JSON document validates
   against the committed schema; this catches the case where the schema
   compiles but excludes a payload shape the IR accepts (or vice versa).
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import jsonschema

REPO = Path(__file__).resolve().parents[2]
COMMITTED = REPO / "schemas" / "invoicekit-ir-v1.json"


def _live_schema() -> dict:
    raw = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "invoicekit-cli",
            "--bin",
            "gen-schema",
        ],
        check=True,
        capture_output=True,
        text=True,
        cwd=REPO,
    ).stdout
    return json.loads(raw)


def test_committed_schema_matches_live() -> None:
    committed = json.loads(COMMITTED.read_text())
    live = _live_schema()
    assert committed == live, (
        "schemas/invoicekit-ir-v1.json is out of sync with crates/ir. Re-run: "
        "`cargo run -p invoicekit-cli --bin gen-schema --quiet > "
        "schemas/invoicekit-ir-v1.json`"
    )


def test_synthetic_invoice_validates_against_committed_schema() -> None:
    """A representative CommercialDocument JSON validates against the schema."""
    committed = json.loads(COMMITTED.read_text())
    sample = {
        "schema_version": "1.0",
        "id": "doc-t-011-validation-1",
        "document_type": "invoice",
        "issue_date": "2026-05-26",
        "due_date": "2026-06-25",
        "document_number": "INV-T-011-0001",
        "currency": "EUR",
        "supplier": {
            "id": "supplier-1",
            "name": "InvoiceKit GmbH",
            "tax_ids": [{"scheme": "vat", "value": "DE123456789"}],
            "address": {
                "lines": ["Main Street 1"],
                "city": "Berlin",
                "postal_code": "10115",
                "country": "DE",
            },
        },
        "customer": {
            "id": "customer-1",
            "name": "ACME SAS",
            "tax_ids": [{"scheme": "vat", "value": "FR123456789"}],
            "address": {
                "lines": ["Rue Principale 1"],
                "city": "Paris",
                "postal_code": "75001",
                "country": "FR",
            },
        },
        "payment_instructions": [],
        "lines": [
            {
                "id": "1",
                "description": "T-011 schema validation smoke",
                "quantity": "1",
                "unit_code": "EA",
                "unit_price": "100.00",
                "line_extension_amount": "100.00",
                "tax_category": "S",
                "extensions": [],
            }
        ],
        "tax_summary": [
            {
                "category_code": "S",
                "taxable_amount": "100.00",
                "tax_amount": "19.00",
                "tax_rate": "19.00",
            }
        ],
        "monetary_total": {
            "line_extension_amount": "100.00",
            "tax_exclusive_amount": "100.00",
            "tax_inclusive_amount": "119.00",
            "payable_amount": "119.00",
        },
        "extensions": [
            {
                "urn": "urn:invoicekit:ext:generic:test:1.0",
                "payload": {"profile_hint": "peppol-bis"},
            }
        ],
        "meta": {
            "tenant_id": "tenant_t011",
            "trace_id": "trace_t011",
        },
    }
    # Will raise if the document does not validate. The schemars 0.9 output
    # is Draft 2020-12; jsonschema's Draft202012Validator is the matching
    # validator.
    validator = jsonschema.Draft202012Validator(committed)
    errors = sorted(validator.iter_errors(sample), key=str)
    assert not errors, "synthetic CommercialDocument failed schema validation: " + "; ".join(
        f"{list(e.absolute_path)}: {e.message}" for e in errors
    )
