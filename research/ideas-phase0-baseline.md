# Phase 0 — Baseline Ideation (pre-market-research)

This is my own ultrathinking brainstorm before formal idea generation skills run. It is **not the final idea pool** — it's a high-quality seed that later phases can challenge, extend, or disprove. Numbered for tracking. Target: ~120 distinct ideas across axes.

## Axes / categories

We're separating ideas along these orthogonal axes to avoid clustering bias:

- **A. Core architecture** — how the thing is shaped technically
- **B. Format / standard handling** — XRechnung, Factur-X, UBL, Peppol BIS, CFDI, etc.
- **C. Intake / extraction (AI angle)** — getting data out of incoming docs
- **D. Rendering / templating (outbound)** — making beautiful, compliant, custom output
- **E. Network / transmission** — Peppol, national portals, point-to-point
- **F. Validation / compliance / audit**
- **G. Integration / DX (developer experience)** — making it actually easy to use
- **H. Monetization seam** — what's free vs paid
- **I. GTM / distribution** — how it reaches buyers
- **J. Community / OSS strategy**
- **K. Radical / out-of-the-box plays** — moonshots
- **L. Defensive moats** — what makes us hard to copy
- **M. Adjacent expansions** — products you could build on top of the core
- **N. Anti-features (deliberately NOT doing)** — what we refuse to do

---

## A. Core Architecture (1–15)

1. **WASM-native Rust core** — single artifact (`invoice_core.wasm`), runs in browser, Node, Bun, Deno, Cloudflare Workers, Vercel Edge, AWS Lambda, JVM via wasmtime, Python via wasmtime, Go via wazero. Kills "Node service" and "JVM only" complaints.
2. **EN 16931 semantic IR as the canonical model** — every format (Factur-X, XRechnung, UBL, FatturaPA, CFDI, etc.) is a serializer to/from this IR. Like LLVM IR for invoices.
3. **Plugin architecture for formats** — third parties can ship `@invoicekit/format-cfdi` as a separate package. We don't bundle every country.
4. **Pure-Rust no-std core** — runs even on embedded / WASM with no syscalls. All I/O at the edges.
5. **Async-by-default but no Tokio dependency** — use `futures` traits so the host runtime picks executor (Tokio, async-std, wasm-bindgen-futures, etc.).
6. **Zero-allocation hot paths** — for high-volume use cases, format conversion shouldn't churn the heap. Borrow-everything design.
7. **Streaming XML/JSON parsing** — quick-xml event-driven, never load full invoice into memory. Lets us handle 100MB batch envelopes.
8. **Capability-based API** — the library exposes `Capabilities` enum; if you only want validation, you don't link the rendering subsystem. Tree-shakable.
9. **Deterministic output** — same input always produces byte-identical output. Critical for signing & audit.
10. **Reproducible PDF generation** — fixed XMP metadata, fixed creation date placeholder, so the PDF hash is stable. Hashable invoices = audit-grade.
11. **Single canonical `.invoice` archive format** — open standard we publish: a directory or zip containing source PDF, extracted IR JSON, signature, validation report, transmission receipts. Self-contained, portable.
12. **CRDT-friendly IR** — JSON-CRDT representation so multi-party invoice editing (buyer-side annotations, AP corrections, supplier responses) can merge without conflicts.
13. **Built-in versioning of IR** — backward-compat-aware. We can introduce IR v2 without breaking v1 consumers.
14. **Engine vs UI strict separation** — core engine has no UI dependencies; UI is a separate crate. Lets framework wars (React vs Svelte vs Vue) happen at the boundary.
15. **Synchronous-first APIs** — async is opt-in. Most invoice operations don't need async; sync APIs are 10× simpler to use.

## B. Format / Standard Handling (16–30)

16. **Factur-X and ZUGFeRD as first-class** — German market is the immediate wave; we own this perfectly before expanding.
17. **All Factur-X profiles supported** (MINIMUM, BASIC WL, BASIC, EN 16931, EXTENDED, XRECHNUNG, CHORUS PRO).
18. **Automatic profile-up and profile-down conversion** — given EN 16931 data, render BASIC; given EXTENDED, downgrade to MINIMUM with explicit warnings on data loss.
19. **Cross-format conversion as a feature** — `convert input.xml --from xrechnung-3.0.2 --to ubl-2.1` works. We're the format universal translator.
20. **Round-trip lossless guarantee** — if it goes XRechnung → IR → XRechnung, the output is semantically identical (canonicalized). Tested via golden file harness.
21. **Validation rule registry as data** — Schematron rules, EN 16931 business rules, country CIUS as YAML/JSON, not hardcoded. Updatable without a release.
22. **Versioned compliance rulesets** — `compliance/de@2025.1`, `compliance/fr@2026.2`. Semver. Forkable. Community-maintained.
23. **Auto-detect input format** — given a PDF/XML/JSON, the engine sniffs the format (Factur-X embedded XML detection, namespace inspection, etc.).
24. **EDI legacy bridge** — EDIFACT INVOIC D96A → EN 16931 IR. Bridges legacy systems into the modern world.
25. **PEPPOL CIUS support** — we don't just speak EN 16931 generic; we speak each country's CIUS profile correctly out of the box.
26. **Format "diff" tool** — `invoicekit diff a.xml b.xml --semantic` ignores whitespace and key ordering, surfaces only semantic deltas.
27. **OASIS UBL 2.1 full implementation** — Order, Despatch Advice, Catalogue, etc., not just Invoice/CreditNote.
28. **Self-billing & credit notes as first-class** — many libs forget these.
29. **Recurring/installment invoice support** — common request, badly supported.
30. **Currency conversion logic per regulation** — Spain, Italy require specific exchange rate handling.

## C. Intake / Extraction (31–45)

31. **Progressive extraction pipeline** — Layer 1: embedded XML check (Factur-X). Layer 2: PDF text + heuristic regex. Layer 3: form-field detection (digital PDFs). Layer 4: local SLM (small language model, ONNX/WASM). Layer 5: cloud LLM fallback. Each layer reports confidence; pipeline stops when threshold met.
32. **Layer 1–3 fully local & free** — no API key needed for the vast majority of inputs.
33. **Browser-side extraction via WASM** — invoice never leaves the user's machine for the basic case. Privacy story = sales tool.
34. **Auditable AI: bounding-box citations for every extracted field** — every `invoice.supplier.name = "ACME GmbH"` comes with `source: {page: 1, x: 50, y: 120, width: 200, height: 18, model: "donut-invoice"}`. Reviewers can click any field and see where it came from.
35. **Field-level confidence scores** — UI can prioritize human review on low-confidence fields.
36. **Active learning loop** — user corrects a field → correction logged → if user opts in, fine-tunes a local LoRA adapter. The system gets better at YOUR invoices.
37. **PII redaction before cloud LLM fallback** — automatically mask account numbers, IBANs before sending to OpenAI/Anthropic if user uses cloud fallback.
38. **Differential 3-way match** — invoice ↔ purchase order ↔ goods receipt. Explainable, auditable, with reason codes.
39. **Multi-page batch invoice handling** — bank statement, fax, scanned batch → split into individual invoices.
40. **Email intake adapter** — IMAP/EWS/Gmail listener that pulls invoices from `accounting@yourcompany.com`, extracts them, files them.
41. **Receipt/expense report support** — same engine, looser schema. Expense reports are 80% of small businesses' "invoicing" problem.
42. **Bilingual / multilingual extraction** — labels in German, French, Italian, Spanish, Polish all map to the same IR field.
43. **Handwriting OCR** — for residual paper invoices in SMB world. Lower priority but a wedge for some markets.
44. **Anomaly detection** — flag invoices with suspicious patterns: round numbers, weekend dates, unusual suppliers, possible duplicates. Fraud-detection-lite.
45. **"Looks like an invoice" detector** — for AP teams ingesting a mixed inbox, classify what's actually an invoice vs spam vs marketing vs PO confirmations.

## D. Rendering / Templating (46–60)

46. **Templates as JSX/TSX** — render to PDF AND accessible HTML AND email-safe HTML from the same template source.
47. **WCAG-accessible HTML5 invoice option** — public-sector buyers need this; nobody offers it well.
48. **Pixel-perfect deterministic PDF** — fixed font subsetting, deterministic font hinting, no system fonts. Two renders = identical bytes.
49. **Bring-your-own-fonts pipeline** — automatic font subsetting & embedding for compliance (PDF/A-3).
50. **Theme tokens system** — `theme.json` defines colors, spacing, typography; templates reference tokens. Same template, infinite brands.
51. **Visual template editor in browser** — WYSIWYG, outputs the JSX template. Optional, separate package.
52. **AI-generated templates from a screenshot** — paste an image of a target invoice layout, get a template scaffold. Genuinely useful for ERP migrations.
53. **Templates respect country layouts** — German invoices typically show VAT differently than French/Italian; presets handle this.
54. **Embedded QR codes (EPC, SEPA, Swissbills QR-bill, Polish split-payment)** — first-class support.
55. **Multi-language invoice rendering** — same data, render in DE/EN/FR/IT for cross-border use.
56. **Right-to-left support** — Arabic invoicing (Saudi ZATCA) is a real market.
57. **Plain-text & ASCII fallback** — for email-only or terminal-rendered invoices.
58. **PDF/A-3 + embedded XML in one call** — `render(ir, {profile: "factur-x-en16931", template: "modern"}).toPdfA3()`. The most-needed combo, hardest to find as one call.
59. **Watermarks, drafts, copies, voids** — first-class lifecycle states reflected in the PDF.
60. **Signature placement & visible signature box** — for qualified electronic signature workflows.

## E. Network / Transmission (61–75)

61. **Peppol AS4 client as a Rust library** — first non-JVM open-source AS4 client. Embeddable in any runtime.
62. **Peppol AP managed service** — pay-per-envelope, no setup, no certificates to manage. The Stripe model.
63. **Self-host AP option** — same code path, run it yourself if you've got the regulatory chops.
64. **Italian SDI gateway** — direct integration with Agenzia delle Entrate.
65. **Polish KSeF gateway** — direct integration with the Polish portal.
66. **Indian IRP gateway** — get IRN/QR back instantly.
67. **Saudi ZATCA Phase 2 integration** — real-time clearance.
68. **French Chorus Pro & PPF gateway** — when France finally launches.
69. **Mexican CFDI PAC integration** — through an existing PAC partner.
70. **Email-based "fallback" delivery** — when Peppol fails or recipient isn't on the network, send PDF+XML attachment via email with delivery receipt.
71. **Universal addressing service** — `invoicekit lookup-recipient ACME-GmbH-DE12345678` returns Peppol ID, fallback email, preferred format.
72. **Webhooks for transmission events** — `invoice.transmitted`, `invoice.acknowledged`, `invoice.rejected`. Lets SaaS apps surface state to users.
73. **Retry & dead-letter queue built-in** — managed service handles flaky national portals.
74. **Idempotency keys** — never double-send. Critical for financial documents.
75. **Outbox pattern client library** — atomic "save to DB + queue for send" so users don't have to invent it.

## F. Validation / Compliance / Audit (76–90)

76. **Single `validate()` call returns all violations across all applicable rule sets** — schema, schematron, business rules, country CIUS, profile-specific.
77. **Validation as a service** — drop a file at our API, get a free validation report. No signup. Lead-gen.
78. **Validation reports as machine-readable JSON** — each violation has rule ID, location (XPath / line / character), severity, message, fix suggestion.
79. **"What format am I missing for jurisdiction X" tool** — given an invoice, tells you what country mandates it doesn't yet meet.
80. **Audit log as event sourcing** — every state change of an invoice is an immutable event. Replayable, court-admissible.
81. **WORM archival service** (paid) — 10-year retention in Object Lock S3 / Azure WORM blob, with cryptographic timestamp.
82. **Cryptographic signatures support** — qualified electronic signature (QES) via integrations with EU eIDAS trust service providers.
83. **Hash chain across documents** — successive invoices form a hash chain (like Spain Verifactu); auditors love this.
84. **Tax-engine integration** — pluggable rule packs for VAT computation per jurisdiction. We don't build a tax engine; we expose hooks.
85. **PCI-DSS scope minimization** — never accept card data. Make this explicit.
86. **GDPR / data-residency switches** — config option to enforce EU-only processing.
87. **SOC2 + ISO27001 from day 1 for managed services** — table stakes for enterprise.
88. **Right-to-be-forgotten flow** — anonymize invoices for past customers while preserving aggregates.
89. **Tamper-evident PDF** — invisible watermarking with hash; can detect modification.
90. **Public ledger for invoice authenticity** — optional: publish a Merkle root daily to a public blockchain or git repo. Anyone can verify an invoice existed at a given date.

## G. Integration / DX (91–105)

91. **`npx invoicekit` works first try** — no account, no API key, generate a sample Factur-X with one command.
92. **One-line embed**: `<script type="module" src="https://cdn.invoicekit.org/v1/embed.js"></script>` gives a browser an invoice-validator widget.
93. **TypeScript-first SDK** — generated types from EN 16931 schemas; full intellisense.
94. **Python SDK with strict types via pydantic**.
95. **Go SDK** generated from a single OpenAPI spec.
96. **Java SDK** for the legacy enterprise market — they're not leaving JVM.
97. **CLI with verb-noun grammar**: `invoicekit convert / validate / send / render / extract`.
98. **Postman / Bruno collection** for every API endpoint.
99. **OpenAPI 3.1 spec** for the managed services.
100. **Code playground at docs site** — paste an invoice, see it parsed, run conversions, all in browser via WASM.
101. **Storybook for templates** — every template viewable, customizable, exportable.
102. **Hot-reload dev mode for templates** — change template, see PDF rerender instantly.
103. **VSCode extension** — paste XML, see preview; click validation errors, jump to source.
104. **GitHub Action** — `uses: invoicekit/validate@v1` to validate invoices in PRs.
105. **AI agent SDK** — drop-in for LangChain/LlamaIndex/Claude tools, so AI agents can issue/validate invoices.

## H. Monetization Seam (106–120)

106. **OSS core MIT-licensed forever** — format conversion, validation, rendering, local extraction. Build trust.
107. **Paid: Peppol AP envelope-by-envelope**, $0.05–0.20 per envelope, undercutting incumbents.
108. **Paid: country gateways (SDI, KSeF, IRP, ZATCA, PPF)** — pay per submission or monthly subscription.
109. **Paid: certificate management** — buy/manage qualified seals on your behalf.
110. **Paid: cloud LLM extraction fallback** — pay per page only for hard cases.
111. **Paid: WORM archival** — per-GB-per-year.
112. **Paid: SLA support tier** — for embedders who need response time guarantees.
113. **Paid: hosted templates marketplace** (% take rate from designers).
114. **Paid: managed validator API at high QPS** — free tier rate-limited.
115. **Paid: enterprise on-prem licensing** — same code, with support, indemnification, SLA.
116. **Paid: training & certification** — official "InvoiceKit Certified Integrator" cred.
117. **Anti-pattern: no per-seat pricing for core OSS** — only network / managed services have metered pricing.
118. **Anti-pattern: no feature gating in OSS core** — if it's in the codebase, it's free forever.
119. **Anti-pattern: no "phone home" telemetry without opt-in**.
120. **Bountied feature pipeline** — sponsors can fund specific country support, get logo on the page, no IP capture.

## I. GTM / Distribution (121–135)

121. **Launch with a viral free tool** — paste-an-invoice-get-it-validated, totally free, no signup, embedded in every Google search for "validate xrechnung".
122. **SEO play: own "[country] e-invoice mandate developer guide"** — write the canonical tutorial for every jurisdiction.
123. **Show HN with the WASM playground** — concrete demo: "validate your invoice without uploading it" beats any pitch.
124. **r/devops, r/SaaS, r/Odoo, r/ERP launch posts** with real value (not just promotion).
125. **DevTo / Medium guide series** — "Building Factur-X in [language]" tutorials.
126. **Sponsor TWiR, JavaScript Weekly, Node Weekly, Bytes, etc.** — developer newsletter co-branding.
127. **Conference talks** — JSConf, RustConf, Posit / PyData, EuroPython, FOSDEM (FOSDEM has a tax/finance dev room).
128. **OSS Maintainer community** — get adopted by invoiceninja, Odoo, ERPNext, sevDesk as their "preferred" e-invoice engine.
129. **Trade-show presence at e-invoicing conferences** — E-Invoicing Exchange Summit, accounting / ERP shows.
130. **Partnership with national authorities** — get listed as a "compliant solution" on KSeF, KoSIT, IMDA, etc. pages.
131. **Direct outreach to ERP partners** — German Microsoft Dynamics partners need this NOW.
132. **Reverse pitch**: open-source the e-invoicing for big incumbents (Pagero, Comarch) — they need a Peppol AP they can deploy on-prem at clients; we are the only one in their stack that's not their competitor.
133. **YC / Tinyseed / Calm Company application** — narrative is "Stripe for invoicing" and the regulatory tailwind.
134. **Big4 partnerships** — Deloitte/EY/KPMG/PwC implementation arms; we become the recommended engine they staff.
135. **Local consultant network** — Steuerberater (DE) / expert-comptables (FR) directories; offer training & referral fees.

## J. Community / OSS Strategy (136–150)

136. **Permissive license (Apache 2.0 or MIT)** — not AGPL, despite the temptation. Adoption > control.
137. **Contribution-friendly repo** — every issue has a "good first issue" backlog, every test has a fixture, scripts to add a new country.
138. **OpenAPI + spec-first development** — schemas are the contract.
139. **Public roadmap on GitHub Projects** — radical transparency.
140. **Per-country maintainer model** — community members can become the "Polish KSeF lead", "Saudi ZATCA lead", get credit, badges, lifetime commercial license.
141. **Bounties for country support** funded by sponsors who need it.
142. **RFC process for IR evolution** — public proposals, discussion, accept/reject.
143. **Annual community summit** (online) — talks, working groups.
144. **Working group with EU CEF / OpenPeppol AISBL participation** — credibility play.
145. **Reference test suite as standalone OSS project** — anyone can verify their lib against our tests, including competitors. Builds ecosystem credibility.
146. **Public benchmark suite** for extraction accuracy — anyone can run, publish results.
147. **Docs site with country pages** — every country gets a deep "how e-invoicing works in [country]" page that ranks in Google.
148. **Localization team** — docs in DE/FR/IT/ES/PL at minimum.
149. **Code of conduct + governance doc** from day 1.
150. **CLA-light**: use DCO instead of CLA to lower friction.

## K. Radical / Out-of-the-Box (151–170)

151. **Invoice-as-code DSL** — declarative TypeScript/YAML: `invoice({ supplier, customer, lines })`. Versioned in git like infra-as-code.
152. **GitOps for invoicing** — PR a new invoice, CI validates, merge triggers transmission. Auditable AP/AR via git.
153. **Invoice "lockfile"** — `invoices.lock.json` records emitted invoices, hashes, transmission receipts, archival URIs. Like `bun.lock` for AR.
154. **AI agent that handles AP autonomously** — receives invoice, validates against PO, queues for approval, posts to ERP, all explainable.
155. **Invoice mediation marketplace** — disputes about invoices get arbitrated by neutral validators using cryptographic evidence (the invoice was tamper-evident, the transmission was signed).
156. **Smart-contract-backed invoice escrow** (optional, opt-in for crypto-native buyers) — never required but available.
157. **"Pay with QR" universal QR** — single QR code carries Factur-X data + EPC SEPA pay + crypto pay. Recipient picks payment method.
158. **Invoice marketplace / factoring SDK** — issued invoice can be tokenized and sold for instant cash. We provide the rails (no liquidity, partner banks bring that).
159. **AI-driven negotiation agent** — buyer's AI talks to seller's AI to dispute, schedule payment, request discount. Sounds gimmicky but quietly enormous.
160. **Reverse Stripe**: instead of "you charge me," let buyers initiate "I will pay this invoice on day X" promises, signed cryptographically. Programmable receivables.
161. **Invoice trust score** — public registry of supplier reliability based on validated past transactions (with privacy-preserving aggregates).
162. **Browser-extension AP automation** — Chrome extension that watches your inbox/portal logins, extracts invoices automatically, never leaves your machine.
163. **Local-first invoicing app on Tauri/Electron** — runs entirely offline, syncs via CRDT to other devices. For SMBs / freelancers in low-trust environments.
164. **Sat-comm fallback for transmission** — for businesses operating in low-connectivity regions, queue & transmit when online (already common; we make it library-grade).
165. **Differential privacy in benchmarks** — aggregate "what's a normal SaaS invoice in Germany" without exposing any single tenant.
166. **Federated learning for extraction models** — many companies' invoices improve a shared model without sharing data.
167. **"Universal invoice GUID" (UIG)** — every invoice gets a globally unique ID hashable from its content. Useful for dedup, fraud detection, audit.
168. **Invoice REPL** — interactive shell: `> validate invoice.xml`, `> show .supplier`, `> set .currency EUR`. Power-user DX.
169. **Audit-replay tool** — given an invoice + receipts + signatures, produce a court-ready PDF/A-3 dossier with every step explained.
170. **Invoice schema as protobuf + cap'n proto** — for binary high-volume use cases (banks, big EDI).

## L. Defensive Moats (171–185)

171. **Network effect via the canonical IR** — if every ERP / SaaS integrates with our IR, switching cost grows.
172. **The free validator becomes the trust default** — like SSL Labs for TLS; everyone links to ours.
173. **Country compliance ruleset community** — we own the largest, most actively maintained ruleset library. Forking is hard because of community gravity.
174. **Performance moat** — we're 10× faster than mustangproject; that math compounds at high volume.
175. **WASM delivery shape** — first-mover. Once we own "the invoicing lib that runs in browser," competitors must rebuild from scratch.
176. **Brand & trust** — invoice tooling is a "boring but mission critical" market; trust takes years to build, then becomes immovable.
177. **Spec leadership** — be the de facto reference implementation. The way Stripe steered the payments API standard.
178. **Data network effect on extraction** — if customers opt-in, we collect fingerprints of common supplier invoice layouts; everyone gets free auto-extraction for those layouts.
179. **Regulator relationships** — once you're certified by 5+ national authorities, that takes ages to replicate.
180. **OSS adoption as moat** — by the time competitors realize, we're inside every billing SaaS.
181. **"Audit-grade" positioning** — we're not "AI-powered invoicing", we're "the only AI extractor that can defend itself in court." Different market.
182. **Hardware-attested signing** — leverage Apple/Google Trusted Execution / TPM for signatures. Hard to replicate without OS-level partnerships.
183. **Tax-firm partnerships** — once the Big4 standardize on our format, switching is enterprise-level.
184. **Education moat** — best docs, courses, certifications. Becomes a hiring signal.
185. **The lock-in is the standard, not the code** — once your invoices are emitted in our IR + canonical archive format, you can leave but you keep using our format. Like Git's data model survives even if you switch hosts.

## M. Adjacent Expansions (186–200)

186. **Expand to purchase orders** — same IR shape, different document type. PO-Invoice matching is a $$$ AP problem.
187. **Expense reports** — receipts, same engine, looser schema.
188. **Goods receipts / despatch advice** — completes the 3-way match story.
189. **Catalogs** — Peppol catalog format, supplier price lists.
190. **Contracts as a related artifact** — invoices reference contract IDs; we host & version contracts.
191. **VAT registration verification (VIES) integration** — built-in.
192. **Bank reconciliation** — match received payments to issued invoices via CAMT.053/MT940.
193. **SEPA Direct Debit mandate management** — adjacent to AR.
194. **Subscription / metered-billing engine** — compete with Stripe Billing, Lago, Metronome; we're the e-invoice-correct alternative.
195. **Tax filing helpers** — VAT return generation from invoice corpus.
196. **Compliance attestations for SaaS** — generate "SOC for compliance" reports.
197. **Payment orchestration** — combine invoice issuance with multiple payment rails (SEPA, ACH, card, crypto, BACS).
198. **Embedded factoring & financing** — partner-provided, we are the rails.
199. **Procurement-side workflows** — punchout catalogs, requisitions, approvals.
200. **Government grants / subsidy invoicing** — special schemas for B2G with detailed line items.

## N. Anti-features (deliberately NOT doing) (201–210)

201. **No CRM** — we don't manage customers; we integrate with HubSpot, Salesforce, Pipedrive.
202. **No ERP** — we don't replace SAP/Oracle; we feed them.
203. **No tax engine** — we expose hooks; you bring Avalara, TaxJar, Vertex.
204. **No "all-in-one" small business app** — that's invoiceninja's market; we're infrastructure.
205. **No banking** — no Open Banking integration directly; partner only.
206. **No support for proprietary EDI VAN networks** unless required for legacy bridge — Peppol is the future.
207. **No proprietary lock-in via custom XML extensions** — only standards.
208. **No closed-source plugins for the OSS core** — must be releasable; commercial extensions live in our managed services.
209. **No telemetry by default** in the OSS core.
210. **No "AI mode" for outbound generation** — AI is for intake. Outbound is deterministic, always. Composability beats magic for the issuer side.

---

## Notes on overlap & exclusivity

Several ideas form **forks** the user may need to decide between:

- **Fork F1: ICP**: Developer-first OSS (1) vs Enterprise CFO sales-led (2). Marketing/product diverge.
- **Fork F2: Geographic focus**: EU-first vs Global-from-day-1.
- **Fork F3: Network monetization**: Own Peppol AP (riskier, deeper moat) vs partner with existing APs (faster, thinner margin).
- **Fork F4: AI placement**: AI as core feature with bold marketing vs AI as supporting capability with quiet positioning.
- **Fork F5: Outbound vs Inbound emphasis**: Issuer-side first (templating, format conversion) vs Receiver-side first (OCR, AP automation).
- **Fork F6: License**: MIT/Apache (max adoption) vs SSPL/BSL (control + future commercial flex).
- **Fork F7: Delivery shape primary**: Library/SDK first vs Hosted API first.

Will revisit after market research returns concrete signal on each.

---

End Phase 0 baseline. ~210 distinct ideas plus 7 explicit forks for downstream decision-making.
