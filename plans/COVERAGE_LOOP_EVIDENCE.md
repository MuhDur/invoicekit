# Coverage Loop — Evidence & Decision Log

> Living log for the `/loop` campaign started **2026-05-29**: drive InvoiceKit to
> honest, end-to-end (local-only) support for every claimed country, with exhaustive
> end-to-end tests, automated build outputs for every artifact, a known-limitations
> count driven to zero (or as close as honestly possible), and a tagged GitHub
> release backed by evidence.
>
> This file is the durable spine across loop turns. Every turn: (1) read this log
> + `AGENTS.md` first, (2) do a bounded chunk of real work, (3) append a turn entry
> with skills used + evidence, (4) re-evaluate next skills. Decisions are made
> autonomously and recorded here.

---

## 0. Mission (verbatim intent)

Improve coverage. Do not stop until **all countries** (local-only where that's the
honest ceiling) are **end-to-end supported** with **exhaustive end-to-end test
coverage**. Produce **automated build outputs** for each output possibility. Reach a
**release version, fully tested with evidence**, and a **GitHub product release that
is DONE**. Drive the count of **known limitations to 0 or as close as possible**.
Use dynamic `workflow` orchestration where possible; log skills per turn; use
`/repeatedly-apply-skill` to converge.

---

## 1. The honest bar — what "end-to-end supported (local only)" means

Per architectural commitment #11 ("country coverage is honest; no blanket supported
claims"), a country counts as **end-to-end supported** when the **offline lifecycle**
works and is proven by tests, with every capability honestly labelled:

1. **Serialize** — build a canonical `CommercialDocument` (IR) and emit the country's
   real artifact (EN 16931 / UBL / CII / Peppol BIS for format-family countries; the
   national format or a faithful typed payload for clearance countries).
2. **Validate (local)** — pure-Rust structural + rule checks pass; reference-grade
   (JVM/veraPDF) validation is labelled `requires_external_backend`, not faked.
3. **Sign (local)** — where a `signer-*` crate exists, sign with a deterministic test
   key; otherwise the adapter's signature concept is exercised by the mock.
4. **Transmit (mock/offline)** — a deterministic `Mock*Provider` returns an accepted
   envelope with the real receipt shape (clearance number, signature, status).
5. **Evidence** — produce a `.ikb` bundle and `verify` it (exit 0).
6. **Capabilities** — a `matrix.json` entry advertises the country with **honest**
   per-capability levels + source provenance.
7. **E2E test** — a `tests/` integration test drives steps 1→6 deterministically.

**Anti-slop rule:** each country adapter must encode something genuinely
country-specific (real tax-ID/format validation, real receipt shape, real format
family). Forty near-identical mock clones = fake parity and are NOT acceptable.
Live network transmission stays bring-your-own-credentials / sandbox by design and
is labelled as such — that is an honest ceiling, not a limitation to "fix".

---

## 2. Baseline facts (verified Turn 1, 2026-05-29)

| Fact | Value | How verified |
|---|---|---|
| Workspace members | 109 packages | `cargo metadata --no-deps` |
| `cargo check --workspace --all-targets` | **PASS** (exit 0) | run Turn 1 |
| `cargo test --workspace` | **PASS** (exit 0) | run Turn 1 |
| Beads | 281 total, **all closed**, 0 ready | `br stats` |
| Capability matrix entries | **7** (DE, FR, IT, NL only) | `jq` on `crates/cli/data/capabilities/matrix.json` |
| Country `report-*` crates | ~34 | `ls crates/report-*` |
| Flagship report adapters (IT,FR,PL,MX,BR,SA) | **60-line identity stubs** | LOC scan |
| Wave-2/3 report adapters (~28) | built-out typed surface + Mock provider + ~7 inline tests | LOC scan |
| Dedicated E2E/`tests/` per report crate | **0** | `find` scan |
| `signer-*` crates | sdi, ksef, cfdi, nfe, zatca, france-ctc, eidas | `ls` |
| CI workflows | 43 (incl. `release.yml`, `wasm-artifact.yml`, all 5 SDK builds) | `ls .github/workflows` |
| Git working state | clean except untracked `.claude/`, `.ntm/` | `git status` |

### The core gaps (the work)
- **G1 — Capability honesty:** matrix knows 4 countries; we claim ~60. Every supported
  country needs an honest matrix entry. *(largest honesty gap)*
- **G2 — Flagship stubs:** IT/FR/PL/MX/BR/SA report adapters are identity-only despite
  having signer crates. Build real adapters reusing the signers.
- **G3 — No E2E:** zero country has an end-to-end offline-lifecycle test.
- **G4 — Asymmetry:** flagships have signers/no-adapter; wave-2/3 have adapters/no-signer.
  Each country needs the full local chain wired + proven.
- **G5 — Stub transmit:** `transmit-email`, `transmit-peppol` are 60-line stubs.
- **G6 — Release not cut:** no tag, nothing published. Release machinery exists but unused.

---

## 3. Known-limitations ledger (drive to 0 or honest-floor)

| # | Limitation (from README §Limitations + scan) | Disposition | Target |
|---|---|---|---|
| L1 | Pre-release; nothing tagged/published | **FIX** | Cut `v0.1.0` GitHub release |
| L2 | Validation needs a JVM for reference grade | **By design** (commitment #6) | Keep, label honestly |
| L3 | Live Peppol delivery is BYOK; native AS4 research-track | **By design** (commitment #7) | Keep, label honestly |
| L4 | Coverage maturity varies by country | **FIX** | Populate honest matrix for all |
| L5 | Inbound RTL/CJK vertical-script intake gap | **INVESTIGATE** | Reduce or document precisely |
| L6 | Flagship report adapters are stubs (G2) | **FIX** | Real adapters + E2E |
| L7 | No per-country E2E tests (G3) | **FIX** | E2E for every country |

"By design" items are honest ceilings, not defects; they stay but must be labelled
accurately in the capability matrix and README. The *count of fixable limitations*
is what we drive to zero.

---

## 4. Skill matrix (available agent skills → campaign phases)

| Phase | Primary skills | Support skills |
|---|---|---|
| Assess / steer | `reality-check-for-project` ✅T1, `mock-code-finder`, `codebase-audit` | `beads-br`, `bv` |
| Plan / decompose | `planning-workflow`, `beads-workflow`, `repeatedly-apply-skill` | `idea-wizard` |
| Implement country adapters | (direct + `Workflow` fan-out) `testing-real-service-e2e-no-mocks` | `legacy-to-rust-porting` |
| Exhaustive tests | `testing-real-service-e2e-no-mocks`, `testing-conformance-harnesses`, `testing-golden-artifacts`, `testing-fuzzing`, `testing-metamorphic` | `e2e-testing-for-webapps` (bindings/demos) |
| Bug elimination | `multi-pass-bug-hunting`, `ubs`, `systematic-debugging` | `deadlock-finder-and-fixer`, `rust-undefined-behavior-exorcist` |
| Rust quality | `running-the-gauntlet-on-your-rust-port`, `rust-unsafe-code-exorcist`, `simplify-and-refactor-code-isomorphically` | `library-updater` |
| Verify / certify | `verification-before-completion`, `reality-check-for-project` (re-run) | `code-review` |
| Build outputs | `gh-actions`, `release-preparations`, `rust-crates-publishing` | `installer-workmanship`, `dsr`, `rch` |
| Release | `release-preparations`, `gh-cli`, `changelog-md-workmanship` | `readme-writing`, `de-slopify` |
| Docs/evidence | `de-slopify`, `readme-writing`, `changelog-md-workmanship` | `documentation-website-for-software-project` |

Convergence driver: `/repeatedly-apply-skill` over the per-country implement→test→verify
unit, and over `multi-pass-bug-hunting` until a pass finds nothing.

---

## 5. Convergence plan (phases; re-evaluated each turn)

- **P0 Assess & log** *(Turn 1)* — reality check, baseline, this log, builder's-manual discovery.
- **P1 Golden reference** — hand-build ONE flagship country (Italy / report-it-sdi) full
  local E2E + capability entry. Proven pattern for fan-out.
- **P2 Flagship build-out** — FR, PL, MX, BR, SA real adapters + E2E (reuse signers).
- **P3 Wave-2/3 E2E + capability** — wire serialize→validate→mock→evidence + E2E for the
  ~28 built-out countries; honest matrix entries for all.
- **P4 Format-family countries** — the ~35 Peppol/EN16931 countries get matrix entries +
  representative E2E via the profile crates.
- **P5 Limitations sweep** — close L1/L4/L5/L6/L7; re-run reality check; `multi-pass-bug-hunting`.
- **P6 Build outputs** — verify every artifact builds (CLI, WASM, 5 SDKs, REST, evidence/validate actions).
- **P7 Release** — `release-preparations`, changelog, tag `v0.1.0`, GitHub release with checksums + evidence.

Each phase = a `Workflow` (pipeline: implement → verify-compiles+tests → adversarial review)
where parallelism is safe (distinct crate dirs; no shared-file edits; central `cargo` verify).

---

## 6. Decision log

- **D1 (T1):** Scope "supported (local only)" = the 7-step offline lifecycle in §1. Live
  transmission stays BYOK/sandbox by design (commitments #6/#7). Rationale: honest, achievable,
  matches architecture; avoids fake-network parity.
- **D2 (T1):** Build a hand-crafted golden reference country (Italy) before any fan-out, to
  prevent templated slop and give parallel agents a proven pattern.
- **D3 (T1):** No git worktrees / no feature branches (AGENTS.md collaboration model). Parallel
  agents edit only their own crate dir + use only already-resolved deps so `Cargo.lock` never
  races; central workspace `cargo test` verifies each wave.
- **D4 (T1):** Capability-matrix honesty (G1/L4) is treated as first-class deliverable equal to
  code — the binary must answer truthfully for every country we claim.
- **D5 (T1, reinforced by principal):** **Dynamic `Workflow` orchestration is the default
  execution mechanism for every loop turn**, not an occasional tool. Standing loop process:
  each turn (a) reads this log + AGENTS.md, (b) picks the next bounded chunk, (c) executes it
  as a `Workflow` (discovery fan-out, or implement→verify→adversarial-review pipeline) whenever
  the work is parallelizable or benefits from independent verification, (d) verifies centrally,
  (e) appends a turn entry. Solo inline work only for trivial/sequential edits. This satisfies
  the principal's explicit instruction to make workflows part of the loop implementation process.

## 5a. Standing loop implementation process (every turn)

1. Read `COVERAGE_LOOP_EVIDENCE.md` (this file) + `AGENTS.md`.
2. Choose the next phase chunk from §5.
3. **Author a dynamic `Workflow`** for it (fan-out for discovery; pipeline `implement →
   cargo verify → adversarial review` for code). Distinct crate dirs only; reuse resolved deps.
4. Verify centrally (`cargo test`/`clippy`); commit on `main` when green.
5. Append a turn entry: skills used, workflow used, evidence, decisions, next skills.
6. `ScheduleWakeup` to continue until convergence; stop only when §3 fixable limitations = 0,
   all countries pass §1, build outputs green, and the GitHub release is cut.

---

## 7. Per-turn log

### Turn 1 — 2026-05-29 — Assess & establish spine
- **Skills used:** `reality-check-for-project` (Phase-1 reality check on the whole project),
  `using-superpowers` (skill discipline).
- **Did:** Read AGENTS.md, README, COVERAGE.md, PLAN inventory. Ran build + full test baseline
  (both green). Mapped crate maturity by LOC. Found capability matrix covers only 4 countries.
  Wrote this evidence log + skill matrix + decisions + convergence plan.
- **Evidence:** §2 baseline table (all verified Turn 1).
- **Decisions:** D1–D4.
- **Next skills to iterate:** discovery workflow (builder's manual) → then `testing-real-service-e2e-no-mocks`
  to hand-build the Italy golden reference (P1).
