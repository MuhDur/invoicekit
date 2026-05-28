# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""Static conformance checks for the SAP Business One connector package."""

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
        return decoder.decode((ROOT / "manifest.json").read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise AssertionError(f"manifest.json is not valid JSON: {exc}") from exc


def test_manifest_describes_sap_b1_addon_shape() -> None:
    manifest = load_manifest()
    sap = manifest["sapBusinessOne"]
    sidecar = manifest["sidecar"]
    menu = manifest["menu"]

    require(manifest["publisher"] == "The InvoiceKit Authors", "publisher mismatch")
    require(manifest["license"] == "Apache-2.0", "license mismatch")
    require(sap["minimumVersion"] == "10.0 FP2410", "SAP B1 minimum version mismatch")
    require(sap["registrationInput"] == "package/addon-registration.input.xml", "registration input mismatch")
    require(sidecar["transmitEndpoint"] == "/v1/transmit", "sidecar endpoint mismatch")
    require(menu["uid"] == "INVOICEKIT_SEND", "menu uid mismatch")


def test_required_files_are_present() -> None:
    expected = {
        "manifest.json",
        "CONFORMANCE.md",
        "package/addon-registration.input.xml",
        "src/InvoiceKit.B1Addon.csproj",
        "src/Program.cs",
        "src/InvoiceKitApplication.cs",
        "src/InvoiceKitSettings.cs",
        "src/InvoiceKitSidecarClient.cs",
        "src/SapInvoiceSnapshot.cs",
    }

    for relative in expected:
        require((ROOT / relative).is_file(), relative)


def test_registration_input_is_parseable_and_matches_manifest() -> None:
    manifest = load_manifest()
    registration = read("package/addon-registration.input.xml")

    def value(tag: str) -> str:
        match = re.search(rf"<{tag}>([^<]+)</{tag}>", registration)
        require(match is not None, f"{tag} missing from registration input")
        return match.group(1)

    values = {
        "AddOnId": value("AddOnId"),
        "EntryPoint": value("EntryPoint"),
        "MenuUid": value("MenuUid"),
        "SidecarEndpoint": value("SidecarEndpoint"),
    }
    require("<InvoiceKitAddOnRegistration>" in registration, "registration input root mismatch")
    require(values["AddOnId"] == manifest["id"], "add-on id mismatch")
    require(values["EntryPoint"] == manifest["entryPoint"], "entry point mismatch")
    require(values["MenuUid"] == manifest["menu"]["uid"], "menu uid mismatch")
    require(values["SidecarEndpoint"] == manifest["sidecar"]["transmitEndpoint"], "sidecar endpoint mismatch")


def test_csharp_surface_matches_connector_contract() -> None:
    project = read("src/InvoiceKit.B1Addon.csproj")
    app = read("src/InvoiceKitApplication.cs")
    client = read("src/InvoiceKitSidecarClient.cs")
    snapshot = read("src/SapInvoiceSnapshot.cs")

    require("<TargetFramework>net48</TargetFramework>" in project, "SAP B1 add-on must target .NET Framework")
    require("<UseWindowsForms>true</UseWindowsForms>" in project, "SAP B1 add-on needs a Windows message loop")
    require("SAPbouiCOM" in project and "SAPbobsCOM" in project, "SAP SDK references missing")
    require("INVOICEKIT_SEND" in app, "InvoiceKit menu command missing")
    require("Send via InvoiceKit" in app, "menu caption missing")
    require("System.Windows.Forms.Application.Run()" in read("src/Program.cs"), "message loop missing")
    require('new Uri(settings.SidecarBaseUrl, "/v1/transmit")' in client, "sidecar transmit call missing")
    async_over_sync = "GetAwaiter()" + ".GetResult()"
    require(async_over_sync not in app, "SAP UI event path must not use async-over-sync")
    require("tenant_id" in client, "tenant id missing from payload")
    require("trace_id" in client, "trace id missing from payload")
    require("idempotency_key" in client, "idempotency key missing from payload")
    require("gateway_attempt_id" in client, "gateway attempt id missing from payload")
    require('form.TypeEx != "133"' in snapshot, "A/R invoice form guard missing")


def test_ci_workflow_covers_static_and_sdk_paths() -> None:
    workflow = (REPO / ".github/workflows/sap-b1-connector.yml").read_text(encoding="utf-8")

    require("python3 -m pytest addons/invoicekit-b1/tests -q" in workflow, "static test command missing")
    require("SAP_B1_SDK_ROOT" in workflow, "SAP SDK gate env missing")
    require("AddOnRegDataGen.exe" in workflow, "registration data generator check missing")
    require("dotnet build addons/invoicekit-b1/src/InvoiceKit.B1Addon.csproj" in workflow, "SDK build command missing")


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
        if path.is_file() and path.suffix.lower() in {".cs", ".csproj", ".json", ".md", ".py", ".xml", ".yml"}:
            text = path.read_text(encoding="utf-8")
            require(forbidden.search(text) is None, path)
