<!-- SPDX-License-Identifier: Apache-2.0 -->

# Skill Matrix — InvoiceKit

What agent skills were applied to this repository, and where. Usage counts are
sourced from the per-turn record in `plans/COVERAGE_LOOP_EVIDENCE.md` (and the
prior hardening/coverage campaigns it summarizes). ~132 skills are *available*;
this matrix lists the ones actually used plus the notable relevant skills that
were deliberately **not** used and why. A 132-row dump would be noise — most of
the library (Next.js admin pages, browser-extension automation, GA4, etc.) is
irrelevant to a Rust e-invoicing engine.

## Skills used on InvoiceKit

| Skill | Used for | Intensity |
|-------|----------|-----------|
| `loop` | The self-paced driving harness for the whole coverage/audit/quality push. | Continuous |
| `verification-before-completion` | Gate before every "done": full `cargo test` + `clippy -D warnings` + (now) `pytest tools/release-checks/`. The discipline that catches "cargo-green ≠ CI-green". | Every turn (~37×) |
| `multi-pass-bug-hunting` | The security audit (51 bugs, 6 dangerous classes → 0) and every new-code adversarial audit (find → adversarially verify → confirm). Caught the 3 date-field, 6 allowance, and 1 schema-drift defects. | ~25× |
| `simplify-and-refactor-code-isomorphically` | Whole-workspace + per-crate code-quality convergence (Score≥2.0, golden-preserving); the `write_address_as` extraction; many honest no-ops. | ~18× |
| `reality-check-for-project` | Periodic "where are we really" coverage/vision assessments and convergence calls. | ~13× |
| `release-preparations` | The v0.1.1 release (test gate → version → tag → verify). | v0.1.1 |
| `repeatedly-apply-skill` | The per-crate loops applying the refactor/audit skills until convergence. | Per-crate waves |
| `testing-golden-artifacts` / `testing-metamorphic` / `testing-fuzzing` | Canonical-serialization golden suite, metamorphic round-trip properties, and the generative proptests that durably fixed the canonical prefix-disambiguation bug. | Foundational |
| `testing-conformance-harnesses` | The UBL/CII conformance-corpus round-trip + path-set gates. | Format crates |
| `profiling-software-performance` / `extreme-software-optimization` | The perf track (render-pdf 20.7× faster, gated by D19). | Hardening campaign |
| `codebase-audit` / `codebase-archaeology` / `mock-code-finder` | Mapping unfamiliar subsystems and the documentation-honesty/overclaim audit (~25 source overclaims fixed). | As needed |
| `git-stash-janitor` / `dcg` | Working-tree hygiene; `dcg` is the force-push guard (blocks `-f`; use `--force-with-lease`). | As needed |
| `ubs` / `gh-cli` / `gh-actions` | Pre-commit safety scan; GitHub operations; CI workflow understanding. | Routine |

## Notable available skills deliberately NOT used (and why)

| Skill | Why not |
|-------|---------|
| `ntm` / `brennerbot-with-ntm` / `code-review-gemini-swarm-with-ntm` | Heavy multi-agent NTM swarms — CLAUDE.md says use only when a specific skill explicitly requires it; routine work used lightweight `Workflow`/`Agent` fan-out instead. |
| `multi-model-triangulation` | Second-model opinions on Risk/Confidence — the adversarial verify stages inside the audit workflows covered this need in-loop. |
| `legacy-to-rust-porting` / `frankensearch-integration-for-rust-projects` | No legacy port or search integration in scope. |
| `documentation-website-for-software-project` | Docs site (Nextra) is a separate workstream; this push was engine/format coverage. |
| `beads-br` / `beads-bv` | `br ready` consulted for unblocked work, but the coverage push was directed by the standing `/loop` directive, not the bead queue. |
| `deadlock-finder-and-fixer` / `gdb-for-debugging` | No concurrency deadlock or native-debugging incident arose. |

## Notes

- The single most valuable discipline this push was `verification-before-completion`
  **extended** to the `tools/release-checks/` Python gate suite — `cargo test`
  alone hid a red CI schema gate for the whole session (caught Turn 48).
- The audit skills (`multi-pass-bug-hunting`) earned their keep repeatedly:
  independent adversarial review found real defects in green-suite code every
  time it ran.
