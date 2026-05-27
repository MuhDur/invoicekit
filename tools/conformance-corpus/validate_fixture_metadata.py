#!/usr/bin/env python3
"""Validate InvoiceKit conformance fixture metadata and artifact integrity."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import date, datetime
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


REPO = Path(__file__).resolve().parents[2]
CORPUS_ROOT = REPO / "conformance-corpus"
SCHEMA_PATH = CORPUS_ROOT / "fixture-metadata.schema.json"


class MetadataError(ValueError):
    """Raised when fixture metadata violates the InvoiceKit schema or policy."""


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.JSONDecoder().decode(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise MetadataError(f"{path}: invalid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise MetadataError(f"{path}: expected a JSON object")
    return value


def validate_instance(value: Any, schema: dict[str, Any], path: str = "$") -> None:
    if "const" in schema and value != schema["const"]:
        raise MetadataError(f"{path}: expected constant {schema['const']!r}")

    if "enum" in schema and value not in schema["enum"]:
        allowed = ", ".join(repr(item) for item in schema["enum"])
        raise MetadataError(f"{path}: expected one of {allowed}")

    expected_type = schema.get("type")
    if expected_type is not None:
        _validate_type(value, expected_type, path)

    if isinstance(value, dict):
        _validate_object(value, schema, path)
    elif isinstance(value, list):
        _validate_array(value, schema, path)
    elif isinstance(value, str):
        _validate_string(value, schema, path)
    elif isinstance(value, int) and not isinstance(value, bool):
        _validate_integer(value, schema, path)


def _validate_type(value: Any, expected_type: str, path: str) -> None:
    checks = {
        "object": lambda item: isinstance(item, dict),
        "array": lambda item: isinstance(item, list),
        "string": lambda item: isinstance(item, str),
        "integer": lambda item: isinstance(item, int) and not isinstance(item, bool),
        "boolean": lambda item: isinstance(item, bool),
    }
    check = checks.get(expected_type)
    if check is None:
        raise MetadataError(f"{path}: schema uses unsupported type {expected_type!r}")
    if not check(value):
        raise MetadataError(f"{path}: expected {expected_type}")


def _validate_object(value: dict[str, Any], schema: dict[str, Any], path: str) -> None:
    required = schema.get("required", [])
    for key in required:
        if key not in value:
            raise MetadataError(f"{path}: missing required property {key!r}")

    properties = schema.get("properties", {})
    if not schema.get("additionalProperties", True):
        extra = sorted(set(value) - set(properties))
        if extra:
            raise MetadataError(f"{path}: unexpected properties {', '.join(extra)}")

    for key, child in properties.items():
        if key in value:
            validate_instance(value[key], child, f"{path}.{key}")


def _validate_array(value: list[Any], schema: dict[str, Any], path: str) -> None:
    min_items = schema.get("minItems")
    if min_items is not None and len(value) < min_items:
        raise MetadataError(f"{path}: expected at least {min_items} item(s)")

    if bool(schema.get("uniqueItems", False)):
        seen: set[str] = set()
        for item in value:
            marker = json.dumps(item, sort_keys=True, separators=(",", ":"))
            if marker in seen:
                raise MetadataError(f"{path}: duplicate array item {item!r}")
            seen.add(marker)

    item_schema = schema.get("items")
    if item_schema is not None:
        for index, item in enumerate(value):
            validate_instance(item, item_schema, f"{path}[{index}]")


def _validate_string(value: str, schema: dict[str, Any], path: str) -> None:
    min_length = schema.get("minLength")
    if min_length is not None and len(value) < min_length:
        raise MetadataError(f"{path}: expected at least {min_length} character(s)")

    pattern = schema.get("pattern")
    if pattern is not None and re.fullmatch(pattern, value) is None:
        raise MetadataError(f"{path}: does not match pattern {pattern!r}")

    fmt = schema.get("format")
    if fmt == "date":
        _parse_date(value, path)
    elif fmt == "date-time":
        _parse_datetime(value, path)
    elif fmt == "uri":
        parsed = urlparse(value)
        if parsed.scheme not in {"http", "https", "urn"}:
            raise MetadataError(f"{path}: expected http, https, or urn URI")


def _validate_integer(value: int, schema: dict[str, Any], path: str) -> None:
    minimum = schema.get("minimum")
    if minimum is not None and value < minimum:
        raise MetadataError(f"{path}: expected value >= {minimum}")


def _parse_date(value: str, path: str) -> date:
    try:
        return date.fromisoformat(value)
    except ValueError as exc:
        raise MetadataError(f"{path}: expected ISO 8601 date") from exc


def _parse_datetime(value: str, path: str) -> datetime:
    normalized = value.replace("Z", "+00:00")
    try:
        return datetime.fromisoformat(normalized)
    except ValueError as exc:
        raise MetadataError(f"{path}: expected ISO 8601 date-time") from exc


def validate_artifact(metadata: dict[str, Any], metadata_path: Path) -> None:
    artifact = metadata["artifact"]
    relative_path = Path(artifact["path"])
    if relative_path.is_absolute() or ".." in relative_path.parts:
        raise MetadataError(f"{metadata_path}: artifact.path must stay inside the fixture directory")

    artifact_path = metadata_path.parent / relative_path
    if not artifact_path.is_file():
        raise MetadataError(f"{metadata_path}: artifact file not found: {relative_path}")

    payload = artifact_path.read_bytes()
    actual_size = len(payload)
    if actual_size != artifact["size_bytes"]:
        raise MetadataError(
            f"{metadata_path}: artifact size mismatch for {relative_path}: "
            f"metadata={artifact['size_bytes']} actual={actual_size}"
        )

    actual_sha256 = hashlib.sha256(payload).hexdigest()
    if actual_sha256 != artifact["sha256"]:
        raise MetadataError(
            f"{metadata_path}: artifact sha256 mismatch for {relative_path}: "
            f"metadata={artifact['sha256']} actual={actual_sha256}"
        )


def validate_policy_semantics(metadata: dict[str, Any], metadata_path: Path) -> None:
    partition = metadata["corpus_partition"]
    publication = metadata["publication"]
    license_info = metadata["license"]
    provenance = metadata["provenance"]
    pii = metadata["pii"]

    if publication == "public":
        if license_info["redistribution"] != "public-ok":
            raise MetadataError(f"{metadata_path}: public fixtures require redistribution=public-ok")
        if pii["contains_personal_data"]:
            raise MetadataError(f"{metadata_path}: public fixtures must not contain personal data")

    if partition == "synthetic":
        if metadata["fixture_id"].split("-")[1] != "synthetic":
            raise MetadataError(f"{metadata_path}: synthetic fixture_id prefix is required")
        if publication != "public":
            raise MetadataError(f"{metadata_path}: synthetic fixtures in this repository are public")
        if license_info["license_id"] not in {"CC0-1.0", "Apache-2.0"}:
            raise MetadataError(f"{metadata_path}: synthetic fixtures require CC0-1.0 or Apache-2.0")
        if provenance["source_kind"] != "generated":
            raise MetadataError(f"{metadata_path}: synthetic fixtures require generated provenance")
        if pii["classification"] != "synthetic" or pii["redaction_status"] != "not-required":
            raise MetadataError(f"{metadata_path}: synthetic fixtures require synthetic/not-required PII fields")

    if partition == "licensed-real":
        if "evidence_path" not in license_info:
            raise MetadataError(f"{metadata_path}: licensed-real fixtures require license.evidence_path")
        if provenance["source_kind"] not in {"licensed-real", "official-sample"}:
            raise MetadataError(f"{metadata_path}: licensed-real fixtures require real-source provenance")
        if pii["redaction_status"] not in {"redacted", "not-required"}:
            raise MetadataError(f"{metadata_path}: licensed-real fixtures require redacted or not-required PII")

    if partition == "private-regression":
        if publication != "private":
            raise MetadataError(f"{metadata_path}: private-regression fixtures must be private")
        if license_info["license_id"] != "PRIVATE-REGRESSION":
            raise MetadataError(f"{metadata_path}: private-regression fixtures require PRIVATE-REGRESSION")
        if license_info["redistribution"] == "public-ok":
            raise MetadataError(f"{metadata_path}: private-regression fixtures are not public redistributable")
        if provenance["source_kind"] != "customer-private":
            raise MetadataError(f"{metadata_path}: private-regression fixtures require customer-private provenance")

    reviewed = _parse_date(metadata["maintenance"]["reviewed_at"], f"{metadata_path}: reviewed_at")
    review_due = _parse_date(metadata["maintenance"]["review_due"], f"{metadata_path}: review_due")
    if review_due <= reviewed:
        raise MetadataError(f"{metadata_path}: review_due must be after reviewed_at")


def validate_metadata_file(metadata_path: Path, schema: dict[str, Any]) -> None:
    metadata = load_json(metadata_path)
    validate_instance(metadata, schema)
    validate_artifact(metadata, metadata_path)
    validate_policy_semantics(metadata, metadata_path)


def iter_metadata_files(corpus_root: Path) -> list[Path]:
    return sorted(path for path in corpus_root.rglob("metadata.json") if path.is_file())


def validate_all(corpus_root: Path = CORPUS_ROOT, schema_path: Path = SCHEMA_PATH) -> list[Path]:
    schema = load_json(schema_path)
    metadata_files = iter_metadata_files(corpus_root)
    if not metadata_files:
        raise MetadataError(f"{corpus_root}: no metadata.json files found")
    for metadata_path in metadata_files:
        validate_metadata_file(metadata_path, schema)
    return metadata_files


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-root", type=Path, default=CORPUS_ROOT)
    parser.add_argument("--schema", type=Path, default=SCHEMA_PATH)
    args = parser.parse_args(argv)

    try:
        metadata_files = validate_all(args.corpus_root, args.schema)
    except MetadataError as exc:
        print(f"fixture metadata validation failed: {exc}", file=sys.stderr)
        return 1

    print(f"validated {len(metadata_files)} fixture metadata file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
