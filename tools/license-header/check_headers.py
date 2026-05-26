#!/usr/bin/env python3
"""InvoiceKit license-header gate.

Walks every Rust source file under the workspace and asserts that it carries
the canonical Apache 2.0 SPDX header. With ``--fix`` the missing header is
inserted in place; without it, the script lists every offending path and
exits non-zero.

The header itself is two lines plus a blank line:

::

    // SPDX-License-Identifier: Apache-2.0
    // Copyright {year} The InvoiceKit Authors

This shape matches what cargo-deny's license-check recognizes and what the
SPDX 3.x specification considers a valid file-level license tag.

Usage
-----

::

    python3 tools/license-header/check_headers.py --check
    python3 tools/license-header/check_headers.py --fix

Exit codes
----------

* 0 — every Rust source file under the workspace has the canonical header.
* 1 — at least one file is missing or has the wrong header.
* 2 — invalid argument (handled by ``argparse``).
"""

from __future__ import annotations

import argparse
import dataclasses
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parents[2]
SCAN_ROOTS = ("crates", "bindings", "services", "bridges")
SPDX_LINE = "// SPDX-License-Identifier: Apache-2.0"
COPYRIGHT_RE = re.compile(r"^// Copyright \d{4} The InvoiceKit Authors$")
DEFAULT_YEAR = datetime.now(timezone.utc).year


@dataclasses.dataclass(frozen=True)
class HeaderStatus:
    """Outcome for one source file."""

    path: Path
    ok: bool
    reason: str  # one of: ok, missing-spdx, missing-copyright, wrong-order


def iter_rust_files(repo_root: Path) -> Iterable[Path]:
    """Yield every `.rs` file under the configured scan roots."""
    for root in SCAN_ROOTS:
        base = repo_root / root
        if not base.is_dir():
            continue
        for candidate in base.rglob("*.rs"):
            # Ignore generated files (auto-generated marker on the first line).
            try:
                first = candidate.open(encoding="utf-8").readline()
            except OSError:
                continue
            if "@generated" in first:
                continue
            yield candidate


def classify(text: str) -> HeaderStatus | None:
    """Return None when the header is present; otherwise an offending status."""
    lines = text.splitlines()
    if not lines:
        return HeaderStatus(path=Path(""), ok=False, reason="missing-spdx")

    if lines[0].strip() != SPDX_LINE:
        return HeaderStatus(path=Path(""), ok=False, reason="missing-spdx")
    if len(lines) < 2 or not COPYRIGHT_RE.match(lines[1].strip()):
        return HeaderStatus(path=Path(""), ok=False, reason="missing-copyright")
    return None


def build_header(year: int = DEFAULT_YEAR) -> str:
    """Build the canonical two-line license header (with trailing blank line)."""
    return f"{SPDX_LINE}\n// Copyright {year} The InvoiceKit Authors\n\n"


def check_file(path: Path) -> HeaderStatus:
    """Run :func:`classify` against a single file path."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        return HeaderStatus(path=path, ok=False, reason=f"read-error: {exc}")
    status = classify(text)
    if status is None:
        return HeaderStatus(path=path, ok=True, reason="ok")
    return dataclasses.replace(status, path=path)


def fix_file(path: Path, year: int = DEFAULT_YEAR) -> bool:
    """Insert the canonical header if missing. Returns True when modified.

    Idempotent: running ``fix_file`` twice on the same file is a no-op the
    second time.
    """
    text = path.read_text(encoding="utf-8")
    status = classify(text)
    if status is None:
        return False

    if status.reason == "missing-spdx":
        new_text = build_header(year) + text
    elif status.reason == "missing-copyright":
        # The SPDX line is present but the copyright line is absent. Insert it
        # directly after the SPDX line so we don't duplicate the SPDX tag.
        lines = text.splitlines(keepends=True)
        copyright_line = f"// Copyright {year} The InvoiceKit Authors\n"
        insertion: list[str] = [lines[0], copyright_line]
        # Ensure a blank line follows the header for readability.
        rest = lines[1:]
        if rest and rest[0].strip() != "":
            insertion.append("\n")
        insertion.extend(rest)
        new_text = "".join(insertion)
    else:
        return False

    path.write_text(new_text, encoding="utf-8")
    return True


def run(check: bool, fix: bool, repo_root: Path) -> int:
    """Execute the gate or the autofix; returns the exit code."""
    failures: list[HeaderStatus] = []
    fixes: list[Path] = []
    for source in iter_rust_files(repo_root):
        status = check_file(source)
        if status.ok:
            continue
        if fix:
            if fix_file(source):
                fixes.append(source)
        else:
            failures.append(status)

    if fix:
        for path in fixes:
            print(f"fixed: {path.relative_to(repo_root)}")
        print(f"{len(fixes)} file(s) updated")
        return 0

    if not check:
        # No mode requested: behave as --check by default.
        check = True

    if not failures:
        print("license-header: all Rust source files carry the Apache 2.0 SPDX header")
        return 0

    for status in failures:
        print(
            f"license-header: {status.reason}: {status.path.relative_to(repo_root)}",
            file=sys.stderr,
        )
    print(
        f"license-header: {len(failures)} file(s) missing or malformed; run "
        f"`python3 tools/license-header/check_headers.py --fix` to repair",
        file=sys.stderr,
    )
    return 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="report missing headers (default)")
    mode.add_argument("--fix", action="store_true", help="insert the canonical header")
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="repository root to scan (default: this script's grandparent)",
    )
    args = parser.parse_args(argv)
    return run(check=args.check, fix=args.fix, repo_root=args.repo_root.resolve())


if __name__ == "__main__":
    raise SystemExit(main())
