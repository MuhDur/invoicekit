# Buyer Personas & ICPs — E-Invoicing Toolkit

**Research date:** May 2026
**Product framing:** Developer-first, library-shaped (not platform-shaped) e-invoicing toolkit. OSS core + paid managed services (Peppol AP transmission, country gateways, certificates, archival, optional cloud LLM fallback).

**Market backdrop (mid-2026):**
- Global e-invoicing market: ~$29.8B (2026), CAGR ~20% to $60–70B by 2030–2034.
- EU ViDA adopted 11 March 2025; intra-EU B2B DRR mandatory July 2030.
- Country-level mandates: Italy (live), Belgium (Jan 2026 live), France (Sep 2026), Poland (Feb–Apr 2026), Germany (issuance from Jan 2027 for >€800k turnover; full Jan 2028), Spain (Jan 2027), Netherlands (2030 → 2032 domestic e-reporting).
- ~400+ accredited Peppol service providers globally; 100+ accredited PDPs in France alone.
- Direct Peppol AP membership: ~€1,800/yr + audits + ongoing operational cost — prohibitive for small vendors. This is the wedge.

---

## 1. Master Persona Matrix

| # | Persona | Realistic ACV | Buying motion | TAM (EU-centric) | Channel difficulty | Strategic fit (1–5) |
|---|---|---|---|---|---|---|
| 1 | Embedded dev @ ERP/billing SaaS vendor | €15k–€150k/yr (platform + per-doc) | Bottom-up dev → CTO/Head of Product sign-off | ~3,000–5,000 ERP/SaaS vendors in EU; ~500 are real targets | Medium (GitHub, dev conferences, partnership BD) | **5** |
| 2 | Embedded dev @ e-commerce platform (Shopify/Woo app) | €5k–€40k/yr | Self-serve, PLG | ~5,000 active EU-focused commerce app authors | Medium (app store SEO, marketplace listings, HN) | **4** |
| 3 | In-house engineer @ midmarket B2B SaaS | €3k–€20k/yr | Self-serve → 1 approval (eng manager) | ~50,000–80,000 EU B2B SaaS with €1M+ ARR | Easy (HN, blog/SEO, GitHub) | **5** |
| 4 | Engineer @ fintech/billing startup (Lago, Maxio class) | €25k–€200k/yr (OEM/embed) | Founder/CTO + procurement | ~150 serious global billing-platform startups | Hard (direct outbound, partnerships) | **4** |
| 5 | AP/AR ops engineer @ corporate (1000+) | €50k–€300k/yr (inbound capture + match) | Eng → Finance Director → CFO/CIO; RFP common | ~15,000 EU corporates >1000 FTE | Hard (RFP, sales cycle 6–12mo) | **2** |
| 6 | Accountant/fiduciary (Steuerberater / expert-comptable / CPA) | €500–€5k/yr per practice, but white-label OEM €50k+ | Practice partner; trade-association-driven | ~100,000 EU practices | Hard (trade journals, language-local) | **2** |
| 7 | DevOps/platform team @ unicorn (PCI + e-invoice multi-country) | €100k–€500k/yr | Eng VP → CFO/CISO; multi-stakeholder | ~300 unicorns/late-stage scale-ups in EU+US | Hard (warm intros, ABM) | **3** |
| 8 | Government IT contractor / agency dev | €100k–€2M (project) | Tender / framework agreement | ~50 ministries × ~30 countries in scope | Very hard (years-long sales cycle) | **1** |
| 9 | OSS maintainer of adjacent project (Invoice Ninja, Crater, Akaunting, ERPNext) | €0 direct; ecosystem leverage | Community contribution / co-marketing | ~30 meaningful projects | Easy (GitHub PRs, Discord) | **5** (force multiplier) |
| 10 | Indie hacker / solo dev shipping niche app | €0–€500/yr (free tier) | Self-serve, viral | ~50,000 indie devs touching invoicing | Easy (HN, Twitter/X, Indie Hackers) | **3** (acquisition funnel) |
| 11 | Tax-tech / compliance consultant (independent) | €2k–€15k/yr (tooling) | Solo decision | ~5,000 specialists EU+US | Medium (LinkedIn, Big4 alumni networks) | **2** |
| 12 | Big4 / boutique consulting practice | €100k–€1M/yr (volume license) | Partner sponsor → procurement | ~50 firms with serious EU practice | Very hard (long-cycle BD) | **2** |
| 13 | ERP systems integrator (Deloitte/Accenture implementation arm) | €50k–€500k/yr | Practice lead → procurement | Same ~50 firms × multiple practices | Hard (partner programs) | **3** |
| 14 | Treasury/Finance Ops @ multinational | €200k–€1M/yr | CFO/Treasurer; RFP | ~5,000 true multinationals (>50 countries) | Very hard | **1** |
| 15 | EDI specialist (legacy operator adding Peppol) | €30k–€150k/yr | Tech lead → COO | ~500 mid-size EDI VANs/operators globally | Medium (EDI trade media, association) | **4** |
| 16 | Bank / PSP (offers invoicing as merchant value-add) | €100k–€1M/yr (OEM/revshare) | Product head → BU GM → procurement | ~200 EU banks + ~100 PSPs that have merchant programs | Very hard (12–24mo sales cycle) | **3** |
| 17 | Government / public-sector body | €50k–€500k (procurement) | Tender | Hundreds of agencies | Very hard | **1** |

**TAM (rough, EU + adjacent, addressable for our shape of product):** ~€2–4B/yr in software/managed services spend on e-invoicing-toolkit-shaped problems. We won't win all of it; we should target €30–80M/yr realistically reachable in 5 years if we win Personas 1, 2, 3, 9.

---

## 2. Per-Persona Deep Dive

### Persona 1 — Embedded developer at ERP / billing SaaS vendor

**Who they are.** Senior engineer or tech lead at Odoo partner, Microsoft Dynamics partner, Sage extension house, Lexware/lexoffice (DE), sevDesk (DE), Pennylane (FR), Pleo, Holvi, Qonto, Penta, Spendesk. Company size 20–500 FTE, €5M–€100M ARR. Geographies: DACH, France, Benelux, Nordics, Iberia, UK, Australia, Singapore. Often a "country localization engineer" or a small team owning compliance modules.

**JTBD.** "Ship country-correct e-invoicing inside our product without becoming an e-invoicing company. Make it work in 7 countries by next quarter. Don't make the customer pay per invoice if I can avoid it. Pass certification audits."

**What they use today.**
- Storecove (Netherlands, API-first, ~€495/mo entry, partner/white-label tier) — most popular partner choice in Benelux/Nordics.
- e-invoice.be (€0.25/invoice, pay-as-you-go, popular with smaller ISVs).
- Pagero / Tickstar / Tradeshift / Basware — heavier enterprise vendors, "too expensive at our margin."
- DDD Invoices, Peppox, Recommand, B2BRouter — emerging API-first competitors.
- Microsoft's unified e-invoicing integration framework (2026wave1) — decouples Dynamics from any single ASP; **a tailwind for us**.

**Complaints (verbatim themes from HN, GetApp, partner forums).**
- "Sales call required for pricing" (Pagero, Sovos, Avalara).
- "Per-document fees kill our margin at scale."
- "Multiple country gateways each with own SDK; we glue 6 vendors together."
- "Vendor lock-in: certificates and AP are theirs; if we leave we lose history."
- "Compliance team can't audit closed-source vendor SDKs."

**Budget & buying.** PLG self-serve at the trial/POC level (free OSS pulls them in). At commercial scale, **CTO + Head of Product sign-off**, occasionally CFO. Annual contract, often with white-label/co-brand and per-document overage. Procurement involvement light unless deal >€100k.

**Switch triggers.** (a) OSS core so they own the in-product code path; (b) Peppol AP transmission priced per envelope, not per invoice value; (c) clean SDK in their stack (TS/Python/PHP/Java/.NET); (d) format coverage: Peppol BIS 3, XRechnung, ZUGFeRD/Factur-X, FatturaPA, SdI, Chorus Pro/PDP, NLCIUS, KSeF (PL), SAF-T extensions, LATAM CFDI/NF-e bonus; (e) certified AP we operate for them; (f) Stripe-quality docs.

**ACV.** €15k–€150k/yr. Top end if they need multi-country AP transmission + white-label support.

**Where to find them.** GitHub (Peppol, e-invoicing topics — already active community), Dynamics Community, Odoo Experience conference, SAP TechEd, dev.to, HN, niche subreddits (r/Dynamics365, r/odoo, r/sap, r/msp), Microsoft AppSource partner programs, OpenPeppol Working Group meetings, EESPA, FNFE (FR), FeRD (DE).

**TAM.** ~500 high-value targets in EU; if 5% adopt at €40k median ACV → €1M ARR. Including LATAM/APAC ERPs as ViDA-adjacents pushes it higher.

---

### Persona 2 — Embedded developer at e-commerce platform / app

**Who they are.** Shopify app author, WooCommerce plugin developer, Shopware/PrestaShop extension dev, Salesforce Commerce Cloud cartridge dev, BigCommerce app dev. Often a 1–10 person studio (e.g., Latori, Web-Vision, EAS, Empact, Billova, SimpleVAT). Geography: heavily DACH, NL, IT, FR, ES, UK, with a long tail of one-person shops.

**JTBD.** "Generate country-correct B2B/B2C invoices for the merchants installing my app. As mandates roll in, my app sells itself; if I miss the deadline I lose the merchant to a competitor app."

**What they use today.** They glue: Storecove or e-invoice.be API + their own XML generation + Shopify Order webhook + a PDF library. Many wrote their own UBL emitter (PHP/Node). Pain points are nailing edge cases in B2C/B2B mixed flows, OSS reverse-charge, IOSS thresholds.

**Complaints.**
- "VIES validation flaky, no good library that just works."
- "Every country adds a flavour of UBL; conformance hell."
- "Storecove monthly minimum painful when I'm starting an app at €19/mo MRR."
- "Validation rules in the Peppol BIS spec are dense; rounding bugs caught only by AP rejection."

**Budget & buying.** PLG self-serve. Credit card. Usage-based with generous free tier wins. Decision in <1 day if there's a working sandbox.

**Switch triggers.** (a) Free tier to dev/test/build; (b) per-merchant or per-doc pricing they can mark up; (c) revshare on managed AP; (d) Shopify/Woo example snippets in repo.

**ACV.** €5k–€40k/yr when they hit scale; many sit at €100–€500/mo. The aggregate (across the app's merchants) is what makes them valuable.

**Where to find them.** Shopify App Store partner Slack, WooCommerce.com, Shopware Community Day, HN, dev.to, Twitter/X, /r/shopify, /r/woocommerce, /r/webdev.

**TAM.** ~5,000 EU-relevant app authors today; growing as mandates force every commerce merchant to need this. ~€20–50M aggregate addressable.

---

### Persona 3 — In-house engineer at midmarket B2B SaaS

**Who they are.** One engineer at a 30–300 FTE B2B SaaS doing €5M–€50M ARR. Title is usually "Senior Engineer, Billing" or "Tech Lead, Platform." Boss is the CTO. They were handed a Notion doc that says "we must e-invoice German and French customers by [date]" and given 6–12 weeks.

**JTBD.** "Make our existing Stripe-or-similar invoicing emit a Peppol/XRechnung/Factur-X-compliant invoice and route it correctly to the recipient. Not become an e-invoicing expert. Not break the existing PDF flow."

**What they use today.** Stripe Invoicing (no native Peppol). They cobble: a UBL generator (Php-EN16931 library, or DotNet equivalent), Storecove or similar for transmission, in-house archival. Many simply send PDFs and hope the customer's PEPPOL gateway tolerates it (it won't, after mandates lock in).

**Complaints.**
- "Stripe doesn't support EN 16931."
- "I just want a function `generate_xrechnung(invoice) -> bytes`."
- "Why are there 6 vendors to know about (Storecove, Pagero, B2BRouter, e-invoice.be, Tradeshift, Pleo)? Which one is right?"
- "Pagero quoted me €30k/yr minimum. Our compliance budget is €5k."
- "Lago Blog hit the nail on the head: 'billing systems are hard'."

**Budget & buying.** **PLG bullseye.** Self-serve, credit card, occasional eng manager sign-off. €5k–€20k/yr is the sweet spot. They prefer "library + small managed-AP fee" over "platform subscription."

**Switch triggers.** (a) "pip install / npm install / cargo add and you're done"; (b) types/schema in their language; (c) optional managed AP that costs <€0.10 per envelope at scale; (d) explicit migration path off Stripe Invoicing; (e) blog post that ranks for "Stripe XRechnung."

**ACV.** €3k–€20k/yr. Reliable, fast cycles.

**Where to find them.** HN (front-page-able for a good Show HN), r/SaaS, r/programming, r/devops, /r/Steuern (DE founders), Indie Hackers, X (#buildinpublic), engineering blogs, dev.to, Lobsters.

**TAM.** ~50,000–80,000 EU B2B SaaS with this problem. Even 0.5% adoption at €8k ACV → €2–3M ARR. **This is our highest-volume self-serve channel.**

---

### Persona 4 — Engineer at fintech / billing startup

**Who they are.** Engineer at Lago, Maxio, Orb, Schematic, Solvimon, Metronome, Stigg, Zenskar, Recurly, Chargebee, RevenueCat, Polar.sh, Stripe-competitor. Mostly Series A–C, €5M–€50M ARR, 30–200 FTE. Focused on usage-based billing. Today most have no real e-invoice support; PDF only.

**JTBD.** "Bake EN 16931 / Peppol / country-specific into our billing engine so we can sell into European mid/upper-market without losing to local incumbents (Pennylane, sevDesk)."

**What they use today.** Mostly nothing; they generate PDFs and let the customer's tax tooling deal with it. Some integrate with Pagero/Sovos but feel the lock-in.

**Complaints.**
- "We're 'Stripe but better'. Adding Peppol natively requires hiring a person who knows it."
- "If we partner with Pagero we look like a thin wrapper. If we build it we burn 4 engineer-quarters."

**Budget & buying.** Founder/CTO decision. **OEM/embed pricing**: per-account or per-doc with revshare. Procurement light, but they'll do legal review for indemnity (compliance is regulatory risk).

**Switch triggers.** (a) Embed/white-label terms; (b) "managed but commodity" AP — they don't want to own certificates; (c) AGPL/MIT split (commercial-friendly OSS license); (d) liability shifting (we hold the AP certification, indemnify them); (e) responsive engineering team they can partner with.

**ACV.** €25k–€200k/yr depending on volume and white-label depth.

**Where to find them.** Direct outbound (Lago and friends are open about who they are), HN, billing-engine-builder Discords (Schematic's, Orb's), Solvimon/Alguna blog content circles, YC W26/S26 batches with billing companies.

**TAM.** Small in count (~150 globally) but high ACV. Two-three of these as anchor design partners = the OSS project's credibility.

---

### Persona 5 — AP/AR ops engineer at 1000+ FTE corporate

**Who they are.** Engineer or Sr. Manager in shared-services/AP automation at a German MDAX, French CAC mid, Italian industrial group. Title: "AP Automation Lead", "Solution Architect, P2P." They process 100k–2M invoices/yr inbound.

**JTBD.** "Capture inbound invoices in any format (Peppol XML, Chorus Pro CII, FatturaPA, paper, email PDF), normalize, 3-way match against PO + GR, route to ERP. Outbound is easy; inbound is the cost center."

**What they use today.** Tipalti, Stampli, AvidXchange, Coupa, SAP Ariba, Tradeshift, Basware, OpenText, Esker. They paid €200k–€2M to implement. They complain bitterly (G2 reviews are explicit: "25% of invoices still manual," "no help during onboarding," "clunky," "slow support," "supplier network fees feel like a tax").

**Complaints.**
- Per-supplier "network fees" (Ariba) feel extortionate.
- Hard-coded country logic; can't tweak.
- Vendor support is bad; implementation 6–12 months.
- OCR/IDP for paper still mediocre.

**Budget & buying.** **Procurement + CFO + CIO**. RFP-driven. 6–12 month sales cycles. They will not buy an OSS library for production AP unless wrapped in a managed contract with SLA + indemnity.

**Switch triggers.** Honestly: rarely. They've sunk cost. The wedge is when an existing contract renews and the vendor raises 30%, or when a new mandate forces them to add a country fast.

**ACV.** €50k–€300k/yr but with sales cycles that burn cash.

**Where to find them.** Gartner P2P Magic Quadrant references, IOFM conferences, SAPinsider, SharedServicesLink, EESPA. LinkedIn DM is viable.

**TAM.** Big on paper, brutal in practice for a developer-first OSS shop. **Avoid as primary ICP** — they will warp us into Coupa Lite.

---

### Persona 6 — Accountant / fiduciary (Steuerberater / expert-comptable / CPA)

**Who they are.** Owner-partner at a 3–30 person practice serving 50–500 SMB clients. Older average age. Geography: DE/AT/CH dominate (Steuerberater + DATEV ecosystem), FR (expert-comptable + Cegid/Sage/Pennylane), IT, BE/NL, ES, UK, US.

**JTBD.** "Make sure every one of my clients sends and receives e-invoices in compliant format, that everything lands in my bookkeeping system (DATEV / Cegid / Loop / Pennylane), and that I don't have to learn 17 new apps."

**What they use today.** DATEV (DE, dominant), Cegid (FR), Sage, Loop, Pennylane. DATEV opened its E-Rechnungsplattform; third-party API hooks landed mid-2026. Alternative DE entrants: Norman, Accountable, sevDesk, BuchhaltungsButler, lexoffice, FastBill.

**Complaints.**
- DATEV is expensive, slow, awkward UX, but switching cost colossal.
- Multi-client master data sync (email-for-Peppol delivery per client) tedious.
- Clients send invoices in 5 formats; practice has to normalize.

**Budget & buying.** Practice partner buys; trade-association (DStV in DE, OEC in FR) influences strongly. Conservative, brand-driven, language-local. **Not a direct buyer of dev libraries.** But they are a *channel*: if we OEM into an accountant-facing SaaS that serves them, we win indirectly.

**Switch triggers.** N/A directly. **Don't sell to them. Sell *through* them** via Persona 1.

**ACV.** Direct: tiny. Indirect (OEM into accountant SaaS): €50k+ per platform-deal.

**Where to find them.** German DStV/StBK chambers; expert-comptable Ordre; DATEVcongress; FOKUS; Steuerberatertag.

**TAM.** ~100k EU practices, but addressed via Persona 1.

---

### Persona 7 — DevOps/platform team at a unicorn

**Who they are.** Eng or platform team at a Stripe/Adyen/Mollie/Klarna/Wise/Revolut/Mistral/Lovable/Datadog-EU class scale-up. Multi-country billing, PCI-DSS scoped, GDPR-strict. They're paying Avalara/Sovos/Pagero millions/yr and grumbling.

**JTBD.** "Replace our €1M+ compliance bill with something we own and operate. Multi-country (20+). PCI-clean. Audit-ready."

**What they use today.** Avalara, Sovos, Pagero, Vertex, Stripe Tax, internal compliance team.

**Complaints.**
- "Avalara is recognized for high pricing, with users paying thousands annually for features they don't fully use." (TaxCloud's positioning, but echoed in user reviews.)
- "Sovos only offers pricing through customized enterprise agreements. Implementation 3–6 months."
- Lock-in; closed source; can't run in our VPC.

**Budget & buying.** Eng VP → CFO/CISO/CIO sign-off. Multi-stakeholder. 6–9 month cycle. Open to OSS-with-paid-support if vendor passes security review.

**Switch triggers.** (a) Self-hostable OSS so it runs in their cloud; (b) SOC2/ISO27001 on the managed AP; (c) us-as-AP for the few countries they don't want to operate themselves; (d) clean separation between core lib (free) and operated services (paid).

**ACV.** €100k–€500k/yr.

**Where to find them.** Direct outbound. ABM. Warm intros via investor network. CFO Connect, FinOps Foundation.

**TAM.** ~300 unicorns globally. Few but lucrative.

---

### Persona 8 — Government IT contractor

**Who they are.** Engineer at Capgemini, Atos, Sopra Steria, Indra, T-Systems, Accenture Federal, Booz Allen, or a national IT services firm; or staff dev at the ministry itself.

**JTBD.** "Stand up the national e-invoice platform / connect ministry ERP to Peppol / build the PDP."

**Buying.** Tender-driven. Long. Open-source bias (Italy/Spain favor open core for public procurement). Very different sales motion.

**Verdict.** Outside our wheelhouse unless we have someone with public-sector BD. **De-prioritize.**

---

### Persona 9 — OSS maintainer of adjacent project

**Who they are.** Maintainers of Invoice Ninja (PHP, ~9k stars, Peppol added), Crater (Laravel/VueJS), Akaunting (PHP), ERPNext/Frappe (Python), Tryton, Dolibarr, Odoo modules. Solo or 2–5 person teams.

**JTBD.** "Add Peppol/XRechnung/Factur-X to my project without burning 6 months on it. Stay relevant to users post-mandate."

**Switch triggers.** A great OSS library they can vendor in, friendly license, helpful maintainer (us), good docs, and **a path to monetize** the AP transmission for their hosted users.

**ACV.** €0 direct. But each integration brings 1k–100k downstream users. **Force multiplier.**

**Where to find them.** GitHub (target via issues like "Peppol support?"), Reddit (r/selfhosted, r/erpnext, r/dolibarr), their Discords, Mastodon.

**Strategic note.** Open-source maintainers are our distribution. Land Invoice Ninja, ERPNext/Frappe, and Odoo Community first; they're entire vertical pipelines.

---

### Persona 10 — Indie hacker / solo dev

**Who they are.** Builds small SaaS, internal tools, freelancer billing, niche verticals (yoga studios, German Vereine, French auto-entrepreneurs). Often in their own country only.

**JTBD.** "I need to issue a compliant invoice. I just want it to work. I'll pay maybe €5–€20/mo total."

**Switch triggers.** Free tier, copy-pasteable code, no calls, no auth dance. Apex of PLG.

**ACV.** €0–€500/yr. They aren't revenue; they're *brand*. They post on HN, they tell their friends.

**Where to find them.** HN, Indie Hackers, X (#buildinpublic), Lobsters, dev.to, niche subreddits.

**TAM.** Vast in count, tiny in revenue. Treat as top-of-funnel.

---

### Persona 11 — Tax-tech / compliance consultant

**Who they are.** Solo or 2–10 person boutique advisory. Often Big4 alumni. Sells e-invoicing readiness assessment + light implementation help to SMB and lower mid-market clients.

**JTBD.** "I need a turnkey toolkit I can recommend to clients, ideally with a referral fee."

**Switch triggers.** Referral revshare, white-label option, good training material.

**ACV.** €2k–€15k/yr direct (tools for their own use) + leverage as channel.

**Where to find them.** LinkedIn, EESPA membership rolls, tax-industry conferences (ITR, IBFD), Big4 alumni networks.

**TAM.** Modest direct; meaningful as a channel partner.

---

### Persona 12 — Big4 / boutique consulting practice

**Who they are.** Deloitte's e-invoicing practice (now wedded to ONESOURCE Pagero via Thomson Reuters partnership), PwC, EY, KPMG, BDO, plus boutique tax-tech practices (Ryan, Vertex services).

**JTBD.** "Implement e-invoicing for our F500 client. Mandate-by-mandate, country-by-country."

**Reality.** Deloitte already partnered with Thomson Reuters (ONESOURCE Pagero). Highly unlikely to switch unless there's pricing pressure or a strong OSS argument from a client. **Don't chase.** Maybe partner opportunistically.

---

### Persona 13 — ERP systems integrator

**Who they are.** Implementation arms of Deloitte/Accenture/EY-Parthenon/Capgemini/Cognizant/Infosys/TCS. SAP/Oracle/Dynamics/Workday practices. Implement e-invoicing as part of broader ERP rollout.

**JTBD.** "Reduce delivery risk on country-localization tasks during ERP implementations."

**Switch triggers.** Pre-built connectors, reference implementations, training content, billable hours protection.

**ACV.** €50k–€500k/yr (subscription + per-project licenses) if we partner.

**Where to find them.** SAP partner ecosystem, Microsoft Partner Network, Oracle Cloud Marketplace, SI conferences.

**Verdict.** Year-3 partner play, not year-1 ICP.

---

### Persona 14 — Treasury/Finance Ops at multinational

**Who they are.** Group treasurer or VP Finance Ops at €1B+ revenue multinational operating in 50+ countries. Direct staff of 5–50.

**JTBD.** "Compliant invoicing in every country, no surprises, one throat to choke."

**Reality.** They want one vendor that takes 100% accountability. They want sales-led, indemnity, SLA, board-reportable. We are not that. **Avoid.**

---

### Persona 15 — EDI specialist

**Who they are.** Engineer at a regional EDI VAN or operator (Babelway, Mecalux, Aayu, OpenText subsidiaries, B2B-Router, ecosio). They run EDIFACT/X12 networks and now Peppol is encroaching on their lane. 100–500 FTE companies.

**JTBD.** "Add Peppol to my existing EDI VAN without ripping out the EDIFACT/X12 pipes. Convert between formats."

**Complaints (industry).** "Hybrid EDI/API has moved from emerging trend to mainstream by 2026." EDI specialists are nervously eyeing the API-first newcomers (Storecove, e-invoice.be).

**Switch triggers.** (a) Library that handles UBL ↔ EDIFACT/X12 mapping; (b) Peppol AP they can rebrand; (c) we don't compete with them for end customers.

**ACV.** €30k–€150k/yr.

**Where to find them.** EDIFICAS, GS1, EESPA member list, OpenText Connections, IBM Sterling community, Cleo customer base.

**TAM.** ~500 mid-size EDI operators. Underserved.

---

### Persona 16 — Bank / PSP

**Who they are.** Product manager at Deutsche Bank "Flow", BNP Paribas merchant services, ING, Santander, Adyen, Stripe, Mollie, Worldline, Nexi, Mastercard Track. They want to offer invoicing as value-add to SMB merchant book.

**JTBD.** "Differentiate from the rival PSP by bundling compliant invoicing into our merchant onboarding."

**Switch triggers.** White-label-able, OEM-priced, runs in their cloud, on their compliance perimeter.

**ACV.** €100k–€1M/yr. Long sales cycle.

**Where to find them.** Sibos, Money20/20, Finovate, EBA Day, partnerships with their innovation labs.

**Verdict.** Promising for year 2–3. Need a flagship logo to credentialize.

---

### Persona 17 — Government / public-sector body

**Who they are.** Internal IT of ministry, regional authority, state-owned enterprise. They run their own portal (Chorus Pro, KSeF, SdI, MyInvois) but want SDK for internal apps.

**Verdict.** Tender-driven, slow, prestige. Year 4+. **Skip for now.**

---

## 3. ICP Ranking (Top 3 for a Developer-First OSS Toolkit)

### #1 — Embedded developer at ERP / billing SaaS vendor (Persona 1)

**Why.** Largest sweet-spot of ACV (€15k–150k), durable contracts, they have the integration skill set, they hate per-document pricing, and Microsoft's 2026 unified-connector framework explicitly tells ISVs to "build your own." Each won account = thousands of downstream SMB invoices flowing through us. They also drag Persona 6 (accountants) and Persona 10 (their SMB merchants) in for free.

**Distribution leverage.** ~500 real targets, addressable via partnership BD + GitHub presence + Odoo/Dynamics conferences.

### #2 — In-house engineer at midmarket B2B SaaS (Persona 3)

**Why.** Massive count (50k+), pure PLG fit, fast sales cycles, low complexity per deal, brand-builders (every blog post helps). The "Stripe Invoicing has no Peppol" gap is the wedge. Even modest adoption fills the funnel.

**Distribution leverage.** HN + blog SEO + Indie Hackers + dev.to + targeted r/SaaS posts. Cheap.

### #3 — OSS maintainer of adjacent project (Persona 9)

**Why.** Force multiplier. Every Invoice Ninja / ERPNext / Odoo Community / Dolibarr integration is essentially free distribution to thousands of downstream users. Integration is a 1–2 week effort and pays dividends for years.

**Distribution leverage.** GitHub PRs, maintainer Discord, joint blog posts.

**Honorable mention:** Persona 4 (billing fintechs) — small in count, but two anchor design partners (e.g., Lago + one Stripe alternative) would be a massive credibility lever for Personas 1 and 3.

---

## 4. The Most Valuable but Currently Most Underserved Persona

**Persona 15 — EDI specialist adding Peppol.**

Reasoning: every EDI VAN in Europe knows Peppol is going to eat their lunch but is locked into 1990s-era EDIFACT/X12 stacks. The big API-first players (Storecove, e-invoice.be, Pagero) ignore them or compete with them. None of the modern Peppol APIs publish a clean UBL↔EDIFACT/X12 mapping library. These shops have committed customers, will pay €50k+/yr for a library that lets them keep their existing customer book alive, and are not chased by anyone in our space. They're also a Trojan horse into mid-market customers we couldn't reach otherwise.

Risk: they'll pull us toward EDI-specific features the rest of our base doesn't need. Keep them as a sidecar module, not a core direction.

---

## 5. Persona–Product Fit Forks (Where Personas Pull the Product Apart)

Building for all 17 will splatter us. The major incompatible forks:

| Fork | Personas pulling it | What they need | Why it's incompatible with the other side |
|---|---|---|---|
| **SMB self-serve vs Enterprise sales** | 2, 3, 10 vs 5, 7, 14, 16 | Free tier, credit-card billing, copy-paste examples vs RFP, SLA, indemnity, on-prem, multi-tenant audit trails | Different cost structures, support orgs, contracts, and product surface area |
| **Library shape vs Platform shape** | 1, 3, 4, 9, 15 vs 5, 6, 14 | A `cargo add` / `npm install` SDK with no UI vs a hosted portal with workflow, dashboards, approvals | Building both = 3× engineering cost; competitive sets diverge entirely |
| **OSS purity vs Closed enterprise** | 9, 10, 3 vs 7, 14, 16 | MIT/Apache/AGPL with reciprocity guarantees vs closed-source for "security review" | The OSS crowd boycotts you if the core goes closed; enterprises won't ship AGPL |
| **Outbound issuance vs Inbound capture/match** | 1, 2, 3, 4 vs 5, 14 | Generate, sign, transmit, archive vs OCR, IDP, 3-way match, ERP ingest | Inbound is a different product (IDP/ML-heavy, OCR rasterizers) — Coupa/Tipalti territory |
| **EU/EN16931 focus vs Global mandates** | 1, 2, 3, 6 vs 4, 7, 14 | Peppol BIS 3 + EU country variants vs LATAM (CFDI, NF-e), MENA (ZATCA, FBR), APAC (MyInvois, IRP) | Country implementations balloon engineering scope; 30 mandates ≠ 5×6 mandates |
| **Country-specific compliance depth vs Format breadth** | 1, 5, 12, 13 vs 2, 3, 10 | Deep validations, ATCUD (PT), QR codes (PT, ES), KSeF nuances vs "just give me a valid UBL out" | Depth requires country experts on staff; breadth requires generality |
| **Developer-first DX vs Compliance-first audit posture** | 1, 2, 3, 4, 9, 10 vs 5, 7, 12 | "It's beautiful, types, examples" vs "It's certified, signed, in audit trail, AVA, ISO 27001" | Different release velocities, different staff |

**Recommended fork choices:**
- Library shape (not platform).
- OSS core + paid managed AP/archival.
- Outbound issuance first; inbound much later (or never).
- EU/Peppol/EN 16931 + DE/FR/IT/ES/BE/NL/PL country depth; LATAM/MENA later as modules.
- Developer-first DX first; certifications added once revenue justifies.

---

## 6. ICP We Should NOT Chase (Even Though They'd Pay)

**Persona 5 — AP/AR ops engineer at 1000+ FTE corporate.**

They have budget (€100k–€300k ACV, sometimes more). They have urgency. They will pay.

But they will reshape the product into the wrong thing:

- They need *inbound capture + 3-way match* (OCR, IDP, ML). That's a different product class — Coupa/Tipalti/Stampli's home turf. To compete we'd need OCR engineers, IDP pipelines, ERP coupling work for SAP/Oracle/Workday, supplier onboarding portals, exception-handling UIs.
- They demand RFPs, certifications, "single throat to choke," and 6–12 month sales cycles. We'd burn cash hiring sales engineers and account execs before we have product-market fit.
- They want closed-source/on-prem deals with strict SLAs that conflict with OSS core.
- Their use case is "platform-shaped." Our thesis is "library-shaped."

If we let them in early, two years from now we will have built a Coupa clone instead of a developer toolkit. Every dollar they pay costs us a soul-fragment of the OSS roadmap. **Defer until year 4+, then only via a partnership/OEM with an existing AP platform.**

Honorable second mention: **Persona 14 (multinational treasury)** for similar reasons + Persona 8 (government contractors) because public-sector sales cycles will eat the company.

---

## 7. Channels & GTM (Concrete, per ICP)

### Persona 1 (ERP/billing SaaS vendor) — BD + Community
- Open-source presence with cleanly-named repo (e.g., `peppol-toolkit`, `einvoice-rs`).
- Partnership program in Year 1: "ship Peppol in 2 weeks via our SDK; AP transmission optional."
- Conferences: **Odoo Experience** (October), **Dynamics 365 Summit / Community Summit Europe**, **SAP TechEd**, **OpenPeppol member meetings**, **EESPA AGM**.
- Trade media: ERP Today, Diginomica, MSDynamicsWorld, Dynamics-Stack-Exchange.
- LinkedIn ABM on "Localization Engineer", "Compliance Engineer", "Head of Platform" titles at ~500 vendors.
- Microsoft AppSource and Odoo Apps marketplace listings.
- Joint blog posts with mid-tier ERP partners; case studies.

### Persona 2 (e-commerce app dev) — Marketplaces + PLG
- Shopify, Woo, Shopware app-store listings of *example apps* built on our library.
- Open-source "starter kit" repos for each major commerce platform.
- SEO targets: "shopify peppol", "woocommerce xrechnung", "shopware factur-x".
- Twitter/X partnerships with commerce-app builders.
- r/shopify, r/woocommerce, /r/ecommerce posts.

### Persona 3 (in-house SaaS engineer) — Content + PLG
- Show HN: launch playbook, target Tuesday morning EU.
- SEO: "stripe XRechnung", "stripe peppol", "how to send Factur-X in [language]", "[country] e-invoice library python/typescript/rust/php".
- Indie Hackers thread when launching.
- /r/SaaS, /r/programming, /r/devops weekly thread participation.
- Lobsters submissions.
- Sponsor a niche SaaS Slack / Discord.
- Tutorial blog posts for top 6 country mandates.

### Persona 4 (billing fintech) — Direct + Anchor Logos
- Direct founder-to-founder outreach to Lago, Maxio, Orb, Schematic, Solvimon, Stigg, Zenskar, Polar.sh.
- Co-marketing with one selected anchor: joint webinar, joint case study.
- YC partner relationships if relevant.

### Persona 7 (unicorn DevOps) — ABM
- ABM with named-account list of ~300.
- Warm intros via investors / advisors.
- CFO Connect, FinOps Foundation podcasts.
- Annual "self-hosted enterprise tier" press.

### Persona 9 (OSS maintainer) — GitHub-Native
- Identify top 10 adjacent projects with "Peppol support" issues open.
- Submit PRs (or invite-to-PR) implementing them via our library.
- Maintainer Discord/Matrix presence.
- Joint blog posts.
- Annual virtual "OSS Invoicing Day."

### Persona 10 (indie hacker) — Authentic Presence
- Same as Persona 3 channels plus Indie Hackers + #buildinpublic.
- Generous free tier: "first 1,000 envelopes/year free, then €0.05."
- Newsletter feature (Indie Hackers, Hacker Newsletter, Pointer).

### Persona 15 (EDI specialist) — Trade Channels
- EDIFICAS conferences, GS1 events.
- EESPA member directory cold outreach.
- Targeted whitepapers ("Peppol-Bridge: UBL ↔ EDIFACT D.96A mapping").
- B2B-Router, ecosio competitive intel + targeted positioning.

---

## 8. Year-1 Concrete Plan (Implied)

1. Ship a clean OSS library that covers EN 16931, Peppol BIS 3, XRechnung, Factur-X/ZUGFeRD, FatturaPA. Two language bindings minimum (TypeScript + one of Python/Go/Rust/PHP).
2. Stand up a managed Peppol AP at €0.05–€0.15 per envelope (undercut e-invoice.be's €0.25, drastically undercut Storecove's €495/mo entry).
3. Lock in 3 ERP/SaaS vendor design partners (Persona 1) + 2 fintech billing partners (Persona 4) + 5 OSS-maintainer integrations (Persona 9).
4. Win HN front page once.
5. Earn 2–5 in-house-SaaS-engineer testimonials (Persona 3) for case studies.

Sources (primary):
- [HN — Ask HN: How are you preparing for PEPPOL?](https://news.ycombinator.com/item?id=42777669)
- [Lago — Why billing systems are a nightmare for engineers](https://getlago.com/blog/why-billing-systems-are-a-nightmare-for-engineers)
- [The Business Research Company — E-Invoicing Market Size 2026](https://www.thebusinessresearchcompany.com/report/e-invoicing-global-market-report)
- [Best Peppol Access Points 2026 — e-invoice.be](https://e-invoice.be/blog/best-peppol-access-points)
- [Storecove pricing](https://www.g2.com/products/storecove/pricing)
- [Redress — SAP Ariba 2026 pricing analysis](https://redresscompliance.com/sap-ariba-pricing-2026)
- [Microsoft — Unified e-invoicing integration framework (2026 wave 1)](https://learn.microsoft.com/en-us/dynamics365/release-plan/2026wave1/enterprise-resource-planning/dynamics365-finance/use-extensible-universal-connector-e-invoicing-service)
- [DATEV E-Rechnungsplattform](https://www.datev.de/web/de/berufsgruppenuebergreifend/themen-im-fokus/e-rechnung-mit-datev/datev-e-rechnungsplattform)
- [Deloitte — E-invoicing services (ONESOURCE Pagero)](https://www.deloitte.com/global/en/services/tax/services/einvoicing.html)
- [Avalara vs Sovos — Numeral](https://www.numeral.com/blog/avalara-vs-sovos)
- [Coupa alternatives — Rillion](https://www.rillion.com/blog/coupa-alternatives/)
- [Belgium grace period 2026 — Babelway](https://www.babelway.com/resources/blog/belgium-e-invoicing-grace-period-2026-smb-peppol-guide/)
- [FNFE-MPE Factur-X](https://fnfe-mpe.org/factur-x/)
- [Invoice Ninja e-invoicing](https://invoiceninja.github.io/en/einvoicing/)
- [Indie Hackers — EU VAT/invoicing threads](https://www.indiehackers.com/forum/what-billing-and-invoicing-tools-are-available-for-eu-based-saas-products-4cdabf8dea)
- [DDD Invoices — Global e-invoicing single API](https://dddinvoices.com/learn/e-invoicing-italy)
- [Qvalia — Peppol white-label / pricing](https://qvalia.com/help/pricing-model-for-peppol-partners/)
- [Pennylane API docs](https://pennylane.readme.io/)
- [sevDesk tech blog — e-invoice via API](https://tech.sevdesk.com/api_news/posts/2024_11_15-einvoice_changes/)
- [TradeCentric — Peppol EDI vs Traditional EDI](https://tradecentric.com/blog/peppol-vs-traditional-edi/)
- [ERP Today — Europe ERP vendors](https://erp.today/europes-erp-vendors-are-gaining-ground-alongside-oracle-microsoft/)
