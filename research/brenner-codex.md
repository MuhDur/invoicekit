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
session id: 019e62cf-a4c2-79d1-a664-19db8e8b7aca
--------
user
You are acting as an adversarial critic / Brenner-style researcher for our open-source e-invoicing toolkit project.

# Top 20 candidate ideas surviving our ideation rounds (mix of agreed-on and contested)

1. WASM-native Rust core (single artifact runs in browser, Node, Bun, Deno, edge, JVM, Python, Go)
2. EN 16931 semantic IR as canonical model, interop with invopop/gobl
3. Schematron→Rust AOT-compiled validator (no XSLT 2.0 / Java needed)
4. Typst-based deterministic PDF/A-3 generator (no Puppeteer/headless Chrome)
5. Browser-side OCR/extraction via WebGPU + Qwen2.5-VL-7B / PaddleOCR
6. Auditable AI: every extracted field carries bounding-box citation + confidence
7. Smart canonicalizer for byte-stable XML output (basis for signing/hashing)
8. Stripe-style sandbox/test mode for Peppol AS4 / SDI / KSeF / IRP / ZATCA / PPF
9. Public client-side WASM validator (browser-only, no upload, free trust play)
10. Peppol AS4 hybrid path: wrap phase4 (JVM) Y1, native Rust Y2-3 with phase4 as conformance oracle
11. `routePlan(invoice, recipient)` API — what network/profile/IDs/gateway/fallback needed
12. Date-aware compliance queries (`capabilities --from DE --to FR --date 2027-01-01`)
13. Type-state invoice builders (Rust + TypeScript) — invalid invoices unrepresentable
14. Tax & rounding proof engine — deterministic decimal math with formal trace
15. Evidence bundle format `.invoicekit` — canonical IR + source + XML + PDF + receipts + signatures
16. Clearance state-machine SDK — unified per-country state machines
17. Deterministic invoice fingerprint for reconciliation (`blake3(VAT||VAT||date||no||total||currency)`)
18. Inbound invoice firewall (XXE, ZIP bombs, malware, etc.)
19. Gateway rejection normalizer — uniform error categories with remediation
20. Invoice LSP / VS Code extension

# Pricing strategy

Free up to 100 envelopes/mo; Pro €29-49/mo for 1000; €0.05/envelope flat above. OSS core MIT/Apache forever.

# Target ICP

Embedded developer at ERP/billing SaaS vendor (Odoo, Microsoft Dynamics, sevDesk, Pennylane, Lexware, Pleo). ~500 EU targets, €15-150k ACV.

# Your task

Be the harshest possible adversarial critic. Specifically:

**A. Five most likely modes of failure**

For each, state the failure precisely, the evidence we'd see, and which idea(s) it kills.

**B. Five hidden assumptions we're making that could be wrong**

For each, state the assumption, why it might be wrong, and what we should test to falsify.

**C. Five competitive threats we're underestimating**

What incumbent, new entrant, or trend could neutralize our wedge?

**D. Three specific experiments / tests we should run in the first 60 days to falsify our biggest bets cheaply**

Be concrete: what would we measure, what threshold means "kill the bet", how much would it cost.

**E. One thing that would make this project a runaway success that we haven't planned for**

Look 18 months out.

Be brutal. Be specific. Reference standards/libs/companies by name. We need signal, not encouragement.
codex
I’m going to use the Brenner workflow because you asked for an adversarial research pass, and I’ll keep it focused on falsifiable failure modes rather than product encouragement. I’ll also pull current primary-ish context for standards, libraries, and market threats so the criticism is anchored in today’s ecosystem.
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
hook: PreToolUse Completed
exec
/bin/zsh -lc "sed -n '1,220p' /home/durakovic/.codex/skills/brenner/SKILL.md" in /home/durakovic/projects/invoices
 succeeded in 0ms:
---
name: brenner
description: >-
  Brenner Bot CLI for research sessions. Use when in your brenner_bot repo,
  managing hypotheses, searching corpus, or running multi-agent sessions.
---

# Brenner Bot

> **Core Insight:** Exclusion beats confirmation. Design experiments to kill hypotheses, not prove them. A theory that survives elimination is stronger than one with supporting evidence.

## Table of Contents

[THE EXACT PROMPT](#the-exact-prompt--research-session-workflow) | [When to Use What](#when-to-use-what) | [Critical Constraints](#critical-constraints) | [Artifact Schema](#7-section-artifact-schema) | [Heuristics](#heuristics) | [References](#references)

---

## THE EXACT PROMPT — Research Session Workflow

```
1. Health Check: Verify toolchain before any session
   brenner doctor --skip-ntm --skip-cass --skip-cm --json

2. Corpus Search: Find relevant Brenner transcript sections
   brenner corpus search "model organism"
   brenner corpus search "reduction to one dimension"

3. Build Excerpt: Create cited context (ALWAYS before session)
   brenner excerpt build --sections 58,78,161 > excerpt.md

4. Start Session: Launch multi-agent research thread
   brenner session start \
     --project-key "$PWD" \
     --sender GreenCastle \
     --to BlueLake \
     --thread-id RS-$(date +%Y%m%d)-SLUG \
     --excerpt-file excerpt.md \
     --question "Your research question here"

5. Monitor: Check session progress
   brenner session status --thread-id RS-YYYYMMDD-SLUG
   brenner mail agents --project-key "$PWD"

6. Compile: Merge agent deltas into artifact
   brenner session compile --thread-id RS-YYYYMMDD-SLUG

7. Validate: Lint against 50+ Brenner-style rules
   brenner artifact lint artifact.md
   brenner artifact nudge artifact.md
```

### Why This Workflow Works

- **Doctor first** — Catch toolchain issues before wasting a session
- **Excerpts before sessions** — Corpus context grounds the research
- **Thread ID consistency** — Same ID across Agent Mail, ntm, artifacts
- **Compile frequently** — Don't wait for session end; incremental merges

---

## When to Use What

| You Want | Use | Why |
|----------|-----|-----|
| Start research | `session start` with excerpt | Corpus context grounds everything |
| Find quotes | `corpus search "term"` | 236 sections with §n anchors |
| Check progress | `session status --thread-id` | See agent activity |
| Merge outputs | `session compile` | Deterministic delta merge |
| Validate artifact | `artifact lint` then `nudge` | 50+ Brenner-style rules |
| Debug setup | `doctor --skip-ntm --skip-cass --skip-cm` | Minimal health check |

---

## Critical Constraints

1. **No vendor API calls** — Use CLI tools via ntm, not direct API calls
2. **Thread ID is the join key** — Same ID across Agent Mail, ntm, artifacts, beads
3. **Agent names = adjective+noun** — GreenCastle, BlueLake, RedStone
4. **Always build excerpts first** — Corpus context before session start
5. **Third alternative required** — Every hypothesis slate needs "both could be wrong"

## Thread ID Formats

| Context | Format | Example |
|---------|--------|---------|
| Research sessions | `RS-{YYYYMMDD}-{slug}` | `RS-20260119-cell-fate` |
| Engineering work | Bead ID directly | `brenner_bot-5so.3.4.2` |

## 7-Section Artifact Schema

Research artifacts must contain:

1. **research_thread** — Stable problem statement
2. **hypothesis_slate** — 2-5 hypotheses (must include "both wrong" third alternative)
3. **predictions_table** — Discriminative predictions per hypothesis
4. **discriminative_tests** — Ranked "decision experiments"
5. **assumption_ledger** — Load-bearing assumptions + scale/physics checks
6. **anomaly_register** — Explicitly quarantined exceptions
7. **adversarial_critique** — What would make the whole framing wrong?

## Delta Operations

Agents emit changes via fenced JSON blocks:

```json brenner-delta
{
  "operation": "ADD",
  "target_section": "hypothesis_slate",
  "payload": {
    "id": "H3",
    "statement": "Both mechanisms are wrong",
    "state": "proposed"
  },
  "rationale": "Third-alternative injection per Brenner operator #3"
}
```

| Operation | Behavior |
|-----------|----------|
| `ADD` | Insert new item (no target_id) |
| `EDIT` | Modify existing (requires target_id) |
| `KILL` | Mark as killed (requires target_id) |

## Key Brenner Operators

| # | Operator | Core Insight |
|---|----------|--------------|
| 1 | Model Organism Selection | Choose simplest system preserving phenomenon |
| 2 | Reduction to One Dimension | Strip to A→B→C causal chain |
| 3 | Third Alternative Injection | "Both could be wrong" |
| 4 | Potency Test | Distinguish "didn't" from "couldn't" |
| 5 | Reconstruction Criterion | Build it from primitives or you don't understand |
| 7 | Exclusion Over Confirmation | Design to kill hypotheses, not confirm |

**Full operators**: See [references/OPERATORS.md](references/OPERATORS.md)

## Heuristics

| Signal | Meaning | Action |
|--------|---------|--------|
| Lint fails HYP-002 | Missing third alternative | Add "both wrong" hypothesis |
| `line_number` 1-3 in corpus | Key Brenner quote | Use §n citation format |
| Session status "stalled" | No deltas in 10+ min | Check Agent Mail, nudge agents |
| Artifact >5 hypotheses | Scope creep | Kill or merge hypotheses |
| Anomaly count >3 | Quarantine overload | Review if pattern emerges |
| No potency control (TEST-003) | Uninformative negatives | Add positive control to test |

---

## Troubleshooting

| Error | Fix |
|-------|-----|
| "Missing --question" | Required flag for session start |
| "Missing --sender" | Add `--sender GreenCastle` or set `AGENT_NAME` env |
| "Agent Mail not available" | Start Agent Mail server on localhost:8765 |
| Invalid agent name | Use adjective+noun format (GreenCastle, BlueLake) |
| Lint fails silently | Run with `--json` for detailed rule violations |
| Deltas not merging | Check thread ID matches across all systems |

## References

| Need | Reference |
|------|-----------|
| Brenner's 13 operators | [OPERATORS.md](references/OPERATORS.md) |
| 50+ linting rules | [LINTING-RULES.md](references/LINTING-RULES.md) |
| Delta format details | [DELTA-FORMAT.md](references/DELTA-FORMAT.md) |
| Tribunal personas | [TRIBUNAL.md](references/TRIBUNAL.md) |
| Workflow recipes | [RECIPES.md](references/RECIPES.md) |
| Prediction locks, hypothesis arena | [ADVANCED.md](references/ADVANCED.md) |

---

## Validation

```bash
# Quick health check
brenner doctor --skip-ntm --skip-cass --skip-cm --json | jq '.status'

# Should return: "healthy"
```

If unhealthy, check:
1. Agent Mail server running on localhost:8765
2. Corpus files present in `$BRENNER_ROOT/corpus/`
3. Bun runtime available

---

## Related Skills

- `agent-mail` — Agent Mail coordination
- `ntm` — Multi-agent tmux orchestration
- `cass` — Session archaeology
- `br` — Beads task tracking

hook: PostToolUse
hook: PostToolUse Completed
exec
/bin/zsh -lc 'cm context "adversarial Brenner-style critique for open-source e-invoicing toolkit bets" --json' in /home/durakovic/projects/invoices
hook: PostToolUse
hook: PostToolUse Completed
 succeeded in 309ms:
{
  "success": true,
  "command": "context",
  "timestamp": "2026-05-26T05:44:13.708Z",
  "data": {
    "task": "adversarial Brenner-style critique for open-source e-invoicing toolkit bets",
    "relevantBullets": [],
    "antiPatterns": [],
    "historySnippets": [],
    "deprecatedWarnings": [],
    "suggestedCassQueries": [
      "cass search 'adversarial brenner-style critique' --days 30",
      "cass search 'adversarial error' --days 60",
      "cass search 'adversarial' --days 90",
      "cass search 'adversarial brenner-style' --days 60",
      "cass search 'critique' --days 90"
    ]
  },
  "metadata": {
    "executionMs": 116,
    "version": "0.2.3"
  }
}

web search: 
web search: European Commission VAT in the Digital Age electronic invoicing mandatory July 2030 official ...
web search: 
web search: https://taxation-customs.ec.europa.eu/news/adoption-vat-digital-age-package-2025-03-11_en?prefLang=it
web search: 
web search: site:impots.gouv.fr facturation electronique 2026 2027 PDP officiel ...
web search: 
web search: GitHub phax phase4 Peppol AS4 Java library ...
web search: 
web search: Sovos e-invoicing API Peppol compliance platform ...
web search: 
web search: Odoo Peppol e-invoicing documentation ...
web search: 
web search: Factur-X ZUGFeRD PDF/A-3 EN16931 official ...
web search: 
web search: Qwen2.5-VL-7B-Instruct model size license Hugging Face ...
web search: 
web search: https://test-docs.peppol.eu/poacc/billing/3.0/bis/
web search: 
web search: 'Model size' in https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct
web search: 
web search: https://docs.peppol.eu/poac/policies/OpenPeppol%20Policy%20on%20BIS%20Billing%20Country%20specific%20validation%20rules%20v1.1.0.pdf
web search: 
web search: 
web search: 
web search: site:podatki.gov.pl KSeF 2.0 obowiązkowy 1 lutego 2026 1 kwietnia 2026 ...
codex
Net: the most likely death is not “can’t build it.” It is “excellent developer artifact, not trusted compliance infrastructure.”

As of **May 26, 2026**, Germany’s B2B e-invoice regime is already active with transition rules, France starts broad receipt obligations on **September 1, 2026**, Poland’s KSeF 2.0 is already live, and ViDA cross-border digital reporting starts **July 1, 2030**. This market is real, but that also means buyers are already choosing vendors.

**A. Five Most Likely Failure Modes**

1. **You build a great validator, but not the validator of record.**  
   Evidence: mismatches against Saxon/KoSIT/phive/OpenPeppol artifacts; customer asks “will this pass my PDP/AP?”; auditors still require official validator output. OpenPeppol explicitly anchors rules in Schematron/XPath/XSLT-style artifacts and country-specific rule releases. Kills: `3`, weakens `9`, `13`, `14`, `19`.

2. **The “canonical EN 16931 IR” collapses under national/profile reality.**  
   EN 16931 is the floor, not the product. Peppol BIS is a CIUS of EN 16931; Germany has XRechnung/ZUGFeRD; France has platform-mediated flows and PDPs; Poland has KSeF FA(3); Italy has SDI/FatturaPA. Evidence: your IR needs escape hatches for every serious customer, then becomes GOBL-but-Rust with more adapters. Kills: `2`, `11`, `12`, `13`, `16`.

3. **Route/sandbox/state-machine APIs are not credible without network authority.**  
   `routePlan(invoice, recipient)` sounds great, but real routing depends on Peppol participant discovery, SMP/SML, buyer registration, PDP/AP constraints, government platform state, certificates, and bilateral commercial setup. Evidence: test users say “use our AP/PDP sandbox,” not yours; your simulator produces false confidence; production failures are access-point/status issues, not XML issues. Kills: `8`, `10`, `11`, `16`, `19`.

4. **OSS trust play attracts users who do not pay.**  
   Free browser validation will get accountants, consultants, hobbyists, and support teams. Your paid ICP wants liability, SLA, country coverage, connectors, and someone to blame. Evidence: GitHub stars, validator traffic, and no ACV; users paste XML into the browser but envelope volume goes to Avalara, Sovos, Pagero, EDICOM, Odoo, Microsoft, SAP. Kills pricing strategy, weakens `1`, `3`, `9`, `15`, `20`.

5. **AI OCR is a distraction from structured mandate reality.**  
   Browser OCR is useful for legacy inbound PDFs, but mandates are pushing machine-readable XML. Ordinary PDFs are explicitly not enough under regimes like Germany/France. Qwen2.5-VL-7B is an 8B BF16 model, and PaddleOCR browser deployment is plausible but operationally fiddly. Evidence: extraction demos impress, but ERP SaaS buyers ask for XML validation, routing, clearance, and rejection handling. Kills: `5`, `6`; weakens `18` if the firewall becomes PDF-centric.

**B. Hidden Assumptions To Falsify**

1. **Assumption: embedded ERP developers want libraries more than compliance services.**  
   Might be wrong because compliance is risk transfer. Test: 30 ICP interviews. Kill library-first if fewer than 5 will sign a paid pilot without managed compliance/SLA.

2. **Assumption: Rust/WASM portability is a decisive wedge.**  
   Might be wrong because the hard parts are certificates, AP/PDP connectivity, national APIs, archiving, and support. Test: ask buyers to rank “runs in browser/edge” against “certified country coverage.” Kill if portability is not top 3.

3. **Assumption: AOT Schematron can reach practical parity cheaply.**  
   Might be wrong because XPath 2.0, decimal/date semantics, codelists, country rules, and hotfixes are brutal. Test: differential harness against official validators. Kill if any fatal-rule mismatch remains after 3 weeks.

4. **Assumption: invalid invoices can be made unrepresentable.**  
   Might be wrong because validity is date-, country-, buyer-, profile-, gateway-, and release-dependent. Test: model 50 real edge cases across DE/FR/PL/IT/Peppol. Kill type-state as headline if escape hatches dominate.

5. **Assumption: flat €0.05/envelope is attractive.**  
   Might be wrong because enterprise buyers compare total compliance risk, not marginal message price. Test: quote €5k, €25k, €75k annual pilots. Kill low-price positioning if buyers trust expensive incumbents more.

**C. Competitive Threats You Are Underestimating**

1. **Avalara/Sovos/Pagero/OpenText/EDICOM/Basware/Comarch.** They already sell “single API, many countries, managed compliance,” with enterprise trust and connectors. Avalara’s developer docs explicitly pitch one API for formats, validation, submission, Peppol and national platforms.

2. **ERP-native distribution.** Odoo already acts as Peppol access point/SMP in its accounting flow. Microsoft Dynamics 365 lists broad country e-invoicing coverage including France, Germany, Italy, Poland, Saudi Arabia, Singapore, Peppol. SAP has Document and Reporting Compliance. They own the workflow.

3. **Government/PDP gravity.** France has approved platform infrastructure; Poland has KSeF; Italy has SDI; Germany allows flexible transmission but structured formats. If the regulated platform is the product, your SDK becomes adapter plumbing.

4. **Open-source incumbents.** invopop/gobl already covers conversions to UBL, CII, Factur-X/ZUGFeRD, XRechnung, FatturaPA, KSeF, Stripe. phax/phase4 is established Java AS4/Peppol infrastructure. KoSIT validators are official-ish in Germany. Your OSS core is not automatically novel.

5. **“Good enough” AI inside incumbents.** Rejection explanation, mapping hints, OCR, and remediation are features incumbents can add to existing transaction pipes. If they add AI repair around their existing compliance rails, your AI wedge shrinks.

**D. First 60-Day Kill Tests**

1. **Validator parity gauntlet.**  
   Build only the minimal Rust AOT Schematron path for Peppol BIS + XRechnung + Factur-X samples. Compare against Saxon/KoSIT/phive on official and generated edge cases. Measure fatal/warning parity, rule IDs, XPath locations, runtime. Kill if `<99.9%` rule parity or any unexplained fatal mismatch after 3 weeks. Cost: mostly one engineer, €0-€3k.

2. **ICP risk-transfer test.**  
   Contact 50 ERP/billing SaaS targets; run 30 calls; ask for a paid 90-day pilot for validator + rejection normalizer + route planner. Threshold: at least 5 signed LOIs or paid pilots at `>=€5k`. Kill developer-tool positioning if most say “we need certified AP/PDP/vendor-of-record.” Cost: €2k-€8k outreach plus founder time.

3. **Real-invoice corpus break test.**  
   Get 200 anonymized invoices/credit notes across DE, FR, PL, IT, Peppol cross-border, including corrections, allowances, reverse charge, mixed VAT, attachments, rounding edge cases. Try canonical IR round-trip to UBL/CII/local formats. Kill canonical-IR-as-headline if more than 2% require lossy fields or opaque per-country extensions. Cost: €5k-€15k via design partners/consultants.

**E. Runaway Success You Have Not Planned For**

Become the **open conformance corpus and differential test harness** for e-invoicing, not just the toolkit. If, 18 months from now, ERP vendors, APs, consultants, and even incumbents use your public fixtures, validator diffs, rejection taxonomy, and `.invoicekit` evidence bundles to debug compliance failures, you win distribution and trust before you win envelope volume.

That is the wedge: “CI for invoice compliance.” The paid product then becomes hosted conformance, private corpus testing, release-diff alerts, and certified gateway adapters.

Sources: European Commission ViDA adoption and 2030/2035 timeline; German BMF e-invoice FAQ; French Service Public/DGFiP rollout; Polish KSeF official roadmap; OpenPeppol BIS and country-rule policy; Avalara developer docs; Odoo e-invoicing docs; Microsoft Dynamics e-invoicing coverage; invopop/gobl; phax/phase4; Typst PDF/A docs; Qwen and PaddleOCR docs.
tokens used
110,773
Net: the most likely death is not “can’t build it.” It is “excellent developer artifact, not trusted compliance infrastructure.”

As of **May 26, 2026**, Germany’s B2B e-invoice regime is already active with transition rules, France starts broad receipt obligations on **September 1, 2026**, Poland’s KSeF 2.0 is already live, and ViDA cross-border digital reporting starts **July 1, 2030**. This market is real, but that also means buyers are already choosing vendors.

**A. Five Most Likely Failure Modes**

1. **You build a great validator, but not the validator of record.**  
   Evidence: mismatches against Saxon/KoSIT/phive/OpenPeppol artifacts; customer asks “will this pass my PDP/AP?”; auditors still require official validator output. OpenPeppol explicitly anchors rules in Schematron/XPath/XSLT-style artifacts and country-specific rule releases. Kills: `3`, weakens `9`, `13`, `14`, `19`.

2. **The “canonical EN 16931 IR” collapses under national/profile reality.**  
   EN 16931 is the floor, not the product. Peppol BIS is a CIUS of EN 16931; Germany has XRechnung/ZUGFeRD; France has platform-mediated flows and PDPs; Poland has KSeF FA(3); Italy has SDI/FatturaPA. Evidence: your IR needs escape hatches for every serious customer, then becomes GOBL-but-Rust with more adapters. Kills: `2`, `11`, `12`, `13`, `16`.

3. **Route/sandbox/state-machine APIs are not credible without network authority.**  
   `routePlan(invoice, recipient)` sounds great, but real routing depends on Peppol participant discovery, SMP/SML, buyer registration, PDP/AP constraints, government platform state, certificates, and bilateral commercial setup. Evidence: test users say “use our AP/PDP sandbox,” not yours; your simulator produces false confidence; production failures are access-point/status issues, not XML issues. Kills: `8`, `10`, `11`, `16`, `19`.

4. **OSS trust play attracts users who do not pay.**  
   Free browser validation will get accountants, consultants, hobbyists, and support teams. Your paid ICP wants liability, SLA, country coverage, connectors, and someone to blame. Evidence: GitHub stars, validator traffic, and no ACV; users paste XML into the browser but envelope volume goes to Avalara, Sovos, Pagero, EDICOM, Odoo, Microsoft, SAP. Kills pricing strategy, weakens `1`, `3`, `9`, `15`, `20`.

5. **AI OCR is a distraction from structured mandate reality.**  
   Browser OCR is useful for legacy inbound PDFs, but mandates are pushing machine-readable XML. Ordinary PDFs are explicitly not enough under regimes like Germany/France. Qwen2.5-VL-7B is an 8B BF16 model, and PaddleOCR browser deployment is plausible but operationally fiddly. Evidence: extraction demos impress, but ERP SaaS buyers ask for XML validation, routing, clearance, and rejection handling. Kills: `5`, `6`; weakens `18` if the firewall becomes PDF-centric.

**B. Hidden Assumptions To Falsify**

1. **Assumption: embedded ERP developers want libraries more than compliance services.**  
   Might be wrong because compliance is risk transfer. Test: 30 ICP interviews. Kill library-first if fewer than 5 will sign a paid pilot without managed compliance/SLA.

2. **Assumption: Rust/WASM portability is a decisive wedge.**  
   Might be wrong because the hard parts are certificates, AP/PDP connectivity, national APIs, archiving, and support. Test: ask buyers to rank “runs in browser/edge” against “certified country coverage.” Kill if portability is not top 3.

3. **Assumption: AOT Schematron can reach practical parity cheaply.**  
   Might be wrong because XPath 2.0, decimal/date semantics, codelists, country rules, and hotfixes are brutal. Test: differential harness against official validators. Kill if any fatal-rule mismatch remains after 3 weeks.

4. **Assumption: invalid invoices can be made unrepresentable.**  
   Might be wrong because validity is date-, country-, buyer-, profile-, gateway-, and release-dependent. Test: model 50 real edge cases across DE/FR/PL/IT/Peppol. Kill type-state as headline if escape hatches dominate.

5. **Assumption: flat €0.05/envelope is attractive.**  
   Might be wrong because enterprise buyers compare total compliance risk, not marginal message price. Test: quote €5k, €25k, €75k annual pilots. Kill low-price positioning if buyers trust expensive incumbents more.

**C. Competitive Threats You Are Underestimating**

1. **Avalara/Sovos/Pagero/OpenText/EDICOM/Basware/Comarch.** They already sell “single API, many countries, managed compliance,” with enterprise trust and connectors. Avalara’s developer docs explicitly pitch one API for formats, validation, submission, Peppol and national platforms.

2. **ERP-native distribution.** Odoo already acts as Peppol access point/SMP in its accounting flow. Microsoft Dynamics 365 lists broad country e-invoicing coverage including France, Germany, Italy, Poland, Saudi Arabia, Singapore, Peppol. SAP has Document and Reporting Compliance. They own the workflow.

3. **Government/PDP gravity.** France has approved platform infrastructure; Poland has KSeF; Italy has SDI; Germany allows flexible transmission but structured formats. If the regulated platform is the product, your SDK becomes adapter plumbing.

4. **Open-source incumbents.** invopop/gobl already covers conversions to UBL, CII, Factur-X/ZUGFeRD, XRechnung, FatturaPA, KSeF, Stripe. phax/phase4 is established Java AS4/Peppol infrastructure. KoSIT validators are official-ish in Germany. Your OSS core is not automatically novel.

5. **“Good enough” AI inside incumbents.** Rejection explanation, mapping hints, OCR, and remediation are features incumbents can add to existing transaction pipes. If they add AI repair around their existing compliance rails, your AI wedge shrinks.

**D. First 60-Day Kill Tests**

1. **Validator parity gauntlet.**  
   Build only the minimal Rust AOT Schematron path for Peppol BIS + XRechnung + Factur-X samples. Compare against Saxon/KoSIT/phive on official and generated edge cases. Measure fatal/warning parity, rule IDs, XPath locations, runtime. Kill if `<99.9%` rule parity or any unexplained fatal mismatch after 3 weeks. Cost: mostly one engineer, €0-€3k.

2. **ICP risk-transfer test.**  
   Contact 50 ERP/billing SaaS targets; run 30 calls; ask for a paid 90-day pilot for validator + rejection normalizer + route planner. Threshold: at least 5 signed LOIs or paid pilots at `>=€5k`. Kill developer-tool positioning if most say “we need certified AP/PDP/vendor-of-record.” Cost: €2k-€8k outreach plus founder time.

3. **Real-invoice corpus break test.**  
   Get 200 anonymized invoices/credit notes across DE, FR, PL, IT, Peppol cross-border, including corrections, allowances, reverse charge, mixed VAT, attachments, rounding edge cases. Try canonical IR round-trip to UBL/CII/local formats. Kill canonical-IR-as-headline if more than 2% require lossy fields or opaque per-country extensions. Cost: €5k-€15k via design partners/consultants.

**E. Runaway Success You Have Not Planned For**

Become the **open conformance corpus and differential test harness** for e-invoicing, not just the toolkit. If, 18 months from now, ERP vendors, APs, consultants, and even incumbents use your public fixtures, validator diffs, rejection taxonomy, and `.invoicekit` evidence bundles to debug compliance failures, you win distribution and trust before you win envelope volume.

That is the wedge: “CI for invoice compliance.” The paid product then becomes hosted conformance, private corpus testing, release-diff alerts, and certified gateway adapters.

Sources: European Commission ViDA adoption and 2030/2035 timeline; German BMF e-invoice FAQ; French Service Public/DGFiP rollout; Polish KSeF official roadmap; OpenPeppol BIS and country-rule policy; Avalara developer docs; Odoo e-invoicing docs; Microsoft Dynamics e-invoicing coverage; invopop/gobl; phax/phase4; Typst PDF/A docs; Qwen and PaddleOCR docs.
