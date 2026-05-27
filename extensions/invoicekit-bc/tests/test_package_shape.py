# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""Static conformance checks for the Business Central connector package."""

from __future__ import annotations

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(condition: bool, message: object) -> None:
    if not condition:
        raise AssertionError(message)


def load_manifest() -> dict[str, object]:
    try:
        decoder = json.JSONDecoder()
        return decoder.decode((ROOT / "app.json").read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise AssertionError(f"app.json is not valid JSON: {exc}") from exc


def test_app_manifest_targets_business_central_16_and_17() -> None:
    manifest = load_manifest()

    require(manifest["publisher"] == "The InvoiceKit Authors", "publisher mismatch")
    require(str(manifest["application"]).startswith("16."), "minimum application must be BC 16")
    require(str(manifest["platform"]).startswith("16."), "minimum platform must be BC 16")
    require(manifest["runtime"] == "5.0", "runtime must remain BC 16 compatible")
    require({"from": 71500, "to": 71549} in manifest["idRanges"], "id range missing")
    require("NoImplicitWith" in manifest["features"], "NoImplicitWith feature missing")


def test_required_al_objects_are_present() -> None:
    expected = {
        "src/InvoiceKitStatus.Enum.al",
        "src/InvoiceKitSetup.Table.al",
        "src/InvoiceKitSetup.Page.al",
        "src/SalesHeaderInvoiceKit.TableExt.al",
        "src/SalesInvoiceInvoiceKit.PageExt.al",
        "src/InvoiceKitSidecarClient.Codeunit.al",
        "src/InvoiceKitPermissions.PermissionSet.al",
        "test/InvoiceKitSidecar.Tests.Codeunit.al",
    }

    for relative in expected:
        require((ROOT / relative).is_file(), relative)


def test_sales_invoice_action_matches_runbook_contract() -> None:
    page_ext = read("src/SalesInvoiceInvoiceKit.PageExt.al")
    client = read("src/InvoiceKitSidecarClient.Codeunit.al")

    require('extends "Sales Invoice"' in page_ext, "Sales Invoice page extension missing")
    require("Send via InvoiceKit" in page_ext, "Send via InvoiceKit action missing")
    require("SidecarClient.SendSalesInvoice(Rec)" in page_ext, "page action is not wired to client")
    require(
        'HttpClient.Post(NormalizeBaseUrl(Setup."Sidecar URL") + \'/v1/transmit\'' in client,
        "sidecar client must post to /v1/transmit",
    )
    require("BuildPayload(SalesHeader" in client, "payload builder missing")
    require("InvoiceKit Submission ID" in client, "submission id receipt handling missing")
    require("InvoiceKit Evidence URL" in client, "evidence URL receipt handling missing")
    require("ContentHeaders.Remove('Content-Type')" in client, "content-type header should be replaced explicitly")
    require("TryReadRequiredText" in client, "required receipt field guard missing")
    require("RecordFailure(SalesHeader" in client, "failure receipt handling missing")
    require(
        "RecordFailure(SalesHeader, 'InvoiceKit sidecar was unreachable.');\n            exit;" in client,
        "unreachable sidecar path must persist failure and return",
    )


def test_setup_and_permissions_are_installable() -> None:
    setup_table = read("src/InvoiceKitSetup.Table.al")
    setup_page = read("src/InvoiceKitSetup.Page.al")
    permissions = read("src/InvoiceKitPermissions.PermissionSet.al")

    require("procedure EnsureExists()" in setup_table, "setup bootstrap procedure missing")
    require('"Sidecar URL"' in setup_table, "sidecar URL setting missing")
    require('"API Key"' in setup_table, "API key setting missing")
    require('page 71500 "InvoiceKit Setup"' in setup_page, "setup page missing")
    require('permissionset 71500 "INVOICEKIT CONNECTOR"' in permissions, "permission set missing")
    require('tabledata "InvoiceKit Setup" = RIMD' in permissions, "setup table permission missing")
    assert_codeunit = "Codeunit " + "Assert"
    require(assert_codeunit not in read("test/InvoiceKitSidecar.Tests.Codeunit.al"), "test app must not depend on extra Assert apps")


def test_connector_has_no_placeholder_markers() -> None:
    terms = [
        "TO" + "DO",
        "FIX" + "ME",
        "X" * 3,
        "REPLACE" + "_ME",
        "un" + "implemented",
        "not " + "implemented",
    ]
    forbidden = re.compile("|".join(re.escape(term) for term in terms), re.IGNORECASE)
    for path in ROOT.rglob("*"):
        if path.is_file() and path.suffix.lower() in {".al", ".json", ".md", ".py", ".yml"}:
            text = path.read_text(encoding="utf-8")
            require(forbidden.search(text) is None, path)


def test_ci_workflow_covers_static_and_bccontainerhelper_paths() -> None:
    workflow = (REPO / ".github/workflows/dynamics-connector.yml").read_text(encoding="utf-8")

    require("python3 -m pytest extensions/invoicekit-bc/tests -q" in workflow, "static test command missing")
    require("BcContainerHelper" in workflow, "bccontainerhelper install missing")
    require("-includeTestToolkit" in workflow, "BC test toolkit flag missing")
    require("bc_version: [16, 17]" in workflow, "BC 16/17 matrix missing")
    require("extensions/invoicekit-bc" in workflow, "extension path missing")
