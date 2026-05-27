#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""T-074c nightly sandbox drift canary.

Reads ``data/sandbox-drift/config.toml``, walks every ``[[gateway]]``
stanza, and for each stanza:

1. **Credentials check.** If the stanza's ``auth_env_var`` is unset
   in the environment, the gateway is recorded as
   ``skipped: no credentials`` and the canary moves on. This is the
   acceptance-criterion "configurable per country (skip if no sandbox
   credentials)" — a missing env var is the contract for "we don't
   have an account here yet."
2. **Cassette check.** If the recorded cassette at ``cassette_path``
   doesn't exist, the canary records ``skipped: cassette missing`` —
   a future bead can record one, but the absence is not itself a
   drift event.
3. **Live replay.** Resolves ``request_fixture`` into an HTTP
   request, sends it to ``endpoint``, normalizes both the live
   response and the cassette's expected response via the same
   ``_normalize_response`` helper, and compares them. Any structural
   difference (status code, header keys, JSON body keys, or matching
   string bodies) is a drift event.
4. **Drift reporting.** Drift events are aggregated into one summary
   per run; if ``--report-mode=github-issue`` is set and at least one
   gateway drifted, the canary runs ``gh issue create`` to open a
   single triage issue listing every drifted gateway. The label
   ``sandbox-drift`` makes the issue easy to filter in the regular
   triage view.

The canary's design rule is "never fail the workflow." A network
flake on one country shouldn't page the whole org; the run records
the failure as a structured event and exits 0. Real drift opens an
issue (which paging policy can hook onto) but still exits 0 so
GitHub Actions doesn't mark the scheduled run as failing.

Run with ``--report-mode=stdout`` for local dry-runs; the CI
workflow uses ``--report-mode=github-issue``.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# Pinned bead identifier so a future grep can correlate issue bodies
# with the bead.
BEAD_ID = "invoices-t-074c-sandbox-drift-canary-vw5"

# Labels stamped on every github-issue report so triage can filter.
DEFAULT_LABELS = ["sandbox-drift", "track-6", "automation"]


@dataclass
class GatewayConfig:
    """One ``[[gateway]]`` stanza from ``config.toml``."""

    country: str
    gateway_id: str
    description: str
    endpoint: str
    auth_env_var: str
    cassette_path: Path
    request_fixture: Path


@dataclass
class GatewayResult:
    """Outcome of one gateway's nightly check."""

    gateway: GatewayConfig
    status: str  # "ok" | "drift" | "skipped" | "error"
    detail: str
    drift_paths: list[str] = field(default_factory=list)

    def to_summary_line(self) -> str:
        bits = [f"[{self.status.upper()}]", self.gateway.country, self.gateway.gateway_id]
        if self.detail:
            bits.append("—")
            bits.append(self.detail)
        return " ".join(bits)


def load_config(path: Path) -> list[GatewayConfig]:
    """Parse ``config.toml`` into a list of [`GatewayConfig`] objects."""
    raw = path.read_text(encoding="utf-8")
    data = tomllib.loads(raw)
    if data.get("schema_version") != "1.0":
        raise ValueError(
            f"unsupported schema_version {data.get('schema_version')!r} in {path}"
        )
    gateways = []
    for entry in data.get("gateway", []):
        gateways.append(
            GatewayConfig(
                country=entry["country"],
                gateway_id=entry["gateway_id"],
                description=entry["description"],
                endpoint=entry["endpoint"],
                auth_env_var=entry["auth_env_var"],
                cassette_path=Path(entry["cassette_path"]),
                request_fixture=Path(entry["request_fixture"]),
            )
        )
    return gateways


def check_gateway(
    cfg: GatewayConfig,
    repo_root: Path,
    *,
    env: dict[str, str] | None = None,
    fetcher=None,
) -> GatewayResult:
    """Run the four-step check for one gateway. Returns a typed result."""
    env = env if env is not None else dict(os.environ)
    if cfg.auth_env_var not in env or not env[cfg.auth_env_var]:
        return GatewayResult(
            cfg,
            status="skipped",
            detail=f"no credentials (set {cfg.auth_env_var} to enable)",
        )

    cassette_path = repo_root / cfg.cassette_path
    request_path = repo_root / cfg.request_fixture
    if not cassette_path.exists():
        return GatewayResult(
            cfg, status="skipped", detail=f"cassette missing: {cfg.cassette_path}"
        )
    if not request_path.exists():
        return GatewayResult(
            cfg,
            status="skipped",
            detail=f"request fixture missing: {cfg.request_fixture}",
        )

    try:
        cassette = json.loads(cassette_path.read_text(encoding="utf-8"))
        request = json.loads(request_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return GatewayResult(
            cfg, status="error", detail=f"fixture not valid JSON: {exc}"
        )

    fetch = fetcher or _http_fetch
    try:
        live = fetch(cfg.endpoint, request, env[cfg.auth_env_var])
    except Exception as exc:  # noqa: BLE001 — network has many surprises
        return GatewayResult(cfg, status="error", detail=f"live fetch failed: {exc}")

    expected = _normalize_response(cassette)
    actual = _normalize_response(live)
    drift = _diff(expected, actual, prefix="")
    if drift:
        return GatewayResult(
            cfg,
            status="drift",
            detail=f"{len(drift)} drift point(s)",
            drift_paths=drift,
        )
    return GatewayResult(cfg, status="ok", detail="no drift")


def _http_fetch(endpoint: str, request: dict[str, Any], token: str) -> dict[str, Any]:
    method = request.get("method", "GET").upper()
    path = request.get("path", "/")
    body = request.get("body")
    headers = dict(request.get("headers", {}))
    headers.setdefault("Authorization", f"Bearer {token}")
    url = endpoint.rstrip("/") + path
    data = body.encode("utf-8") if isinstance(body, str) else (
        json.dumps(body).encode("utf-8") if body is not None else None
    )
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:  # noqa: S310 — sandbox URLs are static, listed in config.toml
            raw_body = resp.read().decode("utf-8", errors="replace")
            response_headers = {k.lower(): v for k, v in resp.headers.items()}
            return {
                "status": resp.status,
                "headers": response_headers,
                "body": raw_body,
            }
    except urllib.error.HTTPError as exc:
        body_bytes = exc.read() if hasattr(exc, "read") else b""
        return {
            "status": exc.code,
            "headers": {k.lower(): v for k, v in exc.headers.items()},
            "body": body_bytes.decode("utf-8", errors="replace"),
        }


def _normalize_response(payload: dict[str, Any]) -> dict[str, Any]:
    """Strip per-request noise so two captures of the same call compare equal.

    Headers like ``date``, ``request-id``, ``x-trace-id`` are
    expected to differ on every call and would otherwise look like
    drift. JSON bodies are decoded so cosmetic whitespace doesn't
    matter; non-JSON bodies are compared as-is.
    """
    ignored_headers = {
        "date",
        "request-id",
        "x-request-id",
        "x-trace-id",
        "x-correlation-id",
        "x-amzn-requestid",
        "etag",
        "set-cookie",
    }
    out = {"status": payload.get("status")}
    headers = {
        k.lower(): v
        for k, v in payload.get("headers", {}).items()
        if k.lower() not in ignored_headers
    }
    out["headers_keys"] = sorted(headers.keys())
    body = payload.get("body", "")
    try:
        decoded = json.loads(body)
        out["body"] = _strip_jitter(decoded)
    except (json.JSONDecodeError, TypeError):
        out["body"] = body
    return out


def _strip_jitter(value: Any) -> Any:
    """Recursively drop fields known to vary per-call (timestamps, request ids)."""
    jitter_keys = {"timestamp", "requestId", "request_id", "trace_id", "correlationId"}
    if isinstance(value, dict):
        return {
            k: _strip_jitter(v)
            for k, v in value.items()
            if k not in jitter_keys
        }
    if isinstance(value, list):
        return [_strip_jitter(v) for v in value]
    return value


def _diff(expected: Any, actual: Any, *, prefix: str) -> list[str]:
    if type(expected) is not type(actual):
        return [f"{prefix}: type {type(expected).__name__} → {type(actual).__name__}"]
    if isinstance(expected, dict):
        out: list[str] = []
        for k in sorted(set(expected.keys()) | set(actual.keys())):
            child = f"{prefix}.{k}" if prefix else k
            if k not in expected:
                out.append(f"{child}: appeared (only in live)")
            elif k not in actual:
                out.append(f"{child}: disappeared (only in cassette)")
            else:
                out.extend(_diff(expected[k], actual[k], prefix=child))
        return out
    if isinstance(expected, list):
        if len(expected) != len(actual):
            return [f"{prefix}: list length {len(expected)} → {len(actual)}"]
        out = []
        for i, (e, a) in enumerate(zip(expected, actual)):
            out.extend(_diff(e, a, prefix=f"{prefix}[{i}]"))
        return out
    if expected != actual:
        return [f"{prefix}: {expected!r} → {actual!r}"]
    return []


def render_report(results: list[GatewayResult]) -> str:
    """Operator-friendly multi-line summary."""
    lines = [f"# T-074c sandbox drift canary — {BEAD_ID}", ""]
    by_status: dict[str, list[GatewayResult]] = {}
    for r in results:
        by_status.setdefault(r.status, []).append(r)
    for status in ("drift", "error", "ok", "skipped"):
        bucket = by_status.get(status, [])
        if not bucket:
            continue
        lines.append(f"## {status.upper()} ({len(bucket)})")
        for r in bucket:
            lines.append(f"- {r.to_summary_line()}")
            for path in r.drift_paths[:10]:
                lines.append(f"    - {path}")
            if len(r.drift_paths) > 10:
                lines.append(f"    - … {len(r.drift_paths) - 10} more")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def open_drift_issue(report: str, drift: list[GatewayResult], repo: str | None) -> None:
    """Open one rolled-up issue per nightly run that found drift."""
    title = f"[sandbox-drift] {len(drift)} gateway(s) drifted on nightly canary"
    cmd = ["gh", "issue", "create", "--title", title, "--body", report]
    for label in DEFAULT_LABELS:
        cmd.extend(["--label", label])
    if repo:
        cmd.extend(["--repo", repo])
    try:
        subprocess.run(cmd, check=True, capture_output=True, text=True)
    except FileNotFoundError:
        sys.stderr.write(
            "warning: gh CLI not installed; drift report not posted as issue\n"
        )
    except subprocess.CalledProcessError as exc:
        sys.stderr.write(
            f"warning: gh issue create failed (rc={exc.returncode}): {exc.stderr}\n"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--config",
        type=Path,
        default=Path("data/sandbox-drift/config.toml"),
        help="path to the sandbox drift config TOML",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path("."),
        help="repository root that cassette + fixture paths are resolved against",
    )
    parser.add_argument(
        "--report-mode",
        choices=("stdout", "github-issue"),
        default="stdout",
        help="how to surface a drift-containing run",
    )
    parser.add_argument(
        "--github-repo",
        default=os.environ.get("GITHUB_REPOSITORY"),
        help="owner/repo to file the issue against (defaults to $GITHUB_REPOSITORY)",
    )
    args = parser.parse_args(argv)

    gateways = load_config(args.config)
    results = [check_gateway(g, args.repo_root) for g in gateways]
    report = render_report(results)
    sys.stdout.write(report)

    drifted = [r for r in results if r.status == "drift"]
    if drifted and args.report_mode == "github-issue":
        open_drift_issue(report, drifted, args.github_repo)
    # Always exit 0 — see module docstring "design rule".
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
