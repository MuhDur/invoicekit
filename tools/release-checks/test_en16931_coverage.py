"""CI gate: EN 16931 BR/BR-CO coverage matrix is complete and non-silent.

The coverage artifact is the bridge between the official CEN/ConnectingEurope
validation release and InvoiceKit's current IR. A rule may be implemented only
when every referenced BT/BG code is represented in the IR; otherwise the
artifact must name the extension field that blocks native Rust validation.
"""

from __future__ import annotations

import json
import re
import runpy
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ARTIFACT = REPO / "crates" / "rulepack" / "data" / "en16931-br-co-coverage.json"
GENERATOR = REPO / "tools" / "en16931-coverage" / "generate_coverage.py"

EXPECTED_RULE_IDS = (
    "BR-01",
    "BR-02",
    "BR-03",
    "BR-04",
    "BR-05",
    "BR-06",
    "BR-07",
    "BR-08",
    "BR-09",
    "BR-10",
    "BR-11",
    "BR-12",
    "BR-13",
    "BR-14",
    "BR-15",
    "BR-16",
    "BR-17",
    "BR-18",
    "BR-19",
    "BR-20",
    "BR-21",
    "BR-22",
    "BR-23",
    "BR-24",
    "BR-25",
    "BR-26",
    "BR-27",
    "BR-28",
    "BR-29",
    "BR-30",
    "BR-31",
    "BR-32",
    "BR-33",
    "BR-36",
    "BR-37",
    "BR-38",
    "BR-41",
    "BR-42",
    "BR-43",
    "BR-44",
    "BR-45",
    "BR-46",
    "BR-47",
    "BR-48",
    "BR-49",
    "BR-50",
    "BR-51",
    "BR-52",
    "BR-53",
    "BR-54",
    "BR-55",
    "BR-56",
    "BR-57",
    "BR-61",
    "BR-62",
    "BR-63",
    "BR-64",
    "BR-65",
    "BR-CO-03",
    "BR-CO-04",
    "BR-CO-05",
    "BR-CO-06",
    "BR-CO-07",
    "BR-CO-08",
    "BR-CO-09",
    "BR-CO-10",
    "BR-CO-11",
    "BR-CO-12",
    "BR-CO-13",
    "BR-CO-14",
    "BR-CO-15",
    "BR-CO-16",
    "BR-CO-17",
    "BR-CO-18",
    "BR-CO-19",
    "BR-CO-20",
    "BR-CO-21",
    "BR-CO-22",
    "BR-CO-23",
    "BR-CO-24",
    "BR-CO-26",
)


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def _require_equal(actual: object, expected: object, message: str) -> None:
    if actual != expected:
        raise AssertionError(f"{message}: expected {expected!r}, got {actual!r}")


def _artifact() -> dict:
    try:
        return json.JSONDecoder().decode(ARTIFACT.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise AssertionError(f"{ARTIFACT} is not valid JSON: {error}") from error


def _rules_by_id() -> dict[str, dict]:
    return {rule["id"]: rule for rule in _artifact()["rules"]}


def test_generator_and_artifact_rule_sets_match() -> None:
    generated_namespace = runpy.run_path(str(GENERATOR))
    _require_equal(
        generated_namespace["EXPECTED_RULE_IDS"],
        EXPECTED_RULE_IDS,
        "generator EXPECTED_RULE_IDS drifted",
    )
    _require_equal(
        tuple(rule["id"] for rule in _artifact()["rules"]),
        EXPECTED_RULE_IDS,
        "coverage artifact rule ids drifted",
    )


def test_artifact_declares_official_source_release() -> None:
    artifact = _artifact()
    _require_equal(artifact["schema_version"], 1, "schema_version must stay stable")
    _require_equal(
        artifact["source"]["repository"],
        "https://github.com/ConnectingEurope/eInvoicing-EN16931",
        "source repository must be the official ConnectingEurope repository",
    )
    _require_equal(
        artifact["source"]["tag"],
        "validation-1.3.16",
        "source tag must be pinned",
    )
    _require_equal(
        artifact["source"]["tag_ref"],
        "refs/tags/validation-1.3.16",
        "source tag ref must be pinned",
    )
    _require_equal(
        artifact["source"]["commit"],
        "b6c9e06a59812fb1a83585da40923b3678a649ad",
        "source commit must be the lightweight tag target",
    )
    _require_equal(
        {source["path"] for source in artifact["source"]["source_files"]},
        {
            "ubl/xslt/EN16931-UBL-validation.xslt",
            "cii/xslt/EN16931-CII-validation.xslt",
        },
        "source file set must stay scoped to EN16931 UBL/CII XSLT",
    )


def test_artifact_counts_match_exact_br_br_co_scope() -> None:
    counts = _artifact()["counts"]
    _require_equal(counts["rules_total"], 81, "rules_total")
    _require_equal(counts["br"], 58, "plain BR rule count")
    _require_equal(counts["br_co"], 23, "BR-CO rule count")
    _require_equal(counts["source_assertions"], 162, "source assertion count")
    _require_equal(
        counts["validator_testable_now"] + counts["blocked_by_ir_gaps"],
        81,
        "testable plus blocked rules must equal total scope",
    )


def test_every_rule_has_terms_sources_and_non_silent_ir_mapping() -> None:
    for rule in _artifact()["rules"]:
        rule_id = rule["id"]
        _require(re.fullmatch(r"BR-(CO-)?\d+", rule_id) is not None, rule_id)
        _require(bool(rule["text"].strip()), rule_id)
        _require(bool(rule["text_variants"]), rule_id)
        _require(bool(rule["business_terms"] or rule["business_groups"]), rule_id)
        _require(bool(rule["term_mappings"]), rule_id)
        _require(bool(rule["source_locations"]), rule_id)

        for source in rule["source_locations"]:
            _require(source["syntax"] in {"UBL", "CII"}, rule_id)
            _require(source["file"].endswith("-validation.xslt"), rule_id)
            _require(isinstance(source["id_line"], int) and source["id_line"] > 0, rule_id)
            _require(isinstance(source["text_line"], int) and source["text_line"] > 0, rule_id)
            _require(bool(source["context"]), rule_id)
            _require(bool(source["test"]), rule_id)
            _require(source["severity_flag"] in {"fatal", "warning"}, rule_id)

        _require(bool(rule["current_ir_paths"] or rule["required_extension_fields"]), rule_id)
        for mapping in rule["term_mappings"]:
            mapping_id = f"{rule_id}:{mapping['code']}"
            _require(
                mapping["code"] in rule["business_terms"] + rule["business_groups"],
                mapping_id,
            )
            _require(
                bool(mapping["current_ir_paths"] or mapping["required_extension_fields"]),
                mapping_id,
            )
            _require(mapping["coverage"] in {"current", "gap"}, mapping_id)

        testability = rule["rust_validator_testability"]
        _require_equal(testability["positive"], testability["negative"], rule_id)
        if rule["required_extension_fields"]:
            _require(not testability["positive"], rule_id)
            _require(bool(testability["blocker"]), rule_id)
        else:
            _require(bool(testability["positive"]), rule_id)
            _require(testability["blocker"] is None, rule_id)


def test_known_ir_gaps_are_explicitly_blocking() -> None:
    rules = _rules_by_id()
    _require(
        "document_allowances[].amount" in " ".join(rules["BR-31"]["required_extension_fields"]),
        "BR-31 must expose the document allowance row gap",
    )
    _require(
        "credit_transfers[]" in " ".join(rules["BR-50"]["required_extension_fields"]),
        "BR-50 must expose the exact credit transfer group gap",
    )
    _require(
        rules["BR-51"]["business_terms"] == ["BT-87"],
        "BR-51 must normalize the upstream CII BT-97 typo to the card PAN term",
    )
    _require(
        "document_allowances" not in " ".join(rules["BR-51"]["required_extension_fields"]),
        "BR-51 must not inherit the unrelated document allowance gap",
    )
    _require(
        "vat_accounting_currency_code" in " ".join(rules["BR-53"]["required_extension_fields"]),
        "BR-53 must expose the VAT accounting currency gap",
    )
    _require(
        "payment_means[].type_code" in " ".join(rules["BR-61"]["required_extension_fields"]),
        "BR-61 must expose the exact payment means type code gap",
    )
    _require(
        "seller_endpoint.scheme_id" in " ".join(rules["BR-62"]["required_extension_fields"]),
        "BR-62 must expose the seller endpoint scheme id gap",
    )
    _require(
        bool(rules["BR-CO-10"]["rust_validator_testability"]["positive"]),
        "BR-CO-10 should remain testable with current IR line totals",
    )
