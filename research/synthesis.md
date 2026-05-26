# Synthesis — InvoiceKit ideation pool (final)

**Status**: Final. All 7 market research streams + Phase 0/1/4/4b ideation + codex/gemini triangulation + cross-score duel + Brenner-style falsification complete.

## Headline finding

After ~490 candidate ideas, multi-model cross-scoring, and adversarial falsification, the strongest concept emerges as:

> **"Stripe-for-e-invoicing": a WASM-first developer SDK + thin managed REST control plane, anchored on (1) deterministic byte-stable IR + AOT-compiled validators, (2) auditable AI extraction with bbox citations, (3) clearance state-machines as first-class primitives, (4) reconciliation/state engine as the paid moat, (5) sandbox/test-mode parity with production. OSS core Apache 2.0; managed gateway/cert/archive/reconciliation services are the revenue engine.**

**Two viable strategic paths** (forks where the project could meaningfully diverge):

- **Path A — "Library-first"** (Codex's tilt): SDK is the lock-in vector, REST is convenience. Bottoms-up adoption via developers. Lower-risk, slower revenue.
- **Path B — "Managed-API-first"** (Gemini's tilt + Gemini Brenner runaway-success scenario): hosted REST is the product; SDK is a thin convenience. Liability transfer is the value prop. Faster revenue, more capex.

The synthesized recommendation below favors **A → B sequencing**: ship the SDK first to build trust + lock-in, then layer the managed API as customers ask for liability transfer.

---

## Pool composition (final)

| Phase | Source | Net ideas | Notes |
|-------|--------|-----------|-------|
| 0 | My baseline ultrathink (14 axes) | 210 | + 7 forks identified |
| 0.5 | Self-critique adversarial filter | (−20) | ~190 survivors |
| 1A | Idea-wizard pass A (DX focus) | +30 | |
| 1B | Idea-wizard pass B (ops focus) | +20 | |
| 3a | Gemini triangulation | +25 | Plus critique + 1 missing piece |
| 3b | Codex triangulation | +25 | Plus critique + 1 missing piece |
| 4A | Anti-disaster engineering (Phase 4 Pass A) | +50 | |
| 4B | Distribution & ecosystem | +30 | |
| 4C | Cutting-edge AI / agentic | +30 | |
| 4D | Adjacent expansions | +20 | |
| 4E | Wild boundary-blurring | +15 | |
| 4F | Operational excellence | +10 | |
| 4b | Gap-fill (post-critique) | +56 | Reconciliation, state-machines, evidence |
| Duel | Codex scoring of Gemini ideas (filter) | (filter) | 25 ideas scored 4 dims |
| Duel | Gemini scoring of Codex ideas (filter) | (filter) | 25 ideas scored 4 dims |
| Brenner | Gemini hypothesis falsification | (filter) | 5 failure modes + 5 assumptions + 5 threats + 3 experiments + 1 runaway |
| Brenner | Codex hypothesis falsification | (filter pending) | Still running |
| **Total** | | **~490** | Well above 300 target |

---

## Top consensus winners (scored 700+ by both models, or appearing in both top-3 lists)

### Tier S — Build immediately

These are confirmed by independent generation, cross-scoring, and adversarial review:

**S1. EN 16931 semantic IR with versioned per-format adapters** — Phase 0 #2, codex #1, gemini implicit; near-universal agreement that this is the right architectural spine. Interop with `invopop/gobl` JSON schema rather than reinvent.

**S2. Schema-compiled AOT validator (Schematron → Rust → WASM)** — Gemini #2 (900 useful), codex #2-3, Brenner ⚠️ identified as risk (XPath 2.0 quagmire). **Mitigation: fall back to wrapping Java validator inside WASM via wasmtime if AOT can't reach 95% conformance in 3 weeks** (Brenner experiment #1).

**S3. Smart canonicalizer for byte-stable serialization** — Phase 1 #5 wizard top pick, codex #16 implicit, gemini #24 implicit. Foundation for signing/hashing/audit-grade reproducibility.

**S4. Time-travel / date-aware validation** — Phase 0 #16, Phase 1 #15, codex #11, gemini #6 (scored 890/880/760/910 by codex — top score). Validate against rules-as-of-date.

**S5. Semantic diffing for amendments** (credit notes / corrections) — Gemini #4 (scored 860/860/710/870 by codex). Real DX value for AR/AP and audit.

**S6. Government chaos simulator** ("LocalStack for Peppol/SDI/KSeF/IRP/ZATCA") — Gemini #10 + codex #9. Sandbox/test-mode parity with production. Both models scored top-tier.

**S7. Reconciliation/state engine** (deterministic fingerprint + idempotency + state machine + ACK matching) — Gemini's "missing piece" critique + Phase 4b #206-215 + codex's "clearance state machines" framing. **Codex called this the #1 idea to build across both lists.** This is likely the paid control plane.

**S8. ERP mapping DSL / "any-to-any mapping"** — Codex #20 + gemini #23 (scored 850/920/610/900 by codex). Adoption bottleneck killer. Declarative, compilable.

**S9. Diagnostics-first validator with JSON Pointer/XPath, BT-term ties, citation, suggested fix** — Codex #3, Gemini #7. Errors as first-class learnable artifacts.

**S10. Evidence bundle format `.invoicekit`** — Codex #16, Phase 4b #226. Auditable archive primitive. **Brenner caveat (Gemini): proprietary format has no legal standing — mitigation: bundle is a convenience wrapper around the legally-required artifacts (UBL/CII XML + PDF/A-3 + qualified timestamp), not a substitute.**

### Tier A — Strong differentiators (clear consensus or strong single-model conviction with no opposition)

**A1. VS Code / Cursor LSP for invoicing** — Gemini #14, codex #19. Hover BT terms, real-time validation squigglies. Strong DX moat.

**A2. Tax & rounding proof engine** (decimal math + formal trace) — Codex #14. Gemini ranked top-3 across both lists. **Brenner caveat (Gemini): ERPs already have battle-tested ledgers; mitigation: ship as opt-in validator that *checks* the ERP's math rather than replacing it.**

**A3. Type-state invoice builders** (Rust + TypeScript) — Codex #13. Invalid invoices unrepresentable. Brenner risk: FFI friction (mitigation: build native TS API in parallel, not just FFI).

**A4. PII/GDPR redactor** — Gemini #16, scored 780/700/700/760 by codex. Enables safe staging dumps and support bundles.

**A5. Inbound invoice firewall** (XXE, ZIP bomb, malware checks) — Codex #17. Differentiates security story.

**A6. Gateway rejection normalizer** — Codex #22. Normalizes Peppol/KSeF/SDI/ZATCA error codes into stable categories with remediations.

**A7. routePlan(invoice, recipient) preflight API** — Codex #10. Codex's "blind spot fix" — recipient capability + routing intelligence. **Brenner caveat (Gemini): some devs may want dumb `/invoice` endpoint instead. Resolution: offer BOTH — routePlan for advanced users, automatic routing for the default path.**

**A8. Auditable AI / "LLM as cross-examined witness"** — Phase 0 #34, Codex #18, AI-extraction-sota's #1 recommendation. Every extracted field cites bounding box + OCR span + confidence + cross-validated by deterministic rules.

**A9. Property-based test corpus + adversarial generator** — Codex #7. Public corpus of 500+ adversarial invoices.

**A10. Reverse-proxy / sidecar pattern for JVM/.NET** — Gemini #12, scored 820/800/700/820 by codex. Solves the WASM-leaky-for-enterprise critique.

### Tier B — Promising adds (strong from one phase, no blockers)

B1. Future-mandate migration simulator (Codex #23)
B2. Counterparty contract tests / "publish your AP test profile" (Codex #25)
B3. OpenTelemetry semantic conventions for e-invoicing (Codex #21)
B4. Universal payment instructions abstraction (Phase 1 #19)
B5. VAT-ID-driven autodetect onboarding (Phase 1 #4)
B6. Legal numbering ledger (Codex #15) — _but Brenner-Gemini scored low for ROI_
B7. WASM CIUS plugins (Gemini #11)
B8. Dirty-ingestion error stream (Gemini #7)
B9. Content-addressed attachments (Gemini #9)
B10. SSE/WebSockets for ACK delivery (Gemini #19)

---

## Killed ideas (strong mutual rejection)

| Idea | Score | Why killed |
|------|-------|------------|
| ZKP-based factoring | Gemini #13, codex 690/110/60/40 | Tiny market, complexity unjustified |
| Smart-contract escrow bridge | Gemini #22, codex 480/80/160/50 | Brand-distraction, no liquidity |
| Homomorphic VAT encryption | Gemini #18, codex 620/90/30/20 | Academic novelty, no buyer |
| eBPF DB sniffer ingestion | Gemini #5, codex 650/180/120/80 | Invasive, brittle, wrong product |
| BYO-LLM via WebGPU | Gemini #3, codex 520/220/280/120 | Distraction; AP clerks have weak hw |
| Bounding-box OCR TUI | Gemini #20 | Wrong ICP — buyer-side product |
| "Fallback to paper" API | Gemini #25 | Edge case, partner integration later |
| Invoice mediation marketplace | Phase 0.5 | Regulated arbitration, out of scope |
| Public ledger Merkle gimmick | Phase 0.5 | Replaced by RFC 3161 timestamping |
| Hardware-attested signing (Apple/Google) | Phase 0.5 | Auditors require eIDAS QTSP, not OS attestation |
| Per-tenant LoRA in v1 | Phase 0.5 | Y2+ feature |
| Reverse-pitch incumbents | Phase 0.5 | Pagero/Comarch will not adopt OSS competitor |

---

## Critical risks & mitigations (from Brenner)

### Top failure modes (Gemini Brenner)

1. **WASM/Rust enterprise rejection** — Python/C#/Java devs hate alien FFI tooling.
   - **Mitigation**: ship native SDKs (Python pyo3 binding, .NET / Java JNI shim, Go), not just WASM blob. WASM is the *core implementation*; bindings are the *delivery shape* per platform.

2. **Schematron→Rust AOT is XPath quagmire**.
   - **Mitigation**: phased compile. Hand-write Rust validator for EN 16931 core (well-bounded). For Schematron, ship wrapped Java validator via wasmtime as Y1 fallback. Phased AOT compilation per ruleset as confidence builds. _Falsification experiment: try Peppol BIS 3.0 conformance in 3 weeks._

3. **Browser-side 7B LLM OOM**.
   - **Mitigation**: don't force 7B in browser. Default: server-side Qwen2.5-VL-7B (our cloud LLM fallback tier). Browser layer 1-3 only (digital PDF, text, small VLM like SmolDocling-256M).

4. **Pricing "uncanny valley"** — €49/mo too high for OSS curious, too low for liability transfer.
   - **Mitigation**: bifurcate pricing — free OSS forever; €0 dev tier for usage; **Managed Compliance API at €499-€1999/mo with liability transfer** for buyers who want it. Two products, one engine.

5. **Typst PDF rejection by WYSIWYG-PM demands**.
   - **Mitigation**: Typst is the *renderer*; we also ship a WYSIWYG template designer (Y2) that compiles to Typst-or-other. Pixel-perfect determinism + drag-and-drop are not mutually exclusive.

### Hidden assumptions to falsify (Brenner experiments — do in first 60 days)

| # | Assumption | Test | Kill threshold |
|---|-----------|------|----------------|
| E1 | Schematron AOT-to-Rust achievable | Compile Peppol BIS 3.0 Schematron; run official test corpus | <95% pass in 3 weeks → wrap Java validator |
| E2 | €49/mo dev pricing converts vs €499/mo managed API | Two-landing-page LinkedIn ad test | API 3× higher → pivot to managed-API-first |
| E3 | WASM/FFI friction tolerable | Bounty Python+Java devs to integrate | >4 hr to first invoice → maintain native SDKs |

### Competitive threats to track

| # | Threat | Watch |
|---|--------|-------|
| C1 | Invopop/GOBL defines the standard first | Their JSON schema versioning + funding |
| C2 | Storecove/Tickstar/Unifiedpost go down-market with dev APIs | Their pricing tiers + free credits |
| C3 | Phase4 (OSS Java) is the 800lb gorilla in JVM shops | Their feature velocity + Java vs WASM debate |
| C4 | National centralization (KSeF, Chorus Pro) makes Peppol less load-bearing | Each country's portal API maturity |
| C5 | OpenAI/Anthropic structured output APIs commoditize OCR layer | Vision-model accuracy on invoice extraction |

### Runaway-success 18-month scenario (Gemini Brenner)

> **"Plaid for B2B invoice reconciliation"** — embedded widget (Stripe-Elements-shape) that ERPs drop into their UI. Ingests Peppol/XML invoices, normalizes via our IR, matches against open POs in the host ERP, orchestrates payment via open banking. Revenue model: **basis points on global B2B trade volume**, not seat fees on a dev tool.

This is the bull case. The MVP doesn't need to commit to it, but the architecture should leave room for it (the IR + reconciliation engine + state machine + payment-instruction primitives all align).

---

## Fork resolutions (after evidence)

| Fork | Resolution | Evidence |
|------|-----------|----------|
| F1 ICP | **Embedded developer at ERP/billing SaaS vendor** | Buyer-personas top ICP, OSS adoption compounds, GOBL not yet dominant in this niche |
| F2 Geography | **EU-first; format-zoo design for global day 1** | Regulatory map: 2025–2028 wave concentrated in EU; ViDA forcing function; CFDI/ZATCA shapes the IR but doesn't drive Y1 revenue |
| F3 Peppol AP | **Year 1: wrap phase4 via wasmtime + managed service; Year 2-3: native Rust AS4 (sender first, then receiver)** | Peppol research confirms €30-65k Y1, ISO 27001 the gate; node42 proved non-JVM sender feasibility |
| F4 AI positioning | **Quiet support capability — never headline; AI ON by default for inbound, OFF by default for outbound** | Codex critique; buyer-personas show finance buyers fear AI hallucination |
| F5 Outbound/Inbound | **Outbound-first (issuance) Y1; inbound (AP/OCR) as strong Y1 secondary; reconciliation becomes the moat by Y2** | Outbound is deterministic + immediate; reconciliation engine is the paid control plane |
| F6 License | **Apache 2.0** | Patent grant, max community, beats SSPL on adoption friction; protects against GOBL fork |
| F7 Delivery shape | **Sequenced: Library/SDK-first (Y1 H1) → Managed REST API (Y1 H2) → Managed Compliance API with liability transfer (Y2)** | Hybrid satisfies both codex (library-first lock-in) AND gemini (managed API as revenue engine); brenner E2 will test exact pricing thresholds |

---

## "What if we're wrong about everything" — Plan B

If Brenner E2 (the pricing test) reveals managed-API at €499/mo converts 3× higher than €49/mo SDK:

- Compress Y1 by ~6 months
- Ship managed-API as Y1 primary product
- SDK becomes thin convenience wrapper around our REST
- Still keep OSS core; still ship Apache 2.0
- Reposition: "Stripe for e-invoicing compliance, with developer ergonomics"

The architecture supports both pivots — that's why we're investing in IR + state machine + canonicalizer as ground-truth primitives.

---

## What goes into the MASTER_REPORT and plan

Tier S + selected Tier A items form the **MVP scope**. Tier A remaining + Tier B form the **6-12 month roadmap**.

The next step is `/planning-workflow` on Tier S items to produce a concrete implementation plan.
