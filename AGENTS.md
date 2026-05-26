# AGENTS.md — InvoiceKit

This file tells coding agents what they need to know to work on this project safely.

---

## RULE 1 — ABSOLUTE (DO NOT EVER VIOLATE)

You may NOT delete any file or directory unless I explicitly give the exact command **in this session**.

- This includes files you just created (tests, tmp files, scripts, etc.).
- You do not get to decide that something is "safe" to remove.
- If you think something should be removed, stop and ask. You must receive clear written approval **before** any deletion command is even proposed.

Treat "never delete files without permission" as a hard invariant.

---

## IRREVERSIBLE GIT & FILESYSTEM ACTIONS

Absolutely forbidden unless I give the **exact command and explicit approval** in the same message:

- `git reset --hard`
- `git clean -fd`
- `rm -rf`
- Any command that can delete or overwrite code/data

Rules:

1. If you are not 100% sure what a command will delete, do not propose or run it. Ask first.
2. Prefer safe tools: `git status`, `git diff`, `git stash`, copying to backups, etc.
3. After approval, restate the command verbatim, list what it will affect, and wait for confirmation.
4. When a destructive command is run, record in your response: the exact user text authorizing it, the command run, when you ran it. If that audit trail is missing, then you must act as if the operation never happened.

---

## What this project is, in one paragraph

InvoiceKit is an open-source toolkit for the full B2B invoicing lifecycle (create → check → render → read → send → archive). The core is written in Rust, delivered as native bindings for server runtimes (Node via napi-rs, Python via pyo3, Java via JNI/FFM, .NET via P/Invoke, Go via cgo) **and** as WebAssembly for browser and edge runtimes. Same engine, two delivery shapes. Apache 2.0 forever. Country coverage is global: ~35 countries reached through Peppol BIS / Peppol PINT / EN 16931 / UBL / Factur-X format families with one engine, plus ~25 additional countries with dedicated national clearance / reporting crates (Germany XRechnung, Italy SDI, Poland KSeF, Spain VeriFactu, France Chorus Pro / PA-PDP, Saudi Arabia ZATCA, India IRP, Mexico CFDI, Brazil NF-e, Malaysia MyInvois, Greece myDATA, Turkey e-Fatura, and the rest of Latin America, Asia-Pacific, MENA, and Africa). The full list lives in `plans/PLAN.md` Section 3.

## The chosen direction (decided May 2026)

**Direction A — "the trust toolkit."** We become the open reference for invoice format correctness. Free for the core, signed evidence bundles for every operation, public conformance corpus, validator that ranks for "validate XRechnung" on Google. Revenue follows trust slowly, from hosted convenience layers (sending, archive, certificates).

We do NOT pursue Direction B ("invoice hub" / Stripe-shape managed API) in the first build push, though the architecture leaves the door open for it later.

## What this project explicitly does NOT do

- No tax engine (no VAT computation logic beyond what an invoice itself requires).
- No accounting ledger or ERP.
- No payment processing.
- No CRM.
- No closed-source SaaS portal as the primary product.
- No "AI-first invoicing" branding. AI is used for reading incoming PDFs; outbound generation is always deterministic.
- No AGPL or SSPL anywhere in the stack.
- No competition with end-user invoicing apps (Invoice Ninja, Crater). We are infrastructure for the developers who build those.

## Architectural commitments

These were settled after multi-model review. Do not casually overturn them; if you think one is wrong, surface the argument explicitly.

1. **Rust core, dual delivery (native bindings + WebAssembly).** The compatibility contract is the engine API, the invoice data model, the rule packs, and the test fixtures — not WebAssembly itself.
2. **Layered invoice model.** Global commercial document at the root → profile views (EN 16931, Peppol BIS, XRechnung, Factur-X, FatturaPA, KSeF, ZATCA, etc.) → typed jurisdiction extensions. EN 16931 is the Year-1 European anchor, not the universal root for every country.
3. **Money, tax, and code lists are first-class.** Never use floating-point arithmetic for monetary values. All amounts use fixed-scale decimal strings at API boundaries. Code lists (ISO 3166, ISO 4217, VAT category, Peppol, etc.) are signed, versioned, effective-dated rule pack data.
4. **Rule packs are signed and versioned with effective dates.** No silent rule drift. `invoicekit validate --date=YYYY-MM-DD` selects rule packs by effective date.
5. **State machine and outbox come before any gateway integration.** Every transmission carries a trace ID, tenant ID, idempotency key, and gateway attempt ID. Each country gateway is a `GatewayAdapter` trait implementation.
6. **Reference validators run as an isolated JVM worker service** (KoSIT, phive, Saxon, Peppol Schematron) called over JSON-RPC. We do NOT embed Java in WebAssembly.
7. **Native AS4 (the Peppol transport protocol) is a research track, not a Year-1 feature.** Year 1 live delivery uses a partner Peppol access point plus `phase4` as a reference adapter.
8. **PDF rendering is deterministic and byte-stable.** Underlying renderer is Typst; we ship a TypeScript template language on top so users never see Typst. Use veraPDF as the reference verifier; do NOT reimplement it.
9. **We interoperate with `invopop/gobl`'s JSON schema.** We do not reinvent it.
10. **Every operation produces a signed evidence bundle** (`.invoicekit`): canonical invoice JSON, generated XML, PDF, validation trace, rule-pack manifest, signatures, gateway receipts, RFC 3161 timestamp. Verification never executes shell scripts.
11. **Country coverage is honest.** A "supported" country has explicit maturity labels per capability (serialize, validate, render, sandbox, partner-live, inbound, archive, correction, SLA). No blanket "supported" claims.

## What makes us special (the five-point pitch)

1. Runs on whatever stack the developer already uses.
2. Free and Apache-2.0 at the core; no signup, no per-envelope fee for the library.
3. Reads invoices in as well as writes them out, with bounding-box-cited extraction.
4. One install covers many countries and many formats.
5. Every step leaves verifiable proof that holds up in audit.

## Working style (solo + AI agents, single push)

This project is built by one person plus AI agents in a single concentrated effort. There are no separate funded phases, no design-partner pilots, no team to coordinate, no investor calendar.

**Implications for agents:**

- Do not write plans that assume "Q1 hiring," "design partner LOIs," "kill tests," or any human-org-shaped milestones. Build it.
- Do not propose 60-day experiments. We commit to architectural choices based on the research, and adjust mid-build only when something concrete breaks.
- Trade off speed against future flexibility in favor of speed within the architectural commitments above.
- When in doubt about scope, prefer "ship it correctly" over "ship a placeholder."

## Project layout (target)

```
invoices/
├── README.md
├── AGENTS.md
├── CLAUDE.md
├── crates/
│   ├── invoicekit-engine/        # Pure deterministic Rust API; the source of truth
│   ├── invoicekit-ffi/           # Stable C ABI
│   ├── invoicekit-wasm/          # Browser/edge WebAssembly artifact
│   ├── money/                    # rust_decimal-based money type
│   ├── codelists/                # Signed, versioned code list registry
│   ├── tax-calculation/          # Deterministic invoice arithmetic
│   ├── rulepack/                 # Signed, effective-dated rule packs
│   ├── ir/                       # Layered invoice model
│   ├── canonical/                # Deterministic XML/JSON serialization
│   ├── validate/                 # Rule registry + reference-worker client
│   ├── render-pdf/               # Typst-based PDF/A-3 + Factur-X embedding
│   ├── intake-pdf/               # Digital PDF parsing + Factur-X XML extraction
│   ├── intake-ocr/               # PaddleOCR + small VLM intake (server-side default)
│   ├── transmit-peppol/          # AS4 envelope exchange (partner-AP + phase4 reference)
│   ├── transmit-mock/            # Sandbox mock gateway
│   ├── report-fr-ctc/            # France PA/PDP e-invoicing + e-reporting flows
│   ├── report-es-verifactu/      # Spain anti-fraud reporting
│   ├── report-gr-mydata/         # Greece myDATA
│   ├── report-in-gst/            # India GST IRP / e-waybill
│   ├── report-pl-ksef/           # Poland KSeF clearance/submission
│   ├── report-it-sdi/            # Italy SDI clearance + receipts
│   ├── report-sa-zatca/          # Saudi ZATCA Phase 2 clearance
│   ├── reconcile/                # Fingerprint, state machine, outbox, idempotency
│   ├── evidence/                 # .invoicekit signed bundle format
│   ├── archive/                  # Pluggable storage (S3 Object Lock / Azure WORM / local FS)
│   ├── verify/                   # Bundle verification library + CLI
│   ├── lsp/                      # Invoice language server
│   └── cli/                      # `invoicekit` binary
├── bindings/
│   ├── node-napi/                # napi-rs (Node native)
│   ├── python/                   # pyo3 + maturin
│   ├── dotnet/                   # P/Invoke over C ABI
│   ├── java/                     # JNI / Java FFM over C ABI
│   ├── go/                       # cgo + REST sidecar fallback
│   ├── wasm-browser/             # wasm-bindgen for browser / Cloudflare Workers
│   └── rest-shim/                # Axum HTTP gateway for conservative customers
├── services/
│   └── validator-worker-jvm/     # KoSIT / phive / Saxon JVM service (JSON-RPC)
├── conformance-corpus/
│   ├── synthetic/                # CC0 / Apache-2.0 generated fixtures
│   ├── licensed-real/            # Explicitly licensed, redacted real invoices
│   ├── private-regression/       # Non-public customer/support fixtures
│   └── generators/               # Adversarial fixture generators
├── plans/
│   ├── PLAN.md                   # Implementation plan (v0.1)
│   └── PLAN_v0.2_revisions.md    # Applied revisions from review round 1
├── research/                     # Market research, idea pool, adversarial critiques
└── docs/                         # Documentation site (Nextra)
```

## Generated files — never edit manually

The TypeScript types for the invoice model are generated from the Rust source of truth. Edit the Rust types; the TypeScript types regenerate.

## Code editing discipline

- Do **not** run scripts that bulk-modify code (codemods, invented one-off scripts, giant `sed`/regex refactors).
- Large mechanical changes: break into smaller, explicit edits and review diffs.
- Subtle/complex changes: edit by hand, file-by-file, with careful reasoning.

## Node / JavaScript toolchain

- Use **bun** for everything JavaScript/TypeScript.
- Never use `npm`, `yarn`, or `pnpm` in our own development scripts. (Public docs may list `npx` for first-touch since most external developers use Node + npm.)
- Lockfiles: only `bun.lock`. Do not introduce any other lockfile.
- Target the latest Node.js.

## Backwards compatibility & file sprawl

We optimize for a clean architecture now, not backwards compatibility.

- No "compat shims" or "v2" file clones.
- When changing behavior, migrate callers and remove old code.
- New files are only for genuinely new domains that don't fit existing modules.
- The bar for adding files is very high.

## Console output

- Prefer structured, minimal logs.
- Treat user-facing UX as UI-first; logs are for operators and debugging.

## Issue tracking with `br` (Beads)

All issue tracking goes through Beads. No other to-do systems.

- `.beads/` is authoritative state and **must always be committed** with code changes.
- Do not edit `.beads/*.jsonl` directly; only via `br`.

```bash
br ready --json                                    # Find unblocked work
br create "Title" -t bug|feature|task -p 0-4 --json
br update br-42 --status in_progress --json
br close br-42 --reason "Completed" --json
```

Types: bug, feature, task, epic, chore. Priorities: 0 critical, 1 high, 2 medium, 3 low, 4 backlog.

## Using `bv` as an AI sidecar

`bv` is a graph-aware triage engine for Beads projects. **Use only `--robot-*` flags.** Bare `bv` launches an interactive TUI that blocks your session.

```bash
bv --robot-triage        # The mega-command: start here
bv --robot-next          # Just the single top pick + claim command
bv --robot-plan          # Parallel execution tracks
bv --robot-insights      # Full graph metrics
```

## `cass` — cross-agent search

Never run bare `cass` (TUI). Always use `--robot` or `--json`.

```bash
cass health
cass search "authentication error" --robot --limit 5
```

## Memory system: `cm` (cass-memory)

Before starting non-trivial tasks:

```bash
cm context "<task description>" --json
```

Returns relevant prior rules, anti-patterns, and history snippets.

## UBS quick reference

Run `ubs <changed-files>` before every commit. Exit 0 = safe. Exit > 0 = fix and re-run.

```bash
ubs file.ts file2.py                    # Specific files (< 1s)
ubs $(git diff --name-only --cached)    # Staged files — before commit
```

## Landing the plane (session completion)

When ending a work session you must complete all steps below. Work is NOT complete until `git push` succeeds.

1. File issues for remaining work.
2. Run quality gates if code changed.
3. Update issue status.
4. **Push to remote:**
   ```bash
   git pull --rebase
   br sync --flush-only
   git add .beads/
   git commit -m "Update beads"
   git push
   git status   # MUST show "up to date with origin"
   ```
5. Clean up stashes, prune remote branches.
6. Verify all changes are committed AND pushed.
7. Hand off context for the next session.

If push fails, resolve and retry until it succeeds. Never stop before pushing.

## Multi-agent coordination (Agent Mail)

Agent Mail is available as a Model Context Protocol server.

- Register identity via `ensure_project` then `register_agent` with the absolute repo path as `project_key`.
- Reserve files before editing: `file_reservation_paths(project_key, agent_name, ["src/**"], ttl_seconds=3600, exclusive=true)`.
- Communicate with `send_message(..., thread_id="FEAT-123")`; read with `fetch_inbox`.
- Prefer macros (`macro_start_session`, `macro_prepare_thread`, `macro_file_reservation_cycle`) when speed matters more than fine control.
