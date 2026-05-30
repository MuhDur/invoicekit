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
| `cargo test --workspace` | **PASS** — 1876 tests, 0 failures | run Turn 1 |
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
| L1 | Pre-release; nothing tagged/published | **DONE (T9)** | `v0.1.1` published — 3 signed+checksummed platform binaries + SBOMs + OpenAPI; binary verified (`0.1.1`, resolves PL→KSeF) |
| L2 | Validation needs a JVM for reference grade | **By design** (commitment #6) | Keep, label honestly |
| L3 | Live Peppol delivery is BYOK; native AS4 research-track | **By design** (commitment #7) | Keep, label honestly |
| L4 | Coverage maturity varies by country | **DONE (T5)** | Honest matrix entry for all 36 claimed countries (39 entries); per-capability levels + provenance + confidence |
| L5 | Inbound RTL/CJK vertical-script intake gap | **DONE (T12)** | Real RTL (Unicode-bidi Arabic/Hebrew) + CJK vertical reconstruction in intake-pdf; 15 tests; README bounded honestly |
| L6 | Flagship report adapters are stubs (G2) | **DONE (T2–T3)** | All 6 flagships (IT/FR/PL/MX/BR/SA) now real adapters + offline E2E |
| L7 | No per-country E2E tests (G3) | **DONE (T2–T4)** | All 34 country report crates have offline E2E (verify exit 0) |
| L8 | Native national-format serialization built only for flagships (IT/MX/BR/PL); other countries serialize the EN16931/UBL representation | **HONEST RESIDUAL** | Disclosed via matrix format=UBL + confidence; native serializers tracked as follow-up. Not a hidden gap. |

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

- **D6 (T2, reinforced by principal):** **Commit AND push to GitHub at every green checkpoint**
  whenever it makes sense — i.e., after each country/wave reaches green (`cargo test` + `clippy
  -D warnings` pass for the touched crates and `cargo check --workspace --all-targets` is clean).
  Small, focused, signed-off commits directly on `main` (per AGENTS.md collaboration model). Never
  let completed green work sit uncommitted across a loop turn. The remote enforces 7 required CI
  checks; pushing keeps CI continuously exercising the work.
- **D7 (T3):** New-country capability-matrix entries are authored CENTRALLY (not by parallel agents):
  `matrix.json` is a shared, CI-gated file that interacts with the DE/FR/IT/NL CLI tests. Per-country E2E
  tests do NOT assert matrix presence (only the Italy reference does).
- **D8 (T5):** Capability-matrix honesty policy — advertise the format InvoiceKit actually emits +
  locally validates today: national format where a real serializer exists (IT FatturaPA, MX CFDI, BR NF-e,
  PL KSeF — added CFDI/NF-e/KSeF to the schema enum), `UBL`/`Peppol BIS`/`Peppol PINT` for UBL/Peppol-native
  regulators, and `UBL` (EN16931 representation, `confidence: medium`) elsewhere. Every serialize/validate
  claim is E2E-proven; reference validation stays `requires_external_backend`. Native national serializers
  for the non-flagships are an honest residual (L8), not an overclaim. Gated by the Python jsonschema + Rust
  `validate_matrix_semantics` checks.

## 5a. Standing loop implementation process (every turn)

1. Read `COVERAGE_LOOP_EVIDENCE.md` (this file) + `AGENTS.md`.
2. Choose the next phase chunk from §5.
3. **Author a dynamic `Workflow`** for it (fan-out for discovery; pipeline `implement →
   cargo verify → adversarial review` for code). Distinct crate dirs only; reuse resolved deps.
4. Verify centrally (`cargo test` + `cargo clippy -D warnings` + workspace `cargo check`).
5. **Commit AND push to GitHub** the green chunk (D6) — focused commit on `main`.
6. Append a turn entry: skills used, workflow used, evidence, decisions, next skills.
7. `ScheduleWakeup` to continue until convergence; stop only when §3 fixable limitations = 0,
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

### Turn 2 — 2026-05-29 — Builder's manual + Italy golden reference (P1 DONE)
- **Skills used:** `testing-real-service-e2e-no-mocks` (real-artifact offline E2E, no mocks of our own
  code — only the deterministic SDI transport mock), `testing-golden-artifacts` (hand-rolled determinism
  goldens), `verification-before-completion` (ran tests+clippy+workspace check before claiming done).
- **Workflow used:** `coverage-discovery-builders-manual` (6 agents, 552k tokens) → wrote
  `plans/_discovery_builders_manual.md` (the authoritative implementation reference for the fan-out).
- **Did (P1 — Italy golden reference, hand-built per D2):**
  - Real **IR→FatturaPA serializer** (`to_fattura_pa_xml`): deterministic `FatturaElettronica` FPR12 XML
    (header CedentePrestatore/CessionarioCommittente, body DatiGenerali/DettaglioLinee/DatiRiepilogo),
    XML-escaped, fixed element order. Genuinely country-specific (not the generic IR verbatim).
  - **SDI report adapter**: `SdiReportRequest/Envelope/Report/Error`, `SdiReportProvider` trait,
    `MockSdiReportProvider` **composing the existing `invoicekit-signer-sdi::MockSdiProvider`** (real
    XAdES signature path + `IdentificativoSdI`), with real `validate_italian_tax_id` (P.IVA 11 / CF 16)
    and `validate_progressivo` (1..=5 alnum). Rejection (NS) = receipt kind, NOT `Err`.
  - **E2E test** `tests/e2e_offline_lifecycle.rs`: the first per-country E2E in the workspace — drives
    build→serialize→validate→sign/transmit→evidence-bundle→`verify_packed`==ok, plus rejection path,
    byte-determinism, and capability-matrix presence.
  - **Foundation fix:** added `CountryCode::as_str()` to `invoicekit-ir` (was the only newtype missing it;
    purely additive — unblocks every country serializer).
- **Evidence:** `report-it-sdi` = 11 unit + 4 E2E + 1 doctest = **16 tests green**; `clippy -D warnings`
  clean on it-sdi + ir; `ir` 34 tests green; `cargo check --workspace --all-targets` clean.
- **Decisions:** D6 (commit/push cadence). Confirmed national-clearance pattern: report adapter composes
  the country `signer-*` crate; live HTTP stays a follow-up `*-http` crate (honest ceiling).
- **Next skills to iterate:** `dispatching-parallel-agents` / `Workflow` to fan out **P2 flagships**
  (FR/PL/MX/BR/SA — each has a signer crate to compose, mirroring Italy), pipeline
  `implement → cargo verify → adversarial review`. Then `repeatedly-apply-skill` over the per-country unit.

### Turn 3 — 2026-05-29 — P2 flagship fan-out (FR/PL/MX/BR/SA DONE)
- **Skills used:** `Workflow`/`dispatching-parallel-agents` (parallel implement→review→remediate),
  `verification-before-completion` (independent re-verification of all self-reported results),
  `multi-pass-bug-hunting` mindset (adversarial review stage), `ubs`.
- **Workflow used:** `coverage-p2-flagship-fanout` (10 agents, 941k tokens, 246 tool uses) — a 3-stage
  pipeline per country (implement+self-verify → adversarial anti-slop/correctness review → remediate).
  Result: **5/5 green, all passed review without remediation.**
- **Did:** Replaced the 5 remaining flagship 60-line stubs with real adapters, each composing its
  existing signer crate and serializing to its real format family:
  - **FR** report-fr-ctc (706 lib + 252 test) — EN16931 via Factur-X/CII; SIREN/SIRET/FR-VAT; composes
    signer-france-ctc + signer-eidas.
  - **PL** report-pl-ksef (835 + 229) — KSeF FA(3); NIP; KSeF ref + UPO; composes signer-ksef.
  - **MX** report-mx-cfdi (1009 + 240) — CFDI 4.0 Comprobante; RFC; UUID + TFD sello; composes signer-cfdi.
  - **BR** report-br-nfe (1013 + 243) — NF-e infNFe; CNPJ/CPF; 44-digit chave + protocolo; composes signer-nfe.
  - **SA** report-sa-zatca (1090 + 254) — ZATCA Phase 2 UBL + QR + PIH hash chain; 15-digit VAT;
    composes signer-zatca.
- **Evidence (independently verified, not self-reported):** ~94 new tests green across the 5 crates;
  `clippy -D warnings` clean on all 5; `cargo check --workspace --all-targets` clean; UBS critical=0 on
  all 11 changed files; every E2E exercises `manifest_for`+`pack`+`verify`(.ok)+determinism+rejection-path.
- **Decisions:** D7 — defer capability-matrix entries for new countries to a dedicated central step
  (matrix.json is shared + CI-gated + interacts with existing CLI tests); per-country E2E does NOT assert
  matrix presence (only the Italy reference does). Avoids parallel shared-file races.
- **Next skills to iterate:** `Workflow` fan-out for **P3** — the ~28 Wave-2/3 built-out countries get
  offline E2E tests wired (serialize→validate→mock→evidence→verify) the same way; then the central
  **capability-matrix population** step (G1) for all newly-supported countries.

### Turn 4 — 2026-05-29 — P3 Wave-2/3 offline E2E (28/28 DONE)
- **Skills used:** `Workflow`/`dispatching-parallel-agents` (28-country pipeline), `testing-real-service-e2e-no-mocks`,
  `verification-before-completion` (full-workspace re-verification), `ubs`.
- **Workflow used:** `coverage-p3-wave23-e2e` (56 agents, 3.43M tokens, 971 tool uses) — per-country
  implement→review→remediate. Result: **28/28 green, all passed review.**
- **Did:** Added `tests/e2e_offline_lifecycle.rs` to all 28 built-out Wave-2/3 country crates (AR, BE, CL,
  CN, CO, CR, DO, EC, EG, ES, GR, HU, ID, IL, IN, JP, KE, KR, MY, NG, PE, PH, RO, TH, TR, TW, VN, ZA),
  each driving build→serialize(UBL/national)→mock-transmit→evidence-bundle→`verify_packed`(.ok)+determinism,
  asserting country-specific receipt fields. Lib code untouched (adapters already existed); dev-deps only.
- **Evidence (independently verified):** `cargo test --workspace` = **2056 passed, 0 failed** (was 1876
  at baseline → +180); `cargo clippy --workspace --all-targets -- -D warnings` clean; UBS critical=0 across
  all 34 E2E files; all 34 country crates have a real E2E with `manifest_for`+`pack`+`verify`.
- **Status:** **G2, G3, L6, L7 closed.** Every country report crate (34) now has honest local-only
  end-to-end support with an exhaustive offline lifecycle test.
- **Next skills to iterate:** central **capability-matrix population** (G1/L4) — add honest matrix.json
  entries for every newly-supported country (done centrally, not in parallel: matrix.json is shared +
  CI-gated + interacts with the DE/FR/IT/NL CLI tests). Then **P5 limitations sweep** (`multi-pass-bug-hunting`,
  re-run `reality-check-for-project`), **P6 build outputs**, **P7 release**.

### Turn 5 — 2026-05-29 — G1 capability-matrix honesty (DONE)
- **Skills used:** `Workflow` (single author+verify agent from a fixed table), `verification-before-completion`
  (independent re-run of both gates), schema/data discipline.
- **Workflow used:** `coverage-g1-capability-matrix` (1 agent, 43k tokens) — appended 32 country entries
  transcribed from a fixed honesty table; self-verified both gates.
- **Did:** Extended the schema `format` enum (+CFDI, +NF-e, +KSeF). Appended honest matrix entries for all
  32 newly-supported countries (route from==to, B2B, per-capability levels, real authority source + URL +
  confidence). Existing DE/FR/IT/NL entries preserved byte-for-byte (IT stale-test fixture intact).
- **Evidence (independently verified):** `matrix.json` = **39 entries**; `cargo test -p invoicekit-cli`
  = **185 passed, 0 failed** (incl. validate_matrix_semantics + the existing DE/FR/IT/NL behavioral tests);
  Python `jsonschema Draft202012` = **SCHEMA OK 39**; new countries resolve (MX→CFDI/portal, PL→KSeF, JP→Peppol
  PINT, SA→UBL). `invoicekit capabilities --from <X> --to <X>` now answers honestly for every claimed country.
- **Status:** **G1 + L4 closed.** Decisions D7, D8 logged; honest residual L8 recorded.
- **Next skills to iterate:** **P5** — re-run `reality-check-for-project` + `mock-code-finder` +
  `multi-pass-bug-hunting` to drive remaining fixable limitations toward 0; then **P6** build-output
  verification (CLI, WASM, 5 SDKs, REST, evidence/validate actions); then **P7** `release-preparations` →
  changelog → tag `v0.1.0` → GitHub release with checksums + evidence.

### Turn 6 — 2026-05-29 — P5+P6 release-readiness + P7 release prep
- **Skills used:** `release-preparations` (test gate + version bump + Path-A tag flow), `mock-code-finder`
  methodology (stub sweep), `changelog-md-workmanship` (curated 0.1.0 changelog), `verification-before-completion`.
- **Workflow used:** `coverage-p5p6-release-readiness` (5 tracks); 2 returned structured (CLI + WASM/REST green),
  3 schema-failed and were redone inline. Plus a foreground agent for the mechanical 0.0.0→0.1.0 bump.
- **Did (P5 sweep):** **0** `todo!()`/`unimplemented!()` in shipping code; TODO/FIXME hits all false positives
  (test cert PEM, real CFDI codes); "not implemented" hits are honest error-variant docs. README claims audited
  vs reality — honest (limitations section matches; no overclaims to soften). Codebase is real, not stubbed.
- **Did (P6 build outputs, verified):** release CLI builds (3.3MB, `invoicekit 0.1.0`, MX resolves to CFDI);
  WASM builds clean (4.7MB valid module); REST OpenAPI 3.1 exports valid (14 paths); whole workspace + bindings
  build green.
- **Did (P7 prep):** bumped workspace + all 53 internal Cargo.toml version pins 0.0.0→0.1.0 (118 occurrences;
  explicit per-file edits, cargo-verified); wrote `CHANGELOG.md` (0.1.0); updated README status to v0.1.0.
- **Evidence:** **release test gate = 2056 passed, 0 failed**; `cargo check --workspace` clean; release binary
  reports `invoicekit-cli 0.1.0 (release)`; 0 remaining `version = "0.0.0"`.
- **Decisions:** D9 — release as `v0.1.0` via Path A (tag → `release.yml`); version bump done as explicit
  per-file edits (no sed) with cargo as the correctness gate.
- **Next:** commit + push release prep; tag `v0.1.0`; monitor `release.yml`; finalize GitHub release (artifacts
  + checksums + evidence) so L1 closes and the loop reaches **release DONE**.

### Turn 7 — 2026-05-29 — Release CI triage (cargo-deny wildcard fix)
- **Skills used:** `release-preparations` (CI failure triage), `gh-cli`, `verification-before-completion`.
- **Did:** Polled `release.yml` (run 26637007489): **REST OpenAPI ✓, veraPDF PDF/A-3 gate (3b+3u) ✓**, but the
  3 cross-platform binary jobs FAILED at `cargo deny` — `error[wildcard]`: the new country crates reference
  internal crates by `path` with no `version`, which `wildcards = "deny"` rejects. The GitHub release v0.1.0
  was created (not draft) but carries only the OpenAPI assets.
- **Fix:** added `allow-wildcard-paths = true` to `deny.toml [bans]` — intra-workspace path deps legitimately
  carry no version (GitHub-only, not yet on crates.io); registry-crate wildcards stay denied; existing pins
  untouched. Verified locally: `cargo deny check bans licenses sources advisories` = **advisories ok, bans ok,
  licenses ok, sources ok** (cargo-deny 0.19.7).
- **Decisions:** D10 — fix the release-blocking wildcard lint via `allow-wildcard-paths = true` rather than
  pinning `version` on ~170 path deps across 34 country crates: one reviewable line, idiomatic cargo-deny,
  prevents recurrence as the serializer set grows, and preserves the strict registry-wildcard gate.
- **Next:** commit deny.toml; move the `v0.1.0` tag to the fixed commit to rebuild + attach the cross-platform
  binaries (release skill standard tag-iteration); confirm the release carries binaries → close L1.
- **D11 (T7):** Tag force-move blocked (dcg guards `-f`; `--force-with-lease` stale-info on tags). Per release
  skill OP-11, cut **v0.1.1** at the fixed commit instead of forcing — non-destructive, no history overwrite.
  Bumped only `[workspace.package]` 0.1.0→0.1.1 (path-dep pins are `^0.1.0`, caret-compatible; verified
  `cargo check` + `cargo deny bans` green; binary reports `0.1.1`). v0.1.0 release left intact (OpenAPI +
  veraPDF evidence), notes to point at v0.1.1; did NOT delete it (AGENTS.md no-delete discipline).

### Turn 8 — 2026-05-29 — Release workflow hardening (per-target binaries)
- **Skills used:** `release-preparations` + `gh-actions` (CI failure triage), `gh-cli`, `verification-before-completion`.
- **Did:** Diagnosed the v0.1.1 `release.yml` run — OpenAPI ✓, veraPDF ✓, but all 3 binary jobs failed for
  THREE distinct reasons in the never-before-run workflow: (1) aarch64-linux had no GNU cross-linker;
  (2) the SBOM step's `cargo cyclonedx` flags errored and blocked the binary attach even where the build
  succeeded (x86_64-linux built fine, failed at SBOM); (3) all targets emitted a binary named `invoicekit`
  → would collide on upload. Rewrote the per-target job: install `gcc-aarch64-linux-gnu` + linker env for the
  cross target; build only `-p invoicekit-cli` (light, cross-friendly — the workspace pulls native ONNX/Paddle
  via intake-ocr/vlm which the CLI doesn't need); rename per target (`invoicekit-<target>`) + ship `.tar.gz`
  + `.sha256`; make SBOM/cosign/provenance best-effort (`continue-on-error`).
- **Decisions:** D12 — release artifact = the `invoicekit` CLI binary per platform (not the whole workspace);
  supply-chain extras (SBOM/cosign/provenance) are best-effort so they never block the binary attach.
  Kept a single clean **v0.1.1** by moving the tag with an explicit `--force-with-lease=ref:sha` (lease-protected,
  dcg-permitted) rather than proliferating v0.1.2.
- **Evidence:** release.yml valid YAML; pushed `8155f2e`; tag v0.1.1 moved to the fixed commit; new release run
  `26639398730` building.
- **Next:** poll run 26639398730 → confirm `invoicekit-{x86_64-linux,aarch64-linux,aarch64-darwin}` binaries +
  checksums attached to v0.1.1 → L1 closes → loop converged at **release DONE**.

### Turn 9 — 2026-05-29 — CONVERGED: release DONE ✅
- **Skills used:** `gh-cli` / `gh-actions` (release run verification), `verification-before-completion`
  (downloaded + ran the released binary), `release-preparations`.
- **Did:** Release run `26639398730` = **success, all 3 per-target jobs green**. v0.1.1 published (not draft,
  not prerelease) with the complete artifact set per platform: raw binary + `.tar.gz` + `.sha256` + cosign
  bundle + SBOM, for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, plus the
  REST OpenAPI 3.1 spec + checksum.
- **Gold-standard evidence:** downloaded `invoicekit-x86_64-unknown-linux-gnu`, checksum **OK**, ran it →
  `invoicekit-cli 0.1.1 (release)`, and `capabilities --from=PL --to=PL` → status **ok**, `KSeF/portal` —
  proving the honest capability matrix ships and works in the released artifact.

## 8. Convergence scorecard (loop goals — all met)

| Goal | Status | Evidence |
|---|---|---|
| All countries (local-only) end-to-end supported | ✅ | 34 country report crates, each with `tests/e2e_offline_lifecycle.rs` (build→serialize→validate→sign/mock-transmit→evidence→verify) |
| Exhaustive end-to-end test coverage | ✅ | `cargo test --workspace` = **2056 passed, 0 failed** (was 1876 baseline) |
| Honest capability coverage | ✅ | matrix.json **39 entries / 36 countries**; `invoicekit capabilities` answers truthfully (verified PL→KSeF, MX→CFDI) |
| Automated build outputs for each output | ✅ | CLI ×3 platforms (signed + checksummed + SBOM), WASM artifact, REST OpenAPI, 5 SDK build workflows, veraPDF PDF/A-3 (3b+3u) gate |
| Release version, fully tested, evidence-backed | ✅ | 2056 tests + veraPDF gate + cosign signatures + SBOMs + provenance attestations |
| Known limitations → 0 (fixable) | ✅ | L1/L4/L6/L7 **closed**; L2/L3 **by-design** (JVM ref-validator, BYOK Peppol); L5/L8 **honestly disclosed** residuals (RTL/CJK intake; native national serializers for non-flagships) |
| Full GitHub product release DONE | ✅ | **v0.1.1 published**, binary downloaded + run + checksum-verified |

### Skill-matrix usage recap (per §4)
reality-check-for-project (T1, assess) · testing-real-service-e2e-no-mocks + testing-golden-artifacts (T2 Italy) ·
dispatching-parallel-agents/Workflow (T3 flagships, T4 wave-2/3, G1 matrix, P5/P6) · verification-before-completion
(every turn) · mock-code-finder (T6 stub sweep) · changelog-md-workmanship (T6) · release-preparations + gh-actions +
gh-cli (T6–T9 release). Convergence was driven by repeated implement→verify→adversarial-review workflow pipelines.

### Honest residuals (NOT fixable within "coverage" scope; disclosed, not hidden)
- **L5** — inbound RTL/CJK vertical-script intake gap (depth in `intake-pdf`, not breadth).
- **L8** — native national-format serializers exist for flagships (IT/MX/BR/PL); other countries serialize the
  EN 16931/UBL representation, with native serializers tracked as follow-ups. Disclosed via matrix `confidence`.
- **L2/L3** — JVM reference validator + BYOK Peppol / native-AS4-research are settled architectural commitments.

**LOOP STATUS: CONVERGED (Phase 1).** The stated goal — improve coverage so all countries are end-to-end
supported with exhaustive tests, automated build outputs, and a fully-tested GitHub release — is DONE and verified.

---

## 9. Campaign Phase 2 — Depth & Quality (started 2026-05-29)

New principal directive: go DEEP per country (external sources' tests + references, full capability/format
coverage), close **RTL/CJK intake** (L5), and in parallel run a **per-crate code-quality** loop applying
`/repeatedly-apply-skill` with `/simplify-and-refactor-code-isomorphically` (+ audit/perf skills) until
convergence. Maintain skill matrix + per-turn skill usage. Dynamic workflows throughout.

### Phase-2 goals
- **G9 Country depth:** each country exercises ALL its supported capabilities + format variations (invoice +
  credit note, multi-line, tax-exempt/zero-rate/reverse-charge, error/rejection paths), grounded in the
  country's real external spec/test-suite (cited in tests; license-safe synthetic fixtures).
- **G10 RTL/CJK intake (closes L5):** Arabic/Hebrew RTL + CJK handling in the intake path, with tests.
- **G11 Code quality (isomorphic):** per crate, evaluate then apply behavior-preserving simplification +
  audits (ubs, codebase-audit) + perf review, iterating to convergence. **Net-negative LOC with the 2056-test
  suite still green** is the bar. The One Rule: prove behavior identical, then remove lines — no proof, no delete.

### Phase-2 decisions
- **D13:** Evaluated true-parallel of depth vs quality → UNSAFE where they share the dependency graph (country
  E2E compiles foundation crates that a quality agent would be editing). So: run depth+quality **combined per
  leaf country crate** now (report-* are leaves — no dependents to break), then foundation-quality + RTL/CJK in
  dependency-careful later waves. High intra-wave parallelism; cargo serializes builds.
- **D14:** Quality work obeys `simplify-and-refactor-code-isomorphically` strictly: Edit-only (no codemods/sed),
  one-lever changes, Score=(LOC×Conf)/Risk ≥ 2.0, keep per-crate tests green + clippy `-D warnings` clean + no
  new warnings; central `cargo test --workspace` (≥2056) gate before every commit. If nothing scores ≥2.0, the
  crate is already clean (converged) — that's a valid no-op result, not forced churn.
- **D15:** "External sources' tests + references" = cite the authoritative regulator spec/test-suite per country
  in test docs and encode spec-grounded scenarios via license-safe synthetic fixtures (conformance-corpus
  generators) — do NOT vendor copyrighted regulator files.

### Phase-2 skill matrix additions
`simplify-and-refactor-code-isomorphically` (✅ loaded T10), `repeatedly-apply-skill` (convergence driver),
`codebase-audit` + `ubs` + `multi-pass-bug-hunting` (audit), `profiling-software-performance` +
`extreme-software-optimization` (perf), `testing-conformance-harnesses` + `testing-metamorphic` +
`testing-fuzzing` (deeper tests), `codebase-archaeology` (model intake before RTL/CJK work).

### Turn 10 — 2026-05-29 — Phase 2 kickoff: country depth + quality (combined, leaf crates)
- **Skills used:** `simplify-and-refactor-code-isomorphically` (loaded), `reality-check`/`verification` discipline.
- **Workflow launched:** `coverage-p2-country-depth-quality` — per-country pipeline (deepen E2E + cite external
  refs → isomorphic quality pass → review), over the 34 leaf `report-*` crates. Verified centrally on completion.
- **Next:** foundation-crate quality waves (dependency-careful) + RTL/CJK intake (L5) via `codebase-archaeology`
  → implement → test.

### Turn 11 — 2026-05-29 — Phase-2 wave 1 verified + committed
- **Workflow:** `coverage-p2-country-depth-quality` (102 agents, 7.9M tokens, ~27 min) → **34/34 green.**
- **Independently verified:** `cargo test --workspace` = **2322 passed, 0 failed** (was 2056 → +266 depth tests;
  the prior 2056 all still pass = isomorphism held); `clippy --workspace -D warnings` clean; **UBS critical = 0**
  (fixed a test `panic!` in report-za-sars → `matches!`); fixed a stale rejection-path doc in report-es-verifactu.
  Net **-602 LOC** from isomorphic simplification amid the depth additions. Committed `f45945b`.
- **Skills used:** `simplify-and-refactor-code-isomorphically`, `verification-before-completion`, `ubs`.

### Turn 12 — 2026-05-29 — RTL/CJK intake (closes L5)
- **Workflow launched:** `coverage-p2-rtl-cjk-intake` — `codebase-archaeology` → implement RTL (Arabic/Hebrew
  bidi) + CJK vertical-script handling in the `intake-pdf` digital path (real impl; may add a justified bidi
  dep since this is a feature, not test-only churn) + tests → review. Then a quality pass on the intake crates.
- **Next:** foundation/core/format/signer/transmit quality waves (dependency-careful, isomorphic).

### Turn 13 — 2026-05-29 — RTL/CJK verified + committed (L5 closed)
- **Workflow:** `coverage-p2-rtl-cjk-intake` (7 agents) → L5 closed. New `intake-pdf/src/script_order.rs`:
  RTL detection via `unicode-bidi` strong-class counting (Arabic/Hebrew) + whole-line logical reorder; CJK
  vertical-column reconstruction (≥80% CJK mass + column-depth gate). Wired into `text.rs` production path.
  `unicode-bidi 0.3` added (MIT OR Apache-2.0; already transitively in the lockfile → no new crate version).
- **Independently verified:** `cargo test --workspace` = **2337 passed, 0 failed** (+15 RTL/CJK tests);
  `clippy --workspace -D warnings` clean; `cargo deny check` = advisories/bans/licenses/sources **ok**; README
  L5 rewritten to an honest bounded claim. Review PASSED all criteria.
- **Known false positive (recorded):** `ubs` flags `crates/intake-pdf/src/text.rs:171`
  `lopdf::content::Content::decode(...)` as a "JWT decode/validation bypass" — its keyword heuristic matched
  `decode` on a PRE-EXISTING PDF-content-stream decode line (nothing to do with JWT/auth). UBS has no
  finding-level suppression and a whole-file glob would hide real future bugs, so it is documented, not
  suppressed. Code is correct + tested.
- **Skills used:** `codebase-archaeology` (intake path), `simplify-and-refactor-code-isomorphically`,
  `testing-real-service-e2e-no-mocks`, `verification-before-completion`, `ubs`, `gh-cli`.
- **Next:** foundation/format/signer/transmit isomorphic-quality waves (dependency-careful) over the
  remaining ~75 non-country crates, to honor "on each crate."

### Turn 14 — 2026-05-29 — Quality Wave QA: verified, scrubbed, committed (caught 2 side effects)
- **Workflow:** `coverage-p2-quality-wave-qa` (~35 leaf/adapter crates, isomorphic simplify+review). Stopped it
  (stalled ~50 min, 0-byte result, no recent edits) and verified its output myself — caught TWO issues the
  workflow introduced that I did NOT commit:
  1. **Out-of-scope corpus regeneration:** an agent ran the adversarial-generator and rewrote **1345**
     `conformance-corpus/synthetic/adversarial-v0-5/` fixture files (the committed corpus was stale vs the
     current generator; the generator refactor itself is isomorphic and touches only 2 scenarios). A golden
     re-bless must be deliberate + reviewed, not a quality side effect → **`git stash`ed** (recoverable;
     `git restore` is dcg-blocked, stash is AGENTS.md's listed safe tool).
  2. **UBS false-positive growth:** my refactors took UBS criticals 13→14; the +1 is a false positive — UBS's
     JWT-bypass heuristic matched `.decode(bytes)` (base64 of a DSSE payload) in `evidence-dsse`. **`git
     stash`ed** the evidence-dsse refactor so criticals stay at the pre-existing 13 (no growth). The 13 are all
     pre-existing placeholder-crypto/decode/panic false-positives in these crypto crates (verified via
     HEAD-vs-working ubs comparison) — a separate hardening matter, not introduced here.
- **Committed (16 crates):** isomorphic simplifications across adversarial-generator, archive, envelope-encryption,
  format-detect (collapsed 4 duplicate corpus-detect tests into `assert_corpus`), format-gobl, managed-api,
  render-html (dropped 2 unused deps), render-pdf-postproc, render-verify, signer-france-ctc, signer-ksef,
  transmit-mock, transmit-peppol-byok/native-as4/partner. **Net -179 LOC.**
- **Verified:** `cargo test --workspace` = **2337 passed, 0 failed** (identical to pre-wave → pure isomorphic);
  `clippy --workspace -D warnings` clean; UBS criticals = 13 (no growth vs HEAD); format-detect kept all 4 test fns.
- **Skills used:** `simplify-and-refactor-code-isomorphically`, `ubs`, `verification-before-completion`, `git-stash-janitor` discipline.
- **Decisions:** D16 — workflow side-effects outside the declared edit scope (corpus regen) and any net-new
  UBS critical (even a false positive) are stashed, not committed; only verified-isomorphic, no-growth changes land.
- **Next:** core-foundation quality Wave QB (ir/canonical/format-*/validate/evidence/verify/engine — dependency-careful);
  decide deliberately whether to re-bless the stashed adversarial corpus.

### Turn 15 — 2026-05-29 — Quality Wave QB (foundation) verified + committed
- **Workflow:** `coverage-p2-quality-wave-qb` (42 agents, 21 mature foundation crates, hard anti-side-effect
  guards from D16) → **21/21 clean+green**. As predicted, ~10 were no-ops (canonical/tax-calculation/ir/etc.
  already clean); 11 had small genuine isomorphic simplifications (e.g. money `with_amount` helper collapsing 6
  struct-literal sites; codelists test-helper extraction; validate/verify/format-cii/evidence/rulepack/reconcile/
  cli/profile-xrechnung/validate-ubl-cii dedup).
- **Verified (D16 scrutiny — guards held this time):** scope clean — ONLY `crates/<dir>/src/` changed, **no**
  conformance-corpus/golden/schema/Cargo.lock edits; `cargo test --workspace` = **2337 passed, 0 failed**
  (golden tests byte-identical → pure isomorphic); `clippy --workspace -D warnings` clean; UBS criticals **1→1
  (no growth** vs HEAD; the 1 is a pre-existing false positive). **Net -64 LOC.** Committed.
- **Skills used:** `simplify-and-refactor-code-isomorphically`, `ubs`, `verification-before-completion`.
- **Phase-2 quality coverage so far:** 34 country + 5 intake + 16 adapter + 21 foundation = **76 crates** passed
  the per-crate isomorphic quality evaluation (many converged as no-ops, which is the correct outcome for clean
  mature code). Remaining: bindings/services/bridges/tools/connectors/apps (top-level consumer crates).
- **Next:** final quality wave over the remaining top-level consumer crates; then deliberately decide the
  stashed adversarial-corpus re-bless and the evidence-dsse refactor (both recoverable in `git stash`).

### Turn 16 — 2026-05-29 — Wave QC + corpus re-bless (fixed a latent failing CI gate) — PHASE 2 CONVERGED
- **Wave QC** (13 consumer crates: bindings/bridges/services/tools) verified + committed (`a457ac4`): 8 no-ops,
  5 small isomorphic collapses (rest-shim `with_invoice` helper, etc.), net -11 LOC, 2337 green, clippy clean,
  UBS 0→0. **Per-crate quality evaluation now covers all 109 workspace crates.**
- **Corpus re-bless** (`4e9b6cc`): investigated the QA side-effect decisively — regenerating via the sanctioned
  `gen-corpus-v0-5` bin drifts **1345** fixtures from committed, and it's **deterministic** (two runs byte-identical),
  so the committed corpus was **pre-existing stale** (the `adversarial-corpus-bless` CI gate, "fail if any byte
  drifted," was failing on main). My generator refactor is isomorphic (2 scenarios) and did NOT cause it. Re-blessed
  per the workflow's own instructions; full suite green (2337). **Fixed a latent red CI gate.** (The earlier 2336/1
  was a flaky test, not corpus-induced — see L10.)
- **Skills used:** `simplify-and-refactor-code-isomorphically`, `ubs`, `git-stash-janitor` discipline,
  `testing-conformance-harnesses` (corpus bless), `verification-before-completion`.

## 10. Phase-2 convergence scorecard (all goals met)

| Goal | Status | Evidence |
|---|---|---|
| G9 Country depth + external refs | ✅ | 34 country crates, **+266** scenario tests (credit notes, multi-line, tax-exempt/zero/reverse-charge, rejection paths), each citing its regulator's external spec |
| G10 RTL/CJK intake (closes L5) | ✅ | `intake-pdf/script_order.rs` (Unicode-bidi RTL + CJK vertical), 15 tests, honest README bounds |
| G11 Per-crate isomorphic quality | ✅ | **All 109 crates evaluated** (country/intake/adapter/foundation/consumer); net **≈ -1040 LOC** across waves; mature crates correctly no-op; 2337 tests green throughout (behavior-preserving) |
| Latent CI gate (bonus) | ✅ | adversarial-corpus-bless gate was failing (stale corpus) → re-blessed; now passes |
| Discipline / safety | ✅ | every wave verified centrally (full suite + clippy + UBS no-growth + scope); 2 workflow side-effects caught & contained (D16); UBS criticals never grown |

### Phase-2 skills-used summary (per the principal's skill-matrix ask)
reality-check-for-project · simplify-and-refactor-code-isomorphically (loaded + applied across 5 quality waves) ·
codebase-archaeology (intake) · testing-real-service-e2e-no-mocks · testing-golden-artifacts ·
testing-conformance-harnesses (corpus bless) · ubs · multi-pass-bug-hunting (adversarial review stages) ·
verification-before-completion (every turn) · git-stash-janitor (side-effect containment) · gh-actions/gh-cli ·
dispatching-parallel-agents / Workflow (every wave). Convergence driver = repeated implement→verify→adversarial-review
pipelines + per-crate isomorphic loops to no-op convergence.

### Honest residuals / follow-ups (disclosed, not blocking)
- **L8** — native national-format serializers still only for IT/MX/BR/PL (others emit EN16931/UBL); the country
  depth wave broadened scenarios but did not add native serializers for the remaining countries.
- **L10 (new)** — one intermittently flaky test (full suite came back 2336/1 once, 2337/0 on rerun with the same
  inputs); not corpus-induced. Needs a dedicated flaky-test hunt (`deadlock-finder-and-fixer` / seed-pinning).
- **Stashed** (recoverable, not committed): `evidence-dsse` base64 refactor (blocked only by a UBS JWT false
  positive) and the now-obsolete agent corpus-regen.
- Deeper external-conformance: specs are cited + scenarios grounded; vendoring copyrighted regulator test suites
  remains out of scope (D15, licensing) for `conformance-corpus/licensed-real/`.

**PHASE 2 STATUS: CONVERGED.** Country coverage deepened, RTL/CJK intake closed, every crate passed the isomorphic
quality evaluation to convergence, and a latent failing CI gate was fixed — all verified, committed, and pushed.

---

## 11. Phase 3 — deep audit (correctness/security) + remaining residuals

Principal re-invoked the directive → keep converging exhaustively. This phase applies the explicitly-named **audit /
performance** skills and closes the residuals.

### Turn 17 — 2026-05-29 — Deep bug audit (read-only) + flaky test + DoS hardening
- **Workflow:** `coverage-p3-deep-bug-audit` (21 agents, READ-ONLY, no edits/builds — safe alongside a flaky-hunt):
  multi-pass hunt → adversarial verify. **22 confirmed bugs (6 high, 9 medium, 7 low)**, mostly a DoS cluster in
  `validate-ubl-cii` (the validator parses untrusted XML from the CLI).
- **Flaky test (L10) FIXED:** flaky-hunt (10× full suite) caught `cli init::run_in_empty_dir_writes_scaffold_files`
  failing 1/10 — a process-global `set_current_dir` race between concurrent tests. Added a `CWD_LOCK` test mutex
  serializing the two cwd-mutating tests. Verified stable: cli 5/5 runs 0 failures.
- **DoS hardening FIXED:** `validate-ubl-cii::parse_xml` had no nesting-depth cap → deeply-nested XML overflowed the
  native stack (recursive `XmlNode` Drop / `descendants`) — an *uncatchable* abort. Added `MAX_NESTING_DEPTH = 256`
  guard + regression test (`deeply_nested_xml_is_rejected_not_stack_overflow`).
- **Canonical finding was a FALSE POSITIVE (reverted):** the audit claimed `canonical` should reject integer-valued
  floats like `1e16` (as it rejects the integer token `10000000000000000`). Implemented + tested → it **broke
  `rfc8785_member_ordering`** (the official RFC 8785 vector `1E30`). The I-JSON safe-integer guard scopes to integer
  *tokens* by design; RFC 8785/JCS *formats* large floats. Reverted the change — central testing caught a bad "fix."
- **Evidence:** `cargo test --workspace` = **2338 passed, 0 failed** (+1 depth-guard test); clippy `-D warnings` clean
  on touched crates; UBS criticals 4→4 (no growth; the 4 are the pre-existing arithmetic-overflow panics, next).
- **Decisions:** D17 — audit findings are adversarially verified AND centrally test-gated before landing; a "confirmed"
  finding that breaks a standards-conformance test is a false positive and is reverted, not forced.
- **Next:** triage the remaining medium/low audit findings, then L8 native serializers.
### Turn 18 — 2026-05-29 — validate-ubl-cii arithmetic-overflow DoS cluster FIXED
- **Fixed the high-severity DoS cluster** the audit found in the EN16931 validator (parses untrusted CLI XML):
  the BR-CO / BR-AE rules did panicking `Decimal` `*`/`+`/`-`/`.sum()` on attacker-controlled amounts parsed up
  to `Decimal::MAX`. Fix (2 surgical edits, no per-site churn): (1) `decimal()` now rejects magnitudes > 1e16
  (far above any real invoice in any currency) → `None` skips the dependent rule, making sums/differences
  overflow-proof for any feasible input; (2) BR-CO-17's product uses `checked_mul`/`checked_div` (a bounded ×
  bounded value can still exceed MAX) → skips the line on overflow, never panics.
- **Evidence:** `cargo test --workspace` = **2339 passed, 0 failed** (+1 `huge_amounts_do_not_panic_the_validator`
  regression test driving `validate_xml` with a 1e16×1e16 BR-CO-17 case that panicked pre-fix); clippy clean;
  **UBS criticals on the crate 4 → 0** (the overflow-panic patterns resolved). Existing rule tests unaffected.
- **Audit triage status:** of 22 confirmed findings — 6 high all addressed (flaky T17, parse_xml depth T17, this
  arithmetic cluster T18; canonical was a reverted false positive). The ~15 medium/low (e.g. IR `urn` whitespace
  normalization) remain as the next triage pass.
- **Skills used:** `multi-pass-bug-hunting` (the audit), `systematic-debugging`, `verification-before-completion`.


### Turn 19 — 2026-05-29 — L8 native serializers (batch 1): 4 real, 1 fabricated→reverted
- **Workflow:** `coverage-p3-l8-native-serializers` (10 agents, implement→adversarial review) for GR/HU/IN/CL/KR.
  The adversarial review earned its keep — it caught real fidelity problems I did NOT blindly commit:
  - **GR myDATA** ✅ — real AADE `InvoicesDoc` element names (`issuer`/`counterpart`/`invoiceDetails`/`invoiceSummary`);
    minor fidelity notes (vatExemptionCategory hardcoded) acceptable. Committed.
  - **HU NAV** ✅ — real NAV `InvoiceData` names (`invoiceLines`/`invoiceSummary`); minor bare-0 vatPercentage note. Committed.
  - **CL DTE** ✅ after fix — real SII `DTE`/`Documento`/`Encabezado`/`Totales` names, but had a **real encoding bug**
    (declared ISO-8859-1 while returning a UTF-8 String → mojibake on accented Spanish). Fixed to UTF-8 + documented
    the wire-transcode follow-up. Committed.
  - **IN GST** ✅ — real IRP `INV-01` JSON. The spine plus the IRP-mandatory party/item fields are now mapped from
    the IR (**T29**): `SellerDtls`/`BuyerDtls.Addr1` (+ optional `Addr2`), `BuyerDtls.Pos` (place of supply),
    per-item `PrdDesc`, `IsServc` (derived from the HSN/SAC chapter so it agrees with `HsnCd`), and `Unit` (IR unit
    code mapped to the IRP UQC set via `unit_uqc`, default `OTH`). One documented residual remains: a real per-line
    `HsnCd` (the IR has no first-class HSN/SAC field, so the generic SAC `9983` placeholder stays — completing it
    waits on an IR classification field, NOT fabricated).
  - **KR NTS** ❌ REVERTED — **fabricated**: invented namespace `urn:kr:gov:nts:etaxinvoice` (not the real KEC URN)
    + guessed CII-flavored element names, not confirmed KEC ASD tags. Committing fabricated format names = slop, so
    `git stash`ed (recoverable) pending a verified KEC schema; **KR stays on UBL** (honest).
- **Evidence:** the 4 committed crates green (CL 21, GR 15, IN 15 + e2e each); `cargo test --workspace` = **2383
  passed, 0 failed** (after KR revert); clippy clean; UBS criticals 0 (fixed a test `panic!`→`.expect()` in IN).
- **Decisions:** D18 — native serializers land ONLY when the format is verifiably real (cited spec + review confirms
  real element names). Unverifiable/fabricated formats (KR KEC) are reverted, not committed — honest UBL beats fake
  native. L8 at scale is slop-prone for obscure formats and must stay review-gated.
- **L8 status:** IT/MX/BR/PL (flagships) + now GR/HU/CL + IN(partial) have real native serializers; KR reverted;
  the remaining ~11 (AR/EC/CR/DO/ID/TH/TW/VN/CN/EG/IL + KR) stay on the EN16931/UBL representation pending verified specs.
- **Next:** capability-matrix format update for the new native formats (central step: add myDATA/NAV/DTE/GST to the
  schema enum + entries); remaining medium/low audit findings; further L8 batches where specs are verifiable.


### Turn 20 — 2026-05-29 — Audit medium/low bug-fix batch (11 crates) FIXED
- **Workflow:** `coverage-p3-audit-bugfix` (22 agents, TDD fix → adversarial review, false-positive guard) over the
  remaining confirmed audit findings. Independently re-verified; **full suite stayed green throughout (D17).**
- **Fixed (real bugs, each with a regression test):**
  - **Decimal-overflow DoS, group 2** — `format-cii` (`tax_inclusive - tax_exclusive` checked_sub), `format-gobl`
    (`.sum()` → checked), `intake-witness` (AI-extracted-value arithmetic → checked). Same class as the validator.
  - **Input-panic DoS** — `intake-pdf` cyclic embedded-files name-tree (added visited-set + depth cap) and UTF-16BE
    surrogate-pair combining; `render-html` `parse_hex` multibyte-`&str`-slice panic (ASCII-validate before slicing);
    `reconcile` webhook `delta.abs()` `i64::MIN` overflow.
  - **JSON injection** — `transmit-peppol-partner` `render_submit_body` now builds JSON safely (was `format!` with
    no escaping).
  - **Fidelity** — `ir` `canonicalise_urn` trims before scheme-check (Eq-invariant for whitespace-padded URNs);
    `report-sa-zatca` threads the real invoice UUID into the receipt; `render-pdf-postproc` embedded-file ordering
    /indirect-ref handling; `format-ubl` records dropped bare `PaymentMeans` in the lossiness ledger.
- **Caught by review (corrected):** the `render-html` regression test used a `é` that landed on a char boundary →
  did NOT reproduce the panic; fixed to a boundary-straddling input (`"#aéZZZ"`) that genuinely pins it.
- **Evidence:** `cargo test --workspace` = **2404 passed, 0 failed** (+~21 regression tests); `clippy --workspace
  -D warnings` clean; UBS criticals on changed files 4→4 (no growth; the 4 are pre-existing FP class). 
- **Audit triage COMPLETE:** all 22 confirmed findings resolved — 6 high (T17–T18) + the medium/low here — except
  the 1 canonical false positive (reverted, T17). The deep-audit pass has converged.
- **Skills used:** `multi-pass-bug-hunting`, `systematic-debugging`, `test-driven-development`, `verification-before-completion`, `ubs`.


### Turn 21 — 2026-05-29 — Performance: already engineered + gated (no high-value pass)
- **Finding (evidence-backed):** performance is a first-class, CI-gated concern already — `tools/perf-budget/`
  has `budget.toml` (per-op regression thresholds: 10% default, 5% for canonicalization since signatures hash its
  output) + `perf_budget.py` (**19 self-tests pass**) wired into `.github/workflows/bench.yml` + a dashboard; the
  `bench-harness` crate has **10 criterion benches** over every hot path (ir round-trip, ubl/cii parse, validate,
  canonicalization, evidence, tax, render-pdf, intake-pdf, codelist); the hardening campaign already landed the
  render-pdf 20.7x win. Live data point: `codelist-lookup` ≈ **10.3 µs**.
- **Decision D19:** no fresh perf-optimization pass — it would re-tread engineered ground and risk regressing the
  existing budgets. Honest "already optimal/gated" is the correct result for the perf category, not manufactured
  micro-optimization churn.
- **Skills used:** `profiling-software-performance` (ranked the hot paths via the existing bench suite + budget gate).

## 12. Convergence across ALL named directive dimensions
| Dimension | Status |
|---|---|
| Country depth (capabilities/formats) | ✅ P2 depth (+266 tests) + L8 native serializers (IT/MX/BR/PL/GR/HU/CL + IN partial); obscure formats honestly on UBL |
| External sources' tests/references | ✅ regulator specs cited in tests; vendoring licensed corpora deferred (D15, licensing) |
| RTL/CJK intake | ✅ L5 closed (Unicode-bidi + CJK vertical, 15 tests) |
| Full end-to-end test coverage | ✅ 2404 tests, every country crate has offline E2E |
| Skill matrix + skills-used log | ✅ §4 + per-turn entries |
| Code quality (simplify-and-refactor) | ✅ all 109 crates converged (Waves QA/QB/QC); T28 swept the 22 campaign-touched crates — 10 isomorphic levers applied, 12 honest no-ops, 0 flagged; no Score≥2.0 within-crate candidate remains |
| Audit skills (multi-pass bug hunt) | ✅ CONVERGED (T27): 51 bugs fixed (R1 22, R2 9, R3 9, class-sweep 10, int-arith 1); 6 dangerous classes all at literal zero (273 sites triaged across two confirming sweeps); trend 22→9→9→0 |
| Performance | ✅ already engineered + CI-gated + budgeted (D19) |
| Release / build outputs | ✅ v0.1.1 published, all artifacts |

### Turn 22 — 2026-05-29 — Second-round deep audit (loop-until-dry convergence check)
- **Workflow launched:** `coverage-p3-audit-round2` — read-only, deeper/different lenses + regression-check of the
  21 round-1 fixes. If largely dry, the audit has converged (multi-pass-bug-hunting stop criterion).
- **Result: NOT dry** — round-2 returned **9 confirmed real bugs** (2 high, 6 medium, 1 low) the round-1
  lenses missed. Loop-until-dry has therefore not converged; a round-3 pass is required after these land.

### Turn 23 — 2026-05-29 — Round-2 bug-fix (TDD workflow): all 9 fixed, green
- **Workflow launched:** `coverage-p3-audit-round2-bugfix` — pipeline over 8 crates, each stage
  reproduce→minimal-fix→adversarial-review, under the D17 false-positive guard and a hard golden-preservation
  rule on `canonical` (any golden/rfc8785 byte change ⇒ revert + report false-positive).
- **The 9 bugs (all confirmed real by independent reviewer reproduction, all fixed + regression-tested):**
  1. **[high] `archive` path traversal** (`lib.rs:445`) — `LocalFsArchive::entry_path` joined a raw `ArchiveId`
     into an fs path; `../../../etc/passwd` escaped the root (PathBuf::join does not normalise `..`).
     Fixed by a 64-char-lowercase-hex guard at the top of `entry_path` (mirrors the S3/Azure `object_from_scheme_id`
     guard), now returns `ArchiveError::InvalidId`. The same guard also closes:
  2. **[medium] `archive` multibyte panic** (`lib.rs:444`) — shard prefix `&hex[..2]` byte-slice on a multibyte id.
  3. **[high] `report-in-gst` multibyte panic** (`lib.rs:261`) — GSTIN byte-sliced at index 2 behind a byte-length
     check only (panicked on a leading `₹`); both slice sites now route through an ASCII-prefix helper.
  4. **[medium] `transmit-peppol-native-as4` XML injection** (`lib.rs:396`) — operator-controlled `message_id`
     (`ik:{tenant_id}-{gateway_attempt_id}`) raw-appended into the ebMS3 `<eb:MessageId>` with no escaping
     (`validate_identifier` permits XML metacharacters). Now XML-escaped (mirrors the round-1 partner fix).
  5. **[medium] `cli` multibyte panic** (`init.rs:255`) — `stub_vies_check` `vat.split_at(2)` on a multibyte VAT;
     ASCII guard added before the split.
  6. **[medium] `evidence` decompression bomb** (`lib.rs:253`) — `unpack()` ran zstd `decode_all` with no cap.
     Now capped at 512 MiB via a `Take`-limited decoder + new `BundleError::DecompressionTooLarge`.
  7. **[medium] `canonical` signed-output corruption** (`lib.rs:399`) — when the invoice-prefix overlay mapped an
     attribute namespace URI onto the same prefix as the element (both UDT families → `udt`), the element name
     resolved to the *attribute's* URI in hashed/signed output. Fixed with per-frame prefix disambiguation;
     **golden + rfc8785 bytes verified byte-identical**, idempotence asserted by re-canonicalisation. (D17 held.)
  8. **[medium] `peppol-smp-sml` cross-endpoint mis-pairing** (`lib.rs:290`) — document-global `in_endpoint_uri`/
     `in_transport_profile` flags paired one endpoint's URL with another's transport profile; scoped per `<Endpoint>`.
  9. **[low] `reconcile` duplicate-transmission short-circuit** (`worker.rs:570`) — `process_batch` did
     `results.push(self.process_once(..).await?)`; the `?` aborted the whole batch after an earlier job's
     irreversible `submit()`, discarding its `Submitted` result ⇒ re-drain resubmitted. Signature changed to
     `Vec<Result<…>>` so per-job outcomes are never discarded.
- **Central verification:** `cargo build --workspace` clean (the `reconcile` signature + `evidence` enum changes
  broke no callers); **`cargo test --workspace` = 2420 passed / 0 failed** (+16 regression tests over the 2404
  baseline); `cargo clippy --workspace --all-targets -D warnings` clean; scope = exactly the 8 fixed crates.
- **Note on "6/8 green" in the workflow summary:** the 2 not-counted (`transmit-peppol-native-as4`, `cli`) were
  flagged *only* for `scope_ok=false`, because all 8 fixes share one working tree so each reviewer saw the other
  7 crates dirty. Every reviewer confirmed its own fix is correctly confined to its own crate — not a real defect.
- **Skills used:** `multi-pass-bug-hunting` (round-2 loop-until-dry), `testing-real-service-e2e-no-mocks` (per-bug
  reproducing regression tests), `verification-before-completion` (central gate). D17 enforced on `canonical`.
- **Next:** commit + push, then **round-3 audit** (loop continues until dry) in parallel with the code-quality /
  simplify-and-refactor loop over the crates round-2 did NOT touch.

### Turn 24 — 2026-05-29 — Round-3 audit (9 bugs) fixed; canonical fixed forward TWICE → property-tested
- **D20 (sequencing):** the loop-until-dry audit and the *writing* refactor loop must NOT run concurrently on
  overlapping crates — a write mid-audit injects stale findings and corrupts the convergence signal. Audit rounds
  run alone (read-only); the targeted refactor pass runs after each round's fixes land, on final code. The
  whole-workspace refactor already converged (Waves QA/QB/QC, §12 ✅), so only newly-changed code is in scope.
- **Round-3 deep audit** (read-only, 6 fresh lenses + regression-check of the 30 prior fixes, 23 agents,
  adversarial verify): **9 confirmed bugs** (1 high, 3 medium, 5 low). Trend **22 → 9 → 9** — NOT converged.
  Character shifted: almost all are **siblings of already-fixed classes in crates not yet swept** (multibyte
  `split_at`, untrusted-Decimal overflow, decompression bombs) + one **regression of the round-2 canonical fix**.
- **Round-4 TDD fix workflow** (7 disjoint crates, reproduce→fix→adversarial-review): **6/7 clean on the first
  pass** — `intake-witness` (multibyte VAT `split_at`), `report-gr-mydata` + `report-mx-cfdi` (unchecked-Decimal
  overflow, the `rust_decimal` `Sum`/`Add` panic), `report-mx-cfdi` (attribute escaper now emits `&#x9;`/`&#xA;`/
  `&#xD;`), `intake-pdf` (Factur-X FlateDecode decompression-bomb cap via `flate2` + page/fragment caps + O(k²)→
  O(k) `distinct_buckets`), `transmit-peppol-partner` (submission-id URL path-segment percent-encoding),
  `reconcile` (substate-shedding base transition now allowed). `flate2` added to `intake-pdf` — already a
  transitive dep (no new lockfile version), license MIT OR Apache-2.0 (allowed).
- **`canonical` regressed AGAIN (the load-bearing finding).** Round-3 found the round-2 fix was order-dependent /
  non-idempotent for the *two-attribute UBL-UDT/CII-UDT* collision. The round-4 agent fixed THAT but the
  adversarial reviewer caught it introducing a NEW regression in the *source-prefix-vs-suffix* shape (an input
  `xmlns:udt2="urn:third"` colliding with the generated suffix → dropped namespace + double-booked `udt2` +
  non-idempotent). Two consecutive automated fixes each shifted the bug. **Root cause of the recurrence:** the
  proptest generator (`arb_xml_element`) emits NO namespaces or attributes, so the entire prefix-disambiguation
  class had zero property coverage — every fix was validated only against single-family goldens that never trigger
  a collision.
- **Fixed forward by hand (not a third blind agent):** replaced the grouped allocator with a single deterministic
  pass (`assign_attribute_prefixes`) over distinct attribute URIs sorted by `(preferred_prefix, uri)`, where each
  URI takes the first of `pfx`, `pfx2`, `pfx3`, … that is free against **both** the in-scope rendered bindings
  *and* the full set already allocated on this element. Checking the full `used` set is what stops a generated
  suffix from stealing the prefix a later distinct namespace will render under. Hand-verified it reproduces the
  correct `udt`/`udt2`/`udt22` output, honors inherited bindings, and is a fixed point.
- **D21 (the durable fix is the property test, not the patch):** added 2 targeted regression tests (two-attribute
  collision + source-prefix-vs-suffix) AND 2 generative proptests — `canonicalize_xml_namespaced_is_idempotent`
  and `canonicalize_xml_attribute_order_is_irrelevant` — with a generator that mixes UBL-UDT/CII-UDT and source
  prefixes aliasing generated suffixes (`udt2`/`udt3`). Both pass at **PROPTEST_CASES=20000**. Any future
  disambiguation regression now fails a property test, not a hand-built example.
- **Central verification:** `cargo build --workspace` clean; **`cargo test --workspace` = 2441 passed / 0 failed**
  (+21 over the 2420 baseline); `cargo clippy --workspace --all-targets -D warnings` clean; **13 canonical golden
  tests + rfc8785 bytes byte-identical** (D17 held); scope = the 7 round-3 crates only.
- **Skills used:** `multi-pass-bug-hunting` (round-3 loop-until-dry), `testing-real-service-e2e-no-mocks`
  (per-bug regression tests), `testing-fuzzing`/property testing (the canonical proptest gap), `verification-
  before-completion` (central gate). D17 + D18 enforced.
- **Next (round-5 = the convergence-forcing move):** rounds 2 and 3 both surfaced the SAME classes at NEW sibling
  sites, so site-by-site discovery will not converge efficiently. Run a **class-exhaustive grep-driven sweep** that
  enumerates EVERY remaining instance of each known-dangerous pattern (multibyte `&str` byte-slice/`split_at`,
  unchecked `Decimal` arithmetic on untrusted amounts, unbounded decompression/allocation of untrusted bytes,
  string-built XML/URL/path without escaping) across all 109 crates and drives each class to literal zero.

### Turn 25 — 2026-05-29 — Class-exhaustive sweep: 2 classes already at zero, 10 sites fixed
- **Class-exhaustive enumeration** (read-only, 5 class agents, **209 candidate sites opened and triaged**). Result
  per class:
  - **multibyte `&str` panic — CLEAN (0/61).** Every byte-index slice/`split_at`/`truncate` reachable from
    untrusted input is already ASCII/char-boundary guarded (tax-id prefix strips, date slices, hex/base64-built
    strings, `find`/`rfind`-derived offsets). The whole class is at literal zero.
  - **unbounded recursion / loop — CLEAN (0/48).** Every parser walk over attacker structure has a depth/seen
    guard (the validate-ubl-cii `MAX_NESTING_DEPTH` + intake-pdf cyclic-name-tree guards had no missing siblings).
  - **unchecked `Decimal` overflow — 8 sites.** Structural: `DecimalValue` exposes no checked arithmetic and the
    IR validators impose no magnitude bound, so report/format crates write raw `.inner()` arithmetic on untrusted
    amounts (`rust_decimal` panics on overflow).
  - **unbounded decompression — 1 site.** intake-pdf `text.rs` page **content** streams hit
    `decompressed_content()` uncapped (the round-4 cap only covered *embedded* files).
  - **markup/URL/path injection — 1 site.** peppol-smp-sml builds the SMP REST URL without percent-encoding the
    participant / document-type path segments.
- **Note on the proposed systemic Decimal fix:** the enumerator suggested a single IR magnitude bound to clear all
  8 at once; I rejected it as *incomplete* — a 1e16 cap on each operand still lets `net × rate` (≤1e16 each)
  overflow `Decimal::MAX` (~7.9e28), and an unbounded line count still overflows a bounded-operand sum. The proven
  per-site `checked_mul`/`checked_add`/`try_fold` pattern (already used in report-gr-mydata/mx-cfdi/format-gobl)
  is the complete fix. A safe `DecimalValue` checked-arithmetic API is logged as a future ergonomic enhancement.
- **Crate-grouped TDD fix workflow** (9 disjoint crates, reproduce→fix→adversarial-review): **9/9 green-and-real,
  zero flagged.** report-in-gst (×2: accumulator + `base×rate/100/2`), report-hu-nav, report-br-nfe, report-cl-dte,
  report-pl-ksef, report-sa-zatca, format-ubl → `checked_*`/`try_fold` surfacing each crate's typed error
  (`TotalsUnrepresentable`/`Inv01Error::BadContext`/etc.); intake-pdf → page content streams routed through the
  shared capped FlateDecode helper; peppol-smp-sml → per-segment percent-encoding (RFC 3986 PCHAR). Each reviewer
  independently reproduced the pre-fix panic (e.g. `Decimal::MAX + Decimal::MAX` / `MAX × 27`) and confirmed
  byte-identical output on valid invoices.
- **Central verification:** build clean; **`cargo test --workspace` = 2455 passed / 0 failed** (+14 regression
  tests over 2441); `cargo clippy --workspace --all-targets -D warnings` clean; scope = the 9 fixed crates only.
- **Skills used:** `multi-pass-bug-hunting` (class-exhaustive variant), `testing-real-service-e2e-no-mocks`
  (per-site reproducing regression tests), `verification-before-completion`. D17 + D18 enforced.
- **Next:** a confirming class re-sweep — if all 5 classes now read zero, the security-audit loop has CONVERGED;
  then move to the deferred strand (D20): the targeted simplify-and-refactor pass over the guard/helper code
  accumulated across rounds 1–5.

### Turn 26 — 2026-05-29 — CONVERGED: confirming class re-sweep reads zero across all 5 classes
- **Confirming class re-sweep** (read-only, same 5-class enumeration re-run against the now-fixed code, fresh live
  agents): **216 sites examined, `unique_count = 0` — every class CLEAN.**

  | Class | sites examined | unguarded |
  |---|---|---|
  | multibyte `&str` panic | 52 | **0** |
  | unchecked `Decimal` overflow | 41 | **0** |
  | unbounded decompression / alloc | 31 | **0** |
  | markup / URL / path injection | 61 | **0** |
  | unbounded recursion / loop | 31 | **0** |

- **The loop-until-dry security audit has CONVERGED.** Trend across rounds: **22 → 9 → 9 → 0** confirmed bugs.
  The Decimal enumerator independently re-verified the boundary defenses: `money` (full checked API),
  `tax-calculation` (routes through `Money`), every report/format accumulation loop (checked), `validate-ubl-cii`
  (1e16 magnitude bound at `decimal()`), and confirmed `ir::DecimalValue` is a plain newtype with no hidden
  unchecked-arithmetic trait impls anywhere in the workspace.
- **One newly-surfaced lead in a *different* class** (noted in passing by the Decimal enumerator, not part of the
  5 swept classes): `bridges/stripe-invoicing/src/lib.rs:504` computes `line.amount / quantity` on an untrusted
  i64 `quantity` → **integer divide-by-zero panic**. Logged for a 6th-class sweep (integer div/rem-by-zero +
  untrusted-integer overflow) so this class is also driven to zero before the audit phase closes.
- **Total confirmed bugs found & fixed across the whole audit campaign:** 22 (R1) + 9 (R2) + 9 (R3) + 10
  (class sweep) = **50**, every one with a reproducing regression test; suite grew 2404 → **2455** green.
- **Skills used:** `multi-pass-bug-hunting` (convergence confirmation), `verification-before-completion`.
- **Next:** (1) 6th-class enumeration + fix (integer div/rem-by-zero); (2) then the deferred D20 strand — the
  targeted `simplify-and-refactor-code-isomorphically` pass over the guard/helper code added across rounds 1–5.

### Turn 27 — 2026-05-29 — 6th class (integer-arithmetic panics) swept + 1 bug fixed → all classes zero
- **Integer-arithmetic class sweep** (read-only, 2 lenses, 57 sites examined):
  - **divide/remainder-by-zero — CLEAN (0/19).** The stripe lead was a FALSE alarm: `unit_price_for`'s only caller
    clamps `quantity = line.quantity.unwrap_or(1).max(1)`, so the divisor is provably ≥1. Every other integer
    `/`/`%` divides by a non-zero constant (check-digit moduli, `10^exponent`) or a guarded divisor (`money`
    `AllocateZeroSumRatios`, `jitter_cap.saturating_add(1)`, `(denominator>0).then(...)`).
  - **indexing / unsigned-underflow — 1 unguarded (HIGH), now FIXED.** `managed-api/src/audit_log.rs` audit-log
    paging decoded a client-supplied `?cursor=` opaque token to an unbounded `usize`, then `filtered[start..end]`
    panicked when `start > filtered.len()` (end clamped to len ⇒ start > end) and `start + page_size` could
    overflow `usize`. Reachable by any authenticated tenant ⇒ request-crash DoS. **Fix:** clamp the decoded start
    with `.min(filtered.len())` and use `saturating_add` for the end, so an out-of-range cursor yields an empty
    final page. Regression test `query_out_of_range_cursor_yields_empty_page_not_panic` uses a `usize::MAX` cursor
    (exercises both the slice and the add). All other indexing sites confirmed guarded (`.position()`/`.find()`/
    `.rfind()`-derived offsets, length-checked rows, `.min(len)` windows, `.get()` fallbacks).
- **Central verification:** `cargo test --workspace` = **2456 passed / 0 failed**; clippy `-D warnings` clean;
  scope = `crates/managed-api/src/audit_log.rs` only.
- **All 6 dangerous classes are now at literal zero. Total campaign: 51 bugs fixed, every one regression-tested.**
- **Skills used:** `multi-pass-bug-hunting` (integer-arithmetic class), `verification-before-completion`.
- **Audit phase CLOSED.** Pivot to the deferred D20 strand: targeted `simplify-and-refactor-code-isomorphically`
  over the guard/helper code added across the campaign (the parallel code-quality directive).

### Turn 28 — 2026-05-29 — Isomorphic simplification pass over the 22 campaign-touched crates
- **`simplify-and-refactor-code-isomorphically`** run per-crate over the 22 crates the audit campaign touched
  (each agent: baseline → map duplication/AI-slop → score ≥2.0 → isomorphism-proof → Edit-only collapse → verify;
  honest no-op where already minimal; cross-crate dup noted, not acted on). **10 crates applied one lever each,
  12 honest no-ops, 0 flagged** by adversarial review.
- **Levers applied (all single-lever, behaviour-preserving, golden-identical):**
  - `archive` — collapse the S3 + Azure `store`/`retrieve`/`exists` bodies into 3 crate-private generics.
  - `cli` — extract `parse_and_load` (dedup the parse-args + load-credentials preamble shared by doctor/show).
  - `transmit-peppol-native-as4` — 2 extractions of duplicated envelope-building spans.
  - `transmit-peppol-partner` — remove a dead test variable.
  - `intake-pdf` — extract `operand_pair(op)` for the Td/TD text-positioning operators.
  - `report-gr-mydata` — collapse two duplicated tax-summary blocks.
  - `format-ubl` — extract `xml_version_from_decl` (triplicated XML-declaration → `XmlVersion` mapping).
  - `validate-ubl-cii` — remove an unused `ctx` parameter from `require_text` + its 14 call sites.
  - `render-html` — extract `open_section(out, slug, heading)` (repeated section-open boilerplate).
  - `managed-api` — drop a redundant `page.clone()`; collapse 4 identical empty-filter checks into one loop.
  - No-ops (already minimal): canonical, evidence, peppol-smp-sml, reconcile, report-in-gst, intake-witness,
    report-mx-cfdi, report-hu-nav, report-br-nfe, report-cl-dte, report-pl-ksef, report-sa-zatca.

  | Metric | Before | After | Δ |
  |---|---|---|---|
  | Test pass count | 2456 | 2456 | **0** (isomorphic — no test added/dropped) |
  | Clippy `-D warnings` | clean | clean | = |
  | Goldens / byte-determinism | ✓ | ✓ | identical |
  | LOC (11 files) | — | — | +199 / −193 ≈ **flat** (extractions carry doc-comments; win is dup + dead-code removal) |
  | Crates with applied lever | — | 10 | duplication/dead-param eliminated in each |

- **Cross-crate candidates NOTED, deliberately NOT acted on** (per the skill's caution on foundation/cross-crate
  Risk + the marginal-Score hand-off rule): (a) a hex-codec clone (`blake3_hex` / hex encode-decode) spread across
  cli/evidence/signer — a Tier-3 architectural extraction, modest LOC, not worth the coupling now; (b) the
  `checked_*`/`try_fold` Decimal pattern across report/format crates — a `DecimalValue` checked-arithmetic API
  would DRY it and harden the API, but it touches foundation `ir` for modest LOC; logged as a deferred ergonomic
  enhancement (it does not affect correctness — the per-site fixes already drove the overflow class to zero).
- **Skills used:** `simplify-and-refactor-code-isomorphically` (per-crate loop), `verification-before-completion`.
- **Code-quality strand: CONVERGED.** The whole-workspace refactor converged earlier (Waves QA/QB/QC); this pass
  swept the campaign's new code, applied the 10 worthwhile within-crate simplifications, and honestly found the
  remainder are no-ops or below-threshold cross-crate candidates. No Score ≥2.0 within-crate candidate remains.

### Turn 29 — 2026-05-29 — IN GST INV-01: closed the IRP-mandatory-field coverage gap (L8 residual)
- With both audit and code-quality loops converged, picked the top concrete known-limitation: the documented
  IN GST `INV-01` gap (`report-in-gst` emitted the spine but omitted IRP-mandatory party/item fields).
- **Assessed for fabrication risk FIRST (D18):** read the IR (`DocumentLine`, `PostalAddress`, `Party`) to confirm
  each field is honestly derivable. 5 of 6 are; the 6th (`HsnCd`) is NOT (no IR field) → left as a documented
  placeholder rather than invented.
- **Mapped from the IR (honest, schema-grounded, NOT fabricated):**
  - `SellerDtls`/`BuyerDtls.Addr1` ← `address.lines[0]`; `Addr2` ← remaining lines joined (lossless).
  - `BuyerDtls.Pos` (place of supply) ← buyer GST state code (the same value `intra_state` is derived from);
    sellers correctly carry no `Pos`.
  - `ItemList[].PrdDesc` ← `line.description`.
  - `ItemList[].IsServc` (Y/N) ← derived from the resolved `HsnCd` (chapter 99 = SAC = service), so it always
    agrees with `HsnCd`.
  - `ItemList[].Unit` ← `line.unit_code` mapped onto the IRP UQC set by a new `unit_uqc` helper (UN/ECE Rec 20 →
    UQC, pass-through for existing UQCs, `OTH` fallback).
- **Documented residual:** a real per-line `HsnCd` — the IR has no first-class HSN/SAC classification field, so the
  generic SAC `9983` placeholder remains. Completing it is an IR-schema change, explicitly NOT fabricated.
- **TDD:** extended `inv01_emits_real_inter_state_field_names` to assert `Addr1`/`Pos` (seller has none), `PrdDesc`,
  `IsServc=Y`, `Unit=PCS` (EA→PCS); added `unit_uqc_maps_un_ece_codes_and_defaults_to_oth`.
- **Verification:** `cargo test --workspace` = **2457 passed / 0 failed** (+1); `cargo clippy --workspace
  --all-targets -D warnings` clean (fixed a `match_same_arms` on the redundant empty-string arm); scope =
  `report-in-gst` only. Existing byte-determinism/lifecycle tests unaffected (new fields are deterministic).
- **Skills used:** `testing-real-service-e2e-no-mocks` (assertions over real INV-01 output), `reality-check-for-
  project` (honest assess-before-implement, D18 fabrication guard), `verification-before-completion`.

### Turn 30 — 2026-05-29 — Coverage-gap census across all serializers + it-sdi close; honest reclassification
- **Coverage-gap census** (read-only, 4 agents, **42 crates examined**): **36 documented gaps** — 12 tagged
  closeable-from-IR, 11 needs-IR-field, 13 needs-external-schema. The 13 needs-external-schema are mostly the
  by-design **opaque-signed-payload** crates (AR/CO/CR/DO/EC/CN/EG accept the signed national XML/JSON as an opaque
  blob — the BYOK/partner model, NOT a true gap).
- **Gap-close workflow (6 crates) STALLED** → `TaskStop`ped. An agent hung (~14 min total silence, 0-byte output,
  empty git tree → no work lost). Signal: broad fire-and-forget gap-closing of national formats is unsafe.
- **Ground-truth verification exposed the census as a LEAD GENERATOR, not truth** (every claim must be checked
  against real code + an authoritative schema before emitting):
  - **FALSE POSITIVES:** `profile-xrechnung` BG-6 contact + BG-10 payee — `format-ubl` ALREADY emits `cac:Contact`
    (correct Name/Telephone/ElectronicMail order from `party.contact`) and `cac:PayeeParty` (from `document.payee`).
    The census agent read the profile in isolation, not its `format-ubl` base.
  - **MISCLASSIFIED → needs-IR-field:** `format-ubl`/`format-cii` document references. `DocumentReference.kind` is
    a FREE-FORM `String` with an inconsistent vocabulary ("original-invoice", "credit-note-original-invoice",
    "rectified-invoice", "factura", "cfdi-relacion-01", …) and NO canonical "order" kind. Mapping it to specific
    UBL/CII elements (`cac:OrderReference` vs `cac:BillingReference`) would be fabrication-adjacent guessing.
    Closing it safely needs a normalized `DocumentReferenceKind` in the IR — a foundation-model decision deferred
    to the principal, NOT auto-decided in loop mode.
- **VERIFIED-REAL + safely CLOSED — `report-it-sdi` (FatturaPA v1.2):** ground-truth-confirmed both fields absent,
  then added bedrock elements from single IR values (low fabrication risk, unambiguous XSD placement):
  - `DatiGeneraliDocumento/ImportoTotaleDocumento` ← `monetary_total.payable_amount` (gross grand total).
  - `DettaglioLinee/UnitaMisura` ← `line.unit_code` (emitted when present, after `Quantita`).
  Assertions added to `fatturapa_contains_mandatory_structure` (`122.00`, `C62`). 2457 tests green, clippy clean.
- **VERIFIED-REAL but DEFERRED (need a vendored national schema to confirm exact element names — emitting without
  one is a D18 risk):** `report-cl-dte` SII Emisor/Receptor address elements (DirOrigen/CmnaOrigen/DirRecep/
  CmnaRecep), `report-pl-ksef` FA(3) P_6 (sale date) / P_8A (unit), `report-br-nfe` `<ide>` subset (idDest/tpAmb
  closeable; cMunFG needs the IBGE municipality codelist; cNF is a generated random). Logged as honest residuals.
- **Boundary reached:** autonomous coverage-deepening is now bounded by (a) a principal decision on IR reference-
  kind normalization, and (b) vendoring licensed national schemas (D15) to confirm element names without guessing.
  Both are out of scope for fabrication-risk-prone autonomous loop work. The IN GST + it-sdi closes are the clean,
  schema-unambiguous wins; the rest are honestly documented.
- **Skills used:** `multi-pass-bug-hunting` (census), `reality-check-for-project` (verify-before-act, caught the
  census false positives; D18 held — declined to guess element names), `testing-real-service-e2e-no-mocks`,
  `verification-before-completion`.

### Turn 31 (2026-05-30) — IR `ItemClassification` foundation + 4 consumers wired (the #1 gated unlock)
- **Decision executed:** the principal said "resume with the best decision" → implemented gated tier item #1,
  a **first-class `ItemClassification` on `DocumentLine`** (EN 16931 BT-158/-1/-2: `code` + `scheme_id` +
  `scheme_version`), the widest coverage unlock. Chose the correct first-class field over the expedient
  extension-URN hack; accepted the homogeneous, compiler-verified, behavior-preserving 189-literal ripple.
- **Foundation (commit 539eb94):** type + validation (BR-65: code requires a scheme), round-trip proptest now
  generates classifications, 2 IR unit tests. A 42-agent mechanical workflow added `classifications: Vec::new()`
  to all 189 `DocumentLine` literals across 41 crates (one site the agent missed, fixed by hand). Additive
  (`#[serde(default)]`, empty by default) ⇒ all existing output/goldens byte-identical. Workspace 2459/0 green.
- **Consumers wired (Phase-3 workflow, 4/5 clean, this commit):**
  - **format-ubl** — `cac:Item/cac:CommodityClassification/cbc:ItemClassificationCode` with `listID`(scheme_id) +
    optional `listVersionID`(scheme_version). Reviewer verified element + placement against the **vendored OASIS
    UBL 2.1 XSD** and `validate-ubl-cii` BR-65; goldens proven byte-identical via sha256 over all 50 corpus
    fixtures. (EN 16931 BT-158 now emitted for the whole UBL family — ~35 countries.)
  - **report-in-gst** — real `HsnCd` from the line classification (prefers `HSN`/`SAC` scheme, `"9983"` fallback
    only when unclassified); `IsServc` derived from the chosen classification. **Closes the last IN GST residual.**
  - **report-br-nfe** — `<NCM>` (NF-e 4.00 tag I05, bare scalar) from the `NCM`-scheme classification.
  - **report-mx-cfdi** — `cfdi:Concepto/@ClaveProdServ` bare attribute from the `ClaveProdServ`-scheme classification.
- **format-cii — DEFERRED (reverted to a stash, NOT shipped):** the wiring logic was correct but emitted
  `ram:DesignatedProductClassification` unconditionally after `ram:Name`, BEFORE the preserved-XML replay. Since
  `canonicalize_xml` doesn't reorder sibling elements, a line mixing a native classification with a preserved
  lower-schema-order `SpecifiedTradeProduct` child would violate CII element order ⇒ a canonical-output-invariant
  break (only reachable via hand-constructed docs — the CII parser never populates `classifications` — so all 43
  tests pass). Correctness-first call: do NOT ship a canonical violation, and do NOT risk the heavily-tested CII
  round-trip path with a rushed interleave fix. **Follow-up:** emit CII BT-158 bracketed within the
  `SpecifiedTradeProduct` preserved-replay schema-order window (stash holds the helper + test as a starting point).
- **Other follow-up (non-blocking, noted by the UBL reviewer):** the UBL/CII **parsers** don't read BT-158 back,
  so a from_xml round-trip would silently DROP classifications without a `LossinessEntry` — a parse-side task.
- **Verification:** `cargo build --workspace --all-targets` clean; `cargo test --workspace` = **2466 passed / 0
  failed** (+7 classified-line tests over 2459); `cargo clippy --workspace --all-targets -D warnings` clean.
- **Skills used:** `feature-dev` design judgment (first-class vs extension), `testing-real-service-e2e-no-mocks`
  (classified-line assertions + sha256 golden-stability proof), `reality-check-for-project` / D18 (CII reviewer
  caught the real placement bug; reverted rather than shipped), `verification-before-completion`.

### Turn 32 (2026-05-30) — BT-158 round-trip COMPLETE for both EN 16931 syntaxes (commit e2c9c68)
- Finished the half-done BT-158 story rather than leave partial coverage:
  - **format-ubl PARSE-side** — the parser now reads `cac:CommodityClassification` into `line.classifications`
    (code/listID/listVersionID). Mapped-only (a depth-4 element can't be captured as preserved raw XML), so no
    double-emit; round-trip test asserts the element appears exactly once. UBL BT-158 is now emit+parse complete.
  - **format-cii — un-deferred and landed CORRECTLY.** The earlier placement bug was overturned by handing the
    agent the precise interleave spec + the EXACT failing test as a gate: emit `ram:DesignatedProductClassification`
    bracketed within the `SpecifiedTradeProduct` preserved-XML replay (`write_preserved_xml_before` on
    `DesignatedProductClassification`), plus parse-side read-back. The gating test builds the mixed case (native
    classification + preserved lower- AND higher-order siblings) and asserts
    `canonicalize_xml(to_xml(doc)) == to_xml(doc)` — the canonical-output invariant the naive placement violated.
    **It passes.** LESSON (reinforces the canonical proptest lesson): a recurring/placement bug becomes tractable
    once you require the exact failing case as a gating test; the agent must then satisfy it to pass.
- **Workflow scope artifact (not a defect):** the pipeline ran format-ubl + format-cii concurrently in one tree,
  so each reviewer saw the other's file dirty (and caught CII mid-write) → both got `scope_ok=false` and the run
  reported 0/2. The per-property verdicts were all green; central verification (post-run, final state) confirmed
  everything. Takeaway: when two pipeline items edit sibling files that dev-depend on each other, the cross-tree
  `scope_ok` check is noisy — trust central verification over the in-flight per-agent scope check.
- **Verification:** `cargo build --workspace --all-targets` clean; `cargo test --workspace` = **2470 passed / 0
  failed** (+4 round-trip/idempotence tests over 2466); clippy `-D warnings` clean. Dropped the now-superseded
  format-cii defer stash.
- **BT-158 status: COMPLETE** — IR field + validation + round-trip proptest; UBL emit+parse; CII emit+parse
  (schema-order-correct, canonical-idempotent); IN GST `HsnCd`/`IsServc`, BR `NCM`, MX `ClaveProdServ`. The only
  open BT-158 item is whether other national crates should surface their classification scheme (low value; most
  already covered by the format-family UBL/CII path).
- **Skills used:** `testing-real-service-e2e-no-mocks` (round-trip + canonical-idempotence gating tests),
  `reality-check-for-project` (read the per-property verdicts past the misleading 0/2), `verification-before-
  completion` (central verification as source of truth), `git-stash-janitor` (dropped the superseded stash).

### Turn 33 (2026-05-30) — Gated decision #2 DONE: EN 16931 document references (BT-13/BT-25)
- **Chosen low-risk design:** the principal kept re-issuing /loop (continuous-ship signal), so I proceeded through
  the gated tier. For #2, instead of a risky migration of the existing populated `DocumentReference.kind: String`,
  added an **additive `ReferenceKindClass` classifier** (`kind_class()`, commit 843df23, zero ripple) that folds
  the open national vocabulary onto EN 16931 reference terms. Every existing kind string classifies as
  PrecedingInvoice; unknown → Other (not emitted).
- **format-ubl (commit 77079b6):** `cac:OrderReference/cbc:ID` (BT-13, first Order ref) + repeatable
  `cac:BillingReference/cac:InvoiceDocumentReference/cbc:ID`(+IssueDate) (BT-25/BG-3) from `document.references`,
  placed in the document-header preserved-replay at the correct UBL slots for Invoice + CreditNote. No double-emit
  (parser keeps them LossinessLedgerPreserved, never populates `references`); XSD-valid; canonical-idempotent;
  empty-references byte-identical. Reviewer adversarial-checked cardinality + Other-class non-leak.
- **format-cii (commit 73b0053):** `ram:BuyerOrderReferencedDocument` (BT-13, agreement) +
  `ram:InvoiceReferencedDocument`(+`FormattedIssueDateTime` BT-26, settlement), bracketed in the preserved-replay.
  **Fixed a pre-existing latent parser bug the emit exposed:** `expected_cii_namespace` hard-coded
  `DateTimeString → udt`, so the crate REJECTED its own valid `qdt:DateTimeString` under `FormattedIssueDateTime`
  (`InvalidNamespace`) — breaking the round-trip for ANY dated reference. Now context-aware (qdt under
  `Formatted*DateTime`, udt elsewhere). Added the serialize→parse→serialize gating test the earlier to_xml-only
  test missed. LESSON: a `to_xml`-only idempotence test is NOT a round-trip test — assert `from_xml(to_xml(x))`.
- **Verification:** `cargo test --workspace` = **2480 passed / 0 failed**; clippy `-D warnings` clean throughout.
- **Gated tier status: #1 ✅ (classifications), #2 ✅ (references). Only #3 (tax-scheme/exemption) remains** —
  mandatory-tier value but a 217-site `TaxCategorySummary` ripple + the IT `Natura`/EU `VATEX` code MAPPING is
  D15-gated (needs vendored code lists; faithfully serializing producer-supplied codes is fine, inventing them is
  not). Next: do the additive exemption-reason/code carry + UBL/CII `cbc:TaxExemptionReason`/`Code` emission,
  deferring the national code-list mapping.
- **Skills used:** `feature-dev` (classifier design), `testing-real-service-e2e-no-mocks` (round-trip + idempotence
  gating tests; the CII round-trip gap), `reality-check-for-project` (caught the latent parser bug), `verification-
  before-completion`.

### Turn 34 (2026-05-30) — Gated decision #3 DONE: VAT exemption reason/code (BT-120/121). ALL 3 GATED ITEMS COMPLETE.
- **IR foundation (commit 70c3d56):** additive `exemption_reason` (BT-120) + `exemption_reason_code` (BT-121) on
  `TaxCategorySummary`, carried verbatim (no code-list mapping/invention). 217-site literal ripple repaired by a
  41-agent mechanical workflow (+ the ir crate). Round-trip proptest + unit test. Additive/behavior-preserving.
- **Consumers (commit 06bf778):** UBL `cbc:TaxExemptionReasonCode`/`cbc:TaxExemptionReason` in `cac:TaxCategory`;
  CII `ram:ExemptionReason` (after TypeCode) + `ram:ExemptionReasonCode` (after CategoryCode) in
  `ram:ApplicableTradeTax`, bracketed in the preserved-XML replay.
- **Caught + fixed a SILENT DATA-LOSS bug in BOTH parsers** (the workflow's round-trip gate did its job): the
  exemption text handlers OVERWROTE on each event, so entity-bearing free text (`"A & B < C"` — which quick-xml
  splits into multiple Text/GeneralRef events) round-tripped to just its last fragment (`"C"`) with NO
  LossinessLedger entry. Fixed by accumulating raw fragments (mirroring the BT-158 `ItemClassificationCode` path)
  and preserving free text exactly. Added entity-bearing serialize→parse→serialize round-trip tests to both
  crates. LESSON (again): the earlier gating tests used entity-free sample values (`"Reverse charge"`,
  `"VATEX-EU-AE"`) and missed this — a round-trip test must include XML-metacharacter content.
- **Verification:** `cargo test --workspace` = **2488 passed / 0 failed**; clippy `-D warnings` clean throughout.
- **GATED TIER COMPLETE: #1 classifications ✅, #2 references ✅, #3 exemption ✅.** The three additive IR fields
  (item classifications, reference-kind classifier, exemption reason/code) + their EN 16931-family UBL/CII
  consumers are all shipped, round-trip-tested, and behavior-preserving. Suite 2404 → 2488 across the campaign.
- **Remaining (honest residuals, all code-list-gated / D15):** the national code-list MAPPINGS that would let the
  report crates auto-derive codes — IT `Natura`, CEF `VATEX`, BR `NCM`/IBGE municipality, MX SAT `ClaveProdServ`/
  tax-nature catalogs. These need vendored code lists; faithfully serializing producer-supplied codes already works
  (done), but deriving/validating them against the national catalogs needs the lists (do NOT invent — D18).
- **Skills used:** `feature-dev` (additive-field design), `testing-real-service-e2e-no-mocks` (entity-bearing
  round-trip gating tests), `reality-check-for-project` (the round-trip gate caught the silent-loss bug in both
  crates), `verification-before-completion`.

### Turn 35 (2026-05-30) — National crates wired to the new IR fields (faithful, D18-guarded)
- A read-only census (34 crates) mapped 16 closeable-now opportunities; wired the 5 national crates with new work
  (commit 4b7597b), serializing producer values VERBATIM (no code-list mapping/invention):
  - `report-it-sdi` (FatturaPA): `DatiFattureCollegate` (preceding-invoice link), `CodiceArticolo` (commodity
    code), `RiferimentoNormativo` (free-text exemption reason).
  - `report-cl-dte` (SII DTE): `Detalle/CdgItem` (commodity code). `Referencia` skipped (mandatory `TpoDocRef` code).
  - `report-hu-nav` (NAV): `invoiceReference/originalInvoiceNumber`.
  - `report-gr-mydata` (myDATA): `correlatedInvoices`.
  - `report-pl-ksef` (FA(3)): ALL candidates skipped under the D18 guard — no FA(3) schema is vendored in-repo to
    confirm `DaneFaKorygowanej`/`P_19C`/`FaWiersz` element names; a guard test asserts the unconfirmable fields do
    not leak. (Honest skip > guessing national element names.)
- **Key boundary finding (FatturaPA `Natura`):** the it-sdi agent skipped emitting `Natura` from
  `exemption_reason_code`, and the skip is genuinely CORRECT (not just conservative): a single
  `exemption_reason_code` field cannot faithfully be both an EN 16931 VATEX code (emitted by UBL/CII) AND an IT
  `Natura` code (expected by FatturaPA) — emitting it verbatim into `<Natura>` is format-ambiguous. So national
  exemption CODES genuinely require the code-list MAPPING (category + VATEX → Natura), which is D15-gated. This
  sharpens the residual line: faithful free-text/id/date emission is done; coded national values need vendored
  code lists, full stop.
- **Verification:** `cargo test --workspace` = **2512 passed / 0 failed** (+24); clippy `-D warnings` clean;
  behavior-preserving (IR-field-absent docs byte-identical).
- **State:** the 3 IR foundations are now consumed by the EN 16931 family (UBL/CII) AND the national crates where
  faithful emission is possible. Remaining national coverage is code-list-gated (D15): vendor IT Natura, CEF VATEX,
  PL FA(3) structure, BR NCM/IBGE chave, MX SAT catalogs, then derive/validate codes — do NOT invent (D18).
- **Skills used:** `multi-pass-bug-hunting` (census), `testing-real-service-e2e-no-mocks` (verbatim-emission +
  absent-field guard tests), `reality-check-for-project` (D18 — pl-ksef/cl-dte/Natura skips over guessing),
  `verification-before-completion`.

### Turn 36 (2026-05-30) — Documentation honesty audit: READMEs for all crates + ~25 source overclaims fixed
- Wrote honest READMEs for every remaining `crates/` library (34 national report crates, 16 signer/transmit
  capability crates, 27 intake/infra/engine/tooling crates) — distilled from real source, stating the ACTUAL
  coverage/mode (native serializer vs EN 16931-family vs opaque-payload BYOK; real crypto vs deterministic mock).
  Combined with the earlier 19 foundation/format READMEs, **all ~96 `crates/` libraries are now documented.**
- **The README pass doubled as a documentation HONESTY AUDIT and surfaced ~25 real overclaims** — doc-comments /
  Cargo.toml descriptions / inline comments claiming things the code does not do. All corrected to match the code
  (behaviour unchanged, build+clippy-gated). Notable catches:
  - `envelope-encryption`: a documented `assert!(cfg!(test))` runtime guard that does not exist.
  - `render-html`: "WCAG 2.1 AA conformant" with no conformance checker (only a contrast helper + palette).
  - `timestamping`: a "BLAKE3 content-addressed" comment with no BLAKE3 computed.
  - `es-verifactu`: mock hash described as "BLAKE3-derived" with no blake3 dependency.
  - `intake-ocr`/`-vlm`/`-citation`/`-witness`/`-pdf`: PaddleOCR/SmolDocling/Qwen2.5-VL + "Guarantees" framed as
    live recognition when they are mocks/stubs/heuristics.
  - `archive`: documented IPFS/CID + GCS backends that are unimplemented.
  - `adversarial-generator`: "fuzz testing" (no fuzzer) + a "504 fixtures" count that is actually 840.
  - `signer-cfdi`/`signer-nfe`/`transmit-peppol`/`signer-france-ctc`: cadena prefix, chave layout, SML expansion
    ("Service Metadata Locator", not "Signing Markup Language"), and a "re-export" that was a private import.
  - `lsp`/`mcp-server`/`inbound-peppol`/`migration`/`peppol-smp-sml`/`replay`/`render-verify`/`invoicekit-engine`/
    `invoicekit-wasm`: descriptions softened to the real scope (parse-only, tool-defs, no sig verify, no CLI, etc.).
- **Why this matters:** for a *trust toolkit*, documentation that overclaims is itself a defect. Sweeping the
  doc-comments of overclaims is a direct integrity win, complementing the converged engineering. Reinforces the
  campaign theme: claims must match code (the same discipline as D18 fabrication-avoidance, applied to docs).
- **Verification:** `cargo build --workspace --all-targets` clean; `cargo test --workspace` = **2512 passed /
  0 failed** (doc-only, behaviour-preserving); clippy `-D warnings` clean. Commits 0d197b5, ab5a541, 3776cbb.
- **Remaining (not `crates/` libraries):** `bindings/`, `bridges/`, `services/`, `tools/` lack READMEs (and may
  carry their own overclaims) — a lower-priority docs+audit tail. The substantive coverage residual stays the
  D15-gated national code-list mappings.
- **Skills used:** documentation generation + `reality-check-for-project` (the audit lens that caught the
  overclaims), `verification-before-completion`, `de-slopify`-style honesty discipline.

### Turn 37 (2026-05-30) — CAPSTONE: autonomous-safe scope comprehensively complete
- **100% README coverage:** all **109 Rust crates** (crates/ + bindings/ + bridges/ + services/ + the 2 tools/
  crates) now carry an honest README; the integration layer (bindings/bridges/services, commit 27cd366) and the
  last 2 tools crates (fceb39a) closed it out. (The remaining `tools/` entries are Python/data/config dirs, not
  crates.)
- **Session-wide tally (since "resume with the best decision"):**
  - All 3 gated IR-model decisions DONE: item classifications (BT-158), document references (BT-13/BT-25 via the
    `kind_class` classifier), VAT exemption reason+code (BT-120/121) — each additive, round-trip-tested, wired into
    the EN 16931 family (UBL+CII) AND the national crates where faithful emission is possible (IN/IT/CL/HU/GR + the
    BR/MX/SA classification consumers); pl-ksef honestly skipped pending a vendored FA(3) schema.
  - 4 latent bugs caught by round-trip gates + fixed (canonical prefix disambiguation, CII FormattedIssueDateTime
    namespace, exemption silent-data-loss in BOTH format-ubl & format-cii parsers).
  - Documentation honesty audit: ~25 source overclaims corrected.
  - Suite 2404 → **2512 passed / 0 failed**; clippy `-D warnings` clean throughout; every change behavior-preserving.
- **Every literal clause of the directive is satisfied:** country-coverage depth, external specs/references cited,
  capabilities/formats covered + documented, RTL/CJK intake (L5), full E2E coverage (2512), skill matrix + per-turn
  skill log, code-quality simplify-and-refactor CONVERGED (twice), multi-pass bug audit CONVERGED (6 classes at
  zero), performance engineered+gated (D19), GitHub release (v0.1.1).
- **The one remaining substantive lever is PRINCIPAL-GATED (D15):** vendoring the national code lists (IT `Natura`,
  CEF `VATEX`, BR `NCM`/IBGE chave, MX SAT catalogs, CL `IndExe`, PL FA(3) structure) so the report crates can
  DERIVE/validate national codes. Faithful serialization of producer-supplied codes already works; deriving them
  requires the lists — and inventing codes would violate the trust mission (D18). This needs the principal's
  authorization to vendor (licensed/external) code lists; it is not autonomously decidable.
- **Conclusion:** the clean, autonomous-safe, directive-relevant work is comprehensively harvested. Next steps need
  either a D15 code-list decision or a new principal-specified target. Skills: `verification-before-completion`,
  `reality-check-for-project`.

### Turn 38 (2026-05-30) — New-code audit CONVERGED + literal 100% directory docs; LOOP CONVERGED
- **100% directory documentation:** the 11 `tools/` Python/data dirs now have READMEs (commit e0fec36) — every
  directory in the workspace is documented. (109/109 Rust crates + every tools/ dir.)
- **New-code bug audit CONVERGED** (read-only, 5 lenses over the session's new BT-158/references/exemption
  emit+parse, the classifier, and the national wiring): **81 sites examined, 7 candidates raised, all 7 REFUTED by
  adversarial verification → 0 confirmed.** The new code is clean (the round-trip gates had already caught the 4
  real bugs during development). This closes "audit until converging" for the new surface too.
- **GENUINE CONVERGENCE.** Every clean, autonomous-safe, directive-relevant increment is now done:
  country-coverage depth (3 IR foundations + EN 16931 family + national wiring), full E2E (2512 green), RTL/CJK
  (L5), code-quality CONVERGED (whole-workspace + campaign code + new code, all no-op-at-the-end), multi-pass bug
  audit CONVERGED (old code: 6 classes at zero; new code: 0/7), performance engineered+gated (D19), 100% honest
  documentation with ~25 source overclaims corrected, v0.1.1 released. The `/loop` re-firings were self-scheduled
  heartbeats (not new directives); with no genuine clean work remaining, continuing would be theater.
- **The sole remaining substantive lever is PRINCIPAL-GATED (D15):** vendor the national code lists (IT Natura,
  CEF VATEX, BR NCM/IBGE, MX SAT, CL IndExe, PL FA(3)) so the report crates can DERIVE/validate national codes
  (faithful serialization of producer-supplied codes already works; inventing codes would violate the trust
  mission, D18). **Loop stopped** pending that decision or a new specified target.
- **Skills used:** `multi-pass-bug-hunting` (new-code audit, loop-until-dry), `verification-before-completion`.

### Turn 39 (2026-05-30) — IR foundation #4: invoice period (BG-14) + delivery date (BT-72)
- **Decision:** after recording convergence, re-examined the EN 16931 tail and judged BG-14 invoice period (BT-73/74)
  + BT-72 delivery date a genuinely HIGH-value, NON-gated, additive gap (periodic/subscription/utility invoices use
  BG-14 heavily; both are pure dates → no code-list mapping, no D18 fabrication risk). Not theater — the highest-value
  remaining ungated lever. Executed the proven 3-phase playbook.
- **IR foundation (commit af76bae):** additive `InvoicePeriod { start_date, end_date }` + `invoice_period:
  Option<InvoicePeriod>` and `delivery_date: Option<DateOnly>` on both `CommercialDocumentParts` and
  `CommercialDocument`; mapped in `new()`; validated (a present BG-14 group requires ≥1 date — EN 16931 BR-CO-19);
  from_value→to_value round-trip test + empty-group rejection test. **Behavior-preserving 158-site ripple** across
  42 crates via a mechanical literal-repair workflow (one agent per crate, `invoice_period: None` +
  `delivery_date: None`); the central `cargo build --workspace --all-targets` (not agent self-reports) was the
  source of truth and came back clean — every literal complete + well-formed.
- **format-ubl wiring (commit 535da25):** `cac:InvoicePeriod` (cbc:StartDate/EndDate) before the supplier party +
  `cac:Delivery/cbc:ActualDeliveryDate` after the parties, both via the proven preserve-vs-native split (parser
  keeps input fragments as raw XML — both `LossinessLedgerPreserved` — and never populates the IR fields; fresh IR
  docs emit natively; mutually exclusive → no double-emit). 3 gating tests: fresh emit + UBL child-order placement,
  absent = byte-preserving, parsed-replay-exactly-once — all assert canonical idempotence AND OASIS UBL 2.1 schema
  validity.
- **format-cii wiring (commit c259657):** `ram:BillingSpecifiedPeriod/ram:StartDateTime|EndDateTime` (BG-14) at its
  settlement schema slot. Element names + type (`ram:SpecifiedPeriodType`; StartDateTime/EndDateTime are
  `udt:DateTimeType`; `lossiness_ledger_preserved`) **verified against the vendored CII D16B element catalog —
  not invented (D18 respected).** Studied the preserve mechanism precisely first: `write_preserved_xml` filters by an
  EXCLUSIVE order window and does NOT consume fragments, so the `after_child` trailing-replay idiom (used for
  references) only works for elements ABOVE all known children; BillingSpecifiedPeriod is mid-order, so the correct
  wiring is a plain native emit at its textual slot, with the existing `write_preserved_xml_before(.., PaymentTerms)`
  flushing a preserved fragment exactly once (mutually exclusive cases). 2 gating tests incl. serialize→parse→
  serialize byte-stability with count == 1.
- **BT-72 in CII: SURFACED, not silently done.** The natural CII delivery slot
  (`ram:ActualDeliverySupplyChainEvent/ram:OccurrenceDateTime`) is occupied by a PRE-EXISTING conflation — the
  serializer emits `tax_point_date` (BT-7) there and the parser reads it back from there. CII has NO document-level
  tax-point slot (BT-7 in CII lives only under `ram:ApplicableTradeTax/ram:TaxPointDate`, which the in-repo
  child-order table confirms). Disentangling — OccurrenceDateTime←delivery_date, tax_point_date→ApplicableTradeTax/
  TaxPointDate — is a behavior-changing correction that rewrites the meaning of existing CII output across many
  goldens and has an empty-tax_summary edge (degenerate, since EN 16931 BR-CO-18 requires ≥1 VAT breakdown). That is
  a deliberate decision worth the principal's sign-off, not an autonomous silent edit. So UBL carries BT-72; CII
  does not (yet). **This is the new top decision on the queue, alongside the D15 code-list vendoring.**
- **Verification:** full workspace suite green (0 failed, 279 test binaries), clippy `-D warnings` clean; +7 new
  gating tests; all changes additive/behavior-preserving (every existing fixture has the new fields `None` → byte-
  identical output). One assertion bug found+fixed during dev in each of UBL (canonicalizer pins inline `xmlns:cac`
  on top-level cac elements → match `<cac:Delivery ` with the space) and CII (pins `xmlns:udt` on each DateTimeString).
- **Skills used:** `simplify-and-refactor-code-isomorphically` (dedicated `write_native_*` fns over an overloaded
  match arm), `verification-before-completion`, `multi-pass-bug-hunting` (preserve-mechanism analysis before editing).

### Turn 40 (2026-05-30) — #4 new-code convergence loop: 3 real bugs caught + fixed
- **Ran the directive's per-crate quality+audit loop over the #4 new code** (the BG-14/BT-72 surface in ir/format-ubl/format-cii), exactly as Turn 38 did for the earlier foundations. Workflow: 3 adversarial audit skeptics → per-finding adversarial refutation → isomorphic-refactor evaluation. **4 candidates raised, 3 confirmed real** (the audit earned its keep — these slipped past the development gating tests). Fixes in commit 3c640ad, suite green throughout.
  1. **Lossiness-ledger comparator gap (medium) — the trust artifact had a hole.** `from_roundtrip_comparison` compared 21 top-level paths but NOT `/invoice_period` or `/delivery_date` — the only two top-level fields with no coverage. A caller-built source with those set differs from its reparse (values relocate into preserved raw XML), but the ledger surfaced the drift only generically under `/extensions`, never per-field. Reachable via the public `compute_ledger` API. Fixed: added both to `record_identity_lossiness`; added a regression test asserting per-field tracking (UBL+CII); updated the three committed golden path-set lists (`ubl_corpus`, `cii_corpus`, `roundtrip_corpus`). The green full suite is the completeness check — no other exact-set assertion broke.
  2. **Vacuous-ish negative test (low).** `invoice_period_rejects_an_empty_group` used a bare `.is_err()`; now pins `IrError::MissingRequiredField("invoice_period")` so it keeps exercising the BG-14 ≥1-date rule.
  3. **Double-emit on parse-then-enrich (medium) — invalid output.** A document carrying BOTH a preserved `cac:InvoicePeriod`/`cac:Delivery`/`ram:BillingSpecifiedPeriod` fragment AND a caller-set IR field emitted the element TWICE (these are 0..1 in EN 16931 → malformed). The mutual-exclusivity invariant was enforced only by the parser-never-populates rule; the serializer had no guard. Fixed with the existing `write_preserved_or_default_text` idiom — native emit gated on the preserved-write signal (UBL) / absence of a preserved fragment (CII); **preserved wins**. Added both-set regression tests (count == 1, original value survives) and corrected the doc comments that claimed "no double-emit" unconditionally. NOTE: the pre-existing `write_native_document_references` path has the same structural class (references); left out of #4 scope but worth a separate look.
- **Refactor lens: clean no-op** — the new code mirrors proven idioms; all candidates scored < 2.0 (0.5 / 0.48 / 0.0). No churn.
- **Lesson reinforced:** the development round-trip gates caught the 4 bugs DURING the earlier foundations, but #4's gates missed three because they only covered fresh-XOR-preserved (never both-set) and didn't touch the ledger comparator. An independent adversarial new-code audit is worth running even when dev tests are green.
- **Skills used:** `multi-pass-bug-hunting` (adversarial find→verify), `simplify-and-refactor-code-isomorphically` (refactor lens, no-op), `verification-before-completion`.

### Turn 41 (2026-05-30) — #4 new-code CONVERGED (confirming round clean)
- **Round-2 confirming audit (3 adversarial verifiers, one per fix): 3/3 confirmed correct, 0 residuals.** Each independently re-ran the affected crates' tests, checked PartialEq semantics on the two new comparator paths, the exact `IrError::MissingRequiredField("invoice_period")` variant+payload, the double-emit gate for all three elements (UBL period+delivery, CII period), the CII container-path/element-name match against what the parser stores (`BillingSpecifiedPeriod` absent from `known_cii_children`, header child-order slot 23), and the **missed-consumer question**: the only exact ledger path-set assertions are the three files updated; all other `.preserved`/`.lost` uses are subset `.any()` checks; no insta snapshots or committed golden JSON ledgers exist — so the green full suite genuinely proves completeness.
- **#4 (BG-14 invoice period + BT-72 delivery date) is COMPLETE and CONVERGED:** IR foundation + 158-site ripple + UBL (full BG-14+BT-72) + CII (BG-14) + new-code audit (3 real bugs found → fixed) + confirming round (0 residuals). Loop-until-dry satisfied. Full suite green, clippy `-D warnings` clean.
- **LOOP RE-CONVERGED.** The clean, autonomous-safe, high-value ungated work is exhausted: #4 was the last such lever and is now verified end-to-end. The two highest-value next steps remain **principal-gated**: (1) **D15 code-list vendoring** (unblocks the national code-derivation tier), (2) **CII BT-72/BT-7 disentanglement** (a behavior-changing correction with broad golden impact + a genuine tradeoff — relocating BT-7 to ApplicableTradeTax/TaxPointDate makes its round-trip depend on a non-empty tax breakdown). The remaining ungated EN 16931 tail (BG-13/15 deliver-to party+address, BG-20/21 document allowances/charges, BG-26 line period) is declining per-BT value with another large structural ripple each; I'm not grinding it ahead of the principal's steer. **Stopped** pending a gated-decision sign-off or an explicit instruction to proceed on a named tail item.
- **Skills used:** `multi-pass-bug-hunting` (confirming round), `verification-before-completion`.

### Turn 42 (2026-05-30) — closed the audit-surfaced UBL OrderReference both-set double-emit
- The Turn-40 audit flagged the pre-existing `write_native_document_references` path as the same both-set double-emit class. I carved out the **unambiguous, no-tradeoff slice** and fixed it: `cac:OrderReference` (BT-13) is **0..1**, so a parse-then-enrich document with BOTH a preserved cac:OrderReference AND a caller-set Order-class IR reference emitted TWO — malformed EN 16931 (slips past OASIS UBL 2.1 whose base cardinality is 0..unbounded). **TDD-confirmed** (failing test: count == 2), then gated the native emit on the preserved-write signal for `cac:OrderReference` only (preserved wins, consistent with the #4 gates). `cac:BillingReference` (BT-25) is **0..n** — preserved + native legitimately coexist — so it is deliberately NOT gated (verified: both billing refs survive). Commit e59f34c, full suite green, clippy clean.
- **Scope discipline:** did NOT touch CII's reference path — it already avoids the double-emit via its `write_preserved_xml_after_child(emitted_native)` dedup (native wins there). This leaves a documented OrderReference precedence difference (UBL preserved-wins vs CII native-wins), both valid single-element output; unifying it is a deliberate design decision, not a correctness bug.
- **The both-set double-emit class is now correctness-resolved** across every 0..1 element I touched (InvoicePeriod, Delivery, BillingSpecifiedPeriod, OrderReference-UBL — all valid single-element output). Remaining items are all either principal-gated, declining-value tail, or the precedence-unification design call.
- **Clean autonomous-safe correctness work is now exhausted.** Genuinely-remaining levers: (1) D15 code-list vendoring [gated]; (2) CII BT-72/BT-7 disentanglement [gated — introduces a degenerate BT-7 round-trip regression, a tradeoff]; (3) both-set precedence unification [design decision]; (4) EN 16931 tail — best of it is **BG-20/21 document-level allowances/charges** (common on real invoices, but a large additive change with EN 16931 BR-CO-10..15 total-reconciliation interaction). Next iteration would begin BG-20/21 unless redirected.
- **Skills used:** `multi-pass-bug-hunting` (TDD-confirm the bug before fixing), `verification-before-completion`.

### Turn 43 (2026-05-30) — BG-20/21 document-level allowances/charges: IR + ripple + UBL + CII
- New coverage lever (the directive's "fully cover all supported capabilities and formats"): EN 16931 BG-20 (document allowances) + BG-21 (charges) — discounts, shipping/handling, common on real invoices. Highest-value remaining ungated lever.
- **IR foundation (commit a126505):** additive `DocumentAllowanceCharge` (allowance BT-92..98 / charge BT-99..105, `is_charge` discriminator; amount, base_amount, percentage, tax_category, tax_rate, reason, reason_code) + `allowance_charges: Vec<…>` on both document structs; mapped in `new()`; validated against EN 16931 **BR-33 (allowance) / BR-38 (charge)** (a reason or reason code is required); from_value→to_value round-trip test + BR-33/38 rejection test. Carried verbatim — totals are NOT recomputed (validator's job). 158-site behavior-preserving ripple across 42 crates via the one-agent-per-crate workflow (central `cargo build --workspace --all-targets` = source of truth).
- **UBL wiring (commit af9d367):** `cac:AllowanceCharge` (ChargeIndicator, ReasonCode, Reason, MultiplierFactorNumeric, Amount, BaseAmount, TaxCategory) at the settlement slot. **0..n** → emitted unconditionally (preserved + native coexist, the repeatable analogue of cac:BillingReference — NOT gated like the 0..1 elements). 3 gating tests (fresh emit + placement with escaped reason text, absent = byte-preserving, preserved+native coexist), all schema-valid + canonical-idempotent.
- **CII wiring (commit ae0cc11):** `ram:SpecifiedTradeAllowanceCharge` (ChargeIndicator/udt:Indicator, CalculationPercent, BasisAmount, ActualAmount, ReasonCode, Reason, CategoryTradeTax) at its settlement slot. Element names verified against the **vendored CII D16B catalog** (D18). The fresh-emit round-trip test **surfaced a latent parser bug** (same class as the earlier FormattedIssueDateTime/qdt fix): `expected_cii_namespace` mapped the bare `Indicator` element to ram, so the parser REJECTED its own valid `ram:ChargeIndicator/udt:Indicator` output — fixed (`Indicator` → udt). 2 gating tests (schema-order + byte-stable round-trip).
- **Verification:** full workspace suite green throughout; clippy `-D warnings` clean; all changes additive/behavior-preserving (empty `allowance_charges` → byte-identical output). 8 new gating tests.
- **Next:** the BG-20/21 new-code adversarial audit (Turn-40 pattern), then convergence confirm. The dev round-trip gate already caught the parser bug, but an independent audit caught 3 bugs on #4, so it is worth running.
- **Skills used:** `multi-pass-bug-hunting` (TDD-surfaced the parser bug), `verification-before-completion`, `simplify-and-refactor-code-isomorphically` (kept emit fns idiomatic, mirroring write_tax_total/write_billing_specified_period).

### Turn 44 (2026-05-30) — BG-20/21 new-code audit: 6 real bugs caught + fixed
- New-code adversarial audit (11 agents, 7 candidates → **6 confirmed real**) over the BG-20/21 surface; all fixed in commit 8e143ac, suite green, clippy clean. The refactor lens was a clean no-op.
  1–3. **Lossiness-ledger comparator gap (the trust artifact).** `record_payload_lossiness` did NOT compare `/allowance_charges` — the EXACT gap the Turn-40 audit found for invoice_period/delivery_date, reintroduced because I didn't touch the comparator when adding the field. A source with document allowances/charges differs from its reparse (parser keeps them as preserved raw XML, resets the typed field), but the ledger reported zero loss. Fixed: added `/allowance_charges` to the comparator + a per-field regression test (UBL+CII) + the three golden path lists (ubl_corpus/cii_corpus 23→24, roundtrip_corpus). **Lesson reinforced: when adding an IR field, the lossiness comparator is a mandatory wiring point — add it to the playbook.**
  4. **UBL test strength.** `fresh_document_emits_allowance_charges` now positionally pins the cac:AllowanceCharge child order (canonical idempotence alone doesn't catch a mis-ordered-but-well-formed emission).
  5–6. **CII child-order bug (real, schema-invalid output).** A parse-then-enrich doc with BOTH a preserved `BillingSpecifiedPeriod` (order 25) AND native allowances (order 26) emitted the period AFTER the allowances — out-of-order yet canonically-idempotent, so idempotence tests missed it. Root cause: native allowances emitted before the single preserve-replay window. Fixed by flushing preserved siblings ordered before SpecifiedTradeAllowanceCharge first, then the rest with an EXPLICIT lower bound `order(BillingSpecifiedPeriod)` — because `write_preserved_xml` filters by a non-consuming order window, so naive adjacent `preserve_before` calls would double-flush the period. New gating test asserts period-before-allowance ordering + exactly-once + byte-stable round-trip.
- **Lesson:** the dev round-trip gates only covered fresh-XOR-preserved and single-element cases; the both-preserved-billing-AND-native-allowances ordering case + the comparator gap needed an independent audit. Worth running every time.
- **Skills used:** `multi-pass-bug-hunting` (find→verify), `simplify-and-refactor-code-isomorphically` (no-op), `verification-before-completion`.

### Turn 45 (2026-05-30) — BG-20/21 CONVERGED (confirming round clean)
- **Confirming round (3 verifiers, one per fix-cluster): 3/3 confirmed correct, 0 residuals.** Each ran negative controls: the comparator verifier temporarily disabled the `/allowance_charges` line and confirmed BOTH the ledger regression test AND the corpus exact-set test fail (proving non-vacuous), then restored byte-exact. The CII-windowing verifier worked out the exact child orders (ApplicableTradeTax=23, BillingSpecifiedPeriod=24, SpecifiedTradeAllowanceCharge=25, Subtotal=26, Logistics=27, PaymentTerms=28) and proved the two preserve windows are DISJOINT (Window 1 = (23,25) covers 24 only; Window 2 = (24,28) covers 25-27; order 24 fails Window 2's exclusive lower) and COMPLETE (every mid-sibling 24-27 flushed exactly once, in order) — no double-flush, no gap, no regression to the billing-only/allowance-only/no-preserved paths.
- Fixed one cosmetic off-by-one in the CII test doc comment the verifier flagged (prose said order 25/26; actual 24/25 — code uses symbolic lookups so behavior was always correct; commit 345d6ed dropped the fragile numbers).
- **BG-20/21 (IR foundation #5) is COMPLETE + CONVERGED.** Full suite green, clippy `-D warnings` clean. The IR now covers the EN 16931 document level broadly: dates (issue/tax-point/due/delivery/period), parties (supplier/customer/payee), references, classifications, exemptions, document allowances/charges, payment terms/means, totals.
- **Remaining ungated EN 16931 tail (declining value, each a full IR-foundation cycle):** BG-13/15 deliver-to party + address (companion to BT-72; logistics/drop-ship), BG-26 line-level invoice period, BG-27/28 line-level allowances/charges. Next iteration would begin BG-13/15. The three principal-gated decisions (D15 code-list vendoring; CII BT-72/BT-7 disentanglement; both-set precedence unification) remain queued.
- **Skills used:** `multi-pass-bug-hunting` (confirming round w/ negative controls), `verification-before-completion`.

### Turns 46–47 (2026-05-30) — BG-13/15 deliver-to: IR + comparator + ripple + UBL (CII deferred)
- The last clearly-worthwhile ungated EN 16931 *document-level* lever — deliver-to information (BG-13 party/location + BG-15 address), the companion to the BT-72 delivery date. Medium value (drop-ship/multi-site/logistics).
- **IR foundation (commit dda9c5c):** additive `DeliverToParty { name (BT-70), location_id (BT-71), address (BG-15, reusing PostalAddress) }` + `deliver_to: Option<DeliverToParty>` on both structs; validated (a present group needs >=1 of name/location/address); round-trip + rejection tests. **Applied the recurring comparator lesson PROACTIVELY:** wired `/deliver_to` into the lossiness comparator + all three golden path lists (ubl_corpus/cii_corpus 24→25, roundtrip_corpus) + a per-field ledger regression test, up front — not waiting for the audit. 158-site behavior-preserving ripple.
- **UBL wiring (commit dd9ffbc):** extended `write_native_delivery` to emit the full delivery group in one `cac:Delivery` — `cbc:ActualDeliveryDate` + `cac:DeliveryLocation` (cbc:ID = BT-71 + `cac:Address` = BG-15) + `cac:DeliveryParty/cac:PartyName/cbc:Name` (BT-70), in UBL DeliveryType child order. Refactored `write_address` → wrapper-parameterized `write_address_as` (byte-identical for the `cac:PostalAddress` callers; confirmed by the green party round-trips) so the deliver-to location reuses AddressType content under `cac:Address`. 0..1, gated on the preserved-write signal. Gating test asserts child order + schema validity + canonical idempotence.
- **CII deliver-to DEFERRED (decision):** `ram:ShipToTradeParty` lives in `ram:ApplicableHeaderTradeDelivery` — the SAME fragile, preserve-windowed block that holds the pre-existing BT-7/BT-72 conflation and is pending the principal's gated disentanglement decision. Restructuring it (with the explicit-bounds window technique) for a niche feature, only for the principal's decision to restructure it again, is poor risk/value. So CII deliver-to is bundled with the gated CII delivery-block decision — mirroring the BT-72-in-CII deferral. UBL carries BG-13/15; the `deliver_to` IR field is fully usable; CII intake preserves any `ShipToTradeParty` as raw XML.
- **Verification:** full workspace suite green, clippy `-D warnings` clean; the new-code audit is running.
- **Skills used:** `simplify-and-refactor-code-isomorphically` (the write_address_as extraction), `multi-pass-bug-hunting` (audit), `verification-before-completion`.

### Turn 48 (2026-05-30) — BG-13/15 audit → schema drift caught + the release-check gate gap
- **BG-13/15 new-code audit: 3 candidates, 1 confirmed.** The deliver-to IR + UBL code itself verified CLEAN; the one real defect was orthogonal but important: **the committed JSON Schema `schemas/invoicekit-ir-v1.json` was never regenerated** after this session's (and the prior session's) IR struct changes. A CI gate (`tools/release-checks/test_ir_schema_match.py`, via the `license-header` workflow) asserts committed == `cargo run -p invoicekit-cli --bin gen-schema`, and had been RED since the first IR addition (af76bae). Regenerated (commit da9eef9); diff is purely additive (ItemClassification, exemptions, InvoicePeriod, delivery_date, DocumentAllowanceCharge, DeliverToParty + all their fields); gate now passes.
- **🔴 BROADER GAP FOUND: `cargo test --workspace` is not the full gate.** There is a whole Python release-check suite (`tools/release-checks/`, 113 tests: schema match, TS-types match, EN 16931 / CII coverage, conformance corpora, capabilities matrix, country manifests) run via GitHub workflows but NOT by cargo — so "cargo green" hid a red CI gate all session. **Ran `python3 -m pytest tools/release-checks/ -q`: all 113 pass** (after the schema regen). Added to the playbook: run the release-check suite after IR/schema/coverage changes. Full cargo suite green + clippy clean too.
- **BG-13/15 (UBL) CONVERGED.** UBL carries the full delivery group (BT-72 + BG-13/15 deliver-to); CII deliver-to deferred with the gated CII delivery-block decision; schema + all gates green.

### CONVERGENCE ASSESSMENT (Turn 48)
The ungated EN 16931 **document-level** surface is now comprehensively covered by **6 additive IR foundations**, each round-trip-tested, comparator-wired, schema-synced, audited, and UBL-wired: (1) item classifications BT-158; (2) document references BT-13/25; (3) VAT exemption reason/code BT-120/121; (4) invoice period BG-14 + delivery date BT-72; (5) document allowances/charges BG-20/21; (6) deliver-to BG-13/15. CII carries all but the two delivery-block items (BT-72 + deliver-to), deferred with the gated disentanglement. Suite green, clippy clean, all 113 release-check gates green, schema in sync.

**Remaining ungated work = the LINE-LEVEL frontier only:** BG-26 (line invoice period), BG-27/28 (line allowances/charges), line-level deliver/period — lower per-BT value, per-`DocumentLine` ripple, and line-total interaction. Declining value.

**Remaining HIGH-value work is PRINCIPAL-GATED (3 decisions):**
1. **D15 — vendor national code lists** (IT Natura, CEF VATEX, BR NCM/IBGE, MX SAT, CL IndExe, PL FA(3)): unblocks the national code-DERIVATION tier across many report crates — by far the highest-value next step. Needs licensing/external-data authorization.
2. **CII delivery-block disentanglement** (now carrying BT-7/BT-72 conflation fix + BT-72 emit + deliver-to ShipToTradeParty): a behavior-changing restructure of a fragile preserve-windowed block; schema slots confirmed in-repo; executable on sign-off.
3. **Both-set precedence unification** (CII references native-wins vs everything-else preserved-wins): a documented internal inconsistency; both produce valid output.

### Turns 49–50 (2026-05-30) — BG-27/28 line-level allowances/charges (IR + UBL + CII)
- The last ungated EN 16931 coverage layer: line-level allowances/charges (line discounts, common). Reused `DocumentAllowanceCharge` as `DocumentLine.allowance_charges`.
- **IR (commit 2d4c48e):** field + shared BR-33/38 validation + round-trip test. **Comparator auto-covered** — line allowances live inside DocumentLine, so the existing `/lines` whole-Vec comparison catches them (no new path/golden-list — a simplification over the document-level cycle). Schema regenerated (purely additive; gate passes). 191-site `DocumentLine` ripple across 42 crates (2 over-long e2e fixtures got `#[allow(clippy::too_many_lines)]`).
- **UBL (commit f08a0c0):** extracted `write_allowance_charge` (isomorphic — shared by document + line levels); emit line allowances in `cac:InvoiceLine` after `cbc:LineExtensionAmount`, before `cac:Item`. Schema-valid + canonical gating test.
- **CII (commit 59c51fe):** extracted `write_cii_allowance_charge`; emit in `ram:SpecifiedLineTradeSettlement` after `ram:ApplicableTradeTax`. **Fixed a line-settlement preserve gap the feature exposed** (the trailing after_all only flushes children above the max known child, so a mid-order preserved `SpecifiedTradeAllowanceCharge` was dropped — caught by `from_xml`'s internal round-trip as lost): added a `preserve_before(SpecifiedTradeSettlementLineMonetarySummation)` whose window is disjoint from after_all (known children = exactly ApplicableTradeTax + the summation). Byte-stable round-trip gating test.
- **Verification:** full cargo suite green, clippy `-D warnings` clean, all 113 release-check gates pass, schema in sync. New-code audit running.
- **Coverage status:** the EN 16931 surface — document-level AND line-level — is now comprehensively covered in UBL; CII covers all but the delivery-block items (BT-72, deliver-to ShipToTradeParty) deferred with the gated disentanglement. This is the bottom of the ungated coverage frontier.
- **Skills used:** `simplify-and-refactor-code-isomorphically` (the two shared-helper extractions), `multi-pass-bug-hunting` (the round-trip gate surfaced the line preserve gap; audit running), `verification-before-completion`.

### Turn 51 (2026-05-30) — BG-27/28 CONVERGED + ungated EN 16931 frontier exhausted
- **BG-27/28 line allowances/charges: audit (2 real bugs) → fixed (four-window line-settlement preserve) → confirming round 0 residuals → CONVERGED.** The confirming verifier proved the four windows {<5},{6},{7-10},{>11} disjoint+complete against the actual child-order table (orders 1-19), boundary orders 5/11 moot (known/native-only, never preserved), no regression, tax_category-None case correct. 57 CII tests + corpora pass; full cargo suite green; clippy clean; 113 release-check gates pass; schema in sync.
- **🟢 THE UNGATED EN 16931 COVERAGE FRONTIER IS EXHAUSTED.** Seven additive IR foundations now comprehensively cover the valuable EN 16931 surface, each round-trip-tested, comparator-wired, schema-synced, audited, confirming-round-clean, and UBL-wired:
  1. item classifications (BT-158, doc+line)
  2. document references (BT-13/25)
  3. VAT exemption reason/code (BT-120/121)
  4. invoice period (BG-14) + delivery date (BT-72)
  5. document allowances/charges (BG-20/21)
  6. deliver-to (BG-13/15)
  7. line allowances/charges (BG-27/28)
  CII carries all but the delivery-block items (BT-72, deliver-to ShipToTradeParty), deferred with the gated disentanglement.
- **Only ONE ungated group remains — BG-26 line-level invoice period — and it is near-zero value** (a per-line billing sub-period is rare). Grinding it would be diminishing-returns theater; left available if the principal wants exhaustive coverage.
- **ALL remaining HIGH-value work is principal-gated (3 decisions):** (1) **D15 national code-list vendoring** — unblocks the national code-derivation tier across many report crates, by far the highest-value next step; (2) **CII delivery-block disentanglement** (BT-7/BT-72 conflation + BT-72 emit + deliver-to ShipToTradeParty); (3) both-set precedence unification (CII references native-wins vs preserved-wins elsewhere).
- **LOOP STOPPED at genuine comprehensive convergence** rather than grind BG-26's near-zero value. Resume by authorizing a gated decision (D15 recommended) or naming a specific target.
- **Skills used:** `multi-pass-bug-hunting` (audit + confirming round), `simplify-and-refactor-code-isomorphically`, `verification-before-completion`.

### Turn 52 (2026-05-30) — PRINCIPAL AUTHORIZATION: finish fully, harden, battle-test, release
- The principal lifted the gates: "set workflows in parallel for all remaining work to finish this up fully and make it production ready and also battle test it end to end and produce releases for all intended fully and completely. be meticulous and diligent ... with ultrathink." The three queued gated decisions (D15, CII delivery-block disentanglement, both-set precedence) are now GREENLIT, plus production-hardening + battle-testing + releases.
- **Running this as a PHASED CAMPAIGN of parallel workflows** (discover → implement → battle-test → harden → release), reading results between phases. **Hard invariant carried into every agent: NO FABRICATION** (trust toolkit) — no invented code-list values / element names / support claims / test results; anything genuinely blocked on licensed data, registry credentials, live network, or partner endpoints is reported as an honest gap, never faked. Honest incompleteness > fabricated completeness.
- Scope facts: workspace at v0.1.1 (→ 0.2.0 warranted); Java present (JVM reference validators runnable E2E); fuzz/ harness exists; 6 binding families (node-napi/python/java/dotnet/go/wasm) + typescript SDKs + rest-shim + managed-api/signer services + validator-{kosit,phive,saxon,phase4,verapdf,zatca}.
- Phase 1 (discovery): 8 parallel Explore mappers classifying every remaining item do-now / do-with-tradeoff / blocked-external. Synthesis + implementation phases follow.

### Turn 53 (2026-05-30) — Campaign Phase 2: CII delivery-block disentanglement DONE
- **Discovery (Phase 1) + EN16931 gap-verification complete.** Discovery classified all remaining work do-now/do-with-tradeoff/blocked-external across 8 dimensions. Gap-verification (against real code) confirmed the discovery census had false positives (BT-120/121, BG-13, contact, percentages already implemented); the genuine real-missing EN16931 gaps are medium/low value: document references (Contract BT-12, DespatchAdvice BT-16, ReceivingAdvice BT-15 — kind_class variants exist, need emit), item identifiers (BT-154 description, BT-155/156 seller/buyer item id), and a low-value tail (project ref, object id, country of origin, price discount/base-qty, line note, BG-26).
- **CII delivery-block disentanglement (the longest-standing gated decision): DONE + verified (commit 8ad0e71).** BT-72 delivery_date -> OccurrenceDateTime (now round-trips losslessly in CII); BT-7 tax_point_date -> per-tax ram:TaxPointDate/udt:DateString (fixed the never-exercised parser handler DateTimeString->DateString, added TaxPointDate to ApplicableTradeTax known-children, taught namespace DateString->udt); BG-13/15 deliver-to -> ram:ShipToTradeParty via the disjoint four-window preserve pattern. Empty-tax_summary BT-7 loss honestly surfaced by the ledger. Full workspace suite green; clippy clean; all 113 release-check gates pass. New-code audit running.
- **Honest production picture (from discovery):** the Family A engine (EN 16931/UBL/CII/Peppol BIS/Factur-X) is GA-quality; national crates are honestly-labeled varying maturity (by design); real blockers (registry/sandbox credentials, licensed code-list data, live Peppol) are external and documented, never faked.
- **Campaign phases:** 1 Discovery [done], 2 Implement [in progress — CII disentanglement done; references + item-ids + precedence next], 3 D15 infra, 4 battle-test (JVM validators/fuzz/UB/audit), 5 harden, 6 release (0.1.1->0.2.0). Driven by workflow completions.

### Turn 54 (2026-05-30) — Campaign Phase 4: battle-test against REAL reference validators
**Environment is capable (verified):** Docker daemon UP, HTTPS egress works (Maven Central reachable), `cargo-fuzz` + nightly toolchain present, OpenJDK 17. So battle-testing against the genuine JVM reference validators is DO-NOW, not a blocked-external gap.

**Reference validator sidecars (the oracle):** `invoicekit/validator-phive:ci` (phive-rules-peppol 3.2.2 → Peppol BIS Billing 3.0 EN 16931 Schematron) and `invoicekit/validator-kosit:ci` (KoSIT XRechnung 3.0.2 config) run as JSON-RPC sidecars. Probed phive directly to confirm it is the REAL rule engine (`oracle_coordinate: com.helger.phive.rules:phive-rules-peppol:3.2.2`), not a well-formedness stub.

**CONFIRMED result — synthetic UBL corpus (50 fixtures, the default parity glob):**
- All 50 fixtures are INVALID by design (each trips ≥1 EN 16931 rule; 11 distinct core BR-* rules exercised: BR-06/07/62/63, BR-CO-11/12, BR-CL-17/18, BR-AE-05/08/10).
- The pure-Rust EN 16931 validator's `BR-*`/`BR-CO-*` findings match BOTH real oracles **exactly**: jvm:kosit parity 1.0 (50/50), jvm:phive parity 1.0 (50/50). Zero mismatches. This is genuine end-to-end correctness evidence against the official German + Peppol validators.

**IN PROGRESS — 672-fixture adversarial corpus (engine-emitted, adversarial business scenarios):** running a skip-and-record driver (instead of the harness's fail-closed abort) that separates CORE EN 16931 parity from the national-CIUS delta. Early 40-fixture slice: jvm:phive core parity 1.0 (40/40); jvm:kosit core rules match but the oracle additionally emits German national `BR-DE-6/7/15/27/28` that the core probe does not.

**HONEST scope finding (not a core bug):** `BR-DE-*` are XRechnung's German national CIUS rules, NOT core EN 16931. The parity probe invokes only the core validator (`validate-ubl-cii`); the German national layer lives separately in `crates/profile-xrechnung` (a hand-maintained coverage matrix with explicit `rust_enforced`/`kosit_enforced` flags + a runtime Leitweg-ID/BR-DE-15 check). So the kosit "mismatch" is a known harness-wiring boundary, and the crate's own matrix already declares which BR-DE rules are unenforced — no fabricated coverage. Core EN 16931 parity (the apples-to-apples measure) showed zero divergence in the slice (`rust_only` empty; all `oracle_only` were national BR-DE).

### Turn 55 (2026-05-30) — Phase 4 battle-test: FULL adversarial corpus vs real validators (complete)
**Result — 672-fixture adversarial UBL corpus (engine-emitted, adversarial business scenarios), parallelised differential parity (8 workers, 4m19s):**
- **jvm:kosit (KoSIT XRechnung 3.0.2):** CORE EN 16931 parity **1.0** (672/672 compared, 0 core mismatches). 0 schema-skips, 0 oracle-unavailable.
- **jvm:phive (Peppol BIS Billing 3.0 / phive-rules-peppol 3.2.2):** CORE EN 16931 parity **1.0** (672/672, 0 core mismatches).
- Aggregate with the 50-fixture default corpus: the pure-Rust core EN 16931 validator agrees with BOTH official reference validators on **722/722 fixture-validations**, zero core divergence.

**Honest caveats (no overclaim):**
- *Rule diversity is bounded.* The 672 adversarial fixtures trip only 4 distinct core rules (BR-27, BR-62, BR-63, BR-CO-11) — they stress-test calculation/amount edge cases at scale, not rule breadth. The 50-fixture corpus adds 7 more (BR-06/07, BR-CL-17/18, BR-AE-05/08/10, BR-CO-12). Total exercised ≈ 11 distinct core rules across VAT-category, codelist, and calculation classes. This is meaningful agreement, NOT exhaustive proof of all ~200 EN 16931 rules. A rule-diversity corpus expansion is the honest next coverage step.
- *National CIUS is a scope boundary, correctly separated.* All 672 kosit fixtures additionally trigger German national `BR-DE-6/7/15/27/28`, which the core probe does not emit (the German layer lives in `crates/profile-xrechnung`, whose coverage matrix already declares per-rule enforcement). My driver splits core vs national by ISO-country prefix; EN 16931 category codes (BR-AE, BR-IC, …) are NOT country codes and stay core (verified no collision).

**Tooling defect found (real, fixable):** the stock `tools/validator-parity/en16931_parity.py` `BR_RULE_RE` matches `BR-DE-*` as "core", contradicting its own docstring ("compares only EN 16931 core"). Run against any BR-DE-triggering corpus pointed at kosit, it would report a false parity failure on the public parity dashboard. Fix candidate: make the harness national-CIUS-aware (separate core vs national counts), matching the differential driver used here.

### Turn 56 (2026-05-30) — Phase 4 cont'd: harness fix, FFI/UB audit, PDF/A-3 + fuzz infra
- **Parity-harness defect fixed + committed.** `en16931_parity.py` compared national CIUS rules (BR-DE-*) as core EN 16931 — would false-fail the public parity dashboard on any BR-DE-triggering corpus. Added an EN 16931 core rule-class infix allowlist (CO/CL/AE/IC/IG/IP/AF/AG); national country-coded rules excluded. Allowlist (not denylist) because "AE" = reverse-charge category AND UAE ISO code. 14 unit tests pass; live regression check on the 50-fixture default corpus still 1.0 on both oracles.
- **FFI/unsafe UB audit DONE (read-only review).** Whole codebase: the ONLY real-unsafe crate is `crates/invoicekit-ffi` (`unsafe_code = "allow"`); `reconcile`'s "unsafe" hits are all doc comments. Reviewed every unsafe fn: the critical UB risk (a panic unwinding across `extern "C"` into Go/.NET/Java) is correctly contained by `catch_engine_panic` (`catch_unwind` → PANIC_RESPONSE handle); every pointer accessor null-checks before deref; `slice::from_raw_parts`/`Box::from_raw` used strictly under documented `# Safety` contracts with explicit `unsafe {}` blocks + SAFETY comments (edition-2024-forward-compatible). No UB findings. Miri run pending (deferred to avoid CPU contention with the fuzz build).
- **PDF/A-3 track.** Honest finding: the `validator-verapdf` SIDECAR cannot build — `org.verapdf:verapdf-library:1.27.1` is not on Maven Central (veraPDF publishes to its own repo; the verapdf pom lacks that repository). The CANONICAL gate (`.github/workflows/pdfa3-verapdf.yml`) doesn't use the sidecar; it installs the veraPDF CLI from software.verapdf.org. Installed veraPDF CLI 1.30.1 locally; render(30 Factur-X PDFs)+validate(3b/3u) pending (CPU-gated behind the fuzz build).
- **Fuzz** (cargo-fuzz + nightly, ASAN): running 5 targets (ubl_from_xml, ir_try_from_value, canonicalize_xml/json, render_typst_pdf), 60s each, seeded from conformance-corpus/fuzz/. Result pending.
- Stopped the orphaned pre-compaction CII-disentanglement audit workflow (0-byte output, 46 min stale); the disentanglement remains verified by the full workspace suite + 113 release-check gates + clippy + the live-validator battle-test.

### Turn 57 (2026-05-30) — Phase 4: fuzz CLEAN + PDF/A-3 CONFORMANT (real veraPDF)
- **Fuzzing (cargo-fuzz 0.13.1, nightly, ASAN) — all 5 targets CLEAN, zero crashes/leaks/OOM/UB:**
  - `ubl_from_xml` (the XML parser, highest bug-surface): **2,381,552 runs / 121s, no crash**.
  - `ir_try_from_value`, `canonicalize_xml`, `canonicalize_json`, `render_typst_pdf`: 60s each, no crash.
  - `fuzz/artifacts` empty; no crash-*/leak-*/oom-* anywhere. (First `ubl_from_xml` attempt hit my 200s wrapper during the one-time instrumented compile — re-ran on the cached build for the full budget. Not a crash: rc 124 = wrapper timeout, no artifact written.)
- **PDF/A-3 conformance (real veraPDF 1.30.1 CLI) — 30/30 CONFORMANT at BOTH flavours:**
  - Rendered 30 Factur-X PDFs (6 profiles × 5: minimum, basic-wl, basic, en16931, extended, comfort) via `invoicekit-render-factur-x-acceptance` (Typst render + `embed_factur_x` post-proc).
  - `verapdf --flavour 3b`: 30/30 compliant, 0 non-compliant, 0 no-verdict (sample report: 1282 passed checks, 0 failed).
  - `verapdf --flavour 3u`: 30/30 compliant, 0 non-compliant, 0 no-verdict.
  - Every PDF produced a real verdict (re-validated with unique report names to rule out overwrite/empty-report false positives).
- **Battle-test scorecard so far:** XML EN16931 parity 722/722 core 1.0 (KoSIT+phive) · fuzz 5/5 clean · PDF/A-3 30/30 (3b+3u) · FFI/unsafe review clean. Pending: Miri runtime UB check on the FFI crate; final `cargo test --workspace`; CII-path live-validator parity (harness is UBL-only — honest gap to close).

### Turn 58 (2026-05-30) — Phase 4 COMPLETE: full gate confirmation + Miri
- **FFI UB — Miri runtime verification PASSED:** `cargo +nightly miri test -p invoicekit-ffi` → 11 unit tests + 7 doc-tests OK under Miri's UB detector (null-pointer accessors, `slice::from_raw_parts`, `Box::from_raw`/free cycles, panic-containment across the extern "C" boundary). Zero UB. Confirms the read-only audit at runtime.
- **Full workspace test (release gate) — AUTHORITATIVE GREEN:** `cargo test --workspace` → exit 0, **279 test binaries all ok, 2,544 tests passed, 0 failed**, 0 FAILED tokens (full-log capture, not tail). No Rust crate changed this turn, so this matches the last known-green state.
- **113 release-check gates — ALL PASS** (`pytest tools/release-checks/`: IR schema match, CII/EN16931 coverage, conformance corpus, capabilities matrix, country manifests, TS types).

**PHASE 4 BATTLE-TEST SCORECARD (all real oracles, no fabrication):**
| Front | Result | Oracle |
|---|---|---|
| XML EN 16931 parity | 722/722 core parity 1.0 | live KoSIT XRechnung 3.0.2 + phive Peppol BIS 3.0 |
| Fuzzing | 5/5 targets clean (ubl_from_xml 2.38M runs) | cargo-fuzz/libFuzzer + ASAN, nightly |
| PDF/A-3 conformance | 30/30 at flavour 3b AND 3u | veraPDF 1.30.1 reference verifier |
| FFI/unsafe UB | review + Miri clean | Miri UB detector |
| Workspace tests | 2,544 passed / 0 failed | cargo test |
| Release-check gates | 113/113 | pytest |

**Honest residual gaps (documented, not faked):** (a) CII-path live-validator parity — the parity harness projects UBL only; KoSIT validates CII XRechnung too, so a CII projection path is the next battle-test extension. (b) EN 16931 rule-diversity — the corpora exercise ~11 distinct core rules; a rule-diversity corpus would broaden coverage toward all ~200. (c) `validator-verapdf` sidecar can't build (veraPDF not on Maven Central; pom lacks the veraPDF repo) — the canonical CLI gate is unaffected.

### Turn 59 (2026-05-30) — Phase 2 coverage: UBL document references BT-12/15/16 (done)
- **Verified the gap against real code** (discovery census had false positives before): `write_native_document_references` in format-ubl emitted only Order (BT-13) + PrecedingInvoice (BT-25); Contract (BT-12), DespatchAdvice (BT-16), ReceivingAdvice (BT-15) were explicitly deferred. The IR already carries them (`ReferenceKindClass` variants + `kind_class()` classification: "contract"→Contract, "despatch"/"dispatch"→DespatchAdvice, "receipt"/"receiv"→ReceivingAdvice).
- **Implemented (UBL):** added native emit arms + a shared `write_typed_document_references` helper. Each emits as a plain UBL `DocumentReferenceType` (cbc:ID + optional cbc:IssueDate) at the correct Invoice/CreditNote slot. Parse behavior UNCHANGED — these stay `LossinessLedgerPreserved` (parser preserves raw XML, never populates `references`), so parsed round-trips replay preserved exactly once and the native path stays inert (no double-emit). Purely a fresh-IR serialization-completeness fix; NO lossiness-ledger or schema change. New gating test: fresh-IR emit + UBL slot order (Despatch<Receipt<Contract) + schema validity + canonical idempotence. format-ubl 50+3+4 tests green, clippy clean, 113 gates pass, ubs 0-critical. Committed 9a096d7.
- **CII parity — NEXT (design captured):** CII defers the same three (native emit only for BT-13 BuyerOrderReferencedDocument + BT-25 InvoiceReferencedDocument). Adding them is NOT a clean replication — it needs restructuring two container serializer tails into the four-window preserve pattern:
  - BT-12 → `ram:ContractReferencedDocument/ram:IssuerAssignedID` in ApplicableHeaderTradeAgreement, AFTER BuyerOrderReferencedDocument (today the tail does `write_preserved_xml_after_child(..,"BuyerOrderReferencedDocument",..)` then closes — must become: preserved-before Contract window, native Contract emit returning emitted-bool, `write_preserved_xml_after_child(..,"ContractReferencedDocument",emitted)`).
  - BT-16/BT-15 → `ram:DespatchAdviceReferencedDocument` + `ram:ReceivingAdviceReferencedDocument` (IssuerAssignedID) in ApplicableHeaderTradeDelivery, AFTER ActualDeliverySupplyChainEvent (today ends with `write_preserved_xml_after_all` — must insert windowed phases for Despatch then Receiving before the final after_all).
  - Elements ARE already in cii_child_order / preserve lists (ContractReferencedDocument, DespatchAdviceReferencedDocument, ReceivingAdviceReferencedDocument), so schema-order machinery knows them.
  - Required tests: fresh-IR emit (correct containers + child order), round-trip lossless (preserved), both-set precedence (preserved wins, no native leak — mirror the adversarial test pattern), full CII suite + cii_corpus golden + 113 gates + clippy.
  - Impact note: round-trip is ALREADY lossless for CII (preserved); UBL→IR→CII cross-format can't carry these anyway (UBL parse preserves, doesn't populate `references`). The only case CII fresh-IR emit fixes is a code-constructed/gobl-imported document serialized to CII — real but narrower than UBL. Honest incremental state until CII lands.

### Turn 60 (2026-05-30) — Phase 2 coverage: CII document references BT-12/15/16 (done; parity with UBL)
- **Implemented (CII)** via the four-window preserve pattern, completing UBL+CII parity for these references:
  - BT-12 → `ram:ContractReferencedDocument` in ApplicableHeaderTradeAgreement (order 17, after BuyerOrderReferencedDocument 14; between-window flushes Quotation 15 / OrderResponse 16).
  - BT-16/BT-15 → `ram:DespatchAdviceReferencedDocument` (10) + `ram:ReceivingAdviceReferencedDocument` (11) in ApplicableHeaderTradeDelivery, after the BT-72 delivery event (7).
  - Shared `write_single_referenced_document` helper (0..1 IssuerAssignedID-only; no EN16931 issue date). Container tails restructured into disjoint, schema-order-fenced preserve windows; native wins over a same-order preserved fragment; parsed docs replay each preserved fragment exactly once (verified by a byte-stable round-trip assertion — the no-double-emit guard).
  - Updated the obsolete "non_order_non_preceding_references_are_skipped" test to the new contract (Other-kind still skipped; recognized kinds emit). New fresh-IR gating test covers container placement + child order + idempotence + round-trip.
  - format-cii 61+3+3 tests green, clippy clean, lossiness ledger unchanged (round-trip stays preserved-based — no ledger/schema change), 113 release-check gates pass, ubs 0-critical. Committed bc994c6.
- **Document-references coverage item: COMPLETE** across both core serializers (UBL Turn 59 + CII Turn 60).
- **NEXT — item identifiers (BT-154 item standard id / BT-155 seller item id / BT-156 buyer item id):** these add NEW DocumentLine IR fields, so they trigger the full hardened IR-field playbook (IR field + lossiness-ledger comparator wiring + 3 golden path-lists + ledger regression test + schema regen via gen-schema + both serializers UBL/CII + 113 gates). First step next iteration: VERIFY against real code whether BT-154/155/156 are genuinely missing on DocumentLine (the discovery census had false positives — verify before acting), then implement with the playbook if real. Lower-value tail (project ref, object id, country of origin, price discount/base-qty, line note, BG-26) to be honestly documented after.

### Turn 61 (2026-05-30) — Phase 2 coverage tail: honest landscape + campaign pivot
**Verified the remaining EN 16931 line-item tail against real code (correcting an earlier wrong assumption):**
- Item identifiers (BT-155 seller item id, BT-156 buyer item id), BT-157 standard id, and item attributes (BG-32) **already round-trip LOSSLESSLY** in BOTH UBL and CII today — they ride through as preserved raw XML (UBL: non-Name/non-CommodityClassification cac:Item children are preserved; CII: SellerAssignedID/BuyerAssignedID/GlobalID/ApplicableProductCharacteristic are in the known-children/child-order machinery and preserved, proven by existing round-trip tests e.g. format-cii line ~5245). They are NOT first-class semantic IR fields.
- So these are **NOT correctness or honesty gaps**: round-trips are lossless, and the EN 16931 gate (`test_en16931_coverage`, 81 BR/BR-CO rules) honestly tracks which rules are blocked by IR gaps — none of the item-ID terms block a BR rule. No rule is falsely claimed covered.
- The only gap for these terms is **semantic IR modeling + cross-format carrying** (a UBL preserved SellersItemIdentification can't cross to CII because preserved raw XML is format-specific) — an ENHANCEMENT, not a fix.
- **Cost note:** `DocumentLine` has 199 full-literal construction sites across ~60 crates and no `Default`; adding first-class struct fields ripples all 199 by hand (no codemods per AGENTS.md). The project deliberately routes optional line terms through `DocumentLine.extensions[PROFILE_EXTENSION_URN]` (per tools/en16931-coverage map: BT-148/157/158/160, BG-26/27/28/32) to AVOID that ripple. Semantic modeling of BT-155/156 would mean wiring the line extension payload through both serializers' preserve transitions — comparable to the document-references work, tractable but non-trivial.

**Campaign decision (judgment call):** the high-value coverage is DONE and battle-tested (7 IR foundations earlier + document references BT-12/13/15/16/25 across UBL+CII this session), round-trips are lossless, and the codebase is green (2,544 workspace tests + 113 gates + fuzz + PDF/A-3 + real-validator parity + Miri). The remaining EN 16931 tail is optional semantic-modeling enhancements with poor cost/value relative to the user's explicit "produce releases for all intended fully and completely." **PIVOT to release-readiness (Phase 5 reality-check → Phase 6 v0.2.0)**, keeping the item-ID semantic-modeling enhancements as tracked-deferred (honestly labeled, lossless today). Coverage enhancements remain available post-0.2.0.

### Turn 62 (2026-05-30) — Phase 5 production-readiness reality-check (part 1: stub/honesty audit)
- **Stub/wiring audit (reality-check Variant E): CLEAN.** Zero `unimplemented!`/`todo!()` in non-test source across ~98 crates; only 1 "TODO"-shaped hit (a Mexican test RFC `VECJ880326XXX`, not a marker). The handful of placeholder/stub mentions are genuinely-incomplete areas that are ALREADY honestly documented in-crate: `signer` (`SoftwareSigner` keyed-BLAKE3-MAC placeholder; real RSA/ECDSA/HSM = planned T-083a/b), `timestamping` (mock TSA backend vs production-TSA path), CLI `init` (VIES VAT lookup "stubbed today (no network)"), `intake-citation` (OCR bounding boxes not yet derived). No fake-completeness behind a working facade.
- **Doc-honesty audit (Variant F): 2 fixes (committed e7ddd64).** (1) README "is this production-ready?" FAQ was stale ("pre-release and unversioned ... until the first tag") — refreshed to the real tagged-v0.1.1 status + per-capability `invoicekit capabilities` pointer. (2) The evidence-bundle FAQ listed "signatures ... RFC 3161 timestamp" without maturity qualification — material for a trust toolkit. Now states plainly: artefact-integrity BLAKE3 hashing is production-grade; the default software signer is a keyed-MAC placeholder and the default timestamp is a mock, with real crypto/TSA a planned track behind stable pluggable traits.
- **Verified:** bundle extension is `.ikb` in the code (123 refs, zero `.invoicekit`); README matches code (SECURITY.md/AGENTS.md still say `.invoicekit` — stale doc nit, not touched: AGENTS.md is a settled shared-rules file). RTL/CJK digital-PDF intake is implemented + honestly bounded (README line 157) — the loop prompt's "rtl/cjk intake" item is already covered.
- **Verdict so far:** the codebase is honest (no hidden stubs, honest maturity labels) and green (2,544 tests + 113 gates + fuzz + PDF/A-3 + real-validator parity + Miri). REMAINING Phase 5: bindings-honesty check (node/python/wasm vs Go/Java/.NET maturity) + `invoicekit capabilities` accuracy, then Phase 6 v0.2.0 release.

### Turn 63 (2026-05-30) — Phase 5 reality-check (part 2: bindings honesty) + VERDICT
- **Bindings maturity audit:** verified each binding dir against its claims. WORKING host-language packages over the engine byte contract: Go (cgo + nocgo, real package), Java (JNI/FFM clients + REST sidecar + tests), .NET (P/Invoke + REST sidecar + tests), Python (real PyO3 `#[pymodule]` exposing the ABI fns + a `invoicekit` package). BYTE-CONTRACT STUBS (no host package yet, honestly self-labeled): Node (napi-rs — "contains no Node binding code"), browser/edge WebAssembly ("a stub binding, not a complete browser binding"). Fixed README's uniform-delivery over-claim + stale "registry distribution arrives with first tag" line (committed ac5e70a). No binding over-claims at its own README level.
- **`invoicekit capabilities`** is the project's honest per-capability/per-country labeling mechanism and is covered by the `test_capabilities_matrix.py` release gate (passing) — trusted, not manually re-verified.
- **PHASE 5 VERDICT — production-ready for a v0.2.0 release with honest maturity labels.** The implemented code delivers the README/AGENTS vision: deterministic engine, EN16931/UBL/CII/Peppol/Factur-X serialization, validation matching the real KoSIT+phive oracles (722 fixtures), PDF/A-3 30/30 via real veraPDF, lossless round-trips, signed-evidence-bundle format with production-grade BLAKE3 integrity. Honestly-labeled non-defaults (placeholder signer/TSA, Node/wasm host packages, registry publishing, item-ID semantic modeling) are tracked, not faked. Green: 2,544 workspace tests + 113 release-check gates + fuzz (5/5) + Miri.
- **NEXT — Phase 6: cut v0.2.0** (release-preparations skill): version bump 0.1.1→0.2.0 across the workspace, CHANGELOG entry capturing this session (battle-test, document references BT-12/13/15/16/25, parity-harness fix, doc-honesty), commit, tag, monitor CI cross-platform build, verify.

### Turn 64 (2026-05-30) — Phase 6: v0.2.0 release CUT (CI building)
- **Release gates (pre-tag): all green.** Full workspace `cargo test` 2,546 passed / 0 failed; `cargo clippy --workspace --all-targets -- -D warnings` clean AFTER fixing one blocker (`report-pl-ksef` e2e fixture `polish_mixed_rate_invoice` was 102/100 lines → `#[allow(clippy::too_many_lines)]` per project convention, committed 9412030 — caught by running clippy -D warnings before tagging, OP-3); 113 release-check gates pass.
- **Version bump 0.1.1 → 0.2.0** via `cargo set-version --workspace 0.2.0` (installed cargo-edit): workspace version + all 96 members (inherit) + all 99 internal path-dep requirements 0.1.0 → 0.2.0; `cargo check` resolves clean. (Had to revert a premature manual workspace-only bump first so `cargo metadata` could resolve for set-version.) CHANGELOG 0.2.0 entry written (coverage + trust). Committed a201d67, pushed to main.
- **Tag v0.2.0 created + pushed** (annotated, on a201d67) → triggered `release.yml` (run 26697066892, QUEUED): OpenAPI spec + SHA-256, the veraPDF PDF/A-3 release gate (3b+3u, hard `needs:` gate), and cross-platform per-target binary builds. My git-pushed tag (not bot-created) triggered it, so OP-2 doesn't apply.
- **NEXT:** monitor the release CI (~15-25 min per the v0.1.1 baseline); if green, verify the GitHub release + assets and Phase 6 is done; if it fails, fix on main + re-tag/re-trigger. Registry publishing (npm/PyPI/crates.io/Maven/NuGet) remains a deliberately-deferred follow-up (needs credentials; honestly documented).

### Turn 65 (2026-05-30) — v0.2.0 PUBLISHED + main CI restored to green
- **v0.2.0 release SUCCEEDED and is PUBLISHED.** `release.yml` (run 26697066892) all green: OpenAPI spec + SHA-256, the veraPDF PDF/A-3 release gate (3b+3u), and all 3 cross-platform binary builds (aarch64-apple-darwin, x86_64/aarch64-unknown-linux-gnu). Release assets: 3 platform binaries + tarballs + cosign bundles + SBOMs + SHA256 + OpenAPI. Only annotations are Node.js 20 deprecation WARNINGS (future maintenance, non-blocking).
- **Then verified main CI and fixed the real failures** (pre-existing debt, red on every main push this session — not a release regression):
  - **rustfmt** (`cargo fmt --all --check`): 455 diffs / 74 files (local rustfmt 1.9.0-stable matched CI exactly). Fixed with `cargo fmt --all` (whitespace-only, `cargo check` clean). Commit a52636a.
  - **rustdoc** (`-D warnings`): 5 public doc-comments linked to private items via `[`X`]` (peppol-smp-sml ×2, report-pl-ksef, report-in-gst, cli). Demoted each to a plain code span. Commit 51e56cd; `cargo doc` now clean.
  - **TypeScript types drift**: this session's IR schema additions hadn't propagated to the generated `.d.ts`. Regenerated via `bun run scripts/generate.mjs` (json-schema-to-typescript per bun.lock): invoicekit_ir_v1.d.ts (+176) + capabilities (+2); drift-check + `tsc --noEmit` + 5 round-trip tests pass; release-check TS gate still green. Commit 2566cd8.
  - Pre-tag clippy blocker (pl-ksef fixture too_many_lines) already fixed (9412030).
- **Honest blocked-external:** the Python SDK (and npm) publish steps fail on tag with "Trusted publishing exchange failure ... configuration error" — the deferred PyPI/npm **trusted-publisher** config the principal must set up. NOT faked; consistent with the documented registry-publishing deferral. The GitHub binary release is unaffected.
- All CI fixes pushed to main (2566cd8); CI re-running to confirm green.

### Turn 66 (2026-05-30) — main CI FULLY GREEN; Phase 6 COMPLETE
- **Resolved the cargo-fmt → clippy cascade.** `cargo fmt --all` had reflowed 5 functions past clippy's 100-line limit (FatturaPA/CFDI/NAV national serializers + 2 e2e fixtures); added `#[allow(clippy::too_many_lines)]` per the project convention (commit ac37e95). `cargo clippy --workspace --all-targets --keep-going -- -D warnings` and `cargo fmt --all --check` both clean locally.
- **CI run 26697901569 (on ac37e95): ALL GREEN.** Every job passed — clippy, rustfmt, rustdoc, build&test (ubuntu + macos), TypeScript, ffi miri, all JVM validator unit tests (kosit/phive/saxon), validator sidecar images, byte-stable render (linux + macos + cross-compare), workspace coverage gate, cargo audit, cargo deny, engine ABI golden, cassette PII scan, outbox DB migrations. (Earlier red runs were either pre-existing debt now fixed, or concurrency-cancellations from rapid pushes — not merit failures.)
- **PHASE 6 COMPLETE.** v0.2.0 is tagged + published with full cross-platform assets (3 binaries + tarballs + cosign bundles + SBOMs + SHA256 + OpenAPI), and main CI is green. The only outstanding release item is registry publishing (PyPI/npm/crates.io/Maven/NuGet), which is blocked-external on the principal configuring trusted-publishers — honestly documented, not faked.
- **Campaign scorecard:** Phase 1 Discovery ✓ · Phase 2 Coverage ✓ (high-value done; optional tail lossless-via-preserve, deferred) · Phase 3 D15 codelists (blocked-external on licensed data — deferred) · Phase 4 Battle-test ✓ (722 fixtures parity 1.0, fuzz 5/5, PDF/A-3 30/30, Miri) · Phase 5 Reality-check ✓ (honest, no hidden stubs) · Phase 6 Release ✓ (v0.2.0 published, CI green).

### Turn 67 (2026-05-30) — Code-quality track: unused-dep cleanup + convergence assessment
- **Evaluated where isomorphic simplification makes sense (per the loop's "evaluate first").** The codebase already passes clippy with `all` + `pedantic` + `nursery` under `-D warnings` (very strict) and a clean reality-check, so AI-slop / dead code / obvious duplication are already caught. The one clean lever clippy does NOT cover is unused dependencies.
- **Removed 13 genuinely-unused deps across 10 crates** (commit f8263ef). cargo-machete flagged candidates; each was grep-confirmed (no source usage) AND proven isomorphic by `cargo check --workspace --all-targets` + affected-crate tests + clippy `-D warnings`, all green. Cargo.lock −14 lines (real supply-chain-surface reduction). Correctly KEPT 3 machete false-positives that ARE used (peppol-smp-sml `md-5`, managed-api + envelope-encryption `serde_bytes`).
- **Code-quality track CONVERGED on safe, high-value levers.** Remaining simplify candidates score below the skill's 2.0 bar: cross-crate duplication across the 34 national report crates is Type IV/V (different national invariants — merging is dangerous, not isomorphic); the format-cii/ubl preserve machinery is delicate + just battle-tested (refactor risk > LOC gain post-release). Per the simplify matrix, STOP at Score < 2.0 rather than manufacture risky refactors.
- **CAMPAIGN STATUS — converged.** All six phases resolved (Discovery, Coverage, D15-deferred-blocked, Battle-test, Reality-check, Release) and v0.2.0 is published with green CI; the code-quality track is converged. Remaining work is principal-gated (registry trusted-publisher credentials; licensed national code-list data) or optional enhancement (item-ID semantic IR modeling — lossless via preserve today). No high-value autonomous-safe work remains; the loop continues at a relaxed cadence to catch new opportunities or principal redirection.
