#!/usr/bin/env python3
"""T-138: build the InvoiceKit public status page.

Reads `status.toml` (the operator-edited site config) and `incidents/`
(append-only Markdown ledger of every recorded incident) and emits a
single-file HTML page suitable for hosting at `status.invoicekit.org`.

The build is deterministic — same inputs produce byte-identical output —
so the generated bundle is safe to publish via any static-hosting
target (GitHub Pages, Cloudflare Pages, S3+CloudFront).

Layout
------

Per-country and per-gateway uptime is computed from the `incidents/`
ledger:

    uptime(target, window_days) =
      max(0, 1 - sum_of_incident_minutes(target, window) /
              (window_days * 24 * 60))

A `target` is either a country alpha-2 (`DE`, `IT`) or a gateway slug
(`peppol-storecove`, `peppol-ecosio`). The site config declares which
of each to surface in which column.

Usage
-----

    python3 tools/status-page/build_status_page.py \\
        --config tools/status-page/status.toml \\
        --incidents-dir tools/status-page/incidents \\
        --out target/status-page/index.html

Exit codes
----------

* 0 — every section rendered cleanly.
* 2 — invalid input (missing files, malformed TOML, malformed incident).
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import html
import re
import sys
from pathlib import Path

try:  # Python 3.11+ ships tomllib.
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore[no-redef]


EXIT_OK = 0
EXIT_INVALID_INPUT = 2

INCIDENT_FILENAME_RE = re.compile(
    r"^(?P<date>\d{4}-\d{2}-\d{2})-(?P<slug>[a-z0-9-]+)\.md$"
)
INCIDENT_HEADER_RE = re.compile(
    r"^---\s*\n"
    r"(?P<body>.*?)\n"
    r"^---\s*\n",
    re.MULTILINE | re.DOTALL,
)


@dataclasses.dataclass(frozen=True)
class Incident:
    date: dt.date
    slug: str
    target: str
    title: str
    minutes: int
    status: str  # one of: resolved, monitoring, investigating
    notes: str


class StatusError(ValueError):
    """Invalid status-page input."""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--incidents-dir", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)

    try:
        config = load_config(args.config)
        incidents = load_incidents(args.incidents_dir)
        html_doc = render_html(config, incidents)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(html_doc, encoding="utf-8")
    except StatusError as exc:
        print(str(exc), file=sys.stderr)
        return EXIT_INVALID_INPUT
    return EXIT_OK


def load_config(path: Path) -> dict:
    try:
        with path.open("rb") as handle:
            data = tomllib.load(handle)
    except FileNotFoundError as exc:
        raise StatusError(f"status config not found: {exc.filename}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise StatusError(f"status config is not valid TOML: {exc}") from exc

    for required in ("site", "targets"):
        if required not in data:
            raise StatusError(f"status config missing [{required}]")
    targets = data.get("targets", {})
    if not isinstance(targets, dict):
        raise StatusError("status config: [targets] must be a table")
    for key, value in targets.items():
        if not isinstance(value, dict):
            raise StatusError(f"status config: targets.{key} must be a table")
    return data


def load_incidents(directory: Path) -> list[Incident]:
    if not directory.is_dir():
        return []
    out: list[Incident] = []
    for path in sorted(directory.glob("*.md")):
        name = path.name
        match = INCIDENT_FILENAME_RE.match(name)
        if not match:
            raise StatusError(f"incident filename does not match convention: {name}")
        date = dt.date.fromisoformat(match.group("date"))
        slug = match.group("slug")
        text = path.read_text(encoding="utf-8")
        header_match = INCIDENT_HEADER_RE.match(text)
        if not header_match:
            raise StatusError(f"{name}: missing YAML front-matter header")
        header = parse_simple_frontmatter(header_match.group("body"), origin=name)
        for required_key in ("target", "title", "minutes", "status"):
            if required_key not in header:
                raise StatusError(f"{name}: incident header missing `{required_key}`")
        try:
            minutes = int(header["minutes"])
        except (TypeError, ValueError) as exc:
            raise StatusError(f"{name}: incident `minutes` must be an integer") from exc
        if minutes < 0:
            raise StatusError(f"{name}: incident `minutes` must be >= 0")
        notes = text[header_match.end():].strip()
        out.append(
            Incident(
                date=date,
                slug=slug,
                target=header["target"],
                title=header["title"],
                minutes=minutes,
                status=header["status"],
                notes=notes,
            )
        )
    return out


def parse_simple_frontmatter(body: str, *, origin: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if ":" not in line:
            raise StatusError(f"{origin}: front-matter line lacks `:`: {raw_line!r}")
        key, _, value = line.partition(":")
        out[key.strip()] = value.strip().strip('"')
    return out


def render_html(config: dict, incidents: list[Incident]) -> str:
    site = config["site"]
    targets = config["targets"]
    window_days = int(site.get("window_days", 90))
    cutoff = dt.date.today() - dt.timedelta(days=window_days)
    in_window = [i for i in incidents if i.date >= cutoff]

    rows: list[str] = []
    for target_key, target_meta in targets.items():
        kind = str(target_meta.get("kind", "gateway"))
        label = html.escape(str(target_meta.get("label", target_key)))
        target_incidents = [i for i in in_window if i.target == target_key]
        total_minutes = sum(i.minutes for i in target_incidents)
        window_minutes = window_days * 24 * 60
        uptime_pct = max(0.0, 100.0 - (total_minutes / window_minutes * 100.0))
        incident_count = len(target_incidents)
        rows.append(
            f"<tr><td>{label} <span class=kind>({html.escape(kind)})</span></td>"
            f"<td>{uptime_pct:.2f}%</td>"
            f"<td>{incident_count}</td>"
            f"<td>{total_minutes} min</td></tr>"
        )

    incident_blocks: list[str] = []
    for incident in sorted(in_window, key=lambda i: (i.date, i.slug), reverse=True):
        incident_blocks.append(
            "<article class=incident>"
            f"<h3>{html.escape(incident.date.isoformat())} — {html.escape(incident.title)}</h3>"
            f"<p class=meta>target=<code>{html.escape(incident.target)}</code>"
            f" status=<code>{html.escape(incident.status)}</code>"
            f" duration=<code>{incident.minutes} min</code></p>"
            f"<p>{html.escape(incident.notes)}</p>"
            "</article>"
        )
    incident_html = "\n".join(incident_blocks) or "<p>No incidents in the rolling window.</p>"

    site_title = html.escape(str(site.get("title", "InvoiceKit Status")))
    site_url = html.escape(str(site.get("url", "https://status.invoicekit.org")))
    digest = hashlib.sha256(
        (site_title + site_url + "".join(rows) + incident_html).encode("utf-8")
    ).hexdigest()[:12]

    return (
        "<!doctype html>\n"
        "<html lang=en>\n"
        "<head>\n"
        f"  <meta charset=utf-8>\n"
        f"  <title>{site_title}</title>\n"
        f"  <link rel=canonical href=\"{site_url}\">\n"
        "  <meta name=robots content=\"index,follow\">\n"
        "  <style>"
        "body{font-family:system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;color:#111}"
        "h1{font-size:1.6rem;margin-bottom:0.2rem}"
        "table{width:100%;border-collapse:collapse;margin:1.5rem 0}"
        "th,td{padding:0.4rem 0.6rem;text-align:left;border-bottom:1px solid #eee}"
        ".kind{color:#888;font-weight:400}"
        ".incident{border:1px solid #eee;border-radius:6px;padding:0.8rem;margin:1rem 0}"
        ".incident .meta{color:#666;font-size:0.85rem}"
        "code{background:#f6f6f6;padding:0.05rem 0.3rem;border-radius:3px}"
        ".footer{color:#888;font-size:0.8rem;margin-top:2rem}"
        "</style>\n"
        "</head>\n"
        "<body>\n"
        f"  <h1>{site_title}</h1>\n"
        f"  <p>Rolling {window_days}-day uptime per country / gateway.</p>\n"
        "  <table>\n"
        "    <thead><tr><th>Target</th><th>Uptime</th><th>Incidents</th><th>Downtime</th></tr></thead>\n"
        f"    <tbody>{''.join(rows)}</tbody>\n"
        "  </table>\n"
        f"  <h2>Incidents (last {window_days} days)</h2>\n"
        f"  {incident_html}\n"
        f"  <p class=footer>Build digest <code>{digest}</code> — regenerated on every push to <code>tools/status-page/</code>.</p>\n"
        "</body>\n"
        "</html>\n"
    )


if __name__ == "__main__":
    raise SystemExit(main())
