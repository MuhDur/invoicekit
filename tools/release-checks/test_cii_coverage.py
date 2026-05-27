"""CI gate: CII D16B element coverage is explicit and auditable."""

from __future__ import annotations

import json
import runpy
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ARTIFACT = REPO / "crates" / "format-cii" / "data" / "cii-d16b-element-coverage.json"
GENERATOR = REPO / "tools" / "cii-coverage" / "generate_coverage.py"

EXPECTED_CLASSES = {
    "cii_document_field_extension",
    "current_ir",
    "invoicekit_metadata_extension",
    "lossiness_ledger_preserved",
    "profile_extension_payload",
    "unsupported_gap",
}

EXPECTED_COUNTS = {
    "cii_document_field_extension": 2,
    "complex_types_reachable": 95,
    "current_ir": 74,
    "elements_total": 951,
    "invoicekit_metadata_extension": 0,
    "lossiness_ledger_preserved": 443,
    "profile_extension_payload": 9,
    "unsupported_gap": 423,
}

EXPECTED_SOURCE_HASHES = {
    "CrossIndustryInvoice_100pD16B.xsd": (
        "b9798aafcba039d0630f0015ffae5092a1758bf924d19d97b6bf0bf9d95f22a0"
    ),
    "CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd": (
        "cc682d67791ffe16c45320619ff089a1c8f3ad22691fc426e4056f2e67909fe0"
    ),
    "CrossIndustryInvoice_QualifiedDataType_100pD16B.xsd": (
        "a39e0662c31fbdca22237118d6b6bbfa16f69efbd1ee5b23787366349c5b924d"
    ),
    "CrossIndustryInvoice_UnqualifiedDataType_100pD16B.xsd": (
        "5d4ce8a6445caa4b7691f6fba6ad08e312e6a853cd3a422666d121ee2d13ef8c"
    ),
}


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


def _row(declaring_type: str, element: str) -> dict:
    for row in _artifact()["elements"]:
        if row["declaring_type"] == declaring_type and row["element"] == element:
            return row
    raise AssertionError(f"missing CII coverage row {declaring_type}/{element}")


def test_generator_and_artifact_use_same_source_constants() -> None:
    generated_namespace = runpy.run_path(str(GENERATOR))
    source = _artifact()["source"]

    _require_equal(source["repository"], generated_namespace["SOURCE_REPOSITORY"], "repository")
    _require_equal(source["tag"], generated_namespace["SOURCE_TAG"], "tag")
    _require_equal(source["tag_ref"], "refs/tags/validation-1.3.16", "tag ref")
    _require_equal(source["commit"], generated_namespace["SOURCE_COMMIT"], "commit")
    _require_equal(source["subset"], generated_namespace["SOURCE_SUBSET"], "schema subset")


def test_artifact_declares_pinned_cii_d16b_schema_bundle() -> None:
    artifact = _artifact()
    _require_equal(artifact["schema_version"], 1, "schema_version")
    _require_equal(set(artifact["classes"]), EXPECTED_CLASSES, "coverage class set")
    _require_equal(artifact["counts"], EXPECTED_COUNTS, "coverage counts")

    source_hashes = {
        source_file["path"]: source_file["sha256"]
        for source_file in artifact["source"]["source_files"]
    }
    _require_equal(source_hashes, EXPECTED_SOURCE_HASHES, "source file hashes")


def test_every_schema_edge_is_classified_non_silently() -> None:
    artifact = _artifact()
    seen = set()
    for row in artifact["elements"]:
        key = (row["declaring_type"], row["element"])
        _require(key not in seen, f"duplicate row {key}")
        seen.add(key)
        _require(row["class"] in EXPECTED_CLASSES, f"{key} has invalid class")
        _require(bool(row["strategy"].strip()), f"{key} must name a strategy")
        _require(".." in row["cardinality"], f"{key} must include schema cardinality")
        if row["class"] == "current_ir":
            _require(bool(row["current_ir_paths"]), f"{key} current row needs IR paths")
        if row["class"].endswith("_extension"):
            _require(bool(row["extension_fields"]), f"{key} extension row needs fields")

    _require_equal(len(seen), EXPECTED_COUNTS["elements_total"], "unique element rows")


def test_named_metadata_overload_boundaries_are_explicit() -> None:
    buyer_reference = _row("HeaderTradeAgreementType", "BuyerReference")
    _require_equal(
        buyer_reference["class"],
        "cii_document_field_extension",
        "BuyerReference must not be core metadata",
    )
    _require(
        "buyer_reference" in " ".join(buyer_reference["extension_fields"]),
        "BuyerReference extension field must be named",
    )
    _require(
        "tenant_id" in buyer_reference["strategy"],
        "BuyerReference strategy must document the tenant_id boundary",
    )

    business_process = _row(
        "ExchangedDocumentContextType",
        "BusinessProcessSpecifiedDocumentContextParameter",
    )
    _require_equal(
        business_process["class"],
        "cii_document_field_extension",
        "business process context must not be trace metadata",
    )
    _require(
        "business_process_context_ids[]" in " ".join(business_process["extension_fields"]),
        "business process extension field must be plural",
    )
    _require(
        "trace_id" in business_process["strategy"],
        "business process strategy must document the trace_id boundary",
    )

    application_context = _row(
        "ExchangedDocumentContextType",
        "ApplicationSpecifiedDocumentContextParameter",
    )
    _require_equal(
        application_context["class"],
        "profile_extension_payload",
        "application context must not be classified wholesale as InvoiceKit metadata",
    )
    named_decisions = _artifact()["named_mapping_decisions"]
    _require(
        any(
            decision["class"] == "invoicekit_metadata_extension"
            and "ApplicationSpecifiedDocumentContextParameter" in decision["element"]
            and "CommercialDocument.meta" in decision["representation"]
            for decision in named_decisions
        ),
        "InvoiceKit-owned application context metadata decision must be named separately",
    )


def test_known_cii_d16b_gap_families_are_not_silent() -> None:
    expectations = {
        ("TradeProductType", "GlobalID"): "lossiness_ledger_preserved",
        ("TradeProductType", "DesignatedProductClassification"): "lossiness_ledger_preserved",
        ("HeaderTradeDeliveryType", "ActualDespatchSupplyChainEvent"): (
            "lossiness_ledger_preserved"
        ),
        ("TradeAllowanceChargeType", "ReasonCode"): "lossiness_ledger_preserved",
        ("HeaderTradeSettlementType", "BillingSpecifiedPeriod"): "lossiness_ledger_preserved",
        ("HeaderTradeAgreementType", "SellerTaxRepresentativeTradeParty"): (
            "lossiness_ledger_preserved"
        ),
        ("HeaderTradeAgreementType", "AdditionalReferencedDocument"): (
            "lossiness_ledger_preserved"
        ),
        ("TradePartyType", "ID"): "lossiness_ledger_preserved",
        ("TradePartyType", "GlobalID"): "lossiness_ledger_preserved",
        ("CrossIndustryInvoiceType", "ValuationBreakdownStatement"): "unsupported_gap",
    }
    for key, expected_class in expectations.items():
        _require_equal(_row(*key)["class"], expected_class, f"{key} class")
