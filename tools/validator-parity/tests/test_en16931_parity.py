# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

from __future__ import annotations

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

# noqa import after sys.path setup
import en16931_parity


def test_core_rule_ids_keeps_only_en16931_business_rules() -> None:
    findings = [
        {"rule_id": "BR-01"},
        {"rule_id": "BR-CO-15"},
        {"rule_id": "PEPPOL-EN16931-R001"},
        {"rule_id": "KOSIT-CONFIGURATION-MISSING"},
        {"rule_id": None},
    ]

    assert en16931_parity.core_rule_ids(findings) == {"BR-01", "BR-CO-15"}


def test_oracle_unavailable_detects_configuration_failure() -> None:
    finding = {"rule_id": "KOSIT-CONFIGURATION-MISSING", "message": "set scenarios.xml"}

    if en16931_parity.oracle_unavailable([finding]) != finding:
        raise AssertionError("configuration failure was not detected")


def test_oracle_precondition_failure_detects_schema_error() -> None:
    finding = {"rule_id": "PHIVE-UNNAMED", "message": "[SAX] invalid content"}

    if en16931_parity.oracle_precondition_failure([finding]) != finding:
        raise AssertionError("schema precondition failure was not detected")


def test_fixture_id_is_repo_relative(tmp_path: Path) -> None:
    fixture = tmp_path / "conformance-corpus" / "synthetic" / "ubl" / "case" / "fixture.xml"
    fixture.parent.mkdir(parents=True)
    fixture.write_text("<Invoice/>", encoding="utf-8")

    assert (
        en16931_parity.fixture_id(tmp_path, fixture)
        == "conformance-corpus/synthetic/ubl/case/fixture.xml"
    )


def test_compare_backend_reports_mismatch(monkeypatch, tmp_path: Path) -> None:
    fixture = tmp_path / "conformance-corpus" / "synthetic" / "ubl" / "case" / "fixture.xml"
    fixture.parent.mkdir(parents=True)
    fixture.write_text("<Invoice/>", encoding="utf-8")

    def fake_rpc_validate(sidecar: dict[str, str], xml: str, request_id: str, timeout: float) -> dict:
        assert sidecar["backend"] == "jvm:phive"
        assert xml == "<Invoice/>"
        assert request_id.endswith("fixture.xml")
        assert timeout == 1.0
        return {"result": {"results": [{"rule_id": "BR-CO-15"}]}}

    def fake_run_rust_probe_xml(
        repo: Path, command: list[str], label: str, xml: str, timeout: float
    ) -> set[str]:
        assert repo == tmp_path
        assert command == ["probe"]
        assert label.endswith("fixture.xml")
        assert xml == "<Invoice/>"
        assert timeout == 2.0
        return {"BR-01"}

    monkeypatch.setattr(en16931_parity, "rpc_validate", fake_rpc_validate)
    monkeypatch.setattr(en16931_parity, "run_rust_probe_xml", fake_run_rust_probe_xml)

    summary = en16931_parity.compare_backend(
        tmp_path,
        {
            "backend": "jvm:phive",
            "profile": "peppol-bis",
            "projection": "none",
            "url": "http://127.0.0.1:9999",
        },
        [fixture],
        ["probe"],
        2.0,
        None,
        3.0,
        timeout=1.0,
    )

    assert summary["status"] == "fail"
    assert summary["parity"] == 0.0
    assert summary["mismatches"] == [
        {
            "fixture": "conformance-corpus/synthetic/ubl/case/fixture.xml",
            "rust_only": ["BR-01"],
            "oracle_only": ["BR-CO-15"],
        }
    ]


def test_compare_backend_normalizes_profile_projected_ubl(
    monkeypatch, tmp_path: Path
) -> None:
    fixture = tmp_path / "conformance-corpus" / "synthetic" / "ubl" / "case" / "fixture.xml"
    fixture.parent.mkdir(parents=True)
    fixture.write_text(
        '<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>',
        encoding="utf-8",
    )

    normalized_xml = "<Invoice><cbc:CustomizationID>normalized</cbc:CustomizationID></Invoice>"

    def fake_normalize_ubl_xml(
        repo: Path, command: list[str], label: str, xml: str, timeout: float
    ) -> str:
        assert repo == tmp_path
        assert command == ["normalize"]
        assert label.endswith("fixture.xml")
        assert en16931_parity.PEPPOL_CUSTOMIZATION_ID in xml
        assert timeout == 3.0
        return normalized_xml

    def fake_rpc_validate(sidecar: dict[str, str], xml: str, request_id: str, timeout: float) -> dict:
        assert sidecar["backend"] == "jvm:phive"
        assert xml == normalized_xml
        assert request_id.endswith("fixture.xml")
        assert timeout == 1.0
        return {"result": {"results": []}}

    def fake_run_rust_probe_xml(
        repo: Path, command: list[str], label: str, xml: str, timeout: float
    ) -> set[str]:
        assert repo == tmp_path
        assert command == ["probe"]
        assert label.endswith("fixture.xml")
        assert xml == normalized_xml
        assert timeout == 2.0
        return set()

    monkeypatch.setattr(en16931_parity, "normalize_ubl_xml", fake_normalize_ubl_xml)
    monkeypatch.setattr(en16931_parity, "rpc_validate", fake_rpc_validate)
    monkeypatch.setattr(en16931_parity, "run_rust_probe_xml", fake_run_rust_probe_xml)

    summary = en16931_parity.compare_backend(
        tmp_path,
        {
            "backend": "jvm:phive",
            "profile": "peppol-bis",
            "projection": "peppol-bis",
            "url": "http://127.0.0.1:9999",
        },
        [fixture],
        ["probe"],
        2.0,
        ["normalize"],
        3.0,
        timeout=1.0,
    )

    assert summary["status"] == "pass"
    assert summary["parity"] == 1.0


def test_project_xml_sets_peppol_profile_ids() -> None:
    xml = (
        '<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" '
        'xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">'
        "<cbc:CustomizationID>old</cbc:CustomizationID>"
        "<cbc:ProfileID>old</cbc:ProfileID>"
        "<cbc:ID>I-1</cbc:ID>"
        "</Invoice>"
    )

    projected = en16931_parity.project_xml(xml, "peppol-bis")

    assert en16931_parity.PEPPOL_CUSTOMIZATION_ID in projected
    assert en16931_parity.PEPPOL_PROFILE_ID in projected
    assert "<cbc:ID>I-1</cbc:ID>" in projected
