# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1406 smoke test: hit /canonicalize for every fixture via the
# FastAPI TestClient. Bypasses uvicorn so the gate stays fast and
# runs the same code path the server would.

from __future__ import annotations

import sys
from pathlib import Path

from fastapi.testclient import TestClient

# Make sibling modules (app, fixtures, invoicekit_bridge) importable
# without requiring the demo to be installed as a package.
HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))

from app import app  # noqa: E402
from fixtures import FIXTURES  # noqa: E402


client = TestClient(app)


def test_root_lists_fixtures() -> None:
    response = client.get("/")
    assert response.status_code == 200
    body = response.json()
    assert body["title"] == "InvoiceKit FastAPI demo"
    assert set(body["fixtures"]) == set(FIXTURES.keys())


def test_healthz() -> None:
    response = client.get("/healthz")
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}


def test_canonicalize_basic_fixture() -> None:
    response = client.post("/canonicalize/basic")
    assert response.status_code == 200, response.text
    body = response.json()
    # The engine returns the canonicalised payload alongside an
    # _engine_status field the bridge injects.
    assert body["_engine_status"] == 0, body
    assert body["status"] == "ok", body


def test_canonicalize_with_allowance_fixture() -> None:
    response = client.post("/canonicalize/with-allowance")
    assert response.status_code == 200, response.text
    assert response.json()["_engine_status"] == 0


def test_canonicalize_reverse_charge_fixture() -> None:
    response = client.post("/canonicalize/reverse-charge")
    assert response.status_code == 200, response.text
    assert response.json()["_engine_status"] == 0


def test_unknown_fixture_returns_404() -> None:
    response = client.post("/canonicalize/does-not-exist")
    assert response.status_code == 404
    body = response.json()
    assert body["detail"]["code"] == "UNKNOWN_FIXTURE"
    assert "basic" in body["detail"]["available"]
