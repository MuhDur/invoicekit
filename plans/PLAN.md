# InvoiceKit — Implementation Plan v0.8 (Global, everything-in-scope, fifteen improvements baked in)

> Consolidated plan with global country coverage, four rounds of multi-model review (Codex × Gemini) plus a self-review pass, and all previously-deferred work pulled into the main build. There is no "Year 2 research" or "weeks 24+" deferral; every feature has a task identifier and runs in parallel against the dependency graph. The only external delays we accept are the ones we genuinely cannot remove: ISO 27001 certification (6–12 months, runs in the background and only gates self-operating as a Peppol Access Point — partner delivery does not need it) and country sandbox onboarding where the regulator requires a local tax identification number we have to procure. Everything else is one build push.
>
> **Round 1 changes**: dual native + WebAssembly artifacts; layered invoice model; money/tax/codelists as first-class crates; reference validator workers (not embedded Java in WebAssembly); signed evidence bundles; partner Peppol access point year 1; honest country maturity matrix.
>
> **Round 2 changes**: polymorphic URN-keyed invoice extensions; feature-flagged WebAssembly builds; per-domain JVM validator sidecars; cassette-replay sandbox; country feasibility manifests before any country crate; archetype-first then parallel agent work; realistic time estimates (6-9mo first general availability, 12-18mo broad beta, 24-36mo 60-country SLA); on-premise signer agent; AI intake pushed to phase 6; "court-grade" claims removed.
>
> **Round 3 changes**: Track 8 archetype-lock-in section made explicit and renamed; signer architecture split into substrate (T-083) + provider adapters (T-083a, T-083b); cassette system promoted to a contract-test framework (T-074a/b/c) with nightly sandbox drift canary; dependency-edge fixes throughout; under-specified tasks expanded (T-006a, T-053, T-074, T-121, T-132); WebAssembly + external validator backend semantics clarified; Phase 2.5 manifests now require initial fixture set and baseline cassettes.
>
> **Round 4 + self-review changes**: T-074 / T-074a / T-074b cassette dependency chain corrected; T-083b split into per-country sub-tasks (T-083b1-5) so Track 8 country crates consume them rather than re-implement crypto; Track 7.5 added with T-770…T-7A5 country feasibility manifest tasks (one per country); Track 8 country crates given T-IDs (T-800…T-849); Phase 0 timing corrected from "weeks 1–4" to "weeks 1–8"; Phase ↔ Track map added (§6 head); §3.4 maturity matrix rewritten with explicit enum values (`absent` / `alpha` / `beta` / `GA` / `simulated` / `proven`) instead of "yes" everywhere; §3.2 Family B counts corrected (33 crates, ~20 fresh countries); `report-be-pep` clarified as archetype-locking scaffolding (not a Belgium-specific format crate); `crates/render-pdf-postproc` + `services/invoicekit-signer-agent` + `services/validator-verapdf` + `services/validator-phase4` added to crate layout; `Invoice.builder()` API example corrected to use `finalizeInvoice(addLine(...))` consistent with the explicit-enrichment pattern; monetary literals in TypeScript example switched to strings per §2.3; `validator-worker container` residue replaced with sidecar names everywhere; partner Peppol access point decision moved from Phase 2 to Phase 2.5.
>
> **v0.7 → v0.8 (Idea-Wizard improvements baked in)**: previously-deferred work pulled into the main build (no Year 2 research; AI intake, native AS4 sender + receiver, web what-you-see-is-what-you-get template designer all in scope and task-ID'd); five top-leverage improvements added as new tracks or tasks (Track 12 billing-platform bridges, T-032a validator explain-plan trace, Track 13 on-premise deployment, T-058 visual regression for PDFs, T-026 schema evolution + automatic migration); ten complementary improvements added (Track 14 reference demo apps, Track 15 enterprise-resource-planning connectors, T-007 performance regression budget, T-141 hot-reloadable rule packs, T-142 customer-facing audit log, T-109a OpenAPI 3.1 specification, T-074d sandbox-production parity diff, T-100a live REPL, T-085 replay-from-bundle, T-008 fuzz continuous integration).

**Project**: InvoiceKit
**License**: Apache 2.0
**Mode**: Solo principal + AI agents, one concentrated build push
**Date**: May 2026

---

## Plain-English summary

We are building one open-source toolkit that lets any developer create, check, render, read, send, and archive legally-correct B2B electronic invoices for every country that has an e-invoicing mandate worth supporting. The same engine runs on Node, Python, Java, .NET, Go, the browser, and edge runtimes. The core is free forever under the Apache 2.0 license. A small hosted layer takes care of the things developers cannot easily do themselves — actually delivering invoices to government portals and the Peppol network, holding signing certificates, and storing legally-binding archives.

The wedge: every existing alternative forces a tradeoff. Java-only libraries do not run in the browser or on edge runtimes. Hosted services cost fifteen thousand euros a year minimum and lock customers in. National compliance vendors only cover one country. We are the only option that gives a developer one install, one engine, and every country.

---

## 1. Goal and scope

**Goal**: ship a complete, production-quality, open-source e-invoicing engine with global country coverage, in one concentrated build push.

**In scope**:
- A Rust engine compiled to native bindings (Node, Python, Java, .NET, Go) and WebAssembly (browser, edge).
- The complete invoice lifecycle: create, check, render to PDF, read incoming invoices, send through Peppol and national gateways, archive with signed evidence.
- Format support for every published e-invoicing format that has real-world demand (Universal Business Language 2.1, Cross Industry Invoice, EN 16931 European norm, Peppol BIS 3.0, Peppol PINT international, Factur-X / ZUGFeRD, German XRechnung 3.x, Italian FatturaPA 1.2.2, Polish KSeF FA(3), Spanish VeriFactu / FacturaE, French Chorus Pro / Factur-X PA-PDP, Saudi ZATCA Phase 2, Indian IRP, Mexican CFDI 4.0, Brazilian NF-e and NFS-e, Malaysian MyInvois, Greek myDATA, UAE PINT-AE, Turkish e-Fatura, and others as listed in the country matrix below).
- Free and open-source forever for the engine.
- A small hosted layer for delivery, certificates, and archive (paid; supports the project commercially).

**Out of scope** (explicit):
- We do not file tax returns.
- We do not run an accounting ledger or general ledger.
- We do not process payments. We describe how an invoice should be paid; the payment itself happens elsewhere.
- We do not build customer relationship management.
- We do not replace ERP systems. We feed them.
- We do not compete with end-user invoicing apps (Invoice Ninja, Crater, Wave). We are infrastructure for the developers who build those.
- We do not ship a closed-source SaaS portal as the primary product.
- We do not lead any marketing or branding with "AI-first." Artificial intelligence is used quietly for reading incoming PDFs. Outbound generation is always deterministic.

---

## 2. Architectural commitments

These were settled after multi-model review. They are load-bearing. Do not casually overturn them.

### 2.1 One engine, two delivery shapes

The Rust engine is the single source of truth. It is delivered through two paths:

- **Native bindings** for runtimes that run on servers and desktops: Node (via `napi-rs`), Python (via `pyo3` and `maturin`), Java (via the Foreign Function and Memory API or JNI over a C ABI), .NET (via P/Invoke over the same C ABI), Go (via `cgo`, with a REST sidecar fallback).
- **WebAssembly** for the browser, Cloudflare Workers, Deno, Bun's WebAssembly path, and other edge runtimes.

The universal contract is the engine API, the invoice data model, the rule packs, and the test fixtures. WebAssembly is not the universal runtime. We made this correction during review because Python, Java, .NET, and Go developers actively dislike WebAssembly indirection when a native binding works fine.

**Feature-flagged WebAssembly builds.** Bundling all 60 country crates into a single WebAssembly artifact would produce a 30+ megabyte payload — too large for Cloudflare Workers (1 megabyte after compression) and slow to start in browsers. WebAssembly builds are therefore feature-flagged: a developer compiles only the countries they need with `cargo build --features="de,fr,it,peppol" --target=wasm32-unknown-unknown`. The core engine and pure-Rust EN 16931 subset are always included; per-country format families, per-country report crates, and the country's validator-backend selection are opt-in. **Important nuance**: a feature-flagged WebAssembly build can include a country's serializer but still be unable to validate it locally if that country's rule pack declares `jvm:*`, `cli:*`, `rest:official`, or `partner` as its validator backend. The capability matrix from `T-033` makes this explicit at runtime; calling `validate()` from a WebAssembly context against a rule pack that needs an external backend returns `RequiresExternalBackend`, not a partial result.

### 2.2 Layered invoice model

The invoice data model has four layers:

- `CommercialDocument` — global commercial invoice and credit-note semantics, not tied to any one country's law.
- `ProfileView` — a projection onto a specific standard or country format (EN 16931, Peppol BIS, Peppol PINT, XRechnung, Factur-X profiles, FatturaPA, KSeF, ZATCA, IRP, CFDI, NF-e, MyInvois, myDATA, etc.).
- `JurisdictionExtension` — typed, namespaced, versioned extension data per country or profile.
- `LossinessLedger` — required output of every projection between layers. Tells the caller what data was preserved and what was not.

EN 16931 is the European anchor. It is not the universal root, because Mexico's CFDI, Brazil's NF-e, Saudi Arabia's ZATCA, and a few other regimes have semantics EN 16931 simply does not represent.

### 2.3 Money, tax, and code lists are first-class

Monetary values never use floating-point arithmetic. They use fixed-scale decimal strings at the boundary and `rust_decimal::Decimal` internally. Code lists (ISO 3166 country codes, ISO 4217 currencies, UN/ECE units, VAT category codes, Peppol code lists, country-specific tax category codes) are signed, versioned, effective-dated rule pack data. Tax arithmetic (line extension, allowances, charges, VAT category subtotals, payable amount, currency conversion) is a separate crate with deterministic decimal math and a formal trace.

### 2.4 Rule packs are signed and versioned with effective dates

Every rule pack carries source URLs, retrieval timestamps, upstream version identifiers, effective date ranges, code list versions, raw upstream checksums, generated Rust and JSON metadata, parity fixtures, and known gaps. The CLI accepts `--date=YYYY-MM-DD` and validates against the rule pack that was effective on that date. Continuous integration refuses unpinned rules.

### 2.5 State machine and outbox come before any gateway integration

Every transmission carries a trace ID, a tenant ID, an idempotency key, and a gateway attempt ID. Each country gateway is a `GatewayAdapter` trait implementation with a normalized error taxonomy. The mock gateway is the first implementation of the trait, ensuring the contract is real before any live gateway code is written.

### 2.6 Reference validators run as isolated JVM worker sidecars

Reference validators are run as **domain-specific JVM sidecars**, not a single monolithic JVM container. Combining KoSIT, phive, Saxon, ZATCA's Java SDK, India's IRP libraries, and Mexican PAC validation into one process leads to classpath conflicts (log4j and BouncyCastle versions clash; transitive dependencies fight), memory bloat, and brittle startup. Splitting by dependency boundary keeps each sidecar small and upgradeable on its own cadence.

Sidecars at launch:

- `validator-kosit` — KoSIT XRechnung validator
- `validator-phive` — Helger phive engine + Peppol Schematron rule packs
- `validator-saxon` — Saxon XSLT 2.0 runtime (called by phive and others as needed)
- `validator-zatca` — Saudi Arabia ZATCA Java software development kit when it lands in Track 8
- `validator-irp` — India IRP if a JVM library proves better than rewriting
- additional per-country sidecars added with their report crate

The Rust engine talks to each sidecar over JSON-RPC. We do not embed Java in WebAssembly. We do not reimplement these validators. They are the official references; we use them as the ground truth.

Pure-Rust validators are introduced incrementally, rule set by rule set, and only after they hit 99.9% rule parity against the JVM reference. If a rule set does not reach parity in a reasonable effort, it stays as a call to the JVM sidecar forever. That is fine.

### 2.7 Native AS4 ships in parallel with partner delivery

The AS4 transport protocol used by Peppol is not trivial. Production-grade certification requires Web Services Security, ebMS3, signing, canonicalization, SMP/SML participant lookup, certificate handling, retries, receipts, OpenPeppol conformance testing, and operational practices.

We build it anyway, in parallel with partner delivery. There are two `GatewayAdapter` implementations from day one:

- **Partner adapter** (Storecove, ecosio, or B2BRouter — choice locked in Phase 2.5) — covers live delivery while certification is in progress.
- **Native Rust AS4 adapter** (sender and receiver) — built concurrently. Promotes to live delivery for each country/profile combination as it passes the OpenPeppol conformance suite. Until conformance passes for a given country, traffic stays on the partner adapter; the switch is per-route, not per-country.

`phase4` runs in the `validator-phase4` JVM sidecar as the conformance oracle that the native Rust adapter is differentially tested against. The native adapter is not a research artifact; it is a shipping component gated only on automated conformance evidence.

### 2.8 PDF rendering is deterministic; veraPDF is the oracle

The underlying renderer is Typst. We ship a TypeScript template language on top so users never see Typst directly. Two renders of the same invoice produce byte-identical PDFs (pinned fonts, pinned harfbuzz, deterministic object ordering, fixed XMP creation date placeholder).

For PDF/A-3 conformance verification, veraPDF is the reference oracle in continuous integration and the managed service. We do not reimplement veraPDF. Our own Rust checks are limited to structural invariants we own.

If Typst cannot satisfy embedded XML plus PDF/A-3 conformance for every Factur-X and ZUGFeRD profile we need, the architecture allows a secondary renderer behind the same `RenderBackend` trait.

### 2.9 We interoperate with `invopop/gobl`'s JSON schema

GOBL is the closest open-source neighbor. They built a respectable invoice data model in Go. We read and write GOBL JSON transparently. We do not reinvent it. Where their model is sound, we copy it.

### 2.10 Every operation produces a signed evidence bundle

The `.invoicekit` bundle is the deterministic, signed record of any invoice operation. Its contents:

- A signed manifest (BLAKE3 hashes of every payload; Dead Simple Signing Envelope signature over the manifest).
- The canonical invoice JSON.
- The generated XML for every format requested.
- The rendered PDF/A-3.
- The intake source bytes and extraction trace, if applicable.
- The validation trace (every rule evaluated, result, location).
- The rule pack manifest with hashes.
- Cryptographic signatures (PKCS#7, XAdES, qualified electronic seal, as required).
- An RFC 3161 timestamp from a trusted timestamping authority.
- Gateway receipts (Peppol ACKs, SDI receipts, KSeF tokens, ZATCA stamps, IRP IRNs, etc.) with timestamps.
- A declarative `replay.json` that the `invoicekit verify` command can re-execute. Verification never executes shell scripts.

The canonical form is a directory tree. The portable packed form is `.ikb` (a `tar.zst` archive with normalized metadata so it is bit-stable).

Important honesty: the `.invoicekit` bundle is not a legal artifact by itself. The legal artifacts are the embedded XML, the PDF/A-3, and the qualified signatures. The bundle is a convenience wrapper with verification metadata.

### 2.11 Honest country coverage with explicit maturity labels

A country is not "supported." A country has a maturity label per capability: serialize, validate, render, sandbox, partner-live, native-live, inbound, archive, correction, SLA. The country matrix below shows where each cell sits. We will not write "supports country X" anywhere unless the matrix backs the claim.

---

## 3. Global country coverage

This is the heart of the project. The model below explains how we reach roughly sixty jurisdictions with realistic effort.

### 3.1 The two layers of "support" — they cost very different amounts

**Format support** — we can generate, check, and read the country's required invoice file.

**Delivery support** — we can actually deliver the invoice through the country's government portal or network.

Format support scales fast because most countries share a few underlying formats. Delivery support scales slower because each gateway has its own protocol, certificates, and sandbox onboarding.

### 3.2 Format families and the countries they cover

The Rust engine ships with these format families. Once each family is implemented, it covers many countries at once.

#### Family A — EN 16931 / Peppol BIS / UBL 2.1 / Cross Industry Invoice

This is one technical family because all four standards are variants of the same European norm. Once the engine speaks this family, it can generate and check invoices for the following countries:

| Country | Standard variant | Notes |
|---|---|---|
| Germany | XRechnung 3.x; ZUGFeRD; Peppol BIS | Federal e-invoicing mandate active |
| France | Factur-X; Chorus Pro; PA / PDP | Broad receipt mandate from September 2026 |
| Italy | Peppol BIS for cross-border (FatturaPA covered separately) | National format also needed |
| Spain | Peppol BIS for cross-border (VeriFactu and FacturaE covered separately) | National formats also needed |
| Poland | Peppol BIS for cross-border (KSeF covered separately) | National format also needed |
| Belgium | Peppol BIS — full mandate live | Peppol-native country, simplest case |
| Netherlands | Peppol BIS | B2G live; B2B growing |
| Romania | Peppol BIS for cross-border; RO e-Factura for national | National handled in Family B |
| Hungary | Peppol BIS for cross-border | National reporting (NAV) handled in Family B |
| Portugal | Peppol BIS; CIUS-PT | |
| Greece | Peppol BIS for cross-border (myDATA covered separately) | National reporting handled in Family B |
| Ireland | Peppol BIS | |
| Denmark | Peppol BIS / OIOUBL | |
| Sweden | Peppol BIS / Svefaktura | |
| Finland | Peppol BIS / Finvoice 3.0 | |
| Estonia, Latvia, Lithuania | Peppol BIS | |
| Slovenia, Slovakia, Croatia | Peppol BIS | |
| Czechia | Peppol BIS | |
| Austria | Peppol BIS / ebInterface | |
| Bulgaria | Peppol BIS | |
| Luxembourg | Peppol BIS | |
| Cyprus, Malta | Peppol BIS | |
| United Kingdom | Peppol BIS (HMRC Making Tax Digital framework) | |
| Norway | Peppol BIS / EHF | |
| Iceland | Peppol BIS / TS 236 | |
| Switzerland | Peppol BIS for cross-border | |
| Australia | Peppol PINT-AU | |
| New Zealand | Peppol PINT-NZ | |
| Singapore | Peppol PINT-SG (InvoiceNow) | |
| Japan | Peppol BIS (Qualified Invoice System integration coming) | |
| UAE | Peppol PINT-AE | National platform onboarding 2026 |

That is 35+ countries reached through one technical family.

#### Family B — National clearance and reporting formats

Several countries use government-mediated clearance or reporting. These are state machines, not just serializers. Each gets a dedicated crate (named `report-<country>` in the layout) because the work involves submission, clearance state, cancellation, and correction flows, not just XML generation.

| Country | National format | Crate |
|---|---|---|
| Italy | FatturaPA 1.2.2 with SDI clearance | `report-it-sdi` |
| Poland | KSeF FA(3) | `report-pl-ksef` |
| Spain | VeriFactu, FacturaE, regional TicketBAI | `report-es-verifactu` |
| France | Chorus Pro and PA-PDP flow with e-reporting | `report-fr-ctc` |
| Romania | RO e-Factura | `report-ro-efactura` |
| Hungary | NAV Online Invoicing | `report-hu-nav` |
| Greece | myDATA | `report-gr-mydata` |
| Saudi Arabia | ZATCA Phase 2 with cryptographic stamping | `report-sa-zatca` |
| India | IRP / GST / e-Waybill | `report-in-gst` |
| Mexico | CFDI 4.0 via a Mexican PAC partner | `report-mx-cfdi` |
| Brazil | NF-e and NFS-e via SEFAZ | `report-br-nfe` |
| Chile | SII DTE | `report-cl-dte` |
| Colombia | DIAN | `report-co-dian` |
| Peru | SUNAT | `report-pe-sunat` |
| Argentina | AFIP | `report-ar-afip` |
| Ecuador | SRI | `report-ec-sri` |
| Costa Rica | Hacienda | `report-cr-hacienda` |
| Dominican Republic | DGII | `report-do-dgii` |
| Malaysia | MyInvois | `report-my-myinvois` |
| Indonesia | DJP Online | `report-id-djp` |
| Philippines | BIR EIS | `report-ph-bir` |
| Vietnam | GDT e-invoice | `report-vn-gdt` |
| Thailand | RD e-tax invoice | `report-th-rd` |
| South Korea | Cash Receipt System / NTS | `report-kr-nts` |
| Japan | Qualified Invoice System (parallel to Peppol) | `report-jp-qis` |
| China | Golden Tax / Fapiao | `report-cn-fapiao` |
| Taiwan | MOF e-invoice | `report-tw-mof` |
| Egypt | ETA e-invoicing portal | `report-eg-eta` |
| Turkey | e-Fatura | `report-tr-efatura` |
| Israel | Tax Authority CTC | `report-il-ita` |
| Kenya | eTIMS | `report-ke-etims` |
| Nigeria | FIRS Merchant Buyer Solution | `report-ng-firs` |
| South Africa | SARS modernization (emerging) | `report-za-sars` |

That is 33 national-format crates. Several of the countries listed here also appear in Family A (Italy, Poland, Spain, France, Romania, Hungary, Greece, Portugal, Japan) — they need a national crate **in addition to** Family A coverage because the national format and the Peppol cross-border format are different artifacts handled separately.

Family A reaches ~35 countries on its own. Adding the Family B national crates **does not** add 33 fresh countries; it adds national-format coverage to the ~13 Family A countries that also need a national flow, plus ~20 fresh non-Family-A countries (Saudi Arabia, India, Mexico, Brazil, Chile, Colombia, Peru, Argentina, Ecuador, Costa Rica, Dominican Republic, Malaysia, Indonesia, Philippines, Vietnam, Thailand, South Korea, China, Taiwan, Egypt, Turkey, Israel, Kenya, Nigeria, South Africa).

Total realistic global coverage: roughly **55–60 unique jurisdictions** once both families are built, with the deeper countries (the ones that need both Family A and a national crate) counted once.

Belgium is intentionally Peppol-native only (no separate national crate) because its Jan 2026 mandate is "use Peppol BIS via an access point" — Family A covers it. Some sources may show a `report-be-pep` crate as the **archetype-locking** vehicle for the Peppol-overlay archetype; that is implementation scaffolding, not a separate Belgium-specific format.

### 3.3 The order of attack

Solo + AI agents in parallel means we can do many things at once, but the engine must come first or every country crate has to be rewritten later. The order is:

**Phase 0 — Engine and foundation (no countries yet; the critical-path bottleneck for everything else):**

> Note on timing: the critical-path chain through Phase 0 is T-001 → T-010 → T-014/T-015 → T-016 → T-023 → T-024/T-025, roughly six to eight weeks of serialized work. The rest of Phase 0 (validator sidecars T-030, evidence bundle T-080, reconciliation T-070..T-077, signer agent T-083) runs in parallel against that chain. As soon as T-023 lands, Phase 1, Phase 4, Phase 5, and Phase 6 all become unblocked.
- Rust workspace, Cargo crates, continuous integration.
- The `money`, `codelists`, `tax-calculation`, and `rulepack` crates.
- The layered invoice model crate (`ir`).
- Canonical serialization (XML C14N 1.1, JSON Canonicalization Scheme).
- The reference validator worker sidecars (per-domain JVM containers: `validator-kosit`, `validator-phive`, `validator-saxon`; further sidecars added as countries land).
- The signed evidence bundle format.
- The state machine, outbox, and idempotency primitives.
- The `GatewayAdapter` trait and the mock gateway (cassette-replay).
- The tenant, key-management, and audit-event schema for the managed layer.
- The C ABI for native bindings, the WebAssembly artifact (feature-flagged), and the engine's stable ABI contract.

**Phase 1 — Family A coverage (parallel; starts when T-010, T-014, T-015, T-019, T-020 are done):**
- Universal Business Language 2.1 parser and serializer.
- Cross Industry Invoice parser and serializer.
- EN 16931 hand-written validator (~50 business rules).
- Peppol BIS 3.0 projection.
- Peppol PINT projection (international variant).
- Factur-X / ZUGFeRD profiles (all six: MINIMUM, BASIC WL, BASIC, EN 16931, EXTENDED, XRECHNUNG).
- German XRechnung 3.x projection.
- Typst-based PDF/A-3 renderer with Factur-X XML embedding.
- veraPDF adapter for conformance checking.
- Peppol participant lookup (SMP / SML client).

After Phase 1 is complete, **35+ countries get format support**. They cannot yet send live, but they can generate, validate, render, and read invoices in all the right shapes.

**Phase 2 — Live Peppol delivery (parallel; starts when T-042 Peppol BIS projection and T-072 transmission worker are done):**
- Partner Peppol access point integration (one integration, ~30 destinations).
- `phase4`-backed reference adapter for testing.
- Universal `transmit-mock` for sandbox testing.
- Inbound Peppol receiver service.

After Phase 2, **every Peppol-using country in Family A can both send and receive live**.

**Phase 2.5 — Country feasibility manifests (parallel with Phase 1 and Phase 2):**

Before any country crate is implemented, that country gets a feasibility manifest. The manifest is a short signed document containing:

- Source URLs and retrieval dates for the country's authoritative documents.
- Whether a sandbox is publicly available, requires local tax ID registration, or is only accessible through a partner.
- Whether a qualified electronic seal, hardware security module, or smart card is required.
- Whether a local fiscal representative or in-country PAC / ASP relationship is required.
- Which validator backend the country's rule pack will use.
- Known partner options (with current per-envelope pricing if disclosed).
- Go / no-go flag, and if no-go, what would unblock it.
- **Initial fixture set**: at minimum 5 valid invoices and 5 invalid invoices in the country's required format. Real or anonymised, with expected validation outcome documented.
- **Baseline sandbox cassettes**: at least one success and one canonical-error cassette recorded from the official or partner sandbox (when a sandbox exists). The manifest agent is responsible for sourcing or generating these — otherwise the country-crate agent will be blocked on day one.

A country crate cannot start without its manifest. Agents can produce manifests in parallel; manifests are cheap (1–3 days each), but a manifest without fixtures and cassettes does not count as "done."

**Phase 3 — National clearance and reporting crates:**

Country crates do NOT start from the foundation alone. Each requires:

1. that country's Phase 2.5 feasibility manifest (Section 3.3, Phase 2.5),
2. the cassette recorder/scrubber/matcher framework (T-074a),
3. the relevant archetype trait locked by one completed archetype crate.

**Archetype lock-in** runs strictly sequentially. No other country crate starts until the matching archetype is locked:

1. **`report-pl-ksef`** — async clearance archetype (submit, poll, reserve invoice number, commit, cancel, correct). 3 weeks.
2. **`report-sa-zatca`** — cryptographic archetype (ECDSA secp256k1 signing, UBL canonicalization, TLV QR code generation, certificate management). 6–8 weeks. (Earlier estimate of 3 weeks was wrong; ZATCA Phase 2 is genuinely heavy.)
3. **`report-be-pep`** — Peppol-mandate / CIUS overlay archetype (mostly reuses Family A with country-specific CIUS overlay). 1 week.

**Parallel waves** start only after archetype lock-in, when agents can pattern-match a new country crate against one of the three archetypes. Each crate is named for its archetype lineage. Realistic order based on regulatory urgency, market size, and onboarding difficulty:

- **Wave 1** (regulatory urgency; parallel only after archetype lock-in): Italy SDI, France PA-PDP, Spain VeriFactu and FacturaE, Greece myDATA, UAE PINT-AE national onboarding.
- **Wave 2** (large markets): India IRP and e-Waybill, Mexico CFDI, Brazil NF-e and NFS-e (see note below on Brazilian complexity), Malaysia MyInvois, Turkey e-Fatura, Romania, Hungary, Japan.
- **Wave 3** (rest of Latin America, MENA, APAC, Africa): Chile, Colombia, Peru, Argentina, Ecuador, Costa Rica, Dominican Republic; Egypt, Israel, Jordan, Bahrain; Indonesia, Philippines, Vietnam, Thailand, South Korea, Japan (Qualified Invoice System), Taiwan, China; Kenya, Nigeria, South Africa; Portugal (national reporting alongside Peppol).

**Note on Brazil**: `report-br-nfe` is not one clean gateway. It is NF-e (federal goods), NFS-e (services with municipal variance across 5,500+ municipalities), certificate rules, SEFAZ behavior, partner-vs-native choice, and validator oracle selection. The country feasibility manifest for Brazil must pin all of these decisions before any agent touches code. Realistic effort: 4 weeks for NF-e proper plus an additional 2–4 weeks per municipal NFS-e sub-flow we commit to.

**Phase 4 — Intake pipeline (runs in parallel from day one, depends only on T-010 + format families):**

The intake pipeline runs in parallel with outbound. Different agents own outbound vs. intake; the dependency graph does not force one to wait for the other. The pipeline is:

- Layer 1: Factur-X / ZUGFeRD embedded XML extraction.
- Layer 2: Digital PDF text extraction.
- Layer 3: PDF form field extraction.
- Layer 4: PaddleOCR PP-StructureV3 for layout-aware OCR (server-side).
- Layer 5: SmolDocling-256M ONNX for short-document vision-language understanding (server-side and limited browser-side).
- Layer 6: Qwen2.5-VL-7B inference adapter for cloud fallback.
- Layer 7: Optional cloud LLM (OpenAI or Anthropic vision API) as the last resort.
- Cross-examination: every AI-extracted field is re-validated by the deterministic rules. Mismatches block AI-only output.
- Bounding-box citation taxonomy: every value carries source coordinates.

**Phase 5 — Developer experience surface (rolling, parallel with all other phases):**
- The `invoicekit` CLI binary.
- `invoicekit doctor`, `init`, `convert`, `validate`, `render`, `send`, `verify`, `fuzz`, `benchmark`, `capabilities`.
- TypeScript SDK with three packages: `@invoicekit/core`, `@invoicekit/render`, `@invoicekit/managed`.
- Python SDK via `pyo3` and `maturin`.
- Java SDK via JNI / Foreign Function and Memory API over the C ABI.
- .NET SDK via P/Invoke over the C ABI.
- Go SDK via cgo, with a REST sidecar fallback.
- Browser bundle via `wasm-bindgen`.
- REST shim service (Axum) for conservative customers.
- Language Server Protocol implementation for Visual Studio Code, Cursor, Neovim, Helix.
- Documentation site (Nextra) with per-rule pages and per-country guides.

**Phase 6 — Hosted managed layer (parallel; T-130/T-131/T-132/T-133 start immediately, others gated on engine readiness):**
- API gateway, authentication, rate limiting, per-tenant key management.
- Tenant audit dashboard.
- Pluggable archive backends (S3 Object Lock, Azure WORM blob, Google Cloud Storage with retention, local file system, content-addressed IPFS hash).
- RFC 3161 timestamping integration with a trusted timestamping authority.
- eIDAS qualified signature integration with a European qualified trust service provider.
- OpenTelemetry instrumentation with per-gateway dashboards.
- Replay and admin tooling for stuck transmissions and dead-letter queues.

**Phase 7 — Conformance and trust infrastructure (rolling, parallel with engine work):**
- Adversarial fixture generator.
- Synthetic public corpus (CC0 / Apache 2.0): 500+ fixtures across all format families.
- Licensed real corpus (with explicit licensing metadata).
- Private regression corpus (not public).
- Public free validator at `validate.invoicekit.org` — dual mode: local browser-only and server-assisted reference.
- Per-rule explanatory pages on the documentation site (the search-engine optimization play).
- Country newsletter / source-watch automation that monitors official sources and opens issues when rules change.

**Phase 8 — Distribution and ecosystem (parallel; starts once T-031, T-091, and the REST shim are usable):**
- Five billing-platform bridges (Stripe Invoicing, Lago, Maxio, Chargebee, Recurly) — Track 12.
- Eight reference demo applications (Next.js, Django, Rails, Spring Boot, ASP.NET, Laravel, FastAPI, Go) — Track 14.
- Six enterprise resource planning connectors (Odoo, Microsoft Dynamics 365, SAP Business One, Lexware, Sage, sevDesk) — Track 15.

**Phase 9 — Deployment artifacts (parallel; starts once the managed layer is operational in development):**
- Single-host docker-compose for the full managed stack.
- Kubernetes Helm chart for production-grade multi-node deployment.
- Terraform module for managed-cloud provisioning (AWS, Azure, Google Cloud Platform).

### 3.4 Honest maturity matrix

For each country, the engine reaches one of these levels per capability. The values are an enum, not booleans:

- **`absent`** — not implemented.
- **`alpha`** — implemented, no tests or fixtures yet.
- **`beta`** — implemented with at least the Phase 2.5 minimum fixture set (5 valid, 5 invalid) passing.
- **`general-availability`** — implemented, tested, documented, with `sandbox-proven` cassette evidence where applicable.

The ten capabilities each cell may carry:

- **Serialize** — we can generate the country's required invoice file.
- **Validate** — we can check it against the country's rules using the validator backend the rule pack declares.
- **Render** — we can produce the visual PDF with embedded data if the country needs it.
- **Sandbox** — `simulated` (cassette-based) or `proven` (live nightly canary against the official or partner sandbox; see §4.11).
- **Partner-live** — we can deliver live through a partner gateway or Peppol access point.
- **Native-live** — we can deliver live through our own integration without a partner. We build this alongside partner-live (per §2.7); a country/profile flips from `partner-live` to `native-live` as the native adapter passes the OpenPeppol conformance suite (for Peppol routes) or the country's official sandbox (for national portals).
- **Inbound** — we can receive and parse invoices coming from the country.
- **Archive** — we can store with the country's required retention rules.
- **Correction** — we can handle credit notes, debit notes, and cancellations.
- **SLA** — the managed layer carries a service-level agreement for that country.

After the build push, the per-country picture is:

| Country group | Serialize | Validate | Render | Sandbox | Partner-live | Native-live | Inbound | Archive | Correction | SLA |
|---|---|---|---|---|---|---|---|---|---|---|
| Family A countries (EN 16931 / Peppol BIS / PINT, 35+ countries) | GA | GA | GA | proven (cassette + nightly canary) | GA | GA where OpenPeppol conformance passes for that route, else partner | GA | GA | GA | GA |
| Germany, France, Italy, Poland, Belgium | GA | GA | GA | proven | GA | GA | GA | GA | GA | GA |
| Spain, Greece, UAE | GA | GA | GA | proven | GA | GA where conformance passes | GA | GA | GA | GA |
| Saudi Arabia, India, Mexico, Brazil, Malaysia, Turkey, Romania, Hungary, Japan | GA | GA | GA | proven where credentials available, simulated otherwise | GA via partner | GA where native adapter passes the country sandbox | GA | GA | GA | GA |
| Rest of Latin America, MENA, APAC, Africa | GA | GA | GA | proven where credentials available, simulated otherwise | GA via partner | GA where the native adapter passes the country sandbox | GA | GA | GA | GA |

The honest caveat is delivery in Latin America (Brazil, Mexico, India) often requires a local tax identification number to test against, and a real-world signing certificate purchased from a local trust provider. We will either purchase those for our own test setup or partner with a local PAC (provedor autorizado de certificación) for delivery — the choice is made in each country's Phase 2.5 manifest. Neither path delays the country: partner delivery ships immediately; native upgrade happens later under the same task once credentials land.

---

## 4. Engine architecture in detail

### 4.1 Crate layout

```
invoices/
├── crates/
│   ├── invoicekit-engine/        Pure deterministic Rust API; the source of truth
│   ├── invoicekit-ffi/           Stable C ABI for native bindings
│   ├── invoicekit-wasm/          Browser and edge WebAssembly artifact
│   ├── money/                    rust_decimal-based money type
│   ├── codelists/                Signed, versioned, effective-dated code list registry
│   ├── tax-calculation/          Deterministic invoice arithmetic with formal trace
│   ├── rulepack/                 Signed, effective-dated rule packs
│   ├── ir/                       Layered invoice model
│   ├── canonical/                Deterministic XML and JSON serialization
│   ├── validate/                 Rule registry and reference-worker client
│   ├── validate-ubl-cii/         Pure-Rust validators for the EN 16931 core
│   ├── render-pdf/               Typst-based PDF/A-3 with Factur-X embedding
│   ├── render-pdf-postproc/      PDF/A-3 dictionary post-processing (XMP, embedded-XML relationships) per T-053
│   ├── render-verify/            veraPDF adapter and structural preflight
│   ├── intake-pdf/               Digital PDF parsing and Factur-X XML extraction
│   ├── intake-ocr/               PaddleOCR and small vision-language model intake
│   ├── intake-vlm/               Cross-examined witness extraction interface
│   ├── transmit-peppol/          AS4 envelope exchange (partner-AP + phase4 reference)
│   ├── transmit-mock/            Sandbox mock gateway
│   ├── transmit-email/           Email fallback delivery with signed attachment
│   ├── report-fr-ctc/            France PA / PDP e-invoicing and e-reporting flows
│   ├── report-it-sdi/            Italy SDI clearance and receipts
│   ├── report-es-verifactu/      Spain anti-fraud reporting and FacturaE
│   ├── report-pl-ksef/           Poland KSeF clearance and submission
│   ├── report-gr-mydata/         Greece myDATA reporting
│   ├── report-ro-efactura/       Romania RO e-Factura
│   ├── report-hu-nav/            Hungary NAV Online Invoicing
│   ├── report-sa-zatca/          Saudi Arabia ZATCA Phase 2
│   ├── report-in-gst/            India IRP / GST / e-Waybill
│   ├── report-mx-cfdi/           Mexico CFDI 4.0 via PAC
│   ├── report-br-nfe/            Brazil NF-e and NFS-e
│   ├── report-cl-dte/            Chile SII DTE
│   ├── report-co-dian/           Colombia DIAN
│   ├── report-pe-sunat/          Peru SUNAT
│   ├── report-ar-afip/           Argentina AFIP
│   ├── report-ec-sri/            Ecuador SRI
│   ├── report-cr-hacienda/       Costa Rica Hacienda
│   ├── report-do-dgii/           Dominican Republic DGII
│   ├── report-my-myinvois/       Malaysia MyInvois
│   ├── report-id-djp/            Indonesia DJP Online
│   ├── report-ph-bir/            Philippines BIR EIS
│   ├── report-vn-gdt/            Vietnam GDT e-invoice
│   ├── report-th-rd/             Thailand RD e-tax invoice
│   ├── report-kr-nts/            South Korea NTS
│   ├── report-jp-qis/            Japan Qualified Invoice System
│   ├── report-cn-fapiao/         China Golden Tax / Fapiao
│   ├── report-tw-mof/            Taiwan MOF e-invoice
│   ├── report-eg-eta/            Egypt ETA e-invoicing
│   ├── report-tr-efatura/        Turkey e-Fatura
│   ├── report-il-ita/            Israel Tax Authority CTC
│   ├── report-ke-etims/          Kenya eTIMS
│   ├── report-ng-firs/           Nigeria FIRS
│   ├── report-za-sars/           South Africa SARS
│   ├── reconcile/                Fingerprint, state machine, outbox, idempotency
│   ├── evidence/                 .invoicekit signed bundle format
│   ├── archive/                  Pluggable storage backends
│   ├── verify/                   Bundle verification library and CLI
│   ├── replay/                   Replay an invoice end-to-end from its `.invoicekit` bundle (T-085)
│   ├── migration/                IR major-version forward migration (T-026)
│   ├── lsp/                      Invoice language server
│   ├── cli/                      `invoicekit` binary (includes `repl`, `explain`, `migrate-archive` subcommands)
│   └── managed-api/              Hosted-service composition layer
├── bindings/
│   ├── node-napi/                napi-rs native binding for Node
│   ├── python/                   pyo3 + maturin wheel
│   ├── dotnet/                   P/Invoke over C ABI
│   ├── java/                     JNI / Foreign Function and Memory API over C ABI
│   ├── go/                       cgo + REST sidecar fallback
│   ├── wasm-browser/             wasm-bindgen for browser and Cloudflare Workers
│   └── rest-shim/                Axum HTTP service for conservative customers
├── services/
│   ├── validator-kosit/          KoSIT XRechnung validator sidecar (JSON-RPC)
│   ├── validator-phive/          Helger phive engine + Peppol Schematron sidecar (JSON-RPC)
│   ├── validator-saxon/          Saxon XSLT 2.0 runtime sidecar (used by others)
│   ├── validator-verapdf/        veraPDF PDF/A-3 conformance sidecar (JSON-RPC over the same Java host)
│   ├── validator-phase4/         phase4 reference adapter sidecar (Y1 receiver path)
│   ├── validator-zatca/          Saudi Arabia ZATCA Java SDK sidecar (added with country)
│   ├── invoicekit-signer-agent/  On-premise signing proxy (T-083). Customer-deployed daemon; the engine talks to it over a local Unix socket or local HTTPS endpoint.
│   └── managed-api-server/       Hosted compliance API (Axum + Postgres + S3)
├── conformance-corpus/
│   ├── synthetic/                CC0 / Apache 2.0 generated fixtures
│   ├── licensed-real/            Explicitly licensed and redacted real invoices
│   ├── private-regression/       Non-public customer fixtures
│   ├── pdf-snapshots/            Baseline PNG rasters for visual regression tests (T-058)
│   └── generators/               Adversarial fixture generators
├── bridges/
│   ├── stripe-invoicing/         Stripe Invoicing → InvoiceKit compliant invoice (T-1200)
│   ├── lago/                     Lago → InvoiceKit (T-1201)
│   ├── maxio/                    Maxio (Chargify + SaaSOptics) → InvoiceKit (T-1202)
│   ├── chargebee/                Chargebee → InvoiceKit (T-1203)
│   └── recurly/                  Recurly → InvoiceKit (T-1204)
├── connectors/
│   ├── odoo/                     Odoo addon (T-1500)
│   ├── ms-dynamics/              Microsoft Dynamics 365 extension (T-1501)
│   ├── sap-b1/                   SAP Business One extension (T-1502)
│   ├── lexware/                  Lexware integration (T-1503)
│   ├── sage/                     Sage integration (T-1504)
│   └── sevdesk/                  sevDesk integration (T-1505)
├── examples/
│   ├── nextjs/                   Next.js reference app (T-1400)
│   ├── django/                   Django reference app (T-1401)
│   ├── rails/                    Rails reference app (T-1402)
│   ├── spring-boot/              Spring Boot reference app (T-1403)
│   ├── asp-net/                  ASP.NET reference app (T-1404)
│   ├── laravel/                  Laravel reference app (T-1405)
│   ├── fastapi/                  FastAPI reference app (T-1406)
│   └── go-chi/                   Go (chi) reference app (T-1407)
├── deploy/
│   ├── docker-compose.yml        Single-host deployment of the full managed stack (T-1300)
│   ├── helm/                     Kubernetes Helm chart (T-1301)
│   └── terraform/                Terraform module for managed cloud provisioning (T-1302)
├── docs/                         Nextra documentation site
└── plans/
    └── PLAN.md                   This file
```

### 4.2 Data flow

```
Source documents (PDF, XML, JSON, CSV, database rows)
    ↓
Intake (layered: embedded XML → digital PDF text → form fields → OCR → small vision model → cloud vision model)
    ↓
Invoice IR (CommercialDocument + ProfileView + JurisdictionExtension + LossinessLedger)
    ↓
Normalize and validate (EN 16931 core + country rule packs + cross-examined witness for AI extraction)
    ↓
Outbound serialization (UBL, CII, Factur-X, XRechnung, Peppol BIS, FatturaPA, KSeF, ZATCA, IRP, CFDI, NF-e, MyInvois, myDATA, ZUGFeRD, etc.) + canonicalization
    ↓
Render PDF/A-3 (Typst, deterministic bytes, embed XML, veraPDF-verified)
    ↓
Sign and timestamp (eIDAS qualified trust service provider, RFC 3161 timestamping authority)
    ↓
Transmit (Peppol AS4 via partner access point, national gateway, or email fallback) with state machine and idempotency
    ↓
Evidence bundle out (.invoicekit signed archive)
    ↓
Archive (S3 Object Lock, Azure WORM, Google Cloud Storage retention, or local file system)
```

### 4.3 The invoice data model (illustrative)

The top-level shape, generated from Rust source of truth:

```typescript
interface CommercialDocument {
  schema_version: '1.0';
  id: DocumentId;                                 // Deterministic fingerprint (BLAKE3)
  document_type: 'invoice' | 'credit_note' | 'debit_note' | 'pro_forma' | 'self_billed';
  issue_date: DateOnly;
  tax_point_date?: DateOnly;
  due_date?: DateOnly;
  document_number: DocumentNumber;                // Per-tenant monotonic sequence
  currency: Iso4217Code;
  supplier: Party;
  customer: Party;
  payee?: Party;
  payment_terms?: PaymentTerms;
  payment_instructions: PaymentInstruction[];     // SEPA, IBAN/BIC, Swiss QR, EPC QR, ZATCA QR
  lines: DocumentLine[];
  tax_summary: TaxCategorySummary[];
  monetary_total: MonetaryTotal;
  attachments?: Attachment[];                     // Content-addressed
  references?: DocumentReference[];               // Purchase order, contract, despatch advice
  notes?: LocalizedString[];
  extensions: JurisdictionExtension[];            // Polymorphic, URN-keyed, opt-in per country
  meta: DocumentMeta;
}

interface JurisdictionExtension {
  urn: string;                                    // e.g., "urn:invoicekit:ext:sa:zatca:2.0"
  payload: Record<string, unknown>;               // Validated by per-country schema, not hardcoded in the core
}
```

The extension layer is **polymorphic and dynamically dispatched**, not a hardcoded struct of country fields. A single struct with `de_xrechnung`, `sa_zatca`, etc. forces the core engine and the foreign function interface boundary to recompile every time any country adds a field — a recurring maintenance cost across 60 jurisdictions. The polymorphic form means:

- The core engine ships untouched when a country evolves.
- Each country crate registers its extension schema by uniform resource name at load time.
- The validator looks up the right per-country schema by URN to type-check the payload.
- Generated bindings (TypeScript, Python, Java, .NET) expose typed helpers per country as separate, opt-in modules — not as fields of the core type.

The model ships as a public JSON Schema plus generated TypeScript, Python, Java, and .NET types. Continuous integration tests every binding for byte-equivalence with the Rust source of truth.

### 4.4 Canonicalization

For any operation that produces a signature, hash, or audit record:

- **XML canonicalization**: XML C14N 1.1 plus an invoice-specific overlay that normalizes namespace prefixes, attribute order, and ignorable whitespace.
- **JSON canonicalization**: RFC 8785 (JSON Canonicalization Scheme) for the invoice JSON form.
- **PDF byte-stable subset**: pinned Typst version; pinned font set (subsetted Inter, DejaVu, Noto); pinned harfbuzz; fixed XMP creation date placeholder; deterministic object ordering. Two renders with the same input produce identical bytes.

Continuous integration runs `canonicalize_xml`, `canonicalize_json`, and `render_pdf` under multiple operating system and architecture combinations, and asserts byte equality.

### 4.5 Validation

Three layers, descending in confidence.

**Schema** — structural validation against XSD and JSON Schema.

**Business rules** — declared by rule pack. Each rule pack names its **validator backend**:

- `rust-native` — hand-written Rust validators (EN 16931 core, ~50 rules; selected country sub-rule-packs once promoted).
- `jvm:kosit`, `jvm:phive`, `jvm:saxon`, `jvm:zatca`, `jvm:irp` — call to a specific JVM sidecar.
- `rest:official` — call the country's official online validator (e.g. Spain VeriFactu live check, France Chorus Pro test endpoint).
- `partner` — defer to the Peppol access point or PAC partner's validator.
- `cli:<binary>` — local binary invocation (some countries ship reference validators as CLI).
- `none` — no public reference validator exists; we ship our own with explicit lower confidence.

Each rule pack also declares a **parity target** (e.g. "99.9% against `jvm:kosit` v1.5.0 on 2026-01 fixture set"). Continuous integration diffs against the named oracle.

**Cross-examined witness** — for AI-extracted invoices, every value goes through the deterministic rules. If the AI says the total is €1234.56 but the lines sum to €1230.45, the AI output is blocked.

Pure-Rust validators are promoted from "wrapped backend call" to "native Rust" one rule pack at a time, gated on 99.9% rule parity against the named oracle. We never gate shipping on this promotion.

### 4.6 Reconciliation engine — the paid moat

Primitives:

- **Deterministic invoice fingerprint**: `blake3(supplier_VAT || customer_VAT || issue_date || document_number || total_amount || currency)`. Dedup key.
- **Idempotency-key envelopes**: every transmission carries one; replays are no-ops.
- **State machine**: `draft → validated → signed → reserved → sent → delivered → acknowledged → rejected → archived`. Per-country sub-states layer in: KSeF `reserved` / `committed`, SDI `accepted` / `rejected`, ZATCA `cleared`, IRP `irn_issued`, etc.
- **Reconciliation API**: customers submit `{internal_id, fingerprint}` lists; we return `{delivered, failed, pending, unknown}` with gateway evidence.
- **Outbox migration**: we ship SQL migrations (Postgres, MySQL, SQLite) for an `invoicekit_outbox` table. One-line install for at-least-once delivery semantics.
- **State-change webhooks plus Server-Sent Events**: pluggable delivery; SSE works behind firewalls.
- **Sealed entity invariant**: post-`delivered`, the invoice is immutable. Corrections create credit notes and replacements.

### 4.7 Evidence bundle format

Directory tree contents (canonical form):

```
manifest.json                    Bundle metadata, schema version, content-address index
signatures/
  manifest.dsse                  Dead Simple Signing Envelope signature over manifest hash
ir.json                          Canonical invoice JSON
canonical/
  invoice.xml                    Generated country-format XML
  invoice.json                   JSON Canonicalization Scheme canonical JSON
render/
  invoice.pdf                    PDF/A-3 with embedded XML
  invoice.html                   Accessible HTML5 render
intake/
  source.{pdf,xml,json,...}      Original source bytes (only if explicitly included)
  extraction-trace.json          AI/OCR layer traces with bounding-box citations
validation/
  trace.json                     Each rule evaluated, result, location
  rulepack-manifest.json         Hashes of every rule pack used
crypto/
  *.sig                          PKCS#7, XAdES, or qualified electronic seal
  rfc3161-tsr.bin                RFC 3161 timestamp response
transmission/
  receipts/*.json                Gateway ACKs and NACKs with timestamps
privacy/
  redaction-map.json             Optional: support-bundle redaction trace
replay.json                      Declarative replay recipe (no shell)
```

Portable packed form: `.ikb` = `tar.zst` with normalized user ID, group ID, modification time, and ordering.

Verification: `invoicekit verify bundle.invoicekit` reproduces validation, asserts signatures, asserts timestamps, asserts content-address consistency. Cryptographically verifiable when paired with qualified electronic signatures from a qualified trust service provider.

### 4.8 Peppol AS4 — practical path

| Phase | Sender | Receiver | Operator status |
|---|---|---|---|
| Phase 2 (initial delivery path) | Partner access point | Partner access point | Use partner; no certification needed |
| Phase 6 onward (per-route promotion) | Native Rust sender with `phase4` as conformance oracle | `phase4` running in a dedicated `validator-phase4` JVM sidecar | Apply for OpenPeppol membership and ISO 27001 in parallel |
| In-push native track | Native Rust sender (T-094) and receiver (T-095), built in parallel with partner adapter; per-route promotion as OpenPeppol conformance passes | Native Rust receiver (T-095) | Apply for OpenPeppol membership and become a tier-2 access point in our own right once ISO 27001 certification completes (parallel background work) |

The ISO 27001 process is the long pole for becoming an access point ourselves. It takes 6 to 12 months even with a consultancy. We start it on day one and run it in the background. Until certification, we rely on partner access points for live delivery.

### 4.9 Intake pipeline

Default is server-side. Browser-side is for light cases only.

| Layer | Server-side default | Browser-side (light variant) |
|---|---|---|
| L1 — Factur-X XML detection | quick-xml + Rust | quick-xml WebAssembly |
| L2 — Digital PDF text | pdf-extract or lopdf | pdf.js |
| L3 — PDF form fields | lopdf | pdf.js |
| L4 — Layout-aware OCR | PaddleOCR PP-StructureV3 (Python or C++ via Rust bindings) | Tesseract WebAssembly |
| L5 — Small vision-language model | SmolDocling-256M ONNX | SmolDocling-256M via Transformers.js (only for short documents) |
| L6 — Large vision-language model | Qwen2.5-VL-7B in our cloud | Not available |
| L7 — Cloud large language model | OpenAI or Anthropic vision API | Not available |

Every extracted field carries `{value, source: {bbox?, ocr_span_id?, pdf_object_id?, model_id}, confidence}`. Deterministic cross-validation: VAT subtotals close, line totals reconcile, supplier and customer VAT IDs validate against the European VIES service or each country's equivalent. Mismatches block AI-only output.

### 4.10 Rendering

The renderer stack:

- **Layer A** — Typst is the underlying engine. Deterministic byte output, programmatic, no headless browser dependency.
- **Layer B** — we ship a TypeScript template language on top so users never see Typst syntax.
- **Layer C** — a web-based what-you-see-is-what-you-get template designer that emits the TypeScript template language (task T-057). Builds in parallel with Layer A/B because it only depends on T-051 (the TypeScript template language compiler).

If Typst proves unable to satisfy embedded XML plus PDF/A-3 conformance for every Factur-X profile we need, the `RenderBackend` trait allows a secondary renderer (for example, a custom Rust PDF builder layered on `lopdf` or `printpdf`).

### 4.11 Test mode — cassette-replay sandbox

Hand-writing 60 mock gateways is not feasible. Tax authorities silently change response formats; manual mocks rot in days. The sandbox is a **declarative cassette-replay proxy** instead.

How it works:

- For each country, we record real interactions against the official or partner sandbox (success path, malformed acknowledgment, certificate chain rejection, IRN-already-issued, SDI 504 timeout, KSeF peak-hour latency, CFDI PAC rejection, etc.).
- The recordings are normalized HTTP traces (`.har`-style) or AS4 traces with timestamps, signatures, and content-addressed payloads scrubbed of personal data.
- Trace files live in `conformance-corpus/cassettes/<country>/` alongside the country crate.
- `transmit-mock` matches a request against the cassette set and replays the exact bytes the real gateway returned.
- Developers can opt into specific failure scenarios with a header (`X-Cassette: ksef/peak-hour-latency`).

The library API is unchanged:

```typescript
const client = new InvoiceKit({ mode: 'test' });
await client.transmit(invoice, { route: 'peppol' });
```

**Honest sandbox claims.** A country's `sandbox` capability is only marked as `proven` if there is automated nightly canary traffic against the real official or partner sandbox plus a cassette set recorded from it. Otherwise the country's sandbox is marked `simulated` — useful for development, but not proof of regulatory acceptance. The capability matrix in Section 3.4 reflects this distinction.

### 4.12 Operations and observability

Every transmission has: trace ID, tenant ID, idempotency key, gateway attempt ID, normalized state transition, raw gateway receipt hash, retry and dead-letter metadata.

Service-level objectives are defined per operation: `validate`, `render`, `transmit-enqueue`, `gateway-accepted`, `archive-write`, `webhook-deliver`. Gateway legal acceptance is never conflated with API availability.

The validator worker, the managed API, and the transmission worker all expose OpenTelemetry traces and metrics. Per-gateway dashboards live in the managed layer. Replay and admin tools surface stuck transmissions and dead-letter queues for ops triage.

---

## 5. Public surface — developer experience

### 5.1 First-touch experience

For end users on Node:

```
$ npx invoicekit init
✓ Detected: Node + TypeScript + ESM
? Country (auto-detected from package.json: DE)
? Default supplier VAT ID: DE123456789  (VIES lookup confirms)
? Sandbox or live? [sandbox]

✓ Generated:
  - invoicekit.config.ts
  - .env.example  (INVOICEKIT_API_KEY=test_*)
  - examples/first-invoice.ts

Try it:  npx invoicekit validate examples/first-invoice.ts --profile=peppol-bis
Then:    npx invoicekit send examples/first-invoice.ts --mode=sandbox
```

We use `bunx` internally for our own development (per project tooling rules) but the public-facing default is `npx` because Node and npm have ten times the install base.

The `invoicekit doctor` command runs before any expensive setup:

```
$ invoicekit doctor --country=DE --profile=xrechnung
✓ engine (native binding loaded)
✓ reference validator worker reachable
✓ rule packs current (last updated 2026-05-22)
✓ PDF/A verifier (veraPDF 1.x)
✓ API key scopes: validate, render, transmit-sandbox
✓ country DE capabilities: serialize, validate, render, sandbox, partner-live, inbound, archive, correction
```

### 5.2 Library API (TypeScript)

```typescript
import { createInvoiceDraft } from '@invoicekit/core';
import { renderPdf } from '@invoicekit/render';
import { InvoiceKitClient } from '@invoicekit/managed';

const client = new InvoiceKitClient({ apiKey: process.env.INVOICEKIT_API_KEY });

const draft = createInvoiceDraft({
  supplier: { vat: 'DE123456789' },
  customer: { vat: 'FR987654321' },
  currency: 'EUR',
});

// Enrichment is explicit (not auto in builder)
const enriched = await client.enrich(draft, {
  sources: ['vies', 'gleif', 'cache'],
  cache: 'tenant',
  consent: true,
});

// Add lines and finalize. `addLine` and `finalize` are pure functions on the draft;
// finalize() is what produces the immutable CommercialDocument.
const invoice = finalizeInvoice(
  addLine(enriched, { description: 'Consulting', quantity: '5', unitPrice: '200.00', vatCategory: 'S', vatRate: '19.00' })  // strings, not numbers — per §2.3 monetary boundary rule
);

const validation = await client.validate(invoice, { profile: 'peppol-bis-3.0' });
if (!validation.ok) console.error(validation.report);

const pdf = await renderPdf(invoice, { template: 'modern', profile: 'factur-x-en16931' });

const result = await client.transmit(invoice, {
  route: 'auto',
  fallback: ['peppol', 'fr-ppf', 'email'],
});
```

### 5.3 Command-line interface

```
invoicekit doctor                                  Diagnostics: what's missing for your country
invoicekit init                                    Walk through first invoice
invoicekit convert in.pdf --to=xrechnung-3.0       Auto-detect input format
invoicekit validate in.xml --profile=peppol-bis    Diagnostics with citations
invoicekit render invoice.json --to=pdf            PDF/A-3 with embedded XML
invoicekit send invoice.json --route=auto          Discover and send (uses managed API)
invoicekit verify bundle.invoicekit                Cryptographically verifiable re-verification of a signed evidence bundle
invoicekit fuzz                                    Adversarial generator
invoicekit benchmark                               Performance dashboard
invoicekit capabilities --from=DE --to=FR --date=2027-01-01
                                                   What rules apply for that direction on that date
invoicekit explain BR-CO-10                        Plain-language rule explainer with formula
invoicekit validate file.xml --explain             Validator explain-plan trace (rule evaluation order, inputs, decisions, citations)
invoicekit replay bundle.invoicekit                Replay the full pipeline from a `.invoicekit` archive
invoicekit migrate-archive --from-version=1.0 --to-version=2.0  Forward-migrate a stored archive when the IR major version bumps
invoicekit repl                                    Interactive session — build, validate, render, send via mock gateway in one flow
invoicekit rulepack update                         Refresh signed, dated rule packs
```

### 5.4 Language Server Protocol

A language server for invoicing. Visual Studio Code, Cursor, Neovim, and Helix extensions. Hover any business term (BT-* or BG-*) to read the EN 16931 explanatory text. Click any validation diagnostic to jump to source. Auto-complete code list values (VAT category, payment means, country codes).

### 5.5 REST API (thin shim)

For non-binding clients:

```
POST /v1/invoices                                  Create (idempotent via Idempotency-Key)
POST /v1/invoices/:id/validate                     Re-validate against current rule pack
POST /v1/invoices/:id/render                       Render PDF
POST /v1/invoices/:id/transmit                     Transmit; returns 202 plus tracking ID
GET  /v1/transmissions/:id                         Current state-machine state
POST /v1/reconcile                                 Bulk fingerprint reconciliation
GET  /v1/bundles/:id                               Download .invoicekit
POST /v1/bundles/verify                            Server-side verification with signed proof
GET  /v1/capabilities                              Country / profile / date matrix lookup
```

---

## 6. Build sequence

The build is organized as parallel tracks. Each track has its own dependency chain. Agents pick up the next unblocked task on any track.

### Phase ↔ Track map

Section 3.3 frames the work in phases (chronological commitments). Section 6 below frames it in tracks (parallel work streams). They are two views of the same plan. Mapping:

| Phase (§3.3) | Tracks that contribute |
|---|---|
| Phase 0 — Engine and foundation | Track 0 (foundation), Track 1 (engine primitives), Track 2 (reference validator), Track 6 (reconciliation, state machine, evidence) — partial; Track 11 (hosted layer foundations: T-130, T-131, T-132, T-133) |
| Phase 1 — Family A coverage | Track 3 (formats), Track 4 (rendering) |
| Phase 2 — Live Peppol delivery | Track 7 (Peppol), T-074 (mock gateway) finished |
| Phase 2.5 — Country feasibility manifests | Track 7.5 (manifests T-770…T-7A5) |
| Phase 3 — National clearance and reporting crates | Track 8 — first archetype lock-in (T-800, T-801, T-802) sequential, then Waves 1-3 in parallel |
| Phase 4 — Intake pipeline (after outbound is GA) | Track 5 |
| Phase 5 — Developer experience surface | Track 9 |
| Phase 6 — Hosted managed layer | Track 11 (the rest) |
| Phase 7 — Conformance and trust infrastructure | Track 10 + the public validator (T-035) |
| Phase 8 — Distribution and ecosystem | Track 12 (billing bridges), Track 14 (reference demo apps), Track 15 (ERP connectors) |
| Phase 9 — Deployment | Track 13 (on-premise stack) |

A task's track tells an agent who can do it in parallel; a task's phase tells the agent what other tasks must be done before its country/feature can claim "shipped."

### Track 0 — Foundation (sequential, no dependencies on country work)

| Task | Description | Effort |
|---|---|---|
| T-001 | Cargo workspace, continuous integration scaffolding, code-of-conduct, contributing guide, security policy | 1 week |
| T-002 | License (Apache 2.0), signed releases, software bill of materials, dependency scanning | 1 week |
| T-005 | ISO 27001 readiness engagement starts (background, 6–12 months) | 0 days direct work |
| T-006 | Compliance source-watch bot (monitors official sources, opens issues on rule changes) | 1 week |
| T-006a | `invoicekit capabilities` complete specification: capability schema (per country / profile / date / route direction), stale-data and auto-downgrade semantics, source confidence rules (official-source / partner-source / community), JSON and human output formats, integration with source-watch manifests | 1 week |
| T-007 | Performance regression budget. Every pull request runs the benchmark suite; if any tracked operation (validate, render, canonicalize, transmit-enqueue, fingerprint, IR round-trip) regresses more than 10 percent versus the rolling 30-day median, the build fails. Benchmark results published to `benchmark.invoicekit.org`. Uses `criterion` for Rust and `vitest bench` for TypeScript. | 1 week | T-001 |
| T-008 | Fuzz continuous integration. Every pull request runs five minutes of `cargo-fuzz` against the XML parser, JSON parser, PDF embedder, and canonicalizer. Crashes block merge. Coverage regressions block merge. Corpus accumulates in `conformance-corpus/fuzz/`. | 1 week | T-001 |

### Track 1 — Engine primitives

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-010 | Layered invoice model in Rust (`CommercialDocument`, `ProfileView`, `JurisdictionExtension`, `LossinessLedger`) | 2 weeks | T-001 |
| T-011 | Public JSON Schema generation from Rust types | 1 week | T-010 |
| T-012 | TypeScript type generation from JSON Schema | 3 days | T-011 |
| T-013 | `invopop/gobl` bidirectional adapter | 2 weeks | T-010 |
| T-014 | `money` crate (`rust_decimal` based) | 1 week | T-001 |
| T-015 | `codelists` crate (signed, effective-dated) | 1 week | T-001 |
| T-016 | `tax-calculation` crate (deterministic decimal arithmetic with formal trace) | 2 weeks | T-014, T-015 |
| T-017 | `rulepack` crate (signed manifest format, source registry) | 1 week | T-001 |
| T-018 | Codelist updater with provenance checksums | 1 week | T-015, T-017 |
| T-019 | XML canonicalization C14N 1.1 | 1 week | T-010 |
| T-020 | JSON canonicalization (RFC 8785) | 3 days | T-010 |
| T-021 | Property-based canonical JSON tests and XML canonicalization tests against synthetic IR | 1 week | T-019, T-020 |
| T-021a | Real IR ↔ UBL/CII XML round-trip tests (uses real serializers once Track 3 ships) | 1 week | T-040, T-041, T-019, T-020 |
| T-022 | Deterministic invoice fingerprint (BLAKE3) | 2 days | T-010, T-014, T-015 |
| T-023 | Stable engine ABI contract + cross-language golden fixtures | 2 weeks | T-010, T-016 |
| T-024 | C ABI surface (`invoicekit-ffi`) | 1 week | T-023 |
| T-025 | WebAssembly artifact (`invoicekit-wasm`) | 1 week | T-023 |
| T-026 | Schema evolution + automatic migration. When the IR major version bumps (v1 → v2), ship a typed `migrate(invoice_v1) -> Result<invoice_v2, MigrationReport>` function and a command-line `invoicekit migrate-archive --from-version=N --to-version=M`. Migration is reversible where semantics allow; the report enumerates fields that could not be migrated cleanly. Continuous integration runs every migration over every prior version's fixture set on every PR. | 2 weeks | T-010 |

### Track 2 — Reference validator and validation

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-030 | Validator sidecar protocol + per-domain workers (`validator-kosit`, `validator-phive`, `validator-saxon`). Split by JVM dependency boundary so adding ZATCA / IRP later doesn't collide. JSON-RPC contract. | 2 weeks | T-001, T-032 |
| T-031 | EN 16931 hand-written Rust validator (~50 core rules). Validated against the reference JVM worker as oracle. | 3 weeks | T-010, T-017, T-030, T-032 |
| T-032 | Validation result schema (rule ID, severity, business-term, JSON Pointer or XPath location, suggested fix, citation, optional `trace` field per T-032a) | 1 week | T-010, T-017 |
| T-032a | Validator explain-plan trace. For any validation result, emit a structured trace of every rule that was evaluated, in evaluation order, with `{rule_id, evaluated_at_path, inputs, decision, citations}` per step. Output both machine-readable JSON and a human-readable Markdown narrative. Command line: `invoicekit validate file.xml --explain`. Powers the language server diagnostic hover, the documentation site, and the customer support tooling. | 1 week | T-031, T-032 |
| T-033 | Browser/edge validator capability matrix per country/profile/date. Reports: `serialize`, `local_validate`, `reference_validate`, `requires_service`, `requires_cli`, `unavailable_in_wasm`. External validator backends must return `RequiresExternalBackend` errors, never panic or silently downgrade. | 1 week | T-030, T-031 |
| T-034 | Time-travel validation (date-pinned rule packs) | 1 week | T-017, T-031 |
| T-035 | Public free validator web UI (dual mode: local browser-only and server-assisted reference) | 2 weeks | T-030, T-033 |

### Track 3 — Format family A (UBL, CII, EN 16931, Peppol BIS, Factur-X, XRechnung)

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-040 | Universal Business Language 2.1 parser and serializer | 2 weeks | T-010, T-019 |
| T-041 | Cross Industry Invoice parser and serializer | 2 weeks | T-010, T-019 |
| T-042 | Peppol BIS 3.0 projection | 1 week | T-040 |
| T-043 | Peppol PINT international projection | 1 week | T-040 |
| T-044 | Factur-X / ZUGFeRD all six profiles | 2 weeks | T-040, T-041 |
| T-045 | German XRechnung 3.x projection | 1 week | T-040 |
| T-046 | Lossiness ledger generator | 1 week | T-040, T-041, T-042, T-043, T-044, T-045 |
| T-047 | Format auto-detection (sniff input bytes, return format identifier) | 1 week | T-040, T-041 |

### Track 4 — Rendering

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-050 | Typst integration as Rust crate dependency | 1 week | T-010 |
| T-051 | TypeScript template language compiles to Typst | 3 weeks | T-050 |
| T-052 | veraPDF adapter for conformance verification | 1 week | T-050 |
| T-053 | PDF/A-3 dictionary post-processing. Acceptance fixtures: 5 ZUGFeRD MINIMUM, 5 BASIC WL, 5 BASIC, 5 EN 16931, 5 EXTENDED, 5 XRECHNUNG profile invoices. Must pass `verapdf --profile=3b` and `--profile=3u`. Decision rule: prefer Typst upstream pull request when the fix is generic (XMP metadata, attachment relationships); use `lopdf` local post-processing when the fix is invoice-format-specific. Post-processing pipeline ships as `crates/render-pdf-postproc`. | 4 weeks | T-052 |
| T-054 | Factur-X XML embedding into PDF/A-3 attachment | 1 week | T-053 |
| T-055 | Deterministic byte-stable rendering subset | 1 week | T-054 |
| T-056 | Accessible HTML5 rendering pipeline (WCAG-conformant) | 1 week | T-051 |
| T-057 | Web what-you-see-is-what-you-get template designer (emits the TypeScript template language). Single-page web app served by the docs site; integrates the Storybook preview from T-114. | 3 weeks | T-051 |
| T-058 | Visual regression tests for every template × every profile × every country output. Renders each fixture to PDF, rasterizes deterministically (via `mupdf-tools` or `pdfium`), compares pixel-by-pixel against baseline PNGs in `conformance-corpus/pdf-snapshots/`. Differences surface in the pull request with side-by-side diff images. Baselines are version-controlled; updates require explicit human sign-off in the PR. | 1 week | T-055 |

### Track 5 — Intake pipeline

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-060 | Layer 1 — Factur-X XML detection and extraction from PDF | 1 week | T-040, T-041, T-050 |
| T-061 | Layer 2 — Digital PDF text extraction | 1 week | T-001 |
| T-062 | Layer 3 — PaddleOCR integration (server-side default) | 2 weeks | T-061 |
| T-063 | Layer 4 — SmolDocling-256M ONNX integration | 2 weeks | T-062 |
| T-064 | Layer 5 — Qwen2.5-VL-7B cloud inference adapter | 1 week | T-063 |
| T-065 | Cross-examined witness flow (deterministic re-validation) | 2 weeks | T-031, T-064 |
| T-066 | Bounding-box citation taxonomy | 1 week | T-062 |
| T-067 | PII/GDPR redactor for support bundles | 1 week | T-010 |

### Track 6 — Reconciliation, state machine, evidence

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-070 | Gateway adapter trait and normalized gateway error taxonomy | 1 week | T-010 |
| T-070a | Extensible transmission state model and transition contract (per-country sub-states layer in cleanly) | 1 week | T-070 |
| T-071 | Outbox SQL schema, idempotency model, retry policy, dead-letter states | 2 weeks | T-022, T-070a |
| T-072 | Transmission worker with backoff, rate limits, circuit breakers, structured gateway logs | 2 weeks | T-071, T-073, T-074 |
| T-073 | State machine implementation (per-country sub-states layered on T-070a) | 2 weeks | T-070a |
| T-074a | Cassette recorder, scrubber, matcher, and scenario metadata schema. The recorder produces deterministic cassettes; the scrubber removes personal data; the matcher routes requests by method + path + body fingerprint. | 2 weeks | T-070, T-120 |
| T-074 | Mock gateway (`transmit-mock`) — first `GatewayAdapter` implementation, drives the declarative cassette-replay engine using the recorder/matcher framework. Acceptance criterion: includes at least 2 baseline cassettes (one success, one failure) recorded by T-074a's recorder. | 1 week | T-074a |
| T-074b | `GatewayAdapter` contract test suite backed by cassettes — required scenarios: idempotent replay, duplicate submission, timeout, malformed receipt, auth failure, certificate rejection, rate limit, delayed async receipt, unknown response field, gateway maintenance page, partner error translation | 1 week | T-074, T-073 |
| T-074c | Nightly sandbox drift canary: replay live official/partner sandbox calls, diff normalized responses, open source-watch issues on drift | 1 week | T-006, T-074a |
| T-074d | Sandbox / production parity diff. Nightly job sends the same invoice through both the official sandbox and the live partner production endpoint (where customer consent exists), diffs the normalized responses, alerts on drift. Catches "the regulator silently changed something" before customers do. Shares infrastructure with T-074c. | 1 week | T-074a, T-074c |
| T-075 | Reconciliation API and outbox SQL migrations (Postgres, MySQL, SQLite) | 1 week | T-071 |
| T-076 | Webhook dispatcher with replay protection and idempotency | 1 week | T-073 |
| T-077 | Server-Sent Events stream for ACK delivery | 1 week | T-073 |
| T-080 | Signed evidence bundle format (`.invoicekit`, packed form `.ikb`). Contains canonical-XML output (one of T-040/T-041/T-044/T-045 produces it, T-019 canonicalizes), canonical JSON (T-020), byte-stable rendered PDF (T-055). Bundle layout per §4.7. | 2 weeks | T-019, T-020, T-022, T-031, T-040, T-041, T-055, T-073 |
| T-081 | Pluggable archive backend (S3 Object Lock, Azure WORM, Google Cloud Storage retention, local file system, IPFS hash) | 2 weeks | T-080 |
| T-082 | RFC 3161 timestamping integration with a trusted timestamping authority | 1 week | T-080 |
| T-083 | Stable signing API + `invoicekit-signer-agent` local proxy. Engine calls signer over a local Unix socket or HTTPS endpoint. The same signing API also routes to in-process software signing for non-regulated use cases. | 2 weeks | T-080 |
| T-083a | eIDAS qualified signature provider adapter (one of many adapters of T-083). Customer plugs in their qualified trust service provider; `invoicekit-signer-agent` keeps keys on-premise. | 2 weeks | T-083 |
| T-083b | Country-specific signer adapters (umbrella). Each sub-task is a separate signing adapter plugged into the T-083 substrate; each is the SINGLE owner of that country's cryptographic stamping — the Track 8 country crate consumes it, never re-implements it. | 4 weeks total (sub-tasks parallelizable) | T-083 |
| T-083b1 | Saudi Arabia ZATCA Phase 2 cryptographic stamp adapter (ECDSA secp256k1 over the canonical TLV payload; output is the base64 stamp returned to the country crate). | 1 week | T-083 |
| T-083b2 | Mexico CFDI 4.0 PAC signing flow adapter (sends to a Mexican PAC partner, receives the timbre fiscal digital). | 1 week | T-083 |
| T-083b3 | Poland KSeF certificate flow adapter (signs with the customer's qualified certificate held via signer-agent). | 1 week | T-083 |
| T-083b4 | Italy SDI / Aruba qualified certificate flow adapter. | 1 week | T-083 |
| T-083b5 | Brazil NF-e federal certificate flow adapter (A1/A3 certificates, SEFAZ-specific signing). | 1 week | T-083 |
| T-084 | `invoicekit verify` library and CLI | 1 week | T-080, T-082, T-083 |
| T-085 | Replay-from-bundle. Given a `.invoicekit` archive, re-run the entire pipeline (intake → IR → validation → render → transmission to mock gateway) and assert that the bytes match the originally-recorded outputs. Critical for audit ("prove this invoice could have produced this output on this date with these rule packs") and for debugging customer-reported anomalies. Command line: `invoicekit replay bundle.invoicekit --against=<recorded-snapshot>`. Lives in `crates/replay`. | 1 week | T-080, T-084 |

### Track 7 — Peppol live delivery

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-090 | Peppol participant lookup (SMP/SML client) | 1 week | T-042 |
| T-091 | Partner Peppol access point adapter (selection of partner: Storecove / ecosio / B2BRouter based on pricing and coverage) | 2 weeks | T-072, T-090 |
| T-092 | `phase4` reference adapter running in the dedicated `validator-phase4` JVM sidecar (per §2.6) | 1 week | T-091 |
| T-093 | Peppol inbound receiver service | 2 weeks | T-091 |
| T-094 | Native Rust AS4 sender. Built in parallel with partner adapter (T-091). Promoted to live delivery per-route as it passes the OpenPeppol conformance suite, differentially tested against the `validator-phase4` JVM sidecar. | 6 weeks | T-090, T-092 |
| T-095 | Native Rust AS4 receiver. Built in parallel with partner adapter. Promoted to live delivery as it passes inbound conformance tests against `phase4`. | 8 weeks | T-093, T-094 |

### Track 7.5 — Country feasibility manifests (Phase 2.5)

Per §3.3 Phase 2.5, every country gets a feasibility manifest before its report crate starts. The manifest is the agent-work-unit input defined in §8.1. Manifests are 1–3 day tasks; they can be produced in parallel. Each manifest must include the initial fixture set (5 valid + 5 invalid) and at least one baseline sandbox cassette when a sandbox exists.

| Task ID | Country | Effort |
|---|---|---|
| T-770 | Poland | 1–3 days |
| T-771 | Saudi Arabia | 1–3 days |
| T-772 | Belgium | 1–3 days |
| T-773 | Italy | 1–3 days |
| T-774 | France | 1–3 days |
| T-775 | Spain | 1–3 days |
| T-776 | Greece | 1–3 days |
| T-777 | UAE | 1–3 days |
| T-778 | India | 1–3 days |
| T-779 | Mexico | 1–3 days |
| T-780 | Brazil (multiple sub-manifests for NF-e + per-municipal NFS-e) | 3–7 days |
| T-781 | Malaysia | 1–3 days |
| T-782 | Turkey | 1–3 days |
| T-783 | Romania | 1–3 days |
| T-784 | Hungary | 1–3 days |
| T-785 | Japan (Qualified Invoice System) | 1–3 days |
| T-786 | Chile | 1–3 days |
| T-787 | Colombia | 1–3 days |
| T-788 | Peru | 1–3 days |
| T-789 | Argentina | 1–3 days |
| T-790 | Ecuador | 1–3 days |
| T-791 | Costa Rica | 1–3 days |
| T-792 | Dominican Republic | 1–3 days |
| T-793 | Egypt | 1–3 days |
| T-794 | Israel | 1–3 days |
| T-795 | Indonesia | 1–3 days |
| T-796 | Philippines | 1–3 days |
| T-797 | Vietnam | 1–3 days |
| T-798 | Thailand | 1–3 days |
| T-799 | South Korea | 1–3 days |
| T-7A0 | China | 1–3 days |
| T-7A1 | Taiwan | 1–3 days |
| T-7A2 | Kenya | 1–3 days |
| T-7A3 | Nigeria | 1–3 days |
| T-7A4 | South Africa | 1–3 days |
| T-7A5 | Portugal (national reporting alongside Peppol) | 1–3 days |

All manifests depend on T-006 (source-watch bot) + T-074a (cassette recorder, so the agent can record baseline cassettes against the country's sandbox). The corresponding Track 8 country task always names its manifest in its `depends on` list.

### Track 8 — National report crates

National crates do **not** start from foundation alone. Each requires (a) the country's Phase 2.5 feasibility manifest, (b) the cassette framework from T-074a, and (c) a locked archetype trait. Common foundation dependencies for every Track 8 crate: T-010, T-017, T-070, T-070a, T-073, T-074, T-074a, T-080, T-083.

**Archetype lock-in (sequential, runs before any Wave starts)**. Three archetypes ship one after the other. Each defines the trait surface its archetype will share, the cassette taxonomy for that archetype, and the per-archetype testing contract.

| Task ID | Crate | Archetype | Effort | Depends on |
|---|---|---|---|---|
| T-800 | `report-pl-ksef` | Async clearance archetype: submit → reserve → poll → commit → cancel → correct. The Track 8 crate consumes T-083b3 for KSeF certificate signing; it does NOT re-implement signing. | 3 weeks | foundation tasks + T-770 (Poland manifest) + T-083b3 |
| T-801 | `report-sa-zatca` | Cryptographic archetype: orchestrates the ZATCA Phase 2 flow (UBL canonicalization, TLV QR generation, certificate-chain orchestration). The Track 8 crate consumes T-083b1 for the actual ECDSA secp256k1 stamp; it does NOT re-implement the cryptographic primitives. This is the heaviest of the three archetypes because the orchestration logic + TLV layout + state machine are large. | 6–8 weeks | foundation tasks + T-771 (Saudi Arabia manifest) + T-083b1 |
| T-802 | `report-be-pep` | Peppol-mandate / CIUS overlay archetype: thin wrapper over Family A. Used as the Belgium implementation AND as the canonical example for every later Peppol-overlay country. | 1 week | foundation tasks + T-091 + T-772 (Belgium manifest) |

**Wave 1 — Regulatory urgency** (parallel; starts only after archetype lock-in is complete):

| Task ID | Crate | Description | Archetype lineage | Effort |
|---|---|---|---|---|
| T-810 | `report-it-sdi` | Italy SDI clearance and receipts | async clearance | 3 weeks |
| T-811 | `report-fr-ctc` | France PA / PDP e-invoicing and e-reporting (PDP is Peppol-shaped; PPF reporting is async) | async clearance + Peppol overlay | 3 weeks |
| T-812 | `report-es-verifactu` | Spain VeriFactu and FacturaE | async clearance | 2 weeks |
| T-813 | `report-gr-mydata` | Greece myDATA reporting | async clearance (reporting variant) | 2 weeks |
| T-814 | `report-ae-pint` | UAE PINT-AE national onboarding | Peppol overlay | 1 week |

**Wave 2 — Large markets** (parallel; starts after Wave 1 has at least one crate at beta):

| Task ID | Crate | Description | Archetype lineage | Effort | Extra deps |
|---|---|---|---|---|---|
| T-820 | `report-in-gst` | India IRP, GST, e-Waybill | async clearance | 3 weeks | |
| T-821 | `report-mx-cfdi` | Mexico CFDI 4.0 via PAC partner | async clearance + cryptographic | 3 weeks | T-083b2 |
| T-822 | `report-br-nfe` | Brazil NF-e (federal goods); NFS-e (services) requires per-municipal sub-flows — manifest must pin scope | async clearance + cryptographic | 4 weeks NF-e plus 2-4 weeks per NFS-e flow | T-083b5 |
| T-823 | `report-my-myinvois` | Malaysia MyInvois | async clearance | 2 weeks | |
| T-824 | `report-tr-efatura` | Turkey e-Fatura | async clearance | 2 weeks | |
| T-825 | `report-ro-efactura` | Romania RO e-Factura | async clearance | 2 weeks | |
| T-826 | `report-hu-nav` | Hungary NAV Online Invoicing | async clearance (reporting variant) | 2 weeks | |
| T-827 | `report-jp-qis` | Japan Qualified Invoice System | Peppol overlay (operates alongside Family A) | 2 weeks | |

**Wave 3 — Latin America, MENA, APAC, Africa** (parallel; starts after Wave 1 + Wave 2 are progressing):

| Task ID | Crate | Description | Archetype lineage | Effort |
|---|---|---|---|---|
| T-830 | `report-cl-dte` | Chile SII DTE | async clearance | 2 weeks |
| T-831 | `report-co-dian` | Colombia DIAN | async clearance | 2 weeks |
| T-832 | `report-pe-sunat` | Peru SUNAT | async clearance | 2 weeks |
| T-833 | `report-ar-afip` | Argentina AFIP | async clearance | 2 weeks |
| T-834 | `report-ec-sri` | Ecuador SRI | async clearance | 2 weeks |
| T-835 | `report-cr-hacienda` | Costa Rica Hacienda | async clearance | 2 weeks |
| T-836 | `report-do-dgii` | Dominican Republic DGII | async clearance | 2 weeks |
| T-837 | `report-eg-eta` | Egypt ETA | async clearance | 2 weeks |
| T-838 | `report-il-ita` | Israel Tax Authority | async clearance | 2 weeks |
| T-839 | `report-id-djp` | Indonesia DJP Online | async clearance | 2 weeks |
| T-840 | `report-ph-bir` | Philippines BIR EIS | async clearance | 2 weeks |
| T-841 | `report-vn-gdt` | Vietnam GDT | async clearance | 2 weeks |
| T-842 | `report-th-rd` | Thailand RD | async clearance | 2 weeks |
| T-843 | `report-kr-nts` | South Korea NTS | async clearance | 2 weeks |
| T-844 | `report-cn-fapiao` | China Golden Tax / Fapiao | async clearance + cryptographic | 3 weeks |
| T-845 | `report-tw-mof` | Taiwan MOF | async clearance | 2 weeks |
| T-846 | `report-ke-etims` | Kenya eTIMS | async clearance | 2 weeks |
| T-847 | `report-ng-firs` | Nigeria FIRS | async clearance | 2 weeks |
| T-848 | `report-za-sars` | South Africa SARS | async clearance | 2 weeks |
| T-849 | `report-pt-cius` | Portugal national reporting (alongside Peppol) | async clearance | 1 week |

Acceptance criteria for every Wave 1/2/3 task: pass the country's fixture set (from its Phase 2.5 manifest) with 100% expected validation outcomes; cassette set covers at least success path and one canonical error; reconciliation API returns correct state for each tested transmission. With agents in parallel the wave compresses to its longest crate.

### Track 9 — Developer experience surface (rolling, parallel with engine work)

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-100 | `invoicekit` command-line binary | 2 weeks rolling | T-031 |
| T-100a | `invoicekit repl` interactive session. Wraps the CLI commands in a `rustyline` shell: build invoices, validate, render, send through mock gateway, all in one process. Powers documentation walkthroughs and quick exploration. | 1 week | T-100 |
| T-101 | `invoicekit doctor` | 3 days | T-100 |
| T-102 | `invoicekit init` interactive | 3 days | T-100 |
| T-103 | TypeScript SDK (`@invoicekit/core`, `@invoicekit/render`, `@invoicekit/managed`) | 2 weeks | T-023, T-024 |
| T-104 | Python SDK (`pyo3` + `maturin`) | 2 weeks | T-023, T-024 |
| T-105 | Java SDK (JNI / Foreign Function and Memory API over C ABI, with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-106 | .NET SDK (P/Invoke over C ABI, with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-107 | Go SDK (cgo with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-108 | Browser bundle (`wasm-bindgen` for Cloudflare Workers, Deno, Bun, browser) | 1 week | T-025 |
| T-109 | REST shim (Axum) | 2 weeks | T-023, T-031 |
| T-109a | OpenAPI 3.1 specification auto-generated from Rust types via `utoipa` or equivalent. The spec is the contract; customers can generate their own bindings; we publish it on every release with a content hash so they can pin. Spec served at `https://api.invoicekit.org/openapi.json`. | 1 week | T-109 |
| T-110 | Reverse-proxy sidecar container | 1 week | T-109 |
| T-111 | Invoice language server (Language Server Protocol) | 3 weeks | T-031, T-032 |
| T-112 | VS Code, Cursor, Neovim, Helix extensions | 1 week | T-111 |
| T-113 | Documentation site (Nextra) with per-rule pages and per-country guides | 4 weeks rolling | T-031 |
| T-114 | Storybook for templates | 2 weeks | T-051 |
| T-115 | GitHub Actions for invoice validation | 1 week | T-035 |
| T-116 | Model Context Protocol server for AI development tools (Claude Code, Cursor, Aider, Continue) | 1 week | T-031 |

### Track 10 — Conformance corpus

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-120 | Corpus licensing and redaction policy, fixture metadata schema | 3 days | T-002 |
| T-121 | Adversarial generator (Rust) — generates pathological invoices in IR and emits via every serializer for differential testing | 2 weeks | T-010, T-040, T-041 |
| T-122 | Synthetic public corpus version 0.5 (500+ adversarial invoices) | 2 weeks | T-121 |
| T-123 | Differential test harness — runs all serializers + both pure-Rust and reference-worker validators against the synthetic + licensed real corpus, diffs results, publishes parity dashboard | 2 weeks | T-030, T-031, T-032, T-040, T-041 |
| T-124 | Public benchmark dashboard | 1 week | T-123 |

### Track 11 — Hosted managed layer

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-130 | Tenant model, scoped API keys, OIDC, RBAC, audit-event schema | 2 weeks | T-001 |
| T-131 | Envelope encryption with key-management-service per tenant, key rotation, data residency tags | 2 weeks | T-130 |
| T-132 | Webhook signing (HMAC-SHA256, `InvoiceKit-Signature: t=<unix>,v1=<hex>` header — same shape Stripe popularized), replay protection (`InvoiceKit-Timestamp` + 5-min window), event-delivery idempotency | 1 week | T-130 |
| T-133 | Software bill of materials, dependency scanning, signed releases, security advisory process | 1 week | T-002 |
| T-134 | API gateway, authentication, rate limiting | 2 weeks | T-130 |
| T-135 | Customer dashboard (audit log, usage, errors) | 3 weeks | T-130 |
| T-136 | OpenTelemetry tracing, metrics, log redaction, per-gateway dashboards | 2 weeks | T-072 |
| T-137 | Replay and admin tooling for stuck transmissions and dead-letter queues | 1 week | T-136 |
| T-138 | Status page and incident tooling | 1 week | T-136 |
| T-139 | Support ticket integration | 1 week | T-135 |
| T-140 | Stripe integration for our own customer invoicing | 1 week | T-130 |
| T-141 | Hot-reloadable rule packs. Managed service picks up signed rule pack updates without a restart via inotify or file watcher plus atomic file swap. Critical for rule drift maintenance: when KoSIT releases an updated XRechnung Schematron, we deploy without a maintenance window. | 1 week | T-017, T-018 |
| T-142 | Customer-facing audit log. Customers can query every action taken on their data via our APIs, exportable as CSV or JSON, signed for evidence purposes. Required for SOC 2 / ISO 27001 customer evidence. Lives behind `/v1/audit/events` with pagination + filtering. | 1 week | T-130, T-136 |

### Track 12 — Billing-platform bridges

Each bridge is a thin event listener + transformer + transmission caller. The billing platform's invoice event is translated into our `CommercialDocument` model, validated and rendered against the destination country, transmitted through the engine, and reconciled back. Distribution multiplier: each bridge unlocks the existing user base of the billing platform.

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-1200 | Stripe Invoicing bridge — webhook listener + transformer + Stripe-receipt write-back | 2 weeks | T-031, T-072, T-091 |
| T-1201 | Lago bridge | 2 weeks | T-031, T-072, T-091 |
| T-1202 | Maxio (Chargify + SaaSOptics) bridge | 2 weeks | T-031, T-072, T-091 |
| T-1203 | Chargebee bridge | 2 weeks | T-031, T-072, T-091 |
| T-1204 | Recurly bridge | 2 weeks | T-031, T-072, T-091 |

### Track 13 — On-premise deployment

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-1300 | Single-host deployment via `docker-compose.yml` bringing up Postgres, all JVM validator sidecars, the signer-agent, archive backend, and managed-API server | 1 week | T-130, T-030, T-083 |
| T-1301 | Kubernetes Helm chart for production-grade multi-node deployment | 1 week | T-1300 |
| T-1302 | Terraform module for managed-cloud provisioning (AWS, Azure, GCP) | 1 week | T-1301 |

### Track 14 — Reference demo applications

Eight working integrations, one per common stack, each landing a German XRechnung in under five minutes from a clean clone. The repositories are the canonical answer to "how do I integrate this in $stack." Each repo is maintained on the same release cadence as the engine.

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-1400 | Next.js reference app | 1 week | T-103 |
| T-1401 | Django reference app | 1 week | T-104 |
| T-1402 | Rails reference app (via REST shim) | 1 week | T-109 |
| T-1403 | Spring Boot reference app | 1 week | T-105 |
| T-1404 | ASP.NET reference app | 1 week | T-106 |
| T-1405 | Laravel reference app (via REST shim) | 1 week | T-109 |
| T-1406 | FastAPI reference app | 1 week | T-104 |
| T-1407 | Go (chi) reference app | 1 week | T-107 |

### Track 15 — Enterprise resource planning connectors

Each connector is a thin wrapper around the engine, packaged for the host ERP's marketplace and update mechanism. A non-technical ERP user installs the connector, configures their VAT identifier, and starts issuing compliant invoices.

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-1500 | Odoo addon (Python module, Odoo App Store) | 2 weeks | T-104, T-109 |
| T-1501 | Microsoft Dynamics 365 extension | 3 weeks | T-106, T-109 |
| T-1502 | SAP Business One extension | 3 weeks | T-105, T-109 |
| T-1503 | Lexware integration (German market) | 2 weeks | T-109 |
| T-1504 | Sage integration | 2 weeks | T-109 |
| T-1505 | sevDesk integration (German market) | 1 week | T-109 |

### Total effort estimate — everything in one push

This is one concentrated build push with agents working in parallel against the dependency graph. There is no "Year 2 research" backlog and no "weeks 24+" deferred features. Everything that has a task identifier in §6 ships in this push.

**Engineering critical path** (the longest single dependency chain): T-001 → T-010 → T-016 → T-023 → T-040 → T-801 ZATCA archetype → first Wave country crate. Roughly 12–14 weeks of serialized work. With agents in parallel on the non-critical-path tracks (intake, rendering, conformance corpus, hosted layer foundations, Family A non-archetype countries, developer-experience surface, the rest of the Wave country crates), the whole push completes in **roughly the same 12–14 weeks**.

**External delays we cannot remove and that the plan does not pretend to**:
- **ISO 27001 certification** — 6 to 12 months from engagement to audit completion. Starts day one (T-005). Required only to operate our own Peppol Access Point; partner delivery is unaffected. The certification timeline does not gate any other Year-1 work.
- **Country sandbox credentialing** — some regulators require a local tax identification number we have to procure (Brazil, Mexico, India). Phase 2.5 manifests document the requirement; partner delivery ships immediately, native delivery upgrades under the same task once credentials arrive.
- **Regulator rule drift** — ongoing forever; T-006 source-watch and T-074c drift canary handle it.

Engineering pace is bounded only by the critical path. Everything else parallelizes.

---

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| The engine is correct but does not match the official validator-of-record byte-for-byte | Continuously diff against KoSIT, Saxon, phive in continuous integration; publish parity dashboards |
| EN 16931 invoice model fails for CFDI, ZATCA, NF-e (Mexico, Saudi Arabia, Brazil have fundamentally different semantics) | The layered model (CommercialDocument → ProfileView → JurisdictionExtension) handles this; lossiness ledger makes data loss explicit |
| Native bindings have friction for JVM/.NET enterprise security policies | Reverse-proxy sidecar pattern as a fallback; native bindings ship via the same engine API |
| Schematron-to-Rust ahead-of-time compilation hits XPath 2.0 quagmire | Wrap KoSIT and Saxon validators in the JVM worker forever if needed; ahead-of-time compilation is an optimization, not a requirement |
| Pricing tension: free OSS attracts users who do not pay | The hosted layer is the revenue product; the OSS engine is the trust funnel; no per-seat pricing on the OSS engine, ever |
| Browser-side large vision-language model out-of-memory on weak hardware | Server-side is the default for OCR layers 4 and above; browser only does layers 1 through 3 |
| Typst proves unable to satisfy embedded XML plus PDF/A-3 conformance | `RenderBackend` trait allows a secondary renderer; T-053 is the explicit decision gate |
| Incumbent vendors (Avalara, Sovos, Pagero) move down-market with developer APIs | Speed: ship before they retrofit; OSS-first is the moat against managed-only competitors |
| ERP-native distribution (Odoo, Microsoft Dynamics, SAP) eats us | Partner with them: ship as the recommended open-source engine; Microsoft's 2026 ISV connector framework is the lever |
| `invopop/gobl` becomes the dominant standard before us | Interoperate, do not compete; contribute to GOBL specs; differentiate on intake, WebAssembly, transmission, and country breadth |
| National centralization (KSeF, Chorus Pro, MyInvois) reduces Peppol relevance | First-class national gateway integrations alongside Peppol; the country is the unit of compliance, not the network |
| ISO 27001 process is the long pole for becoming our own access point | Starts day one; runs in the background; partner-AP delivery covers Phase 2 onward |
| `phase4` maintainer goes inactive or changes license | Maintain a fork ourselves; native Rust AS4 receiver is the long-term replacement |
| Country sandbox is incomplete, unstable, or gated behind local taxpayer credentials | Cassette-replay simulator for developer experience; partner sandbox where available; official sandbox canary only for countries with `sandbox-proven` capability; no sandbox claim without automated evidence |
| Local qualified certificate, fiscal representative, in-country entity, or PAC / ASP relationship required for live delivery | Country feasibility manifest produced in Phase 2.5 documents these requirements before implementation starts; partner-first delivery for blocked jurisdictions; `invoicekit-signer-agent` enables enterprise customers to keep credentials in their own datacenter |
| Rule drift maintenance exceeds initial implementation effort over the project lifetime | Source-watch automation opens issues on changes; each general-availability country has an explicit owner and a freshness service-level objective; stale countries auto-downgrade in the capability matrix |
| 60 hand-written mock gateways become impossible to maintain as gateways change | Cassette-replay proxy (Section 4.11): record real sandbox traces, scrub personal data, commit cassettes alongside country crates, replay them in tests |

---

## 8. Working mode

This project is built by one principal (the project owner) plus AI agents in a single concentrated effort. Implications:

- No funded phasing or milestone-based releases tied to investor calendar.
- No design-partner pilot programs.
- No team to coordinate.
- No 60-day testing periods or kill-tests; commitments are made upfront based on the research, and adjusted mid-build only when something concrete breaks.
- Parallel work is cheap **after archetypes are locked**. Three archetype country crates ship sequentially first (Section 3.3); after that, agents work in parallel on the rest.
- Speed within the architectural commitments above is the optimization target.

The architectural commitments (Section 2) are the ceiling on autonomy. Agents may make any other choice without confirmation.

### 8.1 Agent work unit — required inputs before any country crate starts

No agent gets "research country X and implement it." Every country-crate agent task must arrive with:

1. **Source manifest** — links to authoritative documents, retrieval dates, version pins.
2. **Acceptance fixtures** — at minimum five valid invoices and five invalid invoices for the country, with expected validation outcomes and (where available) signed cassettes from the country's sandbox.
3. **Exact capability target** — which cells in the capability matrix (Section 3.4) the agent is expected to fill: serialize? validate? sandbox-proven? partner-live? archive? correction?
4. **Files owned** — which crate(s), which traits to implement, which archetype (Peppol / async clearance / cryptographic) the country follows.
5. **Validation oracle** — which validator backend the crate uses (one of `rust-native`, `jvm:*`, `rest:official`, `partner`, `cli:<binary>`, `none`), and the parity threshold.
6. **Done criteria** — explicit acceptance test command, expected pass rate against the fixture set, cassette coverage thresholds.

Country feasibility manifests (Phase 2.5) produce these inputs. Agents that try to start without them produce code that cannot be verified and that wastes everyone's time.

---

## 9. Success criteria

The project is successful when:

- The engine ships on Apache 2.0 with native bindings for Node, Python, Java, .NET, Go, plus a feature-flagged WebAssembly artifact.
- Format support is generally available for all 35+ countries in Family A.
- National report crates have shipped at general-availability quality for every country listed in §3.2 Family B (60+ countries total). Per the matrix in §3.4, every country reaches GA on serialize, validate, render, archive, and correction. Sandbox is `proven` for the countries with reachable sandboxes and `simulated` (with cassette evidence) for the rest. Native-live is `GA` for countries where the native AS4 or national-portal adapter passes conformance, partner-live for the rest.
- Live Peppol delivery works through both the partner access point AND the native Rust AS4 adapter (the native adapter promoted per-route as conformance passes).
- The free public validator at `validate.invoicekit.org` is online with dual-mode operation (browser-only and server-assisted reference).
- The synthetic public conformance corpus is published.
- The hosted managed layer is operational with at least one paying customer.
- ISO 27001 certification is in progress (the audit completes after the build push for the parts that need it; partner delivery is unaffected).
- The `.invoicekit` evidence bundle format is published as an open specification.
- The `invoicekit-signer-agent` is available for on-premise signing scenarios.
- The AI/OCR intake pipeline (Layers 1 through 7) is generally available with bounding-box-cited extraction and cross-examined witness validation.
- The web what-you-see-is-what-you-get template designer (T-057) is generally available.
- Billing-platform bridges (Stripe Invoicing, Lago, Maxio, Chargebee, Recurly) are generally available.
- Enterprise resource planning connectors (Odoo, Microsoft Dynamics, SAP Business One, Lexware, Sage, sevDesk) are generally available in their host marketplaces.
- Reference demo applications (Next.js, Django, Rails, Spring Boot, ASP.NET, Laravel, FastAPI, Go) are all in working order on the same release cadence as the engine.
- One-command on-premise deployment is generally available (docker-compose, Helm, Terraform).
- The validator explain-plan trace is generally available (`invoicekit validate --explain`).
- The schema-evolution migration tooling is generally available.
- Visual regression test infrastructure is wired up for every template.
- Performance regression budget and fuzz continuous integration gates are protecting every pull request.
- Hot-reloadable rule packs are in production.
- Customer-facing audit log is generally available behind `/v1/audit/events`.
- OpenAPI 3.1 specification for the REST surface is published.
- Sandbox-production parity diff job runs nightly.
- The replay-from-bundle command (`invoicekit replay`) is generally available.
- A live REPL (`invoicekit repl`) is generally available.

The principal evaluates whether the project is "shipped" by reading the country capability matrix and confirming every cell is honest. The finish line is **every country in §3.2 at general availability** plus every developer-experience surface in §5 generally available **plus every track in §6 closed out**.

---

## 10. What this document does not contain

- A budget. Solo + agents do not need one beyond the principal's time.
- A team plan. There is no team.
- A funding plan. None needed for the build push.
- A go-to-market plan with paid acquisition. Distribution comes from the open-source engine being good enough to be adopted.

These were in earlier drafts; they were assumptions inherited from organization-shaped planning templates. They are removed.

---

## 11. Open questions

These are decisions the principal still needs to make. None block the start of the build.

1. **Project name** — "InvoiceKit" is a placeholder. Alternatives that came up: Forma, Hectare, Pliant. The principal picks.
2. **Partner Peppol access point** — Storecove, ecosio, or B2BRouter. Decision needs to be made during Phase 2.5 (not Phase 2) because inbound quality, sandbox access, pricing, data residency, and supported countries affect the architecture of every Peppol-overlay country crate downstream. The principal compares quotes alongside the first round of feasibility manifests.
3. **Domain name** — `invoicekit.org` or `invoicekit.dev` for the documentation and free validator. Buy when name is final.
4. **Hosted layer pricing public posting** — when to publish. Currently planned around the time the managed layer reaches general availability.
5. **`.invoicekit` open spec governance** — publish as a community-maintained spec from the start, or stabilize internally first.

---

## 12. What happens next

The principal reviews this plan. When confirmed:

1. Initialize the Cargo workspace and continuous-integration scaffolding (T-001).
2. License files, security policy, contributing guide (T-002).
3. Start the ISO 27001 readiness engagement in the background (T-005). This runs for six to twelve months independently of everything else.
4. T-006 source-watch bot and T-006a capabilities specification.
5. After T-001 + T-002 land: Track 1 (engine primitives) and Track 2 (reference validator) begin in parallel. Track 11's foundation tasks (T-130, T-131, T-132, T-133 — tenant model, key management, webhook signing, software bill of materials) start in parallel since they only depend on T-001/T-002.
6. After T-022 lands: Track 6 reconciliation primitives become unblocked (T-070, T-070a, T-071, T-073, T-074a, T-074, T-074b, T-080, T-083).
7. After T-010 + T-019 land: Track 3 format family A begins (T-040 UBL, T-041 CII, then the projections).
8. As soon as a country feasibility manifest exists (Track 7.5) and the corresponding archetype is locked, that country's crate becomes claimable in Track 8.
9. Open issues in Beads for every task in Section 6. Agents pick up unblocked work via `br ready --json`.

Implementation begins after the principal says go.
