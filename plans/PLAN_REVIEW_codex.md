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
session id: 019e62d7-d9f4-7123-b991-085db3b944f2
--------
user
Carefully review this entire plan for me and come up with your best revisions in terms of better architecture, new features, changed features, etc. to make it better, more robust/reliable, more performant, more compelling/useful, etc. For each proposed change, give me your detailed analysis and rationale/justification for why it would make the project better along with the git-diff style change versus the original plan shown below.

Focus particularly on:
- Architectural decisions that could fail at scale
- Missing tasks in the build sequence
- Sequencing/dependency mistakes
- Risks not yet mitigated
- Country/format coverage gaps
- DX surface area improvements

Be opinionated and specific. Suggest concrete code shapes, library choices, file paths. Don't just say "consider X" — say "do X because Y."

Plan follows:

# InvoiceKit — Implementation Plan v0.1 (DRAFT)

> Initial plan, written by the principal architect agent after ~490-idea ideation, multi-model triangulation, and two-model Brenner-style adversarial falsification. This document is the input to the next 4-5 review rounds.

**Working name**: InvoiceKit (placeholder; final name TBD)
**License**: Apache 2.0
**Date**: May 2026
**Status**: DRAFT — about to enter revision rounds

---

## 0. Executive summary (TL;DR)

InvoiceKit is a developer-first toolkit for the entire B2B invoicing lifecycle (intake → normalize → validate → render → transmit → reconcile → archive), built around three architectural bets:

1. **A WASM-native Rust core** with native bindings for every major runtime (TypeScript via wasm-bindgen, Python via pyo3, Go via wazero, Java via wasmtime-java, .NET via Wasmtime.NET, plus a CLI binary and a thin REST shim).
2. **An EN 16931–rooted semantic IR** with first-class country/profile extensions and lossless round-trip to UBL 2.1, CII, Factur-X/ZUGFeRD, XRechnung, Peppol BIS 3.0, FatturaPA, KSeF FA(3), and others. **We interop with GOBL JSON schemas rather than reinvent**.
3. **Trust as the moat** — every validation, rendering, signing, transmission, and reconciliation event produces a **signed, content-addressed, auditable `.invoicekit` evidence bundle** that anyone can re-verify offline. This is the "CI for invoice compliance" runaway-success bet.

The **OSS core is Apache 2.0 forever**. Revenue comes from a **managed compliance API** (Peppol AP, national gateway transmission, certificate management, vault-grade archive, cloud LLM fallback) that satisfies enterprise risk-transfer demands. Pricing has two tiers: free OSS / metered usage for developers (€0.05/envelope above 100/mo free), and a **Managed Compliance API tier at €499–€1,999/mo with SLA, liability cap, and country coverage commitments**.

Year 1 is about shipping the trust foundation (IR, canonicalizer, AOT validator wrapping Java fallback, Typst PDF/A-3, evidence bundles, Stripe-style sandbox) and a Peppol AP managed service (wrapping `phax/phase4` as Year-1 fallback while building native-Rust AS4 in parallel for Year 2).

Three falsification experiments in the first 60 days will kill the project's biggest assumptions cheaply.

---

## 1. Why now

Demand is regulated and concrete. As of May 26, 2026:

- **Germany** B2B e-invoice receipt mandate live since Jan 2025; send mandate Jan 2027 (>€800k) and Jan 2028 (all).
- **France** broad receipt obligations kick in **September 1, 2026**.
- **Poland** KSeF 2.0 already live; mandatory thresholds rolling 2026.
- **Belgium** B2B Peppol mandate live since Jan 1, 2026.
- **Italy** SDI mature; **Spain** Verifactu Jan/Jul 2027; **Saudi Arabia** ZATCA waves 23–24 in Mar/Jun 2026; **India** IRP live; **UAE** Jul/Oct 2026 pilot then Jan 2027 mandate; **Malaysia** MyInvois Phase 4 Jan 2026 plus Dec 2027 cliff; **Greece** myDATA Mar/Oct 2026; **Kenya** "No Invoice No Deduction" Jan 2026.
- **EU ViDA** cross-border digital reporting from July 1, 2030.

Every ERP, billing SaaS, and B2B platform now needs e-invoicing — and the available tooling forces them into a Java stack, a JVM service, or a SaaS bundle they can't embed.

Confirmed developer pain (from real GitHub issues, HN, dev.to threads):

- Schematron validation requires Java/XSLT 2.0 — breaks Node/Deno/Bun/edge.
- Puppeteer / headless Chrome unworkable on Cloudflare Workers.
- Existing libs silently regress (mustangproject 2.13→2.15 NPE, factur-x-python broken on Debian 12).
- Peppol AP membership ~€2,000/yr just to start.
- Stripe Billing maxes at 1k events/sec — no Peppol support.

The wedge:

> **"The only invoicing toolkit that generates and validates EN 16931 (XRechnung + Factur-X + Peppol BIS) on Bun, Deno, Cloudflare Workers, JVM, .NET, Python, Go, and the browser — and produces audit-grade evidence bundles for every operation."**

---

## 2. Goals and non-goals

### Goals (year 1)

- Ship an Apache-2.0 OSS core (Rust+WASM with bindings) that any developer can `npm i / pip install / cargo add / dotnet add / go get` and use within 5 minutes.
- Cover the German/French/Polish/Belgian/Peppol stack with full conformance-tested support for inbound + outbound flows.
- Run a Managed Compliance API on top, with Peppol AP, certificate management, vault archive, and SLA — first commercial offering.
- Establish ourselves as the conformance reference — public test corpus + differential test harness + canonical bug-tracker for invoice format edge cases.
- Achieve 10 paying customers at €5k+ ACV, plus 1,000+ GitHub stars and 100+ active OSS contributors.

### Non-goals (explicitly)

- We do NOT build a tax engine (TaxJar/Avalara/Vertex own this).
- We do NOT build an ERP (we feed them).
- We do NOT build CRM, banking, or payment processing (we expose payment instructions).
- We do NOT build a closed-source SaaS portal as the primary product.
- We do NOT build "AI-first invoicing" branding — AI is a quiet supporting capability.
- We do NOT compete with invoiceninja/crater for SMB end-user invoicing apps; we are infrastructure for the developers who build those.

---

## 3. Strategic positioning

### ICPs (in priority order)

1. **Embedded developer at ERP / billing SaaS vendor** (Odoo, Microsoft Dynamics partners, sevDesk, Pennylane, Lexware, Pleo). ~500 EU targets. €15–150k ACV. Wedge: drop-in compliance, no JVM, multi-runtime.
2. **In-house engineer at midmarket B2B SaaS forced into mandates** (5–50M ARR). 50k+ targets. €3–20k ACV. Wedge: Stripe Billing-style ergonomics + Peppol/XRechnung correctness.
3. **OSS maintainer of adjacent project** (Invoice Ninja, ERPNext, Odoo, Dolibarr). Indirect funnel via downstream adoption.

### Explicit anti-ICP

- AP/AR ops at corporates with 1,000+ FTE. Despite their willingness to pay (€100–300k ACV), they push us toward Coupa/Tipalti shape (inbound OCR + 3-way match + 6-12mo sales cycles + closed-source on-prem). Defer to Year 4 via partnership only.

### Differentiation vs. competitive set

| Competitor class | What they do | Where we differ |
|------------------|--------------|-----------------|
| Pagero, Comarch, Sovos, Avalara, Storecove | SaaS portal + Peppol AP. €15k–250k/yr; per-jurisdiction surcharges; opaque pricing | Self-serve OSS lib; flat €0.05/envelope; no per-country surcharge |
| Mustangproject, ph-en16931, ZUGFeRD-csharp | OSS libs per language, JVM-anchored | One Rust core compiled to WASM; identical behavior across runtimes; modern DX |
| Invopop GOBL | Apache-2.0 Go library, similar IR concept | Cross-runtime delivery (WASM); intake/OCR pipeline; Peppol AS4 client; evidence bundles. **We interop with GOBL schemas, not against them** |
| Rossum, Mindee, Klippa | OCR/extraction SaaS | Local-first OCR via PaddleOCR + Qwen2.5-VL fallback; bounding-box-cited extraction; no per-page cloud cost mandatory |
| Stripe Invoicing, Lago, Maxio, Chargebee | Billing/subscription engines | We're the e-invoice compliance layer that plugs *into* them, not against them |

---

## 4. Architecture

### 4.1 Module layout (the engine = a Rust workspace)

```
invoicekit/
├── crates/
│   ├── core/                 # No-std, no I/O. The IR types, validation traits, serializers.
│   ├── ir/                   # EN 16931 IR with country/profile extensions
│   ├── canonical/            # Deterministic byte-stable XML/JSON serialization
│   ├── validate/             # Rule registry; wraps Java validators initially
│   ├── validate-aot/         # Hand-compiled Rust validators for hot rule packs (EN 16931 core)
│   ├── validate-schematron/  # Schematron→Rust AOT compiler (experiment-gated; falls back to JVM)
│   ├── render-pdf/           # Typst-based PDF/A-3 (deterministic) + Factur-X embedding
│   ├── intake-pdf/           # Digital PDF parsing; Factur-X XML detection & extraction
│   ├── intake-ocr/           # PaddleOCR layer 1-3; Qwen2.5-VL hook for layer 4 (server-side default)
│   ├── intake-ai-shim/       # Bounding-box-cited extraction interface (cross-examined witness)
│   ├── transmit-peppol/      # AS4 sender (Rust-native) + receiver (wraps phase4 via wasmtime year 1)
│   ├── transmit-it-sdi/      # Italy SDI gateway
│   ├── transmit-pl-ksef/     # Poland KSeF FA(3) gateway
│   ├── transmit-fr-ppf/      # France PPF/PDP gateway
│   ├── transmit-in-irp/      # India IRP gateway
│   ├── transmit-sa-zatca/    # Saudi ZATCA Phase 2 with cryptographic stamping
│   ├── transmit-my-myinvois/ # Malaysia MyInvois
│   ├── transmit-be-pep/      # Belgium Peppol-mandate path
│   ├── transmit-gr-mydata/   # Greece myDATA
│   ├── transmit-ke-etims/    # Kenya eTIMS
│   ├── transmit-mock/        # Sandbox: simulates SDI/KSeF/IRP/ZATCA/PPF including bad responses
│   ├── reconcile/            # Fingerprint, state machine, idempotency, outbox helpers
│   ├── evidence/             # .invoicekit archive format; RFC 3161 timestamping; signing
│   ├── archive/              # Pluggable storage (S3 Object Lock / Azure WORM / local FS / IPFS hash)
│   ├── verify/               # Bundle verification CLI/library
│   ├── lsp/                  # Invoice LSP — UBL, CII, GOBL, rulepack diagnostics
│   ├── cli/                  # `invoicekit` binary
│   ├── wasm-glue/            # wasm-bindgen wrappers
│   └── conformance-corpus/   # 500+ adversarial test invoices; public CC-BY-SA
├── bindings/
│   ├── npm/                  # TypeScript SDK
│   ├── python/               # pyo3 wheel
│   ├── go/                   # cgo or wazero wrapper
│   ├── jvm/                  # Java JAR with wasmtime-java
│   ├── dotnet/               # NuGet w/ Wasmtime.NET
│   └── rest-shim/            # Axum-based thin HTTP gateway over the engine (for non-binding clients)
├── docs/
│   ├── pages/                # Nextra docs site
│   ├── rules/                # Per-rule explanatory pages (SEO play)
│   └── countries/            # Per-country guides
└── docker/
    └── managed-compliance/   # Dockerized turnkey deployment
```

### 4.2 Data flow

```
                            ┌─────────────────────────┐
                            │  Source documents        │
                            │  - PDF (digital/scan)    │
                            │  - XML (UBL/CII)         │
                            │  - JSON (GOBL/our IR)    │
                            │  - CSV (ERP export)      │
                            │  - DB rows (via adapter) │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Intake (layered)        │
                            │  L1 Factur-X XML extract │
                            │  L2 digital PDF parse    │
                            │  L3 PaddleOCR            │
                            │  L4 Qwen2.5-VL (server)  │
                            │  L5 cloud LLM fallback   │
                            │  Each emits IR + spans   │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  IR normalize & validate │
                            │  - EN 16931 core         │
                            │  - Country CIUS overlays │
                            │  - Cross-examined witness│
                            │    (AI ↔ deterministic)  │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Outbound serialization  │
                            │  - UBL 2.1               │
                            │  - CII (CrossIndustry)   │
                            │  - Factur-X / ZUGFeRD    │
                            │  - XRechnung 3.x         │
                            │  - Peppol BIS 3.0        │
                            │  - FatturaPA / SDI       │
                            │  - KSeF FA(3)            │
                            │  - CFDI 4.0 (Mexico)     │
                            │  - ZATCA Phase 2         │
                            │  - GOBL JSON             │
                            │  + canonicalization      │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Render PDF/A-3 (Typst)  │
                            │  - Embed XML attachment  │
                            │  - Pin font + harfbuzz   │
                            │  - Deterministic bytes   │
                            │  - veraPDF-equivalent    │
                            │    conformance verify    │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Sign + timestamp         │
                            │  - eIDAS QTSP partner    │
                            │  - RFC 3161 timestamp    │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Transmit (state machine)│
                            │  - Peppol AS4            │
                            │  - National gateway      │
                            │  - Email fallback        │
                            │  Idempotent; reconciled  │
                            └────────────┬─────────────┘
                                         ▼
                            ┌─────────────────────────┐
                            │  Evidence bundle out     │
                            │  .invoicekit archive:    │
                            │  - source bytes          │
                            │  - canonical IR JSON     │
                            │  - generated XML         │
                            │  - PDF/A-3               │
                            │  - validation trace      │
                            │  - rulepack hashes       │
                            │  - signatures + ACKs     │
                            │  - timestamps            │
                            │  - replay script         │
                            └─────────────────────────┘
```

### 4.3 The IR (concrete shape)

The IR is the **most architecturally consequential decision** in the project. Both Brenner critiques (codex + gemini) warn it will collapse under national/profile reality. Mitigations:

- **Strict EN 16931 semantic core** — every field maps to an EN 16931 business term (BT-*) or business group (BG-*).
- **Profile extensions as typed, namespaced sidecar maps** — each country/profile contributes a typed extension struct (e.g. `extensions.de.xrechnung_leitweg_id`, `extensions.sa.zatca_phase2_qr`). Extensions are first-class, **not** an `additional_data: HashMap<String, Value>` escape hatch.
- **Lossiness ledger** — every cross-format conversion produces a structured report enumerating fields that couldn't round-trip cleanly. "Lossless" is a tested claim, not vibes.
- **GOBL interop adapter** — the IR has a documented bidirectional mapping to GOBL's JSON schema. If a customer is GOBL-native, we read/write GOBL transparently.
- **Versioning** — IR is semver'd; major bumps ship codemods + automatic migration.

#### IR top-level (illustrative TypeScript shape — generated from Rust source of truth)

```typescript
interface Invoice {
  schema_version: '1.0';
  id: InvoiceId;                          // Deterministic fingerprint (blake3 of canonical content)
  document_type: 'invoice' | 'credit_note' | 'debit_note' | 'pro_forma' | 'self_billed';
  issue_date: DateOnly;                   // ISO 8601, no time
  tax_point_date?: DateOnly;
  due_date?: DateOnly;
  invoice_number: InvoiceNumber;          // Per-tenant monotonic sequence; legal numbering ledger ensures uniqueness
  currency: Iso4217Code;
  supplier: Party;
  customer: Party;
  payee?: Party;
  payment_terms?: PaymentTerms;
  payment_instructions: PaymentInstruction[];  // SEPA, BIC/IBAN, Swiss QR, EPC QR, ZATCA QR, etc.
  lines: InvoiceLine[];
  tax_summary: TaxCategorySummary[];      // VAT category subtotals
  monetary_total: MonetaryTotal;          // Allowances, charges, line extension, payable amount
  attachments?: Attachment[];             // Content-addressed; hash + MIME + role
  references?: DocumentReference[];       // PO, contract, despatch advice, prior invoice (corrections)
  notes?: LocalizedString[];
  extensions: ProfileExtensions;          // Country/profile-specific typed extensions
  meta: InvoiceMeta;                      // Internal-only: source bytes hash, timestamps, signing chain
}

interface ProfileExtensions {
  de_xrechnung?: { leitweg_id: string; ... };
  de_zugferd?: { profile: 'MINIMUM' | 'BASIC_WL' | 'BASIC' | 'EN16931' | 'EXTENDED' | 'XRECHNUNG'; ... };
  fr_chorus_pro?: { service_code?: string; engagement_number?: string; ... };
  fr_ppf?: { pdp_id: string; ... };
  it_sdi?: { tipo_documento: string; codice_destinatario: string; ... };
  pl_ksef?: { ksef_id?: string; receipt_hash?: string; ... };
  sa_zatca?: { previous_invoice_hash: string; tlv_qr: Uint8Array; ... };
  in_irp?: { irn?: string; ack_dt?: DateTime; signed_qr?: string; ... };
  // ... per-country
}
```

The IR ships with a public **JSON Schema** + **Rust types** + generated bindings; both shapes are CI-tested for byte-equivalence.

### 4.4 Canonicalization

For signed/hashed/audit operations:

- **XML canonicalization**: XML C14N 1.1 + invoice-specific overlay (namespace prefix normalization, attribute ordering, ignorable whitespace handling).
- **JSON canonicalization**: RFC 8785 (JSON Canonicalization Scheme — JCS) for the IR JSON form.
- **PDF byte-stable subset**: pinned Typst version, pinned font set (subsetted Inter + DejaVu + Noto), pinned harfbuzz, fixed XMP creation date placeholder, deterministic object ordering. Two renders with the same input produce identical bytes.
- **Test**: golden-file CI runs all of (`canonicalize_xml`, `canonicalize_json`, `render_pdf`) under multiple OS/arch combinations and asserts byte equality.

### 4.5 Validation

Three layers, descending in confidence:

| Layer | What it validates | Tech |
|-------|-------------------|------|
| 1. Schema | XSD/JSON-Schema structural | quick-xml + xsd11-rs / serde_json |
| 2. Business rules (EN 16931 + CIUS) | BT-/BG- level rules | Hand-written Rust validators for EN 16931 core + AOT-compiled Schematron where feasible + JVM-wrapped reference validators where AOT isn't ready |
| 3. Cross-examined witness | AI extraction validity | Deterministic re-validation of AI-extracted IR against schema + rules |

**Critical risk and mitigation** (from Brenner): Schematron→Rust AOT is XPath 2.0 quagmire. We commit to **incremental AOT migration**:

- Year 1 Q1: hand-write EN 16931 core validator in pure Rust (well-bounded, ~50 rules).
- Year 1 Q1-Q2: wrap KoSIT XRechnung validator + Peppol Schematron via `wasmtime-java` (yes — Java in WASM). Diff against hand-written core to find missing rules.
- Year 1 Q2-Q3: AOT-compile Schematron rules one ruleset at a time, gated behind **99.9% rule parity against the JVM reference**. If a ruleset doesn't reach parity in 3 weeks of effort, stays JVM-wrapped indefinitely.
- This is Brenner experiment E1 — falsifiable in 3 weeks.

### 4.6 Reconciliation engine — the paid moat

Both Brenner critiques converge here: managed reconciliation is the value beyond OSS validation.

Primitives:

- **Deterministic invoice fingerprint**: `blake3(supplier_VAT || customer_VAT || issue_date || invoice_number || total_amount || currency)`. Used as dedup key in our managed service.
- **Idempotency-key envelopes**: every `transmit()` carries an `idempotency_key` (client-provided or auto-derived); replay is no-op.
- **State machine**: `draft → validated → signed → reserved → sent → delivered → acknowledged → rejected → archived`. Per-country sub-states (KSeF reserved/committed; SDI accepted/rejected; ZATCA cleared) layer in.
- **Reconciliation API**: `POST /reconcile` accepts a customer's list of `{internal_id, fingerprint}`; returns `{delivered: [], failed: [], pending: [], unknown: []}` with timestamps and gateway evidence.
- **Outbox migration**: we ship SQL migrations (Postgres, MySQL, SQLite) for an `invoicekit_outbox` table. One-line install for at-least-once delivery semantics from any monolith.
- **State-change webhooks + SSE**: pluggable delivery; SSE works behind NAT/firewalls (a Gemini-flagged win).
- **Sealed entity invariant**: post-`delivered`, the IR is immutable; corrections create credit notes + replacements, never mutations.

### 4.7 Evidence bundles

`.invoicekit` is a tar-gz archive (open spec) containing:

```
manifest.json                # Bundle metadata, schema version, content-address index
ir.json                      # Canonical IR
canonical/
  invoice.xml                # Generated UBL/CII/XRechnung etc.
  invoice.json               # JCS-canonical JSON
render/
  invoice.pdf                # PDF/A-3 with embedded XML
  invoice.html               # WCAG-accessible HTML5 render
intake/
  source.{pdf,xml,json,...}  # Original source bytes (verbatim)
  extraction-trace.json      # AI/OCR layer traces with bbox citations
validation/
  trace.json                 # Each rule evaluated, result, location
  rulepack-manifest.json     # Hashes of each rule pack used
signatures/
  *.sig                      # PKCS#7 / XAdES / qualified seal
  rfc3161-tsr.bin            # RFC 3161 timestamp response
transmission/
  receipts/*.json            # Gateway ACKs/NACKs with timestamps
replay.sh                    # Single shell script that re-runs validation + verification
```

Verification: `invoicekit verify bundle.invoicekit` reproduces validation, asserts signatures, asserts timestamps, asserts content-address consistency. Court-admissible artifact when paired with QTSP-signed receipts.

**Brenner critique addressed**: the `.invoicekit` format has no legal standing by itself; the legal artifacts are the embedded XML + PDF/A-3 + qualified signatures, and the bundle is just a *convenience wrapper* with verification metadata. This is documented prominently.

### 4.8 Peppol AS4 — hybrid implementation path

| Year | Sender | Receiver | Operator status |
|------|--------|----------|-----------------|
| Y1 (2026–2027) | Native Rust AS4 sender (node42 proves this is ~500 LOC; we'll do it cleaner) | Wrap `phax/phase4` via `wasmtime-java` for receiver-side WS-Security validation | Managed service; partner with established AP for delivery of envelopes we don't yet certify to send |
| Y2 (2027–2028) | Native Rust sender certified by OpenPeppol | Native Rust receiver with phase4 still as conformance oracle | Apply for OpenPeppol membership + ISO 27001 (start day 1; ~6-12 month lead) |
| Y3 (2028+) | Full native stack | Full native stack | Become a tier-2 Peppol AP in our own right |

**Brenner critique addressed**: the managed service exists from Y1; we don't need to be a certified AP to ship value, we partner. ISO 27001 starts on day 1 (Brenner identified this as the non-technical long pole).

### 4.9 Intake / OCR pipeline

Default-server-side (avoid Gemini's Brenner critique about 7B-in-browser OOMing AP clerks' laptops). Browser is for **light** intake only:

| Layer | Server-side default | Browser-side (light variant) |
|-------|---------------------|------------------------------|
| L1 Factur-X XML detection | quick-xml + Rust | quick-xml WASM |
| L2 Digital PDF text | pdf-extract or lopdf | pdf.js |
| L3 PDF form fields | lopdf | pdf.js |
| L4 Layout-aware OCR | PaddleOCR PP-StructureV3 (Python/cpp via Rust bindings) | Tesseract WASM |
| L5 Small VLM | SmolDocling-256M ONNX | SmolDocling-256M via Transformers.js WebGPU (only for short docs) |
| L6 Large VLM (cloud fallback) | Qwen2.5-VL-7B (our cloud) | not available browser-side |
| L7 LLM cloud (extreme fallback) | OpenAI/Anthropic vision API | not available browser-side |

Every extraction emits IR + per-field `{value, source: {bbox?, ocr_span_id?, pdf_object_id?, model_id}, confidence}`. Deterministic cross-validation: VAT sums close, totals reconcile, buyer/supplier VAT IDs validate against VIES — mismatches block AI-only output.

### 4.10 Rendering (Typst)

Typst is the underlying renderer because it produces deterministic PDF/A-3 byte-stable output from a programmatic spec. We solve Gemini's Brenner WYSIWYG-PM critique by **layering**:

- Layer A: Typst is the renderer.
- Layer B: We ship a TypeScript template DSL that compiles to Typst (no Typst-language exposure required for end users).
- Layer C (Y2): A web-based WYSIWYG template designer that produces the TS template DSL.

ERP PMs see drag-and-drop in Y2; the deterministic guarantee in Y1 is implementation-level. Both can be true.

### 4.11 Test mode / Stripe-style sandbox

Every transmission path has a mock counterpart:

```typescript
const client = new InvoiceKit({ mode: 'test' });  // Routes to invoicekit-mock-gateway
await client.transmit(invoice, { route: 'peppol' });
// Returns realistic receipts (success / canned errors / latency / partial ACK / etc.)
```

Mock gateways simulate documented failure modes of real gateways: SDI's 504 timeouts, KSeF's peak-hour latency, Peppol's malformed ACKs, ZATCA's certificate chain rejections, IRP's IRN-already-issued errors. Customers can opt into specific failure scenarios for chaos testing.

---

## 5. Public surface — DX

### 5.1 First-touch DX

```
$ npx invoicekit init
✓ Detected: Node.js + TypeScript + ESM
? Country (auto-detected from package.json: DE)
? Default supplier VAT ID: DE123456789  # VIES lookup confirms
? Sandbox or live? [sandbox]

✓ Generated:
  - invoicekit.config.ts
  - .env.example  (INVOICEKIT_API_KEY=test_*)
  - examples/first-invoice.ts

Try it: pnpm exec invoicekit send examples/first-invoice.ts
```

### 5.2 Library API (TypeScript, illustrative)

```typescript
import { Invoice, validate, render, transmit, verify } from '@invoicekit/core';

const invoice = Invoice.builder({
  supplier: { vat: 'DE123456789' },  // Rest auto-resolved via VIES
  customer: { vat: 'FR987654321' },
  currency: 'EUR',
})
  .line({ description: 'Consulting', quantity: 5, unitPrice: 200, vatCategory: 'S', vatRate: 19 })
  .build();   // Type-state ensures all required fields present

const validation = await validate(invoice, { profile: 'peppol-bis-3.0' });
if (!validation.ok) console.error(validation.report);  // Diagnostics with citations

const pdf = await render(invoice, { template: 'modern', profile: 'factur-x-en16931' });

const result = await transmit(invoice, {
  route: 'auto',  // Looks up recipient via SMP, picks best path
  fallback: ['peppol', 'fr-ppf', 'email'],
});
// result: { state: 'delivered', bundle: '...', receipts: [...] }
```

### 5.3 CLI

```
invoicekit doctor                                  # Diagnostics: what's missing for your country
invoicekit init                                    # Walk through first invoice
invoicekit convert in.pdf --to=xrechnung-3.0       # Auto-detect input format
invoicekit validate in.xml --profile=peppol-bis    # Diagnostics with citations
invoicekit render invoice.json --to=pdf            # PDF/A-3 with embedded XML
invoicekit send invoice.json --route=auto          # Discover + send (uses managed API)
invoicekit verify bundle.invoicekit                # Court-grade re-verification
invoicekit fuzz                                    # Adversarial generator for testing
invoicekit benchmark                               # Performance dashboard
invoicekit capabilities --from=DE --to=FR --date=2027-01-01
```

### 5.4 LSP

VSCode/Cursor/Neovim/Helix extension. Hover any BT-* term to read EN 16931 explanatory text; click validation diagnostics to jump to source; auto-complete codelist values (VAT category, payment means).

### 5.5 REST API (thin shim over the engine, for non-binding clients)

```http
POST /v1/invoices                                  # Create invoice (idempotent via Idempotency-Key header)
POST /v1/invoices/:id/validate                     # Re-validate against current rulepack
POST /v1/invoices/:id/render                       # Render PDF
POST /v1/invoices/:id/transmit                     # Transmit; returns 202 + tracking ID
GET  /v1/transmissions/:id                         # State machine current state
POST /v1/reconcile                                 # Bulk fingerprint reconciliation
GET  /v1/bundles/:id                               # Download .invoicekit
POST /v1/bundles/verify                            # Server-side verification (returns signed proof)
GET  /v1/capabilities                              # Country/profile/date matrix lookup
```

---

## 6. Monetization

### 6.1 Pricing matrix (provisional — Brenner experiment E2 will validate)

| Tier | Price | Includes |
|------|-------|----------|
| OSS Core | €0 forever | Apache 2.0; lib + CLI + LSP + sandbox; self-host everything; no SLA |
| Pay-as-you-grow | €0 baseline; €0.05/envelope above 100/mo free | Managed transmit, validate, archive; metered |
| Pro Self-Serve | €49/mo + €0.05/envelope above 1,000 | Above + audit log, support credits, country pack of choice |
| **Managed Compliance API** | **€499–€1,999/mo** + per-envelope | **Liability cap, SLA 99.9%, certified country gateways, vault archive, certificate management** |
| Enterprise | Custom (start €25k/yr) | On-prem option, indemnification, custom SLA |

Brenner E2 (60-day pricing test): we'll run two landing pages — "OSS toolkit + metered" vs "Managed API with liability transfer at €499/mo" — and route LinkedIn ads to each. If the API converts 3× higher, we compress to managed-first.

### 6.2 Cost economics

| Cost element | Rough cost per envelope |
|--------------|-------------------------|
| Marginal compute | €0.001 |
| Peppol delivery (partner Y1) | €0.02 |
| Storage (1 year warm) | €0.001 |
| Storage (10 year cold archive) | €0.001 amortized |
| Average AI cloud fallback | €0.005 (only ~10% of inbound) |
| Total marginal | **~€0.025-0.05** |

Break-even at €0.05/envelope and €140k/yr fixed (ISO 27001 + ops): ~3M envelopes/yr. Feasible with ~50 ERP/billing-SaaS embedders at 5k envelopes/month each.

### 6.3 Plan B if E2 says managed-first

- Compress Y1 by 6 months
- Managed Compliance API becomes the lead product
- SDK becomes thin convenience around REST
- OSS core still Apache 2.0
- Repositioning: "Stripe for e-invoicing compliance, with developer ergonomics"

---

## 7. Roadmap

### Year 1 — Foundation

**Q1 (Mar–May 2026)**: Trust core
- IR (Rust) + JSON Schema + TS bindings
- Canonicalization (XML C14N, JCS, byte-stable PDF subset)
- EN 16931 hand-written validator (50 rules)
- KoSIT + Peppol Schematron wrapped via wasmtime-java
- Typst PDF/A-3 renderer + Factur-X embedding
- `invoicekit verify` CLI + `.invoicekit` bundle format
- Public conformance corpus v0.1 (100 invoices across DE/FR/PL/IT/Peppol)
- Brenner E1 (Schematron AOT parity) starts week 1
- Brenner E2 (pricing landing pages) starts week 4
- Brenner E3 (ICP risk-transfer interviews) starts week 1

**Q2 (Jun–Aug 2026)**: Outbound
- All 8 outbound serializers (UBL, CII, Factur-X profiles, XRechnung, Peppol BIS, FatturaPA, KSeF FA(3), GOBL JSON)
- Type-state TS builder
- Native Rust Peppol AS4 sender (start)
- Sandbox mock gateways
- LSP MVP
- Public validator (browser-only WASM)
- ISO 27001 engagement begins
- Apply for OpenPeppol AP membership
- First 5 design-partner customers signed

**Q3 (Sep–Nov 2026)**: Transmission
- Peppol AS4 receiver (wrapping phase4 inside wasmtime-java)
- SDI gateway (Italy, live)
- KSeF gateway (Poland, live)
- ZATCA Phase 2 (Saudi, with cryptographic stamping)
- French PPF integration (Sep 1 mandate goes live)
- Managed Compliance API live in beta
- 10 paying customers
- Public conformance corpus v0.5 (500+ invoices)

**Q4 (Dec 2026–Feb 2027)**: Intake + scale
- OCR pipeline (PaddleOCR + SmolDocling + Qwen2.5-VL cloud)
- Cross-examined witness flow
- Reconciliation API GA
- IRP (India), MyInvois (Malaysia), myDATA (Greece), eTIMS (Kenya)
- ISO 27001 audit
- Public free validator becomes top-3 Google result for "validate XRechnung"
- 30 paying customers, €500k ARR

### Year 2 — Network and trust infrastructure

- Native Rust AS4 receiver (replace phase4 wrap)
- OpenPeppol AP certification
- US, UK, Australia, NZ, Singapore Peppol overlays
- WYSIWYG template designer
- Conformance corpus v1.0 + differential test harness as a standalone product
- Reconciliation engine GA with PO matching, multi-tenancy, audit dashboards
- 100 paying customers, €3M ARR

### Year 3 — Plaid for B2B reconciliation (the Gemini runaway-success bet)

- Embedded reconciliation widget (Stripe-Elements-shape) for ERPs
- Payment orchestration partner (SEPA, ACH, card)
- Take basis points on volume, not seat licenses
- 300 paying customers, €10M ARR; positioned as compliance + payment network

---

## 8. Build sequence (concrete tasks with dependencies)

Each task has a unique ID for beads conversion later. Dependencies in brackets.

### Foundation (no dependencies)

- **T-001**: Set up workspace (Cargo workspace, package layout, CI scaffolding) — 1 week
- **T-002**: License + governance docs (Apache 2.0, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md) — 1 week
- **T-003**: Bun/Deno/Node minimal harness for testing TS bindings — 1 week
- **T-004**: pyo3 minimal harness for Python bindings — 3 days
- **T-005**: ISO 27001 readiness engagement starts (external consultancy) — 0 days (it's a 6-12mo background process)

### IR + canonical (depends on T-001)

- **T-010**: IR struct types in Rust (`crates/ir`) — 2 weeks [T-001]
- **T-011**: IR JSON Schema generation from Rust types — 1 week [T-010]
- **T-012**: TS type generation from JSON Schema — 3 days [T-011]
- **T-013**: GOBL JSON bidirectional adapter — 2 weeks [T-010, T-011]
- **T-014**: XML C14N 1.1 implementation — 1 week [T-010]
- **T-015**: JCS (RFC 8785) implementation — 3 days [T-010]
- **T-016**: Property-based round-trip tests (IR ↔ JSON ↔ XML) — 1 week [T-014, T-015]
- **T-017**: Deterministic invoice fingerprint (blake3) — 2 days [T-014, T-015]

### Validation (depends on IR)

- **T-020**: Hand-written EN 16931 core validator (50 rules) — 3 weeks [T-010]
- **T-021**: wasmtime-java integration: load KoSIT validator JAR + Peppol Schematron — 2 weeks [T-020]
- **T-022**: Validation result schema: rule_id, severity, BT term, location, suggested fix, citation — 1 week [T-020]
- **T-023**: **Brenner E1 — Schematron AOT prototype** — 3 weeks (parity gauntlet) [T-021, T-022]
- **T-024**: Validation rulepack registry with versioning + date pinning — 2 weeks [T-022]
- **T-025**: Time-travel validation API — 1 week [T-024]
- **T-026**: Public free validator web UI (WASM-only, browser, no upload) — 2 weeks [T-021, T-022]

### Serializers (depends on IR)

- **T-030**: UBL 2.1 serializer + parser — 2 weeks [T-010, T-014]
- **T-031**: CII (CrossIndustry) serializer + parser — 2 weeks [T-010, T-014]
- **T-032**: Factur-X / ZUGFeRD all profiles — 2 weeks [T-030, T-031]
- **T-033**: XRechnung 3.x — 2 weeks [T-030]
- **T-034**: Peppol BIS 3.0 — 1 week [T-030]
- **T-035**: FatturaPA / SDI v1.2.2 — 2 weeks [T-031]
- **T-036**: KSeF FA(3) — 2 weeks [T-010]
- **T-037**: CFDI 4.0 Mexico — 2 weeks [T-010]
- **T-038**: Lossiness ledger generator — 1 week [T-030 thru T-037]

### Rendering (depends on serializers)

- **T-040**: Typst integration as Rust crate dependency — 1 week [T-010]
- **T-041**: TS template DSL → Typst compiler — 3 weeks [T-040]
- **T-042**: PDF/A-3 conformance verification (veraPDF-equivalent in Rust) — 3 weeks [T-040]
- **T-043**: Factur-X XML embedding (PDF/A-3 attachment) — 1 week [T-042]
- **T-044**: Deterministic byte-stable subset — 1 week [T-043]
- **T-045**: Accessible HTML5 rendering pipeline — 1 week [T-041]

### Intake (depends on IR + serializers)

- **T-050**: Layer 1 — Factur-X XML detection + extraction from PDF — 1 week [T-040, T-030, T-031]
- **T-051**: Layer 2 — Digital PDF text extraction — 1 week
- **T-052**: Layer 3 — PaddleOCR integration (server-side default) — 2 weeks [T-051]
- **T-053**: Layer 4 — SmolDocling-256M ONNX integration — 2 weeks [T-052]
- **T-054**: Layer 5 — Qwen2.5-VL-7B cloud inference adapter — 1 week
- **T-055**: Cross-examined witness flow (deterministic re-validation) — 2 weeks [T-020, T-054]
- **T-056**: PII redactor for support bundles — 1 week [T-010]
- **T-057**: Bounding-box citation taxonomy (PDF object ID / OCR span / bbox) — 1 week [T-052]

### Transmission (depends on serializers + reconciliation)

- **T-060**: AS4 sender (Rust-native, ~500 LOC) — 3 weeks [T-034]
- **T-061**: phase4 wrap via wasmtime-java for AS4 receiver — 2 weeks [T-021]
- **T-062**: SMP/SML participant discovery client — 1 week [T-060]
- **T-063**: Italy SDI gateway (live) — 3 weeks [T-035]
- **T-064**: Poland KSeF gateway (FA(3) live) — 3 weeks [T-036]
- **T-065**: France PPF/Chorus Pro gateway — 3 weeks [T-033]
- **T-066**: Saudi ZATCA Phase 2 with cryptographic stamping — 3 weeks
- **T-067**: India IRP — 2 weeks
- **T-068**: Malaysia MyInvois — 2 weeks
- **T-069**: Greece myDATA — 2 weeks
- **T-070**: Kenya eTIMS — 2 weeks
- **T-071**: Universal `transmit_mock` gateway with documented failure modes — 2 weeks
- **T-072**: `routePlan(invoice, recipient)` API — 2 weeks [T-062]
- **T-073**: Email fallback delivery (signed PDF+XML attachment) — 1 week

### Reconciliation (depends on transmission)

- **T-080**: State machine implementation (per-country sub-states) — 2 weeks [T-060]
- **T-081**: Reconciliation API + outbox SQL migrations — 1 week [T-017, T-080]
- **T-082**: Webhook dispatcher — 1 week [T-080]
- **T-083**: SSE event stream — 1 week [T-080]

### Evidence + archive

- **T-090**: `.invoicekit` bundle spec + serializer — 1 week [T-010 thru T-080]
- **T-091**: Pluggable archive backend (S3 Object Lock, Azure WORM, local FS, IPFS) — 2 weeks
- **T-092**: RFC 3161 timestamping integration (TSA partner) — 1 week
- **T-093**: eIDAS qualified signature integration (QTSP partner) — 2 weeks
- **T-094**: `invoicekit verify` CLI — 1 week [T-090, T-092, T-093]

### DX

- **T-100**: CLI (`invoicekit` binary) — 1 week (rolling)
- **T-101**: `invoicekit doctor` — 3 days
- **T-102**: `invoicekit init` interactive — 3 days
- **T-103**: LSP — 3 weeks [T-022]
- **T-104**: VS Code / Cursor extension — 1 week [T-103]
- **T-105**: Docs site (Nextra) + per-rule pages + per-country pages — 4 weeks rolling
- **T-106**: Storybook for templates — 2 weeks [T-041]
- **T-107**: GitHub Actions for invoice validation — 1 week [T-026]
- **T-108**: MCP server for AI tools (Claude Code skill, Cursor, etc.) — 1 week [T-021]

### Bindings

- **T-110**: TypeScript SDK (npm `@invoicekit/core`) — 2 weeks rolling [T-012]
- **T-111**: Python SDK (pyo3) — 2 weeks [T-110]
- **T-112**: Go SDK (wazero) — 2 weeks [T-110]
- **T-113**: Java SDK (wasmtime-java) — 2 weeks [T-110]
- **T-114**: .NET SDK (Wasmtime.NET) — 2 weeks [T-110]
- **T-115**: REST shim (Axum) — 2 weeks [T-110]
- **T-116**: Reverse-proxy sidecar container — 1 week [T-115]

### Public corpus / conformance harness (the "CI for invoice compliance" bet)

- **T-120**: Adversarial generator (Rust) — 2 weeks [T-010]
- **T-121**: Public corpus v0.1 (100 invoices) — 2 weeks [T-120]
- **T-122**: Differential test harness — 2 weeks [T-026]
- **T-123**: Public benchmark dashboard — 1 week [T-122]

### Managed Compliance API (the paid tier)

- **T-130**: API gateway + auth + rate limiting — 2 weeks
- **T-131**: Tenant isolation + KMS-per-tenant — 2 weeks [T-130]
- **T-132**: Customer dashboard (audit log, usage, errors) — 3 weeks [T-130]
- **T-133**: Stripe + invoicing for our own SaaS — 1 week
- **T-134**: SLA monitoring + status page + incident tooling — 2 weeks
- **T-135**: Support ticket integration — 1 week
- **T-136**: Two-landing-page pricing test (Brenner E2) — 1 week
- **T-137**: 30 ICP interviews (Brenner E3) — 4 weeks rolling

---

## 9. Risks & mitigations (final consolidated)

| Risk | Mitigation | Falsification |
|------|------------|---------------|
| R1: Schematron AOT XPath quagmire | Wrap JVM validators first; AOT incrementally per ruleset; never gate shipping on AOT | Brenner E1 |
| R2: WASM friction for JVM/.NET ICPs | Ship native bindings via wasmtime-java + Wasmtime.NET; sidecar pattern for paranoid security policies | Brenner E3 + WASM FFI audit |
| R3: EN 16931 IR collapses under national reality | Typed namespaced profile extensions; lossiness ledger; GOBL adapter for interop | Brenner experiment "200-invoice break test" (codex's) |
| R4: Pricing uncanny valley (€49/mo too high, too low) | Two-product approach: OSS lib + metered for devs; Managed Compliance API at €499+/mo with liability for buyers | Brenner E2 |
| R5: OSS users don't pay | Managed Compliance API is the revenue product; OSS is the trust/adoption funnel; basis-point reconciliation revenue at Y3 | Y1 customer interview signal |
| R6: We're not the validator of record | Diff against KoSIT/Saxon/phive in CI; publish parity dashboards; be transparent | Brenner E1 |
| R7: Incumbents move down-market with dev APIs | Speed: ship Y1 with developer obsession before they retrofit; OSS as moat against managed-only competitors | Quarterly competitive review |
| R8: ERP-native distribution (Odoo, MSFT, SAP) eats us | Become their preferred OSS engine via partnerships; Odoo MCP-style integration; MSFT Dynamics connector | Quarterly partnership pipeline |
| R9: AI hallucination in financial docs | Cross-examined witness; deterministic re-validation; bbox citations; confidence thresholds; AI off by default outbound | Continuous regression tests |
| R10: ISO 27001 timeline (6-12 mo gate to becoming AP) | Start Y1 Q1; partner with existing AP for Y1 delivery | Quarterly checkpoint |
| R11: phase4 maintainer goes inactive / license change | Maintain a fork ourselves; build native Rust AS4 receiver in Y2 | Quarterly upstream review |
| R12: GOBL becomes the dominant standard before us | Interop, don't compete; contribute to GOBL specs; preserve unique value (intake, AS4, WASM) | Quarterly GOBL alignment review |

---

## 10. Success metrics

### Year 1 targets

| Metric | Target |
|--------|--------|
| GitHub stars on `invoicekit/core` | 1,000+ |
| OSS contributors | 100+ |
| Countries with full inbound+outbound support | 8 (DE, FR, IT, PL, BE, ES, SA, IN) |
| Paying customers | 10 at €5k+ ACV |
| ARR | €100–500k |
| Public conformance corpus invoices | 500+ |
| WASM cold-start p99 | <50ms in Cloudflare Worker |
| Validator parity vs KoSIT | >99.9% |
| Brenner experiments validated/falsified | 3/3 |
| ISO 27001 audit | Passed |

### Anti-metrics (things we'd be sad about)

- Per-customer support cost > 30% of ACV
- Mean time to first validated invoice for new developer > 10 minutes
- Any silent line-item hallucination making it into a customer's production
- Brand-perception as "AI invoicing" rather than "compliant invoicing"

---

## 11. Open questions for the review rounds

1. **Should we publish the `.invoicekit` bundle format as an open RFC** (potentially with OpenPeppol's blessing), making it a de facto interchange standard? Bigger ambition; longer timeline.
2. **Do we attempt a Year-1 partnership with the German KoSIT** (the official XRechnung validator authority) to formally cross-certify? Free credibility, but a slow bureaucratic path.
3. **Is "Plaid for B2B reconciliation" (Gemini runaway-success bet) compatible with "CI for invoice compliance" (Codex runaway-success bet) on a single product surface, or do they bifurcate into two companies?** Both share the IR + state machine + evidence bundle foundation; reconciliation engine + payment orchestration is the bifurcation point.
4. **Native AS4 receiver in Rust** is genuinely hard. Should we accept that Y2-Y3 retains some JVM dependency, or push native receiver to Y2 Q1 as a hard deadline?
5. **What's the exact MVP feature set for the public free validator** (T-026) such that it ranks for "validate XRechnung" on Google in 90 days?
6. **Sourcing & funding**: this plan implies a team of ~6 engineers + 1 PM + 1 compliance officer over 18 months — roughly €1.5M to break-even. Bootstrap-friendly via OSS funding (GitHub Sponsors, OpenCollective, design-partner LOIs), or do we raise pre-seed?

---

## 12. Where this plan goes next

Per the planning-workflow skill:
1. **Iterative refinement** — paste this plan to codex / gemini for revision rounds (4-5 rounds, target steady-state).
2. **Multi-model blending** — also paste to alternate strong models (Opus 4.7 in the web app would be the strongest reviewer if available).
3. **Convert to beads** — once steady-state, each task above becomes a self-contained bead with the dependency graph preserved (`br create ... --deps ...`).
4. **Polish beads** (6+ rounds) until ready for execution.

This is the initial draft. Subsequent rounds will deepen the per-component design, expand the test plan, and operationalize the team / hiring shape.
codex
I’ll treat this as a plan review, not an implementation task. I’m going to use the planning-workflow skill for the review structure, and I’ll verify the regulatory/date-sensitive claims against current sources before making country-coverage recommendations.
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
web search: 
web search: Germany B2B e-invoicing mandatory 2027 2028 official BMF ...
web search: 
web search: site:bundesfinanzministerium.de e-invoicing B2B Germany January 2025 2027 2028 ...
web search: 
web search: site:sede.agenciatributaria.gob.es VERI*FACTU 2027 July 2027 January 2027 official ...
web search: 
web search: Spanish Tax Agency VeriFactu obligation 2027 official AEAT July 2027 January 2027 ...
web search: 
web search: wasmtime-java GitHub Java bindings for Wasmtime ...
web search: 
web search: Typst PDF/A-3 support official documentation PDF/A attachments ...
web search: 
web search: OpenPeppol PINT specification e-invoicing international official ...
web search: 
web search: Invopop GOBL GitHub invoice JSON schema Apache 2.0 ...
codex
I used the planning-workflow review lens: self-containment, dependency graph, justification, and source grounding. My main verdict: the thesis is strong, but the plan is trying to be a global compliance network, a universal runtime layer, a validator authority, an OCR product, and a Peppol AP too early. The revision should narrow Year 1 around deterministic correctness, versioned rulepacks, EU/Peppol delivery, evidence, and managed operations.

**Regulatory Corrections**
A few date-sensitive claims should be tightened before beads conversion:

- Germany: keep receipt since January 1, 2025 and outbound transition through 2027/2028, but phrase as transition periods, not a simple send mandate.
- France: receiving for all starts September 1, 2026; issuing is phased: large/intermediate from September 1, 2026, SMEs/micro from September 1, 2027.
- Poland: KSeF rollout is phased from February 1, 2026 and April 1, 2026, with smallest taxpayers and sanctions timing needing explicit handling.
- Spain: Verifactu is anti-fraud invoicing-system reporting, not the same thing as B2B e-invoice exchange. Model it separately.
- UAE: update the plan around the UAE Peppol/PINT path and the 2026/2027 implementation timeline.

Sources are linked at the end.

## 1. Replace “WASM-native everywhere” with dual artifacts

**Analysis**

Do not make WASM the universal runtime bet. WASM is excellent for browser, edge, deterministic sandboxing, and local validation. It is not the best default for Python, Java, .NET, Go, or server Node. For those, native bindings or a sidecar are simpler, faster, easier to debug, and less hostile to enterprise observability.

The universal contract should be the deterministic engine API, canonical IR, rulepack semantics, and fixture corpus. The runtime packaging should vary.

Concrete shape:

- `crates/invoicekit-engine`: pure deterministic Rust API.
- `crates/invoicekit-ffi`: stable C ABI.
- `crates/invoicekit-wasm`: browser/edge WASM.
- `bindings/node-napi`: `napi-rs`, not only wasm-bindgen.
- `bindings/python`: `pyo3` + `maturin`.
- `bindings/dotnet`: P/Invoke over C ABI.
- `bindings/java`: JNI/JNA or Java FFM over C ABI.
- `bindings/go`: cgo first, pure-Go REST/sidecar fallback.
- `bindings/rest-shim`: `axum` service for conservative customers.

```diff
- 1. A WASM-native Rust core with native bindings for every major runtime
-    (TypeScript via wasm-bindgen, Python via pyo3, Go via wazero,
-    Java via wasmtime-java, .NET via Wasmtime.NET, plus a CLI binary and
-    a thin REST shim).

+ 1. A deterministic Rust engine with two first-class delivery artifacts:
+    native bindings for server runtimes and WASM for browser/edge/sandbox use.
+    The compatibility contract is the engine API, canonical IR, rulepacks, and
+    conformance corpus, not WASM itself. Runtime packaging is:
+    Node via napi-rs, Python via pyo3/maturin, Go via cgo plus sidecar fallback,
+    Java via JNI/FFM over a C ABI, .NET via P/Invoke, browser/Workers via
+    wasm-bindgen/WASI, CLI via native Rust, and REST via Axum.
```

## 2. Split global invoice semantics from EN 16931

**Analysis**

An EN 16931-rooted IR is good for Europe, but it will bend badly under India IRP, Mexico CFDI, Saudi ZATCA, Brazil NF-e, Malaysia MyInvois, and UAE PINT. EN 16931 should be a profile/view, not the root of every legal model.

Use a layered model:

- `CommercialDocument`: global commercial invoice/credit-note semantics.
- `ProfileView`: EN 16931, Peppol BIS, PINT, KSeF, FatturaPA, CFDI, IRP, ZATCA.
- `JurisdictionExtension`: typed, versioned extensions.
- `LossinessLedger`: required for every projection.
- `Rulepack`: validates a document against a profile and effective date.

```diff
- An EN 16931–rooted semantic IR with first-class country/profile extensions
- and lossless round-trip to UBL 2.1, CII, Factur-X/ZUGFeRD, XRechnung,
- Peppol BIS 3.0, FatturaPA, KSeF FA(3), and others.

+ A layered invoice model:
+ - `CommercialDocument`: global invoice/credit-note semantics, not tied to one law.
+ - `ProfileView`: EN 16931, Peppol BIS/PINT, XRechnung, Factur-X, KSeF,
+   FatturaPA, ZATCA, IRP, CFDI, MyInvois, etc.
+ - `JurisdictionExtension`: typed, namespaced, versioned profile data.
+ - `LossinessLedger`: mandatory for every profile projection.
+ EN 16931 is the Year-1 European anchor, not the universal root for all countries.
```

## 3. Add money, tax, rounding, and codelists as first-class architecture

**Analysis**

The plan under-specifies the highest-risk correctness area: money math and legal codelists. Many invoice bugs come from decimal scale, VAT rounding, tax category subtotals, mixed rates, allowances/charges, currency minor units, reverse charge, exemptions, and stale codelists.

Do this early:

- `crates/money`: `rust_decimal`, string serialization, no floats.
- `crates/codelists`: ISO 3166, ISO 4217, UN/ECE, VAT category, Peppol, country codes.
- `crates/tax-calculation`: invoice arithmetic, rounding policies, tolerance reports.
- `data/codelists/*.toml`: versioned, signed, effective-dated lists.
- `invoicekit explain BR-CO-10`: shows formula, inputs, and source locations.

```diff
+ ### 4.3.1 Money, tax arithmetic, and codelists
+
+ InvoiceKit must never use floating-point arithmetic for monetary values.
+ All amounts use fixed-scale decimal strings at API boundaries and
+ `rust_decimal::Decimal` internally. Currency minor units, tax categories,
+ payment means, endpoint schemes, country codes, and Peppol codelists are
+ versioned rulepack data with effective dates.
+
+ New crates:
+ - `crates/money`
+ - `crates/codelists`
+ - `crates/tax-calculation`
+ - `crates/rulepack`
+
+ Validation diagnostics must include the exact arithmetic expression and source
+ fields used for every total/subtotal failure.
```

## 4. Fix the Java validator architecture

**Analysis**

The current plan says “wasmtime-java integration: load KoSIT validator JAR” and “Java in WASM.” That is not the right model. `wasmtime-java` lets Java host Wasm; it does not make arbitrary Java validators run inside Rust Wasmtime.

Year 1 should use a reference-validator worker:

- `services/validator-worker-jvm`: Java service/container.
- KoSIT, phive, Saxon, Peppol Schematron run there.
- Rust calls it over Unix socket/gRPC/JSON-RPC.
- Managed service and CI use the worker.
- Browser/edge validator is explicitly “lite” until pure Rust parity exists.

```diff
- T-021: wasmtime-java integration: load KoSIT validator JAR + Peppol Schematron — 2 weeks [T-020]

+ T-021: Reference validator worker: KoSIT/phive/Saxon in an isolated JVM
+ service with a stable JSON-RPC contract — 2 weeks [T-022]
+
+ T-021a: Browser/edge validator capability matrix: mark which profiles are
+ pure-WASM and which require the reference worker — 3 days [T-021]
```

```diff
- Year 1 Q1-Q2: wrap KoSIT XRechnung validator + Peppol Schematron via
- `wasmtime-java` (yes — Java in WASM).

+ Year 1 Q1-Q2: wrap KoSIT, phive, Saxon, and Peppol Schematron through an
+ isolated JVM validator worker. Pure Rust/WASM validators are promoted only
+ after parity against the worker is proven.
```

## 5. Move rulepack provenance before validators and serializers

**Analysis**

Compliance tooling fails when rule versions drift silently. Rulepacks need source provenance, effective dates, checksums, signatures, and reproducible update jobs from day one.

Add:

- `rulepacks/en16931/`
- `rulepacks/xrechnung/`
- `rulepacks/peppol-bis/`
- `rulepacks/fr-ctc/`
- `data/sources/*.toml`
- `invoicekit rulepack update`
- `invoicekit capabilities --date`
- CI that refuses unpinned rules.

```diff
+ ### 4.5.1 Rulepack supply chain
+
+ Every rulepack is a signed, versioned artifact containing:
+ - source URLs and retrieval timestamps
+ - upstream version identifiers
+ - effective date ranges
+ - codelist versions
+ - checksums of raw upstream artifacts
+ - generated Rust/JSON metadata
+ - parity fixtures and known gaps
+
+ `invoicekit validate --date=YYYY-MM-DD` must select rulepacks by effective
+ date, not by whatever package version happens to be installed.
```

```diff
+ T-018: Rulepack source registry + signed manifest format — 1 week [T-001]
+ T-019: Codelist updater with provenance checksums — 1 week [T-018]
- T-024: Validation rulepack registry with versioning + date pinning — 2 weeks [T-022]
+ T-024: Validation rulepack registry using signed source manifests,
+ effective dates, and codelist versions — 2 weeks [T-018, T-019, T-022]
```

## 6. Re-sequence state, outbox, idempotency, and transmission

**Analysis**

Transmission should depend on reconciliation/state infrastructure, not the other way around. Otherwise each gateway connector invents retry behavior, idempotency, state transitions, and error semantics independently.

Do this order:

1. State model.
2. Outbox table and job runner.
3. Idempotency keys.
4. Gateway adapter trait.
5. Mock gateway.
6. Partner/live gateways.

Concrete shape:

- `crates/transmit-core`
- `crates/outbox-sql`
- `crates/gateway-api`
- `services/transmission-worker`
- `migrations/postgres/invoicekit_outbox.sql`

```diff
- ### Transmission (depends on serializers + reconciliation)
+ ### Reconciliation and transmission core (must precede live gateways)

+ T-058: Gateway adapter trait and normalized gateway error taxonomy — 1 week [T-022]
+ T-059: Outbox SQL schema, idempotency model, retry policy, dead-letter states — 2 weeks [T-017, T-058]
+ T-059a: Transmission worker with backoff, rate limits, circuit breakers,
+ and structured gateway logs — 2 weeks [T-059]

- T-060: AS4 sender (Rust-native, ~500 LOC) — 3 weeks [T-034]
+ T-060: Peppol partner-AP adapter for Year-1 live delivery — 2 weeks [T-034, T-059a]

- T-080: State machine implementation (per-country sub-states) — 2 weeks [T-060]
+ T-080: State machine implementation (per-country sub-states) — 2 weeks [T-058]
```

## 7. Stop calling native AS4 a 500 LOC Year-1 task

**Analysis**

AS4 sender/receiver certification is not a 500-line feature. WS-Security, certificates, signing, canonicalization, ebMS3, SMP/SML, retries, receipts, conformance testing, and OpenPeppol operational rules are the hard part.

Year 1 should ship value through:

- partner AP API,
- phase4 reference path,
- Peppol document packaging,
- SMP/SML lookup,
- conformance harness,
- managed evidence and reconciliation.

Native AS4 should be a research track until it passes a conformance suite.

```diff
- Native Rust AS4 sender (node42 proves this is ~500 LOC; we'll do it cleaner)

+ Native Rust AS4 is a long-running conformance project, not the Year-1 live
+ delivery dependency. Year 1 live Peppol delivery uses a partner AP plus a
+ phase4-backed reference adapter. Native AS4 sender/receiver move behind a
+ conformance gate and cannot be marketed as production until OpenPeppol tests,
+ certificate handling, WS-Security, SMP/SML, receipts, and replay behavior pass.
```

## 8. Change `.invoicekit` from tar-gz + replay.sh to deterministic, signed bundles

**Analysis**

`tar-gz` can be made deterministic, but it is easy to get wrong because gzip metadata and tar mtimes leak nondeterminism. `replay.sh` is also a weak verification primitive: shell is platform-specific and risky to execute.

Use:

- deterministic `tar.zst` with normalized metadata, or a directory bundle plus `.ikb` packed form,
- `manifest.json` with BLAKE3 hashes,
- DSSE/JWS signature over manifest,
- `replay.json`, not `replay.sh`,
- encrypted legal bundle vs redacted support bundle,
- no source bytes in public/support artifacts by default.

```diff
- `.invoicekit` is a tar-gz archive (open spec) containing:
+ `.invoicekit` is an open deterministic evidence-bundle spec. The canonical
+ representation is a directory tree plus a signed `manifest.json`; the portable
+ packed form is `.ikb` (`tar.zst` with normalized uid/gid/mtime/order).
+ Verification never executes shell scripts.

- replay.sh                    # Single shell script that re-runs validation + verification
+ replay.json                  # Declarative replay recipe consumed by `invoicekit verify`
+ signatures/manifest.dsse     # DSSE/JWS signature over manifest hash
+ privacy/redaction-map.json   # Optional support-bundle redaction trace
```

## 9. Do not reimplement veraPDF

**Analysis**

“veraPDF-equivalent in Rust” is an expensive side quest. Use veraPDF as the reference verifier in CI/managed service. If you need local/offline verification, ship an optional verifier worker or document the dependency. Keep Rust checks limited to structural invariants you own.

Also confirm Typst’s PDF/A-3 and attachment behavior against current Typst releases before promising deterministic legal output. Keep WeasyPrint/Prince or a controlled PDF library as a fallback if Typst cannot satisfy embedded XML + PDF/A-3 requirements across profiles.

```diff
- T-042: PDF/A-3 conformance verification (veraPDF-equivalent in Rust) — 3 weeks [T-040]
+ T-042: PDF/A-3 verification adapter using veraPDF as the reference oracle,
+ with lightweight Rust preflight checks only — 1 week [T-040]

+ T-042a: Renderer fallback decision gate: prove Typst can produce required
+ PDF/A-3 + embedded XML artifacts for Factur-X/ZUGFeRD fixtures, or keep a
+ secondary renderer behind the same `RenderBackend` trait — 1 week [T-042]
```

## 10. Narrow Year-1 country scope and add a coverage matrix

**Analysis**

The plan promises too many countries at “full inbound+outbound support” depth. That is dangerous because “support” has multiple meanings:

- syntax generation,
- validation,
- legal submission,
- receipt/inbound,
- archive,
- correction/cancellation,
- sandbox,
- production certification,
- support SLA.

Use an explicit matrix and mark maturity.

Year 1 should focus on:

- DE: XRechnung, ZUGFeRD/Factur-X, EN 16931.
- FR: Factur-X/UBL/CII plus PA/PDP/e-reporting readiness.
- BE: Peppol BIS.
- PL: KSeF FA(3), offline/QR/access-token flows.
- IT: FatturaPA/SDI.
- ES: Verifactu as reporting-system compliance, not just e-invoice.
- Peppol BIS/PINT base.
- UAE PINT track if commercial demand exists.

Move India, Saudi, Malaysia, Greece, Kenya to “connector research / partner-led / Y2 unless design partner funds it.”

```diff
- Cover the German/French/Polish/Belgian/Peppol stack with full
- conformance-tested support for inbound + outbound flows.

+ Cover a Year-1 regulated European/Peppol corridor with explicit maturity
+ labels per country/profile: `serialize`, `validate`, `render`, `sandbox`,
+ `partner-live`, `native-live`, `inbound`, `archive`, `correction`, `SLA`.
+ Do not call a country "supported" unless the matrix shows which cells are GA.
```

```diff
- Countries with full inbound+outbound support | 8 (DE, FR, IT, PL, BE, ES, SA, IN)
+ Countries/profiles with GA maturity cells:
+ DE, FR, IT, PL, BE, ES, Peppol BIS/PINT base.
+ SA, IN, MY, GR, KE are partner/research tracks unless backed by a signed
+ design partner.
```

## 11. Add e-reporting as a separate product surface

**Analysis**

Several regimes are not just invoice exchange. France, Spain, India, Greece, Hungary, Poland, and others involve reporting, clearance, or tax authority state. Treating them as serializers or transmitters will produce a bad architecture.

Add a `report-*` family:

- `report-fr-ctc`
- `report-es-verifactu`
- `report-gr-mydata`
- `report-in-gst`
- `report-pl-ksef`
- `report-it-sdi`

```diff
- ├── transmit-fr-ppf/          # France PPF/PDP gateway
+ ├── report-fr-ctc/            # France PA/PDP e-invoicing + e-reporting flows
+ ├── report-es-verifactu/      # Spain anti-fraud invoice-system reporting
+ ├── report-gr-mydata/         # Greece myDATA reporting
+ ├── report-in-gst/            # India GST IRP/e-waybill/reporting adapters

- ├── transmit-pl-ksef/         # Poland KSeF FA(3) gateway
+ ├── transmit-pl-ksef/         # Poland KSeF FA(3) clearance/submission
```

## 12. Move managed-platform security to the beginning

**Analysis**

Tenant isolation, KMS, audit logs, GDPR/data residency, API keys, scoped tokens, usage metering, and retention cannot be bolted on after live transmission. They shape IDs, schemas, storage, logs, evidence bundles, and support tooling.

Add these before customer beta:

- `tenant_id` everywhere persisted.
- KMS-per-tenant envelope encryption.
- scoped API keys and OIDC.
- webhook signing.
- immutable audit event log.
- data residency flags.
- PII redaction.
- backup/restore drills.
- SBOM/SLSA/security release process.

```diff
- ### Managed Compliance API (the paid tier)
+ ### Managed Compliance Platform Foundation

+ T-128: Tenant model, scoped API keys, OIDC, RBAC, and audit-event schema — 2 weeks [T-001]
+ T-129: Envelope encryption with KMS-per-tenant, key rotation, and data residency tags — 2 weeks [T-128]
+ T-129a: Webhook signing, replay protection, and event-delivery idempotency — 1 week [T-128]
+ T-129b: SBOM, dependency scanning, signed releases, and security advisory process — 1 week [T-002]

- T-130: API gateway + auth + rate limiting — 2 weeks
+ T-130: API gateway + auth + rate limiting — 2 weeks [T-128]
```

## 13. Add observability and SLO engineering before SLA claims

**Analysis**

“SLA 99.9%” cannot depend on a status page task in Q4. Gateway integrations fail in country-specific ways. You need trace IDs, per-gateway latency/error metrics, dead-letter queues, replay tooling, customer-visible incident IDs, and reconciliation-first support.

```diff
+ ### 4.12 Operations and observability
+
+ Every transmission has:
+ - trace_id
+ - tenant_id
+ - idempotency_key
+ - gateway_attempt_id
+ - normalized state transition
+ - raw gateway receipt hash
+ - retry/dead-letter metadata
+
+ Managed API SLOs are defined per operation: validate, render, transmit enqueue,
+ gateway accepted, archive write, webhook delivery. Gateway legal acceptance is
+ never conflated with API availability.
```

```diff
+ T-134a: OpenTelemetry tracing, metrics, log redaction, and per-gateway dashboards — 2 weeks [T-059a]
+ T-134b: Replay/admin tooling for stuck transmissions and dead-letter queues — 1 week [T-134a]
```

## 14. Separate pure builder from enrichment/network calls

**Analysis**

This API shape is risky:

```ts
supplier: { vat: 'DE123456789' } // Rest auto-resolved via VIES
```

A builder should be deterministic and offline. Network enrichment should be explicit, cancellable, cached, and privacy-aware. Otherwise `build()` becomes slow, flaky, and surprising.

```diff
- const invoice = Invoice.builder({
-   supplier: { vat: 'DE123456789' },  // Rest auto-resolved via VIES
+ const draft = createInvoiceDraft({
+   supplier: { vat: 'DE123456789' },
    customer: { vat: 'FR987654321' },
    currency: 'EUR',
- })
+ });
+
+ const enriched = await client.enrich(draft, {
+   sources: ['vies'],
+   cache: 'tenant',
+   consent: true,
+ });
+
+ const invoice = Invoice.builder(enriched)
```

Also split packages:

```diff
- import { Invoice, validate, render, transmit, verify } from '@invoicekit/core';
+ import { createInvoiceDraft, validateLocal } from '@invoicekit/core';
+ import { renderPdf } from '@invoicekit/render';
+ import { InvoiceKitClient } from '@invoicekit/managed';
```

## 15. Fix DX commands and first-touch flow

**Analysis**

The sample says `npx` and then `pnpm exec`. That is inconsistent. Since this repo mandates Bun, the internal plan should use Bun. Public docs can still list npm/pnpm/yarn equivalents later, but the default should be one clean path.

Also add `doctor` before `init` does anything expensive.

```diff
- $ npx invoicekit init
+ $ bunx invoicekit init

- Try it: pnpm exec invoicekit send examples/first-invoice.ts
+ Try it: bunx invoicekit validate examples/first-invoice.ts --profile=peppol-bis
+ Then:   bunx invoicekit send examples/first-invoice.ts --mode=sandbox
```

```diff
+ invoicekit doctor --country=DE --profile=xrechnung
+ # Checks local WASM/native engine, reference validator availability,
+ # rulepack freshness, PDF/A verifier, API key scopes, and country capabilities.
```

## 16. Make the public validator server-assisted, not browser-only

**Analysis**

A browser-only validator cannot run the JVM reference stack. If the SEO/public validator claims official-grade XRechnung/Peppol validation but only runs a pure-WASM subset, that will hurt trust.

Use two modes:

- Local/private browser validation for pure Rust rules.
- Server-side reference validation for official parity, with no-retention and optional client-side redaction.

```diff
- Public free validator web UI (WASM-only, browser, no upload)

+ Public validator with two explicit modes:
+ - Local mode: browser-only WASM, privacy-first, limited to pure Rust rulepacks.
+ - Reference mode: server-side KoSIT/phive/Saxon worker, no-retention by default,
+   returns official-parity diagnostics and rulepack provenance.
```

## 17. Add corpus licensing and real-fixture policy

**Analysis**

A public invoice corpus is a moat, but it is also a licensing/privacy trap. Do not default to CC-BY-SA for everything. Use synthetic fixtures under Apache-2.0/CC0, and only include real-world examples when rights, redaction, and redistribution are explicit.

```diff
- conformance-corpus/   # 500+ adversarial test invoices; public CC-BY-SA

+ conformance-corpus/
+   synthetic/          # CC0/Apache-2.0 generated fixtures
+   licensed-real/      # explicitly licensed, redacted real invoices
+   private-regression/ # non-public customer/support fixtures
+   generators/         # adversarial fixture generators
```

```diff
+ T-120a: Corpus licensing/redaction policy and fixture metadata schema — 3 days [T-002]
```

## 18. Add explicit performance budgets by package

**Analysis**

“WASM cold-start p99 <50ms” is not meaningful if the package includes PDF rendering, rulepacks, and OCR. Split performance budgets:

- core parse/canonicalize/validate-lite,
- reference validation,
- render,
- bundle verify,
- transmission enqueue,
- OCR.

```diff
- WASM cold-start p99 | <50ms in Cloudflare Worker

+ Performance budgets:
+ - `@invoicekit/core` WASM cold start p99 <50ms on Cloudflare Workers
+ - local EN 16931 core validation p95 <25ms for 100-line invoice
+ - UBL parse + canonicalize p95 <50ms for 1 MB XML
+ - PDF/A-3 render p95 <2s for 10-page invoice on managed worker
+ - reference validator p95 measured separately by profile/rulepack
+ - OCR/VLM excluded from core-runtime budgets
```

## 19. Add source-watch automation for mandates and rule changes

**Analysis**

The plan relies on fast-changing laws, codelists, and profiles. Make the update process a product capability.

```diff
+ T-006: Compliance source-watch bot: monitor official sources, produce
+ rulepack/codelist update PRs, and open beads for changed mandates — 1 week [T-001]
+
+ T-006a: `invoicekit capabilities` data model generated from source-watch
+ manifests, with confidence level and last-verified date — 1 week [T-006]
```

## 20. Fix binding dependencies

**Analysis**

The SDK tasks should not depend on the TypeScript SDK. They should depend on the stable engine ABI and golden fixtures. Otherwise TS becomes an accidental source of truth.

```diff
- T-111: Python SDK (pyo3) — 2 weeks [T-110]
- T-112: Go SDK (wazero) — 2 weeks [T-110]
- T-113: Java SDK (wasmtime-java) — 2 weeks [T-110]
- T-114: .NET SDK (Wasmtime.NET) — 2 weeks [T-110]

+ T-109: Stable engine ABI contract + cross-language golden fixtures — 2 weeks [T-010, T-022]
+ T-110: TypeScript SDK (`@invoicekit/core`, `@invoicekit/render`, `@invoicekit/managed`) — 2 weeks [T-109]
+ T-111: Python SDK (pyo3/maturin) — 2 weeks [T-109]
+ T-112: Go SDK (cgo + REST sidecar fallback) — 2 weeks [T-109]
+ T-113: Java SDK (JNI/FFM over C ABI + REST sidecar fallback) — 2 weeks [T-109]
+ T-114: .NET SDK (P/Invoke over C ABI + REST sidecar fallback) — 2 weeks [T-109]
```

**Net recommendation**

The best revised plan is not “smaller” in ambition, but it should be stricter about maturity labels and dependencies. Make the deterministic core, rulepack supply chain, evidence format, outbox/state machine, and managed operational substrate the Year-1 spine. Treat native AS4, OCR/VLM, global country coverage, and all-runtime WASM as expansion tracks gated by conformance evidence.

Sources used: [Germany BMF e-invoicing FAQ](https://www.bundesfinanzministerium.de/Content/EN/FAQ/e-invoicing.html), [France impots.gouv reform overview](https://www.impots.gouv.fr/invoicing-and-e-reporting), [France official formats](https://www.impots.gouv.fr/specifications-externes-b2b), [Poland KSeF dates](https://www.podatki.gov.pl/ksef/), [Belgium FPS Finance e-invoicing](https://finance.belgium.be/en/e-invoicing), [Spain AEAT Verifactu FAQ](https://sede.agenciatributaria.gob.es/Sede/en_gb/iva/sistemas-informaticos-facturacion-verifactu/preguntas-frecuentes.html), [UAE MoF eInvoicing](https://mof.gov.ae/einvoicing/), [ZATCA e-invoicing waves](https://zatca.gov.sa/en/E-Invoicing/Introduction/Guidelines/Pages/default.aspx), [Typst PDF docs](https://typst.app/docs/reference/model/document/), [wasmtime-java](https://github.com/bytecodealliance/wasmtime-java), [phase4](https://github.com/phax/phase4), [OpenPeppol BIS Billing 3.0](https://docs.peppol.eu/poacc/billing/3.0/), [GOBL](https://github.com/invopop/gobl).
tokens used
52,691
I used the planning-workflow review lens: self-containment, dependency graph, justification, and source grounding. My main verdict: the thesis is strong, but the plan is trying to be a global compliance network, a universal runtime layer, a validator authority, an OCR product, and a Peppol AP too early. The revision should narrow Year 1 around deterministic correctness, versioned rulepacks, EU/Peppol delivery, evidence, and managed operations.

**Regulatory Corrections**
A few date-sensitive claims should be tightened before beads conversion:

- Germany: keep receipt since January 1, 2025 and outbound transition through 2027/2028, but phrase as transition periods, not a simple send mandate.
- France: receiving for all starts September 1, 2026; issuing is phased: large/intermediate from September 1, 2026, SMEs/micro from September 1, 2027.
- Poland: KSeF rollout is phased from February 1, 2026 and April 1, 2026, with smallest taxpayers and sanctions timing needing explicit handling.
- Spain: Verifactu is anti-fraud invoicing-system reporting, not the same thing as B2B e-invoice exchange. Model it separately.
- UAE: update the plan around the UAE Peppol/PINT path and the 2026/2027 implementation timeline.

Sources are linked at the end.

## 1. Replace “WASM-native everywhere” with dual artifacts

**Analysis**

Do not make WASM the universal runtime bet. WASM is excellent for browser, edge, deterministic sandboxing, and local validation. It is not the best default for Python, Java, .NET, Go, or server Node. For those, native bindings or a sidecar are simpler, faster, easier to debug, and less hostile to enterprise observability.

The universal contract should be the deterministic engine API, canonical IR, rulepack semantics, and fixture corpus. The runtime packaging should vary.

Concrete shape:

- `crates/invoicekit-engine`: pure deterministic Rust API.
- `crates/invoicekit-ffi`: stable C ABI.
- `crates/invoicekit-wasm`: browser/edge WASM.
- `bindings/node-napi`: `napi-rs`, not only wasm-bindgen.
- `bindings/python`: `pyo3` + `maturin`.
- `bindings/dotnet`: P/Invoke over C ABI.
- `bindings/java`: JNI/JNA or Java FFM over C ABI.
- `bindings/go`: cgo first, pure-Go REST/sidecar fallback.
- `bindings/rest-shim`: `axum` service for conservative customers.

```diff
- 1. A WASM-native Rust core with native bindings for every major runtime
-    (TypeScript via wasm-bindgen, Python via pyo3, Go via wazero,
-    Java via wasmtime-java, .NET via Wasmtime.NET, plus a CLI binary and
-    a thin REST shim).

+ 1. A deterministic Rust engine with two first-class delivery artifacts:
+    native bindings for server runtimes and WASM for browser/edge/sandbox use.
+    The compatibility contract is the engine API, canonical IR, rulepacks, and
+    conformance corpus, not WASM itself. Runtime packaging is:
+    Node via napi-rs, Python via pyo3/maturin, Go via cgo plus sidecar fallback,
+    Java via JNI/FFM over a C ABI, .NET via P/Invoke, browser/Workers via
+    wasm-bindgen/WASI, CLI via native Rust, and REST via Axum.
```

## 2. Split global invoice semantics from EN 16931

**Analysis**

An EN 16931-rooted IR is good for Europe, but it will bend badly under India IRP, Mexico CFDI, Saudi ZATCA, Brazil NF-e, Malaysia MyInvois, and UAE PINT. EN 16931 should be a profile/view, not the root of every legal model.

Use a layered model:

- `CommercialDocument`: global commercial invoice/credit-note semantics.
- `ProfileView`: EN 16931, Peppol BIS, PINT, KSeF, FatturaPA, CFDI, IRP, ZATCA.
- `JurisdictionExtension`: typed, versioned extensions.
- `LossinessLedger`: required for every projection.
- `Rulepack`: validates a document against a profile and effective date.

```diff
- An EN 16931–rooted semantic IR with first-class country/profile extensions
- and lossless round-trip to UBL 2.1, CII, Factur-X/ZUGFeRD, XRechnung,
- Peppol BIS 3.0, FatturaPA, KSeF FA(3), and others.

+ A layered invoice model:
+ - `CommercialDocument`: global invoice/credit-note semantics, not tied to one law.
+ - `ProfileView`: EN 16931, Peppol BIS/PINT, XRechnung, Factur-X, KSeF,
+   FatturaPA, ZATCA, IRP, CFDI, MyInvois, etc.
+ - `JurisdictionExtension`: typed, namespaced, versioned profile data.
+ - `LossinessLedger`: mandatory for every profile projection.
+ EN 16931 is the Year-1 European anchor, not the universal root for all countries.
```

## 3. Add money, tax, rounding, and codelists as first-class architecture

**Analysis**

The plan under-specifies the highest-risk correctness area: money math and legal codelists. Many invoice bugs come from decimal scale, VAT rounding, tax category subtotals, mixed rates, allowances/charges, currency minor units, reverse charge, exemptions, and stale codelists.

Do this early:

- `crates/money`: `rust_decimal`, string serialization, no floats.
- `crates/codelists`: ISO 3166, ISO 4217, UN/ECE, VAT category, Peppol, country codes.
- `crates/tax-calculation`: invoice arithmetic, rounding policies, tolerance reports.
- `data/codelists/*.toml`: versioned, signed, effective-dated lists.
- `invoicekit explain BR-CO-10`: shows formula, inputs, and source locations.

```diff
+ ### 4.3.1 Money, tax arithmetic, and codelists
+
+ InvoiceKit must never use floating-point arithmetic for monetary values.
+ All amounts use fixed-scale decimal strings at API boundaries and
+ `rust_decimal::Decimal` internally. Currency minor units, tax categories,
+ payment means, endpoint schemes, country codes, and Peppol codelists are
+ versioned rulepack data with effective dates.
+
+ New crates:
+ - `crates/money`
+ - `crates/codelists`
+ - `crates/tax-calculation`
+ - `crates/rulepack`
+
+ Validation diagnostics must include the exact arithmetic expression and source
+ fields used for every total/subtotal failure.
```

## 4. Fix the Java validator architecture

**Analysis**

The current plan says “wasmtime-java integration: load KoSIT validator JAR” and “Java in WASM.” That is not the right model. `wasmtime-java` lets Java host Wasm; it does not make arbitrary Java validators run inside Rust Wasmtime.

Year 1 should use a reference-validator worker:

- `services/validator-worker-jvm`: Java service/container.
- KoSIT, phive, Saxon, Peppol Schematron run there.
- Rust calls it over Unix socket/gRPC/JSON-RPC.
- Managed service and CI use the worker.
- Browser/edge validator is explicitly “lite” until pure Rust parity exists.

```diff
- T-021: wasmtime-java integration: load KoSIT validator JAR + Peppol Schematron — 2 weeks [T-020]

+ T-021: Reference validator worker: KoSIT/phive/Saxon in an isolated JVM
+ service with a stable JSON-RPC contract — 2 weeks [T-022]
+
+ T-021a: Browser/edge validator capability matrix: mark which profiles are
+ pure-WASM and which require the reference worker — 3 days [T-021]
```

```diff
- Year 1 Q1-Q2: wrap KoSIT XRechnung validator + Peppol Schematron via
- `wasmtime-java` (yes — Java in WASM).

+ Year 1 Q1-Q2: wrap KoSIT, phive, Saxon, and Peppol Schematron through an
+ isolated JVM validator worker. Pure Rust/WASM validators are promoted only
+ after parity against the worker is proven.
```

## 5. Move rulepack provenance before validators and serializers

**Analysis**

Compliance tooling fails when rule versions drift silently. Rulepacks need source provenance, effective dates, checksums, signatures, and reproducible update jobs from day one.

Add:

- `rulepacks/en16931/`
- `rulepacks/xrechnung/`
- `rulepacks/peppol-bis/`
- `rulepacks/fr-ctc/`
- `data/sources/*.toml`
- `invoicekit rulepack update`
- `invoicekit capabilities --date`
- CI that refuses unpinned rules.

```diff
+ ### 4.5.1 Rulepack supply chain
+
+ Every rulepack is a signed, versioned artifact containing:
+ - source URLs and retrieval timestamps
+ - upstream version identifiers
+ - effective date ranges
+ - codelist versions
+ - checksums of raw upstream artifacts
+ - generated Rust/JSON metadata
+ - parity fixtures and known gaps
+
+ `invoicekit validate --date=YYYY-MM-DD` must select rulepacks by effective
+ date, not by whatever package version happens to be installed.
```

```diff
+ T-018: Rulepack source registry + signed manifest format — 1 week [T-001]
+ T-019: Codelist updater with provenance checksums — 1 week [T-018]
- T-024: Validation rulepack registry with versioning + date pinning — 2 weeks [T-022]
+ T-024: Validation rulepack registry using signed source manifests,
+ effective dates, and codelist versions — 2 weeks [T-018, T-019, T-022]
```

## 6. Re-sequence state, outbox, idempotency, and transmission

**Analysis**

Transmission should depend on reconciliation/state infrastructure, not the other way around. Otherwise each gateway connector invents retry behavior, idempotency, state transitions, and error semantics independently.

Do this order:

1. State model.
2. Outbox table and job runner.
3. Idempotency keys.
4. Gateway adapter trait.
5. Mock gateway.
6. Partner/live gateways.

Concrete shape:

- `crates/transmit-core`
- `crates/outbox-sql`
- `crates/gateway-api`
- `services/transmission-worker`
- `migrations/postgres/invoicekit_outbox.sql`

```diff
- ### Transmission (depends on serializers + reconciliation)
+ ### Reconciliation and transmission core (must precede live gateways)

+ T-058: Gateway adapter trait and normalized gateway error taxonomy — 1 week [T-022]
+ T-059: Outbox SQL schema, idempotency model, retry policy, dead-letter states — 2 weeks [T-017, T-058]
+ T-059a: Transmission worker with backoff, rate limits, circuit breakers,
+ and structured gateway logs — 2 weeks [T-059]

- T-060: AS4 sender (Rust-native, ~500 LOC) — 3 weeks [T-034]
+ T-060: Peppol partner-AP adapter for Year-1 live delivery — 2 weeks [T-034, T-059a]

- T-080: State machine implementation (per-country sub-states) — 2 weeks [T-060]
+ T-080: State machine implementation (per-country sub-states) — 2 weeks [T-058]
```

## 7. Stop calling native AS4 a 500 LOC Year-1 task

**Analysis**

AS4 sender/receiver certification is not a 500-line feature. WS-Security, certificates, signing, canonicalization, ebMS3, SMP/SML, retries, receipts, conformance testing, and OpenPeppol operational rules are the hard part.

Year 1 should ship value through:

- partner AP API,
- phase4 reference path,
- Peppol document packaging,
- SMP/SML lookup,
- conformance harness,
- managed evidence and reconciliation.

Native AS4 should be a research track until it passes a conformance suite.

```diff
- Native Rust AS4 sender (node42 proves this is ~500 LOC; we'll do it cleaner)

+ Native Rust AS4 is a long-running conformance project, not the Year-1 live
+ delivery dependency. Year 1 live Peppol delivery uses a partner AP plus a
+ phase4-backed reference adapter. Native AS4 sender/receiver move behind a
+ conformance gate and cannot be marketed as production until OpenPeppol tests,
+ certificate handling, WS-Security, SMP/SML, receipts, and replay behavior pass.
```

## 8. Change `.invoicekit` from tar-gz + replay.sh to deterministic, signed bundles

**Analysis**

`tar-gz` can be made deterministic, but it is easy to get wrong because gzip metadata and tar mtimes leak nondeterminism. `replay.sh` is also a weak verification primitive: shell is platform-specific and risky to execute.

Use:

- deterministic `tar.zst` with normalized metadata, or a directory bundle plus `.ikb` packed form,
- `manifest.json` with BLAKE3 hashes,
- DSSE/JWS signature over manifest,
- `replay.json`, not `replay.sh`,
- encrypted legal bundle vs redacted support bundle,
- no source bytes in public/support artifacts by default.

```diff
- `.invoicekit` is a tar-gz archive (open spec) containing:
+ `.invoicekit` is an open deterministic evidence-bundle spec. The canonical
+ representation is a directory tree plus a signed `manifest.json`; the portable
+ packed form is `.ikb` (`tar.zst` with normalized uid/gid/mtime/order).
+ Verification never executes shell scripts.

- replay.sh                    # Single shell script that re-runs validation + verification
+ replay.json                  # Declarative replay recipe consumed by `invoicekit verify`
+ signatures/manifest.dsse     # DSSE/JWS signature over manifest hash
+ privacy/redaction-map.json   # Optional support-bundle redaction trace
```

## 9. Do not reimplement veraPDF

**Analysis**

“veraPDF-equivalent in Rust” is an expensive side quest. Use veraPDF as the reference verifier in CI/managed service. If you need local/offline verification, ship an optional verifier worker or document the dependency. Keep Rust checks limited to structural invariants you own.

Also confirm Typst’s PDF/A-3 and attachment behavior against current Typst releases before promising deterministic legal output. Keep WeasyPrint/Prince or a controlled PDF library as a fallback if Typst cannot satisfy embedded XML + PDF/A-3 requirements across profiles.

```diff
- T-042: PDF/A-3 conformance verification (veraPDF-equivalent in Rust) — 3 weeks [T-040]
+ T-042: PDF/A-3 verification adapter using veraPDF as the reference oracle,
+ with lightweight Rust preflight checks only — 1 week [T-040]

+ T-042a: Renderer fallback decision gate: prove Typst can produce required
+ PDF/A-3 + embedded XML artifacts for Factur-X/ZUGFeRD fixtures, or keep a
+ secondary renderer behind the same `RenderBackend` trait — 1 week [T-042]
```

## 10. Narrow Year-1 country scope and add a coverage matrix

**Analysis**

The plan promises too many countries at “full inbound+outbound support” depth. That is dangerous because “support” has multiple meanings:

- syntax generation,
- validation,
- legal submission,
- receipt/inbound,
- archive,
- correction/cancellation,
- sandbox,
- production certification,
- support SLA.

Use an explicit matrix and mark maturity.

Year 1 should focus on:

- DE: XRechnung, ZUGFeRD/Factur-X, EN 16931.
- FR: Factur-X/UBL/CII plus PA/PDP/e-reporting readiness.
- BE: Peppol BIS.
- PL: KSeF FA(3), offline/QR/access-token flows.
- IT: FatturaPA/SDI.
- ES: Verifactu as reporting-system compliance, not just e-invoice.
- Peppol BIS/PINT base.
- UAE PINT track if commercial demand exists.

Move India, Saudi, Malaysia, Greece, Kenya to “connector research / partner-led / Y2 unless design partner funds it.”

```diff
- Cover the German/French/Polish/Belgian/Peppol stack with full
- conformance-tested support for inbound + outbound flows.

+ Cover a Year-1 regulated European/Peppol corridor with explicit maturity
+ labels per country/profile: `serialize`, `validate`, `render`, `sandbox`,
+ `partner-live`, `native-live`, `inbound`, `archive`, `correction`, `SLA`.
+ Do not call a country "supported" unless the matrix shows which cells are GA.
```

```diff
- Countries with full inbound+outbound support | 8 (DE, FR, IT, PL, BE, ES, SA, IN)
+ Countries/profiles with GA maturity cells:
+ DE, FR, IT, PL, BE, ES, Peppol BIS/PINT base.
+ SA, IN, MY, GR, KE are partner/research tracks unless backed by a signed
+ design partner.
```

## 11. Add e-reporting as a separate product surface

**Analysis**

Several regimes are not just invoice exchange. France, Spain, India, Greece, Hungary, Poland, and others involve reporting, clearance, or tax authority state. Treating them as serializers or transmitters will produce a bad architecture.

Add a `report-*` family:

- `report-fr-ctc`
- `report-es-verifactu`
- `report-gr-mydata`
- `report-in-gst`
- `report-pl-ksef`
- `report-it-sdi`

```diff
- ├── transmit-fr-ppf/          # France PPF/PDP gateway
+ ├── report-fr-ctc/            # France PA/PDP e-invoicing + e-reporting flows
+ ├── report-es-verifactu/      # Spain anti-fraud invoice-system reporting
+ ├── report-gr-mydata/         # Greece myDATA reporting
+ ├── report-in-gst/            # India GST IRP/e-waybill/reporting adapters

- ├── transmit-pl-ksef/         # Poland KSeF FA(3) gateway
+ ├── transmit-pl-ksef/         # Poland KSeF FA(3) clearance/submission
```

## 12. Move managed-platform security to the beginning

**Analysis**

Tenant isolation, KMS, audit logs, GDPR/data residency, API keys, scoped tokens, usage metering, and retention cannot be bolted on after live transmission. They shape IDs, schemas, storage, logs, evidence bundles, and support tooling.

Add these before customer beta:

- `tenant_id` everywhere persisted.
- KMS-per-tenant envelope encryption.
- scoped API keys and OIDC.
- webhook signing.
- immutable audit event log.
- data residency flags.
- PII redaction.
- backup/restore drills.
- SBOM/SLSA/security release process.

```diff
- ### Managed Compliance API (the paid tier)
+ ### Managed Compliance Platform Foundation

+ T-128: Tenant model, scoped API keys, OIDC, RBAC, and audit-event schema — 2 weeks [T-001]
+ T-129: Envelope encryption with KMS-per-tenant, key rotation, and data residency tags — 2 weeks [T-128]
+ T-129a: Webhook signing, replay protection, and event-delivery idempotency — 1 week [T-128]
+ T-129b: SBOM, dependency scanning, signed releases, and security advisory process — 1 week [T-002]

- T-130: API gateway + auth + rate limiting — 2 weeks
+ T-130: API gateway + auth + rate limiting — 2 weeks [T-128]
```

## 13. Add observability and SLO engineering before SLA claims

**Analysis**

“SLA 99.9%” cannot depend on a status page task in Q4. Gateway integrations fail in country-specific ways. You need trace IDs, per-gateway latency/error metrics, dead-letter queues, replay tooling, customer-visible incident IDs, and reconciliation-first support.

```diff
+ ### 4.12 Operations and observability
+
+ Every transmission has:
+ - trace_id
+ - tenant_id
+ - idempotency_key
+ - gateway_attempt_id
+ - normalized state transition
+ - raw gateway receipt hash
+ - retry/dead-letter metadata
+
+ Managed API SLOs are defined per operation: validate, render, transmit enqueue,
+ gateway accepted, archive write, webhook delivery. Gateway legal acceptance is
+ never conflated with API availability.
```

```diff
+ T-134a: OpenTelemetry tracing, metrics, log redaction, and per-gateway dashboards — 2 weeks [T-059a]
+ T-134b: Replay/admin tooling for stuck transmissions and dead-letter queues — 1 week [T-134a]
```

## 14. Separate pure builder from enrichment/network calls

**Analysis**

This API shape is risky:

```ts
supplier: { vat: 'DE123456789' } // Rest auto-resolved via VIES
```

A builder should be deterministic and offline. Network enrichment should be explicit, cancellable, cached, and privacy-aware. Otherwise `build()` becomes slow, flaky, and surprising.

```diff
- const invoice = Invoice.builder({
-   supplier: { vat: 'DE123456789' },  // Rest auto-resolved via VIES
+ const draft = createInvoiceDraft({
+   supplier: { vat: 'DE123456789' },
    customer: { vat: 'FR987654321' },
    currency: 'EUR',
- })
+ });
+
+ const enriched = await client.enrich(draft, {
+   sources: ['vies'],
+   cache: 'tenant',
+   consent: true,
+ });
+
+ const invoice = Invoice.builder(enriched)
```

Also split packages:

```diff
- import { Invoice, validate, render, transmit, verify } from '@invoicekit/core';
+ import { createInvoiceDraft, validateLocal } from '@invoicekit/core';
+ import { renderPdf } from '@invoicekit/render';
+ import { InvoiceKitClient } from '@invoicekit/managed';
```

## 15. Fix DX commands and first-touch flow

**Analysis**

The sample says `npx` and then `pnpm exec`. That is inconsistent. Since this repo mandates Bun, the internal plan should use Bun. Public docs can still list npm/pnpm/yarn equivalents later, but the default should be one clean path.

Also add `doctor` before `init` does anything expensive.

```diff
- $ npx invoicekit init
+ $ bunx invoicekit init

- Try it: pnpm exec invoicekit send examples/first-invoice.ts
+ Try it: bunx invoicekit validate examples/first-invoice.ts --profile=peppol-bis
+ Then:   bunx invoicekit send examples/first-invoice.ts --mode=sandbox
```

```diff
+ invoicekit doctor --country=DE --profile=xrechnung
+ # Checks local WASM/native engine, reference validator availability,
+ # rulepack freshness, PDF/A verifier, API key scopes, and country capabilities.
```

## 16. Make the public validator server-assisted, not browser-only

**Analysis**

A browser-only validator cannot run the JVM reference stack. If the SEO/public validator claims official-grade XRechnung/Peppol validation but only runs a pure-WASM subset, that will hurt trust.

Use two modes:

- Local/private browser validation for pure Rust rules.
- Server-side reference validation for official parity, with no-retention and optional client-side redaction.

```diff
- Public free validator web UI (WASM-only, browser, no upload)

+ Public validator with two explicit modes:
+ - Local mode: browser-only WASM, privacy-first, limited to pure Rust rulepacks.
+ - Reference mode: server-side KoSIT/phive/Saxon worker, no-retention by default,
+   returns official-parity diagnostics and rulepack provenance.
```

## 17. Add corpus licensing and real-fixture policy

**Analysis**

A public invoice corpus is a moat, but it is also a licensing/privacy trap. Do not default to CC-BY-SA for everything. Use synthetic fixtures under Apache-2.0/CC0, and only include real-world examples when rights, redaction, and redistribution are explicit.

```diff
- conformance-corpus/   # 500+ adversarial test invoices; public CC-BY-SA

+ conformance-corpus/
+   synthetic/          # CC0/Apache-2.0 generated fixtures
+   licensed-real/      # explicitly licensed, redacted real invoices
+   private-regression/ # non-public customer/support fixtures
+   generators/         # adversarial fixture generators
```

```diff
+ T-120a: Corpus licensing/redaction policy and fixture metadata schema — 3 days [T-002]
```

## 18. Add explicit performance budgets by package

**Analysis**

“WASM cold-start p99 <50ms” is not meaningful if the package includes PDF rendering, rulepacks, and OCR. Split performance budgets:

- core parse/canonicalize/validate-lite,
- reference validation,
- render,
- bundle verify,
- transmission enqueue,
- OCR.

```diff
- WASM cold-start p99 | <50ms in Cloudflare Worker

+ Performance budgets:
+ - `@invoicekit/core` WASM cold start p99 <50ms on Cloudflare Workers
+ - local EN 16931 core validation p95 <25ms for 100-line invoice
+ - UBL parse + canonicalize p95 <50ms for 1 MB XML
+ - PDF/A-3 render p95 <2s for 10-page invoice on managed worker
+ - reference validator p95 measured separately by profile/rulepack
+ - OCR/VLM excluded from core-runtime budgets
```

## 19. Add source-watch automation for mandates and rule changes

**Analysis**

The plan relies on fast-changing laws, codelists, and profiles. Make the update process a product capability.

```diff
+ T-006: Compliance source-watch bot: monitor official sources, produce
+ rulepack/codelist update PRs, and open beads for changed mandates — 1 week [T-001]
+
+ T-006a: `invoicekit capabilities` data model generated from source-watch
+ manifests, with confidence level and last-verified date — 1 week [T-006]
```

## 20. Fix binding dependencies

**Analysis**

The SDK tasks should not depend on the TypeScript SDK. They should depend on the stable engine ABI and golden fixtures. Otherwise TS becomes an accidental source of truth.

```diff
- T-111: Python SDK (pyo3) — 2 weeks [T-110]
- T-112: Go SDK (wazero) — 2 weeks [T-110]
- T-113: Java SDK (wasmtime-java) — 2 weeks [T-110]
- T-114: .NET SDK (Wasmtime.NET) — 2 weeks [T-110]

+ T-109: Stable engine ABI contract + cross-language golden fixtures — 2 weeks [T-010, T-022]
+ T-110: TypeScript SDK (`@invoicekit/core`, `@invoicekit/render`, `@invoicekit/managed`) — 2 weeks [T-109]
+ T-111: Python SDK (pyo3/maturin) — 2 weeks [T-109]
+ T-112: Go SDK (cgo + REST sidecar fallback) — 2 weeks [T-109]
+ T-113: Java SDK (JNI/FFM over C ABI + REST sidecar fallback) — 2 weeks [T-109]
+ T-114: .NET SDK (P/Invoke over C ABI + REST sidecar fallback) — 2 weeks [T-109]
```

**Net recommendation**

The best revised plan is not “smaller” in ambition, but it should be stricter about maturity labels and dependencies. Make the deterministic core, rulepack supply chain, evidence format, outbox/state machine, and managed operational substrate the Year-1 spine. Treat native AS4, OCR/VLM, global country coverage, and all-runtime WASM as expansion tracks gated by conformance evidence.

Sources used: [Germany BMF e-invoicing FAQ](https://www.bundesfinanzministerium.de/Content/EN/FAQ/e-invoicing.html), [France impots.gouv reform overview](https://www.impots.gouv.fr/invoicing-and-e-reporting), [France official formats](https://www.impots.gouv.fr/specifications-externes-b2b), [Poland KSeF dates](https://www.podatki.gov.pl/ksef/), [Belgium FPS Finance e-invoicing](https://finance.belgium.be/en/e-invoicing), [Spain AEAT Verifactu FAQ](https://sede.agenciatributaria.gob.es/Sede/en_gb/iva/sistemas-informaticos-facturacion-verifactu/preguntas-frecuentes.html), [UAE MoF eInvoicing](https://mof.gov.ae/einvoicing/), [ZATCA e-invoicing waves](https://zatca.gov.sa/en/E-Invoicing/Introduction/Guidelines/Pages/default.aspx), [Typst PDF docs](https://typst.app/docs/reference/model/document/), [wasmtime-java](https://github.com/bytecodealliance/wasmtime-java), [phase4](https://github.com/phax/phase4), [OpenPeppol BIS Billing 3.0](https://docs.peppol.eu/poacc/billing/3.0/), [GOBL](https://github.com/invopop/gobl).
