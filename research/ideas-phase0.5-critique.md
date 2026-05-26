# Phase 0.5 — Adversarial critique of Phase 0 ideas

Steel-manning the strongest objections to the ideas in `ideas-phase0-baseline.md`. This becomes the seed for the dueling-wizards phase and the synthesis filter. Aim: kill any idea that can't survive contact with an informed skeptic.

---

## Architectural ideas — strongest objections

### #1 WASM-native Rust core
**Best objection:** "WASM is still meaningfully slower than native in some workloads (PDF rendering, XML signing). For high-volume backend use you'll lose to a JVM that's been tuned for 25 years."

**Counter:** True for raw throughput at the **microsecond** level. But (a) for invoicing, the workloads are I/O-bound or human-paced, (b) you can ship the same Rust as native binary for backend (`cargo build`), only the *delivery shape* uses WASM, (c) most actual customer pain is **integration**, not throughput. Strong: keep.

### #2 EN 16931 IR as canonical
**Best objection:** "EN 16931 doesn't cover every field that real invoices need — purchase order references, payment terms variations, attached documents, non-EU formats like CFDI which have a different cardinality model. You'll either fork EN 16931 (losing the standard) or constantly fail customers."

**Counter:** Use EN 16931 as the **core IR**, with explicit extension points (`additional_data` map per node) that don't violate canonical fidelity. Document loss table per format conversion. Worst case: ship a superset "InvoiceIR" that downgrades to EN 16931 with explicit warnings. This is what GOBL does. **Risk acknowledged**: design the extension envelope carefully or it becomes the next problem.

### #6 Zero-allocation hot paths
**Best objection:** "Premature optimization. Invoicing isn't HFT. You'll add complexity for use cases that don't exist."

**Counter:** Mostly right. Make this a stretch goal in a `*_fast` crate; don't sacrifice API ergonomics in the core. Demote to nice-to-have.

### #14 Engine/UI strict separation
**Best objection:** "If the UI lives in a separate crate, the dev experience suffers. Devs want one `npm i` and a React component."

**Counter:** Provide a *thin* React/Vue/Svelte wrapper as a SIBLING package that depends on core. They get one-import DX, we get clean layering. Reframed: "core engine has no UI dependencies; UI sugar is a sibling package, same monorepo, one-shot install."

---

## Format handling

### #19 Cross-format conversion
**Best objection:** "Invoice format conversion is **lossy** because each format encodes different business semantics. Customers will see data degradation, blame us, then blame the standard. We'll spend forever on edge cases."

**Counter:** Promise **semantic preservation within EN 16931 envelope**, explicitly enumerate what's not preserved across the gap (e.g., FatturaPA-specific TipoDocumento → CFDI is a meaningful concept change). Output a **conversion report** with every conversion that lists data loss. Risk acknowledged: customer expectations need careful management.

### #20 Round-trip lossless
**Best objection:** "Round-trip lossless across formats is mathematically impossible if formats encode different things. Don't promise what you can't deliver."

**Counter:** Promise it **only within a single format** (e.g., XRechnung → IR → XRechnung is lossless), not across formats. Wording must be precise.

### #22 Versioned compliance rulesets
**Best objection:** "When Germany updates their CIUS, who guarantees your ruleset is current? Customers will hit production with stale rules and we get sued."

**Counter:** This is a real ops problem. Either: (a) ship a managed "ruleset feed" as a paid service with SLA, (b) make the community responsible with clear maintainer model + freshness badge. Probably both. Need to invest in this.

---

## AI/Intake

### #31 Progressive extraction pipeline
**Best objection:** "Customers want ONE answer with ONE confidence number. They don't want to debug which layer fired and why."

**Counter:** Pipeline is internal; external API returns `{value, confidence, source: {layer, page, bbox}}`. They can ignore the layer if they want, or surface it for power users.

### #34 Auditable AI with bounding boxes
**Best objection:** "Bounding boxes are unstable across PDF renderers; if the PDF was re-rendered or generated from differing fonts, the bbox is wrong. Customers will see misaligned citations and lose trust."

**Counter:** Cite **OCR token spans** (which are content-derived) for scanned input, and **PDF object IDs** for digital PDFs. Fall back to bbox where neither is available. Document the citation taxonomy explicitly. **Important: this is the foundational AI safety story** — invest heavily.

### #36 Active learning loop / LoRA per tenant
**Best objection:** "Per-tenant LoRA is expensive, GPU-heavy, and the cold-start UX is terrible. You're committing to running inference infrastructure for every customer."

**Counter:** Active learning is opt-in and runs as a batch nightly job; per-tenant LoRA is a premium tier feature. Most customers use the shared model. Demote in priority — Y2 feature, not Y1.

### #44 Fraud detection lite
**Best objection:** "Fraud detection is a *very* high-liability claim. If you miss a real fraud, customers will sue. Don't ship this until you have AP-level expertise."

**Counter:** Label it "anomaly hints" not "fraud detection". Surface signals, don't make claims. Position as input to human review. Reframe.

---

## Rendering

### #46 JSX-as-templates rendering to PDF + HTML + email
**Best objection:** "JSX → PDF has a long history of partial implementations. react-pdf is famously slow, mahogany. You'll either build a renderer (months) or wrap something (fragile)."

**Counter:** Use Satori (Vercel) for HTML→PDF; ship a small JSX-to-Satori-friendly renderer. Or, write our own renderer in Rust (compiled to WASM). The renderer is **the largest engineering bet** in the whole project; budget accordingly. Risk: pdf-rendering rabbit hole.

### #48 Pixel-perfect deterministic PDF
**Best objection:** "Pixel-perfect determinism requires byte-stable inputs to OS/font shaping — that's fragile across OS versions and CPU architectures."

**Counter:** Use **harfbuzz pinned** + **embedded fonts pinned** + no system font fallback. Pin everything. Test in CI across architectures. Achievable but expensive.

---

## Network/Transmission

### #61 Rust Peppol AS4 client
**Best objection:** "AS4 receiver-side WS-Security validation is the actual hard part, and Peppol Authorities require ISO 27001 certification of the operator. Even if you write the code, you can't operate without 6-12 months of compliance."

**Counter:** True. Strategy validated by research: **Year 1 wrap phase4 (JVM) in a managed service** to ship; build Rust AS4 in Year 2-3 with phase4 as conformance oracle. Reframe #61 as multi-year roadmap, not v1.

### #62 Peppol AP managed service
**Best objection:** "Storecove, B2BRouter, ecosio already exist with developer-friendly APIs. Why does the market need another one?"

**Counter:** (a) None are OSS-core integrated, (b) pricing is opaque/per-envelope-high, (c) none have first-class browser/edge story for *issuance*, (d) all have minimum commits. Our wedge: free OSS core makes "managed AP" feel like infrastructure, not a vendor lock-in.

### #70 Email-based fallback
**Best objection:** "Email-as-invoice-transmit is what we're trying to escape. Re-introducing it as 'fallback' is anti-pattern; it normalizes the legacy."

**Counter:** Reality: 80% of small business invoices still arrive by email. Fallback is **defensive** for senders whose recipient isn't on Peppol — not encouragement. Document carefully.

---

## Monetization

### #107 €0.05/envelope pricing
**Best objection:** "At €0.05/envelope and €140k/yr fixed costs for an AP, you need 2.8M envelopes/yr just to break even. That's 230k/month. SMBs and most midmarket won't deliver that volume even cumulatively."

**Counter:** Pricing is per-envelope but TAM includes huge midmarket-multi-tenant (one ERP vendor multiplies into thousands of merchants). With 100 ERP partners avg 25k envelopes/month each, that's 30M/yr — comfortably profitable. Need to model carefully. **Strategy implication: ERP-partner channel is the volume play, not direct SMB.**

### #116 Training & certification
**Best objection:** "Cert programs need maturity (2+ years of established product) before they make sense. Don't waste energy here in v1."

**Counter:** Defer.

---

## GTM

### #121 Free viral validator widget
**Best objection:** "Validation results are often confusing to non-experts. A free public tool will surface complaints, edge cases, and 'why does this say my invoice is invalid?' tickets. You'll spend more time supporting it than selling from it."

**Counter:** Pair with a clear "what to do next" sidebar (recommend fix, link to docs, suggest tools). Validator becomes top of funnel.

### #132 Sell to incumbents (reverse pitch)
**Best objection:** "Pagero/Comarch will never adopt an OSS competitor's library. The pitch doesn't work in their org politics."

**Counter:** Probably right. Demote.

---

## Defensive moats

### #178 Data network effect from extraction fingerprints
**Best objection:** "GDPR. You cannot collect invoice fingerprints without explicit consent for each tenant. The legal complexity outweighs the moat."

**Counter:** Make it explicitly opt-in with privacy-preserving local fingerprints (hashed layout features, not data). May still be tricky. Demote in priority.

### #182 Hardware-attested signing
**Best objection:** "Apple/Google attestation is gimmick territory; auditors don't care, regulators don't accept it. eIDAS-qualified signature from a TSP is the only thing that matters."

**Counter:** Correct. Drop or demote to "nice for SMB self-signed use cases".

---

## Radical / out-of-the-box

### #155 Invoice mediation marketplace
**Best objection:** "This is a regulated arbitration market. You'd need to be a registered ADR provider in every jurisdiction. Massive legal lift for unclear value."

**Counter:** Probably out of scope. Park.

### #158 Invoice factoring rails
**Best objection:** "Adjacent industry, totally different sales motion, regulatory beast. We'd lose focus."

**Counter:** Right. Note as adjacent expansion **only if our core succeeds**.

### #160 Programmable receivables
**Best objection:** "Sounds cool, but in practice, payment terms are a legal contract, not a programming construct. Customers want SEPA Direct Debit and Net 30, not crypto-style escrow."

**Counter:** Park. Maybe revisit if Stripe Treasury / Wise / FinTech opens API surface.

### #170 Protobuf IR
**Best objection:** "Adds binary format complexity for theoretical performance benefit. Premature."

**Counter:** Park, revisit at scale.

---

## Anti-features

All anti-features defensible. Particularly important to **explicitly NOT do tax engine** — that's a $$$ market that would consume all engineering attention.

---

## Forks: which way does the critique tilt?

| Fork | Path that survives critique better |
|---|---|
| F1 ICP | **Embedded developers** — enterprise sales motion is slower, deeper, but burns runway. Embedded dev = OSS adoption = compounding. |
| F2 Geo | **EU-first, format-zoo design from day 1** — Germany Jan 2025 mandate is the most concrete near-term demand. ViDA + 5 country mandates by 2028 is the bulk of TAM. |
| F3 Peppol AP | **Hybrid: wrap phase4 Y1, build Rust-native Y2-3** — confirmed by Peppol research. Both paths combined in time. |
| F4 AI positioning | **Quiet support capability** — buyers fear AI hallucination in financial documents. Lead with "compliant", footnote with AI. |
| F5 Outbound/Inbound | **Outbound first (issuance)** — easier wedge: low-touch, deterministic, immediate value. Inbound (AP) is bigger long-term but slower to land. |
| F6 License | **Apache 2.0** — better for adoption than MIT (patent grant), better for community than SSPL/BSL. |
| F7 Delivery shape | **Library/SDK first, hosted API as second-class entry point** — library is the lock-in vector. |

These fork resolutions are NOT final — formal idea-wizard + dueling-wizards may overturn them. But they're defensible after first critique.

---

## What survives the critique?

**Strongest 30 ideas** that emerge after critique (numbered as in Phase 0):

1, 2 (with extension envelope caveats), 3, 5, 7, 8, 9, 11, 13, 14 (reframed), 16, 17, 18, 19 (with conversion report), 22 (with managed feed), 23, 25, 31, 34 (with citation taxonomy), 38, 46 (with budget), 47, 48 (with pinning), 54, 58, 60, 61 (multi-year), 62, 75, 91, 92, 99, 100, 105, 106, 107 (ERP-channel-focused), 110, 117, 118, 120, 121 (with sidebar), 130, 136, 138, 141, 145, 147, 171, 174, 177, 181, 185, 196, 201–210.

**Weakest 20 ideas** (drop or defer):

6 (deferred), 36 (Y2), 44 (rebrand), 116 (defer), 132 (drop), 155 (drop), 158 (adjacent only), 160 (park), 170 (defer), 178 (legal complexity), 182 (drop), 90 (public ledger gimmick), 153 (invoice lockfile — cute but premature), 156 (crypto escrow — drop), 159 (AI negotiation — gimmick), 161 (trust score — privacy minefield), 167 (UIG — solving non-problem unless dedup at network scale).

This gives us a defensible ~190-idea pool entering Phase 1 formal generation.
