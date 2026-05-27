#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""T-074d sandbox vs production parity diff.

For each `[[pair]]` stanza in ``data/sandbox-prod-parity/config.toml``:

1. **Consent gate.** If ``consent_signed_at`` is empty, the canary
   records "skipped: no production-call consent on file" and exits
   for this pair WITHOUT touching the production endpoint. This is
   the bead's load-bearing safety gate — production endpoints (which
   may be regulator-side and rate-limited) only get touched when the
   customer's recorded consent is on file.
2. **Credentials gate.** Both ``sandbox_auth_env_var`` and
   ``production_auth_env_var`` must be set. Either missing surfaces
   as "skipped: missing credentials" and exits without any HTTP
   call. (We never fall back to sandbox-only — T-074c already
   covers that.)
3. **Replay both sides.** Send the same request fixture to both
   endpoints, normalize the responses to strip per-call jitter, and
   diff. Status / header keys / JSON body keys are compared
   recursively; any difference is a parity-drift event.
4. **Report.** Drift events open one rolled-up GitHub issue per
   nightly run via ``gh issue create``. The ``sandbox-prod-parity``
   label lets triage filter cleanly. Like T-074c, the workflow
   never exits non-zero — the issue is the page, not the workflow
   status.

The implementation reuses T-074c's response-normalization and
diff helpers so the two canaries report drift in the same shape.

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

# Reuse T-074c's normalization helpers via a sibling sys.path entry
# so we don't fork the jitter rules. Falling back to a local import
# keeps the script runnable from any cwd.
HERE = Path(__file__).resolve().parent
SANDBOX_DRIFT_DIR = HERE.parent / "sandbox-drift"
if SANDBOX_DRIFT_DIR.exists():
    sys.path.insert(0, str(SANDBOX_DRIFT_DIR))
try:
    from sandbox_drift import _normalize_response, _diff  # type: ignore
except ImportError:  # pragma: no cover — workflow always installs alongside T-074c
    def _normalize_response(payload):  # type: ignore
        return payload

    def _diff(a, b, *, prefix):  # type: ignore
        return [] if a == b else [f"{prefix}: differs"]


BEAD_ID = "invoices-t-074d-sandbox-prod-parity-diff-eze"
DEFAULT_LABELS = ["sandbox-prod-parity", "track-6", "automation"]


@dataclass
class PairConfig:
    """One `[[pair]]` stanza from config.toml."""

    country: str
    pair_id: str
    description: str
    sandbox_endpoint: str
    production_endpoint: str
    sandbox_auth_env_var: str
    production_auth_env_var: str
    request_fixture: Path
    consent_signed_at: str


@dataclass
class PairResult:
    pair: PairConfig
    status: str  # "ok" | "drift" | "skipped" | "error"
    detail: str
    drift_paths: list[str] = field(default_factory=list)

    def to_summary_line(self) -> str:
        bits = [f"[{self.status.upper()}]", self.pair.country, self.pair.pair_id]
        if self.detail:
            bits.extend(["—", self.detail])
        return " ".join(bits)


def load_config(path: Path) -> list[PairConfig]:
    raw = path.read_text(encoding="utf-8")
    data = tomllib.loads(raw)
    if data.get("schema_version") != "1.0":
        raise ValueError(
            f"unsupported schema_version {data.get('schema_version')!r} in {path}"
        )
    pairs = []
    for entry in data.get("pair", []):
        pairs.append(
            PairConfig(
                country=entry["country"],
                pair_id=entry["pair_id"],
                description=entry["description"],
                sandbox_endpoint=entry["sandbox_endpoint"],
                production_endpoint=entry["production_endpoint"],
                sandbox_auth_env_var=entry["sandbox_auth_env_var"],
                production_auth_env_var=entry["production_auth_env_var"],
                request_fixture=Path(entry["request_fixture"]),
                consent_signed_at=entry.get("consent_signed_at", ""),
            )
        )
    return pairs


def check_pair(
    cfg: PairConfig,
    repo_root: Path,
    *,
    env: dict[str, str] | None = None,
    fetcher=None,
) -> PairResult:
    env = env if env is not None else dict(os.environ)

    # 1. Consent gate.
    if not cfg.consent_signed_at:
        return PairResult(
            cfg,
            status="skipped",
            detail="no production-call consent on file (set consent_signed_at in config.toml)",
        )

    # 2. Credentials gate.
    if not env.get(cfg.sandbox_auth_env_var):
        return PairResult(
            cfg,
            status="skipped",
            detail=f"missing sandbox credentials ({cfg.sandbox_auth_env_var})",
        )
    if not env.get(cfg.production_auth_env_var):
        return PairResult(
            cfg,
            status="skipped",
            detail=f"missing production credentials ({cfg.production_auth_env_var})",
        )

    # 3. Replay both sides.
    request_path = repo_root / cfg.request_fixture
    if not request_path.exists():
        return PairResult(
            cfg, status="skipped", detail=f"request fixture missing: {cfg.request_fixture}"
        )
    try:
        request = json.loads(request_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return PairResult(cfg, status="error", detail=f"request fixture not JSON: {exc}")

    fetch = fetcher or _http_fetch
    try:
        sandbox_response = fetch(
            cfg.sandbox_endpoint, request, env[cfg.sandbox_auth_env_var]
        )
        production_response = fetch(
            cfg.production_endpoint, request, env[cfg.production_auth_env_var]
        )
    except Exception as exc:  # noqa: BLE001 — network has many surprises
        return PairResult(cfg, status="error", detail=f"live fetch failed: {exc}")

    normalized_sandbox = _normalize_response(sandbox_response)
    normalized_production = _normalize_response(production_response)
    drift = _diff(normalized_sandbox, normalized_production, prefix="")
    if drift:
        return PairResult(
            cfg, status="drift", detail=f"{len(drift)} drift point(s)", drift_paths=drift
        )
    return PairResult(cfg, status="ok", detail="sandbox + production agree")


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
        with urllib.request.urlopen(req, timeout=30) as resp:  # noqa: S310
            return {
                "status": resp.status,
                "headers": {k.lower(): v for k, v in resp.headers.items()},
                "body": resp.read().decode("utf-8", errors="replace"),
            }
    except urllib.error.HTTPError as exc:
        body_bytes = exc.read() if hasattr(exc, "read") else b""
        return {
            "status": exc.code,
            "headers": {k.lower(): v for k, v in exc.headers.items()},
            "body": body_bytes.decode("utf-8", errors="replace"),
        }


def render_report(results: list[PairResult]) -> str:
    lines = [f"# T-074d sandbox vs production parity — {BEAD_ID}", ""]
    by_status: dict[str, list[PairResult]] = {}
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


def open_drift_issue(report: str, drift: list[PairResult], repo: str | None) -> None:
    title = f"[sandbox-prod-parity] {len(drift)} pair(s) drifted between sandbox and production"
    cmd = ["gh", "issue", "create", "--title", title, "--body", report]
    for label in DEFAULT_LABELS:
        cmd.extend(["--label", label])
    if repo:
        cmd.extend(["--repo", repo])
    try:
        subprocess.run(cmd, check=True, capture_output=True, text=True)
    except FileNotFoundError:
        sys.stderr.write(
            "warning: gh CLI not installed; parity drift not posted as issue\n"
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
        default=Path("data/sandbox-prod-parity/config.toml"),
    )
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument(
        "--report-mode",
        choices=("stdout", "github-issue"),
        default="stdout",
    )
    parser.add_argument(
        "--github-repo",
        default=os.environ.get("GITHUB_REPOSITORY"),
    )
    args = parser.parse_args(argv)

    pairs = load_config(args.config)
    results = [check_pair(p, args.repo_root) for p in pairs]
    report = render_report(results)
    sys.stdout.write(report)

    drifted = [r for r in results if r.status == "drift"]
    if drifted and args.report_mode == "github-issue":
        open_drift_issue(report, drifted, args.github_repo)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
