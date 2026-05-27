"""Tests for the compliance source-watch bot."""

from __future__ import annotations

import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[3]
TOOL = REPO / "tools" / "source-watch-bot"
sys.path.insert(0, str(TOOL))
import source_watch  # noqa: E402


def write_registry(path: Path, source_url: str, *, signature: str | None = None) -> None:
    """Write a one-source signed registry for tests."""
    registry = {
        "registry": {
            "version": "test",
            "generated_at": "2026-05-27T00:00:00Z",
            "signature_alg": source_watch.REGISTRY_SIGNATURE_ALG,
            "signature": "",
        },
        "sources": [
            {
                "id": "test-source",
                "name": "Mock regulator page",
                "jurisdiction": "ZZ",
                "url": source_url,
                "cadence": "daily",
                "kind": "rulepack",
                "confidence": "official-source",
                "proposed_action": "Review the mock source and update the rule pack fixture.",
                "headers": {"Accept": "text/plain"},
            }
        ],
    }
    registry["registry"]["signature"] = signature or source_watch.expected_registry_signature(registry)
    path.write_text(
        "\n".join(
            [
                "[registry]",
                'version = "test"',
                'generated_at = "2026-05-27T00:00:00Z"',
                f'signature_alg = "{source_watch.REGISTRY_SIGNATURE_ALG}"',
                f'signature = "{registry["registry"]["signature"]}"',
                "",
                "[[sources]]",
                'id = "test-source"',
                'name = "Mock regulator page"',
                'jurisdiction = "ZZ"',
                f'url = "{source_url}"',
                'cadence = "daily"',
                'kind = "rulepack"',
                'confidence = "official-source"',
                'proposed_action = "Review the mock source and update the rule pack fixture."',
                'headers = { Accept = "text/plain" }',
                "",
            ]
        ),
        encoding="utf-8",
    )


def records(path: Path) -> list[dict[str, str]]:
    """Read JSONL records from a local issue sink."""
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def test_official_registry_signature_verifies() -> None:
    """The committed source registry is signed and includes the required seed set."""
    _, sources = source_watch.load_registry(REPO / "data" / "sources" / "official.toml")
    source_ids = {source.id for source in sources}
    assert {
        "de-kosit-xrechnung",
        "it-agenzia-fatturapa",
        "pl-ksef-api",
        "sa-zatca-einvoicing",
        "in-gst-irp",
        "sg-imda-invoicenow",
        "au-ato-einvoicing",
    }.issubset(source_ids)


def test_bad_registry_signature_is_rejected(tmp_path: Path) -> None:
    """A tampered registry cannot be used by the bot."""
    registry = tmp_path / "sources.toml"
    source = tmp_path / "source.txt"
    source.write_text("version one\n", encoding="utf-8")
    write_registry(registry, source.as_uri(), signature="bad")
    try:
        source_watch.load_registry(registry)
    except source_watch.RegistryError as exc:
        assert "signature mismatch" in str(exc)
    else:
        raise AssertionError("bad registry signature should fail verification")


def test_mock_source_change_opens_structured_issue(tmp_path: Path) -> None:
    """End-to-end: baseline a mock source, mutate it, and emit one issue record."""
    source = tmp_path / "source.txt"
    source.write_text("rule version: 1\nunchanged line\n", encoding="utf-8")
    registry = tmp_path / "sources.toml"
    write_registry(registry, source.as_uri())
    state = tmp_path / "state.json"
    issues = tmp_path / "issues.jsonl"

    first = source_watch.run_once(
        registry_path=registry,
        state_path=state,
        sink=source_watch.LocalJsonlSink(issues),
        timeout_seconds=1.0,
    )
    assert first["opened"] == []
    assert records(issues) == []

    source.write_text("rule version: 2\nunchanged line\n", encoding="utf-8")
    second = source_watch.run_once(
        registry_path=registry,
        state_path=state,
        sink=source_watch.LocalJsonlSink(issues),
        timeout_seconds=1.0,
    )
    assert second["events"][0]["status"] == "changed-opened"
    opened = records(issues)
    assert len(opened) == 1
    assert "Previous SHA-256" in opened[0]["body"]
    assert "-rule version: 1" in opened[0]["body"]
    assert "+rule version: 2" in opened[0]["body"]
    assert "Review the mock source" in opened[0]["body"]


def test_repeated_change_hash_does_not_duplicate_issue(tmp_path: Path) -> None:
    """The same current hash is not reported twice."""
    source = tmp_path / "source.txt"
    source.write_text("one\n", encoding="utf-8")
    registry = tmp_path / "sources.toml"
    write_registry(registry, source.as_uri())
    state = tmp_path / "state.json"
    issues = tmp_path / "issues.jsonl"
    sink = source_watch.LocalJsonlSink(issues)

    source_watch.run_once(registry, state, sink, 1.0)
    source.write_text("two\n", encoding="utf-8")
    source_watch.run_once(registry, state, sink, 1.0)

    saved_state = json.loads(state.read_text(encoding="utf-8"))
    saved_state["sources"]["test-source"]["sha256"] = "old-hash"
    state.write_text(json.dumps(saved_state), encoding="utf-8")
    source_watch.run_once(registry, state, sink, 1.0)

    assert len(records(issues)) == 1


def test_fetch_error_is_reported_without_opening_issue(tmp_path: Path) -> None:
    """An unavailable source is surfaced in the summary instead of panicking."""
    registry = tmp_path / "sources.toml"
    write_registry(registry, (tmp_path / "missing.txt").as_uri())
    result = source_watch.run_once(
        registry_path=registry,
        state_path=tmp_path / "state.json",
        sink=source_watch.DryRunSink(),
        timeout_seconds=1.0,
    )
    assert result["events"][0]["status"] == "fetch-error"
    assert result["opened"] == []


def test_source_watch_workflow_is_daily_and_runs_tests() -> None:
    """The GitHub Action deploys the daily bot and keeps test coverage wired."""
    workflow = (REPO / ".github" / "workflows" / "source-watch.yml").read_text(encoding="utf-8")
    assert "cron: \"17 3 * * *\"" in workflow
    assert "python3 tools/source-watch-bot/source_watch.py verify" in workflow
    assert "pytest tools/source-watch-bot/tests -q" in workflow
    assert "python3 tools/source-watch-bot/source_watch.py run" in workflow
    assert "issues: write" in workflow
