# Phase 1 — Idea Wizard: divergent generation

Following the idea-wizard methodology. Rubric: robust, reliable, performant, intuitive, user-friendly, ergonomic, useful, compelling, accretive, pragmatic.

**Constraint vs. Phase 0**: I'm forbidding myself from repeating ideas from Phase 0. These are 30 distinct *new* ideas, then we winnow.

---

## Round 1.A — 30 fresh ideas

Focus axis: **Developer experience at point of first contact** + concrete engineering primitives we haven't yet enumerated.

1. **`npx invoicekit doctor`** — runs in any directory and tells you: which e-invoicing formats your business needs (based on cwd `.env` country code or `package.json` repository URL), what your current setup is missing, and a one-line install command per gap. Anti-bashed-head DX.
2. **Zero-config first invoice**: `npx invoicekit init` walks user through their first invoice in 90 seconds, generating a real, valid PDF/A-3 Factur-X they can email a customer. Wizard mode that *teaches* the model.
3. **`invoicekit.config.ts`** — typed config file at project root. Encodes default seller, default tax behavior, default templates. Eliminates the "fill in 30 fields for every API call" complaint everyone has about competitors.
4. **Heuristic compliance autodetect from VAT ID**: paste a German VAT ID → kit configures for XRechnung/ZUGFeRD. Paste a French SIRET → configures for Factur-X with future Chorus Pro readiness. Paste an Italian P.IVA → SDI/FatturaPA. One field → opinion.
5. **Smart canonicalizer**: a single function `canonicalize(invoice)` that normalizes XML whitespace, attribute order, namespace prefixes, date precision, line-item ordering — so two semantically-equivalent invoices produce identical bytes for signing and hashing. **Critical for cryptographic workflows nobody else does well.**
6. **Format inference from filename + first 4KB**: `auto.parse(bytes)` figures out it's XRechnung 3.0.2 vs FatturaPA v1.2.2 vs UBL Invoice 2.1 by sniffing namespaces and root elements. No more "which parser do I use?" code-reading sessions.
7. **Replayable test harness**: ship a golden-file corpus of 500+ real-world invoices (synthesized to be anonymous, per ICP) — buyers can run their own implementation against this benchmark to prove conformance. The corpus *is* part of the OSS distribution.
8. **`invoicekit fuzz`** — a fuzzer that generates pathological invoices (Unicode pile-of-poo in supplier name, microsecond timestamps, negative line totals, decimal-precision edge cases, line items with empty descriptions, currency code that doesn't match country). Use to find competitor bugs and prevent our own.
9. **Build-time validation**: a `vite-plugin-invoicekit` / `rollup-plugin-invoicekit` / webpack-loader that validates every invoice template at build time, so misconfigurations don't reach production.
10. **Schema-aware diff format**: a custom diff output for invoice XML/JSON that ignores cosmetic changes (whitespace, attribute order) and highlights *semantic* deltas. Reviewers see what changed; CI bots can warn on regressions. Tooling for AR/AP teams.
11. **Public Schematron rule explorer**: docs.invoicekit.org has a page per rule (`BR-DE-1`, `BR-CO-3`, etc.) — what it means in plain language, what causes it to fail, common fixes, real-world examples. Becomes the canonical reference; ranks in Google for every rule code.
12. **WASM cold-start under 50ms**: aggressive sliced loading so the WASM module loads + runs first validation in under 50ms in a Cloudflare Worker. Engineering target that justifies the architecture choice.
13. **Stream-mode parsing for batch invoices**: SAF-T files, multi-invoice envelopes, MT940 statements — process gigabytes without OOM. Event-driven parser API.
14. **Built-in PDF/A-3 verifier (no veraPDF needed)** — embed the conformance checker in WASM. Today everyone shells out to veraPDF Java. Removing that one dependency makes WASM stories real.
15. **Universal printable invoice**: a "what-you-see-is-what-the-customer-sees" rendering that's identical across all email clients, browser print, Adobe Reader, mobile, accessibility readers. Single template per company, ten downstream renderings, all stable.
16. **Time-machine validator**: validate an invoice as if it were the rules of date X. Critical for audit ("would this invoice have passed compliance when it was issued?"). Every rule pack is date-stamped and queryable.
17. **First-class corrections workflow**: credit notes, debit notes, replacements, void-and-reissue — each a named operation, not a hand-rolled XML edit. Most libs make corrections impossible.
18. **Sandboxed JavaScript template DSL**: a sandboxed mini-language for templates that's safe to render server-side or in iframes — eval safe, no XSS surface, no fs access. Compiles to deterministic output. (Reuses tools like jsondiscord or built ad-hoc.)
19. **Invoice payment instructions abstraction**: one `payment_instruction` object → outputs as EPC QR (SEPA), Swiss QR-bill, Polish split-payment QR, Bahraini QR, ZATCA QR, EU PSD2 payment link, ACH instruction, crypto address. Templates render whichever the country needs.
20. **`@invoicekit/types` for TypeScript** — single source of truth for the IR with full intellisense from JSDoc'd descriptions sourced from EN 16931 explanatory texts. Hover any field = read the spec.
21. **Tax-engine adapter pattern**: pluggable interface (`TaxEngine` trait/interface) — bring Avalara, TaxJar, Vertex, or built-in basic VAT-rate-by-country. Don't compete with tax engines; harmonize with them.
22. **Idempotency keys + outbox pattern built-in**: every `send_invoice()` call has an idempotency key; replays are no-ops. The outbox is a SQL helper migration we ship. Solves the "we double-sent the customer their invoice" class of bug.
23. **First-class supplier/customer registry**: a typed registry of parties with VAT IDs validated against VIES (EU) / GSTIN (India) / SAT (Mexico) at registration time. Caches with TTL. Eliminates "valid VAT ID" surprises at send-time.
24. **CLI tracebacks point to docs**: every error message ends with `(see docs.invoicekit.org/errors/E0042)`. Make every error a learnable moment.
25. **GitHub Action: validate every PR's sample invoices**: drop `*.invoice.xml` files in your repo; on every PR our action validates them. Becomes part of every billing team's PR template.
26. **Lockstep "what changed in country X" newsletter** — daily/weekly bot reads national authority announcement feeds (KSeF, ZATCA, SDI, KoSIT…), summarizes what changed, what's needed. Free, public, ranks in Google.
27. **Self-hostable status page generator** — for embedded users, generate a static "our invoicing is up" page from their telemetry. Counts as a trust accelerator for their downstream customers.
28. **Cryptographically signed validation receipts**: when our hosted validator says "this invoice is compliant for Germany on 2026-06-01", you get a signed receipt good in audit. Free tier doesn't get receipts; paid tier does. Conversion vector.
29. **Built-in test-double for transmission**: in dev, calls to `transmit()` go to a local "mock Peppol AP" that records & returns canned receipts. Eliminates "I can't test without burning real envelopes". (Stripe test mode equivalent.)
30. **Plugin marketplace, day-one** — a `plugins.invoicekit.org` page with templates, format adapters, transmission adapters. Open submission, with quality bar. Long tail of country/format/template support handled by community.

---

## Winnow → Top 5

Applying the rubric. Top 5 in order from best to worst:

### 🥇 #1: **Smart canonicalizer (idea #5)**

**What it does**: One function `canonicalize(invoice)` produces a byte-stable serialization of any invoice regardless of input ordering, whitespace, namespace prefix choice, attribute order, etc.

**Why it's #1**:

- **Robust + reliable**: it's the foundation for cryptographic operations. Signatures only work on a stable byte stream. Today everyone hand-rolls this; everyone gets it wrong.
- **Compelling**: every customer doing signed/sealed invoices needs this. Without it they can't sign reliably.
- **Accretive**: small to ship (a few hundred LOC), but unlocks #28 (signed receipts), all signing workflows, all dedup, all hash-chain features. Every other feature stands on it.
- **Pragmatic**: clear spec (XML C14N is similar, JSON canonicalization JCS is standardized, we adapt). No moonshot.
- **User-friendly + ergonomic**: one function call vs. fiddling with XML serializers.
- **Performance-friendly**: deterministic output enables caching, dedup, content-addressed storage.

**Why I'm confident**: it solves a problem that every signed-document workflow has, today nobody offers it well in OSS, and it's *the* primitive that compounds. Compare to git's content-addressed model — once you have stable hashing, everything downstream becomes easier.

**Implementation**: Rust core function over IR; serializers produce canonical output; XSDL/JSON canonicalization standards exist; pin font and renderer for PDF byte-stable subset.

---

### 🥈 #2: **Heuristic compliance autodetect from VAT ID (idea #4)**

**What it does**: paste a VAT ID, the toolkit detects the country, selects mandatory formats and rules, suggests templates, validates the rest of the configuration.

**Why it's #2**:

- **Intuitive + user-friendly**: one field → opinionated configuration. The DX equivalent of Stripe's "we figured out your tax behavior automatically".
- **Compelling**: removes the #1 friction in onboarding (figuring out which format you need for which country).
- **Accretive**: every onboarding flow benefits; the country-detection logic is reusable across every product surface.
- **Pragmatic**: VAT ID parsing per country is well-documented; VIES + national lookup APIs exist.
- **Useful**: solves a real pain — customers don't know whether they need XRechnung or Factur-X or both.

**Why confident**: it converts a multi-hour spec-reading exercise into one paste. This is exactly the kind of "obviously better" UX move that wins hearts. And it doubles as marketing — "we know your country's rules before you do".

---

### 🥉 #3: **Stripe-style sandbox / test mode (idea #29)**

**What it does**: `transmit()` calls in development hit a local mock Peppol AP / SDI / KSeF endpoint that returns realistic responses without burning real envelopes.

**Why it's #3**:

- **Intuitive + ergonomic**: matches Stripe's test mode mental model that every developer already knows.
- **Robust**: enables CI testing of full transmission flow, including error paths (rejection, timeout, malformed receipt).
- **Reliable**: catches bugs *before* customers do.
- **Compelling**: removes the #1 anxiety in production-deploying an invoicing change ("did I just send 1000 customers a malformed invoice?").
- **Accretive**: pairs with idempotency (#22), GitHub Action (#25), build-time validation (#9).
- **Pragmatic**: well-trodden pattern from Stripe, OpenAI, Twilio. Implementation is straightforward.

**Why confident**: there is NO competitor who offers a great test-mode for Peppol. This single feature makes "switching away from Pagero" feel safe in the way "switching to Stripe" felt safe in 2012.

---

### 4: **WASM cold-start under 50ms (idea #12)**

**What it does**: aggressive engineering target — module loads & validates a Factur-X invoice in <50ms on a Cloudflare Worker.

**Why it's #4**:

- **Performance**: every milli matters in edge runtimes — they bill by CPU time.
- **Compelling**: it's the proof-point that lets us claim "runs on the edge". A 5-second cold start would defeat the architectural bet.
- **Pragmatic**: achievable with careful Rust + module slicing + dictionary-init tricks (we know it's done in other WASM modules).
- **Accretive**: enables every edge-runtime deployment, which is our biggest GTM differentiator.

**Why confident**: it's an engineering target, not a guess. If we don't hit it, the architectural bet (Idea #1 in Phase 0) is weakened. **This is more a non-negotiable commitment than an idea**.

---

### 5: **Stream-mode parsing for batch invoices (idea #13)**

**What it does**: event-driven streaming parser for multi-MB envelopes, SAF-T files, multi-invoice batches.

**Why it's #5**:

- **Performance + robust**: handles enterprise-scale loads without rearchitecture.
- **Useful**: a real wedge for big-volume cases (banks, telcos, govt) that demand "millions of envelopes per night" handling.
- **Pragmatic**: quick-xml + serde-stream in Rust is mature.
- **Accretive**: pairs with idempotency, outbox, and high-volume Peppol AP economics.

**Why confident**: most competitor libs OOM on enterprise data. The bar is low; the impact is high.

---

## Phase 3 → next 10

In order of strength after the top 5:

### #6: **`invoicekit doctor` (idea #1)**

Brings the diagnostic mental model that Cargo/Brew/Rustup popularized into invoicing. Anyone running the tool will get a tailored to-do list. Compelling and pragmatic; one-day implementation.

### #7: **Public Schematron rule explorer (idea #11)**

This is a **SEO + trust** machine. Every rule code becomes a docs page that ranks for "BR-DE-1 explanation" — those queries pull frustrated devs straight to us. Cost: write a docs page per rule (auto-generatable from rule descriptions + community fills in examples).

### #8: **GitHub Action for PR-level invoice validation (idea #25)**

The viral distribution vector — every billing-team repo that adopts it surfaces our brand to PR reviewers daily. Cost: small, mostly Docker + CLI wrapper.

### #9: **First-class corrections workflow (idea #17)**

Credit notes, debit notes, replacements, voids are operations that most libs make impossible. Naming them first-class is a UX win and a correctness win.

### #10: **Universal payment instructions (idea #19)**

A single object → many country QR codes / payment formats. Reduces 12 different country-specific payment integration code paths to one config object.

### #11: **Replayable corpus / golden-file suite (idea #7)**

Ships our trust story: anyone can verify our conformance, compare other libs against ours, generate their own corpus from real data. Becomes the de-facto invoice testing benchmark.

### #12: **TypeScript types from EN 16931 explanatory texts (idea #20)**

Hovering any field in your IDE reads the spec. This is the DX moat that takes years for competitors to replicate.

### #13: **Country newsletter bot (idea #26)**

Free, public, captures everyone in the regulatory anxiety market. Top-of-funnel; demonstrates expertise.

### #14: **`invoicekit fuzz` (idea #8)**

Adversarial generation. Lets us find our own bugs before customers do, and lets customers prove their integration is robust. Differentiates from "happy path validators".

### #15: **Time-machine validator (idea #16)**

Audit-grade — validate "was this invoice compliant as of the date it was issued?" Critical for litigation and audit. Most competitors don't even know this is a problem.

---

## Round 1.B — 20 more ideas (different focus axis)

Focus axis: **Operations, hosting, billing, ops tooling, internal infrastructure** (since most preceding ideas were DX-focused).

31. **Synthetic monitoring**: hosted product runs nightly synthetic invoices through every supported country pipe and publishes uptime + correctness on a public status page. Trust play.
32. **Hash-chain invoice ledger**: each tenant's invoices form a hash chain (Verifactu-style); public Merkle root published daily to a public timestamped log. Anyone can later prove an invoice existed at a date.
33. **Per-tenant key isolation with HSM**: signing keys stored in cloud HSM (AWS CloudHSM / Hashicorp Vault HSM), per-tenant, never decrypted in app memory. Enterprise-table-stakes.
34. **eIDAS-qualified seal as a service**: integrate with EU qualified TSP (e.g. D-Trust, GlobalSign, Adobe Sign EU) to attach qualified electronic seals on outbound invoices. Differentiates against US-y tools.
35. **Rate-card transparency**: live `pricing.invoicekit.org/calculator` lets prospects estimate spend before signing up. Trust-through-transparency. Strange how rare this is.
36. **Customer-owned encryption keys (CKMS)**: paying customers can BYO KMS; we never see plaintext invoice content. Enterprise privacy story.
37. **Per-country compliance attestation reports**: monthly PDF reports for SOC2 / ISO27001 auditors of compliance with each jurisdiction's e-invoicing rules.
38. **Audit log streaming to customer SIEM**: structured logs delivered to Splunk/Datadog/Loki via OpenTelemetry. Enterprise table-stakes.
39. **GDPR right-to-erasure tooling**: pseudonymize old invoices while preserving aggregates and hash-chain integrity.
40. **Sovereignty-aware deployment**: select EU-only / US-only / India-only data residency at signup. Region-locked queues, region-locked compute. Important for govt buyers.
41. **Cross-region replication for archive**: critical archive copies across regions for disaster recovery. Tied to retention SLAs.
42. **WORM-compliant archive (Object Lock S3)**: write-once-read-many archival storage, 10-year retention default, tamper-evident.
43. **PCI-DSS scope minimization**: explicit "this product never touches payment card data" claim, designed-in.
44. **Bring-your-own-CDN for templates**: customers can serve invoices from their own CDN with their own domain; we sign the renders.
45. **Pluggable storage backends**: S3, Azure Blob, GCS, MinIO, local FS — same code, different binding.
46. **Open telemetry semantic conventions for invoicing**: define & ship a standard OTel schema for invoicing operations. Other tools can adopt. Contributes to ecosystem; positions us as standards-setters.
47. **Per-customer KPI dashboards**: outbound throughput, rejection rate per country, p50/p99 latency, oldest unacked envelope. Inside-baseball metrics for ops teams.
48. **Cost telemetry**: per-tenant breakdown of which features drive cost (envelopes, OCR pages, archival GB). Helps customers attribute spend internally.
49. **Open-source ops toolkit**: terraform / pulumi / cdk modules to self-host the full stack. Even paid customers get them — encourages mixed deployment.
50. **Capacity reservation contracts**: enterprise customers can pre-buy envelope quotas to lock in price. Reduces churn risk.

---

## After this round

- **Phase 0**: 210 ideas
- **Phase 0.5**: ~190 survivors after critique
- **Phase 1 Round A**: 30 new (top 5 + next 10 detailed; remainder catalogued)
- **Phase 1 Round B**: 20 new

**Total distinct ideas to date**: 210 + 30 + 20 = **260 ideas** (minus ~20 dropped in critique = 240 active).

Next: Phase 2 dueling, Phase 3 multi-model, Phase 4 expansion, Phase 5 Brenner. Target 300+ should be reached comfortably.
