# Phase 4 — Repeatedly Apply Skill: idea-wizard, more focus axes

I'm running additional idea-wizard cycles, each with a different focus axis, to push the pool past 300 and surface ideas Phase 0/1 missed.

---

## Pass A — Focus: "Anti-disaster" engineering (50 ideas)

Focus axis: **What could go catastrophically wrong, and what specific engineering primitive prevents it?**

51. **Append-only event store for invoice state**: every state transition (draft → validated → signed → transmitted → acked → archived) is an append-only event. Replayable from zero. Bug-tolerant.
52. **Two-phase send with reservation**: outbound `send()` first reserves a sequence number / IRN with the recipient/portal, then commits the actual transmission. Reservation can roll back. Reduces "I sent it twice / partially" scenarios.
53. **Backpressure handling**: in batch mode, drainer respects downstream rate limits (SDI, KSeF, etc.) without ever dropping. Persistent retry with exponential backoff + jitter, max retries configurable, dead-letter queue for true failures.
54. **Schema drift detector**: if the country authority releases an updated schema and customer's pinned ruleset is older, our managed validator warns at every invoice; CI hook can fail builds.
55. **Currency-precision discipline**: all monetary values use `MoneyDecimal` type (256-bit fixed-decimal), never floats. Banker's rounding standard, configurable per jurisdiction.
56. **Number-format negotiator**: invoices addressed to German recipients use `1.234,56`; same data to French shows `1 234,56`; UK shows `1,234.56`. Driven by recipient locale, not seller.
57. **Sealed entity model**: invoices are immutable post-transmission — any "fix" creates a credit note + replacement, not a mutation. Library enforces this; mutation methods throw after seal.
58. **Tamper-evident envelope**: PDF + XML bundle ships with a Merkle hash; recipients can verify integrity from the receipt without needing keys.
59. **Bill of materials for renders**: every output PDF embeds in metadata the exact font, template version, schema version, validator version used. Reproducibility for audit.
60. **Out-of-band signing service interface**: signing is always a separate process (HSM call, TSP API) — library never holds private keys in memory after release.
61. **Forward-secrecy for archive**: archive encryption uses ephemeral keys rotated yearly; even if we're compromised, historical archive remains opaque to attackers without rotating-key escrow.
62. **Built-in clock skew tolerance**: signatures + timestamps allow ±5min skew vs NTP; settings configurable. Avoids "signature invalid because your laptop clock is off" reports.
63. **Idempotency in the IR**: every invoice has an `idempotency_id` independent of issuer's invoice number; we dedup on this. Solves "your client sent the same invoice 7 times" support tickets.
64. **Dead-letter promotion workflow**: stuck envelopes get a UI to inspect, fix, retry without code; ops engineers can do triage without engineering tickets.
65. **Per-country failure mode runbook**: when KSeF/SDI/IRP/etc. flag an error, the library produces a localized message with the exact fix steps. Sourced from real failure cases.
66. **No `null` in core IR**: every field that could be "missing" is explicit (`Some/None`, `option`). No `null` ambiguity.
67. **Forbid silent truncation**: if a customer attempts to set a field longer than spec allows, library throws (configurable to warn + truncate with explicit log).
68. **Validation gate before send**: `transmit()` cannot be called on an invoice that hasn't passed validation. Type-system enforced where possible.
69. **Stateless retry semantics**: every operation accepts an `attempt_id`; if the network ack got lost on attempt 1, attempt 2 with same id is reconciled, not duplicated.
70. **Receiver-side acknowledgement deadline**: `awaiting_acknowledgement` invoices auto-transition to `lost` after configurable timeout; alarms trigger.
71. **Cross-process IR fingerprinting**: serialize an IR through any binding (npm, pip, cargo), fingerprint is identical. Enables interop testing.
72. **Property-based tests in our test suite**: proptest / quickcheck-style — feed random valid IR, must always serialize-deserialize round-trip.
73. **Fuzzing harness in CI**: every PR runs a 30-second fuzz cycle on parsers and validators to catch crashes.
74. **Mutation testing on validators**: ensure our validator catches the bugs we expect it to.
75. **License compatibility tracker**: every dependency (in JS, Rust, Python bindings) tracked for Apache 2.0 compatibility; CI fails on incompatible dep.
76. **Crash dump capture**: opt-in crash reports include sanitized IR (PII stripped) so we can repro.
77. **Replayable bug reports**: every customer-reported bug can be reproduced with a single command + zipfile (IR + config + step), if customer opts to share.
78. **Multi-version validator running side-by-side**: customer can validate same invoice against compliance rules of multiple years/versions in one call. Useful for transitioning periods.
79. **National authority polling for rule updates**: managed service polls every authoritative source nightly, alerts customers on changes that affect them.
80. **Failure budget tracking**: managed service surfaces per-customer per-country failure rate; SLO-like — if it drifts, we proactively reach out.
81. **No "ignore validation errors" feature**: this isn't a config flag. If you need to send an invalid invoice, you write `mark_invalid_for_test()` explicitly and the receipt is annotated as test.
82. **Time-bomb detection**: validate "this invoice is valid as of issue date; here's a list of upcoming rule changes that would invalidate similar invoices issued after X date".
83. **Hosted "sandbox-clone-of-production" environment**: managed customers can clone their production into a sandbox, mutate, see what breaks.
84. **No "default to silent" anywhere**: every default behavior surfaces via logs. No mysterious silence.
85. **Built-in load testing utility**: `invoicekit benchmark` runs a stress test on the user's deployment; produces a report (throughput, p99 latency).
86. **No global state**: every operation accepts its config explicitly; library is functional. Concurrency-safe by construction.
87. **Memory safety from Rust core; no `unsafe` outside explicit FFI seams**: documented & lint-enforced.
88. **JSON Schema for all public APIs**: every request/response has a published schema; clients can validate.
89. **Versioned breaking-change guarantee**: semver-strict, with codemods for major version bumps. Like Vite-style migrations.
90. **No global mutex contention in WASM**: deliberate concurrency model so heavy multi-tenant Cloudflare Workers don't serialize on shared locks.
91. **Defensive limits on all parser inputs**: max envelope size, max line items, max attached docs. Configurable. Defaults prevent OOM/algorithmic complexity attacks.
92. **Open-source security policy from day 1**: SECURITY.md, sec mailing list, CVE process, disclosure timeline. Builds enterprise trust.
93. **No hardcoded test data**: every test fixture is generated from explicit factory functions. No "magic UUID 42a13b..." anywhere.
94. **Per-operation telemetry**: structured logs with span IDs, correlation across operations.
95. **Failure injection test mode**: `transmit(... { fail: "timeout" })` makes the test harness return a timeout; covers error paths.
96. **Schema migration tooling for the IR itself**: when we evolve the IR, customers' stored invoices get migrated forward with explicit code paths.
97. **Audit replay tool**: given an archive snapshot, replay the original validation, signing, transmission. Forensic-grade.
98. **No leaky abstractions across runtime bindings**: Python binding hides Rust panics, never bubbles raw Rust errors.
99. **Per-tenant rate limiting baked into managed service**: each customer can configure their own ceiling, surfaced in usage dashboards.
100. **Graceful degradation when ML model unavailable**: if Qwen2.5-VL fails to load, pipeline drops to OCR-only with explicit confidence drop.

---

## Pass B — Focus: "Distribution & ecosystem" (30 ideas)

Focus axis: **How do we become unavoidable infrastructure?**

101. **Be the validator that schools teach**: write the canonical CS/accounting curriculum module on e-invoicing, give it free to universities. Future engineers learn invoicing through our library.
102. **Stack Overflow tag ownership**: every `xrechnung`, `factur-x`, `peppol-bis`, `zugferd` SO question gets a high-quality answer from our team. We become the canonical reference.
103. **Cookiecutter / template repos**: `invoicekit-starter-nextjs`, `-fastapi`, `-django`, `-rails`, `-go`, `-shopify-app`. One command, working invoicing.
104. **WordPress / WooCommerce plugin**: 40% of global e-commerce. Cheapest distribution.
105. **Shopify app**: invoice compliance as a one-click install.
106. **Magento / Salesforce Commerce extension**: enterprise long tail.
107. **Stripe Apps integration**: when a Stripe invoice is paid, our app auto-generates the country-correct Peppol invoice & transmits. Plugs the Stripe Billing gap that customers complain about.
108. **Lago integration**: open-source billing engine; we're their e-invoice layer.
109. **Maxio / Chargebee / Recurly compliance add-on**: their customers desperately need country compliance; we ship as an add-on.
110. **Backstage plugin**: enterprise dev portals get an "invoicing" tab; standardized DX.
111. **Cursor / Continue / Aider / Claude Code skill**: an MCP server exposes our library to AI dev tools. AI agents can "send a compliant invoice" as a tool call.
112. **OpenAI custom GPT**: free public "Invoicing compliance assistant" GPT; demos our library.
113. **Anthropic MCP server**: same. We become the canonical e-invoicing MCP.
114. **GitHub Marketplace presence**: discoverable Action + the marketplace listing.
115. **NPM trending hijack**: launch with curated showcase repos that drive npm trend signal.
116. **Awesome-list curation**: own `awesome-e-invoicing`, `awesome-peppol`, `awesome-xrechnung` repos. Become the gateway.
117. **Stack of "build in public" content**: weekly Twitter/LinkedIn posts on architecture decisions, code drops, war stories.
118. **Conference circuit**: JSConf, RustConf, FOSDEM Tax/Finance devroom, EuroPython, AWS re:Invent. Be everywhere with talks.
119. **Annual e-invoicing OSS hackathon**: sponsor a yearly virtual hackathon; bounties for missing country support.
120. **CTA: "free conformance audit"** — visit our site, upload your invoice, get a free report. Lead-gen + viral.
121. **Translation crowd-sourcing**: docs in DE/FR/IT/ES/PL contributed by community with bounties.
122. **Tax advisor "white label" program**: Steuerberater / expert-comptables can co-brand our embedded experience for their clients.
123. **University capstone program**: partner with EU universities to have undergrads contribute country implementations as semester projects.
124. **Bug bounty program**: HackerOne or Intigriti, signal that we take security seriously.
125. **Substack newsletter "Compliance Weekly"**: 5-min read on what changed in e-invoicing, with our brand. Build mailing list.
126. **YouTube tutorials**: "How to send your first XRechnung", "AP automation in 100 lines of TypeScript", etc.
127. **Public roadmap voting**: customers vote on which country / feature ships next; democratic prioritization.
128. **Annual "State of E-Invoicing" report**: ours. Like Stripe's annual reports. PR & SEO machine.
129. **Direct relationships with national authorities**: become a listed/recommended solution on KoSIT, IMDA, ATO, Agenzia delle Entrate sites.
130. **Direct relationships with Big4 implementation arms**: Deloitte/EY/KPMG/PwC certified partners.

---

## Pass C — Focus: "Cutting-edge AI / agentic" (30 ideas)

Focus axis: **AI capabilities that are credible in 2026 specifically**.

131. **AI agent that handles compliance Q&A for users**: an MCP server / chat agent that takes "I'm shipping to a French customer, what fields do I need on the XRechnung?" and answers with citations to the actual rule.
132. **Agentic AP**: in receiver mode, an agent receives an invoice, looks up matching PO, confirms goods receipt, posts the journal entry to the ERP, queues approval. Explainable each step.
133. **Auto-categorization for chart of accounts**: small fine-tuned model on common GL codes per industry; predicts which account a vendor's line items belong to. Editable. Learns per tenant.
134. **Vendor master enrichment**: when a new vendor appears on an invoice, automatically enrich: VIES lookup, D&B lookup, website extraction, normalization.
135. **Duplicate detection across formats**: same invoice arriving as PDF email AND Peppol XML AND emailed XML attachment — collapse to one canonical record.
136. **Anomaly explainer**: when our anomaly detector flags an invoice, the AI explains why (in plain language with citation).
137. **VLM-based template auto-detection**: paste a screenshot of a target invoice layout; small VLM identifies fields, generates initial template scaffold.
138. **Negotiation drafting**: AI drafts a "we dispute this invoice" letter referencing specific line items and reasons; user reviews & sends.
139. **Schema-aware delete prevention**: warn if AI extraction lost a field that's typically present for this vendor.
140. **Predictive cash flow**: AP/AR aggregate to predict next-quarter cash flow. Side feature; useful for CFO buyer persona.
141. **Multilingual customer support agent**: our hosted support agent handles invoicing questions in 8 EU languages with citations.
142. **Doc-aware comparison**: "show me how this invoice differs from the same vendor's last invoice" — semantic diff with explanation.
143. **AI-assisted IR migration**: when our IR version bumps, AI helps migrate stored invoices and flags anything ambiguous.
144. **Receipt-to-invoice promotion**: AI generates a draft invoice from a customer's expense report or quotation.
145. **Hosted "playground" with multimodal model**: paste any invoice image, see extraction + validation + would-be-transmission, all in browser.
146. **Continuous prompt evals**: every prompt our infra uses gets versioned + evaluated against golden corpus on every change. Models change, our outputs must stay correct.
147. **Cost-aware routing**: if envelope volume is high, route extraction through cheaper local model; if low, splurge on GPT-4o-class.
148. **Private/local-only mode**: customer chooses whether AI extraction runs locally (Qwen2.5-VL 7B in their VPC) or in our cloud; transparent toggle, transparent cost.
149. **Differential privacy in aggregated feedback**: only differentially-private aggregates leave a tenant; no raw extracted data crosses tenant boundary.
150. **Federated learning option**: customers can opt-in to contribute to a shared extraction model that improves for everyone; never share raw data.
151. **Function calling for ERP write-back**: AI agent uses tools to write extracted invoice into customer's ERP via REST/SOAP/OData calls. Each ERP gets an MCP-style adapter.
152. **AI-driven evidence collection for disputes**: collects PO, GR, prior invoices, communication trail to support dispute resolution.
153. **Cross-document reasoning**: link an invoice to its PO and contract via embedding similarity; verify alignment.
154. **Drift detection on AI outputs**: monitor extracted field distributions over time; alert if a vendor's invoice format changed.
155. **Voice interface for AP**: "Hey assistant, what invoices are overdue?" Useful for warehouse / on-the-floor staff.
156. **Image generation for invoice mock-ups**: marketing teams use it to design brand-aligned invoice layouts; outputs become templates.
157. **AI auto-categorization explanation card**: every AI categorization comes with a "why I think this" card editable by user; corrections fine-tune.
158. **Multi-modal validation**: VLM checks that PDF rendering matches XML semantically (no rendering bug hides a price discrepancy).
159. **AI-driven test data generation**: generates realistic synthetic invoices for testing (with country-correct, statistically-faithful distributions). 
160. **Active fraud signal aggregation**: AI watches across tenants (privacy-preserving) for emerging patterns; surfaces to all customers as "vendor X has elevated dispute rate this month".

---

## Pass D — Focus: "Adjacent expansions" + "User journeys" (20 ideas)

161. **Recurring invoice subscriptions with country-correct semantics**: every renewal honors current rules even as they change.
162. **Multi-currency invoice with FX disclosure**: shows base + converted currency with timestamp & rate source.
163. **Discount/early-payment offer**: encode 2/10 net 30 as a structured object; recipients can act on it programmatically.
164. **Payment-on-invoice-page hosted experience**: like Stripe's hosted invoice URL but rendered with our templates, with country-correct payment options.
165. **Approval workflow primitives**: "this invoice needs manager approval before send" — built-in state machine with hooks.
166. **Customer self-service portal**: hosted brand-able portal where customers can see/download invoices, view history, dispute. Out of the box for embedders.
167. **Invoice negotiation messaging thread**: structured comm channel attached to each invoice — disputes, questions, agreements — preserved with the document.
168. **Vendor onboarding workflow**: structured collection of vendor info (VAT ID, Peppol ID, payment details, certs) with country-correct validation at each step.
169. **B2C consumer invoice format support**: ZATCA simplified VAT invoices for B2C; same engine.
170. **Donation receipt support**: nonprofits need receipts that meet jurisdiction tax-deduction rules; our engine handles it.
171. **Pro-forma invoice support**: distinct lifecycle, must convert to final invoice properly.
172. **Self-billing workflow**: customer issues invoices on behalf of supplier with mutual agreement; first-class.
173. **Service-level statements**: monthly statement summarizing invoices, payments, balances. PDF + Peppol "Statement" doctype.
174. **Cross-tenant supplier discovery**: opt-in directory of vendors that any customer can find. Network effect within our ecosystem.
175. **AP/AR cash forecasting embedded**: simple cash forecasting using invoice timing data.
176. **Reverse-charge VAT visualization**: clearly explain who pays VAT when on B2B EU cross-border. AI explainer per invoice.
177. **Import duty + customs invoice support**: separate primitives but related; many enterprise customers have this need.
178. **Public sector contract invoice formats**: Italy SDI ItalIA, French Chorus Pro Public CIUS; first-class.
179. **Multiple-establishment invoice**: companies with multiple branches must use establishment-specific VAT; first-class.
180. **Embedded carbon-footprint reporting**: regulation coming in EU; ship hooks for line-item carbon attribution.

---

## Pass E — Focus: "Boundary-blurring innovations" (15 wild ones)

181. **"Invoice as a Pull Request"**: customer's AP system reviews invoices via familiar PR review UI. Comments, approve, merge to GL. Github-shape financial workflows. (Genuinely odd, but engineers would *love* this.)
182. **GitHub Pages for invoices**: every invoice gets a unique URL (with auth) — fully static, browser-renderable, sharable like a doc.
183. **Live-collab edit for invoice drafts**: multiple AR reps + customer rep can co-edit a draft invoice with CRDT, like Figma. Disputes resolved before issuance.
184. **Smart-contract escrow as opt-in payment rail**: customer can fund a contract that auto-releases upon Peppol delivery + receipt. Optional, off by default.
185. **Auto-generated press release** when monthly invoicing volume crosses milestones: marketing-flavored, customer-branded; opt-in.
186. **Invoice as podcast**: AI reads the invoice aloud for accessibility / radio-style consumption. (Niche but interesting.)
187. **Browser-extension PO matcher**: when looking at any invoice in any web UI, our Chrome extension overlays "I found a matching PO".
188. **Cross-vendor benchmarking**: opt-in pricing benchmarks across the network; tells you when a vendor is overcharging vs peer-set.
189. **Forward-payment trading desk**: receivables you'd like to monetize early get quoted by partner banks in real time; sell with one click. (Adjacent, regulated.)
190. **CO2 footprint attribution from invoice line items**: line items get auto-mapped to carbon factors; quarterly reports for ESG.
191. **Invoice replay from blockchain-anchored hash**: if a customer's archive is lost, the published Merkle root + a few cooperating signatures can prove an invoice existed.
192. **"Did this customer get our invoice?" sniffer**: detect via email-tracking pixel + Peppol receipt + portal acknowledgement whether the customer's AP team opened it.
193. **Embedded e-invoicing literacy for end users**: explainer tooltips, video walkthroughs — embedded in our hosted portal so non-experts learn while using.
194. **"Invoice debugger"**: like Chrome devtools, but for an invoice — inspect any element, see which rule it violates, click to fix.
195. **Open data sharing for academic research**: opt-in tenant invoice metadata (PII stripped, statistically faithful) for university researchers studying B2B trade patterns.

---

## Pass F — Focus: "Operational excellence in long-term ops" (10 ideas)

196. **Disaster recovery runbook**: published, tested quarterly, customers can audit.
197. **Compliance certification audits public**: SOC2 / ISO27001 reports published on a `/trust` page.
198. **Public incident history with deep RCAs**: like Stripe / Cloudflare; transparency builds trust.
199. **Customer-facing SLA dashboards**: track customer's own performance against contracted SLAs.
200. **Rolling chaos testing**: production tier participates in chaos tests; resilience proven, not assumed.
201. **Onboarding success metric**: time-to-first-validated-invoice tracked, monthly target.
202. **Quarterly customer interview**: PM team runs scheduled interviews; published themes (privacy-preserving).
203. **Blameless postmortems published**: contributes to industry knowledge.
204. **Open architecture review board**: external advisory (security researchers, compliance officers) reviews changes.
205. **Time-bound support: 1-hour-acknowledge / 24-hour-resolve on paid tier**: SLA explicit, measured publicly.

---

## Phase 4 total count

210 (Phase 0) + ~190 surviving from critique + 50 (Phase 1 Round A new) + 20 (Phase 1 Round B new) + 50 (Phase 4 Pass A) + 30 (Pass B) + 30 (Pass C) + 20 (Pass D) + 15 (Pass E) + 10 (Pass F) = **425 distinct candidate ideas** (some overlap; ~380 distinct after dedup).

**Comfortably over the 300 target.** Phase 2 (dueling) and Phase 5 (Brenner) will refine. Phase 3 (codex/gemini in flight) will add external-perspective ideas.
