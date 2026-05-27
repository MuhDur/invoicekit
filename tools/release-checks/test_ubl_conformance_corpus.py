# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Release checks for the synthetic UBL 2.1 conformance corpus."""

from __future__ import annotations

import sys
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "conformance-corpus" / "synthetic" / "ubl-2-1"
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
    "party-tax-registration",
    "party-contact",
    "payee-party",
    "buyer-reference",
    "accounting-cost",
    "customization-and-profile-ids",
    "included-notes",
    "profile-peppol-bis-billing-3",
    "profile-xrechnung-ubl",
    "profile-peppol-pint",
    "profile-en16931-ubl",
    "profile-peppol-bis-credit-note",
}


def load_metadata() -> list[dict[str, object]]:
    metadata_paths = sorted(CORPUS.glob("ubl-2-1-*/metadata.json"))
    if len(metadata_paths) != EXPECTED_FIXTURES:
        raise AssertionError(
            f"expected {EXPECTED_FIXTURES} UBL metadata files, got {len(metadata_paths)}"
        )
    return [metadata_validator.load_json(path) for path in metadata_paths]


def test_ubl_corpus_metadata_and_hashes_validate() -> None:
    metadata_files = metadata_validator.validate_all()
    ubl_metadata = [path for path in metadata_files if CORPUS in path.parents]
    assert len(ubl_metadata) == EXPECTED_FIXTURES


def test_ubl_corpus_has_expected_count_and_unique_fixture_ids() -> None:
    metadata = load_metadata()
    fixture_ids = [item["fixture_id"] for item in metadata]
    assert len(set(fixture_ids)) == EXPECTED_FIXTURES
    assert fixture_ids[0] == "ik-synthetic-ubl-2-1-0001"
    assert fixture_ids[-1] == "ik-synthetic-ubl-2-1-0050"


def test_ubl_corpus_declares_required_coverage_scenarios() -> None:
    scenarios: set[str] = set()
    profiles: set[str] = set()
    document_types: set[str] = set()
    for item in load_metadata():
        coverage = item["coverage"]
        artifact = item["artifact"]
        jurisdiction = item["jurisdiction"]
        assert coverage["capabilities"] == ["parse", "serialize", "validate"]
        assert item["validation"]["expected_outcome"] == "valid"
        assert all(
            validator["result"] == "pass" for validator in item["validation"]["validators"]
        )
        scenarios.update(coverage["scenarios"])
        profiles.add(jurisdiction["profile"])
        document_types.add(artifact["document_type"])

    assert document_types == {"invoice", "credit_note"}
    assert profiles == {
        "Peppol BIS Billing 3.0",
        "XRechnung UBL 3.0",
        "Peppol PINT (international)",
        "EN 16931 (UBL core)",
        "Peppol BIS Billing 3.0 (CreditNote)",
    }
    assert REQUIRED_SCENARIOS <= scenarios
