from __future__ import annotations

import copy
import json
import shutil
import sys
from pathlib import Path

import pytest


REPO = Path(__file__).resolve().parents[3]
TOOL = REPO / "tools" / "conformance-corpus"
sys.path.insert(0, str(TOOL))
import validate_fixture_metadata as validator  # noqa: E402


def sample_metadata() -> dict[str, object]:
    path = (
        REPO
        / "conformance-corpus"
        / "synthetic"
        / "examples"
        / "ubl-invoice-basic"
        / "metadata.json"
    )
    return validator.load_json(path)


def test_committed_sample_metadata_validates() -> None:
    metadata_files = validator.validate_all()
    if len(metadata_files) < 4:
        raise AssertionError(f"expected at least 4 sample metadata files, got {metadata_files}")


def test_artifact_without_sibling_metadata_is_rejected(tmp_path: Path) -> None:
    fixture_dir = tmp_path / "synthetic" / "examples" / "missing-metadata"
    fixture_dir.mkdir(parents=True)
    (fixture_dir / "fixture.json").write_text("{}", encoding="utf-8")
    metadata_files: list[Path] = []

    with pytest.raises(validator.MetadataError, match="missing sibling metadata"):
        validator.validate_metadata_coverage(tmp_path, metadata_files)


def test_reference_json_is_not_treated_as_fixture_artifact(tmp_path: Path) -> None:
    fixture_dir = tmp_path / "licensed-real" / "examples" / "redacted"
    fixture_dir.mkdir(parents=True)
    (fixture_dir / "fixture.xml").write_text("<Invoice />", encoding="utf-8")
    (fixture_dir / "redaction-report.json").write_text("{}", encoding="utf-8")

    metadata = copy.deepcopy(sample_metadata())
    metadata["artifact"]["path"] = "fixture.xml"
    metadata["pii"]["redaction_report_path"] = "redaction-report.json"
    metadata_path = fixture_dir / "metadata.json"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    validator.validate_metadata_coverage(tmp_path, [metadata_path])


def test_unknown_top_level_property_is_rejected() -> None:
    schema = validator.load_json(validator.SCHEMA_PATH)
    metadata = sample_metadata()
    metadata["unexpected"] = True

    with pytest.raises(validator.MetadataError, match="unexpected properties"):
        validator.validate_instance(metadata, schema)


def test_public_fixture_with_personal_data_is_rejected() -> None:
    metadata = sample_metadata()
    metadata["pii"] = copy.deepcopy(metadata["pii"])
    metadata["pii"]["contains_personal_data"] = True

    with pytest.raises(validator.MetadataError, match="must not contain personal data"):
        validator.validate_policy_semantics(metadata, Path("metadata.json"))


def test_licensed_real_fixture_requires_license_evidence() -> None:
    metadata = sample_metadata()
    metadata["fixture_id"] = "ik-licensed-real-sample-0001"
    metadata["corpus_partition"] = "licensed-real"
    metadata["provenance"] = copy.deepcopy(metadata["provenance"])
    metadata["provenance"]["source_kind"] = "official-sample"
    metadata["pii"] = copy.deepcopy(metadata["pii"])
    metadata["pii"]["classification"] = "redacted-real"
    metadata["pii"]["redaction_status"] = "redacted"

    with pytest.raises(validator.MetadataError, match="license.evidence_path"):
        validator.validate_policy_semantics(metadata, Path("metadata.json"))


def test_reference_paths_must_stay_inside_fixture_directory() -> None:
    metadata = sample_metadata()
    metadata["license"] = copy.deepcopy(metadata["license"])
    metadata["license"]["evidence_path"] = "../../outside-license.txt"

    with pytest.raises(validator.MetadataError, match="license.evidence_path must stay inside"):
        validator.validate_policy_semantics(metadata, Path("metadata.json"))


def test_redaction_report_path_must_stay_inside_fixture_directory() -> None:
    metadata = sample_metadata()
    metadata["pii"] = copy.deepcopy(metadata["pii"])
    metadata["pii"]["redaction_report_path"] = "../../outside-redaction.md"

    with pytest.raises(validator.MetadataError, match="pii.redaction_report_path must stay inside"):
        validator.validate_policy_semantics(metadata, Path("metadata.json"))


def test_artifact_hash_mismatch_is_rejected(tmp_path: Path) -> None:
    source = (
        REPO
        / "conformance-corpus"
        / "synthetic"
        / "examples"
        / "ubl-invoice-basic"
    )
    fixture_dir = tmp_path / "fixture"
    shutil.copytree(source, fixture_dir)
    artifact = fixture_dir / "fixture.xml"
    artifact.write_text(artifact.read_text(encoding="utf-8") + "\n<!-- changed -->\n", encoding="utf-8")

    metadata = validator.load_json(fixture_dir / "metadata.json")
    metadata["artifact"]["size_bytes"] = artifact.stat().st_size

    with pytest.raises(validator.MetadataError, match="sha256 mismatch"):
        validator.validate_artifact(metadata, fixture_dir / "metadata.json")
