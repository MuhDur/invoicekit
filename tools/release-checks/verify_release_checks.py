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
* ``.github/workflows/ci.yml`` contains a cassette PII scan that runs the
  ``cassette_corpus_has_no_unscrubbed_pii`` guard.
* ``.github/workflows/release.yml`` contains a step that runs
  ``cargo cyclonedx`` (the CycloneDX SBOM generator) and a step that runs
  ``cosign sign-blob`` (the keyless OIDC signer).
* ``.github/workflows/license-header.yml`` exists and runs
  ``tools/license-header/check_headers.py --check``.
* The T-050 Typst advisory waivers are present in both ``deny.toml`` and
  ``.cargo/audit.toml``, and their ignored crates appear only through
  ``invoicekit-render-pdf``'s documented Typst dependency path.

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
from collections.abc import Callable
import dataclasses
from pathlib import Path
import re
import subprocess

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
DENY_TOML = REPO_ROOT / "deny.toml"
AUDIT_TOML = REPO_ROOT / ".cargo" / "audit.toml"
DEPTH_TREE_LINE_RE = re.compile(r"^(?P<depth>\d+)(?P<package>[A-Za-z0-9_.-]+)\s+v[0-9]")


@dataclasses.dataclass(frozen=True)
class Requirement:
    """One thing that must appear inside a named workflow file."""

    workflow: str
    must_contain: str
    label: str


@dataclasses.dataclass(frozen=True)
class AdvisoryWaiver:
    """One deliberately waived RustSec advisory and its allowed crate path."""

    advisory: str
    crate: str
    required_crates: tuple[str, ...]
    allowed_workspace_crates: tuple[str, ...]


REQUIREMENTS: tuple[Requirement, ...] = (
    Requirement("ci.yml", "cargo audit", "ci.yml runs cargo audit"),
    Requirement("ci.yml", "cargo deny", "ci.yml runs cargo deny"),
    Requirement(
        "ci.yml",
        "cassette_corpus_has_no_unscrubbed_pii",
        "ci.yml runs the cassette PII scan",
    ),
    Requirement("release.yml", "cargo cyclonedx", "release.yml emits a CycloneDX SBOM"),
    Requirement("release.yml", "cosign sign-blob", "release.yml signs artifacts with cosign"),
    Requirement(
        "license-header.yml",
        "tools/license-header/check_headers.py",
        "license-header.yml gates the SPDX header on every PR",
    ),
)

ADVISORY_WAIVERS: tuple[AdvisoryWaiver, ...] = (
    AdvisoryWaiver(
        "RUSTSEC-2024-0320",
        "yaml-rust",
        ("syntect", "typst-library"),
        (
            "invoicekit-render-pdf",
            "invoicekit-render-factur-x-acceptance",
            "invoicekit-binding-rest-shim",
        ),
    ),
    AdvisoryWaiver(
        "RUSTSEC-2024-0436",
        "paste",
        ("hayagriva", "typst-library"),
        (
            "invoicekit-render-pdf",
            "invoicekit-render-factur-x-acceptance",
            "invoicekit-binding-rest-shim",
        ),
    ),
    AdvisoryWaiver(
        "RUSTSEC-2025-0141",
        "bincode",
        ("syntect", "typst-library"),
        (
            "invoicekit-render-pdf",
            "invoicekit-render-factur-x-acceptance",
            "invoicekit-binding-rest-shim",
        ),
    ),
)

CargoTreeRunner = Callable[[Path, str], tuple[int, str, str]]


def run_cargo_tree(repo_root: Path, crate: str) -> tuple[int, str, str]:
    """Return inverse cargo tree output for one crate."""
    try:
        completed = subprocess.run(
            ["cargo", "tree", "--locked", "--workspace", "-i", crate, "--prefix", "depth"],
            cwd=repo_root,
            text=True,
            capture_output=True,
            check=False,
            timeout=120,
        )
    except subprocess.TimeoutExpired as error:
        return 124, str(error.stdout or ""), str(error.stderr or "cargo tree timed out")
    return completed.returncode, completed.stdout, completed.stderr


def workspace_dependency_paths(cargo_tree_stdout: str) -> list[tuple[str, ...]]:
    """Return inverse-tree paths that terminate in an InvoiceKit workspace crate."""
    stack: list[str] = []
    paths: list[tuple[str, ...]] = []
    for line in cargo_tree_stdout.splitlines():
        match = DEPTH_TREE_LINE_RE.match(line)
        if match is None:
            continue

        depth = int(match.group("depth"))
        package = match.group("package")
        stack = stack[:depth]
        stack.append(package)

        if package.startswith("invoicekit-"):
            paths.append(tuple(stack))

    return paths


def check_advisory_waiver_scope(
    repo_root: Path = REPO_ROOT,
    cargo_tree_runner: CargoTreeRunner = run_cargo_tree,
) -> list[str]:
    """Return messages for Typst advisory waivers that drift out of scope."""
    messages: list[str] = []
    config_files = (DENY_TOML, AUDIT_TOML)
    for config_file in config_files:
        if not config_file.is_file():
            messages.append(f"advisory waiver config not found: {config_file}")
            continue
        body = config_file.read_text(encoding="utf-8")
        for waiver in ADVISORY_WAIVERS:
            if waiver.advisory not in body:
                messages.append(f"{config_file}: missing advisory waiver `{waiver.advisory}`")

    for waiver in ADVISORY_WAIVERS:
        return_code, stdout, stderr = cargo_tree_runner(repo_root, waiver.crate)
        if return_code != 0:
            detail = stderr.strip() or stdout.strip() or "cargo tree returned no detail"
            messages.append(f"cargo tree failed for `{waiver.crate}`: {detail}")
            continue

        workspace_paths = workspace_dependency_paths(stdout)
        workspace_crates = {path[-1] for path in workspace_paths}
        allowed = set(waiver.allowed_workspace_crates)
        unexpected = sorted(workspace_crates - allowed)
        if unexpected:
            messages.append(
                f"{waiver.advisory}/{waiver.crate} appears outside allowed workspace crates: "
                f"{', '.join(unexpected)}"
            )
        if not workspace_crates.intersection(allowed):
            messages.append(
                f"{waiver.advisory}/{waiver.crate} does not reach an allowed workspace crate: "
                f"{', '.join(waiver.allowed_workspace_crates)}"
            )

        for path in workspace_paths:
            if path[-1] not in allowed:
                continue
            missing_required = [
                required_crate
                for required_crate in waiver.required_crates
                if required_crate not in path
            ]
            if missing_required:
                messages.append(
                    f"{waiver.advisory}/{waiver.crate} reaches {path[-1]} without required "
                    f"Typst-path crates {', '.join(missing_required)}: {' -> '.join(path)}"
                )

    return messages


def check(
    workflows_dir: Path = WORKFLOWS,
    *,
    check_advisory_scope: bool = True,
    cargo_tree_runner: CargoTreeRunner = run_cargo_tree,
) -> tuple[int, list[str]]:
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
    if check_advisory_scope:
        messages.extend(check_advisory_waiver_scope(REPO_ROOT, cargo_tree_runner))
    if missing_files:
        return 2, messages
    if missing_checks or any("advisory waiver" in line or "RUSTSEC-" in line or "cargo tree" in line for line in messages):
        return 1, messages
    return 0, messages


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--workflows-dir", type=Path, default=WORKFLOWS)
    parser.add_argument(
        "--skip-advisory-waiver-scope-check",
        action="store_true",
        help="only verify workflow wiring; intended for focused unit tests",
    )
    args = parser.parse_args(argv)
    exit_code, messages = check(
        args.workflows_dir.resolve(),
        check_advisory_scope=not args.skip_advisory_waiver_scope_check,
    )
    if exit_code == 0:
        print("release-checks: every required CI step is wired in")
    else:
        for line in messages:
            print(f"release-checks: {line}")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
