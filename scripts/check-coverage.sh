#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# 9cty: parse a cargo-llvm-cov JSON summary and compare each
# crate's line coverage against scripts/coverage-thresholds.json.
# Exits non-zero on the first floor breach so CI surfaces a clear
# diff, not a slow flake.
#
# Inputs:
#   $1  path to cargo-llvm-cov --json summary file
#   $2  path to thresholds JSON (defaults to scripts/coverage-thresholds.json)
#
# Outputs:
#   stdout      a markdown table suitable for $GITHUB_STEP_SUMMARY
#   exit code   0 when all gated crates clear their floor, 1 otherwise
set -euo pipefail

SUMMARY="${1:?usage: check-coverage.sh <summary.json> [thresholds.json]}"
THRESHOLDS="${2:-scripts/coverage-thresholds.json}"

if [[ ! -f "$SUMMARY" ]]; then
  echo "check-coverage: summary file not found at $SUMMARY" >&2
  exit 2
fi
if [[ ! -f "$THRESHOLDS" ]]; then
  echo "check-coverage: thresholds file not found at $THRESHOLDS" >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "check-coverage: jq is required" >&2
  exit 2
fi

# cargo-llvm-cov --json reports line coverage per source file under
# data[0].files[].filename / .summary.lines.percent. Group by the
# crate name extracted from the path segment after "crates/" or
# "bindings/" or "services/".
ROWS=$(jq -r '
  .data[0].files
  | map(
      . as $f
      | ($f.filename
          | capture("(?<root>crates|bindings|services)/(?<crate>[^/]+)/";"x")
          | .crate
          // null
        ) as $crate
      | select($crate != null)
      | {crate: $crate, covered: $f.summary.lines.covered, total: $f.summary.lines.count}
    )
  | group_by(.crate)
  | map({
      crate: .[0].crate,
      covered: (map(.covered) | add // 0),
      total: (map(.total) | add // 0)
    })
  | sort_by(.crate)
  | .[]
  | [.crate, .covered, .total]
  | @tsv
' "$SUMMARY")

declare -A FLOOR
declare -A SEEN
while IFS=$'\t' read -r crate floor; do
  if [[ -n "$crate" ]]; then
    FLOOR["$crate"]="$floor"
  fi
done < <(jq -r '.crates | to_entries[] | [.key, .value] | @tsv' "$THRESHOLDS")

WORKSPACE_FLOOR=$(jq -r '.overall_workspace_floor // 0' "$THRESHOLDS")

TOTAL_COVERED=0
TOTAL_LINES=0
BREACHES=()

printf '| Crate | Lines | Covered | Coverage | Floor | Status |\n'
printf '|---|---:|---:|---:|---:|:---|\n'
while IFS=$'\t' read -r crate covered total; do
  [[ -z "$crate" ]] && continue
  SEEN["$crate"]=1
  TOTAL_COVERED=$((TOTAL_COVERED + covered))
  TOTAL_LINES=$((TOTAL_LINES + total))
  if [[ "$total" -eq 0 ]]; then
    pct="n/a"
    status="skip (no lines)"
  else
    pct=$(awk -v c="$covered" -v t="$total" 'BEGIN { printf "%.1f", (c * 100.0) / t }')
    floor="${FLOOR[$crate]:-}"
    if [[ -z "$floor" ]]; then
      status="report only"
    else
      breach=$(awk -v p="$pct" -v f="$floor" 'BEGIN { print (p + 0 < f + 0) ? "1" : "0" }')
      if [[ "$breach" == "1" ]]; then
        status="**FAIL (floor ${floor}%)**"
        BREACHES+=("$crate: $pct% < ${floor}%")
      else
        status="ok (floor ${floor}%)"
      fi
    fi
  fi
  floor_display="${FLOOR[$crate]:--}"
  printf '| %s | %s | %s | %s%% | %s | %s |\n' "$crate" "$total" "$covered" "$pct" "$floor_display" "$status"
done <<< "$ROWS"

# Any threshold-listed crate that we never saw in the summary is
# a configuration drift — flag it but don't fail the build (it
# may simply have been split or renamed by a downstream bead).
for crate in "${!FLOOR[@]}"; do
  if [[ -z "${SEEN[$crate]:-}" ]]; then
    printf '| %s | _missing_ | _missing_ | _missing_ | %s | report only (no source files) |\n' "$crate" "${FLOOR[$crate]}"
  fi
done

if [[ "$TOTAL_LINES" -gt 0 ]]; then
  OVERALL=$(awk -v c="$TOTAL_COVERED" -v t="$TOTAL_LINES" 'BEGIN { printf "%.1f", (c * 100.0) / t }')
else
  OVERALL="0.0"
fi
printf '\n**Workspace overall: %s%% (floor %s%%)**\n' "$OVERALL" "$WORKSPACE_FLOOR"

OVERALL_BREACH=$(awk -v p="$OVERALL" -v f="$WORKSPACE_FLOOR" 'BEGIN { print (p + 0 < f + 0) ? "1" : "0" }')
if [[ "$OVERALL_BREACH" == "1" ]]; then
  BREACHES+=("workspace: $OVERALL% < ${WORKSPACE_FLOOR}%")
fi

if [[ "${#BREACHES[@]}" -gt 0 ]]; then
  printf '\nCoverage floor breaches:\n' >&2
  for b in "${BREACHES[@]}"; do
    printf '  - %s\n' "$b" >&2
  done
  exit 1
fi
