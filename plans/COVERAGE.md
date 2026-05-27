# Coverage policy and ramp

InvoiceKit measures line coverage with `cargo-llvm-cov` on every
Ubuntu CI run. The build does **not** fail on coverage today.
Instead, a per-crate floor in `scripts/coverage-thresholds.json`
guards a small set of foundation crates against regressions, and
the workspace-wide rate is reported on every PR.

This document explains the ramp from "report only" to the
universal 90% line-coverage gate the PLAN promises.

## Why a ramp and not a hard 90% gate today

Several boundary crates carry shapes that genuinely should not
be 90% covered:

- C ABI shims and bindings reach out through `cargo build` and
  cross-language tests that `cargo-llvm-cov` cannot see.
- Validator JVM workers run as a JSON-RPC sidecar in a separate
  process; their coverage is owned by `services/validator-*`
  unit tests, not the Rust workspace.
- Format/transmit adapters that wrap a partner SDK are exercised
  by sandbox cassettes — `cargo test` only sees the playback
  layer, not the third-party code.

Forcing 90% on day one would push contributors to mock-heavy
tests that do not survive real-network parity checks. The trust
toolkit's whole point is to test against the real artefact and
record its evidence — coverage is one signal among many.

## Floor table — current

Floors are keyed by the directory name under `crates/`,
`bindings/`, or `services/`, not by the Cargo package name —
this keeps the script free of any need to parse `Cargo.toml`.

| Crate dir | Floor | Rationale |
|---|---:|---|
| `ir` | 60 | Layered invoice model is the spine; round-trip + envelope tests already > 60. |
| `money` | 60 | Money/tax math is pure and well-tested; regressions here are silent. |
| `codelists` | 60 | Manifest validation guards the signing payload. |
| `canonical` | 60 | Canonical JSON ordering is load-bearing for `Doc` equality. |
| `tax-calculation` | 50 | Strategies have wide branching; tightening this needs more category fixtures. |
| `rulepack` | 40 | Rulepack is evaluated end-to-end through `validate`; bumping the floor needs unit-level tests. |
| `format-ubl` | 40 | UBL exit lane is exercised by profile crates; bump after CIUS coverage matrices land. |
| `validate` | 40 | Validate orchestrates JVM workers via JSON-RPC; pure-Rust paths covered, sidecar paths report only. |

Crates not listed are **report only** until a follow-up bead
explicitly raises their floor. The overall workspace floor is
30%, set just under the current rate, so a regression draws
attention without making "add a tiny crate" change CI red.

## Ramp toward 90%

The path is per-crate, not workspace-wide. Each milestone gets
its own bead so PRs stay small and reviewable.

1. **+10% per quarter on listed crates** — when a crate sustains
   its floor for two consecutive months without a waiver, raise
   the floor by 10 points and update this document.
2. **Add boundary crates one at a time** — when a binding or
   format crate grows pure-Rust logic, list it here with a 40%
   floor and the rationale.
3. **Workspace floor follows the trailing 30-day median** —
   never the latest run; this absorbs single-PR noise.
4. **At 70% on every foundation crate**, tighten the workspace
   floor to 60% and the universal target becomes opt-out per
   crate, with the opt-out documented in this file.
5. **At 80% on every foundation crate**, run a one-off
   measurement of the bindings against the cross-language ABI
   golden suite and capture the gap. From this point, the 90%
   target applies to crates listed here, not to the whole
   workspace blindly.

## Operational notes

- The coverage job runs only on Ubuntu — macOS coverage on
  `cargo-llvm-cov` is flaky in shared-runner setups.
- `cargo-llvm-cov` is installed as a prebuilt binary via the
  `taiki-e/install-action` step in CI; bumping it is a one-line
  workflow change.
- The summary table lands in the Actions step summary on every
  PR. Reviewers can click through to the lcov artefact to drill
  into a specific file.
- `scripts/check-coverage.sh` runs locally too: feed it any
  cargo-llvm-cov `--json` summary file. Useful before opening a
  PR that touches a gated crate.

## Out of scope

- Branch coverage — `cargo-llvm-cov` reports it but we do not
  gate on it yet. The instrumentation is noisy on match arms
  and generic dispatch.
- Mutation testing — tracked separately under the `cargo-mutants`
  beads, not part of this ramp.
- Per-line annotations in PR reviews — would need a comment-bot
  workflow; intentionally out of scope.
