#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Differential parity harness for the Rust EN 16931 validator.

The harness compares the pure-Rust rule IDs emitted by
``invoicekit-en16931-findings`` with live JVM validator sidecar responses over
the committed XML conformance corpus. It only compares EN 16931 core
``BR-*``/``BR-CO-*`` identifiers and fails closed when a sidecar reports a
configuration or library error.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[2]
DEFAULT_FIXTURE_GLOB = "conformance-corpus/synthetic/ubl-2-1/*/fixture.xml"
BR_RULE_RE = re.compile(r"^BR(?:-[A-Z]{2,})?-\d+[A-Z]?$")
UBL_NS = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"
CBC_NS = "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"
CAC_NS = "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"
EXT_NS = "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2"
PEPPOL_CUSTOMIZATION_ID = (
    "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0"
)
PEPPOL_PROFILE_ID = "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0"
XRECHNUNG_CUSTOMIZATION_ID = (
    "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0"
)
XRECHNUNG_PROFILE_ID = "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0"
ORACLE_UNAVAILABLE_MARKERS = (
    "CONFIGURATION-MISSING",
    "LIBRARY-ERROR",
    "NO-MATCHING-SET",
)
ORACLE_PRECONDITION_RULE_IDS = {"PHIVE-UNNAMED", "KOSIT-XML-WELLFORMED"}


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    repo = args.repo_root.resolve()
    fixtures = fixture_paths(repo, args.fixture_glob, args.limit)
    if not fixtures:
        print(json.dumps({"status": "configuration_error", "message": "no XML fixtures found"}))
        return 2

    sidecars = configured_sidecars(args)
    if not sidecars:
        print(
            json.dumps(
                {
                    "status": "configuration_error",
                    "message": "provide --kosit-url and/or --phive-url, or set the matching env vars",
                },
                sort_keys=True,
            )
        )
        return 2

    summary = {
        "status": "pass",
        "fixture_count": len(fixtures),
        "min_parity": args.min_parity,
        "backends": {},
    }
    exit_code = 0

    for sidecar in sidecars:
        backend_summary = compare_backend(
            repo,
            sidecar,
            fixtures,
            args.rust_probe,
            args.rust_timeout,
            None if args.no_ubl_normalize else args.ubl_normalizer,
            args.normalizer_timeout,
            args.timeout,
        )
        summary["backends"][sidecar["backend"]] = backend_summary
        if backend_summary["status"] == "configuration_error":
            summary["status"] = "configuration_error"
            exit_code = max(exit_code, 2)
        elif backend_summary["parity"] < args.min_parity:
            summary["status"] = "fail"
            exit_code = max(exit_code, 1)

    print(json.dumps(summary, indent=2, sort_keys=True))
    return exit_code


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Compare Rust EN 16931 findings to JVM sidecars.")
    parser.add_argument("--repo-root", type=Path, default=REPO)
    parser.add_argument(
        "--fixture-glob",
        action="append",
        default=[],
        help=f"Repo-relative XML fixture glob. Default: {DEFAULT_FIXTURE_GLOB}",
    )
    parser.add_argument("--limit", type=int, default=0, help="Deterministic fixture limit.")
    parser.add_argument("--min-parity", type=float, default=0.999)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--rust-timeout", type=float, default=120.0)
    parser.add_argument("--kosit-url", default=os.environ.get("INVOICEKIT_VALIDATOR_KOSIT_URL"))
    parser.add_argument("--phive-url", default=os.environ.get("INVOICEKIT_VALIDATOR_PHIVE_URL"))
    parser.add_argument(
        "--rust-probe",
        nargs="+",
        default=[
            "cargo",
            "run",
            "-q",
            "-p",
            "invoicekit-validate-ubl-cii",
            "--bin",
            "invoicekit-en16931-findings",
            "--",
        ],
        help="Command prefix for the Rust findings probe; fixture paths are appended.",
    )
    parser.add_argument(
        "--ubl-normalizer",
        nargs="+",
        default=[
            "cargo",
            "run",
            "-q",
            "-p",
            "invoicekit-format-ubl",
            "--bin",
            "invoicekit-ubl-normalize",
            "--",
        ],
        help=(
            "Command prefix for normalizing projected UBL XML through InvoiceKit; "
            "--stdin <label> is appended."
        ),
    )
    parser.add_argument(
        "--no-ubl-normalize",
        action="store_true",
        help="Send profile-projected fixture XML directly to sidecars without Rust normalization.",
    )
    parser.add_argument("--normalizer-timeout", type=float, default=120.0)
    return parser.parse_args(argv)


def fixture_paths(repo: Path, globs: list[str], limit: int) -> list[Path]:
    patterns = globs or [DEFAULT_FIXTURE_GLOB]
    paths: list[Path] = []
    for pattern in patterns:
        paths.extend(repo.glob(pattern))
    unique = sorted({path.resolve() for path in paths if path.is_file()})
    if limit > 0:
        return unique[:limit]
    return unique


def configured_sidecars(args: argparse.Namespace) -> list[dict[str, str]]:
    sidecars: list[dict[str, str]] = []
    if args.kosit_url:
        sidecars.append(
            {
                "backend": "jvm:kosit",
                "profile": "xrechnung",
                "url": args.kosit_url,
                "projection": "xrechnung",
            }
        )
    if args.phive_url:
        sidecars.append(
            {
                "backend": "jvm:phive",
                "profile": "peppol-bis",
                "url": args.phive_url,
                "projection": "peppol-bis",
            }
        )
    return sidecars


def run_rust_probe(
    repo: Path, command: list[str], fixtures: list[Path], timeout: float = 120.0
) -> dict[str, set[str]]:
    completed = subprocess.run(
        command + [str(path) for path in fixtures],
        cwd=repo,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise SystemExit(f"rust findings probe failed with exit {completed.returncode}: {detail}")
    try:
        reports = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"rust findings probe did not emit JSON: {error}") from error

    results: dict[str, set[str]] = {}
    for report in reports:
        path = str(Path(report["path"]).resolve())
        if report.get("error"):
            raise SystemExit(f"rust findings probe failed for {path}: {report['error']}")
        results[path] = core_rule_ids(report.get("findings", []))
    return results


def run_rust_probe_xml(
    repo: Path, command: list[str], label: str, xml: str, timeout: float
) -> set[str]:
    completed = subprocess.run(
        command + ["--stdin", label],
        cwd=repo,
        text=True,
        input=xml,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise SystemExit(f"rust findings probe failed with exit {completed.returncode}: {detail}")
    try:
        reports = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"rust findings probe did not emit JSON: {error}") from error
    if len(reports) != 1:
        raise SystemExit(f"rust findings probe expected one report for {label}, got {len(reports)}")
    report = reports[0]
    if report.get("error"):
        raise SystemExit(f"rust findings probe failed for {label}: {report['error']}")
    return core_rule_ids(report.get("findings", []))


def normalize_ubl_xml(
    repo: Path, command: list[str], label: str, xml: str, timeout: float
) -> str:
    completed = subprocess.run(
        command + ["--stdin", label],
        cwd=repo,
        text=True,
        input=xml,
        capture_output=True,
        check=False,
        timeout=timeout,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise SystemExit(
            f"UBL normalizer failed for {label} with exit {completed.returncode}: {detail}"
        )
    if not completed.stdout.strip():
        raise SystemExit(f"UBL normalizer emitted empty XML for {label}")
    return completed.stdout


def compare_backend(
    repo: Path,
    sidecar: dict[str, str],
    fixtures: list[Path],
    rust_probe: list[str],
    rust_timeout: float,
    ubl_normalizer: list[str] | None,
    normalizer_timeout: float,
    timeout: float,
) -> dict[str, object]:
    mismatches: list[dict[str, object]] = []
    compared = 0
    unavailable: dict[str, object] | None = None
    precondition_failed: dict[str, object] | None = None

    for fixture in fixtures:
        label = fixture_id(repo, fixture)
        projected_xml = project_xml(fixture.read_text(encoding="utf-8"), sidecar["projection"])
        xml = (
            normalize_ubl_xml(repo, ubl_normalizer, label, projected_xml, normalizer_timeout)
            if ubl_normalizer is not None
            else projected_xml
        )
        response = rpc_validate(sidecar, xml, label, timeout)
        results = response.get("result", {}).get("results", [])
        unavailable_finding = oracle_unavailable(results)
        if unavailable_finding is not None:
            unavailable = {
                "fixture": fixture_id(repo, fixture),
                "rule_id": unavailable_finding.get("rule_id"),
                "message": unavailable_finding.get("message"),
            }
            break
        precondition_finding = oracle_precondition_failure(results)
        if precondition_finding is not None:
            precondition_failed = {
                "fixture": label,
                "rule_id": precondition_finding.get("rule_id"),
                "message": precondition_finding.get("message"),
            }
            break

        rust_rule_ids = run_rust_probe_xml(repo, rust_probe, label, xml, rust_timeout)
        oracle_rule_ids = core_rule_ids(results)
        compared += 1
        if rust_rule_ids != oracle_rule_ids:
            mismatches.append(
                {
                    "fixture": fixture_id(repo, fixture),
                    "rust_only": sorted(rust_rule_ids - oracle_rule_ids),
                    "oracle_only": sorted(oracle_rule_ids - rust_rule_ids),
                }
            )

    if unavailable is not None:
        return {
            "status": "configuration_error",
            "compared": compared,
            "parity": 0.0,
            "unavailable": unavailable,
            "mismatch_count": len(mismatches),
            "mismatches": mismatches[:20],
        }

    if precondition_failed is not None:
        return {
            "status": "configuration_error",
            "compared": compared,
            "parity": 0.0,
            "precondition_failed": precondition_failed,
            "mismatch_count": len(mismatches),
            "mismatches": mismatches[:20],
        }

    parity = 1.0 if compared == 0 else (compared - len(mismatches)) / compared
    return {
        "status": "pass" if not mismatches else "fail",
        "compared": compared,
        "parity": parity,
        "mismatch_count": len(mismatches),
        "mismatches": mismatches[:20],
    }


def project_xml(xml: str, projection: str) -> str:
    if projection == "peppol-bis":
        return set_ubl_profile(xml, PEPPOL_CUSTOMIZATION_ID, PEPPOL_PROFILE_ID)
    if projection == "xrechnung":
        return set_ubl_profile(xml, XRECHNUNG_CUSTOMIZATION_ID, XRECHNUNG_PROFILE_ID)
    if projection == "none":
        return xml
    raise ValueError(f"unsupported projection {projection!r}")


def set_ubl_profile(xml: str, customization_id: str, profile_id: str) -> str:
    ET.register_namespace("", UBL_NS)
    ET.register_namespace("cbc", CBC_NS)
    ET.register_namespace("cac", CAC_NS)
    ET.register_namespace("ext", EXT_NS)
    root = ET.fromstring(xml)
    set_or_insert_child_text(root, f"{{{CBC_NS}}}CustomizationID", customization_id, 0)
    set_or_insert_child_text(root, f"{{{CBC_NS}}}ProfileID", profile_id, 1)
    set_missing_endpoint_scheme_ids(root)
    return ET.tostring(root, encoding="unicode")


def set_or_insert_child_text(root: ET.Element, tag: str, text: str, index: int) -> None:
    child = root.find(tag)
    if child is None:
        child = ET.Element(tag)
        root.insert(index, child)
    child.text = text


def set_missing_endpoint_scheme_ids(root: ET.Element) -> None:
    for endpoint in root.findall(f".//{{{CBC_NS}}}EndpointID"):
        if not (endpoint.get("schemeID") or "").strip():
            endpoint.set("schemeID", "0204")


def rpc_validate(sidecar: dict[str, str], xml: str, request_id: str, timeout: float) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "validator.validate",
        "params": {
            "backend": sidecar["backend"],
            "profile": sidecar["profile"],
            "trace_id": f"parity-{request_id}",
            "rule_pack": {
                "id": "en16931-parity",
                "version": "2026.05",
                "effective_date": "2026-05-28",
            },
            "document": {
                "content_type": "application/xml",
                "encoding": "utf-8",
                "xml": xml,
            },
        },
    }
    request = urllib.request.Request(
        sidecar["url"].rstrip("/") + "/rpc",
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except (OSError, urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        raise SystemExit(f"{sidecar['backend']} request failed at {sidecar['url']}: {error}") from error


def core_rule_ids(findings: list[dict]) -> set[str]:
    return {
        rule_id
        for finding in findings
        if isinstance((rule_id := finding.get("rule_id")), str) and BR_RULE_RE.fullmatch(rule_id)
    }


def oracle_unavailable(findings: list[dict]) -> dict | None:
    for finding in findings:
        rule_id = str(finding.get("rule_id", ""))
        if any(marker in rule_id for marker in ORACLE_UNAVAILABLE_MARKERS):
            return finding
    return None


def oracle_precondition_failure(findings: list[dict]) -> dict | None:
    for finding in findings:
        rule_id = str(finding.get("rule_id", ""))
        message = str(finding.get("message", ""))
        if rule_id in ORACLE_PRECONDITION_RULE_IDS:
            return finding
        if "[SAX]" in message or "schema" in message.lower() or "well-formed" in message.lower():
            return finding
    return None


def fixture_id(repo: Path, fixture: Path) -> str:
    try:
        return fixture.resolve().relative_to(repo.resolve()).as_posix()
    except ValueError:
        return fixture.as_posix()


if __name__ == "__main__":
    sys.exit(main())
