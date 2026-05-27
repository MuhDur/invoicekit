# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Release checks for the synthetic CII D16B conformance corpus."""

from __future__ import annotations

import sys
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "conformance-corpus" / "synthetic" / "cii-d16b"
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


def load_metadata() -> list[dict[str, object]]:
    metadata_paths = sorted(CORPUS.glob("cii-d16b-*/metadata.json"))
    if len(metadata_paths) != EXPECTED_FIXTURES:
        raise AssertionError(f"expected {EXPECTED_FIXTURES} CII metadata files, got {len(metadata_paths)}")
    return [metadata_validator.load_json(path) for path in metadata_paths]


def test_cii_corpus_metadata_and_hashes_validate() -> None:
    metadata_files = metadata_validator.validate_all()
    cii_metadata = [path for path in metadata_files if CORPUS in path.parents]
    assert len(cii_metadata) == EXPECTED_FIXTURES


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
