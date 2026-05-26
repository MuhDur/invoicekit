"""Unit tests for the release-checks meta guard."""

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
TOOL = REPO / "tools" / "release-checks"
sys.path.insert(0, str(TOOL))
import verify_release_checks as guard  # noqa: E402


def _write_workflow(dir_: Path, name: str, body: str) -> None:
    target = dir_ / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(body, encoding="utf-8")


def _seed_full(tmp_path: Path) -> Path:
    workflows = tmp_path / ".github" / "workflows"
    _write_workflow(
        workflows,
        "ci.yml",
        "jobs:\n  audit:\n    steps: [{run: 'cargo audit'}]\n"
        "  deny:\n    steps: [{run: 'cargo deny check'}]\n",
    )
    _write_workflow(
        workflows,
        "release.yml",
        "jobs:\n  release:\n    steps:\n      - run: cargo cyclonedx --format json\n"
        "      - run: cosign sign-blob --yes path/to/binary\n",
    )
    _write_workflow(
        workflows,
        "license-header.yml",
        "jobs:\n  check:\n    steps:\n      - run: python3 tools/license-header/check_headers.py --check\n",
    )
    return workflows


def test_full_wiring_passes(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 0


def test_missing_cargo_audit_fails(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    ci = (workflows / "ci.yml").read_text()
    (workflows / "ci.yml").write_text(ci.replace("cargo audit", "cargo nope"))
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 1


def test_missing_cargo_deny_fails(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    ci = (workflows / "ci.yml").read_text()
    (workflows / "ci.yml").write_text(ci.replace("cargo deny check", "cargo other"))
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 1


def test_missing_sbom_step_fails(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    release = (workflows / "release.yml").read_text()
    (workflows / "release.yml").write_text(release.replace("cargo cyclonedx", "cargo nothing"))
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 1


def test_missing_cosign_step_fails(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    release = (workflows / "release.yml").read_text()
    (workflows / "release.yml").write_text(release.replace("cosign sign-blob", "cosign verify"))
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 1


def test_missing_workflow_file_returns_two(tmp_path: Path) -> None:
    workflows = _seed_full(tmp_path)
    (workflows / "license-header.yml").unlink()
    exit_code = guard.main(
        ["--workflows-dir", str(workflows), "--skip-advisory-waiver-scope-check"]
    )
    assert exit_code == 2


def test_typst_advisory_scope_passes_for_render_pdf_path() -> None:
    def fake_cargo_tree(_repo_root: Path, crate: str) -> tuple[int, str, str]:
        if crate == "paste":
            return (
                0,
                "paste v1.0.15\n"
                "└── hayagriva v0.8.1\n"
                "    └── typst-library v0.13.1\n"
                "        └── invoicekit-render-pdf v0.0.0 (/repo/crates/render-pdf)\n",
                "",
            )
        return (
            0,
            f"{crate} v0.0.0\n"
            "└── syntect v5.3.0\n"
            "    └── typst-library v0.13.1\n"
            "        └── invoicekit-render-pdf v0.0.0 (/repo/crates/render-pdf)\n",
            "",
        )

    messages = guard.check_advisory_waiver_scope(cargo_tree_runner=fake_cargo_tree)

    if messages:
        raise AssertionError(f"expected no waiver-scope messages, got {messages}")


def test_typst_advisory_scope_fails_for_other_workspace_crate() -> None:
    def fake_cargo_tree(_repo_root: Path, crate: str) -> tuple[int, str, str]:
        return (
            0,
            f"{crate} v0.0.0\n"
            "└── syntect v5.3.0\n"
            "    ├── typst-library v0.13.1\n"
            "    │   └── invoicekit-render-pdf v0.0.0 (/repo/crates/render-pdf)\n"
            "    └── invoicekit-engine v0.0.0 (/repo/crates/invoicekit-engine)\n",
            "",
        )

    messages = guard.check_advisory_waiver_scope(cargo_tree_runner=fake_cargo_tree)

    if not any("invoicekit-engine" in message for message in messages):
        raise AssertionError(f"expected invoicekit-engine scope failure, got {messages}")
