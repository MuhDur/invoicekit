#!/usr/bin/env python3
"""InvoiceKit release-check meta-test.

This guard ensures the supply-chain CI surface stays intact across future
edits. It does NOT replace running the actual checks; it asserts that the
checks are still wired into the workflow files, so a refactor that
accidentally deletes the `cargo-audit` or `cargo-deny` job is caught
before merge.

Specifically the script verifies:

* ``.github/workflows/ci.yml`` contains both a job that runs ``cargo audit``
  and one that runs ``cargo deny``.
* ``.github/workflows/release.yml`` contains a step that runs
  ``cargo cyclonedx`` (the CycloneDX SBOM generator) and a step that runs
  ``cosign sign-blob`` (the keyless OIDC signer).
* ``.github/workflows/license-header.yml`` exists and runs
  ``tools/license-header/check_headers.py --check``.

Usage::

    python3 tools/release-checks/verify_release_checks.py

Exit codes
----------

* 0 — every required job/step is wired in.
* 1 — one or more required checks are missing.
* 2 — a workflow file is missing entirely.
"""

from __future__ import annotations

import argparse
import dataclasses
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"


@dataclasses.dataclass(frozen=True)
class Requirement:
    """One thing that must appear inside a named workflow file."""

    workflow: str
    must_contain: str
    label: str


REQUIREMENTS: tuple[Requirement, ...] = (
    Requirement("ci.yml", "cargo audit", "ci.yml runs cargo audit"),
    Requirement("ci.yml", "cargo deny", "ci.yml runs cargo deny"),
    Requirement("release.yml", "cargo cyclonedx", "release.yml emits a CycloneDX SBOM"),
    Requirement("release.yml", "cosign sign-blob", "release.yml signs artifacts with cosign"),
    Requirement(
        "license-header.yml",
        "tools/license-header/check_headers.py",
        "license-header.yml gates the SPDX header on every PR",
    ),
)


def check(workflows_dir: Path = WORKFLOWS) -> tuple[int, list[str]]:
    """Run every requirement; return (exit_code, list_of_messages)."""
    missing_files: list[str] = []
    missing_checks: list[str] = []
    for req in REQUIREMENTS:
        path = workflows_dir / req.workflow
        if not path.is_file():
            missing_files.append(f"workflow not found: {path}")
            continue
        body = path.read_text(encoding="utf-8")
        if req.must_contain not in body:
            missing_checks.append(f"{req.label}: missing `{req.must_contain}` in {path.name}")

    messages: list[str] = []
    messages.extend(missing_files)
    messages.extend(missing_checks)
    if missing_files:
        return 2, messages
    if missing_checks:
        return 1, messages
    return 0, messages


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--workflows-dir", type=Path, default=WORKFLOWS)
    args = parser.parse_args(argv)
    exit_code, messages = check(args.workflows_dir.resolve())
    if exit_code == 0:
        print("release-checks: every required CI step is wired in")
    else:
        for line in messages:
            print(f"release-checks: {line}")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
