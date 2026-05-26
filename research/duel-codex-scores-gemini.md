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
session id: 019e62cd-cdc0-7180-b6ad-98fe16268099
--------
user
I asked another AI model (Gemini 3 Pro) for radical innovative ideas for our open-source e-invoicing toolkit. Score each of their 25 ideas 0-1000 on:
- technical quality / smart
- usefulness in real-world scenarios
- practicality to implement correctly
- whether the value justifies the complexity

Be candid and critical — don't just agree. We need honest signal. Where they're wrong, say so. Where they're better than what you proposed, admit it.

Then identify:
- The top 3 ideas across BOTH lists (yours and theirs combined) that we should absolutely build
- The bottom 3 across both lists that we should kill
- One blind spot neither of us caught

Background for context:
- We're building a WASM-native, OSS, developer-first e-invoicing toolkit (EN 16931 + Factur-X + XRechnung + Peppol)
- Target ICP: embedded developer at ERP/billing SaaS vendor
- OSS core MIT/Apache; paid managed services
- We can interop with invopop/gobl JSON schema

# Gemini's 25 ideas (followed by their critique):

```
1. Deterministic Typst-based PDF/A-3 Generation: pure Rust embedding of typst to generate PDFs directly from canonical IR. Edge-compatible WASM, embeds Factur-X XML.
2. Schema-Compiled Validation (AOT): macro/compiler takes country Schematron rules, compiles to raw Rust branching logic, then WASM. O(N) DOM parsing becomes O(1).
3. BYO-LLM via WebGPU: developers run local quantized models (Llama-3 8B etc) in browser/Node via WebGPU/WASM. Zero data exfiltration.
4. Git-like Semantic Diffing for Amendments: structural semantic diff API (diff.tax_rate: 19% -> 7%) rather than raw XML diff.
5. eBPF/BPF Network Sniffer for Legacy Ingestion: eBPF agent sniffs DB traffic (Postgres) or legacy HTTP from ancient ERPs, generates e-invoices out-of-band.
6. Time-Travel Validation Debugging: validate an invoice as if it were a specific date in past or future. Version-pinned ASTs of validation rules.
7. GraphQL/JSON-RPC Stream for Dirty Ingestion: ingest unschema'd JSON; return structured stream of specific localized errors with JSON Paths.
8. SQLite VFS for Invoice Envelopes: custom SQLite VFS in Rust handling at-rest encryption, structural dedup of PDFs, FTS on canonical IR.
9. Content-Addressed Attachments (CID/IPFS-style): hash + store by content address. Same contract attached to 50 invoices = dedup.
10. Chaos Engineering API ("Gov-Simulator"): test environment simulating worst gateway behaviors — random 504 timeouts SDI, malformed XML rejections, KSeF peak-hour latency.
11. WASM-based Custom CIUS Plugins: community writes local tax rule extensions in AssemblyScript/Rust, compiled to WASM, hot-reload.
12. Reverse-Proxy Sidecar: InvoiceKit as Rust sidecar (Envoy-like). Legacy app sends standard HTTP JSON; sidecar translates, signs, sends Peppol, returns synthetic ACK.
13. Zero-Knowledge Proofs (ZKP) for Factoring: ZK-SNARKs prove invoice totals + valid signature without revealing line items.
14. VS Code / Cursor Language Server (LSP): real-time squigglies for business logic ("Total tax does not match line items").
15. "Invoice as Code" CLI: Terraform-like `invoicekit apply -f invoice.yaml`. State reconciliation, polls async gov ACK.
16. Built-in PII/GDPR Redactor: native function strips/cryptographically masks PII from IR for safe staging dumps.
17. Fuzzing as a Service (FaaS) for Buyers: AP test harness generates thousands of structurally valid but semantically weird invoices to stress-test ERP ingestion.
18. Homomorphic Encryption for Aggregation: tax accountants sum total VAT across encrypted invoices without decrypting line items.
19. SSE/WebSockets over Webhooks: outbound SSE connection from client to managed service for real-time gov ACKs behind NAT/firewall.
20. Visual Bounding-Box TUI: keyboard-driven terminal UI for ops teams to correct OCR bounding boxes without heavy web app.
21. Public "SSL Labs" for Invoices: free un-gated website, drop XML/JSON, get A-F grade on compliance with EU/Global mandates.
22. Smart Contract Escrow Bridge: out-of-box adapter listens to invoice state changes ("Accepted by Buyer") and triggers EVM/Solana smart contract.
23. Any-to-Any Mapping DSL: tiny DSL for mapping proprietary CSV/JSON exports into canonical IR; compile DSL to WASM.
24. Cryptographic Payload Tamper-Evident Logs: Merkle tree logs state transitions; immutable audit trail API.
25. "Fallback to Paper" API: partner with print-and-mail API (Lob); if recipient not on Peppol, transparently route to physical mail.
```

Their critique highlighted: (a) WASM is leaky abstraction for JVM/.NET enterprise, (b) EN 16931 canonical IR is politically fraught — tax authorities bastardize standards, (c) margin squeeze against free gov gateways, (d) Peppol AP SLA imposes heavy fixed costs.

They argued F7 should be hosted-API priority (cryptography, async state, network hell). They named the missing piece: "Reconciliation Engine" with deterministic invoice hash and reconciliation API.

Be brutal. Score each 1-25. Identify top 3, bottom 3, blind spot.
codex
I’ll treat this as a strategy review, not an implementation task. I’m going to use the local idea workflow plus the repo’s required context lookup so the scoring is grounded in the project’s own conventions before I answer.
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc "sed -n '1,220p' /home/durakovic/.codex/skills/idea-wizard/SKILL.md" in /home/durakovic/projects/invoices
 succeeded in 0ms:
---
name: idea-wizard
description: >-
  Generate and operationalize improvement ideas for projects. Use when brainstorming
  features, planning improvements, creating beads from ideas, or "what should we build next".
---

# idea-wizard

> Generate many → winnow ruthlessly → operationalize into self-documenting beads.

## Quick Start

```
1. Read AGENTS.md + all beads       → Ground in reality
2. Phase 2 prompt (30→5)            → Generate & winnow
3. Phase 3 prompt (next 10)         → Expand to 15
4. Check overlaps with open beads   → Merge, don't duplicate
5. Phase 5 prompt                   → Create beads
6. Phase 6 prompt (repeat 4-5x)     → Refine in "plan space"
```

---

## THE EXACT PROMPTS

**Phase 2 — Generate 30→5:**
```
Come up with your very best ideas for improving this project to make it more robust, reliable, performant, intuitive, user-friendly, ergonomic, useful, compelling, etc. while still being obviously accretive and pragmatic. Come up with 30 ideas and then really think through each idea carefully, how it would work, how users are likely to perceive it, how we would implement it, etc; then winnow that list down to your VERY best 5 ideas. Explain each of the 5 ideas in order from best to worst and give your full, detailed rationale and justification for how and why it would make the project obviously better and why you're confident of that assessment.
```

**Phase 3 — Expand:** `ok and your next best 10 and why`

**Phase 5 — Create Beads:**
```
OK so please take ALL of that and elaborate on it and use it to create a comprehensive and granular set of beads for all this with tasks, subtasks, and dependency structure overlaid, with detailed comments so that the whole thing is totally self-contained and self-documenting (including relevant background, reasoning/justification, considerations, etc.-- anything we'd want our "future self" to know about the goals and intentions and thought process and how it serves the overarching goals of the project.). The beads should be so detailed that we never need to consult back to the original markdown plan document. Remember to ONLY use the `br` tool to create and modify the beads and add the dependencies.
```

**Phase 6 — Refine (repeat 4-5x):**
```
Reread AGENTS.md so it's still fresh in your mind. Check over each bead super carefully-- are you sure it makes sense? Is it optimal? Could we change anything to make the system work better for users? If so, revise the beads. It's a lot easier and faster to operate in "plan space" before we start implementing these things! DO NOT OVERSIMPLIFY THINGS! DO NOT LOSE ANY FEATURES OR FUNCTIONALITY! Also make sure that as part of the beads we include comprehensive unit tests and e2e test scripts with great, detailed logging so we can be sure that everything is working perfectly after implementation. Make sure to ONLY use the `br` cli tool for all changes, and you can and should also use the `bv` tool to help diagnose potential problems with the beads.
```

---

## Commands

```bash
br list --json && br list --status closed --json   # Phase 1: Research
br list --json | jq '.issues[]?.title'             # Phase 4: Check overlaps
br create "Title" -p 1 -t task --body "..."        # Phase 5: Create
br dep add <child> <parent>                        # Phase 5: Dependencies
bv --robot-insights | jq '.Cycles'                 # Validate: Must be empty!
```

---

## Anti-Patterns

| Don't | Do |
|-------|-----|
| Skip Phase 1 | Read beads first — prevents duplicates |
| Stop at 5 ideas | Expand to 15 — #6-15 often complementary |
| Single-pass beads | 4-5 passes — first draft never optimal |
| Omit tests | Explicit test tasks with logging |
| Bare `bv` | `--robot-*` flags — bare bv blocks TUI |
| Oversimplify | Resist — complexity exists for reasons |

**Phase 6 constraints:** DO NOT OVERSIMPLIFY. DO NOT LOSE FEATURES. After compaction → re-read AGENTS.md.

---

## References

| Topic | File |
|-------|------|
| Phase details | [PHASES.md](references/PHASES.md) |
| Bead patterns | [BEADS.md](references/BEADS.md) |
| Examples | [EXAMPLES.md](references/EXAMPLES.md) |
| Evaluation rubric | [RUBRIC.md](references/RUBRIC.md) |

Ideas evaluated on: robust, reliable, performant, intuitive, user-friendly, ergonomic, useful, compelling, accretive, pragmatic — see [RUBRIC.md](references/RUBRIC.md)

hook: PostToolUse
exec
/bin/zsh -lc 'rg -n "invoices|e-invoic|EN 16931|Factur-X|XRechnung|Peppol|invopop|gobl|Reconciliation Engine|invoicekit" /home/durakovic/.codex/memories/MEMORY.md' in /home/durakovic/projects/invoices
 exited 1 in 0ms:
hook: PostToolUse
hook: PostToolUse Completed
hook: PostToolUse Completed
exec
/bin/zsh -lc 'cm context "score Gemini e-invoicing toolkit ideas against developer-first WASM OSS roadmap" --json' in /home/durakovic/projects/invoices
hook: PostToolUse
hook: PostToolUse Completed
 succeeded in 309ms:
{
  "success": true,
  "command": "context",
  "timestamp": "2026-05-26T05:42:13.944Z",
  "data": {
    "task": "score Gemini e-invoicing toolkit ideas against developer-first WASM OSS roadmap",
    "relevantBullets": [
      {
        "id": "b-mmdc59dq-yf21qe",
        "scope": "global",
        "category": "architecture",
        "content": "Implement graceful degradation via a FallbackChain pattern (Full -> Partial -> DirectQuotes -> Refusal). Each pipeline layer can fail independently; the chain assembles the best possible response from whatever succeeded. This eliminates an entire class of 'what if X fails' bugs.",
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
        "createdAt": "2026-03-05T10:41:22.622Z",
        "updatedAt": "2026-03-05T10:41:22.622Z",
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
        "reasoning": "From unknown session on 3/5/2026"
      },
      {
        "id": "b-mmdc5c43-1152x9",
        "scope": "global",
        "category": "multi-agent",
        "content": "When dispatching parallel agents, complete all type/API renames (e.g. ClaudeClient->LlmClient, claim.quote->claim.quote_hint, Storage->StorageV2) in a single wave BEFORE other agents depend on the new names. Type renames cascade across crate boundaries and each one typically affects 5+ files.",
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
        "createdAt": "2026-03-05T10:41:26.163Z",
        "updatedAt": "2026-03-05T10:41:26.163Z",
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
        "reasoning": "From unknown session on 3/5/2026"
      },
      {
        "id": "b-mmdcfwmj-p3sdw8",
        "scope": "global",
        "category": "islamkb",
        "content": "Hadith dedup threshold 0.7 Jaccard on word bigrams is conservative — catches near-exact duplicates across collections while avoiding false positives from hadith that merely share common phrases. The canonical version is chosen as the longest text (most complete).",
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
        "createdAt": "2026-03-05T10:49:39.307Z",
        "updatedAt": "2026-03-05T10:49:39.307Z",
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
        "reasoning": "From unknown session on 3/5/2026"
      },
      {
        "id": "b-mmdwfzoj-cosz9w",
        "scope": "global",
        "category": "architecture",
        "content": "Corpus embeddings generated: 142,527 vectors (BGE-M3, 1024-dim) at data/embeddings/corpus.ikbe (560 MB). TEI runs natively at text-embeddings-inference/local-install/bin/text-embeddings-router on GPU 1 port 8089. BGE-M3 model converted to safetensors (lossless) at models/bge-m3/model.safetensors. TEI health returns empty body on 200 OK. Full ops guide: docs/OPERATIONS.md.",
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
        "createdAt": "2026-03-05T20:09:35.587Z",
        "updatedAt": "2026-03-05T20:09:35.587Z",
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
        "reasoning": "From unknown session on 3/5/2026"
      },
      {
        "id": "b-mmr8sn8a-hzrcap",
        "scope": "global",
        "category": "data",
        "content": "Collection names normalized (2026-03-15): abudawud→abu_dawud, ibnmajah→ibn_majah, ahmad→musnad_ahmad. 43 distinct collections. Cross-source duplicates still exist (same hadith_number from DS-002 and DS-013 under same collection).",
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
        "createdAt": "2026-03-15T04:16:21.658Z",
        "updatedAt": "2026-03-15T04:16:21.658Z",
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
        "reasoning": "From unknown session on 3/15/2026"
      }
    ],
    "antiPatterns": [],
    "historySnippets": [],
    "deprecatedWarnings": [],
    "suggestedCassQueries": [
      "cass search 'score gemini e-invoicing' --days 30",
      "cass search 'score error' --days 60",
      "cass search 'score' --days 90",
      "cass search 'score gemini' --days 60",
      "cass search 'e-invoicing' --days 90"
    ]
  },
  "metadata": {
    "executionMs": 110,
    "version": "0.2.3"
  }
}

codex
The project memory search did not turn up prior e-invoicing notes, so I’m treating “your list” as the implicit product direction we have in this thread: canonical IR, validation, Factur-X/XRechnung/Peppol, GOBL interop, OSS core plus hosted services. I’m pulling the idea rubric now, then I’ll score directly.
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc "sed -n '1,220p' /home/durakovic/.codex/skills/idea-wizard/references/RUBRIC.md" in /home/durakovic/projects/invoices
 succeeded in 0ms:
# Idea Evaluation Rubric

## Quick Score Card

Rate each idea 1-5 on each criterion:

| Criterion | 1 (Poor) | 3 (Acceptable) | 5 (Excellent) |
|-----------|----------|----------------|---------------|
| **Robust** | Breaks on edge cases | Handles common cases | Handles all cases gracefully |
| **Reliable** | Intermittent failures | Usually works | Always works |
| **Performant** | Noticeably slow | Acceptable speed | Imperceptibly fast |
| **Intuitive** | Confusing UX | Learnable | Obvious immediately |
| **User-friendly** | Frustrating | Neutral | Delightful |
| **Ergonomic** | Adds friction | No change | Reduces friction |
| **Useful** | Solves nothing | Solves minor pain | Solves major pain |
| **Compelling** | Nobody wants | Nice to have | Must have |
| **Accretive** | Negative value | Marginal value | Clear value |
| **Pragmatic** | Impossible | Difficult | Straightforward |

**Threshold:** Ideas scoring <3 average should be cut.

---

## Detailed Criteria

### Robust
- Does it handle empty input?
- Does it handle malformed input?
- Does it handle unicode?
- Does it handle concurrent access?
- Does it fail gracefully?

### Reliable
- Does it work the first time?
- Does it work the 1000th time?
- Does it work under load?
- Does it work offline?
- Does it recover from errors?

### Performant
- Is latency acceptable (<100ms for interactive)?
- Is throughput sufficient?
- Does it scale with data size?
- Does it use resources efficiently?
- Is it cache-friendly?

### Intuitive
- Can users predict behavior?
- Are defaults sensible?
- Is naming clear?
- Is documentation needed?
- Do errors explain themselves?

### User-friendly
- Is the happy path smooth?
- Are error messages helpful?
- Is recovery easy?
- Is undo available?
- Is help accessible?

### Ergonomic
- How many steps to accomplish goal?
- How much typing required?
- Are shortcuts available?
- Does it remember preferences?
- Does it reduce cognitive load?

### Useful
- What problem does it solve?
- How often does the problem occur?
- How painful is the problem?
- Are there workarounds?
- Does it create new problems?

### Compelling
- Would users request this?
- Would users pay for this?
- Would users switch for this?
- Would users recommend this?
- Would users miss this?

### Accretive
- Does it add capability?
- Does it reduce complexity?
- Does it improve existing features?
- Does it open new possibilities?
- Is the value measurable?

### Pragmatic
- Is the technology mature?
- Do we have the skills?
- Is the scope clear?
- Are dependencies manageable?
- Is the timeline reasonable?

---

## Winnowing Process

### Round 1: Hard Cuts
Remove any idea that scores 1 on ANY criterion.

### Round 2: Threshold
Remove any idea scoring <3 average.

### Round 3: Ranking
Sort remaining by weighted average:
- Useful: 2x weight
- Pragmatic: 2x weight
- Accretive: 1.5x weight
- Others: 1x weight

### Round 4: Synergy
Consider which ideas complement each other. A weaker idea that enables a stronger idea may be worth keeping.

---

## Red Flags

Immediate disqualification:
- "Users will figure it out"
- "We'll document it later"
- "It's technically correct"
- "Nobody does it differently"
- "We've always done it this way"

hook: PostToolUse
hook: PostToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc "find /home/durakovic/.codex/memories/rollout_summaries -maxdepth 1 -type f -iname '*invoice*' -o -iname '*peppol*' -o -iname '*factur*' | head -20" in /home/durakovic/projects/invoices
 succeeded in 0ms:
hook: PostToolUse
hook: PostToolUse Completed
codex
Caveat: I do not have my prior 25-item list in this visible thread, so the “both lists” ranking below uses Gemini’s list plus the roadmap ideas explicit here: canonical IR, validators, GOBL interop, hosted Peppol/gov orchestration, reconciliation. If you paste my prior list, I can re-rank exactly.

Scores are 0-1000 per dimension: `Smart / Useful / Practical / ROI`.

| # | Idea | Scores | Blunt take |
|---|---:|---:|---|
| 1 | Typst PDF/A-3 generation | 780 / 830 / 520 / 700 | Valuable, but deterministic PDF/A-3 is harder than they imply. PDF/A conformance, embedded XML, fonts, metadata, signatures, and byte stability are nasty. |
| 2 | Schema-compiled validation | 760 / 900 / 500 / 780 | Core idea is strong. “O(1)” is wrong; you still traverse invoice data and rule sets. AOT helps startup, portability, and determinism. |
| 3 | BYO-LLM via WebGPU | 520 / 220 / 280 / 120 | Mostly a distraction. Compliance tooling needs deterministic explainability, not browser-local Llama demos. |
| 4 | Semantic diffing for amendments | 860 / 860 / 710 / 870 | Excellent. This is real developer value: corrections, credit notes, audit trails, support debugging. |
| 5 | eBPF sniffer ingestion | 650 / 180 / 120 / 80 | Clever but wrong product. Invasive, brittle, high-liability, non-WASM, hard to support across customer infra. |
| 6 | Time-travel validation | 890 / 880 / 760 / 910 | One of the best. Date-pinned rules, profile versions, and reproducible validation are essential. |
| 7 | Dirty ingestion error stream | 730 / 780 / 680 / 720 | Useful, but GraphQL is incidental. The value is precise JSON-path diagnostics and repair hints. |
| 8 | SQLite VFS invoice envelopes | 650 / 420 / 260 / 250 | Overengineered. Use normal SQLite/storage layers first; custom VFS is complexity cosplay here. |
| 9 | Content-addressed attachments | 760 / 520 / 780 / 620 | Solid infrastructure, not a flagship. Good once storage/audit volume matters. |
| 10 | Gov chaos simulator | 850 / 840 / 640 / 860 | Strong managed-service idea. Network hell, bad ACKs, latency, and weird rejections are exactly where buyers feel pain. |
| 11 | WASM CIUS plugins | 780 / 700 / 520 / 630 | Good later. Hard part is stable plugin APIs, legal correctness, and community governance. |
| 12 | Reverse-proxy sidecar | 820 / 800 / 700 / 820 | Strong. This directly answers the JVM/.NET “WASM is leaky” critique. |
| 13 | ZKP for factoring | 690 / 110 / 60 / 40 | Kill. Technically fashionable, commercially weak, implementation burden absurd. |
| 14 | VS Code / Cursor LSP | 800 / 760 / 740 / 780 | Good developer-first surface, especially if backed by the same validator engine. |
| 15 | Invoice-as-Code CLI | 680 / 480 / 620 / 420 | Useful for tests, demos, fixtures, and ops. Bad if treated like Terraform for production invoices. |
| 16 | PII/GDPR redactor | 780 / 700 / 700 / 760 | Good, practical, especially for support dumps, staging, and bug reports. |
| 17 | Fuzzing as a Service | 820 / 650 / 590 / 680 | Good, but more buyer/AP-side than your stated ICP. Better as conformance harness than main product. |
| 18 | Homomorphic VAT aggregation | 620 / 90 / 30 / 20 | Kill. Academic novelty, no sane ROI for this product stage. |
| 19 | SSE/WebSockets over webhooks | 700 / 720 / 800 / 740 | Not radical, but practical. Good managed-service UX for ACK/event delivery. |
| 20 | Bounding-box OCR TUI | 500 / 240 / 520 / 200 | Wrong ICP. OCR correction is a different product. |
| 21 | Public “SSL Labs” for invoices | 780 / 820 / 700 / 860 | Strong GTM. Must be client-side or explicitly non-retentive because invoice payloads are sensitive. |
| 22 | Smart contract escrow bridge | 480 / 80 / 160 / 50 | Kill. Tiny market, reputationally distracting, complexity unjustified. |
| 23 | Any-to-any mapping DSL | 850 / 920 / 610 / 900 | Very strong if kept declarative and testable. ERP data mapping is the adoption bottleneck. |
| 24 | Tamper-evident logs | 820 / 700 / 760 / 780 | Good if it underpins reconciliation/state history. Weak if marketed as generic Merkle magic. |
| 25 | Fallback to paper API | 560 / 500 / 650 / 430 | Useful edge case, but not strategic. Partner integration later, not core. |

Gemini’s critique is partly right. WASM alone is not enough for enterprise embedding; you need native SDKs and a sidecar. Canonical IR is politically fraught only if you pretend one schema erases national CIUS weirdness. Free government gateways compress transmission margins, but not validation, mapping, observability, reconciliation, and support costs. Peppol AP operations are real fixed-cost pain, which argues for a hosted orchestration layer or partner strategy before becoming a full AP yourself.

Where Gemini is better than a pure SDK-first roadmap: the “Reconciliation Engine” callout is the strongest strategic idea here. They are also right that cryptography, async state, gateway behavior, retries, ACKs, and legal evidence are better as managed infrastructure than as “just use our WASM library.”

**Top 3 To Build**
1. **Reconciliation/state engine**: deterministic invoice fingerprint, idempotency, lifecycle state, amendments, credit notes, ACK matching, audit evidence, webhook/SSE events. This is likely the paid control plane.
2. **Versioned validation/profile engine**: EN 16931 plus CIUS profiles, date-pinned rule packs, golden fixtures, explainable diagnostics, AOT where useful. Do not claim O(1).
3. **Developer ingestion/mapping workbench**: GOBL interop, JSON/CSV-to-IR mapping DSL, precise JSON-path errors, LSP/editor feedback, fixture generation. This removes adoption friction.

**Bottom 3 To Kill**
1. **#18 Homomorphic encryption aggregation**: no credible near-term buyer.
2. **#13 ZKP factoring**: huge complexity for speculative value.
3. **#22 Smart contract escrow bridge**: brand-damaging distraction unless a real customer drags you there with money.

Near-kills: `#5 eBPF sniffer`, `#3 BYO-LLM`, `#20 OCR TUI`.

**Blind Spot**
Neither list clearly names **recipient capability and routing intelligence**: given seller, buyer, country, tax IDs, document type, date, and amount, determine the required profile, reachable transport route, Peppol participant status, endpoint scheme, fallback path, and preflight blockers before invoice generation. That is a high-value bridge between OSS validation and paid managed services.
tokens used
72,154
Caveat: I do not have my prior 25-item list in this visible thread, so the “both lists” ranking below uses Gemini’s list plus the roadmap ideas explicit here: canonical IR, validators, GOBL interop, hosted Peppol/gov orchestration, reconciliation. If you paste my prior list, I can re-rank exactly.

Scores are 0-1000 per dimension: `Smart / Useful / Practical / ROI`.

| # | Idea | Scores | Blunt take |
|---|---:|---:|---|
| 1 | Typst PDF/A-3 generation | 780 / 830 / 520 / 700 | Valuable, but deterministic PDF/A-3 is harder than they imply. PDF/A conformance, embedded XML, fonts, metadata, signatures, and byte stability are nasty. |
| 2 | Schema-compiled validation | 760 / 900 / 500 / 780 | Core idea is strong. “O(1)” is wrong; you still traverse invoice data and rule sets. AOT helps startup, portability, and determinism. |
| 3 | BYO-LLM via WebGPU | 520 / 220 / 280 / 120 | Mostly a distraction. Compliance tooling needs deterministic explainability, not browser-local Llama demos. |
| 4 | Semantic diffing for amendments | 860 / 860 / 710 / 870 | Excellent. This is real developer value: corrections, credit notes, audit trails, support debugging. |
| 5 | eBPF sniffer ingestion | 650 / 180 / 120 / 80 | Clever but wrong product. Invasive, brittle, high-liability, non-WASM, hard to support across customer infra. |
| 6 | Time-travel validation | 890 / 880 / 760 / 910 | One of the best. Date-pinned rules, profile versions, and reproducible validation are essential. |
| 7 | Dirty ingestion error stream | 730 / 780 / 680 / 720 | Useful, but GraphQL is incidental. The value is precise JSON-path diagnostics and repair hints. |
| 8 | SQLite VFS invoice envelopes | 650 / 420 / 260 / 250 | Overengineered. Use normal SQLite/storage layers first; custom VFS is complexity cosplay here. |
| 9 | Content-addressed attachments | 760 / 520 / 780 / 620 | Solid infrastructure, not a flagship. Good once storage/audit volume matters. |
| 10 | Gov chaos simulator | 850 / 840 / 640 / 860 | Strong managed-service idea. Network hell, bad ACKs, latency, and weird rejections are exactly where buyers feel pain. |
| 11 | WASM CIUS plugins | 780 / 700 / 520 / 630 | Good later. Hard part is stable plugin APIs, legal correctness, and community governance. |
| 12 | Reverse-proxy sidecar | 820 / 800 / 700 / 820 | Strong. This directly answers the JVM/.NET “WASM is leaky” critique. |
| 13 | ZKP for factoring | 690 / 110 / 60 / 40 | Kill. Technically fashionable, commercially weak, implementation burden absurd. |
| 14 | VS Code / Cursor LSP | 800 / 760 / 740 / 780 | Good developer-first surface, especially if backed by the same validator engine. |
| 15 | Invoice-as-Code CLI | 680 / 480 / 620 / 420 | Useful for tests, demos, fixtures, and ops. Bad if treated like Terraform for production invoices. |
| 16 | PII/GDPR redactor | 780 / 700 / 700 / 760 | Good, practical, especially for support dumps, staging, and bug reports. |
| 17 | Fuzzing as a Service | 820 / 650 / 590 / 680 | Good, but more buyer/AP-side than your stated ICP. Better as conformance harness than main product. |
| 18 | Homomorphic VAT aggregation | 620 / 90 / 30 / 20 | Kill. Academic novelty, no sane ROI for this product stage. |
| 19 | SSE/WebSockets over webhooks | 700 / 720 / 800 / 740 | Not radical, but practical. Good managed-service UX for ACK/event delivery. |
| 20 | Bounding-box OCR TUI | 500 / 240 / 520 / 200 | Wrong ICP. OCR correction is a different product. |
| 21 | Public “SSL Labs” for invoices | 780 / 820 / 700 / 860 | Strong GTM. Must be client-side or explicitly non-retentive because invoice payloads are sensitive. |
| 22 | Smart contract escrow bridge | 480 / 80 / 160 / 50 | Kill. Tiny market, reputationally distracting, complexity unjustified. |
| 23 | Any-to-any mapping DSL | 850 / 920 / 610 / 900 | Very strong if kept declarative and testable. ERP data mapping is the adoption bottleneck. |
| 24 | Tamper-evident logs | 820 / 700 / 760 / 780 | Good if it underpins reconciliation/state history. Weak if marketed as generic Merkle magic. |
| 25 | Fallback to paper API | 560 / 500 / 650 / 430 | Useful edge case, but not strategic. Partner integration later, not core. |

Gemini’s critique is partly right. WASM alone is not enough for enterprise embedding; you need native SDKs and a sidecar. Canonical IR is politically fraught only if you pretend one schema erases national CIUS weirdness. Free government gateways compress transmission margins, but not validation, mapping, observability, reconciliation, and support costs. Peppol AP operations are real fixed-cost pain, which argues for a hosted orchestration layer or partner strategy before becoming a full AP yourself.

Where Gemini is better than a pure SDK-first roadmap: the “Reconciliation Engine” callout is the strongest strategic idea here. They are also right that cryptography, async state, gateway behavior, retries, ACKs, and legal evidence are better as managed infrastructure than as “just use our WASM library.”

**Top 3 To Build**
1. **Reconciliation/state engine**: deterministic invoice fingerprint, idempotency, lifecycle state, amendments, credit notes, ACK matching, audit evidence, webhook/SSE events. This is likely the paid control plane.
2. **Versioned validation/profile engine**: EN 16931 plus CIUS profiles, date-pinned rule packs, golden fixtures, explainable diagnostics, AOT where useful. Do not claim O(1).
3. **Developer ingestion/mapping workbench**: GOBL interop, JSON/CSV-to-IR mapping DSL, precise JSON-path errors, LSP/editor feedback, fixture generation. This removes adoption friction.

**Bottom 3 To Kill**
1. **#18 Homomorphic encryption aggregation**: no credible near-term buyer.
2. **#13 ZKP factoring**: huge complexity for speculative value.
3. **#22 Smart contract escrow bridge**: brand-damaging distraction unless a real customer drags you there with money.

Near-kills: `#5 eBPF sniffer`, `#3 BYO-LLM`, `#20 OCR TUI`.

**Blind Spot**
Neither list clearly names **recipient capability and routing intelligence**: given seller, buyer, country, tax IDs, document type, date, and amount, determine the required profile, reachable transport route, Peppol participant status, endpoint scheme, fallback path, and preflight blockers before invoice generation. That is a high-value bridge between OSS validation and paid managed services.
