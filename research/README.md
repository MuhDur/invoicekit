# Research — invoices project

Goal: produce a world-class market analysis + idea pool + plan for an open-source, developer-first e-invoicing toolkit that can also be partially monetized.

## Phase A — Market research (parallel agents)

| # | File | Status | Headline finding |
|---|------|--------|------------------|
| 1 | [oss-landscape.md](oss-landscape.md) | **done** | 5 critical gaps: no non-Java Peppol AS4 client; no WASM Schematron validator; no OCR→EN16931 pipeline; no WASM PDF/A-3 builder; no open PPF SDK. **invopop/gobl** is conceptual competitor → interop with their JSON schema, differentiate on WASM + OCR + AS4. |
| 2 | [regulatory-map.md](regulatory-map.md) | **done** | Top 10 urgency × market: DE, FR, PL, BE, IT, ES, SA, IN, AE, GR. Underestimated: Malaysia MyInvois Jan 2026, Kenya "No Invoice No Deduction" Jan 2026. |
| 3 | [competitive-pricing.md](competitive-pricing.md) | **done** | €0.05–0.10/envelope undercuts incumbents 5–50×; €15k/jurisdiction is the price floor we're attacking. |
| 4 | [ai-extraction-sota.md](ai-extraction-sota.md) | **done** | Qwen2.5-VL-7B + PaddleOCR is the open stack. Browser-side OK for Layers 1–2. #1 failure mode: silent line-item hallucination → mandatory bbox grounding. |
| 5 | [buyer-personas.md](buyer-personas.md) | **done** | Top ICP: embedded dev at ERP/billing SaaS (€15-150k ACV, 500 EU targets). Underserved: EDI specialists adding Peppol (€50k+/yr). Do NOT chase: AP/AR at 1000+ FTE corps — they warp the product. |
| 6 | [peppol-network.md](peppol-network.md) | **done** | 6-12mo, €30-65k Y1 to become AP. ISO 27001 is the gate. Non-JVM AS4 sender feasible (node42 proves it); receiver-side WS-Sec is the moat. Strategy: wrap phase4 Y1 → Rust-native Y2-3. Break-even ~3M envelopes/yr at €0.05/env. |
| 7 | [dev-pain-points.md](dev-pain-points.md) | **done** | 5 pain themes: (1) Schematron's hidden Java dependency, (2) Peppol AP gatekeeping, (3) silent lib regressions, (4) headless-browser PDF impossible on edge, (5) Stripe Billing 1k-events/sec cap. **Wedge: TS/WASM lib that gens+validates EN16931 on Bun/Deno/CF Workers.** |

## Phase B — Idea generation (target ≥300 ideas)

| # | File | Status | Count |
|---|------|--------|-------|
| 0 | [ideas-phase0-baseline.md](ideas-phase0-baseline.md) | **done** | 210 + 7 forks |
| 1 | [ideas-phase1-divergent.md](ideas-phase1-divergent.md) | pending | — |
| 2 | [ideas-phase2-duel.md](ideas-phase2-duel.md) | pending | — |
| 3 | [ideas-phase3-triangulation.md](ideas-phase3-triangulation.md) | pending | — |
| 4 | [ideas-phase4-expansion.md](ideas-phase4-expansion.md) | pending | — |
| 5 | [brenner-hypotheses.md](brenner-hypotheses.md) | pending | — |

## Phase C — Synthesis & planning

| File | Status |
|------|--------|
| [synthesis.md](synthesis.md) | **done** |
| [../plans/PLAN.md](../plans/PLAN.md) v0.1 | **done** |
| [../plans/PLAN_REVIEW_codex.md](../plans/PLAN_REVIEW_codex.md) | **done** — 20 revisions |
| [../plans/PLAN_v0.2_revisions.md](../plans/PLAN_v0.2_revisions.md) | **done** — integrated |
| [MASTER_REPORT.md](MASTER_REPORT.md) | **done** |
