Warning: True color (24-bit) support not detected. Using a terminal with true color enabled will result in a better visual experience.
Ripgrep is not available. Falling back to GrepTool.
Here is the aggressive, opinionated review of the v0.3 global plan. 

At a high level, the ambition is correct, but the execution model has hidden naive assumptions about how global compliance scales. You cannot run 60 countries through a single monolithic JVM worker, you cannot build 30 country crates in parallel with agents without establishing archetypes first, and maintaining 60 mock gateways manually is a maintenance death sentence.

Here is the breakdown of what is broken, followed by the explicit `git-diff` style changes to the plan.

### 1. Architectural decisions that won't survive global scale
**The `JurisdictionExtensions` God-Object.** In Section 4.3, the IR uses a single struct containing every country's extension (`de_xrechnung`, `sa_zatca`, etc.). This forces the core engine and the FFI boundary to recompile every time a single country adds a field.
*Fix:* Do not hardcode country extensions into the core schema. Use a polymorphic payload (e.g., `serde_json::Value` or a map of dynamic components) keyed by the country's URN.
**The WebAssembly Bloat.** Compiling 60 country rulesets and XML/PDF generators into a single `.wasm` artifact will result in a 30MB+ payload, making it unusable for Edge Workers (Cloudflare has limits) and slow for browsers. 
*Fix:* The WASM build must be feature-flagged. You don't ship "the world" in WASM; you ship a core parser + specific requested countries via `cargo build --features=fr,de`.

### 2. Hidden assumptions in the country crate structure
**Cryptographic Hardware.** The plan assumes we can just route XML to gateways. Countries like Poland (KSeF), Saudi Arabia (ZATCA), and Mexico (CFDI) require qualified electronic signatures using HSMs (Hardware Security Modules) or Smart Cards. A cloud SaaS layer cannot hold these for on-premise enterprise customers legally.
*Fix:* We must define a `LocalSignerProxy` trait and an open-source agent that runs in the customer's datacenter to hold their certificates, which the Rust engine calls during the state machine flow.

### 3. Tasks that are sequenced wrong
**Agent Parallelization (Track 8) without Archetypes.** Setting agents loose on 30 country crates in parallel will result in 30 different architectural interpretations of "clearance state machine."
*Fix:* You must sequence Track 8 to build **3 Archetypes** sequentially first: one Peppol (e.g., `be-pep`), one Async Clearance (e.g., `pl-ksef`), and one Cryptographic (e.g., `sa-zatca`). ONLY when these 3 are perfect do you unleash agents in parallel on the remaining 57.
**Intake Pipeline (Track 5) is too early.** AI extraction (weeks 6-14) is a massive distraction from shipping the core deterministic engine. 
*Fix:* Push AI extraction to Phase 6. Secure the outbound/transmission moat first.

### 4. Wrong-by-degree estimates
- **ZATCA (Saudi Arabia) in 3 weeks.** ZATCA requires specific ECDSA secp256k1 signing, custom UBL canonicalization, and QR code TLV generation. This is a 6-8 week effort minimum.
- **Typst PDF/A-3 (Track 4) in 1 week.** Typst does not natively support writing arbitrary PDF/A-3 XMP metadata and embedding ZUGFeRD XML flawlessly without upstream forks or heavy post-processing via `lopdf`. Allocate 4 weeks.

### 5. JVM Validator Worker at Global Scale
The plan puts KoSIT, phive, and Saxon in a single JVM container. When we add ZATCA's Java SDK, India's validation rules, and Mexican PAC validation, this monolith will suffer from class-path hell (log4j/bouncycastle conflicts) and massive memory bloat.
*Fix:* The validator worker must be a Sidecar pattern (`validator-jvm-kosit`, `validator-jvm-zatca`). Split them by dependency boundaries, not a single monolithic container.

### 6. Sandbox / Test Mode strategy for 60 countries
Maintaining 60 handwritten mock gateways (Section 4.11) is impossible. Tax authorities change APIs silently. 
*Fix:* Do not write manual mock gateways. Build a VCR-style declarative replay proxy. We record real interactions with the real government sandboxes once, commit the HTTP traces (`.har` or `.vcr` files) to the repo, and the mock gateway just replays them.

---

### Proposed Plan Revisions (`git-diff` style)

```diff
--- a/PLAN.md
+++ b/PLAN.md
@@ -107,17 +107,17 @@
 
 ### 2.6 Reference validators run as an isolated JVM worker service
 
-KoSIT (German XRechnung), phive (Helger's validation engine), Saxon (XSLT 2.0), and the Peppol Schematron rule packs all run inside a dedicated containerized Java service.
+KoSIT (German XRechnung), phive (Helger's validation engine), and Saxon (XSLT 2.0) run inside isolated, domain-specific JVM sidecars.
-We do not embed Java in WebAssembly. We do not reimplement these validators. They are the official references; we use them as the ground truth.
+We do not run a single monolithic JVM container, as adding ZATCA, IRP, and CFDI SDKs will cause class-path hell. Validators are split into dedicated micro-containers (e.g., `validator-kosit`, `validator-zatca`). We communicate over JSON-RPC.
 
@@ -176,14 +176,12 @@
 
 interface JurisdictionExtensions {
-  de_xrechnung?: { leitweg_id: string; /* ... */ };
-  de_zugferd?: { profile: 'MINIMUM' | 'BASIC_WL' | /* ... */ };
-  fr_chorus_pro?: { service_code?: string; /* ... */ };
-  // ... one per country
+  urn: string;                              // e.g., "urn:invoicekit:ext:sa:zatca:2.0"
+  payload: Record<string, any>;             // Dynamic payload to prevent core engine recompilation
 }
```

```diff
@@ -242,8 +242,12 @@
-Track 8 — National report crates. Each national crate is roughly one to three weeks of work depending on complexity. Agents work in parallel — one crate per agent at a time.
+Track 8 — National report crates.
+Do NOT parallelize immediately.
+Phase 8a (Sequential Archetypes): Build 3 crates sequentially to establish traits: 
+  1. `report-pl-ksef` (Async Clearance archetype)
+  2. `report-sa-zatca` (Cryptographic archetype)
+  3. `report-be-pep` (Peppol archetype).
+Phase 8b (Agent Swarm): Unleash agents in parallel on the remaining 27+ crates using the archetypes as rigorous templates.
 
@@ -353,10 +357,11 @@
 ### 4.11 Test mode — Stripe-style sandbox
 
-Every transmission path has a mock counterpart. Mock gateways simulate documented failure modes of real gateways.
+Manual mock gateways do not scale to 60 countries. We use a VCR-style declarative replay proxy.
+We record real HTTP/AS4 traces against government sandboxes (success, failures, timeouts) and commit the anonymized `.vcr` trace files to the repo. 
+`transmit-mock` simply matches requests against the VCR tape and replays the exact headers and bytes the real gateway returned.
```

```diff
@@ -481,7 +481,7 @@
 | T-050 | Typst integration as Rust crate dependency | 1 week | T-010 |
 | T-051 | TypeScript template language compiles to Typst | 3 weeks | T-050 |
 | T-052 | veraPDF adapter for conformance verification | 1 week | T-050 |
-| T-053 | Renderer fallback decision gate (prove Typst meets Factur-X requirements or keep secondary renderer behind `RenderBackend` trait) | 1 week | T-052 |
+| T-053 | Typst PDF/A-3 Dictionary Hacking (Upstream PRs or lopdf post-processing to ensure valid XMP and ZUGFeRD embedding) | 4 weeks | T-052 |
 
 ### Track 5 — Intake pipeline
+**Moved to Phase 6 (Weeks 24+).** Core deterministic engine and outbound transmission must be secured before burning cycles on AI/OCR extraction.
```

```diff
@@ -512,6 +512,7 @@
 | T-081 | Pluggable archive backend | 2 weeks | T-080 |
 | T-082 | RFC 3161 timestamping integration | 1 week | T-080 |
 | T-083 | eIDAS qualified signature integration | 2 weeks | T-080 |
+| T-083a| `invoicekit-signer-agent` (On-premise proxy for Enterprise HSM / Smartcard signing without exposing keys to SaaS) | 2 weeks | T-083 |
```

Do you agree with these architectural corrections and sequence adjustments? Once confirmed, I will integrate them into a final `PLAN_v0.4.md` and exit plan mode.
