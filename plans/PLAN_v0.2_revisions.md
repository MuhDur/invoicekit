# PLAN v0.2 — applied revisions from Codex review round 1

This document supersedes/amends sections of `PLAN.md` v0.1. Each revision below is annotated with the principal architect's confidence: ✅ wholeheartedly agree, 🟡 partially agree, ❌ disagree.

---

## ✅ Revision 1 — Dual delivery artifacts (native + WASM), not "WASM-native everywhere"

**Original**: "WASM-native Rust core with native bindings via wasm-bindgen/pyo3/wazero/wasmtime-java/Wasmtime.NET."

**Codex correction**: WASM is the right delivery shape for browser, edge, and deterministic sandboxing — but for Python/JVM/.NET/Node/Go server-side, native FFI is simpler, faster, easier to debug, and friendlier to enterprise observability. The universal contract is the *engine API + canonical IR + rulepack semantics + fixture corpus*, NOT the WASM runtime.

**Applied — crate/binding layout becomes**:
```
crates/invoicekit-engine     # Pure deterministic Rust API. Source of truth.
crates/invoicekit-ffi        # Stable C ABI.
crates/invoicekit-wasm       # Browser/edge WASM artifact.
bindings/node-napi           # napi-rs (native), preferred over wasm-bindgen for server Node
bindings/python              # pyo3 + maturin
bindings/dotnet              # P/Invoke over C ABI
bindings/java                # JNI / Java FFM over C ABI
bindings/go                  # cgo + pure-Go REST/sidecar fallback
bindings/wasm-browser        # wasm-bindgen for browser/CF Workers
bindings/rest-shim           # Axum service for conservative customers
```

**Impact on positioning**: the wedge becomes more honest. "WASM where it matters (browser/edge), native where it doesn't (your server runtime), same engine."

## ✅ Revision 2 — Layered invoice model; EN 16931 is a ProfileView, not the root

**Original**: "EN 16931–rooted semantic IR with first-class country/profile extensions."

**Codex correction**: EN 16931 bends badly under CFDI (MX), ZATCA (SA), MyInvois (MY), NFe (BR), PINT (UAE) — they have fundamentally different commercial semantics. Forcing EN 16931 as global root means every non-EU customer hits escape-hatch hell.

**Applied — replace IR with layered model**:
- `CommercialDocument` — global commercial invoice/credit-note semantics, jurisdiction-agnostic.
- `ProfileView` — EN 16931, Peppol BIS, Peppol PINT, XRechnung, Factur-X profiles, KSeF, FatturaPA, ZATCA, IRP, CFDI, MyInvois, etc.
- `JurisdictionExtension` — typed, namespaced, versioned profile-specific data.
- `LossinessLedger` — mandatory output of every profile projection.
- `Rulepack` — validates a `CommercialDocument` against a `ProfileView` and effective date.

**Y1 focus**: EN 16931 + Peppol BIS as the primary Y1 ProfileViews. The deeper layering is the architectural commitment that lets Y2+ extend without rewrites.

## ✅ Revision 3 — Money/tax/codelists as first-class crates

**Original**: implicit decimal handling; codelists assumed.

**Codex correction**: highest-correctness-risk area is monetary math, VAT rounding, codelists. Bug class: floats anywhere; stale codelists; reverse-charge edge cases; cross-currency minor units.

**Applied — new Y1 Q1 crates**:
- `crates/money` — `rust_decimal::Decimal` internally; fixed-scale decimal strings at API boundaries; never floats.
- `crates/codelists` — ISO 3166, ISO 4217, UN/ECE codes, VAT category codes, Peppol codelists. Versioned + effective-dated + signed.
- `crates/tax-calculation` — invoice arithmetic, rounding policies (per jurisdiction), tolerance reports.
- `data/codelists/*.toml` — versioned signed lists with effective dates.

**Validation diagnostics requirement**: every monetary failure carries the exact arithmetic expression and source fields used (e.g. `"BR-CO-10: line_extension_amount(=Σ lines) − allowances + charges = 1234.56; actual 1234.55 (delta 0.01)"`).

CLI: `invoicekit explain BR-CO-10` shows formula, inputs, source locations.

## ✅ Revision 4 — Reference validator worker (JVM service), NOT wasmtime-java

**Original**: "wasmtime-java integration: load KoSIT validator JAR + Peppol Schematron"

**Codex correction**: this misreads `wasmtime-java`. That library lets *Java host WebAssembly*, not the inverse. Year 1 reference validation should be an isolated JVM worker service exposed over JSON-RPC / Unix socket / gRPC.

**Applied**:
- `services/validator-worker-jvm` — Containerized JVM service running KoSIT, phive, Saxon, Peppol Schematron.
- Rust engine calls it over a stable JSON-RPC contract.
- Managed service + CI use the worker.
- Browser/edge validator is explicitly "lite" (pure-WASM rule subset) until parity proven.

**Task replacement**:
```diff
- T-021: wasmtime-java integration — 2 weeks
+ T-021: Reference validator worker (KoSIT/phive/Saxon JVM service with JSON-RPC) — 2 weeks
+ T-021a: Browser/edge validator capability matrix (pure-WASM vs requires-worker) — 3 days
```

This is a meaningful simplification: a normal JVM container with a clean RPC contract is **boring infrastructure**, whereas embedding Java in WASM was speculative complexity.

## ✅ Revision 5 — Rulepack supply chain (signed, versioned, effective-dated)

**Original**: "validation rulepack registry with versioning + date pinning"

**Codex extension**: rulepack provenance must be from day 1. Every rulepack is a signed, versioned artifact carrying:
- source URLs + retrieval timestamps
- upstream version identifiers
- effective date ranges
- codelist versions
- checksums of raw upstream artifacts
- generated Rust/JSON metadata
- parity fixtures + known gaps

**Applied — new Y1 Q1 tasks**:
- `T-018: Rulepack source registry + signed manifest format — 1 week`
- `T-019: Codelist updater with provenance checksums — 1 week`
- Updated `T-024` depends on T-018 + T-019.
- CI refuses unpinned rules.
- `invoicekit validate --date=YYYY-MM-DD` selects rulepacks by effective date, not installed-package version.

## ✅ Revision 6 — Reconciliation/state machine BEFORE gateway integrations

**Original sequence**: gateway integrations (T-060..T-070) then reconciliation (T-080..T-083).

**Codex correction**: this lets each gateway invent its own idempotency, retry, and state semantics. Always bottoms-up: state model first.

**Applied — new sequence**:
1. `T-058: Gateway adapter trait + normalized gateway error taxonomy — 1 week [T-022]`
2. `T-059: Outbox SQL schema + idempotency model + retry policy + dead-letter states — 2 weeks [T-017, T-058]`
3. `T-059a: Transmission worker (backoff, rate limits, circuit breakers, structured gateway logs) — 2 weeks [T-059]`
4. `T-060: Peppol partner-AP adapter for Y1 live delivery — 2 weeks [T-034, T-059a]`
5. `T-080: State machine implementation (per-country sub-states) — 2 weeks [T-058]`

Mock gateway (`T-071`) becomes the first concrete `GatewayAdapter` trait impl, ensuring the contract is real before any live gateway code.

## ✅ Revision 7 — Native AS4 is research/conformance, NOT Y1 production

**Original**: "Native Rust AS4 sender (node42 proves this is ~500 LOC; we'll do it cleaner) — Y1Q2."

**Codex correction**: AS4 sender/receiver certification is not a 500-LOC weekend project. WS-Security, ebMS3, signing, canonicalization, SMP/SML, certificates, retries, receipts, OpenPeppol conformance testing — these are the real work.

**Applied — Y1 live Peppol delivery uses**:
- Partner AP API (e.g. Storecove, ecosio, B2BRouter) for actual envelope delivery
- `phase4`-backed reference adapter behind the same `GatewayAdapter` trait
- SMP/SML lookup as our own (boring) work
- Conformance harness exercising both paths

Native AS4 becomes a long-running research track. It cannot be marketed as production until OpenPeppol conformance, certificate handling, WS-Security, SMP/SML, receipts, and replay behavior all pass — likely Y2-Y3, not Y1.

## ✅ Revision 8 — Deterministic signed bundles (no `replay.sh`)

**Original**: `tar.gz` with a `replay.sh` shell script.

**Codex correction**: `tar.gz` metadata leaks nondeterminism (mtimes, gzip metadata); `replay.sh` is platform-specific and dangerous to execute.

**Applied — new bundle spec**:
- Canonical form: directory tree + signed `manifest.json` (BLAKE3 hashes; DSSE/JWS signature over manifest).
- Portable packed form: `.ikb` = `tar.zst` with normalized uid/gid/mtime/ordering.
- `replay.json` (declarative recipe consumed by `invoicekit verify`), never `replay.sh`.
- Verification **never** executes shell scripts.
- Separate `encrypted legal bundle` from `redacted support bundle`.
- No source bytes in public/support artifacts by default; opt-in inclusion only.

## ✅ Revision 9 — Use veraPDF as oracle, don't reimplement

**Original**: "PDF/A-3 conformance verification (veraPDF-equivalent in Rust) — 3 weeks"

**Codex correction**: veraPDF is an expensive side quest. Use veraPDF as the reference verifier in CI / managed service. Keep our Rust checks limited to structural invariants we own.

**Applied**:
- `T-042: PDF/A-3 verification adapter using veraPDF as reference oracle, with lightweight Rust preflight only — 1 week`
- `T-042a: Renderer fallback decision gate — prove Typst can produce required Factur-X PDF/A-3 + embedded XML; otherwise ship a secondary renderer behind the same RenderBackend trait — 1 week`

## ✅ Revision 10 — Y1 country scope: maturity matrix, not blanket "8 countries"

**Original**: "Cover DE, FR, IT, PL, BE, ES, SA, IN with full inbound+outbound support."

**Codex correction**: "support" has too many meanings (syntax, validation, legal submission, receipt, archive, correction, sandbox, production cert, SLA). Promising 8 countries at "full support" depth is dangerous.

**Applied — explicit maturity matrix per country × capability**:

| Country | Serialize | Validate | Render | Sandbox | Partner-live | Native-live | Inbound | Archive | Correction | SLA |
|---------|-----------|----------|--------|---------|--------------|-------------|---------|---------|------------|-----|
| DE | GA Y1Q2 | GA Y1Q2 | GA Y1Q2 | GA Y1Q2 | GA Y1Q3 | Beta Y2 | GA Y1Q3 | GA Y1Q4 | GA Y1Q3 | GA Y2 |
| FR | GA Y1Q2 | GA Y1Q3 | GA Y1Q2 | GA Y1Q3 | GA Y1Q3 | Research Y2+ | GA Y1Q3 | GA Y1Q4 | GA Y1Q3 | GA Y2 |
| BE | GA Y1Q2 | GA Y1Q2 | GA Y1Q2 | GA Y1Q2 | GA Y1Q3 | Beta Y2 | GA Y1Q3 | GA Y1Q4 | GA Y1Q3 | GA Y2 |
| PL | GA Y1Q3 | GA Y1Q3 | GA Y1Q3 | GA Y1Q3 | GA Y1Q4 | Research Y2+ | GA Y1Q4 | GA Y1Q4 | Beta Y1Q4 | GA Y2 |
| IT | GA Y1Q3 | GA Y1Q3 | GA Y1Q3 | GA Y1Q3 | GA Y1Q4 | Research Y2+ | GA Y1Q4 | GA Y1Q4 | GA Y1Q4 | GA Y2 |
| ES | GA Y1Q4 | Beta Y1Q4 | GA Y1Q4 | Beta Y1Q4 | Beta Y2Q1 | Research Y2+ | Beta Y2Q1 | Beta Y2Q1 | Research Y2+ | GA Y2 |
| Peppol BIS/PINT base | GA Y1Q2 | GA Y1Q3 | GA Y1Q2 | GA Y1Q3 | GA Y1Q3 | Research Y2+ | GA Y1Q3 | GA Y1Q4 | GA Y1Q3 | GA Y2 |
| SA, IN, MY, GR, KE, UAE | Research / partner-led — gated on signed design-partner LOI |

The "8 countries" claim is replaced by **"6 GA-quality Y1 jurisdictions plus partner-led tracks."** Honesty wins trust.

## ✅ Revision 11 — Separate `report-*` crates from `transmit-*`

**Original**: every country in `transmit-*` family.

**Codex correction**: France PA/PDP, Spain Verifactu, Greece myDATA, India GST IRP, Italy SDI, Poland KSeF all involve *reporting + clearance + tax-authority state*, not just exchange. Modeling these as "transmit serializers" is architecturally wrong.

**Applied — split `report-*` from `transmit-*`**:
```
crates/transmit-peppol/      # AS4 envelope exchange
crates/transmit-mock/        # Sandbox

crates/report-fr-ctc/        # France PA/PDP e-invoicing + e-reporting flows
crates/report-es-verifactu/  # Spain anti-fraud invoice-system reporting
crates/report-gr-mydata/     # Greece myDATA reporting
crates/report-in-gst/        # India GST IRP / e-waybill / reporting adapters
crates/report-pl-ksef/       # Poland KSeF clearance/submission state machine
crates/report-it-sdi/        # Italy SDI clearance + receipts
crates/report-sa-zatca/      # Saudi ZATCA Phase 2 clearance + cryptographic stamping
```

These are state machines, not serializers.

## ✅ Revision 12 — Managed platform security FROM DAY 1

**Original**: managed compliance API tasks at the end (T-130..T-135).

**Codex correction**: tenant isolation, KMS, audit logs, GDPR/data residency, scoped API keys, usage metering, retention — these shape IDs, schemas, storage, logs, evidence bundles, and support tooling. Cannot be retrofitted.

**Applied — new Y1 Q1 tasks**:
- `T-128: Tenant model + scoped API keys + OIDC + RBAC + audit-event schema — 2 weeks [T-001]`
- `T-129: Envelope encryption with KMS-per-tenant + key rotation + data residency tags — 2 weeks [T-128]`
- `T-129a: Webhook signing + replay protection + event-delivery idempotency — 1 week [T-128]`
- `T-129b: SBOM + dependency scanning + signed releases + security advisory process — 1 week [T-002]`

All subsequent tasks have `tenant_id` as a structural assumption.

## ✅ Revision 13 — Observability/SLO engineering before SLA claims

**Original**: "SLA 99.9%" with a status-page task in Q4.

**Codex correction**: SLA 99.9% cannot depend on a status page. Per-gateway latency/error metrics, trace IDs, dead-letter queues, replay tooling, customer-visible incident IDs, reconciliation-first support — all need to exist *before* you make an SLA claim.

**Applied — new section 4.12**:

Every transmission has: `trace_id`, `tenant_id`, `idempotency_key`, `gateway_attempt_id`, normalized state transition, raw gateway receipt hash, retry/dead-letter metadata.

Managed API SLOs are defined per operation: `validate`, `render`, `transmit-enqueue`, `gateway-accepted`, `archive-write`, `webhook-deliver`. **Gateway legal acceptance is never conflated with API availability.**

New tasks:
- `T-134a: OpenTelemetry tracing/metrics + log redaction + per-gateway dashboards — 2 weeks [T-059a]`
- `T-134b: Replay/admin tooling for stuck transmissions and dead-letter queues — 1 week [T-134a]`

## ✅ Revision 14 — Pure builder + explicit enrichment

**Original**: `Invoice.builder({ supplier: { vat: 'DE123' } })  // Rest auto-resolved via VIES`

**Codex correction**: builders should be deterministic and offline. Network enrichment should be explicit, cancellable, cached, and privacy-aware. Otherwise `.build()` becomes slow, flaky, and surprising.

**Applied — new API shape**:
```typescript
import { createInvoiceDraft, validateLocal } from '@invoicekit/core';
import { renderPdf } from '@invoicekit/render';
import { InvoiceKitClient } from '@invoicekit/managed';

const draft = createInvoiceDraft({
  supplier: { vat: 'DE123456789' },
  customer: { vat: 'FR987654321' },
  currency: 'EUR',
});

// Enrichment is explicit
const enriched = await client.enrich(draft, {
  sources: ['vies', 'gleif', 'cache'],
  cache: 'tenant',
  consent: true,
});

const invoice = Invoice.builder(enriched).line(...).build();
```

Package split: `@invoicekit/core` (pure), `@invoicekit/render`, `@invoicekit/managed`. Each has its own surface; tree-shakable.

## 🟡 Revision 15 — DX consistency: bunx-first but multi-runtime documented

**Original**: mixed `npx` and `pnpm exec`.

**Codex correction**: pick one default. AGENTS.md mandates Bun for our own dev; therefore internal/onboarding default to `bunx`. Public docs list `npx` / `pnpm exec` / `yarn dlx` equivalents as well.

**Applied**:
```
$ bunx invoicekit init
✓ Detected: Bun + TypeScript + ESM
...
Try it:   bunx invoicekit validate examples/first-invoice.ts --profile=peppol-bis
Then:     bunx invoicekit send examples/first-invoice.ts --mode=sandbox
```

`invoicekit doctor` runs **before** `init` does anything expensive:
```
invoicekit doctor --country=DE --profile=xrechnung
# Checks: local engine, reference validator availability, rulepack freshness,
# PDF/A verifier, API key scopes, country capability matrix.
```

## ✅ Revision 16 — Public validator: dual mode (local + server-assisted reference)

**Original**: "WASM-only browser; no upload"

**Codex correction**: browser-only validator can't run the JVM reference stack. Claiming official-grade XRechnung/Peppol validation from a pure-WASM subset would hurt trust.

**Applied — two explicit modes**:
- **Local mode**: browser-only WASM, privacy-first, runs the pure-Rust rulepack subset; clearly labeled.
- **Reference mode**: server-side KoSIT/phive/Saxon worker call (no-retention default; optional client-side redaction); returns official-parity diagnostics with rulepack provenance.

UI surfaces which mode produced the result.

## 🟡 Revision 17 — Corpus licensing (synthetic vs licensed-real vs private)

**Original**: `conformance-corpus/   # 500+ adversarial test invoices; public CC-BY-SA`

**Codex correction**: real invoices have licensing/privacy traps; one blanket license is wrong.

**Applied**:
```
conformance-corpus/
  synthetic/             # CC0 / Apache-2.0 generated fixtures
  licensed-real/         # Explicitly licensed, redacted real invoices (with provenance metadata)
  private-regression/    # Non-public customer/support fixtures
  generators/            # Adversarial fixture generators (Apache-2.0)
```

New task: `T-120a: Corpus licensing/redaction policy + fixture metadata schema — 3 days [T-002]`.

## 🟡 Revision 18 — Per-package performance budgets

**Original**: "WASM cold-start p99 <50ms in Cloudflare Worker"

**Codex correction**: blanket numbers are meaningless when a single bundle could include PDF rendering, rulepacks, and OCR. Split budgets.

**Applied**:
- `@invoicekit/core` WASM cold start p99 <50ms on Cloudflare Workers (excludes PDF/render/OCR)
- Local EN 16931 core validation p95 <25ms for 100-line invoice
- UBL parse + canonicalize p95 <50ms for 1 MB XML
- PDF/A-3 render p95 <2s for 10-page invoice on managed worker
- Reference validator p95 measured separately by profile/rulepack
- OCR/VLM excluded from core-runtime budgets

## ✅ Revision 19 — Source-watch automation as product capability

**Codex addition**: regulatory sources update constantly. Make tracking them a product capability, not a manual cron.

**Applied — new Y1 Q1 tasks**:
- `T-006: Compliance source-watch bot — monitor official sources (KoSIT, Agenzia delle Entrate, KSeF gov.pl, ZATCA, etc.), produce rulepack/codelist update PRs, open beads for changed mandates — 1 week [T-001]`
- `T-006a: invoicekit capabilities data model generated from source-watch manifests, with confidence level and last-verified date — 1 week [T-006]`

This becomes part of the "CI for compliance" runaway-success scenario.

## ✅ Revision 20 — Stable engine ABI as binding-foundation

**Original**: bindings depended on T-110 (TypeScript SDK).

**Codex correction**: SDKs must depend on the stable engine ABI + golden fixtures, not on TS. Otherwise TS becomes an accidental source of truth.

**Applied**:
```diff
+ T-109: Stable engine ABI contract + cross-language golden fixtures — 2 weeks [T-010, T-022]
- T-110: TS SDK [T-012]
+ T-110: TS SDK (@invoicekit/core, @invoicekit/render, @invoicekit/managed) — 2 weeks [T-109]
+ T-111: Python SDK (pyo3/maturin) — 2 weeks [T-109]
+ T-112: Go SDK (cgo + REST sidecar fallback) — 2 weeks [T-109]
+ T-113: Java SDK (JNI/FFM over C ABI + REST sidecar fallback) — 2 weeks [T-109]
+ T-114: .NET SDK (P/Invoke over C ABI + REST sidecar fallback) — 2 weeks [T-109]
```

---

## Net effect on the plan

- **Architectural integrity**: layered IR + dual artifacts + stable engine ABI is genuinely better.
- **Risk surface reduced**: AS4 demoted to research track; reference validator as boring JVM service; veraPDF as oracle.
- **Honesty improved**: country maturity matrix replaces "8 countries supported"; report-vs-transmit separation.
- **Operational realism**: security + observability + outbox/state-first sequencing — these were latent risks v0.1 didn't address.

**Estimated impact on Year 1 timeline**: +4 to +6 weeks for the architectural depth, but **reduces risk of needing a Y2 rewrite by ~50%**. The trade is correct.

## Items the principal architect (me) somewhat disagrees with

- **Rev 15 (DX defaults)**: bunx is fine for *our* dev; public docs should lead with `npx` because Node-with-npm has 10× the install base. Will use `bunx` internally, `npx` in the public quickstart.
- **Rev 17 (corpus licensing)**: agreed structure, but CC-BY-SA on `synthetic/` is fine too — the codex push toward CC0/Apache is more permissive than needed.
- **Rev 18 (perf budgets)**: agreed to split, but keep the 50ms claim on `@invoicekit/core` as the marketable target.

## Items I do NOT disagree with

(None.)

---

## Next revision rounds

The planning-workflow skill recommends 4-5 review rounds. This is round 1. Subsequent rounds (deferred — outside this session's goal scope):

- **Round 2**: paste v0.2 into Gemini for a complementary critique.
- **Round 3**: re-paste merged v0.2+gemini into Codex.
- **Round 4**: ideally GPT Pro Extended Reasoning in web app.
- **Round 5**: principal architect final pass.

Once steady-state, convert build sequence to beads via `br create ... --deps` (see PLAN.md § 8 — task IDs T-001..T-137 + new T-006/006a/018/019/058/059/059a/109/120a/128/129/129a/129b/134a/134b).
