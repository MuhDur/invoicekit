# Prompt for codex and gemini — cross-model ideation

We are designing a new open-source, developer-first e-invoicing toolkit ("InvoiceKit"). Your task: generate radical, non-obvious, technically rigorous ideas that could differentiate this product, AND offer critique of our current direction.

## Background (validated by market research, May 2026)

**The pain we're attacking** (from real developer threads, GitHub issues, HN, dev.to):
- Schematron validation requires Java/XSLT 2.0 — breaks Node/Deno/Bun/edge runtimes
- Headless-browser PDF (Puppeteer) is unworkable on Cloudflare Workers / edge
- Existing libs silently regress (mustangproject 2.13→2.15 NPE, factur-x Python broken on Debian 12)
- Peppol AP gatekeeping costs ~€2000/yr just for membership plus more
- Stripe Billing maxes at 1k events/sec — no Peppol support
- Customers either need Node service or JVM; "easy to drop in" doesn't exist

**The regulatory wave (driver)**:
- Germany B2B mandate Jan 2027/2028, France PPF Sep 2026, Poland KSeF Feb/Apr 2026, Belgium Jan 2026 (live), Italy SDI mature
- Saudi ZATCA waves 23/24 (Mar/Jun 2026), India IRP, UAE Jan 2027
- EU ViDA full rollout 2030-2035
- Underestimated: Malaysia MyInvois Phase 4 Jan 2026, Kenya "No Invoice, No Deduction" Jan 2026

**Architecture bet**:
- Rust core compiled to WASM, runs everywhere (browser, edge, Node, Bun, Deno, JVM via wasmtime, Python, Go)
- EN 16931 semantic IR as canonical model, every format is a serializer
- Interop with `invopop/gobl` JSON schema (don't reinvent)
- OSS core MIT/Apache 2.0 forever; paid managed services (Peppol AP, country gateways, certs, archive, cloud LLM fallback)

**Buyer (primary ICP)**: embedded developer at ERP/billing SaaS vendor (Odoo partners, Microsoft Dynamics ISVs, sevDesk, Pennylane, Lexware, Pleo) — ~500 EU targets, €15-150k ACV.

**Strategic moats we want to build**:
- Only WASM-native invoicing library — first mover
- Auditable AI (every extracted field cites bounding box; never silent hallucination)
- Canonical IR (lock-in via standard)
- Peppol AP wrapping phase4 Y1, Rust-native Y2-3
- Free public validator becomes the trust default (like SSL Labs)

**Pricing**: Free up to ~100 envelopes/mo; Pro €29-49/mo for 1000 envelopes; €0.05/envelope flat above. No per-country, no per-entity, no supplier fees. Marginal cost €0.001-0.005/envelope; break-even at €0.05 needs ~3M envelopes/yr.

**Top forks we still need to decide**:
- F1: Embedded dev (Stripe-shape) vs Enterprise CFO (Pagero-shape)
- F2: EU-first vs global-from-day-1
- F3: DIY Peppol AP vs partner (current plan: hybrid — wrap phase4 Y1)
- F4: AI as headline vs quiet support
- F5: Outbound-first vs inbound-first
- F6: Apache 2.0 vs SSPL/BSL
- F7: Library/SDK vs hosted-API priority

## What I want from you

1. **Give me 25 radical, specific, non-obvious ideas** for InvoiceKit that you don't think we've thought of. Be technical. Be opinionated. Avoid generic SaaS suggestions ("add SSO") — go deep on invoicing-specific or developer-tooling-specific innovation. Out-of-the-box appreciated.

2. **Critique our current direction**: what's the strongest objection to our wedge / architecture / GTM you can think of? Where might we be wrong?

3. **Pick one fork decision (F1-F7)** and argue strongly for one path.

4. **One thing we're missing** that you, with fresh eyes, see clearly.

Don't be polite. Be useful. Be specific. Reference actual technologies, libraries, RFCs, standards by name where relevant. Reply in clearly-numbered structured markdown.
