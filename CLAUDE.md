# CLAUDE.md — InvoiceKit

This file is Claude-specific guidance. The full project rules live in `AGENTS.md`; read that first.

## Where to look first

- `AGENTS.md` — safety rules, project layout, architectural commitments, tooling.
- `README.md` — public-facing pitch and project status.
- `plans/PLAN.md` — implementation plan (v0.1).
- `plans/PLAN_v0.2_revisions.md` — applied review revisions on top of v0.1.
- `research/MASTER_REPORT.md` — one-page synthesis of all market research and ideation.
- `research/` — every research file, idea-generation phase, and adversarial critique.

## The chosen direction

**Direction A — "the trust toolkit."** Open reference for invoice format correctness; free core; signed evidence bundles; public conformance corpus. Decided May 2026 after multi-model review. Do not casually pitch the alternative ("invoice hub" / Stripe-shape managed API) — it stays available architecturally but is not the current narrative.

## Working context

This is a solo developer plus AI agents working in one concentrated push. There is no funded phasing, no team to coordinate, no design-partner pilot, no kill-test calendar. When planning work:

- Skip anything that assumes humans other than the principal.
- Skip 60-day pilot programs, hiring milestones, investor checkpoints.
- Build the thing.

## Communication style the principal prefers

- Plain English. No three-letter abbreviations without spelling them out the first time.
- Short sentences when explaining.
- Direct, opinionated answers. Do not list options when you have one good recommendation; give the recommendation and the reason.
- If a question is exploratory, two or three sentences with a recommendation and the main tradeoff. Save deep dives for when asked.
- No emoji unless the principal uses one first.

## Architectural commitments (settled — do not casually overturn)

See `AGENTS.md` for the full list. The most-load-bearing ones:

1. Rust core; native bindings for server runtimes, WebAssembly for browser/edge — not "WebAssembly everywhere."
2. Layered invoice model. EN 16931 is the Year-1 European anchor, not the universal root.
3. Money/tax/code lists as first-class crates. No floats.
4. Reference validators run as an isolated JVM worker service called over JSON-RPC. We do NOT embed Java in WebAssembly.
5. Native AS4 is a research track. Year 1 live Peppol delivery uses a partner access point plus `phase4` as a reference adapter.
6. Interop with `invopop/gobl`. We do not reinvent its schema.
7. Apache 2.0 license everywhere. Not AGPL, not SSPL.

## Things to NOT propose

- Adding a tax engine.
- Building an end-user invoicing app (we are infrastructure).
- Switching the license off Apache 2.0.
- Spawning a heavy NTM agent swarm for routine work. Use it only when a specific skill explicitly requires it.
- Bulk-modifying code with codemods / `sed` / regex refactors.
- Deleting files without explicit permission in the current session.

## When the principal asks "what does this do" or "how would this work"

Lead with the plain-English explanation. Save jargon and code blocks for when explicitly asked. The principal is technical but is using these conversations to think about product/strategy, not always to read code.

## Country coverage order

Global. The order follows the dependency graph (see `plans/PLAN.md` Sections 3 and 6):

1. **Foundation** — engine, money, codelists, tax-calculation, layered invoice model, canonical serialization, validator worker, state machine, evidence bundles.
2. **Format family A** — Universal Business Language, Cross Industry Invoice, EN 16931, Peppol BIS, Peppol PINT, Factur-X, XRechnung. Unlocks ~35 countries automatically.
3. **Peppol live delivery** — partner access point integration, ~30 destination countries through one integration.
4. **National report crates** in waves — Italy, France, Poland, Spain, Greece, Belgium, Saudi Arabia (Wave 1); India, Mexico, Brazil, Malaysia, Turkey, Romania, Hungary, Japan (Wave 2); the rest of Latin America, Asia-Pacific, MENA, Africa (Wave 3).

When implementing a country crate, do not "skip ahead" without finishing the foundation tasks the crate depends on. Check `br ready --json` for unblocked work.

## Memory

Persistent context lives in `~/.claude/projects/-home-durakovic-projects-invoices/memory/`. Notable files:

- `project_invoices.md` — project mission and architectural bets.
- `feedback_strategic_forks.md` — the seven fork decisions and their resolutions.
- `feedback_communication_style.md` — how the principal prefers to be talked to.
- `reference_competitors.md` — competitor landscape as of May 2026.
- `user_context.md` — principal's working style.

Update these when something durable changes.
