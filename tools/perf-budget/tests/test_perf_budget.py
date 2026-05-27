"""Unit tests for the InvoiceKit performance regression budget gate.

These tests cover the happy path, four explicit failure modes, and the
false-positive scenario that T-007's acceptance criterion calls out.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
TOOL = REPO / "tools" / "perf-budget"
sys.path.insert(0, str(TOOL))
import perf_budget  # noqa: E402  (path adjustment is intentional)


def make_estimates(criterion_dir: Path, op: str, mean_ns: float) -> None:
    """Write a criterion-shaped `estimates.json` for `op` under `criterion_dir`."""
    estimates_dir = criterion_dir / op / "new"
    estimates_dir.mkdir(parents=True, exist_ok=True)
    payload = {"mean": {"point_estimate": float(mean_ns)}}
    (estimates_dir / "estimates.json").write_text(
        json.dumps(payload), encoding="utf-8"
    )


def write_budget(path: Path, op: str, threshold_pct: float, *, default: float = 10.0) -> None:
    path.write_text(
        f"default_max_regression_pct = {default}\n\n"
        f"[operations.{op}]\nmax_regression_pct = {threshold_pct}\n",
        encoding="utf-8",
    )


def run_perf_budget_cli(*args: str) -> subprocess.CompletedProcess[str]:
    """Run the CLI entrypoint so process exit-code behavior is covered."""
    return subprocess.run(
        [sys.executable, str(TOOL / "perf_budget.py"), *args],
        check=False,
        text=True,
        capture_output=True,
        timeout=10,
    )


def test_known_good_pr_does_not_fail(tmp_path: Path) -> None:
    """T-007 acceptance: a known-good PR (no real movement) must not fail."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    make_estimates(baseline, "ir-round-trip", 1_000_000.0)
    make_estimates(current, "ir-round-trip", 1_010_000.0)  # 1% drift, well under 10%
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_OK


def test_regression_beyond_threshold_fails(tmp_path: Path) -> None:
    """A regression past the configured threshold must fail the build."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    make_estimates(baseline, "ir-round-trip", 1_000_000.0)
    make_estimates(current, "ir-round-trip", 1_150_000.0)  # 15% slowdown, over 10%
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_REGRESSION


def test_regression_just_under_threshold_passes(tmp_path: Path) -> None:
    """A regression strictly under the threshold must not fail (boundary case)."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    make_estimates(baseline, "ir-round-trip", 1_000_000.0)
    make_estimates(current, "ir-round-trip", 1_099_000.0)  # 9.9%
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_OK


def test_per_operation_threshold_overrides_default(tmp_path: Path) -> None:
    """A tighter per-operation threshold must catch what the default would miss."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    make_estimates(baseline, "canonicalize", 1_000_000.0)
    make_estimates(current, "canonicalize", 1_070_000.0)  # 7% — over a tight 5%
    budget = tmp_path / "budget.toml"
    write_budget(budget, "canonicalize", 5.0, default=10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_REGRESSION


def test_new_benchmark_seeds_baseline_without_failing(tmp_path: Path) -> None:
    """When the baseline is missing for an operation, the PR introducing it passes."""
    baseline = tmp_path / "baseline"
    baseline.mkdir()
    current = tmp_path / "current"
    make_estimates(current, "ir-round-trip", 1_000_000.0)
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_OK


def test_missing_current_measurement_fails(tmp_path: Path) -> None:
    """Operation tracked in the budget but absent from current run fails the build."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    current.mkdir()  # empty
    make_estimates(baseline, "ir-round-trip", 1_000_000.0)
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    exit_code = perf_budget.main(
        ["--current", str(current), "--baseline", str(baseline), "--budget", str(budget)]
    )
    assert exit_code == perf_budget.EXIT_REGRESSION


def test_malformed_budget_exits_with_invalid_input_code(tmp_path: Path) -> None:
    """Malformed budget.toml is invalid input, not a regression failure."""
    current = tmp_path / "current"
    current.mkdir()
    budget = tmp_path / "budget.toml"
    budget.write_text("[operations.ir-round-trip\n", encoding="utf-8")

    completed = run_perf_budget_cli(
        "--current", str(current),
        "--budget", str(budget),
    )

    assert completed.returncode == perf_budget.EXIT_INVALID_INPUT
    assert "budget file is not valid TOML" in completed.stderr


def test_non_table_operations_budget_exits_with_invalid_input_code(
    tmp_path: Path,
) -> None:
    """Wrong-shaped budget operations data is invalid input."""
    current = tmp_path / "current"
    current.mkdir()
    budget = tmp_path / "budget.toml"
    budget.write_text("operations = []\n", encoding="utf-8")

    completed = run_perf_budget_cli(
        "--current", str(current),
        "--budget", str(budget),
    )

    assert completed.returncode == perf_budget.EXIT_INVALID_INPUT
    assert "budget file: `operations` must be a table" in completed.stderr


def test_malformed_estimate_json_exits_with_invalid_input_code(tmp_path: Path) -> None:
    """Malformed Criterion estimates are invalid input, not a regression failure."""
    current = tmp_path / "current"
    estimates_dir = current / "ir-round-trip" / "new"
    estimates_dir.mkdir(parents=True)
    (estimates_dir / "estimates.json").write_text("{not-json", encoding="utf-8")
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    completed = run_perf_budget_cli(
        "--current", str(current),
        "--budget", str(budget),
    )

    assert completed.returncode == perf_budget.EXIT_INVALID_INPUT
    assert "malformed criterion JSON" in completed.stderr


def test_wrong_shape_estimate_json_exits_with_invalid_input_code(
    tmp_path: Path,
) -> None:
    """Valid JSON with the wrong shape is invalid input, not a traceback."""
    current = tmp_path / "current"
    estimates_dir = current / "ir-round-trip" / "new"
    estimates_dir.mkdir(parents=True)
    (estimates_dir / "estimates.json").write_text("[]", encoding="utf-8")
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)

    completed = run_perf_budget_cli(
        "--current", str(current),
        "--budget", str(budget),
    )

    assert completed.returncode == perf_budget.EXIT_INVALID_INPUT
    assert "must be an object" in completed.stderr


def test_summary_is_markdown_table(tmp_path: Path) -> None:
    """The summary written for the PR comment is a recognizable markdown table."""
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    make_estimates(baseline, "ir-round-trip", 1_000_000.0)
    make_estimates(current, "ir-round-trip", 1_010_000.0)
    budget = tmp_path / "budget.toml"
    write_budget(budget, "ir-round-trip", 10.0)
    summary_out = tmp_path / "summary.md"

    exit_code = perf_budget.main(
        [
            "--current", str(current),
            "--baseline", str(baseline),
            "--budget", str(budget),
            "--summary-out", str(summary_out),
        ]
    )
    assert exit_code == perf_budget.EXIT_OK

    summary = summary_out.read_text(encoding="utf-8")
    assert summary.startswith("## Performance regression budget")
    assert "| `ir-round-trip` |" in summary
    assert "ok" in summary
