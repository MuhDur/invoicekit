"""T-074d sandbox vs production parity diff tests.

These tests verify the consent and credentials gates BEFORE the
network — the safety-critical "no consent, no production call"
invariant gets a dedicated case and is asserted via a fake
fetcher that records every call. If consent is missing, the
fetcher must never see the production URL.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

import parity_diff  # noqa: E402 — sys.path manipulated above


def _write_toml(tmp_path: Path, consent: str = "") -> Path:
    text = f"""\
schema_version = "1.0"

[[pair]]
country = "TT"
pair_id = "tt-parity"
description = "Toy parity"
sandbox_endpoint = "https://sandbox.example.invalid"
production_endpoint = "https://prod.example.invalid"
sandbox_auth_env_var = "INVOICEKIT_SANDBOX_TT_TOKEN"
production_auth_env_var = "INVOICEKIT_PROD_TT_TOKEN"
request_fixture = "fixtures/tt.request.json"
consent_signed_at = "{consent}"
"""
    p = tmp_path / "config.toml"
    p.write_text(text, encoding="utf-8")
    return p


def _seed_request(tmp_path: Path) -> None:
    fdir = tmp_path / "fixtures"
    fdir.mkdir()
    (fdir / "tt.request.json").write_text(
        json.dumps({"method": "GET", "path": "/v1/probe", "headers": {}}), encoding="utf-8"
    )


def test_load_config_parses_one_pair(tmp_path: Path) -> None:
    cfg_path = _write_toml(tmp_path, consent="2026-05-27T10:00:00Z")
    pairs = parity_diff.load_config(cfg_path)
    assert len(pairs) == 1
    p = pairs[0]
    assert p.pair_id == "tt-parity"
    assert p.consent_signed_at == "2026-05-27T10:00:00Z"


def test_consent_gate_blocks_run_without_consent(tmp_path: Path) -> None:
    _write_toml(tmp_path, consent="")
    pairs = parity_diff.load_config(tmp_path / "config.toml")
    calls: list[tuple[str, str]] = []

    def recording_fetcher(endpoint, _req, _token):
        calls.append((endpoint, "called"))
        return {"status": 200, "headers": {}, "body": "{}"}

    result = parity_diff.check_pair(
        pairs[0],
        tmp_path,
        env={
            "INVOICEKIT_SANDBOX_TT_TOKEN": "set",
            "INVOICEKIT_PROD_TT_TOKEN": "set",
        },
        fetcher=recording_fetcher,
    )
    assert result.status == "skipped"
    assert "consent" in result.detail.lower()
    assert calls == [], "production endpoint must NEVER be called without consent"


def test_missing_sandbox_credentials_skips(tmp_path: Path) -> None:
    _write_toml(tmp_path, consent="2026-05-27T10:00:00Z")
    pairs = parity_diff.load_config(tmp_path / "config.toml")
    result = parity_diff.check_pair(
        pairs[0],
        tmp_path,
        env={"INVOICEKIT_PROD_TT_TOKEN": "set"},
    )
    assert result.status == "skipped"
    assert "sandbox credentials" in result.detail


def test_missing_production_credentials_skips(tmp_path: Path) -> None:
    _write_toml(tmp_path, consent="2026-05-27T10:00:00Z")
    pairs = parity_diff.load_config(tmp_path / "config.toml")
    result = parity_diff.check_pair(
        pairs[0],
        tmp_path,
        env={"INVOICEKIT_SANDBOX_TT_TOKEN": "set"},
    )
    assert result.status == "skipped"
    assert "production credentials" in result.detail


def test_parity_ok_when_responses_match(tmp_path: Path) -> None:
    _write_toml(tmp_path, consent="2026-05-27T10:00:00Z")
    _seed_request(tmp_path)
    pairs = parity_diff.load_config(tmp_path / "config.toml")

    def fake_fetcher(_endpoint, _req, _token):
        return {
            "status": 200,
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"status": "ok", "timestamp": "x"}),
        }

    result = parity_diff.check_pair(
        pairs[0],
        tmp_path,
        env={
            "INVOICEKIT_SANDBOX_TT_TOKEN": "set",
            "INVOICEKIT_PROD_TT_TOKEN": "set",
        },
        fetcher=fake_fetcher,
    )
    assert result.status == "ok", result.detail


def test_parity_drift_when_responses_differ(tmp_path: Path) -> None:
    _write_toml(tmp_path, consent="2026-05-27T10:00:00Z")
    _seed_request(tmp_path)
    pairs = parity_diff.load_config(tmp_path / "config.toml")
    state = {"first": True}

    def fake_fetcher(endpoint, _req, _token):
        # First call (sandbox) returns ok; second call (prod) returns degraded.
        if state["first"]:
            state["first"] = False
            return {"status": 200, "headers": {}, "body": json.dumps({"status": "ok"})}
        return {"status": 503, "headers": {}, "body": json.dumps({"status": "ok"})}

    result = parity_diff.check_pair(
        pairs[0],
        tmp_path,
        env={
            "INVOICEKIT_SANDBOX_TT_TOKEN": "set",
            "INVOICEKIT_PROD_TT_TOKEN": "set",
        },
        fetcher=fake_fetcher,
    )
    assert result.status == "drift"


def test_three_country_pair_baseline_in_committed_config() -> None:
    """Bead acceptance: tests on at least 3 country pairs."""
    config_path = (
        Path(__file__).resolve().parents[3]
        / "data"
        / "sandbox-prod-parity"
        / "config.toml"
    )
    pairs = parity_diff.load_config(config_path)
    assert len(pairs) >= 3, f"expected ≥3 country pairs; got {len(pairs)}"
    countries = {p.country for p in pairs}
    assert len(countries) >= 3, f"expected ≥3 distinct countries; got {countries}"


def test_render_report_groups_by_status() -> None:
    cfg = parity_diff.PairConfig(
        country="TT",
        pair_id="tt",
        description="Toy",
        sandbox_endpoint="x",
        production_endpoint="y",
        sandbox_auth_env_var="A",
        production_auth_env_var="B",
        request_fixture=Path("r"),
        consent_signed_at="2026-05-27T00:00:00Z",
    )
    results = [
        parity_diff.PairResult(cfg, status="drift", detail="1 pt", drift_paths=["a"]),
        parity_diff.PairResult(cfg, status="ok", detail=""),
        parity_diff.PairResult(cfg, status="skipped", detail="no consent"),
    ]
    text = parity_diff.render_report(results)
    assert "## DRIFT (1)" in text
    assert "## OK (1)" in text
    assert "## SKIPPED (1)" in text
    assert parity_diff.BEAD_ID in text


def test_load_config_rejects_unsupported_schema(tmp_path: Path) -> None:
    bad = tmp_path / "config.toml"
    bad.write_text('schema_version = "99.0"\n[[pair]]\n', encoding="utf-8")
    with pytest.raises(ValueError):
        parity_diff.load_config(bad)
