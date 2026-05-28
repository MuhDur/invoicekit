#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Generate MDX pages for the InvoiceKit docs site (T-113).

Reads:

* ``crates/rulepack/data/en16931-br-co-coverage.json`` — EN 16931 rule
  catalogue with business-term mappings, XSLT source locations, and
  validator-testability flags.
* ``crates/report-*/Cargo.toml`` — every country adapter crate. The
  Cargo metadata's ``description`` field becomes the country page
  summary; the path becomes the linked source.

Writes:

* ``apps/docs-site/pages/rules/<RULE-ID>.mdx`` — one page per rule.
* ``apps/docs-site/pages/rules/_meta.json`` — Nextra ordering manifest.
* ``apps/docs-site/pages/countries/<CC>-<adapter>.mdx`` — one page per
  country adapter (multiple per country if more than one report crate
  exists for it).
* ``apps/docs-site/pages/countries/_meta.json`` — Nextra ordering.
* ``apps/docs-site/pages/operators/<NAME>.mdx`` — one page per runbook
  under ``docs/operators/`` so the public site can link them.
* ``apps/docs-site/pages/operators/_meta.json`` — Nextra ordering.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
RULE_DATA = REPO / "crates" / "rulepack" / "data" / "en16931-br-co-coverage.json"
DOCS_OPERATORS = REPO / "docs" / "operators"
REPORT_CRATES_GLOB = "crates/report-*"
SITE_ROOT = REPO / "apps" / "docs-site" / "pages"


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--rule-data",
        type=pathlib.Path,
        default=RULE_DATA,
        help="Path to the EN 16931 rule coverage JSON",
    )
    p.add_argument(
        "--site",
        type=pathlib.Path,
        default=SITE_ROOT,
        help="Path to apps/docs-site/pages",
    )
    return p.parse_args(argv)


def render_rule_mdx(rule: dict) -> str:
    rule_id = rule["id"]
    business_terms = rule.get("business_terms", [])
    business_groups = rule.get("business_groups", [])
    locations = rule.get("source_locations", [])
    testability = rule.get("rust_validator_testability", {})
    current_ir = rule.get("current_ir_paths", [])
    blocker = testability.get("blocker")
    pos = testability.get("positive")
    neg = testability.get("negative")
    parts = [f"# {rule_id}", ""]
    parts.append(
        "EN 16931 business rule. Source data: `crates/rulepack/data/en16931-br-co-coverage.json`."
    )
    parts.append("")
    parts.append("## Business terms")
    if business_terms or business_groups:
        for term in business_terms:
            parts.append(f"- **{term}**")
        for group in business_groups:
            parts.append(f"- **{group}** (group)")
    else:
        parts.append("_None recorded — this rule is cross-cutting._")
    parts.append("")
    parts.append("## Validator testability (pure-Rust)")
    parts.append("")
    parts.append(f"- Positive coverage: **{pos}**")
    parts.append(f"- Negative coverage: **{neg}**")
    if blocker:
        parts.append(f"- Blocker: `{blocker}`")
    parts.append("")
    if current_ir:
        parts.append("## InvoiceKit IR paths exercised")
        parts.append("")
        for ir_path in current_ir:
            parts.append(f"- `{ir_path}`")
        parts.append("")
    if locations:
        parts.append("## Upstream Schematron source")
        parts.append("")
        parts.append("| Syntax | File | Line | Test |")
        parts.append("| --- | --- | ---: | --- |")
        for loc in locations:
            syntax = loc.get("syntax", "?")
            f = loc.get("file", "?")
            line = loc.get("assert_line", loc.get("id_line", "?"))
            test = (loc.get("test") or "").replace("|", "&#124;")
            parts.append(f"| {syntax} | `{f}` | {line} | `{test}` |")
        parts.append("")
    return "\n".join(parts) + "\n"


def write_rule_pages(rule_data: pathlib.Path, site: pathlib.Path) -> int:
    data = json.loads(rule_data.read_text(encoding="utf-8"))
    rules = data.get("rules", [])
    rules_dir = site / "rules"
    rules_dir.mkdir(parents=True, exist_ok=True)
    meta: dict[str, str] = {"index": "Overview"}
    (rules_dir / "index.mdx").write_text(
        f"# EN 16931 rules\n\n"
        f"{len(rules)} business rules, generated from `crates/rulepack/data/en16931-br-co-coverage.json`.\n\n"
        f"Pick a rule in the sidebar to see business terms, validator-testability flags, and source-XSLT line citations.\n",
        encoding="utf-8",
    )
    for rule in rules:
        rule_id = rule["id"]
        path = rules_dir / f"{rule_id}.mdx"
        path.write_text(render_rule_mdx(rule), encoding="utf-8")
        meta[rule_id] = rule_id
    (rules_dir / "_meta.json").write_text(
        json.dumps(meta, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return len(rules)


_DESC_RE = re.compile(r'description\s*=\s*"([^"]+)"', re.MULTILINE)
_NAME_RE = re.compile(r'name\s*=\s*"(invoicekit-report-[a-z0-9-]+)"', re.MULTILINE)


def parse_report_crate(cargo_toml: pathlib.Path) -> tuple[str, str, str] | None:
    text = cargo_toml.read_text(encoding="utf-8")
    name_match = _NAME_RE.search(text)
    if not name_match:
        return None
    crate_name = name_match.group(1)
    suffix = crate_name.removeprefix("invoicekit-report-")
    # suffix looks like `it-sdi`, `cn-fapiao`, `es-verifactu`, ...
    cc = suffix.split("-", 1)[0].upper()
    desc_match = _DESC_RE.search(text)
    description = desc_match.group(1) if desc_match else "InvoiceKit country report adapter."
    return cc, crate_name, description


def render_country_mdx(cc: str, crate_name: str, description: str) -> str:
    country_name = COUNTRY_NAMES.get(cc, cc)
    return (
        f"# {country_name} — `{crate_name}`\n\n"
        f"**Country code:** {cc}  \n"
        f"**Crate:** [`{crate_name}`](https://github.com/MuhDur/invoicekit/tree/main/crates/{crate_name.removeprefix('invoicekit-')})\n\n"
        "## Overview\n\n"
        f"{description}\n\n"
        "## Substrate\n\n"
        "The crate ships the typed Provider trait + a deterministic MockProvider so "
        "engine wiring is stable even when the operator hasn't yet provisioned "
        "credentials for the live national tax-authority endpoint. Live transport "
        "lands in a follow-up `*-http` crate that wraps `reqwest`.\n"
    )


COUNTRY_NAMES: dict[str, str] = {
    "AR": "Argentina",
    "BE": "Belgium",
    "BR": "Brazil",
    "CL": "Chile",
    "CN": "China",
    "CO": "Colombia",
    "CR": "Costa Rica",
    "DO": "Dominican Republic",
    "EC": "Ecuador",
    "EG": "Egypt",
    "ES": "Spain",
    "FR": "France",
    "GR": "Greece",
    "HU": "Hungary",
    "ID": "Indonesia",
    "IL": "Israel",
    "IN": "India",
    "IT": "Italy",
    "JP": "Japan",
    "KE": "Kenya",
    "KR": "South Korea",
    "MX": "Mexico",
    "MY": "Malaysia",
    "NG": "Nigeria",
    "PE": "Peru",
    "PH": "Philippines",
    "PL": "Poland",
    "RO": "Romania",
    "SA": "Saudi Arabia",
    "TH": "Thailand",
    "TR": "Türkiye",
    "TW": "Taiwan",
    "VN": "Vietnam",
    "ZA": "South Africa",
}


def write_country_pages(site: pathlib.Path) -> int:
    countries_dir = site / "countries"
    countries_dir.mkdir(parents=True, exist_ok=True)
    meta: dict[str, str] = {"index": "Overview"}
    pages: list[tuple[str, str, str, str]] = []
    for cargo in sorted((REPO).glob(f"{REPORT_CRATES_GLOB}/Cargo.toml")):
        parsed = parse_report_crate(cargo)
        if not parsed:
            continue
        cc, crate_name, description = parsed
        slug = crate_name.removeprefix("invoicekit-report-")
        pages.append((cc, slug, crate_name, description))
    for cc, slug, crate_name, description in pages:
        page_name = f"{cc.lower()}-{slug.split('-', 1)[1] if '-' in slug else slug}"
        path = countries_dir / f"{page_name}.mdx"
        path.write_text(
            render_country_mdx(cc, crate_name, description),
            encoding="utf-8",
        )
        meta[page_name] = f"{COUNTRY_NAMES.get(cc, cc)} ({cc})"
    (countries_dir / "index.mdx").write_text(
        f"# Country guides\n\n"
        f"{len(pages)} country adapter(s) shipped under `crates/report-*`.\n\n"
        "Each adapter ships a typed Provider + MockProvider substrate today; live "
        "national tax-authority transport lands in follow-up `*-http` crates.\n",
        encoding="utf-8",
    )
    (countries_dir / "_meta.json").write_text(
        json.dumps(meta, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return len(pages)


def write_operator_pages(site: pathlib.Path) -> int:
    ops_dir = site / "operators"
    ops_dir.mkdir(parents=True, exist_ok=True)
    meta: dict[str, str] = {"index": "Overview"}
    runbooks = sorted(DOCS_OPERATORS.glob("*.md"))
    for rb in runbooks:
        # The runbook is plain markdown; MDX accepts it directly.
        body = rb.read_text(encoding="utf-8")
        out = ops_dir / f"{rb.stem.lower()}.mdx"
        out.write_text(body, encoding="utf-8")
        meta[rb.stem.lower()] = rb.stem.replace("-", " ").title()
    (ops_dir / "index.mdx").write_text(
        f"# Operator runbooks\n\n"
        f"{len(runbooks)} runbook(s) mirrored from `docs/operators/`.\n",
        encoding="utf-8",
    )
    (ops_dir / "_meta.json").write_text(
        json.dumps(meta, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return len(runbooks)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    rule_count = write_rule_pages(args.rule_data, args.site)
    country_count = write_country_pages(args.site)
    runbook_count = write_operator_pages(args.site)
    print(
        f"generated {rule_count} rule page(s), "
        f"{country_count} country page(s), "
        f"{runbook_count} operator runbook(s) -> {args.site}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
