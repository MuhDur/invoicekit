# InvoiceKit — Master Report
## Market analysis · Idea pool synthesis · Fork decisions · Recommended plan

**Date**: May 26, 2026
**Project**: `invoices` (working name: InvoiceKit)
**Scope**: Inception-stage strategic + technical plan for an open-source, partially-monetized e-invoicing toolkit.

---

## 0. TL;DR — read this first

We've completed exhaustive market research (7 parallel research streams), generated ~490 candidate product/architecture/GTM ideas across 15+ axes, cross-scored them with two independent foundation-model panels (Codex and Gemini), and adversarially falsified our biggest assumptions via two-model Brenner-style critique. This document is the synthesized output.

**The single highest-leverage product concept**:

> **InvoiceKit — a developer-first, WASM-native, multi-runtime toolkit for the entire B2B invoicing lifecycle (intake → normalize → validate → render → transmit → reconcile → archive), built on an EN 16931 IR with first-class profile extensions and a paid managed compliance layer (Peppol AP + national gateways + certificates + vault archive + liability transfer).**

The wedge — the *exact phrasing* a developer would react positively to:

> "The only invoicing toolkit that generates and validates EN 16931 (XRechnung + Factur-X + Peppol BIS) on Bun, Deno, Cloudflare Workers, JVM, .NET, Python, Go, and the browser — and produces court-grade evidence bundles for every operation."

**Two viable strategic paths** with combinable architectures but divergent emphasis:

- **Path A** (Codex's runaway-success bet): **"CI for invoice compliance"** — open conformance corpus, differential test harness, public validator becomes the trust default; monetize hosted conformance + private corpora + release-diff alerts + certified gateway adapters.
- **Path B** (Gemini's runaway-success bet): **"Plaid for B2B reconciliation"** — embedded widget that ERPs drop into their UI for inbound Peppol invoice normalization + PO matching + open-banking-orchestrated payments; monetize basis points on global B2B trade volume.

**Recommended**: build the architecture so **both paths are reachable** from the same Y1 foundation. Path A is the Year-1/Year-2 trust-building product. Path B is the Year-3+ revenue scale-up. The shared substrate (IR + state machine + evidence bundle + reconciliation primitives) supports both.

**Three falsification experiments in the first 60 days** kill the project's biggest hidden assumptions cheaply:

1. **Schematron AOT parity gauntlet** (3 weeks, ~€3k). If we can't compile Peppol BIS Schematron to Rust with ≥99.9% parity vs KoSIT/Saxon, we fall back to wrapping the JVM validator in WASM via wasmtime-java.
2. **Pricing landing-page A/B** (2 days, €500 ad spend). If the "Managed API with liability transfer at €499/mo" landing page converts 3× higher than "OSS toolkit + €49/mo metered," we compress to managed-API-first.
3. **WASM/FFI friction audit** (1 week, ~€500 bounty). Give a mid-level Python + Java dev the WASM build and watch how long to first invoice. If > 4 hours, we commit to maintaining native non-WASM SDKs.

---

## 1. The opportunity

### 1.1 Regulatory tailwind (concentrated 2025–2028)

E-invoicing is regulated. Every ERP and billing SaaS now needs country-correct support across an expanding jurisdiction set. As of May 2026:

| Jurisdiction | Status | Key dates | Format |
|--------------|--------|-----------|--------|
| Germany | Active receive; phased send | Jan 2025 receive, Jan 2027 (>€800k), Jan 2028 all | XRechnung 3.x / ZUGFeRD |
| France | Imminent | **Sep 1, 2026** broad receipt obligation | Factur-X via PDP/PPF |
| Italy | Mature | SDI live; ViDA alignment ongoing | FatturaPA 1.2.2 |
| Poland | Live | KSeF 2.0 live; mandatory thresholds rolling 2026 | FA(3) |
| Belgium | Live | Jan 1, 2026 B2B Peppol mandate | Peppol BIS via AP |
| Spain | Coming | VeriFactu Jan/Jul 2027 + Crea y Crece B2B 2027 | FacturaE / VeriFactu |
| Saudi Arabia | Active waves | ZATCA waves 23–24 (Mar/Jun 2026) | XML+QR with cryptographic stamp |
| India | Mature | ₹5cr threshold, 30-day rule ≥₹10cr | IRP / GST |
| UAE | Pilot then mandate | Jul/Oct 2026 pilot, Jan 2027 mandate (>AED 50m) | Peppol PINT-AE |
| Greece | Coming | myDATA Mar/Oct 2026 (B2B) | myDATA XML |
| Malaysia | Phase 4 | **Jan 1, 2026** (RM 1–5m) + Dec 2027 relaxation cliff | MyInvois UBL/JSON |
| Kenya | Live | "No Invoice, No Deduction" Jan 2026 | eTIMS |
| EU bloc | Future | ViDA cross-border digital reporting Jul 1, 2030 | EN 16931–aligned |

**The 2025–2028 wave** is the buying window. **Most underestimated**: Malaysia (Phase 4 + 2027 cliff hits tens of thousands of SMEs simultaneously) and Kenya (a tax change that converts e-invoicing from "nice to have" into a balance-sheet event).

### 1.2 The pricing arbitrage

Incumbent vendor pricing is opaque, expensive, and rife with surcharges. From the [competitive pricing research](competitive-pricing.md):

- **Sovos**: €15k–50k/yr per jurisdiction. Customers report bills tripling at renewal when "estimates" silently count as transactions.
- **Pagero / Comarch / SAP DRC**: bundles stack multiple SKUs to send one envelope; per-legal-entity multipliers; €5k–50k+ implementations.
- **Tradeshift**: sellers forced into paid tier at >30 invoices/quarter; €500/yr surcharge "with no choice."
- **AvidXchange**: suppliers pay 1.2% on ACH + virtual card fees.
- **Tipalti**: FX margin 1.9–3% per transaction.
- **Avalara**: customers report 200–300% repricing at tier breaks.
- **Stripe Invoicing**: no Peppol support; 1k events/sec ceiling; Stripe paid $1B for Metronome rather than fix it.

Floor for marginal cost of delivering a Peppol envelope: **~€0.01 at scale** (compute + bandwidth + cert amortization). Self-serve at **€0.05/envelope** undercuts incumbents 5–50× without race-to-bottom dynamics. The opportunity is to capture the underserved long-tail of midmarket ERP and SaaS who don't qualify for incumbent enterprise discounts and balk at €15k+/yr/country price floors.

### 1.3 Developer pain (validated from real threads)

From the [dev pain points research](dev-pain-points.md):

1. **"Schematron is the hidden second validation step that breaks every non-JVM stack."** (Stefan Meier, dev.to). Node, Deno, Bun, .NET all lack XSLT 2.0 and shell to Java or fail.
2. **"You're looking at a basic yearly fee of 2000 euros just for the Peppol membership"** — Peppol AP gatekeeping feels like a tax (HN 42777669).
3. **"Failed to parse PDF: java.lang.NullPointerException because getQuantity() is null"** — mustangproject regressed between 2.13 and 2.15. factur-x Python install fails on Debian 12 because PyPDF4 is abandoned. Libs silently rot.
4. **"Puppeteer cannot run on Cloudflare Workers because Chromium is 200MB"** — Forme. Even pdfkit fails on Supabase Edge: *"PermissionDenied: Deno.readFileSync is blocklisted."*
5. **"Stripe Billing maxes out at around 1,000 events per second... Progressive billing breaks because the whole system assumes monthly or annual cycles"** (Lago blog).

The wedge cuts all five at once.

### 1.4 OSS landscape (validated gaps)

From the [OSS landscape research](oss-landscape.md):

| Gap | Detail |
|-----|--------|
| **No non-Java Peppol AS4 client** | `phase4` (Helger, Java) and `oxalis-ng` dominate. Everyone else pays Storecove/Pagero. **Biggest commercial wedge**. |
| **No browser/WASM Schematron validator** | Saxon-JS exists but no one bundled the rule sets. |
| **No end-to-end OCR → EN16931 pipeline** | Docling, invoice2data extract; none output conformant validated UBL/CII. |
| **No browser/WASM strict PDF/A-3 builder** | pdf-lib attaches files but conformance is unreliable. |
| **No open French PPF/Chorus Pro SDK** | Sept 2026 mandate looming. Timing opportunity. |

**Conceptual competitor**: `invopop/gobl` (Apache-2.0, Go, ~277 stars, very active). Covers UBL/CII/FatturaPA/CFDI/KSeF/VeriFactu with JSON schema + JWS signing. **Decision: interop with GOBL's JSON schema; do NOT reinvent**. Our differentiation is (a) WASM/cross-runtime delivery, (b) OCR→EN16931 pipeline, (c) non-Java Peppol AS4, (d) browser-native PDF/A-3 with veraPDF-verified Factur-X embedding, (e) WASM Schematron validator.

### 1.5 AI tech inflection

From the [AI extraction SOTA research](ai-extraction-sota.md):

- **Qwen2.5-VL-7B** (Apache-2.0) is the workhorse open model — DocVQA 96.4, fits on a 16 GB GPU (8 GB at int4).
- **PaddleOCR PP-StructureV3** is the layout/OCR/table workhorse for the deterministic pipeline.
- **Transformers.js v4 + WebGPU** runs at 20–60 tok/s on consumer laptops; SmolDocling-256M, Florence-2-base feasible in browser.
- **Browser-side extraction realistic for Layers 1–2** (Factur-X parsing, digital PDFs, clean scans). Dense multi-page line-item tables on noisy scans still need server.

**Most expensive AI failure mode**: silent line-item hallucination. LLMs invent plausible quantities/prices when rows are occluded or split across pages, with confident self-reported scores. Evades automated validation; surfaces only in audit. **Mandatory mitigation**: every emitted value is grounded to OCR span / PDF object / bbox; never accept LLM self-confidence as triage signal.

### 1.6 Peppol AP economics (validated)

From the [Peppol network research](peppol-network.md):

- 6-12 months end-to-end to become an operational AP (single country, our size).
- Year-1 hard cost: **€30–65k** (OpenPeppol fees + ISO 27001 + insurance + certs).
- **ISO 27001 is the long pole** — 6-12mo even with consultancy, €15–40k initial.
- Pure Node.js AS4 sender exists in ~500 LOC (node42, March 2026); Rust/Go equally tractable.
- Receiver-side WS-Security validator: **no production-grade outside JVM as of May 2026**.
- Ongoing fixed cost: **~€140-155k/yr**. Break-even at €0.05/envelope is **~3M envelopes/yr**.

**Strategy**: wrap `phase4` (JVM) in Y1 via wasmtime; build native Rust AS4 sender Y1 and receiver Y2-3 with phase4 as conformance oracle. Apply for OpenPeppol membership + ISO 27001 Y1Q1 (it's the long pole).

### 1.7 Buyer personas (validated)

From the [buyer personas research](buyer-personas.md):

| Rank | Persona | TAM | ACV | Acquisition |
|------|---------|-----|-----|-------------|
| 1 | Embedded developer at ERP / billing SaaS vendor (Odoo, MS Dynamics ISVs, sevDesk, Pennylane, Lexware, Pleo) | ~500 EU targets | €15–150k | PLG, GitHub, MSFT 2026 connector framework |
| 2 | In-house engineer at midmarket B2B SaaS forced into mandates (5–50M ARR) | 50k+ EU targets | €3–20k | PLG, Stripe-Billing-gap content marketing |
| 3 | OSS maintainer of adjacent project (Invoice Ninja, ERPNext, Odoo, Dolibarr) | Indirect | €0 direct | Distribution multiplier — every integration is years of downstream funnel |

**Underserved opportunity**: EDI specialists adding Peppol (regional VANs like ecosio, Babelway, B2B-Router class) — they know Peppol is eating their lunch, API-first newcomers ignore them, no one ships clean UBL↔EDIFACT/X12 mapping. **They will pay €50k+/yr** for migration support.

**Explicitly do NOT chase**: AP/AR ops at corporates 1000+ FTE. They'd pay €100–300k ACV but reshape us into a Coupa/Tipalti clone (closed-source on-prem + 6-12 month sales cycles). Every dollar from them costs a soul-fragment of the OSS roadmap.

---

## 2. The idea pool — composition & process

| Phase | Source | Net new ideas | Output file |
|-------|--------|---------------|-------------|
| 0 | Principal architect baseline ultrathink, 14 axes | 210 + 7 fork decisions | [ideas-phase0-baseline.md](ideas-phase0-baseline.md) |
| 0.5 | Self-critique adversarial filter | (~20 dropped → 190 survivors) | [ideas-phase0.5-critique.md](ideas-phase0.5-critique.md) |
| 1 | Idea-wizard methodology, 2 passes (DX axis + ops axis) | +50, top-15 winnowed | [ideas-phase1-divergent.md](ideas-phase1-divergent.md) |
| 3a | Codex (GPT-5) triangulation: 25 ideas + critique + missing piece | +25 | [triangulation-codex.md](triangulation-codex.md) |
| 3b | Gemini 3 Pro triangulation: 25 ideas + critique + missing piece | +25 | [triangulation-gemini.md](triangulation-gemini.md) |
| 4 | Repeated-apply expansion: anti-disaster, distribution, AI/agentic, adjacent, wild, ops | +155 | [ideas-phase4-expansion.md](ideas-phase4-expansion.md) |
| 4b | Gap-fill ideation in response to critiques (reconciliation, state machines, evidence, JVM coverage, pricing, AI positioning, SSL-Labs reframing) | +56 | [ideas-phase4b-gap-fill.md](ideas-phase4b-gap-fill.md) |
| Duel | Codex cross-scores Gemini's 25 ideas (0–1000 per dim) | (filter) | [duel-codex-scores-gemini.md](duel-codex-scores-gemini.md) |
| Duel | Gemini cross-scores Codex's 25 ideas | (filter) | [duel-gemini-scores-codex.md](duel-gemini-scores-codex.md) |
| Brenner | Codex adversarial falsification | (filter) | [brenner-codex.md](brenner-codex.md) |
| Brenner | Gemini adversarial falsification | (filter) | [brenner-gemini.md](brenner-gemini.md) |
| **Total** | | **~490** | |

**Process notes**:

- All idea generation happened *after* market research completed (research first, ideation second).
- Cross-model triangulation surfaced ideas neither single model would have generated; the cattier the disagreement, the higher the signal.
- Two-model Brenner critique converged on the same 5 highest-risk assumptions, raising confidence in mitigation strategy.
- Two-model Brenner *diverged* on the runaway-success scenario (Plaid-for-B2B vs CI-for-compliance) — both are compatible with the same Y1 foundation.

---

## 3. The synthesized winners — Tier S (10), Tier A (10), Tier B (10)

Full reasoning in [synthesis.md](synthesis.md). Summary here:

### Tier S — Build immediately (consensus, mitigated risks)

| # | Idea | Confirmed by |
|---|------|--------------|
| S1 | **EN 16931 semantic IR with versioned profile extensions; interop with `invopop/gobl` JSON schema** | Phase 0, Codex, Gemini |
| S2 | **Schema-compiled AOT validator (Schematron → Rust → WASM); JVM-wrapped fallback** | Gemini, Codex — risk gated by Brenner E1 |
| S3 | **Smart canonicalizer for byte-stable XML + JSON + PDF subset** (basis for signing, hashing, audit) | Phase 1 wizard top pick, Codex, Gemini |
| S4 | **Time-travel validation** (date-pinned rule packs; "would this invoice have passed compliance when it was issued?") | Codex top score 890/880/760/910 |
| S5 | **Semantic diff API for amendments** (credit notes, corrections, audit trails) | Codex score 860/860/710/870 |
| S6 | **Gov-simulator / Peppol LocalStack** — local AS4/SMP/SML mock with documented failure modes | Codex + Gemini convergence |
| S7 | **Reconciliation engine** (deterministic fingerprint + idempotency + state machine + ACK matching) | Codex's #1 across both lists; Gemini's "missing piece" |
| S8 | **Any-to-any mapping DSL** (declarative ERP-to-IR mappings, compilable to WASM) | Codex score 850/920/610/900 |
| S9 | **Diagnostics-first validator** with rule_id, BT-term, JSON Pointer / XPath, severity, suggested fix, citation | Codex, Gemini |
| S10 | **Evidence bundle format `.invoicekit`** with RFC 3161 timestamps + signature receipts (convenience wrapper, not legal artifact replacement) | Codex, Phase 4b |

### Tier A — Strong differentiators (high consensus or strong single-model conviction)

| # | Idea |
|---|------|
| A1 | **VS Code / Cursor LSP for invoicing** (hover BT terms, real-time squigglies, code actions) |
| A2 | **Tax & rounding proof engine** (decimal math with formal trace — opt-in checker, not replacement) |
| A3 | **Type-state invoice builders** (Rust + TypeScript; invalid invoices unrepresentable in common cases) |
| A4 | **PII/GDPR redactor** (safe staging dumps, shareable support bundles) |
| A5 | **Inbound invoice firewall** (XXE, ZIP bombs, malware detection, parser sandboxing) |
| A6 | **Gateway rejection normalizer** (uniform error categories with remediation across Peppol/KSeF/SDI/ZATCA/IRP/PPF) |
| A7 | **`routePlan(invoice, recipient)` preflight API** — what network/profile/IDs/gateway/fallback needed |
| A8 | **Auditable AI / "LLM as cross-examined witness"** (bbox + OCR span + confidence + deterministic re-validation) |
| A9 | **Property-based test corpus + adversarial generator** (the CI-for-compliance foundation) |
| A10 | **Reverse-proxy sidecar pattern** for JVM/.NET enterprise (solves WASM-leaky-for-enterprise critique) |

### Tier B — Strong adds (1-2 phase consensus)

B1 Future-mandate migration simulator · B2 Counterparty contract tests · B3 OpenTelemetry semantic conventions · B4 Universal payment instructions · B5 VAT-ID-driven autodetect onboarding · B6 Legal numbering ledger · B7 WASM CIUS plugins · B8 Dirty-ingestion error stream · B9 Content-addressed attachments · B10 SSE/WebSockets for ACK delivery.

### Killed by mutual rejection

| Idea | Reason |
|------|--------|
| ZKP-based factoring | Tiny market, complexity unjustified |
| Smart-contract escrow bridge | Brand-distraction, no liquidity |
| Homomorphic VAT encryption | Academic novelty, no buyer |
| eBPF DB sniffer ingestion | Invasive, brittle, wrong product |
| BYO-LLM via WebGPU as primary | Distraction; AP clerks have weak hardware |
| Bounding-box OCR TUI | Wrong ICP |
| Invoice mediation marketplace | Regulated arbitration, out of scope |
| Public ledger Merkle gimmick | Replaced by RFC 3161 |
| Hardware-attested signing (Apple/Google) | Auditors require eIDAS QTSP |
| Per-tenant LoRA in v1 | Y2+ feature |
| Reverse-pitch incumbents | Pagero/Comarch won't adopt OSS competitor lib |

---

## 4. Fork resolutions

Seven strategic forks identified at inception; all resolved with evidence.

| Fork | Resolution | Reasoning |
|------|-----------|-----------|
| **F1 Primary ICP** | **Embedded developer at ERP/billing SaaS vendor** (Stripe-shape) | Buyer-personas research confirms #1 ICP; OSS adoption compounds; MSFT 2026 ISV framework provides tailwind |
| **F2 Geography** | **EU-first; design IR for global day 1** | Regulatory map shows 2025–2028 wave concentrated in EU; ViDA is forcing function; non-EU formats (CFDI, ZATCA) shape the IR but don't drive Y1 revenue |
| **F3 Peppol AP** | **Hybrid: phase4 wrap Y1 + managed service; native Rust sender Y1 / receiver Y2-3** | Peppol research confirms ISO 27001 is the long-pole gate; node42 proved non-JVM sender feasibility |
| **F4 AI positioning** | **Quiet support capability — never headline; AI ON by default for inbound, OFF by default for outbound** | Codex Brenner critique; buyer-personas show finance buyers fear AI hallucination in regulated docs |
| **F5 Outbound vs Inbound** | **Outbound-first Y1; inbound (AP/OCR) as strong Y1 secondary; reconciliation engine becomes the moat by Y2** | Outbound is deterministic + immediate value; reconciliation engine is the paid control plane |
| **F6 License** | **Apache 2.0** | Patent grant, max community, beats SSPL on adoption friction; protects against GOBL fork |
| **F7 Delivery shape** | **Sequenced: Library/SDK-first (Y1H1) → Managed REST (Y1H2) → Managed Compliance API w/ liability transfer (Y2)** | Codex (library-first lock-in) + Gemini (managed API as revenue) reconciled via sequencing; Brenner E2 will test exact pricing thresholds |

---

## 5. Critical risks & mitigations

Top failure modes converged from both Brenner critiques:

| # | Risk | Mitigation | Falsification |
|---|------|-----------|---------------|
| R1 | **"You build a great validator, but not the validator of record"** (Codex) — auditors require official KoSIT/Saxon/phive output | Diff against KoSIT/Saxon/phive in CI; publish parity dashboards; be transparent | Brenner E1 |
| R2 | **EN 16931 IR collapses under national/profile reality** (both Brenners) | Typed namespaced profile extensions (NOT escape-hatch hashmaps); lossiness ledger; GOBL adapter | Codex's 200-invoice break test |
| R3 | **WASM/Rust enterprise rejection** (Gemini) — Python/C#/Java devs hate FFI tooling | Ship native bindings (pyo3, wasmtime-java, Wasmtime.NET); sidecar pattern for paranoid security policies | Brenner E3 (FFI friction audit) |
| R4 | **Schematron AOT XPath quagmire** (Gemini) | Wrap KoSIT/Saxon validators first (wasmtime-java); AOT incrementally per ruleset; never gate shipping on AOT | Brenner E1 |
| R5 | **Pricing "uncanny valley"** (Gemini) — €49/mo too high for OSS curious, too low for liability transfer | Two-product split: OSS+metered for devs (€0.05/envelope); Managed Compliance API at €499–€1999/mo with liability cap | Brenner E2 |
| R6 | **OSS users don't pay** (Codex) | Managed Compliance API is the revenue product; reconciliation engine is the paid moat; basis-point reconciliation revenue at Y3 | Y1 customer-interview signal |
| R7 | **Browser-side 7B LLM OOM on AP clerks' laptops** (Gemini) | Default server-side Qwen2.5-VL; browser-side limited to Layer 1-3 (SmolDocling-256M max) | A/B test in beta |
| R8 | **Typst PDF rejection by WYSIWYG-PM demands** (Gemini) | Typst is renderer; ship TS template DSL on top; Y2 web WYSIWYG that compiles to TS-DSL-to-Typst | Customer feedback in beta |
| R9 | **Incumbents go down-market with dev APIs** (Codex) — Avalara, Sovos, Storecove already pitching one-API multi-country | Speed: ship Y1 H1 before they retrofit; OSS as moat against managed-only competitors | Quarterly competitive review |
| R10 | **ERP-native distribution eats us** (Codex) — Odoo is Peppol AP; MSFT Dynamics covers 8+ countries; SAP DRC | Become their preferred OSS engine via partnerships; MSFT 2026 connector framework is the lever | Quarterly partnership pipeline |
| R11 | **GOBL becomes dominant standard before us** (Codex) | Interop, don't compete; contribute to GOBL specs; differentiate on intake/WASM/AS4 | Quarterly GOBL alignment review |
| R12 | **National centralization (KSeF, Chorus Pro) makes Peppol less load-bearing** (Gemini) | First-class national gateway integrations alongside Peppol; treat country as the unit of compliance, not the network | Quarterly regulatory review |

---

## 6. The three 60-day falsification experiments (do these first)

| # | Experiment | Goal | Threshold to kill | Cost |
|---|-----------|------|-------------------|------|
| E1 | **Schematron AOT parity gauntlet** | Prove Rust AOT validators can match JVM reference at ≥99.9% rule parity | <99.9% rule parity OR any unexplained fatal mismatch after 3 weeks → wrap Java validator long-term | ~€3k (1 engineer × 3 weeks) |
| E2 | **Pricing landing-page A/B** | Validate €49/mo dev tier vs €499/mo managed API positioning | API conversion 3× higher → pivot to managed-API-first | ~€500 ad spend + 2 days dev |
| E3 | **ICP risk-transfer interviews** | 30 ERP/billing SaaS calls; offer paid 90-day pilot at €5k+ | <5 LOIs / paid pilots → kill library-first; pivot to managed compliance positioning | ~€2-8k outreach + founder time |

Run all three in **Year 1 Q1, in parallel**.

---

## 7. The recommended plan (8-quarter horizon)

Full detail in [/plans/PLAN.md](../plans/PLAN.md) — summary here.

### Year 1 (Mar 2026 – Feb 2027) — Foundation

| Quarter | Focus | Output |
|---------|-------|--------|
| Y1Q1 | Trust core | IR + canonicalizer + EN 16931 core validator + KoSIT/Saxon wrap + Typst PDF/A-3 + .invoicekit bundle + public conformance corpus v0.1. Brenner E1/E2/E3 run. ISO 27001 engagement starts. |
| Y1Q2 | Outbound | 8 outbound serializers + type-state TS builder + native Rust Peppol AS4 sender + sandbox mock gateways + LSP MVP. First 5 design partners. OpenPeppol AP application. |
| Y1Q3 | Transmission | Peppol AS4 receiver (wrap phase4) + SDI + KSeF + ZATCA Phase 2 + French PPF (Sep 1 mandate). Managed Compliance API beta. 10 paying customers. Conformance corpus v0.5. |
| Y1Q4 | Intake + scale | OCR pipeline (PaddleOCR + SmolDocling + Qwen2.5-VL cloud) + cross-examined witness + reconciliation API GA + IRP/MyInvois/myDATA/eTIMS. ISO 27001 audit. 30 paying customers, €100–500k ARR. |

### Year 2 — Network + trust infrastructure

- Native Rust AS4 receiver (replace phase4 wrap)
- OpenPeppol AP certification
- US, UK, Australia, NZ, Singapore Peppol overlays
- WYSIWYG template designer
- Conformance corpus v1.0 + differential test harness as standalone product (Path A)
- Reconciliation engine GA with PO matching, multi-tenancy, audit dashboards
- 100 paying customers, ~€3M ARR

### Year 3 — Plaid for B2B reconciliation (Path B scale-up)

- Embedded reconciliation widget (Stripe-Elements-shape) for ERPs
- Payment orchestration partner (SEPA, ACH, card)
- Take basis points on volume
- 300 paying customers, ~€10M ARR

---

## 8. Two visions, one foundation

The most important insight from the multi-model Brenner phase:

> **Codex's runaway-success bet**: "Become the open conformance corpus and differential test harness for e-invoicing. CI for invoice compliance. Paid product becomes hosted conformance, private corpus testing, release-diff alerts, and certified gateway adapters."

> **Gemini's runaway-success bet**: "Plaid for B2B Invoice Reconciliation. Embedded widget that ERPs put in their UI (Stripe-Elements-shape). Securely ingests Peppol/XML invoices, perfectly normalizes them, automatically matches against open POs, orchestrates payment via open banking APIs. Take basis points on global B2B trade volume."

These are **not in conflict**. They are sequential phases of the same product:

- **Path A (CI for compliance)** builds the **trust infrastructure**. Public corpus, validator-of-record positioning, evidence bundles, differential test harness. This is the **moat against incumbents** because trust takes years to build and is sticky.
- **Path B (Plaid for B2B)** builds the **financial network** on top of that trust. You can't run a B2B reconciliation/payment network without provable invoice compliance — Path A is the prerequisite.

The same architecture supports both: IR + canonicalizer + state machine + evidence bundle + reconciliation API + payment-instruction primitives. **Don't pick one; sequence them.**

---

## 9. What to do next

1. **Read this report end-to-end with the principal**. Confirm fork resolutions, especially F5 (outbound vs inbound emphasis) and F7 (delivery shape).
2. **Decide on the team shape and funding** — this plan implies ~6 engineers + 1 PM + 1 compliance officer over 18 months, roughly €1.5M to break-even. Options: bootstrap via OSS funding + design-partner LOIs, or raise pre-seed.
3. **Begin Brenner E1, E2, E3 immediately** (60 days). These cost ~€10k combined and falsify our biggest bets.
4. **Run plan revision rounds** (the planning-workflow skill recommends 4-5 rounds). One round of codex revision is included with this delivery; recommend running 2-3 more via GPT Pro Extended Reasoning when available.
5. **Convert the build sequence to beads** when the team is ready to implement. Each task in `/plans/PLAN.md` § 8 maps to a self-contained bead with dependencies.
6. **Decide on the name**. "InvoiceKit" is a placeholder. Other working candidates: Forma, Hectare, Pliant. Strong opinions welcome.

---

## 10. Appendix — research file index

| Phase A — Market research | Phase B — Idea generation | Phase C — Synthesis & planning |
|---------------------------|---------------------------|-------------------------------|
| [oss-landscape.md](oss-landscape.md) | [ideas-phase0-baseline.md](ideas-phase0-baseline.md) | [synthesis.md](synthesis.md) |
| [regulatory-map.md](regulatory-map.md) | [ideas-phase0.5-critique.md](ideas-phase0.5-critique.md) | [/plans/PLAN.md](../plans/PLAN.md) |
| [competitive-pricing.md](competitive-pricing.md) | [ideas-phase1-divergent.md](ideas-phase1-divergent.md) | [/plans/PLAN_REVIEW_codex.md](../plans/PLAN_REVIEW_codex.md) |
| [ai-extraction-sota.md](ai-extraction-sota.md) | [ideas-phase4-expansion.md](ideas-phase4-expansion.md) | [project-framing.md](project-framing.md) |
| [buyer-personas.md](buyer-personas.md) | [ideas-phase4b-gap-fill.md](ideas-phase4b-gap-fill.md) | |
| [peppol-network.md](peppol-network.md) | [triangulation-codex.md](triangulation-codex.md) | |
| [dev-pain-points.md](dev-pain-points.md) | [triangulation-gemini.md](triangulation-gemini.md) | |
| | [duel-codex-scores-gemini.md](duel-codex-scores-gemini.md) | |
| | [duel-gemini-scores-codex.md](duel-gemini-scores-codex.md) | |
| | [brenner-codex.md](brenner-codex.md) | |
| | [brenner-gemini.md](brenner-gemini.md) | |

End of master report.
