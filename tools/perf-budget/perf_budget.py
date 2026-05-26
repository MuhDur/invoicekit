#!/usr/bin/env python3
"""InvoiceKit performance regression budget gate.

Reads a criterion benchmark output directory and a baseline directory, applies
the per-operation thresholds from `budget.toml`, and exits non-zero when any
tracked operation regresses beyond its threshold.

The schema this script consumes is the on-disk format criterion leaves under
`target/criterion/<bench-name>/new/estimates.json`. Each estimates file is
a JSON object that includes a `mean.point_estimate` (nanoseconds, float) for
the most recent run; the baseline directory mirrors that layout for the
reference data point.

Usage
-----

    perf_budget.py \\
        --current target/criterion \\
        --baseline baseline/criterion \\
        --budget tools/perf-budget/budget.toml \\
        [--summary-out summary.md]

Exit codes
----------

* 0 — every tracked operation is within budget.
* 1 — one or more tracked operations regressed beyond their threshold.
* 2 — invalid input (missing files, malformed JSON, malformed budget).

Design notes
------------

* The tracked-operation list comes from `budget.toml`. Operations present in
  the current criterion output but absent from the budget are reported as
  informational; they never fail the build. This lets a bead introduce a new
  benchmark without immediately gating CI on it.
* When the baseline is missing for a tracked operation, the script reports the
  operation as new and does not fail; the first run of a new benchmark seeds
  the baseline rather than failing the PR that introduced it.
* The script writes a markdown summary (when `--summary-out` is given) that
  the CI workflow surfaces as a sticky PR comment.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from pathlib import Path
from typing import Iterable, Mapping

try:  # Python 3.11+ ships tomllib in the standard library.
    import tomllib  # type: ignore[attr-defined]
except ModuleNotFoundError:  # pragma: no cover - runtime fallback for 3.10.
    import tomli as tomllib  # type: ignore[no-redef]


EXIT_OK = 0
EXIT_REGRESSION = 1
EXIT_INVALID_INPUT = 2


@dataclasses.dataclass(frozen=True)
class OperationResult:
    """Outcome for a single tracked operation."""

    name: str
    baseline_ns: float | None
    current_ns: float | None
    threshold_pct: float
    status: str  # one of: ok, regression, new, missing-current, baseline-missing

    @property
    def delta_pct(self) -> float | None:
        if self.baseline_ns is None or self.current_ns is None:
            return None
        if self.baseline_ns <= 0.0:
            return None
        return (self.current_ns - self.baseline_ns) / self.baseline_ns * 100.0

    def to_row(self) -> str:
        delta = self.delta_pct
        delta_str = "n/a" if delta is None else f"{delta:+.2f}%"
        baseline_str = "n/a" if self.baseline_ns is None else f"{self.baseline_ns:,.0f} ns"
        current_str = "n/a" if self.current_ns is None else f"{self.current_ns:,.0f} ns"
        return (
            f"| `{self.name}` | {baseline_str} | {current_str} | {delta_str} | "
            f"{self.threshold_pct:.1f}% | {self.status} |"
        )


def load_budget(path: Path) -> tuple[float, Mapping[str, float]]:
    """Parse `budget.toml` into (default_threshold, {operation: threshold})."""
    try:
        with path.open("rb") as handle:
            data = tomllib.load(handle)
    except FileNotFoundError as exc:
        raise SystemExit(f"budget file not found: {exc.filename}") from exc
    except tomllib.TOMLDecodeError as exc:
        raise SystemExit(f"budget file is not valid TOML: {exc}") from exc

    default_pct = float(data.get("default_max_regression_pct", 10.0))
    operations_section = data.get("operations") or {}
    if not isinstance(operations_section, dict):
        raise SystemExit("budget file: `operations` must be a table")

    operations: dict[str, float] = {}
    for name, section in operations_section.items():
        if not isinstance(section, dict):
            raise SystemExit(f"budget file: `operations.{name}` must be a table")
        threshold = float(section.get("max_regression_pct", default_pct))
        if threshold <= 0.0:
            raise SystemExit(
                f"budget file: `operations.{name}.max_regression_pct` must be > 0"
            )
        operations[name] = threshold
    return default_pct, operations


def load_estimate(criterion_dir: Path, op_name: str) -> float | None:
    """Return the mean point estimate (ns) for `op_name`, or None if absent.

    Criterion lays the file out at `<root>/<op_name>/new/estimates.json`. When
    the bench has never been run, that file is missing; we treat it as None
    rather than as a failure so the baseline-seeding case works.
    """
    estimates_path = criterion_dir / op_name / "new" / "estimates.json"
    if not estimates_path.is_file():
        return None
    try:
        with estimates_path.open() as handle:
            payload = json.load(handle)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"malformed criterion JSON at {estimates_path}: {exc}") from exc

    mean = payload.get("mean")
    if not isinstance(mean, dict):
        raise SystemExit(f"criterion JSON at {estimates_path} missing `mean`")
    point_estimate = mean.get("point_estimate")
    if not isinstance(point_estimate, (int, float)):
        raise SystemExit(
            f"criterion JSON at {estimates_path} missing `mean.point_estimate`"
        )
    return float(point_estimate)


def evaluate(
    current_dir: Path,
    baseline_dir: Path | None,
    operations: Mapping[str, float],
) -> list[OperationResult]:
    """Compute the regression status for every tracked operation."""
    results: list[OperationResult] = []
    for name, threshold_pct in operations.items():
        current_ns = load_estimate(current_dir, name)
        baseline_ns = (
            load_estimate(baseline_dir, name) if baseline_dir is not None else None
        )

        if current_ns is None:
            status = "missing-current"
        elif baseline_ns is None:
            status = "new"
        else:
            delta_pct = (current_ns - baseline_ns) / baseline_ns * 100.0
            status = "regression" if delta_pct > threshold_pct else "ok"

        results.append(
            OperationResult(
                name=name,
                baseline_ns=baseline_ns,
                current_ns=current_ns,
                threshold_pct=threshold_pct,
                status=status,
            )
        )
    return results


def render_summary(results: Iterable[OperationResult]) -> str:
    """Render a markdown table for the PR comment."""
    rows = list(results)
    header = (
        "## Performance regression budget\n\n"
        "| Operation | Baseline | Current | Δ | Threshold | Status |\n"
        "|-----------|----------|---------|----|-----------|--------|"
    )
    body = "\n".join(r.to_row() for r in rows)
    footer_lines = []
    if any(r.status == "regression" for r in rows):
        footer_lines.append("\n**One or more tracked operations regressed beyond the budget.**")
    elif any(r.status == "missing-current" for r in rows):
        footer_lines.append("\n**One or more tracked operations are missing a current measurement.**")
    elif any(r.status == "new" for r in rows):
        footer_lines.append("\nA new benchmark seeded its baseline this run; no regression check applied.")
    return f"{header}\n{body}{''.join(footer_lines)}\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--current", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument(
        "--budget",
        type=Path,
        default=Path(__file__).parent / "budget.toml",
    )
    parser.add_argument("--summary-out", type=Path)
    args = parser.parse_args(argv)

    if not args.current.is_dir():
        print(f"--current path is not a directory: {args.current}", file=sys.stderr)
        return EXIT_INVALID_INPUT

    baseline_dir = args.baseline if args.baseline and args.baseline.is_dir() else None

    _default_pct, operations = load_budget(args.budget)
    if not operations:
        print("budget defines no tracked operations; nothing to check", file=sys.stderr)
        return EXIT_OK

    results = evaluate(args.current, baseline_dir, operations)
    summary = render_summary(results)
    print(summary)
    if args.summary_out is not None:
        args.summary_out.write_text(summary)

    if any(r.status == "regression" for r in results):
        return EXIT_REGRESSION
    if any(r.status == "missing-current" for r in results):
        return EXIT_REGRESSION
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
