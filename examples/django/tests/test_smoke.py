# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 smoke test: hit every endpoint via Django's test client.
# Bypasses the wsgi/runserver layer so the gate stays fast and
# runs the same code path the server would.

from __future__ import annotations

import os
import sys
from pathlib import Path

import django


# Make sibling modules (fixtures, invoicekit_bridge) importable
# without requiring the demo to be installed as a package.
HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))

os.environ.setdefault("DJANGO_SETTINGS_MODULE", "demo.settings")
django.setup()

from django.test import Client  # noqa: E402

from fixtures import FIXTURES  # noqa: E402


def test_root_lists_fixtures() -> None:
    response = Client().get("/")
    assert response.status_code == 200
    body = response.json()
    assert body["title"] == "InvoiceKit Django demo"
    assert set(body["fixtures"]) == set(FIXTURES.keys())


def test_healthz() -> None:
    response = Client().get("/healthz")
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}


def test_canonicalize_basic_fixture() -> None:
    response = Client().post("/canonicalize/basic")
    assert response.status_code == 200, response.content
    body = response.json()
    assert body["_engine_status"] == 0, body
    assert body["status"] == "ok", body


def test_canonicalize_with_allowance_fixture() -> None:
    response = Client().post("/canonicalize/with-allowance")
    assert response.status_code == 200, response.content
    assert response.json()["_engine_status"] == 0


def test_canonicalize_reverse_charge_fixture() -> None:
    response = Client().post("/canonicalize/reverse-charge")
    assert response.status_code == 200, response.content
    assert response.json()["_engine_status"] == 0


def test_unknown_fixture_returns_404() -> None:
    response = Client().post("/canonicalize/does-not-exist")
    assert response.status_code == 404
    body = response.json()
    assert body["error"]["code"] == "UNKNOWN_FIXTURE"
    assert "basic" in body["error"]["available"]
