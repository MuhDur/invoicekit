Reading additional input from stdin...
OpenAI Codex v0.133.0
--------
workdir: /home/durakovic/projects/invoices
model: gpt-5.5
provider: openai
approval: never
sandbox: read-only
reasoning effort: xhigh
reasoning summaries: none
session id: 019e645a-0cb0-7040-a850-7f7d35adc4e1
--------
user
Carefully review this implementation plan for InvoiceKit (an open-source global e-invoicing toolkit). This is revision round 2 — a previous codex revision round already integrated 20 structural changes (dual native+WASM artifacts, layered IR with CommercialDocument as root, money/tax/codelists as first-class crates, reference validator worker not wasmtime-java, signed evidence bundles, partner Peppol AP year 1, narrow country GA + maturity matrix, etc.).

The plan now also has global country ambition: ~35 countries via the EN 16931 / Peppol BIS / UBL / CII / Factur-X format family, plus ~25 additional dedicated national report crates for everything from ZATCA to CFDI to NF-e to KSeF to MyInvois.

Focus this revision specifically on what's still broken or risky:
- Architectural decisions that won't survive global ambition
- Hidden assumptions in the country crate structure
- Tasks that are sequenced wrong
- Missing risks
- Wrong-by-degree estimates
- Whether the parallelization-via-agents claim is real or hand-wavy
- Whether the engine ABI / native binding story scales to 60 countries
- Whether the JVM validator worker is the right answer at global scale (or do we need national validator workers?)
- Sandbox / test mode coverage for 60 countries (it's basically impossible to maintain — what's the real strategy?)

Be opinionated. Give git-diff-style change blocks. Don't say "consider X" — say "do X because Y."

Plan:

# InvoiceKit — Implementation Plan v0.3 (Global)

> Consolidated plan, with all review-round-1 revisions integrated and country coverage expanded to global ambition. This replaces v0.1.

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

### 2.6 Reference validators run as an isolated JVM worker service

KoSIT (German XRechnung), phive (Helger's validation engine), Saxon (XSLT 2.0), and the Peppol Schematron rule packs all run inside a dedicated containerized Java service. The Rust engine calls it over JSON-RPC. We do not embed Java in WebAssembly. We do not reimplement these validators. They are the official references; we use them as the ground truth.

Pure-Rust validators are introduced incrementally, ruleset by ruleset, and only after they hit 99.9% rule parity against the JVM reference. If a ruleset does not reach parity in a reasonable effort, it stays as a call to the JVM worker forever. That is fine.

### 2.7 Native AS4 is a research track, not the live delivery path

The AS4 transport protocol used by Peppol is not a 500-line weekend project. Production-grade certification requires Web Services Security, ebMS3, signing, canonicalization, SMP/SML participant lookup, certificate handling, retries, receipts, OpenPeppol conformance testing, and operational practices.

Live Peppol delivery uses a partner access point (Storecove, ecosio, B2BRouter — pick one based on coverage and pricing) plus `phase4` as a reference adapter behind the same `GatewayAdapter` trait. Native Rust AS4 is a long-running research track that ships when conformance tests pass, not when the Rust code compiles.

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

That is 33+ additional countries reached through national-format crates.

Total realistic global coverage: roughly **60+ jurisdictions** once both families are built.

### 3.3 The order of attack

Solo + AI agents in parallel means we can do many things at once, but the engine must come first or every country crate has to be rewritten later. The order is:

**Phase 0 — Engine and foundation (no countries yet, weeks 1–4):**
- Rust workspace, Cargo crates, continuous integration.
- The `money`, `codelists`, `tax-calculation`, and `rulepack` crates.
- The layered invoice model crate (`ir`).
- Canonical serialization (XML C14N 1.1, JSON Canonicalization Scheme).
- The reference validator worker service (containerized JVM with KoSIT, phive, Saxon).
- The signed evidence bundle format.
- The state machine, outbox, and idempotency primitives.
- The `GatewayAdapter` trait and the mock gateway.
- The tenant, key-management, and audit-event schema for the managed layer.
- The C ABI for native bindings, the WebAssembly artifact, and the engine's stable ABI contract.

**Phase 1 — Family A coverage (weeks 5–10):**
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

**Phase 2 — Live Peppol delivery (weeks 11–13):**
- Partner Peppol access point integration (one integration, ~30 destinations).
- `phase4`-backed reference adapter for testing.
- Universal `transmit-mock` for sandbox testing.
- Inbound Peppol receiver service.

After Phase 2, **every Peppol-using country in Family A can both send and receive live**.

**Phase 3 — National clearance crates, parallel agent work (weeks 14–28):**

Each national crate is roughly one to three weeks of work depending on complexity. Agents work in parallel — one crate per agent at a time. Realistic order based on regulatory urgency, market size, and onboarding difficulty:

- **Wave 1** (regulatory urgency, weeks 14–18): Italy SDI, France PA-PDP, Poland KSeF, Spain VeriFactu and FacturaE, Greece myDATA, Belgium Peppol-mandate path, UAE PINT-AE national onboarding.
- **Wave 2** (large markets, weeks 18–24): Saudi Arabia ZATCA, India IRP and e-Waybill, Mexico CFDI, Brazil NF-e and NFS-e, Malaysia MyInvois, Turkey e-Fatura.
- **Wave 3** (rest of Latin America, MENA, APAC, weeks 24–28): Chile, Colombia, Peru, Argentina, Ecuador, Costa Rica, Dominican Republic; Egypt, Israel, Jordan, Bahrain; Indonesia, Philippines, Vietnam, Thailand, South Korea, Japan (Qualified Invoice System), Taiwan, China; Kenya, Nigeria, South Africa; Romania, Hungary, Portugal (national reporting alongside Peppol).

**Phase 4 — Intake pipeline (weeks 6–14, runs in parallel with Phase 1 and 2):**
- Layer 1: Factur-X / ZUGFeRD embedded XML extraction.
- Layer 2: Digital PDF text extraction.
- Layer 3: PDF form field extraction.
- Layer 4: PaddleOCR PP-StructureV3 for layout-aware OCR (server-side).
- Layer 5: SmolDocling-256M ONNX for short-document vision-language understanding (server-side and limited browser-side).
- Layer 6: Qwen2.5-VL-7B inference adapter for cloud fallback.
- Layer 7: Optional cloud LLM (OpenAI or Anthropic vision API) as the last resort.
- Cross-examination: every AI-extracted field is re-validated by the deterministic rules. Mismatches block AI-only output.
- Bounding-box citation taxonomy: every value carries source coordinates.

**Phase 5 — Developer experience surface (weeks 4–28, rolling):**
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

**Phase 6 — Hosted managed layer (weeks 20–32):**
- API gateway, authentication, rate limiting, per-tenant key management.
- Tenant audit dashboard.
- Pluggable archive backends (S3 Object Lock, Azure WORM blob, Google Cloud Storage with retention, local file system, content-addressed IPFS hash).
- RFC 3161 timestamping integration with a trusted timestamping authority.
- eIDAS qualified signature integration with a European qualified trust service provider.
- OpenTelemetry instrumentation with per-gateway dashboards.
- Replay and admin tooling for stuck transmissions and dead-letter queues.

**Phase 7 — Conformance and trust infrastructure (weeks 8–32, rolling):**
- Adversarial fixture generator.
- Synthetic public corpus (CC0 / Apache 2.0): 500+ fixtures across all format families.
- Licensed real corpus (with explicit licensing metadata).
- Private regression corpus (not public).
- Public free validator at `validate.invoicekit.org` — dual mode: local browser-only and server-assisted reference.
- Per-rule explanatory pages on the documentation site (the search-engine optimization play).
- Country newsletter / source-watch automation that monitors official sources and opens issues when rules change.

### 3.4 Honest maturity matrix

For each country, the engine reaches one of these levels per capability:

- **Serialize** — we can generate the country's required invoice file.
- **Validate** — we can check it against the country's rules with our validator and the JVM reference worker.
- **Render** — we can produce the visual PDF with embedded data if the country needs it.
- **Sandbox** — we can run it against the country's test environment.
- **Partner-live** — we can deliver live through a partner gateway or Peppol access point.
- **Native-live** — we can deliver live through our own integration without a partner.
- **Inbound** — we can receive and parse invoices coming from the country.
- **Archive** — we can store with the country's required retention rules.
- **Correction** — we can handle credit notes, debit notes, and cancellations.
- **SLA** — the managed layer carries a service-level agreement for that country.

After the build push, the realistic levels are:

| Country group | Serialize | Validate | Render | Sandbox | Partner-live | Inbound | Archive | Correction | SLA |
|---|---|---|---|---|---|---|---|---|---|
| Peppol-native EU/EEA/UK/AU/NZ/SG/JP (Family A only) | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Germany | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| France | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Italy | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Spain | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Poland | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Saudi Arabia, India, UAE | yes | yes | yes | yes | yes (via PAC partner) | yes | yes | yes | yes |
| Mexico, Brazil | yes | yes | yes | yes | yes (via in-country PAC partner) | yes | yes | yes | yes |
| Malaysia, Greece, Romania, Hungary, Portugal (national report layer) | yes | yes | yes | yes | yes | yes | yes | yes | yes |
| Other Latin America, MENA, APAC, Africa (full list above) | yes | yes | yes | yes | yes (via partner per country) | yes | yes | yes | yes |

The honest caveat is delivery in Latin America (Brazil, Mexico, India) often requires a local tax identification number to test against, and a real-world signing certificate purchased from a local trust provider. We will need to either purchase those for our own test setup or partner with a local PAC (provedor autorizado de certificación) for delivery — the agent doing each crate makes that call.

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
│   ├── lsp/                      Invoice language server
│   ├── cli/                      `invoicekit` binary
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
│   ├── validator-worker-jvm/     KoSIT / phive / Saxon JVM service over JSON-RPC
│   └── managed-api-server/       Hosted compliance API (Axum + Postgres + S3)
├── conformance-corpus/
│   ├── synthetic/                CC0 / Apache 2.0 generated fixtures
│   ├── licensed-real/            Explicitly licensed and redacted real invoices
│   ├── private-regression/       Non-public customer fixtures
│   └── generators/               Adversarial fixture generators
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
  extensions: JurisdictionExtensions;             // Typed per-country data
  meta: DocumentMeta;
}

interface JurisdictionExtensions {
  de_xrechnung?: { leitweg_id: string; /* ... */ };
  de_zugferd?: { profile: 'MINIMUM' | 'BASIC_WL' | 'BASIC' | 'EN16931' | 'EXTENDED' | 'XRECHNUNG'; /* ... */ };
  fr_chorus_pro?: { service_code?: string; engagement_number?: string; /* ... */ };
  fr_ppf?: { pdp_id: string; /* ... */ };
  it_sdi?: { tipo_documento: string; codice_destinatario: string; /* ... */ };
  pl_ksef?: { ksef_id?: string; receipt_hash?: string; /* ... */ };
  sa_zatca?: { previous_invoice_hash: string; tlv_qr: Uint8Array; cryptographic_stamp: Uint8Array; /* ... */ };
  in_irp?: { irn?: string; ack_dt?: DateTime; signed_qr?: string; /* ... */ };
  mx_cfdi?: { uuid?: string; complemento?: object; /* ... */ };
  br_nfe?: { chave_acesso?: string; protocolo?: string; /* ... */ };
  my_myinvois?: { irn?: string; validation_status?: string; /* ... */ };
  gr_mydata?: { mark?: string; uid?: string; /* ... */ };
  // ... one per country
}
```

The model ships as a public JSON Schema plus generated TypeScript, Python, Java, and .NET types. Continuous integration tests every binding for byte-equivalence with the Rust source of truth.

### 4.4 Canonicalization

For any operation that produces a signature, hash, or audit record:

- **XML canonicalization**: XML C14N 1.1 plus an invoice-specific overlay that normalizes namespace prefixes, attribute order, and ignorable whitespace.
- **JSON canonicalization**: RFC 8785 (JSON Canonicalization Scheme) for the invoice JSON form.
- **PDF byte-stable subset**: pinned Typst version; pinned font set (subsetted Inter, DejaVu, Noto); pinned harfbuzz; fixed XMP creation date placeholder; deterministic object ordering. Two renders with the same input produce identical bytes.

Continuous integration runs `canonicalize_xml`, `canonicalize_json`, and `render_pdf` under multiple operating system and architecture combinations, and asserts byte equality.

### 4.5 Validation

Three layers, descending in confidence:

- **Schema** — structural validation against XSD and JSON Schema.
- **Business rules** — EN 16931 core (hand-written Rust validators for ~50 rules); country-specific rule packs (KoSIT, Peppol Schematron, country variants) executed through the JVM validator worker.
- **Cross-examined witness** — for AI-extracted invoices, every value goes through the deterministic rules. If the AI says the total is €1234.56 but the lines sum to €1230.45, the AI output is blocked.

Pure-Rust validators are promoted from "wrapped JVM call" to "native Rust" one rule pack at a time, gated on 99.9% rule parity against the JVM reference. We never gate shipping on this promotion.

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

Verification: `invoicekit verify bundle.invoicekit` reproduces validation, asserts signatures, asserts timestamps, asserts content-address consistency. Court-admissible when paired with qualified electronic signatures from a qualified trust service provider.

### 4.8 Peppol AS4 — practical path

| Phase | Sender | Receiver | Operator status |
|---|---|---|---|
| Phase 2 (weeks 11–13) | Partner access point | Partner access point | Use partner; no certification needed |
| Phase 6 onward (weeks 20+) | Native Rust sender with `phase4` as conformance oracle | `phase4` wrapped in the validator-worker container | Apply for OpenPeppol membership and ISO 27001 in parallel |
| Future research | Native Rust sender certified by OpenPeppol | Native Rust receiver | Become a tier-2 access point in our own right |

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
- **Layer C (later)** — a web-based what-you-see-is-what-you-get template designer that produces the TypeScript template language. Built once the engine is stable.

If Typst proves unable to satisfy embedded XML plus PDF/A-3 conformance for every Factur-X profile we need, the `RenderBackend` trait allows a secondary renderer (for example, a custom Rust PDF builder layered on `lopdf` or `printpdf`).

### 4.11 Test mode — Stripe-style sandbox

Every transmission path has a mock counterpart:

```typescript
const client = new InvoiceKit({ mode: 'test' });
await client.transmit(invoice, { route: 'peppol' });
// Returns realistic receipts (success / canned errors / latency / partial ACK)
```

Mock gateways simulate documented failure modes of real gateways: SDI's 504 timeouts, KSeF's peak-hour latency, Peppol's malformed acknowledgments, ZATCA's certificate chain rejections, IRP's "IRN-already-issued" errors, CFDI's PAC rejection codes. Developers can opt into specific failure scenarios for chaos testing.

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

const invoice = Invoice.builder(enriched)
  .line({ description: 'Consulting', quantity: 5, unitPrice: 200, vatCategory: 'S', vatRate: 19 })
  .build();

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
invoicekit verify bundle.invoicekit                Court-grade re-verification
invoicekit fuzz                                    Adversarial generator
invoicekit benchmark                               Performance dashboard
invoicekit capabilities --from=DE --to=FR --date=2027-01-01
                                                   What rules apply for that direction on that date
invoicekit explain BR-CO-10                        Plain-language rule explainer with formula
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

### Track 0 — Foundation (sequential, no dependencies on country work)

| Task | Description | Effort |
|---|---|---|
| T-001 | Cargo workspace, continuous integration scaffolding, code-of-conduct, contributing guide, security policy | 1 week |
| T-002 | License (Apache 2.0), signed releases, software bill of materials, dependency scanning | 1 week |
| T-005 | ISO 27001 readiness engagement starts (background, 6–12 months) | 0 days direct work |
| T-006 | Compliance source-watch bot (monitors official sources, opens issues on rule changes) | 1 week |
| T-006a | `invoicekit capabilities` data model from source-watch manifests, confidence and last-verified date | 1 week |

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
| T-021 | Property-based round-trip tests (IR ↔ JSON ↔ XML) | 1 week | T-019, T-020 |
| T-022 | Deterministic invoice fingerprint (BLAKE3) | 2 days | T-019, T-020 |
| T-023 | Stable engine ABI contract + cross-language golden fixtures | 2 weeks | T-010, T-016 |
| T-024 | C ABI surface (`invoicekit-ffi`) | 1 week | T-023 |
| T-025 | WebAssembly artifact (`invoicekit-wasm`) | 1 week | T-023 |

### Track 2 — Reference validator and validation

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-030 | Validator worker container (KoSIT, phive, Saxon, JSON-RPC API) | 2 weeks | T-001 |
| T-031 | EN 16931 hand-written Rust validator (~50 core rules) | 3 weeks | T-010, T-017 |
| T-032 | Validation result schema (rule ID, severity, BT term, JSON Pointer or XPath location, suggested fix, citation) | 1 week | T-031 |
| T-033 | Browser/edge validator capability matrix (which profiles are pure-WASM vs require the worker) | 3 days | T-030, T-031 |
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
| T-053 | Renderer fallback decision gate (prove Typst meets Factur-X requirements or keep secondary renderer behind `RenderBackend` trait) | 1 week | T-052 |
| T-054 | Factur-X XML embedding (PDF/A-3 attachment) | 1 week | T-052 |
| T-055 | Deterministic byte-stable rendering subset | 1 week | T-054 |
| T-056 | Accessible HTML5 rendering pipeline (WCAG-conformant) | 1 week | T-051 |

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
| T-070 | Gateway adapter trait and normalized gateway error taxonomy | 1 week | T-032 |
| T-071 | Outbox SQL schema, idempotency model, retry policy, dead-letter states | 2 weeks | T-022, T-070 |
| T-072 | Transmission worker with backoff, rate limits, circuit breakers, structured gateway logs | 2 weeks | T-071 |
| T-073 | State machine implementation (per-country sub-states) | 2 weeks | T-070 |
| T-074 | Mock gateway (`transmit-mock`) — first `GatewayAdapter` implementation | 1 week | T-070 |
| T-075 | Reconciliation API and outbox SQL migrations (Postgres, MySQL, SQLite) | 1 week | T-071 |
| T-076 | Webhook dispatcher with replay protection and idempotency | 1 week | T-073 |
| T-077 | Server-Sent Events stream for ACK delivery | 1 week | T-073 |
| T-080 | Signed evidence bundle format (`.invoicekit`, packed form `.ikb`) | 2 weeks | T-022, T-031, T-073 |
| T-081 | Pluggable archive backend (S3 Object Lock, Azure WORM, Google Cloud Storage retention, local file system, IPFS hash) | 2 weeks | T-080 |
| T-082 | RFC 3161 timestamping integration with a trusted timestamping authority | 1 week | T-080 |
| T-083 | eIDAS qualified signature integration with a qualified trust service provider | 2 weeks | T-080 |
| T-084 | `invoicekit verify` library and CLI | 1 week | T-080, T-082, T-083 |

### Track 7 — Peppol live delivery

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-090 | Peppol participant lookup (SMP/SML client) | 1 week | T-042 |
| T-091 | Partner Peppol access point adapter (selection of partner: Storecove / ecosio / B2BRouter based on pricing and coverage) | 2 weeks | T-072, T-090 |
| T-092 | `phase4` reference adapter (Java in the validator-worker container for now) | 1 week | T-091 |
| T-093 | Peppol inbound receiver service | 2 weeks | T-091 |
| T-094 | Native Rust AS4 sender (research track; gated on OpenPeppol conformance suite) | 6+ weeks (background) | T-091 |

### Track 8 — National report crates

Each country crate is roughly one to three weeks of work depending on complexity. Agents work in parallel — one crate per agent at a time. The dependency on all national crates is the same set of foundation tasks (T-010, T-017, T-070, T-073, T-074). Beyond that, each is independent.

**Wave 1 — Regulatory urgency** (weeks 14–18 if parallel):

| Crate | Description | Effort |
|---|---|---|
| `report-it-sdi` | Italy SDI clearance and receipts | 3 weeks |
| `report-fr-ctc` | France PA / PDP e-invoicing and e-reporting | 3 weeks |
| `report-pl-ksef` | Poland KSeF FA(3) clearance | 3 weeks |
| `report-es-verifactu` | Spain VeriFactu and FacturaE | 2 weeks |
| `report-gr-mydata` | Greece myDATA | 2 weeks |
| `report-sa-zatca` | Saudi Arabia ZATCA Phase 2 with cryptographic stamping | 3 weeks |
| `report-be-pep` | Belgium Peppol mandate path (mostly Family A reuse) | 1 week |

**Wave 2 — Large markets** (weeks 18–24):

| Crate | Description | Effort |
|---|---|---|
| `report-in-gst` | India IRP, GST, e-Waybill | 3 weeks |
| `report-mx-cfdi` | Mexico CFDI 4.0 via PAC partner | 3 weeks |
| `report-br-nfe` | Brazil NF-e and NFS-e | 4 weeks |
| `report-my-myinvois` | Malaysia MyInvois | 2 weeks |
| `report-tr-efatura` | Turkey e-Fatura | 2 weeks |
| `report-ro-efactura` | Romania RO e-Factura | 2 weeks |
| `report-hu-nav` | Hungary NAV Online Invoicing | 2 weeks |
| `report-jp-qis` | Japan Qualified Invoice System | 2 weeks |

**Wave 3 — Latin America, MENA, APAC, Africa** (weeks 24–28):

| Crate | Description | Effort |
|---|---|---|
| `report-cl-dte` | Chile SII DTE | 2 weeks |
| `report-co-dian` | Colombia DIAN | 2 weeks |
| `report-pe-sunat` | Peru SUNAT | 2 weeks |
| `report-ar-afip` | Argentina AFIP | 2 weeks |
| `report-ec-sri` | Ecuador SRI | 2 weeks |
| `report-cr-hacienda` | Costa Rica Hacienda | 2 weeks |
| `report-do-dgii` | Dominican Republic DGII | 2 weeks |
| `report-eg-eta` | Egypt ETA | 2 weeks |
| `report-il-ita` | Israel Tax Authority | 2 weeks |
| `report-id-djp` | Indonesia DJP Online | 2 weeks |
| `report-ph-bir` | Philippines BIR EIS | 2 weeks |
| `report-vn-gdt` | Vietnam GDT | 2 weeks |
| `report-th-rd` | Thailand RD | 2 weeks |
| `report-kr-nts` | South Korea NTS | 2 weeks |
| `report-cn-fapiao` | China Golden Tax / Fapiao | 3 weeks |
| `report-tw-mof` | Taiwan MOF | 2 weeks |
| `report-ke-etims` | Kenya eTIMS | 2 weeks |
| `report-ng-firs` | Nigeria FIRS | 2 weeks |
| `report-za-sars` | South Africa SARS | 2 weeks |

These weeks are sequential if done by a single human; with agents in parallel, the wave can collapse to its longest crate.

### Track 9 — Developer experience surface (rolling, weeks 4–28)

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-100 | `invoicekit` command-line binary | 2 weeks rolling | T-031 |
| T-101 | `invoicekit doctor` | 3 days | T-100 |
| T-102 | `invoicekit init` interactive | 3 days | T-100 |
| T-103 | TypeScript SDK (`@invoicekit/core`, `@invoicekit/render`, `@invoicekit/managed`) | 2 weeks | T-023, T-024 |
| T-104 | Python SDK (`pyo3` + `maturin`) | 2 weeks | T-023, T-024 |
| T-105 | Java SDK (JNI / Foreign Function and Memory API over C ABI, with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-106 | .NET SDK (P/Invoke over C ABI, with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-107 | Go SDK (cgo with REST sidecar fallback) | 2 weeks | T-023, T-024 |
| T-108 | Browser bundle (`wasm-bindgen` for Cloudflare Workers, Deno, Bun, browser) | 1 week | T-025 |
| T-109 | REST shim (Axum) | 2 weeks | T-023, T-031 |
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
| T-121 | Adversarial generator (Rust) | 2 weeks | T-010 |
| T-122 | Synthetic public corpus version 0.5 (500+ adversarial invoices) | 2 weeks | T-121 |
| T-123 | Differential test harness | 2 weeks | T-035 |
| T-124 | Public benchmark dashboard | 1 week | T-123 |

### Track 11 — Hosted managed layer

| Task | Description | Effort | Depends on |
|---|---|---|---|
| T-130 | Tenant model, scoped API keys, OIDC, RBAC, audit-event schema | 2 weeks | T-001 |
| T-131 | Envelope encryption with key-management-service per tenant, key rotation, data residency tags | 2 weeks | T-130 |
| T-132 | Webhook signing, replay protection, event-delivery idempotency | 1 week | T-130 |
| T-133 | Software bill of materials, dependency scanning, signed releases, security advisory process | 1 week | T-002 |
| T-134 | API gateway, authentication, rate limiting | 2 weeks | T-130 |
| T-135 | Customer dashboard (audit log, usage, errors) | 3 weeks | T-130 |
| T-136 | OpenTelemetry tracing, metrics, log redaction, per-gateway dashboards | 2 weeks | T-072 |
| T-137 | Replay and admin tooling for stuck transmissions and dead-letter queues | 1 week | T-136 |
| T-138 | Status page and incident tooling | 1 week | T-136 |
| T-139 | Support ticket integration | 1 week | T-135 |
| T-140 | Stripe integration for our own customer invoicing | 1 week | T-130 |

### Total effort estimate (with agents in parallel)

If executed by a single human in sequence: roughly 18 months of focused work.

With AI agents in parallel on independent tracks: realistic compression to **8 to 10 months end-to-end**, dominated by the longest dependency chain (engine → format family A → first national report crate → first live partner integration).

Once tracks 1–7 are complete (foundation + Family A + Peppol partner delivery), the project has shipped real value with ~35 countries reached. The remaining waves can ship as they complete.

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

---

## 8. Working mode

This project is built by one principal (the project owner) plus AI agents in a single concentrated effort. Implications:

- No funded phasing or milestone-based releases tied to investor calendar.
- No design-partner pilot programs.
- No team to coordinate.
- No 60-day testing periods or kill-tests; commitments are made upfront based on the research, and adjusted mid-build only when something concrete breaks.
- Parallel work is cheap. One agent per track. Multiple agents per track when one is stuck.
- Speed within the architectural commitments above is the optimization target.

The architectural commitments (Section 2) are the ceiling on autonomy. Agents may make any other choice without confirmation.

---

## 9. Success criteria

The project is successful when:

- The engine ships on Apache 2.0 with native bindings for Node, Python, Java, .NET, Go, plus a WebAssembly artifact.
- Format support is generally available for all 35+ countries in Family A.
- National reporting crates have shipped for at least 15 of the countries in Family B.
- Live Peppol delivery works through a partner access point.
- The free public validator at `validate.invoicekit.org` is online with dual-mode operation.
- The conformance corpus (synthetic) is published.
- The hosted managed layer is operational with at least one paid customer.
- ISO 27001 certification is in progress (the audit can complete after the build push).
- The `.invoicekit` evidence bundle format is published as an open spec.

The principal evaluates whether the project is "shipped" by reading the country matrix and confirming each cell is honest.

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
2. **Partner Peppol access point** — Storecove, ecosio, or B2BRouter. Decision based on per-envelope pricing, country coverage, and inbound-receiver quality. Compare quotes when Phase 2 starts.
3. **Domain name** — `invoicekit.org` or `invoicekit.dev` for the documentation and free validator. Buy when name is final.
4. **Hosted layer pricing public posting** — when to publish. Currently planned around the time the managed layer reaches general availability.
5. **`.invoicekit` open spec governance** — publish as a community-maintained spec from the start, or stabilize internally first.

---

## 12. What happens next

The principal reviews this plan. When confirmed:

1. Initialize the Cargo workspace and continuous-integration scaffolding (T-001).
2. License files, security policy, contributing guide (T-002).
3. Start the ISO 27001 readiness engagement in the background (T-005).
4. Begin Track 1 (engine primitives) and Track 2 (validator worker) in parallel.
5. Open issues in Beads for every task in Section 6. Agents pick up unblocked work via `br ready --json`.

Implementation begins after the principal says go.
2026-05-26T12:54:46.927267Z ERROR codex_memories_write::phase2: Phase 2 no changes
codex
I’m going to ground this in the project contract and do a light current-facts check before giving the review. Since this is compliance-heavy, I’ll separate structural plan changes from factual/date drift where I can verify it.
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc 'pwd && rg -n "InvoiceKit|invoices|e-invoicing|e-invoice|Peppol|KSeF|ZATCA|CFDI|NF-e" /home/durakovic/.codex/memories/MEMORY.md' in /home/durakovic/projects/invoices
 exited 1 in 0ms:
/home/durakovic/projects/invoices

hook: PostToolUse
hook: PostToolUse Completed
exec
/bin/zsh -lc 'cm context "review InvoiceKit global e-invoicing implementation plan v0.3 for architecture risks, sequencing, estimates, validator strategy, sandbox strategy" --json' in /home/durakovic/projects/invoices
hook: PostToolUse
hook: PostToolUse Completed
 succeeded in 311ms:
{
  "success": true,
  "command": "context",
  "timestamp": "2026-05-26T12:55:03.753Z",
  "data": {
    "task": "review InvoiceKit global e-invoicing implementation plan v0.3 for architecture risks, sequencing, estimates, validator strategy, sandbox strategy",
    "relevantBullets": [
      {
        "id": "b-mmu6rhad-ki1qcf",
        "scope": "global",
        "category": "methodology",
        "content": "For 100% accuracy data matching: use parallel agent review (each agent reviews batch with narrator chain, Jaccard, n-gram, hadith number checks) then independent algorithmic verification (Jaccard >= threshold, no attractors). Agent + algorithm catches both reasoning errors and mechanical false positives.",
        "source": "learned",
        "type": "rule",
        "isNegative": false,
        "kind": "workflow_rule",
        "state": "draft",
        "maturity": "candidate",
        "helpfulCount": 0,
        "harmfulCount": 0,
        "feedbackEvents": [],
        "confidenceDecayHalfLifeDays": 90,
        "createdAt": "2026-03-17T05:42:46.597Z",
        "updatedAt": "2026-03-17T05:42:46.597Z",
        "pinned": false,
        "deprecated": false,
        "sourceSessions": [
          "manual-cli"
        ],
        "sourceAgents": [
          "unknown"
        ],
        "tags": [],
        "relevanceScore": 3,
        "effectiveScore": 0,
        "finalScore": 0.30000000000000004,
        "lastHelpful": "Never",
        "reasoning": "From unknown session on 3/17/2026"
      },
      {
        "id": "b-mp4398jc-g7jbcg",
        "scope": "global",
        "category": "orchestration",
        "content": "When spawning opencode pane via ntm add --oc, ALWAYS pin the model explicitly (e.g., --oc=1:github-copilot/gpt-5.2-codex). Bare --oc=1 has defaulted to claude-opus-4.5 which opencode CLI rejects with 'model not supported'. Fallback model when gpt-5.2-codex crashes 2+ times: --oc=1:zai-coding-plan/glm-5.1.",
        "source": "learned",
        "type": "rule",
        "isNegative": false,
        "kind": "workflow_rule",
        "state": "draft",
        "maturity": "candidate",
        "helpfulCount": 0,
        "harmfulCount": 0,
        "feedbackEvents": [],
        "confidenceDecayHalfLifeDays": 90,
        "createdAt": "2026-05-13T13:21:43.032Z",
        "updatedAt": "2026-05-13T13:21:43.032Z",
        "pinned": false,
        "deprecated": false,
        "sourceSessions": [
          "manual-cli"
        ],
        "sourceAgents": [
          "unknown"
        ],
        "tags": [],
        "relevanceScore": 1,
        "effectiveScore": 0,
        "finalScore": 0.1,
        "lastHelpful": "Never",
        "reasoning": "From unknown session on 5/13/2026"
      }
    ],
    "antiPatterns": [],
    "historySnippets": [],
    "deprecatedWarnings": [],
    "suggestedCassQueries": [
      "cass search 'strategy review invoicekit' --days 30",
      "cass search 'strategy error' --days 60",
      "cass search 'strategy' --days 90",
      "cass search 'strategy review' --days 60",
      "cass search 'invoicekit' --days 90"
    ]
  },
  "metadata": {
    "executionMs": 114,
    "version": "0.2.3"
  }
}

web search: 
web search: site:impots.gouv.fr facturation electronique calendrier septembre 2026 2027 France B2B ...
web search: 
web search: site:finance.belgium.be electronic invoicing b2b Belgium 2026 official ...
web search: 
web search: site:einvoice.hasil.gov.my MyInvois timeline official Malaysia e-Invoice 2026 ...
web search: 
web search: site:sdk.myinvois.hasil.gov.my Malaysia e-Invoice MyInvois SDK timeline ...
web search: 
web search: site:docs.peppol.eu Peppol BIS Billing 3.0 country CIUS official validation rules ...
web search: 
web search: site:finance.belgium.be/en b2b e-invoicing 2026 Belgium mandatory structured invoices Peppol ...
codex
The first pass already shows one major theme: the plan still treats country coverage as mostly code throughput, but the hard parts are regulator source truth, local credentials, partner contracts, sandboxes, and ongoing rule drift. I’m checking a few official sources now so the review does not accidentally preserve stale country assumptions.
web search: 
web search: https://docs.peppol.eu/poacc/billing/3.0/national-rules/
web search: 
web search: https://efactuur.belgium.be/nl/article/gestructureerde-elektronische-facturen-tussen-ondernemingen-verplicht-vanaf-2026
codex
I’m also using the `planning-workflow` skill here because the task is a plan review, but I’m keeping it as a light checklist rather than turning this into process advice.
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc "sed -n '1,220p' /home/durakovic/.codex/skills/planning-workflow/SKILL.md" in /home/durakovic/projects/invoices
 succeeded in 0ms:
---
name: planning-workflow
description: >-
  Comprehensive markdown planning methodology for software projects. Use when
  starting a new project, creating implementation plans, or refining architecture
  before coding.
---

<!-- TOC: Philosophy | THE EXACT PROMPT | Process Overview | References -->

# Planning Workflow — The Foundation of Agentic Development

> **Core Philosophy:** "Planning tokens are a lot fewer and cheaper than implementation tokens."
>
> The models are far smarter when reasoning about a detailed plan that fits within their context window. This is the key insight behind spending 80%+ of time on planning.

---

## Outcome — When This Skill Has Delivered

You have a usable plan when **all** of the following hold:

- The plan is **self-contained**: a fresh agent who has never seen the project can read it and start implementing without asking the human for clarification.
- It is **dependency-aware**: every non-trivial task names what blocks it and what it unblocks, so the work decomposes cleanly into a beads graph.
- It is **justified**: every architectural choice and every non-obvious feature has at least one sentence on *why*, not just *what* — future agents need the rationale to make consistent local decisions.
- It has survived **at least 4 review rounds** by a strong reasoning model (GPT Pro Extended Reasoning is the proven choice) and reached steady-state — i.e., the most recent round produces marginal rather than structural revisions.
- It has been **converted to beads** with the dependency graph intact, so implementation agents can pick up ready work via `br ready --json` without re-reading the plan.

You have NOT delivered if any of these is true: the plan is < ~1,500 lines for a non-trivial project (under-specified); the plan is > ~10,000 lines with no decomposition (impossible to act on); the human is still being asked "what should this do?" mid-implementation (planning was abandoned, not completed); beads exist but have no dependency edges (the plan's structure was lost in conversion).

## When NOT to Use This Skill

Reach for something else if:

- **The change is small and local** (one bug fix, one file, < ~200 LOC) → plan in chat or as a one-line TaskCreate; the planning overhead exceeds the implementation cost.
- **You are doing pure research** (investigating an unknown codebase, prototyping an idea) → use `codebase-archaeology` or `idea-wizard` first; planning is for execution, not discovery.
- **The architecture is dictated** (you are porting an existing system, following a spec, or implementing a well-defined RFC) → use `porting-to-rust` or `testing-conformance-harnesses`; the plan is the spec itself.
- **You're under a hard deadline that doesn't permit 80%-on-planning** (live incident, hotfix, security patch) → ship the fix, then retrofit the plan if the area needs further work.

## Grounding — Sources of Truth for Plan Decisions

When a planning model proposes architecture, the proposal is a hypothesis. Ground every load-bearing claim in a verifiable source before letting it survive a review round:

- **Library/framework choices:** read the actual current docs (latest stable version, not the model's training-time snapshot). If the model says "use X for Y," verify X still exists, is maintained, and supports Y in the version you'd install.
- **Existing-codebase claims:** for any plan that touches an existing project, the model's understanding of the project structure is suspect. Grep, `git log`, or use `codebase-archaeology` to confirm structural claims before they shape the plan.
- **Performance / scaling claims:** never accept a number ("handles 10k req/s," "loads in <100ms") without a citation or a planned benchmark. Bare numbers in plans are guesses dressed as facts.
- **Cost claims:** verify against the provider's pricing page at planning time; pricing models change. Pin the plan to the specific tier you priced against.
- **Cross-references to other skills/tools:** if the plan says "use `<tool>` for X," confirm the tool's current contract supports X. Linkrot in plans is a quiet failure mode.

A plan that survived review without grounding is a plan that will surprise you in implementation. Cheap verification at planning time beats expensive rework after the code is half-written.

## Validation Loop (between review rounds)

After each review round, before sending the plan back for another pass, run all four:

1. **Self-containment check** — pick the most obscure task in the plan, paste it alone into a fresh chat, and ask "is this implementable as written?" If no, expand.
2. **Dependency-graph check** — can you draw the DAG of tasks? Are there cycles? Are there orphans (tasks with no consumers)? Either is a planning bug.
3. **Justification check** — sample 5 random architectural decisions. Each must have a paragraph of *why*. If not, ask the planning model to add it.
4. **Steady-state check** — diff this round's plan against the previous round's. If the diff is large structural changes, you need another round. If it's typo-level polish, you're done.

If any of these fails, the plan is not ready for beads conversion — do another review round.

---

## Why Planning Matters

- **Measure twice, cut once** — becomes "Check your plan N times, implement once"
- A very big, complex markdown plan is still shorter than a few substantive code files
- Front-loading human input in planning enables removing yourself from implementation
- The code will be written ridiculously quickly when you start enough agents with a solid plan

---

## THE EXACT PROMPT — Plan Review (GPT Pro Extended Reasoning)

```
Carefully review this entire plan for me and come up with your best revisions in terms of better architecture, new features, changed features, etc. to make it better, more robust/reliable, more performant, more compelling/useful, etc. For each proposed change, give me your detailed analysis and rationale/justification for why it would make the project better along with the git-diff style change versus the original plan shown below:

<PASTE YOUR EXISTING COMPLETE PLAN HERE>
```

---

## THE EXACT PROMPT — Integrate Revisions (Claude Code)

After GPT Pro finishes (may take 20-30 minutes), paste output into Claude Code:

```
OK, now integrate these revisions to the markdown plan in-place; use ultrathink and be meticulous. At the end, you can tell me which changes you wholeheartedly agree with, which you somewhat agree with, and which you disagree with:

```[Pasted text from GPT Pro]```
```

---

## Process Overview

```
1. INITIAL PLAN (GPT Pro / Opus 4.7 in web app)
   └─► Explain goals, intent, workflows, tech stack

2. ITERATIVE REFINEMENT (GPT Pro Extended Reasoning)
   └─► 4-5 rounds of revision until steady-state

3. MULTI-MODEL BLENDING (Optional but recommended)
   └─► Gemini 3.1 Pro Deep Think, Grok4 Heavy, Opus 4.7
   └─► GPT Pro as final arbiter

4. CONVERT TO BEADS (Claude Code + Opus 4.7)
   └─► Self-contained tasks with dependency structure

5. POLISH BEADS (6+ rounds until steady-state)
   └─► Cross-model review, never oversimplify
```

---

## What Makes a Great Plan

| Good Plan | Great Plan |
|-----------|------------|
| Describes what to build | Explains WHY you're building it |
| Lists features | Details user workflows and interactions |
| Mentions tech stack | Justifies tech choices with tradeoffs |
| Has tasks | Has tasks with dependencies and rationale |
| ~500 lines | ~3,500+ lines after refinement |

### Essential Elements

1. **Self-contained** — Never need to refer back to external docs
2. **Granular** — Break complex features into specific subtasks
3. **Dependency-aware** — What blocks what?
4. **Justified** — Include reasoning, not just instructions
5. **User-focused** — How does each piece serve the end user?

---

## Common Mistakes

1. **Starting implementation too early** — 3 hours of planning saves 30 hours of rework
2. **Single-round review** — You continue to get improvements even at round 6+
3. **Not using GPT Pro** — Extended Reasoning is uniquely good for this
4. **Skeleton-first coding** — One big comprehensive plan beats incremental coding
5. **Losing context** — Convert plans to beads so agents don't need the original

---

## References

| Topic | Reference |
|-------|-----------|
| All exact prompts | [PROMPTS.md](references/PROMPTS.md) |
| Real-world examples | [EXAMPLES.md](references/EXAMPLES.md) |
| FAQ | [FAQ.md](references/FAQ.md) |

hook: PostToolUse
hook: PostToolUse Completed
codex
Verdict: v0.3 is much better structurally, but it still overclaims by treating 60-country coverage as mostly parallelizable implementation. It is not. The remaining hard gates are regulator source truth, local credentials, partner contracts, official sandboxes, validator provenance, and ongoing rule drift. As of May 26, 2026, official sources also support tightening several facts: Peppol has country-qualified rule sets, France phases receive/issue obligations, KSeF has staged 2026 obligations, UAE has ASP and implementation timelines, Belgium is mandatory since January 1, 2026, Malaysia is phased through January 1, 2026, India exposes IRP sandbox APIs, and ZATCA Phase 2 is wave-based. Sources: OpenPeppol docs, France DGFiP, Poland KSeF, UAE MoF, Belgium e-factuur, Malaysia LHDN, India GSTN, ZATCA.         

```diff
@@ Section 1 / Plain-English summary
- We are the only option that gives a developer one install, one engine, and every country.
+ We are one engine with a capability registry. “One install” means common IR,
+ rule-pack selection, validation reports, evidence bundles, and SDK shape.
+ Country support is opt-in by profile/gateway package and is advertised only
+ at the maturity level proven by fixtures, source manifests, and live/sandbox runs.
```

```diff
@@ Section 3.4 Honest maturity matrix
- Other Latin America, MENA, APAC, Africa ... yes | yes | yes | yes | yes ...
+ Delete the grouped all-yes matrix. Replace it with one row per country and one
+ required evidence pointer per cell:
+ source_manifest, fixture_count, reference_validator, official_sandbox,
+ partner_contract, live_receipt_sample, archive_rule_source, correction_tests.
+ A cell cannot be “yes” without a dated artifact. Use labels:
+ none | researched | serialize-alpha | validate-beta | sandbox-proven |
+ partner-live-proven | managed-sla.
```

```diff
@@ Section 3.2 Family A
- Once the engine speaks this family, it can generate and check invoices for 35+ countries.
+ Once the engine speaks UBL/CII/EN16931/Peppol, it covers the shared syntax and
+ baseline business rules. It does not automatically cover each country.
+ Add a country-qualified rule-pack layer in Phase 1 because Peppol BIS explicitly
+ supports seller-country-triggered national rules maintained by Peppol Authorities.
```

```diff
@@ Section 4.1 Crate layout / national crates
- crates/report-<country>/
+ jurisdictions/<iso>/
+   sources/              # official URLs, versions, retrieval timestamps, licenses
+   profile/              # serialize/parse/projection code
+   validation/           # local rules + reference/backend declaration
+   gateway/              # live/partner protocol, optional
+   sandbox/              # simulator + official/partner sandbox contract
+   archive/              # retention/correction rules
+   fixtures/             # positive/negative/cassette corpus
+
+ Do this because “report-<country>” hides different domains: format, clearance,
+ transport, credentials, tax reporting, archive, and correction.
```

```diff
@@ Section 4.3 Data model / ABI
- JurisdictionExtensions { de_xrechnung?: ..., it_sdi?: ..., ... one per country }
+ Do not put 60 countries into one stable ABI or one mega-schema.
+ The stable engine ABI exposes opaque handles, canonical JSON bytes, profile IDs,
+ capability queries, validation reports, evidence bundle streams, and async job IDs.
+ Per-country typed SDKs are generated from jurisdiction packages and can version
+ independently without breaking C ABI, WASM ABI, or older bindings.
```

```diff
@@ Section 2.6 Validator worker
- Reference validators run as an isolated JVM worker service.
+ Reference validation runs through a validator-orchestrator. The JVM worker is
+ backend #1 for KoSIT/phive/Saxon/Peppol Schematron. Other backends are native
+ Rust, official REST/API validator, partner validator, CLI/container validator,
+ or “no official reference exists”. Each rule pack declares its validator backend
+ and parity target. Do this because global e-invoicing is not JVM-shaped.
```

```diff
@@ Section 4.11 Test mode
- Every transmission path has a mock counterpart ... Returns realistic receipts.
+ Every transmission path has a deterministic simulator. Only countries with
+ automated official or partner sandbox runs may claim sandbox-proven.
+ Simulators are contract tests, not proof of regulatory acceptance.
+ Add nightly canaries for GA countries and recorded cassettes for known gateway
+ behavior. Mark all other countries sandbox=simulated.
```

```diff
@@ Section 3.3 / Build sequence
- Phase 3 national clearance crates, parallel agent work, weeks 14–28
+ Insert Phase 0.5 before country implementation:
+ country source manifests, partner/credential feasibility, official sandbox access,
+ validator-backend classification, certificate/local-tax-ID requirements,
+ and no-go flags. A country crate cannot start until this manifest exists.
+ Agents can parallelize serializers and fixtures; they cannot parallelize missing
+ credentials, regulator access, partner contracts, or legal interpretation.
```

```diff
@@ Section 6 Total effort estimate
- With AI agents ... realistic compression to 8 to 10 months end-to-end.
+ First real GA: 6–9 months for engine + Family A baseline + Peppol partner live
+ + 1–3 national live integrations.
+ Broad global beta: 12–18 months.
+ Honest 60-country managed SLA: 24–36+ months, dominated by external access,
+ credentialing, maintenance, and regulatory churn.
```

```diff
@@ Section 7 Risks
+ Add: Official sandboxes are incomplete, unstable, unavailable, or gated behind
+ local taxpayer credentials. Mitigation: simulator for developer UX, partner
+ sandbox where available, official sandbox canary only for GA, and no sandbox
+ maturity claim without automated evidence.
+
+ Add: Local certificate, local fiscal representative, local entity, or PAC/ASP
+ relationship may be required. Mitigation: country feasibility manifest before
+ implementation and partner-first delivery for blocked jurisdictions.
+
+ Add: Rule drift maintenance exceeds initial implementation effort. Mitigation:
+ source-watch opens issues, but GA requires an owner and a freshness SLO per
+ country; stale countries auto-downgrade.
```

```diff
@@ Section 8 Working mode
- Parallel work is cheap. One agent per track.
+ Parallel work is cheap only after contracts are frozen.
+ Required agent work unit: source manifest, acceptance fixtures, exact capability
+ target, files owned, validation oracle, and done criteria. No agent starts a
+ country from “research this and implement it”; that produces unverifiable code.
```

```diff
@@ Section 9 Success criteria
- National reporting crates have shipped for at least 15 countries in Family B.
+ National reporting crates have shipped for at least 5 countries with sandbox-proven
+ or partner-live-proven status, plus at least 15 researched/source-manifested
+ country packages with serialize-alpha or validate-beta where evidence exists.
+ This is the honest finish line for one concentrated build push.
```

```diff
@@ Section 11 Open questions
- Partner Peppol access point ... Compare quotes when Phase 2 starts.
+ Partner Peppol access point is not an open question at Phase 2.
+ Decide during Phase 0.5 because inbound quality, sandbox access, pricing,
+ data residency, supported countries, and SLA terms change architecture and tests.
```

Also remove “court-grade” and “court-admissible” from `invoicekit verify`. Say “cryptographically verifiable evidence bundle.” Courts and regulators decide admissibility; the plan should not make that promise without jurisdiction-specific legal review.
tokens used
142,990
Verdict: v0.3 is much better structurally, but it still overclaims by treating 60-country coverage as mostly parallelizable implementation. It is not. The remaining hard gates are regulator source truth, local credentials, partner contracts, official sandboxes, validator provenance, and ongoing rule drift. As of May 26, 2026, official sources also support tightening several facts: Peppol has country-qualified rule sets, France phases receive/issue obligations, KSeF has staged 2026 obligations, UAE has ASP and implementation timelines, Belgium is mandatory since January 1, 2026, Malaysia is phased through January 1, 2026, India exposes IRP sandbox APIs, and ZATCA Phase 2 is wave-based. Sources: OpenPeppol docs, France DGFiP, Poland KSeF, UAE MoF, Belgium e-factuur, Malaysia LHDN, India GSTN, ZATCA.         

```diff
@@ Section 1 / Plain-English summary
- We are the only option that gives a developer one install, one engine, and every country.
+ We are one engine with a capability registry. “One install” means common IR,
+ rule-pack selection, validation reports, evidence bundles, and SDK shape.
+ Country support is opt-in by profile/gateway package and is advertised only
+ at the maturity level proven by fixtures, source manifests, and live/sandbox runs.
```

```diff
@@ Section 3.4 Honest maturity matrix
- Other Latin America, MENA, APAC, Africa ... yes | yes | yes | yes | yes ...
+ Delete the grouped all-yes matrix. Replace it with one row per country and one
+ required evidence pointer per cell:
+ source_manifest, fixture_count, reference_validator, official_sandbox,
+ partner_contract, live_receipt_sample, archive_rule_source, correction_tests.
+ A cell cannot be “yes” without a dated artifact. Use labels:
+ none | researched | serialize-alpha | validate-beta | sandbox-proven |
+ partner-live-proven | managed-sla.
```

```diff
@@ Section 3.2 Family A
- Once the engine speaks this family, it can generate and check invoices for 35+ countries.
+ Once the engine speaks UBL/CII/EN16931/Peppol, it covers the shared syntax and
+ baseline business rules. It does not automatically cover each country.
+ Add a country-qualified rule-pack layer in Phase 1 because Peppol BIS explicitly
+ supports seller-country-triggered national rules maintained by Peppol Authorities.
```

```diff
@@ Section 4.1 Crate layout / national crates
- crates/report-<country>/
+ jurisdictions/<iso>/
+   sources/              # official URLs, versions, retrieval timestamps, licenses
+   profile/              # serialize/parse/projection code
+   validation/           # local rules + reference/backend declaration
+   gateway/              # live/partner protocol, optional
+   sandbox/              # simulator + official/partner sandbox contract
+   archive/              # retention/correction rules
+   fixtures/             # positive/negative/cassette corpus
+
+ Do this because “report-<country>” hides different domains: format, clearance,
+ transport, credentials, tax reporting, archive, and correction.
```

```diff
@@ Section 4.3 Data model / ABI
- JurisdictionExtensions { de_xrechnung?: ..., it_sdi?: ..., ... one per country }
+ Do not put 60 countries into one stable ABI or one mega-schema.
+ The stable engine ABI exposes opaque handles, canonical JSON bytes, profile IDs,
+ capability queries, validation reports, evidence bundle streams, and async job IDs.
+ Per-country typed SDKs are generated from jurisdiction packages and can version
+ independently without breaking C ABI, WASM ABI, or older bindings.
```

```diff
@@ Section 2.6 Validator worker
- Reference validators run as an isolated JVM worker service.
+ Reference validation runs through a validator-orchestrator. The JVM worker is
+ backend #1 for KoSIT/phive/Saxon/Peppol Schematron. Other backends are native
+ Rust, official REST/API validator, partner validator, CLI/container validator,
+ or “no official reference exists”. Each rule pack declares its validator backend
+ and parity target. Do this because global e-invoicing is not JVM-shaped.
```

```diff
@@ Section 4.11 Test mode
- Every transmission path has a mock counterpart ... Returns realistic receipts.
+ Every transmission path has a deterministic simulator. Only countries with
+ automated official or partner sandbox runs may claim sandbox-proven.
+ Simulators are contract tests, not proof of regulatory acceptance.
+ Add nightly canaries for GA countries and recorded cassettes for known gateway
+ behavior. Mark all other countries sandbox=simulated.
```

```diff
@@ Section 3.3 / Build sequence
- Phase 3 national clearance crates, parallel agent work, weeks 14–28
+ Insert Phase 0.5 before country implementation:
+ country source manifests, partner/credential feasibility, official sandbox access,
+ validator-backend classification, certificate/local-tax-ID requirements,
+ and no-go flags. A country crate cannot start until this manifest exists.
+ Agents can parallelize serializers and fixtures; they cannot parallelize missing
+ credentials, regulator access, partner contracts, or legal interpretation.
```

```diff
@@ Section 6 Total effort estimate
- With AI agents ... realistic compression to 8 to 10 months end-to-end.
+ First real GA: 6–9 months for engine + Family A baseline + Peppol partner live
+ + 1–3 national live integrations.
+ Broad global beta: 12–18 months.
+ Honest 60-country managed SLA: 24–36+ months, dominated by external access,
+ credentialing, maintenance, and regulatory churn.
```

```diff
@@ Section 7 Risks
+ Add: Official sandboxes are incomplete, unstable, unavailable, or gated behind
+ local taxpayer credentials. Mitigation: simulator for developer UX, partner
+ sandbox where available, official sandbox canary only for GA, and no sandbox
+ maturity claim without automated evidence.
+
+ Add: Local certificate, local fiscal representative, local entity, or PAC/ASP
+ relationship may be required. Mitigation: country feasibility manifest before
+ implementation and partner-first delivery for blocked jurisdictions.
+
+ Add: Rule drift maintenance exceeds initial implementation effort. Mitigation:
+ source-watch opens issues, but GA requires an owner and a freshness SLO per
+ country; stale countries auto-downgrade.
```

```diff
@@ Section 8 Working mode
- Parallel work is cheap. One agent per track.
+ Parallel work is cheap only after contracts are frozen.
+ Required agent work unit: source manifest, acceptance fixtures, exact capability
+ target, files owned, validation oracle, and done criteria. No agent starts a
+ country from “research this and implement it”; that produces unverifiable code.
```

```diff
@@ Section 9 Success criteria
- National reporting crates have shipped for at least 15 countries in Family B.
+ National reporting crates have shipped for at least 5 countries with sandbox-proven
+ or partner-live-proven status, plus at least 15 researched/source-manifested
+ country packages with serialize-alpha or validate-beta where evidence exists.
+ This is the honest finish line for one concentrated build push.
```

```diff
@@ Section 11 Open questions
- Partner Peppol access point ... Compare quotes when Phase 2 starts.
+ Partner Peppol access point is not an open question at Phase 2.
+ Decide during Phase 0.5 because inbound quality, sandbox access, pricing,
+ data residency, supported countries, and SLA terms change architecture and tests.
```

Also remove “court-grade” and “court-admissible” from `invoicekit verify`. Say “cryptographically verifiable evidence bundle.” Courts and regulators decide admissibility; the plan should not make that promise without jurisdiction-specific legal review.
