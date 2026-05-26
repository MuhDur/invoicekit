# Project Framing — "InvoiceKit" (working name)

**Date:** May 2026. Living doc. Updated as market research returns.

## One-liner

**A WASM-native, developer-first toolkit for the entire invoicing lifecycle — intake, normalization, validation, rendering, and transmission — that runs anywhere (browser, edge, Node, Bun, Deno, JVM, Python, Go) and turns the EU/global e-invoicing regulatory wave into a one-import dependency. OSS core, paid network & compliance services.**

## Why now (validated so far)

- **Regulatory tailwind**: EU ViDA, Germany Jan 2025, France PPF, Italy SDI, Spain Verifactu, Poland KSeF, Belgium Jan 2026, plus Saudi ZATCA, India IRP, Mexico CFDI, Singapore Peppol — every ERP/billing app must comply. _[full map: regulatory-map.md, pending]_
- **Pricing arbitrage**: incumbents floor at **€15k/jurisdiction (Sovos)**, average per-doc cost **€0.18–1.50**, vs. realistic marginal cost of **€0.01/envelope** at scale. Self-serve €0.05–0.10/envelope undercuts 5–50× without race-to-bottom. _[competitive-pricing.md]_
- **Tech stack inflection**: Qwen2.5-VL-7B, PaddleOCR PP-StructureV3, Transformers.js v4 + WebGPU make **browser-side extraction realistic in 2026 for Layers 1–2** (digital PDFs, clean scans, Factur-X parsing) — privacy story + cost story. _[ai-extraction-sota.md]_
- **User pain (originating spark)**: existing options force you into Node service OR JVM, browser-native is anemic, no single lib covers intake + normalize + render + transmit cleanly.
- **OSS landscape gaps (confirmed)**: no non-Java Peppol AS4 client; no WASM Schematron validator; no OCR→EN16931 pipeline; no WASM-strict PDF/A-3 builder; no open French PPF SDK. _[oss-landscape.md]_

## Competitive position

**Nearest conceptual competitor**: **invopop/gobl** (Apache-2.0, Go, ~277 stars). Covers UBL/CII/FatturaPA/CFDI/KSeF/VeriFactu/FacturaE/TicketBAI with JSON schema + JWS signing.

**Decision**: **interop with GOBL JSON schema, do NOT reinvent**. Our differentiation:
- (a) **JS/WASM/cross-runtime delivery** (GOBL is Go-only, no browser story)
- (b) **OCR→EN16931 pipeline** (GOBL has none)
- (c) **Non-Java Peppol AS4** (does not exist in OSS — biggest commercial wedge)
- (d) **Browser-native PDF/A-3 with veraPDF-verified Factur-X embedding**
- (e) **WASM Schematron validator** for instant browser validation

## Wedge

> "The only invoicing library that runs in your browser, your Cloudflare Worker, your JVM, and your Python app — and gets the format right for every EU country."

## Strategic principles (lock these in early)

1. **Library, not platform.** Stripe-shape, not Pagero-shape.
2. **Standards-first.** EN 16931 semantic model is the IR. No proprietary lock-in.
3. **OSS core MIT/Apache forever.** Network + compliance services are paid.
4. **Auditable AI only.** Every AI-extracted value carries source-region citation + confidence. **Silent line-item hallucination is the #1 failure mode we refuse to ship.**
5. **Deterministic outbound.** AI for intake; outbound generation is byte-stable.
6. **No per-seat OSS pricing.** Ever.
7. **Country support as data, not code.** Versioned rulesets, community-maintained.

## The seven forks (TBD after research)

| # | Fork | Path A | Path B | Decision driver |
|---|------|--------|--------|-----------------|
| F1 | Primary ICP | Embedded developers (Stripe-shape) | Enterprise CFO/AP (Pagero-shape) | Buyer-persona research |
| F2 | Geographic focus | EU-first (Germany Jan 2025 + ViDA wave) | Global-from-day-1 | Regulatory-map + persona |
| F3 | Peppol AP | DIY (own AP, deeper moat) | Partner (faster, thinner margin) | Peppol-network deep dive |
| F4 | AI positioning | Headline ("AI-powered invoicing") | Quiet support capability | Buyer trust signals |
| F5 | Outbound vs Inbound | Issuer-first (rendering, format conversion) | Receiver-first (OCR, AP automation) | TAM analysis |
| F6 | License | MIT/Apache (max adoption) | SSPL/BSL (control + commercial flex) | Adoption math |
| F7 | Delivery shape | Library/SDK first | Hosted API first | Market research signal |

## Working title candidates

- **InvoiceKit** — generic, descriptive, available on npm? to check
- **Factura** — too generic
- **Invoicely** — taken
- **Bill.dev** — dev-shape
- **Hectare** — random, memorable, .com may be available
- **Forma** — clean, suggests "form" + format
- **invoice-rs** / **invoice-wasm** — descriptive but boring

_(naming is downstream, leaving as parking lot)_

## Reference architecture (provisional)

```
┌─────────────────────────────────────────────────────────┐
│ Bindings  npm  cargo  pip  go  jvm  cli  REST           │
├─────────────────────────────────────────────────────────┤
│ Capability layers (tree-shakable)                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐  │
│  │ INTAKE   │ │ NORMALIZE│ │ RENDER   │ │ TRANSMIT   │  │
│  │ pdf→IR   │ │ rules    │ │ IR→PDF/  │ │ Peppol AS4 │  │
│  │ ocr→IR   │ │ validate │ │ XML/HTML │ │ SDI/KSeF/  │  │
│  │ ai→IR    │ │ canonic  │ │ template │ │ IRP/...    │  │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬───────┘  │
│       └────────────┴────────────┴────────────┘          │
│                        ▼                                │
│               ┌──────────────────┐                      │
│               │ Canonical IR     │  (EN 16931-based)    │
│               │ JSON-CRDT-able   │                      │
│               └──────────────────┘                      │
├─────────────────────────────────────────────────────────┤
│ Rust core (no-std, no Tokio, WASM-compatible)           │
└─────────────────────────────────────────────────────────┘
```

## Confirmed wedge (from dev-pain-points.md)

> **A TypeScript/WASM-first library that generates AND validates EN 16931 (XRechnung + Factur-X + Peppol BIS) on Bun/Deno/Cloudflare Workers/edge runtimes** — solving the Schematron-requires-Java trinity, the edge-PDF-rendering wall, and silent-library-regression risk simultaneously. Defer "operate our own Peppol AP" past Y1; route around the gatekeepers initially.

## Buyer profile (from buyer-personas.md)

- **Primary ICP** = embedded developer at ERP/billing SaaS vendor (~500 EU targets, €15-150k ACV)
- **Secondary** = in-house engineer at midmarket B2B SaaS forced into mandates (50k+ targets, €3-20k ACV, PLG fit)
- **Distribution multiplier** = OSS maintainer of adjacent project (Invoice Ninja, ERPNext, Odoo, Dolibarr)
- **Underserved opportunity** = EDI specialists adding Peppol (regional VANs)
- **Explicit non-target** = AP/AR ops at corporates 1000+ FTE (they'd warp us into Coupa/Tipalti)

## Regulatory priority (from regulatory-map.md)

**Top 10 markets by urgency × size**: Germany, France, Poland, Belgium, Italy, Spain, Saudi Arabia, India, UAE, Greece.

**Underestimated deadlines**: Malaysia MyInvois Phase 4 (Jan 2026), Kenya "No Invoice No Deduction" (Jan 2026).

## Open questions (post-research)

- Specific repository to fork vs greenfield (decided per-module — see synthesis)
- Exact AP partner vs DIY timing
- Hosted API vs library priority at GA
