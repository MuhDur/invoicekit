"""T-074c sandbox drift canary tests.

We exercise the pure-Python core (no real HTTP) by injecting a
fake fetcher. The HTTP layer itself is a thin urllib wrapper that
gets exercised by the workflow's nightly run.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

import sandbox_drift  # noqa: E402 — sys.path manipulated above


def _write_toml(tmp_path: Path) -> Path:
    text = """\
schema_version = "1.0"

[[gateway]]
country = "TT"
gateway_id = "tt-test"
description = "Toy test gateway"
endpoint = "https://example.invalid"
auth_env_var = "INVOICEKIT_TEST_TOKEN"
cassette_path = "cassettes/tt.json"
request_fixture = "cassettes/tt.request.json"
"""
    p = tmp_path / "config.toml"
    p.write_text(text, encoding="utf-8")
    return p


def _seed_repo(tmp_path: Path, cassette: dict, request: dict) -> Path:
    cdir = tmp_path / "cassettes"
    cdir.mkdir()
    (cdir / "tt.json").write_text(json.dumps(cassette), encoding="utf-8")
    (cdir / "tt.request.json").write_text(json.dumps(request), encoding="utf-8")
    return tmp_path


def test_load_config_parses_one_stanza(tmp_path: Path) -> None:
    cfg_path = _write_toml(tmp_path)
    gateways = sandbox_drift.load_config(cfg_path)
    assert len(gateways) == 1
    g = gateways[0]
    assert g.country == "TT"
    assert g.gateway_id == "tt-test"
    assert g.auth_env_var == "INVOICEKIT_TEST_TOKEN"


def test_load_config_rejects_unsupported_schema_version(tmp_path: Path) -> None:
    bad = tmp_path / "config.toml"
    bad.write_text('schema_version = "99.0"\n[[gateway]]\n', encoding="utf-8")
    with pytest.raises(ValueError):
        sandbox_drift.load_config(bad)


def test_check_gateway_skips_when_credentials_missing(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")
    result = sandbox_drift.check_gateway(gateways[0], tmp_path, env={})
    assert result.status == "skipped"
    assert "no credentials" in result.detail


def test_check_gateway_skips_when_cassette_missing(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")
    result = sandbox_drift.check_gateway(
        gateways[0], tmp_path, env={"INVOICEKIT_TEST_TOKEN": "set"}
    )
    assert result.status == "skipped"
    assert "cassette missing" in result.detail


def test_check_gateway_reports_ok_on_matching_response(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    expected = {
        "status": 200,
        "headers": {"content-type": "application/json", "date": "Tue, 27 May 2026 00:00:00 GMT"},
        "body": json.dumps({"status": "ok", "timestamp": "old"}),
    }
    request = {"method": "POST", "path": "/v1/probe", "body": {"foo": "bar"}, "headers": {}}
    _seed_repo(tmp_path, expected, request)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")

    def fake_fetch(endpoint, req, token):
        # Same response except for jitter fields (timestamp, request_id, date).
        return {
            "status": 200,
            "headers": {"content-type": "application/json", "date": "Wed, 28 May 2026 00:00:00 GMT"},
            "body": json.dumps({"status": "ok", "timestamp": "new"}),
        }

    result = sandbox_drift.check_gateway(
        gateways[0],
        tmp_path,
        env={"INVOICEKIT_TEST_TOKEN": "set"},
        fetcher=fake_fetch,
    )
    assert result.status == "ok", result.detail
    assert result.drift_paths == []


def test_check_gateway_reports_drift_on_status_change(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    expected = {"status": 200, "headers": {}, "body": json.dumps({"status": "ok"})}
    request = {"method": "GET", "path": "/v1/probe", "headers": {}}
    _seed_repo(tmp_path, expected, request)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")

    def fake_fetch(_endpoint, _req, _token):
        return {"status": 503, "headers": {}, "body": json.dumps({"status": "ok"})}

    result = sandbox_drift.check_gateway(
        gateways[0],
        tmp_path,
        env={"INVOICEKIT_TEST_TOKEN": "set"},
        fetcher=fake_fetch,
    )
    assert result.status == "drift"
    assert any("status" in path for path in result.drift_paths)


def test_check_gateway_reports_drift_on_body_field_change(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    expected = {
        "status": 200,
        "headers": {},
        "body": json.dumps({"vat_rate": "21", "country": "TT"}),
    }
    request = {"method": "GET", "path": "/v1/profile", "headers": {}}
    _seed_repo(tmp_path, expected, request)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")

    def fake_fetch(_endpoint, _req, _token):
        return {
            "status": 200,
            "headers": {},
            "body": json.dumps({"vat_rate": "23", "country": "TT"}),
        }

    result = sandbox_drift.check_gateway(
        gateways[0],
        tmp_path,
        env={"INVOICEKIT_TEST_TOKEN": "set"},
        fetcher=fake_fetch,
    )
    assert result.status == "drift"
    assert any("vat_rate" in path for path in result.drift_paths)


def test_check_gateway_reports_error_on_fetcher_exception(tmp_path: Path) -> None:
    _write_toml(tmp_path)
    expected = {"status": 200, "headers": {}, "body": "{}"}
    request = {"method": "GET", "path": "/v1/probe", "headers": {}}
    _seed_repo(tmp_path, expected, request)
    gateways = sandbox_drift.load_config(tmp_path / "config.toml")

    def boom(_endpoint, _req, _token):
        raise ConnectionError("simulated network outage")

    result = sandbox_drift.check_gateway(
        gateways[0],
        tmp_path,
        env={"INVOICEKIT_TEST_TOKEN": "set"},
        fetcher=boom,
    )
    assert result.status == "error"
    assert "outage" in result.detail


def test_render_report_groups_by_status(tmp_path: Path) -> None:
    cfg = sandbox_drift.GatewayConfig(
        country="TT",
        gateway_id="tt-test",
        description="Toy",
        endpoint="https://example.invalid",
        auth_env_var="X",
        cassette_path=Path("c"),
        request_fixture=Path("r"),
    )
    results = [
        sandbox_drift.GatewayResult(cfg, status="drift", detail="2 points", drift_paths=["a", "b"]),
        sandbox_drift.GatewayResult(cfg, status="skipped", detail="no credentials"),
        sandbox_drift.GatewayResult(cfg, status="ok", detail=""),
    ]
    text = sandbox_drift.render_report(results)
    assert "## DRIFT (1)" in text
    assert "## SKIPPED (1)" in text
    assert "## OK (1)" in text
    assert sandbox_drift.BEAD_ID in text


def test_committed_config_loads(tmp_path: Path) -> None:
    """Smoke: the checked-in production config must parse."""
    config_path = Path(__file__).resolve().parents[3] / "data" / "sandbox-drift" / "config.toml"
    assert config_path.exists(), f"missing {config_path}"
    gateways = sandbox_drift.load_config(config_path)
    assert gateways, "config.toml has zero gateways; CI smoke is meaningless"
    # Every stanza must point at an env var no developer is likely to
    # have set locally — the canary must default to "everything
    # skipped" on a clean checkout.
    for g in gateways:
        assert g.auth_env_var.startswith("INVOICEKIT_SANDBOX_"), g.auth_env_var
