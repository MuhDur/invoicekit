# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Release checks for the synthetic CII D16B conformance corpus."""

from __future__ import annotations

import sys
import xml.etree.ElementTree as ET
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "conformance-corpus" / "synthetic" / "cii-d16b-profiled"
LEGACY_CORPUS = REPO / "conformance-corpus" / "synthetic" / "cii-d16b"
CONFORMANCE_TOOL = REPO / "tools" / "conformance-corpus"
sys.path.insert(0, str(CONFORMANCE_TOOL))
import validate_fixture_metadata as metadata_validator  # noqa: E402


EXPECTED_FIXTURES = 50
REQUIRED_SCENARIOS = {
    "invoice-type-code-380",
    "credit-note-type-code-381",
    "multi-line-document",
    "vat-category-standard",
    "vat-category-reduced",
    "vat-category-zero",
    "vat-category-exempt",
    "vat-category-reverse-charge",
    "header-allowance-total",
    "header-charge-total",
    "prepaid-amount",
    "payment-means-iban",
    "payment-reference",
    "payment-terms",
    "delivery-event",
    "party-tax-registration",
    "party-contact",
    "payee-party",
    "buyer-reference",
    "business-process-context",
    "included-notes",
    "profile-factur-x-minimum",
    "profile-factur-x-basic-wl",
    "profile-factur-x-basic",
    "profile-factur-x-en16931",
    "profile-factur-x-extended",
    "profile-xrechnung",
}
PROFILE_GUIDELINES = {
    "Factur-X MINIMUM": "urn:factur-x.eu:1p0:minimum",
    "Factur-X BASIC WL": "urn:factur-x.eu:1p0:basicwl",
    "Factur-X BASIC": "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
    "Factur-X EN 16931": "urn:cen.eu:en16931:2017",
    "Factur-X EXTENDED": "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
    "XRechnung CII": (
        "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0"
    ),
}


def load_metadata() -> list[dict[str, object]]:
    metadata_paths = sorted(CORPUS.glob("cii-d16b-*/metadata.json"))
    if len(metadata_paths) != EXPECTED_FIXTURES:
        raise AssertionError(f"expected {EXPECTED_FIXTURES} CII metadata files, got {len(metadata_paths)}")
    return [metadata_validator.load_json(path) for path in metadata_paths]


def load_legacy_metadata() -> list[dict[str, object]]:
    metadata_paths = sorted(LEGACY_CORPUS.glob("cii-d16b-*/metadata.json"))
    if len(metadata_paths) != EXPECTED_FIXTURES:
        raise AssertionError(f"expected {EXPECTED_FIXTURES} legacy CII metadata files, got {len(metadata_paths)}")
    return [metadata_validator.load_json(path) for path in metadata_paths]


def test_cii_corpus_metadata_and_hashes_validate() -> None:
    metadata_files = metadata_validator.validate_all()
    cii_metadata = [path for path in metadata_files if CORPUS in path.parents]
    assert len(cii_metadata) == EXPECTED_FIXTURES


def test_cii_fixture_ids_are_unique_across_active_and_legacy_corpora() -> None:
    fixture_ids = [item["fixture_id"] for item in load_metadata() + load_legacy_metadata()]
    assert len(fixture_ids) == EXPECTED_FIXTURES * 2
    assert len(set(fixture_ids)) == len(fixture_ids)


def test_legacy_cii_corpus_is_retired_parser_regression_data() -> None:
    for item in load_legacy_metadata():
        coverage = item["coverage"]
        jurisdiction = item["jurisdiction"]
        assert item["status"] == "retired"
        assert str(item["fixture_id"]).startswith("ik-synthetic-cii-d16b-legacy-")
        assert jurisdiction["profile"] == "Retired CII D16B legacy parser regression"
        assert "legacy-profile-context-overload" in coverage["scenarios"]
        assert not any(str(scenario).startswith("profile-") for scenario in coverage["scenarios"])


def test_cii_corpus_has_expected_count_and_unique_fixture_ids() -> None:
    metadata = load_metadata()
    fixture_ids = [item["fixture_id"] for item in metadata]
    assert len(set(fixture_ids)) == EXPECTED_FIXTURES
    assert fixture_ids[0] == "ik-synthetic-cii-d16b-0001"
    assert fixture_ids[-1] == "ik-synthetic-cii-d16b-0050"


def test_cii_corpus_declares_required_coverage_scenarios() -> None:
    scenarios: set[str] = set()
    profiles: set[str] = set()
    document_types: set[str] = set()
    for item in load_metadata():
        coverage = item["coverage"]
        artifact = item["artifact"]
        jurisdiction = item["jurisdiction"]
        assert coverage["capabilities"] == ["parse", "serialize", "validate"]
        assert item["validation"]["expected_outcome"] == "valid"
        assert all(validator["result"] == "pass" for validator in item["validation"]["validators"])
        scenarios.update(coverage["scenarios"])
        profiles.add(jurisdiction["profile"])
        document_types.add(artifact["document_type"])

    assert document_types == {"invoice", "credit_note"}
    assert profiles == {
        "Factur-X MINIMUM",
        "Factur-X BASIC WL",
        "Factur-X BASIC",
        "Factur-X EN 16931",
        "Factur-X EXTENDED",
        "XRechnung CII",
    }
    assert REQUIRED_SCENARIOS <= scenarios


def test_cii_profile_claims_are_encoded_as_guideline_context() -> None:
    observed: set[str] = set()
    for item in load_metadata():
        fixture_dir = CORPUS / f"cii-d16b-{item['fixture_id'].rsplit('-', maxsplit=1)[-1]}"
        guideline_ids = guideline_context_ids(fixture_dir / "fixture.xml")
        expected = PROFILE_GUIDELINES[item["jurisdiction"]["profile"]]
        assert expected in guideline_ids
        observed.update(guideline_ids)

    assert set(PROFILE_GUIDELINES.values()) <= observed


def guideline_context_ids(path: Path) -> set[str]:
    root = ET.parse(path).getroot()
    ids: set[str] = set()
    for parameter in root.iter():
        if local_name(parameter.tag) != "GuidelineSpecifiedDocumentContextParameter":
            continue
        for child in parameter:
            if local_name(child.tag) == "ID" and child.text:
                ids.add(child.text)
    return ids


def local_name(tag: str) -> str:
    return tag.rsplit("}", maxsplit=1)[-1]
