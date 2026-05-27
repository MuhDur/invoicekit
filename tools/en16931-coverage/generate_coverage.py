#!/usr/bin/env python3
"""Generate the checked EN 16931 BR/BR-CO coverage matrix.

The authoritative rule ids and rule text come from the generated XSLT files in
ConnectingEurope/eInvoicing-EN16931. The InvoiceKit mapping layer is kept here
so validator work can distinguish current IR coverage from explicit model gaps.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import subprocess
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT = REPO_ROOT / "crates/rulepack/data/en16931-br-co-coverage.json"
PROFILE_EXTENSION_URN = "urn:invoicekit:profile:en16931:2017"
SOURCE_REPOSITORY = "https://github.com/ConnectingEurope/eInvoicing-EN16931"
SOURCE_TAG = "validation-1.3.16"

SOURCE_FILES = (
    ("UBL", "ubl/xslt/EN16931-UBL-validation.xslt"),
    ("CII", "cii/xslt/EN16931-CII-validation.xslt"),
)

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

CURRENT_IR_PATHS = {
    "BG-3": ("CommercialDocument.references[kind=preceding_invoice]",),
    "BG-4": ("CommercialDocument.supplier",),
    "BG-5": ("CommercialDocument.supplier.address",),
    "BG-8": ("CommercialDocument.customer.address",),
    "BG-10": ("CommercialDocument.payee",),
    "BG-16": ("CommercialDocument.payment_instructions[]",),
    "BG-17": ("CommercialDocument.payment_instructions[]",),
    "BG-23": ("CommercialDocument.tax_summary[]",),
    "BG-25": ("CommercialDocument.lines[]",),
    "BT-1": ("CommercialDocument.document_number",),
    "BT-2": ("CommercialDocument.issue_date",),
    "BT-3": ("CommercialDocument.document_type",),
    "BT-5": ("CommercialDocument.currency",),
    "BT-7": ("CommercialDocument.tax_point_date",),
    "BT-24": ("ProfileView.profile.urn", "ProfileView.profile.version"),
    "BT-25": ("CommercialDocument.references[kind=preceding_invoice].id",),
    "BT-27": ("CommercialDocument.supplier.name",),
    "BT-29": ("CommercialDocument.supplier.id",),
    "BT-31": ("CommercialDocument.supplier.tax_ids[scheme=vat].value",),
    "BT-40": ("CommercialDocument.supplier.address.country",),
    "BT-44": ("CommercialDocument.customer.name",),
    "BT-48": ("CommercialDocument.customer.tax_ids[scheme=vat].value",),
    "BT-55": ("CommercialDocument.customer.address.country",),
    "BT-59": ("CommercialDocument.payee.name",),
    "BT-81": ("CommercialDocument.payment_instructions[].kind",),
    "BT-84": ("CommercialDocument.payment_instructions[].account",),
    "BT-106": ("CommercialDocument.monetary_total.line_extension_amount",),
    "BT-107": ("CommercialDocument.monetary_total.allowance_total_amount",),
    "BT-108": ("CommercialDocument.monetary_total.charge_total_amount",),
    "BT-109": ("CommercialDocument.monetary_total.tax_exclusive_amount",),
    "BT-112": ("CommercialDocument.monetary_total.tax_inclusive_amount",),
    "BT-113": ("CommercialDocument.monetary_total.prepaid_amount",),
    "BT-115": ("CommercialDocument.monetary_total.payable_amount",),
    "BT-116": ("CommercialDocument.tax_summary[].taxable_amount",),
    "BT-117": ("CommercialDocument.tax_summary[].tax_amount",),
    "BT-118": ("CommercialDocument.tax_summary[].category_code",),
    "BT-119": ("CommercialDocument.tax_summary[].tax_rate",),
    "BT-126": ("CommercialDocument.lines[].id",),
    "BT-129": ("CommercialDocument.lines[].quantity",),
    "BT-130": ("CommercialDocument.lines[].unit_code",),
    "BT-131": ("CommercialDocument.lines[].line_extension_amount",),
    "BT-146": ("CommercialDocument.lines[].unit_price",),
    "BT-151": ("CommercialDocument.lines[].tax_category",),
    "BT-153": ("CommercialDocument.lines[].description",),
}

REQUIRED_EXTENSION_FIELDS = {
    "BG-16": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].payment_means[].type_code",),
    "BG-17": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].credit_transfers[]",),
    "BG-11": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_tax_representative",),
    "BG-12": (
        f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_tax_representative.address",
    ),
    "BG-14": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].invoicing_period",),
    "BG-15": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].deliver_to_address",),
    "BG-20": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_allowances[]",),
    "BG-21": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_charges[]",),
    "BG-24": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].supporting_documents[]",),
    "BG-26": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_period",),
    "BG-27": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_allowances[]",),
    "BG-28": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_charges[]",),
    "BG-32": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_attributes[]",),
    "BT-6": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].vat_accounting_currency_code",),
    "BT-8": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].tax_point_date_code",),
    "BT-30": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_legal_registration_id",),
    "BT-34": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_endpoint.scheme_id",),
    "BT-49": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].buyer_endpoint.scheme_id",),
    "BT-62": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_tax_representative.name",),
    "BT-63": (
        f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_tax_representative.vat_id",
    ),
    "BT-69": (
        f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].seller_tax_representative.address.country",
    ),
    "BT-73": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].invoicing_period.start_date",),
    "BT-74": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].invoicing_period.end_date",),
    "BT-80": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].deliver_to_address.country",),
    "BT-81": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].payment_means[].type_code",),
    "BT-87": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].payment_card.primary_account_number",),
    "BT-92": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_allowances[].amount",),
    "BT-95": (
        f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_allowances[].vat_category_code",
    ),
    "BT-97": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_allowances[].reason",),
    "BT-98": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_allowances[].reason_code",),
    "BT-99": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_charges[].amount",),
    "BT-102": (
        f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_charges[].vat_category_code",
    ),
    "BT-104": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_charges[].reason",),
    "BT-105": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].document_charges[].reason_code",),
    "BT-110": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].vat_total_amount",),
    "BT-111": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].vat_accounting_currency_amount",),
    "BT-114": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].rounding_amount",),
    "BT-122": (f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].supporting_documents[].reference",),
    "BT-134": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_period.start_date",),
    "BT-135": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_period.end_date",),
    "BT-136": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_allowances[].amount",),
    "BT-139": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_allowances[].reason",),
    "BT-140": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_allowances[].reason_code",),
    "BT-141": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_charges[].amount",),
    "BT-144": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_charges[].reason",),
    "BT-145": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].line_charges[].reason_code",),
    "BT-148": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_gross_price",),
    "BT-157": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_standard_identifier.scheme_id",),
    "BT-158": (
        f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_classification_identifier.scheme_id",
    ),
    "BT-160": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_attributes[].name",),
    "BT-161": (f"DocumentLine.extensions[{PROFILE_EXTENSION_URN}].item_attributes[].value",),
}

TERM_CODE_RE = re.compile(r"\b(BT|BG)-\d+\b")
TEMPLATE_RE = re.compile(r'<xsl:template\s+match="([^"]+)"')
FAILED_ASSERT_RE = re.compile(r'<svrl:failed-assert\s+test="([^"]+)"')
ID_RE = re.compile(r'<xsl:attribute\s+name="id">(BR-(?:CO-)?\d+)</xsl:attribute>')
FLAG_RE = re.compile(r'<xsl:attribute\s+name="flag">([^<]+)</xsl:attribute>')
TEXT_RE = re.compile(r"<svrl:text>\[(BR-(?:CO-)?\d+)\]-(.*?)</svrl:text>")
LABEL_RE = re.compile(r"([A-Za-z][A-Za-z0-9 /'-]+?)\s+\((B[GT]-\d+)\)")

TERM_OVERRIDES = {
    # The CII XSLT text for BR-51 says BT-97, but the context/test and the UBL
    # text both describe the payment card primary account number business term.
    "BR-51": ("BT-87",),
}


def natural_rule_key(rule_id: str) -> tuple[Any, ...]:
    return tuple(int(part) if part.isdigit() else part for part in re.split(r"(\d+)", rule_id))


def normalize_text(value: str) -> str:
    return " ".join(html.unescape(value).replace("\u00a0", " ").split())


def parse_source_file(source_root: Path, syntax: str, relative_path: str) -> list[dict[str, Any]]:
    path = source_root / relative_path
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    context: str | None = None
    failed_assert_test: str | None = None
    failed_assert_line: int | None = None
    pending_id: str | None = None
    pending_id_line: int | None = None
    pending_flag: str | None = None
    records: list[dict[str, Any]] = []

    for line_number, line in enumerate(lines, start=1):
        if match := TEMPLATE_RE.search(line):
            context = normalize_text(match.group(1))
        if match := FAILED_ASSERT_RE.search(line):
            failed_assert_test = normalize_text(match.group(1))
            failed_assert_line = line_number
        if match := ID_RE.search(line):
            pending_id = match.group(1)
            pending_id_line = line_number
            pending_flag = None
        if match := FLAG_RE.search(line):
            pending_flag = normalize_text(match.group(1))
        if match := TEXT_RE.search(line):
            rule_id = match.group(1)
            if pending_id != rule_id:
                raise ValueError(f"{relative_path}:{line_number}: text {rule_id} does not match id {pending_id}")
            records.append(
                {
                    "id": rule_id,
                    "text": normalize_text(match.group(2)),
                    "source": {
                        "syntax": syntax,
                        "file": relative_path,
                        "id_line": pending_id_line,
                        "text_line": line_number,
                        "context": context,
                        "test": failed_assert_test,
                        "assert_line": failed_assert_line,
                        "severity_flag": pending_flag,
                    },
                }
            )
    return records


def source_commit(source_root: Path) -> str:
    try:
        completed = subprocess.run(
            ["git", "-C", str(source_root), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, FileNotFoundError):
        return "unknown"
    return completed.stdout.strip()


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def term_label(code: str, texts: list[str]) -> str:
    labels: list[str] = []
    for text in texts:
        for match in LABEL_RE.finditer(text):
            label = normalize_text(match.group(1))
            found_code = match.group(2)
            if found_code == code and label not in labels:
                labels.append(label)
    return labels[0] if labels else code


def mapping_for(code: str, texts: list[str]) -> dict[str, Any]:
    current_paths = list(CURRENT_IR_PATHS.get(code, ()))
    required_fields = list(REQUIRED_EXTENSION_FIELDS.get(code, ()))
    if not current_paths and not required_fields:
        required_fields = [f"CommercialDocument.extensions[{PROFILE_EXTENSION_URN}].unmapped.{code.lower().replace('-', '_')}"]
    return {
        "code": code,
        "label": term_label(code, texts),
        "current_ir_paths": current_paths,
        "required_extension_fields": required_fields,
        "coverage": "current" if current_paths and not required_fields else "gap" if required_fields else "unknown",
    }


def build_artifact(source_root: Path) -> dict[str, Any]:
    parsed: dict[str, dict[str, Any]] = {}
    for syntax, relative_path in SOURCE_FILES:
        for record in parse_source_file(source_root, syntax, relative_path):
            rule = parsed.setdefault(
                record["id"],
                {"id": record["id"], "texts": [], "source_locations": []},
            )
            if record["text"] not in rule["texts"]:
                rule["texts"].append(record["text"])
            rule["source_locations"].append(record["source"])

    actual_ids = tuple(sorted(parsed, key=natural_rule_key))
    if actual_ids != EXPECTED_RULE_IDS:
        missing = sorted(set(EXPECTED_RULE_IDS) - set(actual_ids), key=natural_rule_key)
        extra = sorted(set(actual_ids) - set(EXPECTED_RULE_IDS), key=natural_rule_key)
        raise ValueError(f"unexpected BR/BR-CO rule set; missing={missing}; extra={extra}")

    rules: list[dict[str, Any]] = []
    for rule_id in EXPECTED_RULE_IDS:
        rule = parsed[rule_id]
        texts = sorted(rule["texts"])
        codes = sorted(
            TERM_OVERRIDES.get(
                rule_id,
                tuple({match.group(0) for text in texts for match in TERM_CODE_RE.finditer(text)}),
            ),
            key=lambda value: (value[:2], int(value[3:])),
        )
        term_mappings = [mapping_for(code, texts) for code in codes]
        current_paths = sorted(
            {path for mapping in term_mappings for path in mapping["current_ir_paths"]}
        )
        required_fields = sorted(
            {field for mapping in term_mappings for field in mapping["required_extension_fields"]}
        )
        fully_represented = not required_fields
        rules.append(
            {
                "id": rule_id,
                "text": texts[0],
                "text_variants": texts,
                "business_terms": [code for code in codes if code.startswith("BT-")],
                "business_groups": [code for code in codes if code.startswith("BG-")],
                "term_mappings": term_mappings,
                "source_locations": sorted(
                    rule["source_locations"],
                    key=lambda source: (source["syntax"], source["file"], source["id_line"] or 0),
                ),
                "current_ir_paths": current_paths,
                "required_extension_fields": required_fields,
                "rust_validator_testability": {
                    "positive": fully_represented,
                    "negative": fully_represented,
                    "blocker": None
                    if fully_represented
                    else "IR gap: add the required extension fields before treating this rule as covered.",
                },
            }
        )

    source_file_entries = []
    for _syntax, relative_path in SOURCE_FILES:
        path = source_root / relative_path
        source_file_entries.append(
            {"path": relative_path, "sha256": file_sha256(path)}
        )

    br_count = sum(1 for rule in rules if re.fullmatch(r"BR-\d+", rule["id"]))
    br_co_count = sum(1 for rule in rules if re.fullmatch(r"BR-CO-\d+", rule["id"]))
    blocked_count = sum(
        1 for rule in rules if not rule["rust_validator_testability"]["positive"]
    )
    return {
        "schema_version": 1,
        "generated_at": "2026-05-27",
        "source": {
            "repository": SOURCE_REPOSITORY,
            "tag": SOURCE_TAG,
            "tag_ref": f"refs/tags/{SOURCE_TAG}",
            "commit": source_commit(source_root),
            "source_files": source_file_entries,
            "rule_filter": "BR-* and BR-CO-* assertions only; excludes BR-CL, BR-DEC, BR-S, BR-E, syntax-only, and VAT-category-specific rules.",
        },
        "counts": {
            "rules_total": len(rules),
            "br": br_count,
            "br_co": br_co_count,
            "source_assertions": sum(len(rule["source_locations"]) for rule in rules),
            "validator_testable_now": len(rules) - blocked_count,
            "blocked_by_ir_gaps": blocked_count,
        },
        "ir_gap_policy": {
            "rule": "A rule is validator-testable only when every referenced BT/BG code has a current IR path and no required extension field.",
            "extension_urn": PROFILE_EXTENSION_URN,
        },
        "rules": rules,
    }


def write_artifact(artifact: dict[str, Any], output: Path) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source-root",
        required=True,
        type=Path,
        help="Path to a checkout of ConnectingEurope/eInvoicing-EN16931 at validation-1.3.16.",
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    artifact = build_artifact(args.source_root)
    write_artifact(artifact, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
