#!/usr/bin/env python3
"""InvoiceKit compliance source-watch bot.

The bot reads a signed source registry, fetches every configured source,
compares the response against a persisted state file, and opens a structured
follow-up when a source changes. It intentionally uses the standard library so
the scheduled GitHub Action can run without a dependency install step.
"""

from __future__ import annotations

import argparse
import copy
import dataclasses
from datetime import datetime, timezone
import difflib
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tomllib
from typing import Any
from urllib.parse import unquote, urlparse
from urllib.request import Request, urlopen

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REGISTRY = REPO_ROOT / "data" / "sources" / "official.toml"
DEFAULT_STATE = REPO_ROOT / ".source-watch-state.json"
USER_AGENT = "InvoiceKit-source-watch/0.1 (+https://github.com/MuhDur/invoicekit)"
MAX_FETCH_BYTES = 20 * 1024 * 1024
MAX_STORED_TEXT_BYTES = 200_000
MAX_DIFF_LINES = 160
REGISTRY_SIGNATURE_ALG = "sha256:identity"


class SourceWatchError(Exception):
    """Base error for source-watch failures."""


class RegistryError(SourceWatchError):
    """The source registry is malformed or its signature is invalid."""


class FetchError(SourceWatchError):
    """A configured source could not be fetched."""


@dataclasses.dataclass(frozen=True)
class Source:
    """One monitored compliance source."""

    id: str
    name: str
    jurisdiction: str
    url: str
    cadence: str
    kind: str
    confidence: str
    proposed_action: str
    headers: dict[str, str]


@dataclasses.dataclass(frozen=True)
class Snapshot:
    """Persisted observation for one source at one point in time."""

    sha256: str
    checked_at: str
    url: str
    status: int | None
    etag: str | None
    last_modified: str | None
    content_type: str | None
    text: str | None

    def to_json(self) -> dict[str, Any]:
        """Return a JSON-serializable snapshot."""
        payload: dict[str, Any] = dataclasses.asdict(self)
        if self.text is None:
            payload.pop("text")
        return payload


@dataclasses.dataclass(frozen=True)
class Issue:
    """Structured issue emitted by a change detector."""

    marker: str
    title: str
    body: str
    source_id: str
    sha256: str

    def to_json(self) -> dict[str, str]:
        """Return a JSON-serializable issue record."""
        return dataclasses.asdict(self)


class IssueSink:
    """Destination for source-change issues."""

    def already_open(self, issue: Issue) -> bool:
        """Return true when this issue has already been opened."""
        return False

    def open_issue(self, issue: Issue) -> dict[str, Any]:
        """Open an issue and return backend-specific metadata."""
        raise NotImplementedError


class DryRunSink(IssueSink):
    """Issue sink that records intended writes in the run summary only."""

    def open_issue(self, issue: Issue) -> dict[str, Any]:
        return {"backend": "dry-run", "marker": issue.marker}


class LocalJsonlSink(IssueSink):
    """Issue sink used by tests and local dry-run workflows."""

    def __init__(self, path: Path) -> None:
        self.path = path

    def already_open(self, issue: Issue) -> bool:
        if not self.path.exists():
            return False
        for line in self.path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get("marker") == issue.marker:
                return True
        return False

    def open_issue(self, issue: Issue) -> dict[str, Any]:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(issue.to_json(), sort_keys=True) + "\n")
        return {"backend": "local-jsonl", "path": str(self.path), "marker": issue.marker}


class GitHubIssueSink(IssueSink):
    """Issue sink backed by the GitHub CLI."""

    def __init__(self, repo: str | None, labels: tuple[str, ...]) -> None:
        self.repo = repo
        self.labels = labels

    def _base_cmd(self) -> list[str]:
        cmd = ["gh"]
        if self.repo:
            cmd.extend(["--repo", self.repo])
        return cmd

    def already_open(self, issue: Issue) -> bool:
        marker_token = issue.marker.replace("<!-- ", "").replace(" -->", "")
        cmd = [
            *self._base_cmd(),
            "issue",
            "list",
            "--state",
            "open",
            "--search",
            marker_token,
            "--json",
            "title,body",
            "--limit",
            "20",
        ]
        completed = subprocess.run(  # nosec B603
            cmd,
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        if completed.returncode != 0:
            return False
        try:
            issues = json.loads(completed.stdout)
        except json.JSONDecodeError:
            return False
        return any(issue.marker in item.get("body", "") for item in issues)

    def open_issue(self, issue: Issue) -> dict[str, Any]:
        cmd = [*self._base_cmd(), "issue", "create", "--title", issue.title, "--body", issue.body]
        for label in self.labels:
            cmd.extend(["--label", label])
        completed = subprocess.run(  # nosec B603
            cmd,
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        if completed.returncode != 0:
            raise SourceWatchError(completed.stderr.strip() or "gh issue create failed")
        return {"backend": "github", "url": completed.stdout.strip(), "marker": issue.marker}


class BeadSink(IssueSink):
    """Issue sink backed by Beads (`br`)."""

    def already_open(self, issue: Issue) -> bool:
        completed = subprocess.run(  # nosec B603,B607
            ["br", "list", "--json"],
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        if completed.returncode != 0:
            return False
        try:
            issues = json.loads(completed.stdout)
        except json.JSONDecodeError:
            return False
        return any(issue.marker in item.get("description", "") for item in issues)

    def open_issue(self, issue: Issue) -> dict[str, Any]:
        completed = subprocess.run(  # nosec B603,B607
            [
                "br",
                "create",
                issue.title,
                "-t",
                "task",
                "-p",
                "1",
                "-d",
                issue.body,
                "-l",
                "source-watch,compliance",
                "--json",
            ],
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
        if completed.returncode != 0:
            raise SourceWatchError(completed.stderr.strip() or "br create failed")
        try:
            payload = json.loads(completed.stdout)
        except json.JSONDecodeError:
            payload = {"stdout": completed.stdout.strip()}
        return {"backend": "bead", "payload": payload, "marker": issue.marker}


def utc_now() -> str:
    """Return an RFC 3339 UTC timestamp with second precision."""
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def canonical_registry_payload(registry: dict[str, Any]) -> bytes:
    """Return the canonical bytes covered by the registry signature."""
    payload = copy.deepcopy(registry)
    metadata = dict(payload.get("registry", {}))
    metadata.pop("signature", None)
    payload["registry"] = metadata
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def expected_registry_signature(registry: dict[str, Any]) -> str:
    """Return the expected `sha256:identity` registry signature."""
    return hashlib.sha256(canonical_registry_payload(registry)).hexdigest()


def load_registry(path: Path) -> tuple[dict[str, Any], list[Source]]:
    """Load, validate, and verify a source registry.

    Raises:
        RegistryError: if the registry is malformed or unsigned.
    """
    try:
        registry = tomllib.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise RegistryError(f"source registry read failed: {path}: {exc}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise RegistryError(f"source registry TOML is invalid: {path}: {exc}") from exc

    metadata = registry.get("registry")
    if not isinstance(metadata, dict):
        raise RegistryError("source registry missing [registry] metadata")
    alg = metadata.get("signature_alg")
    if alg != REGISTRY_SIGNATURE_ALG:
        raise RegistryError(f"source registry uses unsupported signature_alg `{alg}`")
    actual = metadata.get("signature")
    expected = expected_registry_signature(registry)
    if actual != expected:
        raise RegistryError(f"source registry signature mismatch: expected {expected}, got {actual}")

    raw_sources = registry.get("sources")
    if not isinstance(raw_sources, list) or not raw_sources:
        raise RegistryError("source registry must contain at least one [[sources]] entry")

    seen: set[str] = set()
    sources: list[Source] = []
    for raw in raw_sources:
        if not isinstance(raw, dict):
            raise RegistryError("source registry contains a non-table source entry")
        source = source_from_raw(raw)
        if source.id in seen:
            raise RegistryError(f"duplicate source id `{source.id}`")
        seen.add(source.id)
        sources.append(source)
    return registry, sources


def source_from_raw(raw: dict[str, Any]) -> Source:
    """Convert a TOML source table into a typed source."""
    required = (
        "id",
        "name",
        "jurisdiction",
        "url",
        "cadence",
        "kind",
        "confidence",
        "proposed_action",
    )
    missing = [key for key in required if not isinstance(raw.get(key), str) or not raw.get(key)]
    if missing:
        raise RegistryError(f"source entry missing required string field(s): {', '.join(missing)}")
    if raw["cadence"] != "daily":
        raise RegistryError(f"source `{raw['id']}` must use daily cadence for T-006")
    if raw["confidence"] not in {"official-source", "partner-source", "community"}:
        raise RegistryError(f"source `{raw['id']}` uses unknown confidence `{raw['confidence']}`")

    headers = raw.get("headers", {})
    if not isinstance(headers, dict):
        raise RegistryError(f"source `{raw['id']}` has non-table headers")
    clean_headers: dict[str, str] = {}
    for key, value in headers.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise RegistryError(f"source `{raw['id']}` headers must be strings")
        clean_headers[key] = value

    return Source(
        id=raw["id"],
        name=raw["name"],
        jurisdiction=raw["jurisdiction"],
        url=raw["url"],
        cadence=raw["cadence"],
        kind=raw["kind"],
        confidence=raw["confidence"],
        proposed_action=raw["proposed_action"],
        headers=clean_headers,
    )


def load_state(path: Path) -> dict[str, Any]:
    """Load the persisted source-watch state."""
    if not path.exists():
        return {"version": 1, "sources": {}}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SourceWatchError(f"state file is invalid: {path}: {exc}") from exc
    if not isinstance(payload, dict):
        raise SourceWatchError(f"state file must contain a JSON object: {path}")
    payload.setdefault("version", 1)
    payload.setdefault("sources", {})
    if not isinstance(payload["sources"], dict):
        raise SourceWatchError(f"state file sources must be a JSON object: {path}")
    return payload


def save_state(path: Path, state: dict[str, Any]) -> None:
    """Persist source-watch state."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def fetch_source(source: Source, timeout_seconds: float) -> Snapshot:
    """Fetch one source and return a snapshot."""
    parsed = urlparse(source.url)
    checked_at = utc_now()
    if parsed.scheme == "file":
        local_path = Path(unquote(parsed.path))
        try:
            body = local_path.read_bytes()
        except OSError as exc:
            raise FetchError(f"{source.id}: file source read failed: {local_path}: {exc}") from exc
        headers: dict[str, str] = {}
        status = None
    else:
        if parsed.scheme not in {"http", "https"}:
            raise FetchError(f"{source.id}: unsupported source URL scheme `{parsed.scheme}`")
        request_headers = {"User-Agent": USER_AGENT, **source.headers}
        request = Request(source.url, headers=request_headers)
        try:
            with urlopen(request, timeout=timeout_seconds) as response:  # nosec B310
                body = response.read(MAX_FETCH_BYTES + 1)
                headers = {key.lower(): value for key, value in response.headers.items()}
                status = getattr(response, "status", None)
        except OSError as exc:
            raise FetchError(f"{source.id}: fetch failed: {exc}") from exc
        if len(body) > MAX_FETCH_BYTES:
            raise FetchError(f"{source.id}: response exceeds {MAX_FETCH_BYTES} bytes")

    content_type = headers.get("content-type")
    text = text_snapshot(body, source.url, content_type)
    return Snapshot(
        sha256=hashlib.sha256(body).hexdigest(),
        checked_at=checked_at,
        url=source.url,
        status=status,
        etag=headers.get("etag"),
        last_modified=headers.get("last-modified"),
        content_type=content_type,
        text=text,
    )


def text_snapshot(body: bytes, url: str, content_type: str | None) -> str | None:
    """Return a bounded text snapshot when a response appears text-like."""
    parsed = urlparse(url)
    suffix = Path(parsed.path).suffix.lower()
    is_text_content_type = content_type is not None and (
        content_type.startswith("text/")
        or "json" in content_type
        or "xml" in content_type
        or "yaml" in content_type
    )
    is_text_suffix = suffix in {".atom", ".csv", ".htm", ".html", ".json", ".rss", ".txt", ".xml"}
    if b"\0" in body[:1024] or not (is_text_content_type or is_text_suffix or not suffix):
        return None
    return body[:MAX_STORED_TEXT_BYTES].decode("utf-8", errors="replace")


def build_issue(source: Source, previous: dict[str, Any], current: Snapshot) -> Issue:
    """Build a structured issue for a detected source change."""
    short_hash = current.sha256[:12]
    marker = f"<!-- invoicekit-source-watch:{source.id}:{current.sha256} -->"
    title = f"Source-watch: {source.name} changed ({short_hash})"
    previous_sha = str(previous.get("sha256", "unknown"))
    previous_checked = str(previous.get("checked_at", "unknown"))
    diff = diff_preview(previous.get("text"), current.text, source.id, previous_sha, current.sha256)
    body = "\n".join(
        [
            marker,
            "",
            "## Source change detected",
            "",
            f"- Source ID: `{source.id}`",
            f"- Name: {source.name}",
            f"- Jurisdiction: `{source.jurisdiction}`",
            f"- Kind: `{source.kind}`",
            f"- Confidence: `{source.confidence}`",
            f"- URL: {source.url}",
            f"- Previous SHA-256: `{previous_sha}`",
            f"- Current SHA-256: `{current.sha256}`",
            f"- Previous checked at: `{previous_checked}`",
            f"- Current checked at: `{current.checked_at}`",
            f"- Previous ETag: `{previous.get('etag')}`",
            f"- Current ETag: `{current.etag}`",
            f"- Previous Last-Modified: `{previous.get('last_modified')}`",
            f"- Current Last-Modified: `{current.last_modified}`",
            "",
            "## Diff",
            "",
            diff,
            "",
            "## Proposed action",
            "",
            source.proposed_action,
        ]
    )
    return Issue(marker=marker, title=title, body=body, source_id=source.id, sha256=current.sha256)


def diff_preview(
    previous_text: Any,
    current_text: str | None,
    source_id: str,
    previous_sha: str,
    current_sha: str,
) -> str:
    """Return a bounded unified diff for issue bodies."""
    if not isinstance(previous_text, str) or current_text is None:
        return "Text diff unavailable; one side is binary, non-text, or outside the stored text limit."
    lines = list(
        difflib.unified_diff(
            previous_text.splitlines(),
            current_text.splitlines(),
            fromfile=f"{source_id}@{previous_sha[:12]}",
            tofile=f"{source_id}@{current_sha[:12]}",
            lineterm="",
        )
    )
    if not lines:
        return "Text body was unchanged; binary bytes, headers, or truncation boundary changed."
    if len(lines) > MAX_DIFF_LINES:
        lines = lines[:MAX_DIFF_LINES] + [f"... diff truncated after {MAX_DIFF_LINES} lines ..."]
    return "```diff\n" + "\n".join(lines) + "\n```"


def sink_from_args(args: argparse.Namespace) -> IssueSink:
    """Create the requested issue sink."""
    if args.issue_backend == "dry-run":
        return DryRunSink()
    if args.issue_backend == "local-jsonl":
        return LocalJsonlSink(args.local_issues)
    if args.issue_backend == "github":
        return GitHubIssueSink(args.github_repo, tuple(args.github_label))
    if args.issue_backend == "bead":
        return BeadSink()
    raise SourceWatchError(f"unsupported issue backend: {args.issue_backend}")


def run_once(
    registry_path: Path,
    state_path: Path,
    sink: IssueSink,
    timeout_seconds: float,
    *,
    open_on_new: bool = False,
) -> dict[str, Any]:
    """Run one source-watch pass and return a structured summary."""
    _, sources = load_registry(registry_path)
    state = load_state(state_path)
    state_sources: dict[str, Any] = state["sources"]
    events: list[dict[str, Any]] = []
    opened: list[dict[str, Any]] = []

    for source in sources:
        try:
            current = fetch_source(source, timeout_seconds)
        except FetchError as exc:
            events.append({"source_id": source.id, "status": "fetch-error", "error": str(exc)})
            continue

        previous = state_sources.get(source.id)
        status = "baseline"
        if isinstance(previous, dict):
            if previous.get("sha256") != current.sha256:
                issue = build_issue(source, previous, current)
                if sink.already_open(issue):
                    status = "changed-duplicate"
                else:
                    opened.append(sink.open_issue(issue))
                    status = "changed-opened"
            else:
                status = "unchanged"
        elif open_on_new:
            issue = build_issue(source, {}, current)
            if not sink.already_open(issue):
                opened.append(sink.open_issue(issue))
                status = "new-opened"

        state_sources[source.id] = current.to_json()
        events.append(
            {
                "source_id": source.id,
                "status": status,
                "sha256": current.sha256,
                "checked_at": current.checked_at,
            }
        )

    save_state(state_path, state)
    return {
        "registry": str(registry_path),
        "state": str(state_path),
        "checked": len(sources),
        "events": events,
        "opened": opened,
    }


def build_parser() -> argparse.ArgumentParser:
    """Build the command-line parser."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subcommands = parser.add_subparsers(dest="command", required=True)

    verify = subcommands.add_parser("verify", help="verify the signed source registry")
    verify.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)

    sign = subcommands.add_parser("sign-registry", help="print the expected registry signature")
    sign.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)

    run = subcommands.add_parser("run", help="fetch sources and open issues on changes")
    run.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    run.add_argument("--state", type=Path, default=DEFAULT_STATE)
    run.add_argument(
        "--issue-backend",
        choices=("dry-run", "local-jsonl", "github", "bead"),
        default="dry-run",
    )
    run.add_argument("--local-issues", type=Path, default=Path("source-watch-issues.jsonl"))
    run.add_argument("--github-repo", default=None)
    run.add_argument("--github-label", action="append", default=[])
    run.add_argument("--open-on-new", action="store_true")
    run.add_argument("--timeout-seconds", type=float, default=30.0)

    return parser


def main(argv: list[str] | None = None) -> int:
    """CLI entry point."""
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.command == "verify":
            _, sources = load_registry(args.registry)
            print(f"source-watch: registry verified ({len(sources)} sources)")
            return 0
        if args.command == "sign-registry":
            registry = tomllib.loads(args.registry.read_text(encoding="utf-8"))
            print(expected_registry_signature(registry))
            return 0
        if args.command == "run":
            summary = run_once(
                registry_path=args.registry,
                state_path=args.state,
                sink=sink_from_args(args),
                timeout_seconds=args.timeout_seconds,
                open_on_new=args.open_on_new,
            )
            print(json.dumps(summary, indent=2, sort_keys=True))
            return 0
    except SourceWatchError as exc:
        print(f"source-watch: {exc}", file=sys.stderr)
        return 1
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
