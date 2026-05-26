#!/usr/bin/env python3
"""
plan_to_beads.py — generate the InvoiceKit bead graph from PLAN.md v0.8.

Each bead is defined here in compact form: a tuple of
  (tid, slug, title, type, priority, deps, labels, body)

A shared template wraps `body` into a strict self-contained structure with
acceptance gates. The script then calls `br create` for each bead (capturing
IDs into a `tid -> br-id` map), wires dependencies with `br dep add`, and
validates the graph.

Re-running this script after creation is idempotent only if the workspace is
clean; otherwise it errors on duplicate slugs (the `--slug` flag generates a
deterministic ID prefix).

Usage:
    ./plan_to_beads.py                 # create all
    ./plan_to_beads.py --check         # check graph health, no creation
    ./plan_to_beads.py --validate-only # run br dep cycles + bv insights only
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
MAPPING_FILE = REPO_ROOT / ".beads" / "tid_mapping.json"

# ────────────────────────────────────────────────────────────────────────────
# Bead template — every bead body gets wrapped in this structure.
# ────────────────────────────────────────────────────────────────────────────

BODY_TEMPLATE = """## Background

{background}

## Goal

{goal}

## Acceptance criteria (strict gates — bead-specific)

{acceptance}

## Agent kickoff — read this before you touch any code

This section is identical across every bead and exists so any agent (Claude, Codex, or any future binding) follows the same starting protocol. Skipping these steps causes silent collisions with other parallel agents and produces work that does not survive review.

### Before you write code

1. **Claim the bead atomically.** Run `br update <bead-id> --claim` to assign yourself as owner and set status to `in_progress`. If the claim fails because the bead is already claimed, do NOT take it — pick another from `bv --robot-next`.
2. **Read this entire bead from top to bottom.** Specifically including the universal quality gates and the applicable type-specific gates below. Make a mental list of which type-specific gates you will need to waive (record the rationale for the close step).
3. **Walk the dependencies.** Every bead under "Dependencies" upstream of this one has shipped what you need; read each upstream bead's body (`br show <id>`) so you know exactly what API contract you can rely on. If an upstream bead is open, this bead should not be `in_progress` — your claim was premature.
4. **Read the referenced PLAN.md sections.** They contain the architectural context this bead assumes. The PLAN is at `/home/durakovic/projects/invoices/plans/PLAN.md` in this repo.
5. **Reserve files via Agent Mail.** Call `file_reservation_paths(project_key=<abs repo path>, agent_name=<your name>, paths=[<paths you will edit>], ttl_seconds=3600, exclusive=true)`. This prevents a parallel agent from clobbering your work in the same crate. Hold the reservation until you close the bead.
6. **Post your design plan as a comment.** `br comments add <bead-id> --body "Design: <one paragraph on approach, file layout, traits, testing plan>"`. This becomes the contract the reviewer holds you to.

### While you work

- **Commit small, push often.** Open the pull request in DRAFT mode as soon as the first happy-path test passes, not at the end. Reviewers prefer 10 small incremental diffs to one giant one.
- **Re-read the acceptance criteria daily.** Universal + type-specific gates are dense; one re-read per day prevents end-of-bead surprise gaps.
- **Post progress comments at meaningful milestones.** "Design locked", "happy path passing", "fuzz harness green", "ready for review". Six comments across a bead's lifetime is good; one comment is too few.
- **If you hit a blocker, post it.** Don't silently switch beads. The blocker becomes a follow-up bead with `discovered-from:<this-bead-id>`.

### Before you close

- **Walk every gate.** Universal section, all applicable type-specific sections, plus this bead's specific gates. Mark each as PASS or WAIVED. A waived gate needs a one-line rationale in the close reason.
- **Open the PR (out of DRAFT), get a review, merge.** CI must be green. At least one reviewer (human or AI) must approve.
- **Close with a complete reason.** `br close <bead-id> --reason "Closed by PR #N — <one-line summary>. Waived gates: <list with rationale>. Discovered follow-ups: <list of new bead IDs>."`
- **Release file reservations** via `release_file_reservations(...)` so the next agent can take adjacent work.

## Universal quality gates (apply to every bead, non-negotiable)

These hold for any bead to be closed. They are ADDITIONAL to the bead-specific gates above, not replacement. A bead with all bead-specific gates met but a single universal gate failed is NOT closed.

### Completeness — no stubs, no half-done work
- [ ] Zero `todo!()`, `unimplemented!()`, `panic!("not implemented")`, or `unreachable!()` left in production paths.
- [ ] No commented-out implementations, no `// TODO` comments left behind, no `XXX`/`FIXME` markers.
- [ ] No placeholder values (no `"REPLACE_ME"`, no `42` as a sentinel, no example secrets in env files).
- [ ] No silent stubs that return `Default::default()` or an empty result when the real implementation is missing.
- [ ] Every code path enumerated in the bead-specific acceptance criteria has been implemented AND exercised by at least one test.

### Tests — happy path + failure modes
- [ ] At least one happy-path test.
- [ ] At least three explicit failure-mode tests (invalid input, downstream failure, edge case).
- [ ] Unit tests cover the bead's public API surface.
- [ ] Integration tests where the bead touches I/O, another bead, or an external service.
- [ ] Property-based tests where the bead has algebraic or canonicalization invariants (use `proptest`).
- [ ] Test coverage on the bead's primary crate / module ≥ 90% line coverage.

### Determinism and reproducibility
- [ ] Where the bead produces output that is signed, hashed, audited, or compared elsewhere (canonicalization, fingerprint, PDF render, ABI surface, signatures), output is byte-identical across two runs on the same input on the same platform.
- [ ] Where the bead targets cross-platform parity, output is byte-identical across Linux x86_64 and macOS aarch64 (Windows x86_64 best-effort).

### Logging, tracing, errors
- [ ] Structured logs using the `tracing` crate. Logs carry `trace_id`, `tenant_id` (where applicable), and `bead_id` (for diagnostic correlation).
- [ ] No `println!` / `eprintln!` left in library code.
- [ ] All errors are typed (`thiserror` or equivalent). User-facing errors include a remediation hint.
- [ ] Library code does NOT panic on user input. Internal invariant violations may use `debug_assert!`.

### Documentation
- [ ] Every public item carries rustdoc. `cargo doc --workspace --no-deps` compiles without warnings.
- [ ] At least one rustdoc example per public function or trait method that actually compiles (verified by `cargo test --doc`).
- [ ] The docs site (T-113) has a corresponding page added or updated where the bead introduces a user-facing capability.
- [ ] The changelog / release notes draft updated.

### Continuous integration gates
- [ ] `cargo build --workspace --all-targets` passes.
- [ ] `cargo test --workspace` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo fmt --check` passes.
- [ ] T-007 (performance regression budget) does not regress.
- [ ] T-008 (fuzz continuous integration) does not regress.
- [ ] T-058 (visual regression for PDFs) does not regress when the bead touches rendering.
- [ ] Bead's parity gate (where it declares one) passes (e.g., T-031 requires 99.9% rule parity vs JVM oracle).

### Bead workflow + multi-agent coordination
- [ ] Agent claimed the bead by `br update <id> --status=in_progress` BEFORE writing code.
- [ ] Agent used Agent Mail file reservations (`file_reservation_paths`) for any directory it edits exclusively, to avoid clobbering other agents working in parallel.
- [ ] Agent posted at least one progress update on the bead at a meaningful milestone (e.g. "design locked", "happy path passing", "fixtures added") via `br comments add`.
- [ ] Agent closed the bead via `br close <id> --reason "..."` with a reason that includes a link to the merged pull request and a one-line summary of what shipped.
- [ ] Agent released its Agent Mail file reservations after close.

### Self-review pass
- [ ] Final diff reviewed end-to-end before pull request opens.
- [ ] Debug `println!`, `dbg!`, scratch files, and test-only configuration removed.
- [ ] Imports tidied; dead code removed; `#[allow(dead_code)]` annotations justified or removed.
- [ ] Public-facing copy (CLI help text, error messages, docs) read aloud for clarity.

### Acceptance: definition of "done"

The bead is closed only when:
1. All bead-specific gates pass.
2. All universal gates above pass.
3. All **applicable** type-specific gates below pass. A gate is "applicable" when its precondition fits the bead's scope; gates that are out of scope are recorded as N/A in the close reason with a one-line rationale.
4. A pull request is open with the bead identifier in its title or body, CI is green, and at least one reviewer (human or AI) has approved.
5. The bead is `br close`d with a reason that quotes the merged PR URL and lists any waived type-specific gates with rationale.

If ANY universal gate is missing, the bead is `in_progress`, not closed. Type-specific gates that are genuinely N/A may be waived — but the waiver is explicit, not implicit.

## Type-specific quality gates

{type_specific_gates}

## Implementation notes

{notes}

## Out of scope

{out_of_scope}

## References

{references}
"""


@dataclass
class Bead:
    tid: str
    slug: str
    title: str
    type: str = "task"  # task | feature | epic | bug | chore
    priority: int = 1
    deps: List[str] = field(default_factory=list)
    labels: List[str] = field(default_factory=list)
    background: str = ""
    goal: str = ""
    acceptance: str = ""
    notes: str = ""
    out_of_scope: str = "Anything not listed under Acceptance criteria. Future enhancements live in separate beads."
    references: str = ""

    def render_body(self) -> str:
        # Epics need a "## Success Criteria" header per br lint convention; prepend to acceptance.
        acceptance = self.acceptance.strip()
        if self.type == "epic":
            acceptance = (
                "## Success Criteria\n\n"
                "(This is an epic — success means every sub-bead listed under the acceptance gates below has been closed AND no downstream bead has had to re-implement the epic's deliverable.)\n\n"
                + acceptance
            )
        return BODY_TEMPLATE.format(
            background=self.background.strip(),
            goal=self.goal.strip(),
            acceptance=acceptance,
            type_specific_gates=compute_type_specific_gates(self.labels).strip(),
            notes=self.notes.strip(),
            out_of_scope=self.out_of_scope.strip(),
            references=self.references.strip(),
        )


# ────────────────────────────────────────────────────────────────────────────
# Helpers for compact bead authorship.
# ────────────────────────────────────────────────────────────────────────────

DEFAULT_ACCEPTANCE_FOR_TASK = """- [ ] Implementation matches the specification in the referenced PLAN.md section.
- [ ] Unit tests cover the public API surface with at least 90% line coverage.
- [ ] Integration tests exercise the happy path AND at least three failure modes.
- [ ] `cargo build --workspace --all-targets` and `cargo test --workspace` pass on Linux x86_64 and macOS aarch64.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] Public types and functions carry rustdoc; the rustdoc compiles without warnings.
- [ ] Code review by at least one human or AI reviewer (recorded in PR thread)."""

DEFAULT_ACCEPTANCE_FOR_DOC_TASK = """- [ ] Documentation matches the specification in the referenced PLAN.md section.
- [ ] Markdown renders correctly in the docs site (Nextra build passes).
- [ ] Code samples in the documentation are tested in CI."""

DEFAULT_OUT_OF_SCOPE = (
    "Anything not explicitly listed under Acceptance criteria. Future enhancements live in separate beads."
)


def lbl(*items: str) -> List[str]:
    return list(items)


# ────────────────────────────────────────────────────────────────────────────
# Type-specific gates by label.
# Each bead picks up zero or more sets of gates based on its labels.
# ────────────────────────────────────────────────────────────────────────────

TYPE_GATES = {
    "country": """### Country crate gates (label `country`)
- [ ] The country's Phase 2.5 feasibility manifest is signed, merged, and referenced as a dependency of this bead.
- [ ] The country's fixture set (≥ 5 valid + ≥ 5 invalid) lives in `conformance-corpus/licensed-real/<country>/` or `conformance-corpus/synthetic/<country>/`; every fixture has expected validation outcome metadata.
- [ ] The country's cassette set (≥ 1 success + ≥ 2 canonical errors) lives in `conformance-corpus/cassettes/<country>/`; cassettes are scrubbed of personal data.
- [ ] The archetype trait (`async-clearance` / `cryptographic` / `peppol-overlay`) is implemented via the right trait import — the crate does NOT define its own clearance / signing / overlay logic.
- [ ] The signing adapter from Track 6 (T-083b1..T-083b5) is consumed via dependency, NOT re-implemented in this crate.
- [ ] The capability matrix in PLAN.md §3.4 is updated to reflect the country's new maturity cells.
- [ ] `bv --robot-next` confirms this bead's closure unblocks the documented downstream beads.""",

    "manifest": """### Country manifest gates (label `manifest`)
- [ ] Manifest written in the schema defined in `data/country-manifests/SCHEMA.md`.
- [ ] All required fields populated: source URLs, retrieval dates, sandbox availability, certificate requirements, fiscal-representative requirements, validator backend, partner options, go/no-go.
- [ ] Manifest signed with sigstore (or minisign for offline cases). Signature verified by CI.
- [ ] Initial fixture set (≥ 5 valid + ≥ 5 invalid) committed.
- [ ] Baseline sandbox cassettes (≥ 1 success + ≥ 1 canonical error) recorded where a sandbox exists; documented as "no sandbox" with rationale otherwise.
- [ ] Manifest URL added to T-006 source-watch bot's monitored list.""",

    "sdk": """### SDK gates (label `sdk`)
- [ ] Bindings expose every C ABI entry point documented in T-023 `ABI.md`.
- [ ] Package published to the language's canonical registry (`npm`, `PyPI`, `Maven Central`, `NuGet`, `crates.io`, or Go modules) on release tag.
- [ ] Versioning follows semver, mirroring the engine's version.
- [ ] Cross-platform binary distribution tested: at minimum Linux x86_64 + macOS aarch64 + Windows x86_64 where the runtime supports it.
- [ ] Cross-language golden fixture suite (`conformance-corpus/golden/`) passes against this binding on every PR.
- [ ] Migration guide added if any public API changes vs the previous release.
- [ ] Quickstart example in the docs site runs verbatim from a clean clone.""",

    "rendering": """### Rendering gates (label `rendering`)
- [ ] Pinned fonts are subsetted to ASCII + Latin-1 Supplement + Latin Extended-A + EUR sign; no system fonts referenced.
- [ ] PDF/A-3 conformance verified by `validator-verapdf` sidecar for every output profile.
- [ ] T-058 visual regression baseline updated only with explicit human sign-off in the PR.
- [ ] Determinism: two renders on Linux x86_64 produce identical bytes; two renders on macOS aarch64 produce identical bytes; cross-platform byte-equality where the bead claims it.
- [ ] If touching `crates/render-pdf-postproc`: veraPDF passes for ALL six Factur-X / ZUGFeRD profiles on the fixture set.""",

    "validators": """### Validator gates (label `validators`)
- [ ] Differential parity vs the named JVM oracle: ≥ 99.9% rule-result match on the conformance corpus. Mismatches are catalogued, classified (oracle bug vs. our bug), and tracked.
- [ ] Per-rule unit tests: at least one positive and one negative case per rule.
- [ ] Each rule includes a `citation` referencing the rule pack source (EN 16931 PDF section, KoSIT XSL, Peppol Schematron, etc.).
- [ ] Rule pack version pinned via T-017 manifest; PR fails if pack manifest is unsigned or stale.""",

    "managed": """### Hosted-service gates (label `managed`)
- [ ] Service deployed via Track 13 artifacts (docker-compose, Helm, or Terraform); no hand-rolled deploy scripts.
- [ ] Service-level objectives defined and recorded in `services/<name>/SLO.md`: latency p95, error rate, throughput targets.
- [ ] Per-operation OpenTelemetry traces emitted; metrics flow to T-136 dashboards.
- [ ] Secrets managed via the KMS path from T-131; no secrets in env files, no secrets in code, no secrets in container images.
- [ ] Health endpoint (`/healthz`) returns 200 only when downstream dependencies are healthy.
- [ ] Runbook for common incidents in `services/<name>/RUNBOOK.md`.""",

    "bridge": """### Billing-platform bridge gates (label `bridge`)
- [ ] End-to-end test against the host platform's sandbox account (Stripe test mode, Lago sandbox, etc.).
- [ ] Webhook signature verification (HMAC) wired up.
- [ ] Idempotency on retries: replaying a webhook with the same event ID is a no-op.
- [ ] LossinessLedger entries enumerated for fields the bridge cannot round-trip cleanly.
- [ ] Documentation includes step-by-step setup for a customer to point their billing platform at our bridge.""",

    "connector": """### ERP-connector gates (label `connector`)
- [ ] Packaged for the host ERP marketplace (Odoo App Store, Microsoft AppSource, SAP Store, etc.).
- [ ] Marketplace submission accepted (or accepted-pending-review status documented).
- [ ] At least one screencast / asciinema recording showing a clean install + first invoice issued via the connector.
- [ ] Documented uninstall path: removing the connector does not corrupt the host ERP's data.""",

    "demo": """### Demo-app gates (label `demo`)
- [ ] Clean-clone walkthrough verified: a developer cloning the repo and following the README issues + validates a German XRechnung in under 5 minutes from cold start.
- [ ] README has copy-paste setup commands that work without modification.
- [ ] CI runs the demo end-to-end on every PR.
- [ ] At least three example invoices are issued, validated, and persisted in the demo's storage layer.""",

    "archetype": """### Archetype gates (label `archetype`)
- [ ] The archetype trait + companion crate are documented as the canonical pattern for every later country crate following this lineage.
- [ ] At least one downstream country crate consumes the trait (Wave 1 reference impl).
- [ ] The cassette taxonomy specific to this archetype is documented (what scenarios every country following the archetype must record).
- [ ] The per-archetype testing contract is documented (`tests/archetype/<name>.md`).""",

    "wasm": """### WebAssembly gates (label `wasm`)
- [ ] WASM artifact size measured and tracked; bundle stays under the budget documented in PLAN.md §2.1 (default 5 MB with feature set `de,fr,it,peppol`).
- [ ] Feature flags compile cleanly: at least 3 distinct feature combinations build and pass tests.
- [ ] Cold-start latency on Cloudflare Workers under the budget in PLAN.md (best-effort; benchmark recorded if exceeded).
- [ ] External-backend calls (`jvm:*`, `cli:*`, `rest:official`, `partner`) return `RequiresExternalBackend` errors with remediation text — they never panic, never silently downgrade.""",

    "evidence": """### Evidence-bundle gates (label `evidence`)
- [ ] Bundle layout matches PLAN.md §4.7 exactly: every directory and file present where required, absent where forbidden.
- [ ] DSSE / JWS signature over the manifest verifies against the documented public key flow.
- [ ] BLAKE3 hashes for every content-addressed entry recomputed at verify-time and matched against the manifest.
- [ ] `.ikb` packed form: `tar -tvf` lists every entry in deterministic order; mtimes, uid, gid normalised; archive bytes identical across two pack runs.
- [ ] `invoicekit verify` rejects every kind of tampering (added entry, removed entry, mutated entry, mutated manifest, mutated signature) with a typed error pointing to the exact violation.""",

    "cassette": """### Cassette / sandbox gates (label `cassette`)
- [ ] Recorder produces deterministic cassettes: recording the same interaction twice yields byte-identical files.
- [ ] Scrubber removes personally identifying data; a CI rule scans cassettes for unscrubbed PII patterns (VAT IDs in unexpected positions, IBANs, etc.) and fails the build.
- [ ] Matcher routes by method + path + body fingerprint; collisions surface as test failures, not silent reuse.
- [ ] Scenario metadata schema followed (`scenario.json` per cassette directory).""",

    "ci": """### Continuous-integration gates (label `ci`)
- [ ] CI job runs on every pull request and on `main` after merge.
- [ ] CI artifacts (test reports, benchmark JSON, coverage reports) uploaded to a deterministic path so other CI jobs and humans can fetch them.
- [ ] Failure modes documented: when this CI job fails, the README points the developer at the fix.""",

    "lsp": """### Language-server-protocol gates (label `lsp`)
- [ ] Hover on every BT-* / BG-* term returns the corresponding EN 16931 explanatory text within 100 ms (p95).
- [ ] Diagnostics surface on save AND on type with debounce.
- [ ] Code actions implemented for the most common rule violations.""",
}


def compute_type_specific_gates(labels: List[str]) -> str:
    """Compose the type-specific gates section from a bead's labels."""
    parts: List[str] = []
    seen = set()
    for label in labels:
        if label in TYPE_GATES and label not in seen:
            parts.append(TYPE_GATES[label])
            seen.add(label)
    if not parts:
        parts.append("(No type-specific gates apply to this bead beyond the universal gates above.)")
    return "\n\n".join(parts)


# ────────────────────────────────────────────────────────────────────────────
# Bead definitions.
#
# To keep this file readable, beads are defined per track in helper functions.
# Each function returns a list of Bead objects.
#
# Bead IDs (tid) follow the PLAN.md T-* convention. Slugs are derived
# deterministically as `{tid-lower}-{short-name}`.
# ────────────────────────────────────────────────────────────────────────────


def track0_foundation() -> List[Bead]:
    return [
        Bead(
            tid="T-001",
            slug="t-001-cargo-workspace",
            title="Initialize Cargo workspace, continuous integration, code-of-conduct, contributing guide, security policy",
            type="task",
            priority=0,
            deps=[],
            labels=lbl("track-0", "foundation", "critical-path", "p0"),
            background=(
                "InvoiceKit ships as a Rust workspace with multiple crates plus native bindings and a WebAssembly artifact. "
                "Every downstream task depends on this scaffolding being in place. This is the first bead. "
                "See PLAN.md §4.1 for the target crate layout (roughly 75 directories under crates/, bindings/, services/, examples/, deploy/, bridges/, connectors/)."
            ),
            goal="A working Rust workspace with continuous integration, signed releases, security policy, contribution guide, and code of conduct, ready for every downstream task to land.",
            acceptance=(
                "- [ ] `Cargo.toml` workspace file lists all crates from PLAN.md §4.1.\n"
                "- [ ] `cargo build --workspace` succeeds on a clean clone.\n"
                "- [ ] CI runs on every pull request via GitHub Actions (build + test + clippy + rustfmt + audit).\n"
                "- [ ] `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md`, `LICENSE` (Apache 2.0) all present at repo root.\n"
                "- [ ] Branch protection on `main`: require PR review, require CI green.\n"
                "- [ ] Repo has a `Makefile` or `justfile` with the canonical commands: `build`, `test`, `lint`, `fmt`, `audit`."
            ),
            notes=(
                "Use the standard Rust 2021 edition. Pin a stable Rust toolchain in `rust-toolchain.toml`. "
                "CI uses `actions-rs/toolchain` or `dtolnay/rust-toolchain`. "
                "For audit, use `cargo-audit` and `cargo-deny`."
            ),
            references="PLAN.md §4.1 (Crate layout); §6 Track 0.",
        ),
        Bead(
            tid="T-002",
            slug="t-002-license-sbom-signed-releases",
            title="License (Apache 2.0), signed releases, software bill of materials, dependency scanning",
            type="task",
            priority=0,
            deps=["T-001"],
            labels=lbl("track-0", "foundation", "security", "p0"),
            background=(
                "All downstream operational security work (T-133, SBOM generation, supply-chain hardening) depends on "
                "this initial pass having locked in: an Apache 2.0 license, signed releases, an SBOM pipeline, and "
                "dependency scanning. This bead also locks the security advisory process."
            ),
            goal="The repository is auditable for supply-chain integrity from day one.",
            acceptance=(
                "- [ ] All source files carry Apache 2.0 license headers (verified by `licensure` or equivalent in CI).\n"
                "- [ ] Release pipeline produces signed binaries (cosign or minisign) for every tag.\n"
                "- [ ] CI generates a CycloneDX SBOM on every release (`cargo cyclonedx`).\n"
                "- [ ] `cargo-deny` and `cargo-audit` run on every PR and fail on advisories.\n"
                "- [ ] `SECURITY.md` documents the responsible disclosure process and points to a security mailbox."
            ),
            notes="Use `cosign` for signing. Use `dependabot` or `renovate` for dependency updates.",
            references="PLAN.md §6 Track 0; §7 risks (security advisory process).",
        ),
        Bead(
            tid="T-005",
            slug="t-005-iso-27001-engagement",
            title="ISO 27001 readiness engagement starts (background, 6–12 months)",
            type="chore",
            priority=2,
            deps=[],
            labels=lbl("track-0", "compliance", "background"),
            background=(
                "ISO 27001 is the long pole for becoming our own Peppol Access Point (see PLAN.md §2.7, §4.8). "
                "It requires 6–12 months even with a consultancy, costs €15k–€40k initially plus annual surveillance. "
                "It is not technical and does not gate engineering, but it must START on day one so it can finish in parallel "
                "with the build push."
            ),
            goal="An ISO 27001 consultancy is engaged and the readiness assessment has started.",
            acceptance=(
                "- [ ] Consultancy contract signed.\n"
                "- [ ] Gap assessment completed.\n"
                "- [ ] Information Security Management System (ISMS) scope documented.\n"
                "- [ ] First quarterly review scheduled."
            ),
            notes="Non-engineering work. Owner is the principal, not an agent.",
            references="PLAN.md §2.7, §4.8, §6 Track 0; risk row 'ISO 27001 process is the long pole'.",
        ),
        Bead(
            tid="T-006",
            slug="t-006-source-watch-bot",
            title="Compliance source-watch bot",
            type="feature",
            priority=1,
            deps=["T-001"],
            labels=lbl("track-0", "foundation", "compliance", "automation"),
            background=(
                "Regulatory sources update constantly (KoSIT, Agenzia delle Entrate, KSeF gov.pl, ZATCA, IMDA, ATO, etc.). "
                "If we treat tracking them as a manual chore, the rule packs go stale and customers get burned. "
                "The source-watch bot makes this a product capability: it monitors authoritative sources, opens issues "
                "on changes, and is the upstream of T-006a (capabilities) and T-074c (sandbox drift canary)."
            ),
            goal="An automated bot that monitors official e-invoicing sources and opens beads/issues when rule packs need updating.",
            acceptance=(
                "- [ ] Bot pulls from a configurable list of source URLs daily (initial seed: KoSIT, Agenzia delle Entrate, KSeF, ZATCA, IRP, IMDA, ATO).\n"
                "- [ ] For each detected change, opens a structured GitHub issue or bead with the diff and proposed action.\n"
                "- [ ] Source registry is signed (cf. T-018) so changes are auditable.\n"
                "- [ ] Bot is deployed as a scheduled GitHub Action or systemd timer.\n"
                "- [ ] End-to-end test: seed a mock source change; verify the bot opens an issue within 24h."
            ),
            notes="Live in `tools/source-watch-bot/`. Initial source list lives in `data/sources/*.toml`.",
            references="PLAN.md §6 Track 0 T-006; §7 risk row 'Rule drift maintenance'.",
        ),
        Bead(
            tid="T-006a",
            slug="t-006a-capabilities-spec",
            title="`invoicekit capabilities` complete specification",
            type="task",
            priority=1,
            deps=["T-006"],
            labels=lbl("track-0", "foundation", "capabilities", "cli"),
            background=(
                "Customers need a single answer to: 'for country X, route Y, on date Z, what formats are required, what "
                "rule packs apply, what signing is needed, what gateway delivers it, and what archive is required?'. "
                "This is the `invoicekit capabilities` command. The data model is a schema with versioned, dated entries; "
                "the output formats are both human and JSON; the source confidence rules distinguish official sources, "
                "partner sources, and community contributions."
            ),
            goal="A complete data model + CLI for `invoicekit capabilities` queries, sourced from signed manifests produced by T-006.",
            acceptance=(
                "- [ ] Capability schema (per country / profile / date / route direction / source confidence) defined in JSON Schema.\n"
                "- [ ] Stale-data and auto-downgrade semantics documented and tested.\n"
                "- [ ] CLI: `invoicekit capabilities --from=DE --to=FR --date=2027-01-01 --scenario=B2B` returns structured JSON and pretty-printed human output.\n"
                "- [ ] Integration with source-watch manifests verified end-to-end.\n"
                "- [ ] Unit tests cover edge cases: missing source, stale source, ambiguous date, conflicting overlays."
            ),
            notes="Lives in `crates/cli` plus `crates/capabilities`.",
            references="PLAN.md §5.3 CLI; §6 Track 0 T-006a.",
        ),
        Bead(
            tid="T-007",
            slug="t-007-perf-regression-budget",
            title="Performance regression budget in continuous integration",
            type="task",
            priority=1,
            deps=["T-001"],
            labels=lbl("track-0", "ci", "performance", "p1"),
            background=(
                "Without an automated performance budget, the engine slowly rots: a 5% slowdown per quarter compounds "
                "into a 20% regression in a year. The performance regression budget catches this on every PR. "
                "Tracked operations: validate, render, canonicalize, transmit-enqueue, fingerprint, IR round-trip."
            ),
            goal="Every pull request runs the benchmark suite; if any tracked operation regresses more than 10% versus the rolling 30-day median, the build fails.",
            acceptance=(
                "- [ ] Benchmarks use `criterion` for Rust and `vitest bench` for TypeScript.\n"
                "- [ ] CI publishes benchmark results to `benchmark.invoicekit.org` (initially a GitHub Pages site).\n"
                "- [ ] PR comment surfaces the diff vs. baseline for each tracked operation.\n"
                "- [ ] Threshold (default 10%) is configurable per operation.\n"
                "- [ ] False-positive test: known-good PR does not fail."
            ),
            notes="Implementation guidance: use `criterion-compare` for diffs and `criterion-table` for output.",
            references="PLAN.md §6 Track 0 T-007.",
        ),
        Bead(
            tid="T-008",
            slug="t-008-fuzz-ci",
            title="Fuzz continuous integration",
            type="task",
            priority=1,
            deps=["T-001"],
            labels=lbl("track-0", "ci", "security", "p1"),
            background=(
                "Parsers, the PDF embedder, and the canonicalizer are attack surface. Without fuzz CI, a malformed "
                "input can crash the engine or corrupt output. Crashes block merge; coverage regressions block merge."
            ),
            goal="Every pull request runs five minutes of cargo-fuzz against the XML parser, JSON parser, PDF embedder, and canonicalizer.",
            acceptance=(
                "- [ ] `cargo-fuzz` targets exist for: XML parser, JSON parser, PDF embedder, canonicalizer.\n"
                "- [ ] CI runs each target for 5 minutes per PR; crashes fail the build.\n"
                "- [ ] Coverage regression also fails the build (libFuzzer's coverage feedback).\n"
                "- [ ] Corpus accumulates in `conformance-corpus/fuzz/` and is committed.\n"
                "- [ ] Crash reproductions are saved as `tests/fuzz_crashes/{name}.input` and added as regression tests."
            ),
            notes="Use `cargo-fuzz` + `honggfuzz` (the former for libFuzzer integration, the latter for additional sanitizer combos).",
            references="PLAN.md §6 Track 0 T-008.",
        ),
    ]


def track1_engine_primitives() -> List[Bead]:
    notes_engine = "All crates live under `crates/`. Source of truth is Rust types; bindings are generated from JSON Schema in T-011."
    return [
        Bead(
            tid="T-010",
            slug="t-010-ir-layered-invoice-model",
            title="Layered invoice model in Rust (CommercialDocument, ProfileView, JurisdictionExtension, LossinessLedger)",
            type="feature",
            priority=0,
            deps=["T-001"],
            labels=lbl("track-1", "foundation", "critical-path", "ir", "p0"),
            background=(
                "The invoice data model is the load-bearing decision of the project. Layered: CommercialDocument is the "
                "jurisdiction-agnostic core; ProfileView is the projection onto a standard (EN 16931, Peppol BIS, etc.); "
                "JurisdictionExtension is polymorphic URN-keyed extension data (NOT a hardcoded struct of country fields); "
                "LossinessLedger is the required output of every projection. See PLAN.md §2.2, §4.3."
            ),
            goal="The four layered types exist as `rust_decimal`-backed Rust structs in `crates/ir`, with full unit-test coverage on construction, validation, and round-trip into JSON.",
            acceptance=(
                "- [ ] `CommercialDocument` carries all fields enumerated in PLAN.md §4.3 (id, document_type, issue_date, supplier, customer, lines, monetary_total, etc.).\n"
                "- [ ] `JurisdictionExtension` is polymorphic: `{urn: String, payload: serde_json::Value}` with per-URN schema lookup at validate-time.\n"
                "- [ ] `LossinessLedger` is a structured type with fields preserved/lost lists.\n"
                "- [ ] No hardcoded country structs in the core type — extension data is added by per-country crates at load time via a registry.\n"
                "- [ ] Unit tests construct each type from synthetic fixtures; round-trip into serde_json::Value succeeds and equals the input.\n"
                "- [ ] `cargo test -p invoicekit-ir` passes."
            ),
            notes=notes_engine + " The polymorphic extension pattern is critical — see PLAN.md §4.3 'extension layer'.",
            references="PLAN.md §2.2 layered invoice model; §4.3 the invoice data model.",
        ),
        Bead(
            tid="T-011",
            slug="t-011-json-schema-generation",
            title="Public JSON Schema generation from Rust types",
            type="task",
            priority=1,
            deps=["T-010"],
            labels=lbl("track-1", "ir", "schema"),
            background=(
                "Customer integrations and our own typed SDKs depend on a public JSON Schema. The schema is generated from the "
                "Rust source of truth (T-010) via `schemars` or equivalent."
            ),
            goal="A canonical JSON Schema 2020-12 file is generated from Rust types and committed under `schemas/invoicekit-ir-v1.json`.",
            acceptance=(
                "- [ ] `cargo run --bin gen-schema` produces `schemas/invoicekit-ir-v1.json`.\n"
                "- [ ] Schema is committed; CI fails on drift between Rust types and committed schema.\n"
                "- [ ] Schema validates the example invoices in `conformance-corpus/synthetic/`.\n"
                "- [ ] Schema is referenced from the website and documentation site."
            ),
            notes="Use `schemars` crate. Output goes to `schemas/`.",
            references="PLAN.md §4.3.",
        ),
        Bead(
            tid="T-012",
            slug="t-012-typescript-type-generation",
            title="TypeScript type generation from JSON Schema",
            type="task",
            priority=1,
            deps=["T-011"],
            labels=lbl("track-1", "ir", "typescript", "bindings"),
            background="Downstream TypeScript SDK depends on these types. Generated, never hand-written.",
            goal="`@invoicekit/types` npm package contains generated TypeScript types matching the JSON Schema.",
            acceptance=(
                "- [ ] `bunx tsc --noEmit` passes on the generated types.\n"
                "- [ ] Generated types match the rust source of truth byte-for-byte (verified via JSON Schema → TS → JSON Schema round-trip).\n"
                "- [ ] Package publishes to npm under `@invoicekit/types` on release."
            ),
            notes="Use `json-schema-to-typescript` or equivalent.",
            references="PLAN.md §4.3, §5.2.",
        ),
        Bead(
            tid="T-013",
            slug="t-013-gobl-adapter",
            title="`invopop/gobl` bidirectional adapter",
            type="task",
            priority=1,
            deps=["T-010"],
            labels=lbl("track-1", "ir", "interop"),
            background=(
                "GOBL is the closest OSS neighbor (Apache 2.0, Go). Per §2.9, we interoperate with their JSON schema "
                "rather than reinvent it. A bidirectional adapter `gobl <-> CommercialDocument` lets customers move data freely."
            ),
            goal="A bidirectional adapter between InvoiceKit's CommercialDocument and GOBL JSON.",
            acceptance=(
                "- [ ] `to_gobl(doc: &CommercialDocument) -> serde_json::Value` produces GOBL-conformant JSON.\n"
                "- [ ] `from_gobl(json: &serde_json::Value) -> Result<CommercialDocument>` works for all GOBL document types we cover (invoice, credit_note, debit_note).\n"
                "- [ ] Round-trip tests: `gobl -> ir -> gobl` is byte-stable for all of GOBL's example invoices.\n"
                "- [ ] LossinessLedger is populated when fields cannot round-trip cleanly.\n"
                "- [ ] Tests cover at least 20 invoices from GOBL's own test corpus."
            ),
            notes="Lives in `crates/ir-adapters-gobl`.",
            references="PLAN.md §2.9 GOBL interop.",
        ),
        Bead(
            tid="T-014",
            slug="t-014-money-crate",
            title="`money` crate (`rust_decimal` based)",
            type="task",
            priority=0,
            deps=["T-001"],
            labels=lbl("track-1", "foundation", "critical-path", "p0"),
            background=(
                "Monetary values never use floating-point arithmetic. The `money` crate is the type that all other crates use "
                "for monetary boundary values."
            ),
            goal="A `Money` type backed by `rust_decimal::Decimal` with serde support, currency-code aware, with deterministic rounding policies.",
            acceptance=(
                "- [ ] `Money` is `Decimal + Iso4217Code` with serde via fixed-scale string representation.\n"
                "- [ ] Operations: add, subtract, multiply by scalar, allocate (Stripe-style banker's allocation), with overflow protection.\n"
                "- [ ] Property-based tests verify associativity, commutativity, and allocation invariants.\n"
                "- [ ] Rounding policies (half-up, banker's, half-even) selectable per operation.\n"
                "- [ ] `cargo test -p invoicekit-money` passes with 100% line coverage."
            ),
            notes="Use `rust_decimal` 1.x. Fork or wrap to expose only what we want.",
            references="PLAN.md §2.3 money/tax/codelists; §4.1 crate layout.",
        ),
        Bead(
            tid="T-015",
            slug="t-015-codelists-crate",
            title="`codelists` crate (signed, versioned, effective-dated)",
            type="task",
            priority=0,
            deps=["T-001"],
            labels=lbl("track-1", "foundation", "critical-path", "p0"),
            background=(
                "Code lists (ISO 3166, ISO 4217, UN/ECE units, VAT category codes, Peppol code lists, country-specific tax category codes) "
                "are versioned, effective-dated data — not constants. See §2.3."
            ),
            goal="A `codelists` crate that loads signed, dated code list snapshots and answers lookup queries with effective-date semantics.",
            acceptance=(
                "- [ ] Each code list has a signed manifest with version, effective-from, effective-to, source URL, retrieved-at.\n"
                "- [ ] API: `lookup(list, code, on_date)` returns `Option<Entry>`.\n"
                "- [ ] Initial seed includes: ISO 3166-1, ISO 3166-2, ISO 4217 (2024), UN/ECE recommendation 20 units, EN 16931 VAT category codes, Peppol code lists.\n"
                "- [ ] Update path documented; codelist updater (T-018) lands updates atomically.\n"
                "- [ ] Property test: lookup on any effective date returns the entry valid on that date."
            ),
            notes="Lives in `crates/codelists`. Data files in `data/codelists/*.toml`.",
            references="PLAN.md §2.3, §4.1.",
        ),
        Bead(
            tid="T-016",
            slug="t-016-tax-calculation-crate",
            title="`tax-calculation` crate (deterministic decimal arithmetic with formal trace)",
            type="task",
            priority=0,
            deps=["T-014", "T-015"],
            labels=lbl("track-1", "foundation", "critical-path", "tax", "p0"),
            background=(
                "Invoice arithmetic (line extension, allowances, charges, VAT category subtotals, payable amount, currency conversion) "
                "must be deterministic and traceable. A formal trace lets the validator (T-031) explain `BR-CO-*` errors with exact arithmetic."
            ),
            goal="A pure-function tax-arithmetic library with formal trace output for every computation.",
            acceptance=(
                "- [ ] Every operation (line extension, allowance/charge application, tax category subtotal, payable amount) "
                "produces both a result and a `Trace` enum entry.\n"
                "- [ ] Trace is enough to reconstruct the calculation byte-for-byte from input.\n"
                "- [ ] Property tests: associativity, commutativity, rounding consistency across runs.\n"
                "- [ ] Cross-platform determinism: same input produces byte-identical output on Linux x86_64, macOS aarch64, Windows x86_64."
            ),
            notes="Lives in `crates/tax-calculation`. Used by T-031 (validator), T-032a (explain-plan), T-080 (evidence bundle).",
            references="PLAN.md §2.3, §4.5 validation, §4.7 evidence bundle.",
        ),
        Bead(
            tid="T-017",
            slug="t-017-rulepack-crate",
            title="`rulepack` crate (signed manifest format, source registry)",
            type="task",
            priority=0,
            deps=["T-001"],
            labels=lbl("track-1", "foundation", "critical-path", "rulepack", "p0"),
            background=(
                "Rule packs are signed, versioned, effective-dated artifacts. Every validator (T-031, T-030 sidecars) and every "
                "country crate consumes rule packs through this crate. See §2.4."
            ),
            goal="A `rulepack` crate that loads, verifies, and serves signed rule pack manifests with effective-date queries.",
            acceptance=(
                "- [ ] Manifest schema includes: source URLs, retrieved-at timestamps, upstream version, effective date range, "
                "code list versions, raw upstream checksums, generated metadata, parity fixtures, known gaps.\n"
                "- [ ] Signed manifests (Sigstore or minisign) verified on load; unsigned packs refused in CI.\n"
                "- [ ] Effective-date query API: `pack_for(country, profile, on_date)` returns the right pack.\n"
                "- [ ] Initial registry includes EN 16931 (CEN), Peppol BIS 3.0 (OpenPeppol), XRechnung (KoSIT).\n"
                "- [ ] Continuous integration refuses unpinned rules."
            ),
            notes="Lives in `crates/rulepack`. Data in `rulepacks/`.",
            references="PLAN.md §2.4, §6 Track 1 T-017.",
        ),
        Bead(
            tid="T-018",
            slug="t-018-codelist-updater",
            title="Codelist updater with provenance checksums",
            type="task",
            priority=1,
            deps=["T-015", "T-017"],
            labels=lbl("track-1", "codelists", "automation"),
            background=(
                "Code lists drift. The updater pulls fresh data from authoritative sources, verifies checksums, and atomically "
                "swaps the code list version."
            ),
            goal="A CLI + scheduled job that updates code lists from authoritative sources with full provenance.",
            acceptance=(
                "- [ ] `invoicekit codelist-update --list=iso-4217` fetches, verifies, and writes the new snapshot.\n"
                "- [ ] Each update is signed and tagged with retrieved-at, source URL, expected checksum.\n"
                "- [ ] Continuous integration job runs nightly and opens a pull request on diff."
            ),
            notes="Reuses T-006 source-watch bot patterns.",
            references="PLAN.md §6 Track 1 T-018.",
        ),
        Bead(
            tid="T-019",
            slug="t-019-xml-canonicalization",
            title="XML canonicalization (C14N 1.1 plus invoice-specific overlay)",
            type="task",
            priority=0,
            deps=["T-010"],
            labels=lbl("track-1", "canonical", "critical-path", "p0"),
            background=(
                "Signed and audited operations need a byte-stable canonical XML output. XML C14N 1.1 covers the standard; an "
                "invoice-specific overlay normalizes namespace prefixes, attribute order, and ignorable whitespace."
            ),
            goal="A pure-Rust XML canonicalizer for our invoice XML outputs, producing byte-identical output across runs and platforms.",
            acceptance=(
                "- [ ] Implements XML Canonicalization 1.1 (W3C REC).\n"
                "- [ ] Invoice-specific overlay normalizes namespace prefixes (e.g. always `cac:` for UBL Aggregate Components).\n"
                "- [ ] Property tests: random valid XML input round-trips to byte-identical canonical output.\n"
                "- [ ] Cross-platform determinism test.\n"
                "- [ ] Performance: canonicalize a 1 MB invoice XML in under 50 ms (p95) on a baseline reference machine."
            ),
            notes="Build on `quick-xml` for parsing. No external XSLT engine.",
            references="PLAN.md §4.4, §2.10 evidence bundle.",
        ),
        Bead(
            tid="T-020",
            slug="t-020-json-canonicalization",
            title="JSON canonicalization (RFC 8785)",
            type="task",
            priority=0,
            deps=["T-010"],
            labels=lbl("track-1", "canonical", "critical-path", "p0"),
            background=(
                "Signed JSON forms of invoices need RFC 8785 (JSON Canonicalization Scheme) for byte-stable hashing."
            ),
            goal="A pure-Rust RFC 8785 implementation.",
            acceptance=(
                "- [ ] Implements RFC 8785 exactly.\n"
                "- [ ] Passes the RFC 8785 official test vectors.\n"
                "- [ ] Property tests on random valid JSON inputs.\n"
                "- [ ] Cross-platform determinism test."
            ),
            notes="If a maintained crate exists (`json-canonicalization-scheme` or similar), use it; otherwise implement.",
            references="PLAN.md §4.4.",
        ),
        Bead(
            tid="T-021",
            slug="t-021-property-tests-canonical",
            title="Property-based canonical JSON and XML tests against synthetic IR",
            type="task",
            priority=1,
            deps=["T-019", "T-020"],
            labels=lbl("track-1", "testing"),
            background="The canonicalizers must survive arbitrary valid IR inputs without crashing or producing variant output.",
            goal="A property-based test harness that exercises both canonicalizers on synthetic IR inputs.",
            acceptance=(
                "- [ ] Uses `proptest` for Rust.\n"
                "- [ ] Generates synthetic valid IR documents and asserts: canonicalize is idempotent (canon(canon(x)) == canon(x)).\n"
                "- [ ] Asserts canonicalize is deterministic across runs.\n"
                "- [ ] Runs 10,000 cases per PR in CI."
            ),
            notes="",
            references="PLAN.md §4.4.",
        ),
        Bead(
            tid="T-021a",
            slug="t-021a-roundtrip-tests-real-serializers",
            title="Real IR ↔ UBL/CII XML round-trip tests",
            type="task",
            priority=1,
            deps=["T-040", "T-041", "T-019", "T-020"],
            labels=lbl("track-1", "testing"),
            background="With real serializers from Track 3 in place, we can test that IR → XML → IR is lossless within EN 16931 semantics.",
            goal="Round-trip property tests that load real UBL/CII fixtures, parse to IR, re-serialize, canonicalize, and assert equality.",
            acceptance=(
                "- [ ] Tests cover at least 20 fixtures from the conformance corpus.\n"
                "- [ ] Lossiness ledger entries are validated against expected losses per fixture.\n"
                "- [ ] CI runs on every PR."
            ),
            notes="",
            references="PLAN.md §3.2, §4.4.",
        ),
        Bead(
            tid="T-022",
            slug="t-022-invoice-fingerprint",
            title="Deterministic invoice fingerprint (BLAKE3)",
            type="task",
            priority=0,
            deps=["T-010", "T-014", "T-015"],
            labels=lbl("track-1", "foundation", "critical-path", "reconcile", "p0"),
            background=(
                "The reconciliation engine (T-070..T-080) uses a deterministic content-derived ID to dedup invoices. "
                "Formula: `blake3(supplier_VAT || customer_VAT || issue_date || document_number || total_amount || currency)`."
            ),
            goal="A function `fingerprint(doc: &CommercialDocument) -> Blake3Hash` plus its test vectors.",
            acceptance=(
                "- [ ] Pure function (no I/O, no globals).\n"
                "- [ ] Property test: same input → same output across runs.\n"
                "- [ ] Negative test: changing any input field produces a different fingerprint.\n"
                "- [ ] Test vectors documented and committed."
            ),
            notes="Use the `blake3` crate.",
            references="PLAN.md §4.6 reconciliation engine; §6 T-022.",
        ),
        Bead(
            tid="T-023",
            slug="t-023-stable-engine-abi",
            title="Stable engine ABI contract + cross-language golden fixtures",
            type="feature",
            priority=0,
            deps=["T-010", "T-016"],
            labels=lbl("track-1", "foundation", "critical-path", "abi", "p0"),
            background=(
                "The engine ABI is the cross-language contract. Every native binding (Node, Python, Java, .NET, Go) and "
                "the WebAssembly artifact all consume it. Stability is non-negotiable; once published, the C ABI "
                "follows semver."
            ),
            goal="A documented, frozen C ABI for the engine, plus cross-language golden fixtures that test every binding for byte-equivalence.",
            acceptance=(
                "- [ ] C ABI documented in `crates/invoicekit-ffi/ABI.md`.\n"
                "- [ ] Header file `invoicekit.h` generated and committed.\n"
                "- [ ] Golden fixtures in `conformance-corpus/golden/` consumed by every binding's test suite.\n"
                "- [ ] Cross-language CI: each binding (Node, Python, Java, .NET, Go, WASM) runs the golden suite on every PR."
            ),
            notes="Use `cbindgen` to generate the header. Use opaque pointers + canonical-JSON byte streams across the boundary.",
            references="PLAN.md §2.1 dual delivery shapes; §4.1.",
        ),
        Bead(
            tid="T-024",
            slug="t-024-c-abi-surface",
            title="C ABI surface (`invoicekit-ffi`)",
            type="task",
            priority=0,
            deps=["T-023"],
            labels=lbl("track-1", "foundation", "abi", "ffi", "p0"),
            background="The C ABI implementation. Wraps engine operations behind extern \"C\" entry points.",
            goal="`crates/invoicekit-ffi` exposes the engine through a stable C ABI defined in T-023.",
            acceptance=(
                "- [ ] Every entry point in `ABI.md` is implemented.\n"
                "- [ ] No memory unsafety: Miri passes in CI on the ffi crate.\n"
                "- [ ] Smoke test: a tiny C program links and calls every entry point."
            ),
            notes="",
            references="PLAN.md §4.1.",
        ),
        Bead(
            tid="T-025",
            slug="t-025-wasm-artifact",
            title="WebAssembly artifact (`invoicekit-wasm`)",
            type="task",
            priority=1,
            deps=["T-023"],
            labels=lbl("track-1", "foundation", "wasm"),
            background=(
                "WebAssembly delivery for browser, Cloudflare Workers, Deno, Bun. Feature-flagged so customers compile only what they need."
            ),
            goal="A WebAssembly artifact buildable with `cargo build --features=...` selecting which countries/formats to include.",
            acceptance=(
                "- [ ] Feature flags work: `cargo build --features=de,fr,it,peppol --target=wasm32-unknown-unknown` builds a < 5 MB artifact.\n"
                "- [ ] Calls into the engine work from browser-side JavaScript via wasm-bindgen.\n"
                "- [ ] WASM tests run in headless Chrome via wasm-pack."
            ),
            notes="Use `wasm-bindgen`. Output published to npm as `@invoicekit/wasm`.",
            references="PLAN.md §2.1 feature-flagged WebAssembly builds.",
        ),
        Bead(
            tid="T-026",
            slug="t-026-schema-evolution-migration",
            title="Schema evolution + automatic IR forward migration",
            type="feature",
            priority=1,
            deps=["T-010"],
            labels=lbl("track-1", "ir", "migration"),
            background=(
                "When the IR major version bumps (v1 → v2), customer archives must upgrade in place without data loss. "
                "Idea-wizard top-5 pick. See PLAN.md §6 T-026."
            ),
            goal="A typed `migrate(invoice_vN) -> Result<invoice_vM, MigrationReport>` plus a CLI `invoicekit migrate-archive`.",
            acceptance=(
                "- [ ] `migrate()` is reversible where semantics allow.\n"
                "- [ ] `MigrationReport` enumerates fields that could not be migrated cleanly with remediation hints.\n"
                "- [ ] CI runs every migration over every prior version's fixture set on every PR.\n"
                "- [ ] `invoicekit migrate-archive --from-version=N --to-version=M` works on a directory of invoice files."
            ),
            notes="Lives in `crates/migration`.",
            references="PLAN.md §6 Track 1 T-026.",
        ),
    ]


def track2_reference_validator() -> List[Bead]:
    return [
        Bead(
            tid="T-030",
            slug="t-030-validator-sidecars",
            title="Validator sidecar protocol + per-domain workers (kosit, phive, saxon)",
            type="feature",
            priority=0,
            deps=["T-001", "T-032"],
            labels=lbl("track-2", "validators", "critical-path", "p0"),
            background=(
                "Reference validators run as per-domain JVM sidecars: validator-kosit, validator-phive, validator-saxon, plus "
                "validator-verapdf (T-052), validator-phase4 (T-092). Not a monolithic JVM. See PLAN.md §2.6, §4.5."
            ),
            goal="Three containerized JVM sidecars (kosit, phive, saxon) speaking a stable JSON-RPC contract.",
            acceptance=(
                "- [ ] Each sidecar runs as a separate Docker container.\n"
                "- [ ] JSON-RPC contract documented in `services/validator-rpc.md`.\n"
                "- [ ] Latency: validate a 1 MB XML in under 200 ms (p95) on each sidecar.\n"
                "- [ ] Smoke test: each sidecar produces correct output for a known good and known bad sample.\n"
                "- [ ] CI builds each container image on every PR."
            ),
            notes="Use Java 21 LTS. Containers built via Buildkit.",
            references="PLAN.md §2.6, §4.5.",
        ),
        Bead(
            tid="T-031",
            slug="t-031-en16931-rust-validator",
            title="EN 16931 hand-written Rust validator (~50 core rules)",
            type="feature",
            priority=0,
            deps=["T-010", "T-017", "T-030", "T-032"],
            labels=lbl("track-2", "validators", "critical-path", "en16931", "p0"),
            background=(
                "Hand-written Rust implementation of the EN 16931 core business rules (~50 rules). Pure-Rust validation "
                "ships with the engine; differential-tested against the JVM sidecars for parity."
            ),
            goal="Pure-Rust validator covers all ~50 EN 16931 core rules at 99.9% parity with the JVM oracle.",
            acceptance=(
                "- [ ] Every BR-* and BR-CO-* rule from EN 16931 implemented as a typed Rust function.\n"
                "- [ ] Differential test against `validator-kosit` and `validator-phive`: 99.9% parity on the conformance corpus.\n"
                "- [ ] Per-rule unit tests with at least one positive and one negative case each.\n"
                "- [ ] Performance: validate a 1 MB invoice in under 25 ms (p95) on a baseline reference machine."
            ),
            notes="Rules go in `crates/validate-ubl-cii/src/rules/`.",
            references="PLAN.md §4.5 validation.",
        ),
        Bead(
            tid="T-032",
            slug="t-032-validation-result-schema",
            title="Validation result schema (rule ID, severity, BT/BG term, location, fix, citation)",
            type="task",
            priority=0,
            deps=["T-010", "T-017"],
            labels=lbl("track-2", "validators", "critical-path", "p0"),
            background=(
                "Every validation result speaks the same language: rule ID, severity, business-term (BT-* / BG-*), JSON Pointer "
                "or XPath location, suggested fix, and citation. Plus an optional `trace` field for T-032a."
            ),
            goal="A typed `ValidationResult` struct + JSON Schema, used by every validator backend.",
            acceptance=(
                "- [ ] `ValidationResult` has fields: rule_id, severity, term, location, suggested_fix, citation, trace?.\n"
                "- [ ] JSON Schema is generated and committed.\n"
                "- [ ] All validator backends (rust-native, jvm:*, rest:official, partner, cli, none) produce this shape."
            ),
            notes="",
            references="PLAN.md §4.5.",
        ),
        Bead(
            tid="T-032a",
            slug="t-032a-validator-explain-plan",
            title="Validator explain-plan trace",
            type="feature",
            priority=1,
            deps=["T-031", "T-032"],
            labels=lbl("track-2", "validators", "dx"),
            background=(
                "Idea-wizard top-2 pick. For any validation result, emit a structured trace of every rule evaluated in order with "
                "{rule_id, evaluated_at_path, inputs, decision, citations}. Both machine-readable JSON and human-readable Markdown."
            ),
            goal="`invoicekit validate file.xml --explain` produces an explain-plan trace.",
            acceptance=(
                "- [ ] Trace shape documented; JSON Schema committed.\n"
                "- [ ] Markdown rendering produces a readable narrative.\n"
                "- [ ] Snapshot tests on at least 5 sample invoices."
            ),
            notes="Powers the language server hover, the docs site, and the customer support tooling.",
            references="PLAN.md §6 Track 2 T-032a.",
        ),
        Bead(
            tid="T-033",
            slug="t-033-browser-edge-capability-matrix",
            title="Browser/edge validator capability matrix",
            type="task",
            priority=1,
            deps=["T-030", "T-031"],
            labels=lbl("track-2", "wasm", "capabilities"),
            background=(
                "WebAssembly builds can include serializers but not necessarily validators that need external backends. The "
                "capability matrix makes this explicit at runtime: `serialize`, `local_validate`, `reference_validate`, "
                "`requires_service`, `requires_cli`, `unavailable_in_wasm`. External backends return `RequiresExternalBackend`, never panic."
            ),
            goal="A per-country/profile/date capability matrix reachable via API + CLI; WASM validators return typed errors when an external backend is required.",
            acceptance=(
                "- [ ] Matrix data model documented and tested.\n"
                "- [ ] `RequiresExternalBackend` error type implemented and returned everywhere.\n"
                "- [ ] WASM build never silently downgrades or panics.\n"
                "- [ ] CLI: `invoicekit capabilities --runtime=wasm` reports correctly."
            ),
            notes="",
            references="PLAN.md §2.1, §4.5.",
        ),
        Bead(
            tid="T-034",
            slug="t-034-time-travel-validation",
            title="Time-travel validation (date-pinned rule packs)",
            type="task",
            priority=1,
            deps=["T-017", "T-031"],
            labels=lbl("track-2", "validators", "compliance"),
            background="Auditor question: 'would this invoice have validated on the date it was issued?'. Answer requires date-pinned rule pack selection.",
            goal="`invoicekit validate --date=YYYY-MM-DD` selects the rule pack effective on that date.",
            acceptance=(
                "- [ ] CLI and library both honor `--date`.\n"
                "- [ ] Tests verify a known-bad invoice validates against a pre-rule-change date and fails on the post-change date.\n"
                "- [ ] Audit log records which rule pack was used."
            ),
            notes="",
            references="PLAN.md §2.4.",
        ),
        Bead(
            tid="T-035",
            slug="t-035-public-validator-web-ui",
            title="Public free validator web UI (dual mode)",
            type="feature",
            priority=1,
            deps=["T-030", "T-033"],
            labels=lbl("track-2", "trust", "marketing"),
            background=(
                "Free public validator at `validate.invoicekit.org`. Two explicit modes: local browser-only (privacy-first) and "
                "server-assisted reference (official-parity). UI surfaces which mode produced the result."
            ),
            goal="A public single-page web app for invoice validation, dual-mode, deployed to `validate.invoicekit.org`.",
            acceptance=(
                "- [ ] Local mode runs entirely in browser via WASM; nothing leaves the device.\n"
                "- [ ] Reference mode calls the JVM sidecar service; clearly labeled, no-retention by default.\n"
                "- [ ] UI shows mode + rule pack version + validator backend per result.\n"
                "- [ ] Deployed and reachable at `validate.invoicekit.org`.\n"
                "- [ ] Analytics: page visits + drop-off (no PII, no payload retention)."
            ),
            notes="Use Next.js. Static export for the local-mode page; serverless function for reference-mode calls.",
            references="PLAN.md §3.3 Phase 7 conformance and trust infrastructure.",
        ),
    ]


def track3_format_family_a() -> List[Bead]:
    """UBL, CII, Peppol BIS, Peppol PINT, Factur-X/ZUGFeRD, XRechnung."""
    common_refs = "PLAN.md §3.2 Family A; §4.5 validation; §6 Track 3."
    return [
        Bead("T-040", "t-040-ubl-2-1-parser-serializer", "Universal Business Language 2.1 parser and serializer",
             "feature", 0, ["T-010", "T-019"], lbl("track-3", "format", "ubl", "critical-path", "p0"),
             background="UBL 2.1 is the European invoice XML standard underlying Peppol BIS, XRechnung, and many national variants. Parser and serializer live in `crates/format-ubl`.",
             goal="A round-trip-stable UBL 2.1 parser and serializer for Invoice + CreditNote document types.",
             acceptance=(
                "- [ ] Parser handles all UBL 2.1 Invoice and CreditNote elements per OASIS UBL 2.1 specification.\n"
                "- [ ] Serializer emits canonical UBL XML; output passes T-019 canonicalization byte-identically across runs.\n"
                "- [ ] Round-trip property test: parse → serialize → parse produces equal IRs for 50 fixture invoices.\n"
                "- [ ] Schema validation passes against OASIS XSD for all serialized outputs.\n"
                "- [ ] Performance: parse 1 MB invoice under 100 ms (p95) on a baseline reference machine."),
             notes="Use `quick-xml` for low-level parsing. Avoid `xmltree` (slow). Define IR mapping in `format-ubl/src/mapping.rs`.",
             references=common_refs),
        Bead("T-041", "t-041-cii-parser-serializer", "Cross Industry Invoice (CII) parser and serializer",
             "feature", 0, ["T-010", "T-019"], lbl("track-3", "format", "cii", "critical-path", "p0"),
             background="UN/CEFACT Cross Industry Invoice is the XML schema used by Factur-X / ZUGFeRD. Lives in `crates/format-cii`.",
             goal="A round-trip-stable CII parser and serializer.",
             acceptance=(
                "- [ ] Parser handles CII D16B namespace fully.\n"
                "- [ ] Serializer emits canonical CII XML.\n"
                "- [ ] Round-trip property tests on at least 50 CII fixtures pass.\n"
                "- [ ] Performance: parse 1 MB CII XML under 100 ms (p95)."),
             references=common_refs),
        Bead("T-042", "t-042-peppol-bis-3-0-projection", "Peppol BIS 3.0 projection",
             "feature", 1, ["T-040"], lbl("track-3", "format", "peppol-bis"),
             background="Peppol BIS Billing 3.0 is the cross-border European e-invoicing profile. It is a CIUS of UBL Invoice with specific business rules.",
             goal="A `to_peppol_bis_3_0(doc: &CommercialDocument) -> UblInvoice` projection plus the validate-rules associated with Peppol BIS.",
             acceptance=(
                "- [ ] Projection produces a Peppol BIS-conformant UBL invoice for all valid IRs.\n"
                "- [ ] All Peppol BIS Schematron rules pass for the projection's output (validated by `validator-phive`).\n"
                "- [ ] At least 20 cross-border fixtures pass end-to-end."),
             references=common_refs),
        Bead("T-043", "t-043-peppol-pint-projection", "Peppol PINT international projection",
             "feature", 1, ["T-040"], lbl("track-3", "format", "peppol-pint"),
             background="Peppol PINT is the international (non-EU) Peppol variant for AU, NZ, SG, JP, AE, and more.",
             goal="A `to_peppol_pint(doc, country)` projection.",
             acceptance=(
                "- [ ] Projection produces a PINT-conformant UBL invoice for AU, NZ, SG, JP, AE.\n"
                "- [ ] At least 10 fixtures pass."),
             references=common_refs),
        Bead("T-044", "t-044-factur-x-zugferd-all-profiles", "Factur-X / ZUGFeRD all six profiles",
             "feature", 1, ["T-040", "T-041"], lbl("track-3", "format", "factur-x", "zugferd"),
             background="Factur-X/ZUGFeRD has six profiles: MINIMUM, BASIC WL, BASIC, EN 16931, EXTENDED, XRECHNUNG. Each is a subset/superset of EN 16931 in CII form.",
             goal="All six profiles serialize and validate correctly.",
             acceptance=(
                "- [ ] Each of the six profiles has at least one valid and one invalid fixture.\n"
                "- [ ] Profile downgrade and upgrade conversions (e.g. EN 16931 → BASIC) emit a populated LossinessLedger.\n"
                "- [ ] Validation per profile against the official ZUGFeRD validator (T-052 veraPDF setup helps here)."),
             references=common_refs),
        Bead("T-045", "t-045-xrechnung-3-x-projection", "German XRechnung 3.x projection",
             "feature", 1, ["T-040"], lbl("track-3", "format", "xrechnung"),
             background="XRechnung 3.x is the German B2G/B2B Schematron-rules CIUS of EN 16931 in UBL form.",
             goal="A `to_xrechnung_3_x` projection that validates against KoSIT.",
             acceptance=(
                "- [ ] Projection passes `validator-kosit` for at least 30 valid fixtures.\n"
                "- [ ] LeitwegID field handling tested for B2G scenarios.\n"
                "- [ ] Schematron rules BR-DE-* covered."),
             references=common_refs),
        Bead("T-046", "t-046-lossiness-ledger-generator", "Lossiness ledger generator",
             "task", 1, ["T-040", "T-041", "T-042", "T-043", "T-044", "T-045"], lbl("track-3", "format", "lossiness"),
             background="Cross-format conversions are lossy by definition. The lossiness ledger is the structured output that tells callers what was lost.",
             goal="Every projection produces a populated `LossinessLedger` listing preserved and lost fields.",
             acceptance=(
                "- [ ] LossinessLedger schema documented and tested.\n"
                "- [ ] At least one expected-loss case per cross-format pair (e.g. XRechnung → MINIMUM Factur-X loses payment terms).\n"
                "- [ ] Tests assert the right entries appear in the ledger."),
             references=common_refs),
        Bead("T-047", "t-047-format-auto-detection", "Format auto-detection (sniff input bytes, return format identifier)",
             "task", 1, ["T-040", "T-041"], lbl("track-3", "format", "auto-detect"),
             background="Customers will hand us bytes and ask 'what is this?'. Format auto-detection sniffs the first N bytes and namespace declarations to identify the format.",
             goal="A `detect_format(bytes) -> FormatId` function with at least UBL, CII, Factur-X PDF, FatturaPA, KSeF FA(3), CFDI 4.0, ZUGFeRD profiles.",
             acceptance=(
                "- [ ] At least 10 formats detected correctly.\n"
                "- [ ] False-positive rate under 1% on the test corpus.\n"
                "- [ ] Returns `Unknown` for unrecognized inputs (never panics)."),
             references=common_refs),
    ]


def track4_rendering() -> List[Bead]:
    return [
        Bead("T-050", "t-050-typst-integration", "Typst integration as Rust crate dependency",
             "feature", 0, ["T-010"], lbl("track-4", "rendering", "typst", "critical-path", "p0"),
             background="Typst is the underlying deterministic PDF renderer. See PLAN.md §2.8, §4.10.",
             goal="Typst is pulled in as a Rust dependency; a minimal hello-world invoice renders.",
             acceptance=(
                "- [ ] `crates/render-pdf` depends on Typst with a pinned version.\n"
                "- [ ] Hello-world invoice template renders successfully.\n"
                "- [ ] Output validates as PDF/A-3 via T-052 veraPDF adapter."),
             references="PLAN.md §2.8, §4.10."),
        Bead("T-051", "t-051-typescript-template-language", "TypeScript template language compiles to Typst",
             "feature", 1, ["T-050"], lbl("track-4", "rendering", "templates"),
             background="A TypeScript template language hides Typst syntax from users. Templates compile to Typst at build time.",
             goal="A TypeScript template language that compiles to Typst.",
             acceptance=(
                "- [ ] Template language documented with at least 5 example templates.\n"
                "- [ ] Templates compile deterministically to Typst.\n"
                "- [ ] Type-safe: TypeScript catches missing fields at compile time."),
             references="PLAN.md §4.10."),
        Bead("T-052", "t-052-verapdf-adapter", "veraPDF adapter for conformance verification",
             "task", 1, ["T-050"], lbl("track-4", "rendering", "verapdf"),
             background="veraPDF is the reference PDF/A-3 conformance verifier. Runs as a JVM sidecar (`validator-verapdf`).",
             goal="An adapter that calls veraPDF and parses its output.",
             acceptance=(
                "- [ ] `validator-verapdf` sidecar deployed.\n"
                "- [ ] Adapter parses veraPDF JSON output into a typed `PdfAReport`.\n"
                "- [ ] Tests cover passing and failing PDFs."),
             references="PLAN.md §2.8, §6 Track 4 T-052."),
        Bead("T-053", "t-053-pdf-a-3-dictionary-postproc", "PDF/A-3 dictionary post-processing",
             "task", 1, ["T-052"], lbl("track-4", "rendering", "pdfa3"),
             background="Typst does not natively write all XMP metadata or ZUGFeRD-grade attachments. Post-processing via `lopdf` or upstream Typst PRs fixes XMP, attachment relationships, and metadata.",
             goal="A post-processing crate that turns Typst output into ZUGFeRD-grade PDF/A-3.",
             acceptance=(
                "- [ ] Acceptance fixtures: 5 ZUGFeRD MINIMUM, 5 BASIC WL, 5 BASIC, 5 EN 16931, 5 EXTENDED, 5 XRECHNUNG profiles.\n"
                "- [ ] All fixtures pass `verapdf --profile=3b` and `--profile=3u`.\n"
                "- [ ] Decision rule documented: upstream PR vs `lopdf` patch.\n"
                "- [ ] Lives in `crates/render-pdf-postproc`."),
             references="PLAN.md §4.10, §6 Track 4 T-053."),
        Bead("T-054", "t-054-factur-x-xml-embedding", "Factur-X XML embedding into PDF/A-3 attachment",
             "task", 1, ["T-053"], lbl("track-4", "rendering", "factur-x"),
             background="Factur-X is a PDF/A-3 with an embedded `factur-x.xml` attachment.",
             goal="Generated PDFs carry a correctly-named embedded XML attachment with the right /AFRelationship.",
             acceptance=(
                "- [ ] Embedded XML attachment name is `factur-x.xml`.\n"
                "- [ ] /AFRelationship is `Alternative`.\n"
                "- [ ] veraPDF passes both `3b` and `3u`.\n"
                "- [ ] Reader test: pdftk and qpdf both extract the attachment correctly."),
             references="PLAN.md §2.8."),
        Bead("T-055", "t-055-byte-stable-pdf-subset", "Deterministic byte-stable rendering subset",
             "task", 0, ["T-054"], lbl("track-4", "rendering", "deterministic", "critical-path", "p0"),
             background="Two renders of the same invoice must produce identical bytes. Pinned fonts, pinned harfbuzz, fixed XMP creation date placeholder, deterministic object ordering.",
             goal="`render_pdf(invoice) -> Vec<u8>` is byte-stable across runs and platforms.",
             acceptance=(
                "- [ ] CI test: render an invoice twice on Linux x86_64 and macOS aarch64; byte-equal output.\n"
                "- [ ] Pinned fonts: subsetted Inter, DejaVu, Noto in `crates/render-pdf/fonts/`.\n"
                "- [ ] No system fonts ever; never call out to system harfbuzz."),
             references="PLAN.md §2.8, §4.4."),
        Bead("T-056", "t-056-accessible-html5-render", "Accessible HTML5 rendering pipeline (WCAG-conformant)",
             "task", 1, ["T-051"], lbl("track-4", "rendering", "wcag", "accessibility"),
             background="Customers want HTML5 renders of invoices for archival display and email-safe viewing. Output must be WCAG-conformant.",
             goal="Templates render to WCAG 2.1 AA conformant HTML5.",
             acceptance=(
                "- [ ] axe-core scan passes WCAG 2.1 AA on all default templates.\n"
                "- [ ] Templates use semantic HTML5 elements.\n"
                "- [ ] Color contrast >= 4.5:1 for all text."),
             references="PLAN.md §4.10."),
        Bead("T-057", "t-057-wysiwyg-template-designer", "Web what-you-see-is-what-you-get template designer",
             "feature", 1, ["T-051"], lbl("track-4", "rendering", "wysiwyg", "dx"),
             background="A web-based designer emits the TypeScript template language. Single-page web app served by the docs site.",
             goal="A drag-and-drop template designer that emits valid TypeScript template language code.",
             acceptance=(
                "- [ ] Single-page web app deployed to `studio.invoicekit.org`.\n"
                "- [ ] Designer outputs syntactically valid TypeScript template code.\n"
                "- [ ] Output round-trips: load, edit, save, reload identical."),
             references="PLAN.md §4.10, §6 Track 4 T-057."),
        Bead("T-058", "t-058-visual-regression-tests-pdf", "Visual regression tests for rendered PDFs",
             "task", 1, ["T-055"], lbl("track-4", "rendering", "testing", "ci"),
             background="PDF rendering drift is silent and catastrophic. Visual regression catches it. Idea-wizard top-4 pick.",
             goal="Every template × every profile × every country output is visually regression-tested in CI.",
             acceptance=(
                "- [ ] Baselines stored in `conformance-corpus/pdf-snapshots/`.\n"
                "- [ ] Rasterization via `mupdf-tools` or `pdfium`; pixel diff via `pixelmatch`.\n"
                "- [ ] Diff failures surface as PR comments with side-by-side images.\n"
                "- [ ] Baseline updates require explicit human sign-off."),
             references="PLAN.md §6 Track 4 T-058."),
    ]


def track5_intake() -> List[Bead]:
    return [
        Bead("T-060", "t-060-l1-factur-x-xml-extract", "Layer 1 — Factur-X XML detection and extraction from PDF",
             "task", 1, ["T-040", "T-041", "T-050"], lbl("track-5", "intake", "factur-x"),
             background="The fastest, most reliable intake path: extract the embedded XML from a Factur-X PDF. Layer 1 in the intake pipeline (§4.9).",
             goal="`extract_factur_x_xml(pdf_bytes) -> Option<Vec<u8>>` reliably extracts the embedded XML.",
             acceptance=(
                "- [ ] Extracts XML from all six ZUGFeRD profiles correctly.\n"
                "- [ ] Returns None for non-Factur-X PDFs (never panics).\n"
                "- [ ] Performance: extract from a 5 MB PDF in under 50 ms."),
             references="PLAN.md §4.9 intake."),
        Bead("T-061", "t-061-l2-digital-pdf-text", "Layer 2 — Digital PDF text extraction",
             "task", 1, ["T-001"], lbl("track-5", "intake", "pdf"),
             background="Digital (non-scanned) PDFs carry text. Extract it deterministically.",
             goal="`extract_pdf_text(pdf_bytes) -> StructuredText` extracts text with position information.",
             acceptance=(
                "- [ ] Output preserves reading order.\n"
                "- [ ] Position information sufficient for downstream OCR layer to align.\n"
                "- [ ] Handles encrypted PDFs gracefully (returns error)."),
             references="PLAN.md §4.9."),
        Bead("T-062", "t-062-l3-paddleocr-integration", "Layer 3 — PaddleOCR integration (server-side default)",
             "task", 1, ["T-061"], lbl("track-5", "intake", "ocr"),
             background="Layout-aware OCR via PaddleOCR PP-StructureV3.",
             goal="PaddleOCR called via Rust bindings, output normalized to our `OcrSpan` schema.",
             acceptance=(
                "- [ ] At least 10 invoice scans processed correctly.\n"
                "- [ ] Output includes per-span text + bounding box.\n"
                "- [ ] Performance: 5-page invoice processed under 10 seconds (p95)."),
             references="PLAN.md §4.9."),
        Bead("T-063", "t-063-l4-smoldocling-onnx", "Layer 4 — SmolDocling-256M ONNX integration",
             "task", 1, ["T-062"], lbl("track-5", "intake", "vlm"),
             background="SmolDocling-256M is a small vision-language model for document understanding. Server-side and limited browser-side.",
             goal="SmolDocling-256M runs via ONNX Runtime, output normalized to our extraction schema.",
             acceptance=(
                "- [ ] Model loads under 2 seconds.\n"
                "- [ ] Per-field extraction for at least 10 invoice fields (supplier name, VAT, total, etc.).\n"
                "- [ ] Browser-side run via Transformers.js for short documents."),
             references="PLAN.md §4.9."),
        Bead("T-064", "t-064-l5-qwen-2-5-vl-7b", "Layer 5 — Qwen2.5-VL-7B cloud inference adapter",
             "task", 2, ["T-063"], lbl("track-5", "intake", "vlm", "cloud"),
             background="Qwen2.5-VL-7B is the cloud fallback for documents the smaller models can't extract.",
             goal="A cloud inference adapter that calls our hosted Qwen2.5-VL-7B endpoint.",
             acceptance=(
                "- [ ] Adapter handles rate limits and retries.\n"
                "- [ ] Cost telemetry logs per-call cost.\n"
                "- [ ] Tests cover happy path + timeout + auth failure."),
             references="PLAN.md §4.9."),
        Bead("T-065", "t-065-cross-examined-witness", "Cross-examined witness flow (deterministic re-validation)",
             "feature", 1, ["T-031", "T-064"], lbl("track-5", "intake", "ai-safety"),
             background="Every AI-extracted field is re-validated by deterministic rules. Mismatches block AI-only output.",
             goal="The AI extraction pipeline never emits a result that fails deterministic cross-checks.",
             acceptance=(
                "- [ ] VAT subtotals close: tested.\n"
                "- [ ] Line totals reconcile: tested.\n"
                "- [ ] VAT IDs validate against VIES: tested.\n"
                "- [ ] Mismatches surface as actionable errors with rule-id + cited fields."),
             references="PLAN.md §4.9."),
        Bead("T-066", "t-066-bounding-box-citation-taxonomy", "Bounding-box citation taxonomy",
             "task", 1, ["T-062"], lbl("track-5", "intake", "audit"),
             background="Every extracted field carries `{value, source: {bbox?, ocr_span_id?, pdf_object_id?, model_id}, confidence}`. Required for audit defensibility.",
             goal="A typed `CitationSource` enum + carrier per extracted field.",
             acceptance=(
                "- [ ] Schema documented.\n"
                "- [ ] At least 5 sample extractions verified to carry the right citation."),
             references="PLAN.md §4.9."),
        Bead("T-067", "t-067-pii-gdpr-redactor", "PII/GDPR redactor for support bundles",
             "task", 1, ["T-010"], lbl("track-5", "privacy", "gdpr"),
             background="Customers share support bundles; we must redact PII before storing.",
             goal="`redact_for_support(bundle) -> RedactedBundle` strips names, addresses, account numbers.",
             acceptance=(
                "- [ ] Strips supplier/customer party names, addresses, phone, email, bank account, IBAN.\n"
                "- [ ] Preserves structure for debugging (replaces with `<REDACTED>` placeholders).\n"
                "- [ ] Reversible only with explicit unredaction key (out of scope for v1)."),
             references="PLAN.md §6 Track 5 T-067."),
    ]


def track6_reconciliation_evidence() -> List[Bead]:
    return [
        Bead("T-070", "t-070-gateway-adapter-trait", "Gateway adapter trait and normalized gateway error taxonomy",
             "feature", 0, ["T-010"], lbl("track-6", "reconcile", "gateway", "critical-path", "p0"),
             background="Every country gateway is a `GatewayAdapter` trait impl. The trait + a normalized error taxonomy are the spine of transmission.",
             goal="A `GatewayAdapter` trait with normalized error taxonomy.",
             acceptance=(
                "- [ ] Trait methods: `submit`, `poll`, `cancel`, `correct`.\n"
                "- [ ] Error taxonomy normalized: AuthFailure, RateLimited, MalformedReceipt, GatewayMaintenance, CertificateRejected, DuplicateSubmission, etc.\n"
                "- [ ] At least one impl (T-074 mock gateway)."),
             references="PLAN.md §2.5, §4.6."),
        Bead("T-070a", "t-070a-transmission-state-model", "Extensible transmission state model and transition contract",
             "task", 0, ["T-070"], lbl("track-6", "reconcile", "state-machine", "critical-path", "p0"),
             background="Per-country sub-states layer in cleanly on the base state machine. See §4.6.",
             goal="A state model with valid transitions, extensible for per-country sub-states.",
             acceptance=(
                "- [ ] Base states: draft, validated, signed, reserved, sent, delivered, acknowledged, rejected, archived.\n"
                "- [ ] Per-country sub-state hook documented + tested with KSeF, SDI, ZATCA cases.\n"
                "- [ ] Invalid transitions return typed errors."),
             references="PLAN.md §4.6."),
        Bead("T-071", "t-071-outbox-sql-schema", "Outbox SQL schema, idempotency model, retry policy, dead-letter states",
             "feature", 0, ["T-022", "T-070a"], lbl("track-6", "reconcile", "outbox", "critical-path", "p0"),
             background="At-least-once delivery semantics. Customers ship our outbox migrations with their schema.",
             goal="SQL migrations + a typed Outbox API for Postgres, MySQL, SQLite.",
             acceptance=(
                "- [ ] Migrations idempotent and reversible.\n"
                "- [ ] Idempotency-key column with unique constraint.\n"
                "- [ ] Retry policy with exponential backoff + jitter.\n"
                "- [ ] Dead-letter table for unrecoverable failures.\n"
                "- [ ] Tests against all three DBs."),
             references="PLAN.md §4.6."),
        Bead("T-072", "t-072-transmission-worker", "Transmission worker with backoff, rate limits, circuit breakers, logs",
             "feature", 0, ["T-071", "T-073", "T-074"], lbl("track-6", "reconcile", "transmission", "critical-path", "p0"),
             background="The worker drains the outbox, calls GatewayAdapters, handles retries.",
             goal="A long-running worker that processes the outbox with all observability and resilience built in.",
             acceptance=(
                "- [ ] Exponential backoff with configurable max-retries.\n"
                "- [ ] Per-gateway rate limit honored.\n"
                "- [ ] Circuit breaker on persistent failures.\n"
                "- [ ] Structured JSON logs with trace_id + tenant_id."),
             references="PLAN.md §4.6, §4.12."),
        Bead("T-073", "t-073-state-machine-implementation", "State machine implementation (per-country sub-states)",
             "task", 0, ["T-070a"], lbl("track-6", "reconcile", "state-machine", "critical-path", "p0"),
             background="The actual implementation of the state model on top of T-070a's contract.",
             goal="A typed state machine with per-country sub-state extensions.",
             acceptance=(
                "- [ ] All transitions implemented.\n"
                "- [ ] Per-country sub-state extensions tested with at least 3 country examples."),
             references="PLAN.md §4.6."),
        Bead("T-074a", "t-074a-cassette-recorder-framework", "Cassette recorder, scrubber, matcher, scenario metadata",
             "feature", 0, ["T-070", "T-120"], lbl("track-6", "cassette", "testing", "critical-path", "p0"),
             background="The cassette framework lets us record real interactions against official sandboxes and replay them deterministically. See §4.11.",
             goal="A cassette framework with recorder, scrubber, matcher, scenario metadata schema.",
             acceptance=(
                "- [ ] Recorder produces deterministic .vcr-format cassettes.\n"
                "- [ ] Scrubber removes personal data (configurable rules per country).\n"
                "- [ ] Matcher routes by method + path + body fingerprint.\n"
                "- [ ] Scenario metadata schema documented."),
             references="PLAN.md §4.11, §6 Track 6 T-074a."),
        Bead("T-074", "t-074-mock-gateway-transmit-mock", "Mock gateway (transmit-mock) — first GatewayAdapter impl",
             "task", 1, ["T-074a"], lbl("track-6", "cassette", "testing"),
             background="The mock gateway is the first GatewayAdapter, drives the cassette engine.",
             goal="A working mock gateway that replays cassettes.",
             acceptance=(
                "- [ ] At least 2 baseline cassettes (success + failure) recorded by T-074a.\n"
                "- [ ] Recorder produces them deterministically (byte-equal across runs).\n"
                "- [ ] Mock gateway returns canned receipts based on cassette match."),
             references="PLAN.md §4.11."),
        Bead("T-074b", "t-074b-gateway-contract-test-suite", "GatewayAdapter contract test suite backed by cassettes",
             "task", 1, ["T-074", "T-073"], lbl("track-6", "cassette", "testing"),
             background="A contract test suite ensures every future GatewayAdapter conforms to the same protocol.",
             goal="A contract test suite that any GatewayAdapter impl must pass.",
             acceptance=(
                "- [ ] Required scenarios covered: idempotent replay, duplicate submission, timeout, malformed receipt, auth failure, certificate rejection, rate limit, delayed async receipt, unknown response field, gateway maintenance page, partner error translation.\n"
                "- [ ] Every Track 7/8 adapter passes."),
             references="PLAN.md §6 Track 6 T-074b."),
        Bead("T-074c", "t-074c-sandbox-drift-canary", "Nightly sandbox drift canary",
             "task", 1, ["T-006", "T-074a"], lbl("track-6", "cassette", "drift"),
             background="Nightly job replays live sandbox calls and diffs against normalized cassettes. Catches silent regulator changes.",
             goal="A nightly job that detects sandbox drift and opens issues.",
             acceptance=(
                "- [ ] Runs nightly via GitHub Actions.\n"
                "- [ ] On drift: opens an issue via T-006 source-watch.\n"
                "- [ ] Configurable per country (skip if no sandbox credentials)."),
             references="PLAN.md §6 Track 6 T-074c."),
        Bead("T-074d", "t-074d-sandbox-prod-parity-diff", "Sandbox / production parity diff",
             "task", 1, ["T-074a", "T-074c"], lbl("track-6", "cassette"),
             background="Catches drift between sandbox and production endpoints. Customer-consent-gated.",
             goal="A nightly job that diffs sandbox vs production for the same input.",
             acceptance=(
                "- [ ] Customer consent recorded before any production calls.\n"
                "- [ ] Drift alerts opened via T-006.\n"
                "- [ ] Tests on at least 3 country pairs."),
             references="PLAN.md §6 Track 6 T-074d."),
        Bead("T-075", "t-075-reconciliation-api", "Reconciliation API and outbox SQL migrations",
             "task", 1, ["T-071"], lbl("track-6", "reconcile", "api"),
             background="Customers reconcile their internal IDs against our state. See §4.6.",
             goal="`POST /v1/reconcile` accepts `{internal_id, fingerprint}` lists, returns `{delivered, failed, pending, unknown}`.",
             acceptance=(
                "- [ ] API documented in OpenAPI.\n"
                "- [ ] Handles batches of up to 10,000 entries.\n"
                "- [ ] Tests on Postgres, MySQL, SQLite."),
             references="PLAN.md §4.6."),
        Bead("T-076", "t-076-webhook-dispatcher", "Webhook dispatcher with replay protection and idempotency",
             "task", 1, ["T-073"], lbl("track-6", "webhooks"),
             background="State changes fire webhooks. Replay protection + idempotency are mandatory.",
             goal="A webhook dispatcher with retries and signature verification.",
             acceptance=(
                "- [ ] HMAC-SHA256 signature (T-132 format).\n"
                "- [ ] Replay protection via timestamp + 5-min window.\n"
                "- [ ] Idempotency via event ID.\n"
                "- [ ] At-least-once delivery with retry backoff."),
             references="PLAN.md §6 Track 6 T-076, §6 T-132."),
        Bead("T-077", "t-077-sse-ack-stream", "Server-Sent Events stream for ACK delivery",
             "task", 2, ["T-073"], lbl("track-6", "sse"),
             background="SSE works behind firewalls; useful for some integration patterns.",
             goal="A `GET /v1/events/sse` stream that delivers state changes in real time.",
             acceptance=(
                "- [ ] Auth via API key.\n"
                "- [ ] Filtered by tenant.\n"
                "- [ ] Reconnect handling tested."),
             references="PLAN.md §4.6."),
        Bead("T-080", "t-080-evidence-bundle-format", "Signed evidence bundle format (.invoicekit, packed .ikb)",
             "feature", 0, ["T-019", "T-020", "T-022", "T-031", "T-040", "T-041", "T-055", "T-073"], lbl("track-6", "evidence", "audit", "critical-path", "p0"),
             background="The signed, content-addressed audit artifact. See §2.10, §4.7.",
             goal="The `.invoicekit` bundle format + `.ikb` packed form, with verification.",
             acceptance=(
                "- [ ] Bundle layout per §4.7.\n"
                "- [ ] DSSE signature over manifest.\n"
                "- [ ] BLAKE3 hashes for content addressing.\n"
                "- [ ] `.ikb` is byte-stable (tar.zst with normalized metadata).\n"
                "- [ ] Verification reproduces every claim in the manifest."),
             references="PLAN.md §2.10, §4.7."),
        Bead("T-081", "t-081-archive-backend-pluggable", "Pluggable archive backend",
             "task", 1, ["T-080"], lbl("track-6", "archive"),
             background="S3 Object Lock, Azure WORM, GCS retention, local FS, IPFS hash.",
             goal="An archive trait with at least 3 backends.",
             acceptance=(
                "- [ ] Trait: `Archive::store(bundle) -> ArchiveId`, `Archive::retrieve(id) -> Bundle`.\n"
                "- [ ] S3 Object Lock impl + tests against LocalStack.\n"
                "- [ ] Local FS impl + tests."),
             references="PLAN.md §4.7."),
        Bead("T-082", "t-082-rfc-3161-timestamping", "RFC 3161 timestamping integration",
             "task", 1, ["T-080"], lbl("track-6", "evidence", "timestamping"),
             background="Bundles include an RFC 3161 timestamp from a trusted timestamping authority.",
             goal="Bundles can be timestamped via at least one trusted TSA.",
             acceptance=(
                "- [ ] Adapter for at least one TSA (FreeTSA for testing, qualified TSA for production).\n"
                "- [ ] Timestamp verification works offline (with the TSA's public cert)."),
             references="PLAN.md §4.7."),
        Bead("T-083", "t-083-signing-substrate-signer-agent", "Stable signing API + invoicekit-signer-agent local proxy",
             "feature", 0, ["T-080"], lbl("track-6", "evidence", "signing", "critical-path", "p0"),
             background="Local signing proxy for enterprise customers whose keys cannot leave their datacenter. See §6 T-083.",
             goal="A signing substrate + an on-premise signer-agent.",
             acceptance=(
                "- [ ] Signing API: `sign(payload, key_ref) -> Signature`.\n"
                "- [ ] In-process software signing for non-regulated cases.\n"
                "- [ ] `invoicekit-signer-agent` daemon listens on a local Unix socket.\n"
                "- [ ] Engine calls signer-agent over local HTTPS or Unix socket."),
             references="PLAN.md §6 Track 6 T-083."),
        Bead("T-083a", "t-083a-eidas-qualified-signature-adapter", "eIDAS qualified signature provider adapter",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "eidas"),
             background="eIDAS qualified signature provider plug-in. Keys stay on-premise via signer-agent.",
             goal="An eIDAS QTSP adapter.",
             acceptance=(
                "- [ ] At least one QTSP integration (D-Trust or GlobalSign).\n"
                "- [ ] Signature verification round-trips."),
             references="PLAN.md §6 Track 6 T-083a."),
        Bead("T-083b", "t-083b-country-signer-adapters-umbrella", "Country-specific signer adapters (umbrella)",
             "epic", 1, ["T-083"], lbl("track-6", "evidence", "signing"),
             background="Umbrella for per-country adapters T-083b1..T-083b5.",
             goal="All five per-country signer adapters delivered.",
             acceptance=(
                "- [ ] T-083b1..T-083b5 all closed.\n"
                "- [ ] Country crates consume these adapters, never re-implement crypto."),
             references="PLAN.md §6 Track 6 T-083b."),
        Bead("T-083b1", "t-083b1-zatca-cryptographic-stamp", "Saudi Arabia ZATCA Phase 2 cryptographic stamp adapter",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "zatca"),
             background="ECDSA secp256k1 signing over the canonical TLV payload. Returns base64 stamp.",
             goal="An adapter that produces ZATCA Phase 2 cryptographic stamps.",
             acceptance=(
                "- [ ] ECDSA secp256k1 implementation.\n"
                "- [ ] Test vectors from ZATCA documentation pass.\n"
                "- [ ] Test against a real sandbox certificate."),
             references="PLAN.md §6 Track 6 T-083b1."),
        Bead("T-083b2", "t-083b2-cfdi-pac-signing", "Mexico CFDI 4.0 PAC signing flow adapter",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "cfdi"),
             background="Sends to a Mexican PAC partner; receives the timbre fiscal digital.",
             goal="A PAC adapter for CFDI 4.0.",
             acceptance=(
                "- [ ] Adapter for at least one Mexican PAC (EDICOM, Aspel, etc.).\n"
                "- [ ] Test against PAC sandbox."),
             references="PLAN.md §6 Track 6 T-083b2."),
        Bead("T-083b3", "t-083b3-ksef-certificate", "Poland KSeF certificate flow adapter",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "ksef"),
             background="Signs with the customer's qualified certificate via signer-agent.",
             goal="A KSeF certificate adapter.",
             acceptance=(
                "- [ ] Signs with a qualified certificate via signer-agent.\n"
                "- [ ] Test against KSeF sandbox."),
             references="PLAN.md §6 Track 6 T-083b3."),
        Bead("T-083b4", "t-083b4-sdi-aruba-italian-cert", "Italy SDI / Aruba qualified certificate flow",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "sdi"),
             background="Italian qualified certificate flow (Aruba, InfoCert, etc.).",
             goal="An SDI certificate adapter.",
             acceptance=(
                "- [ ] Aruba qualified certificate adapter.\n"
                "- [ ] Test against SDI sandbox."),
             references="PLAN.md §6 Track 6 T-083b4."),
        Bead("T-083b5", "t-083b5-nfe-federal-certificate", "Brazil NF-e federal certificate flow",
             "task", 1, ["T-083"], lbl("track-6", "evidence", "signing", "nfe"),
             background="A1/A3 certificates, SEFAZ-specific signing.",
             goal="An NF-e certificate adapter.",
             acceptance=(
                "- [ ] A1 certificate adapter.\n"
                "- [ ] Test against SEFAZ sandbox."),
             references="PLAN.md §6 Track 6 T-083b5."),
        Bead("T-084", "t-084-invoicekit-verify-cli", "`invoicekit verify` library and CLI",
             "task", 1, ["T-080", "T-082", "T-083"], lbl("track-6", "evidence", "verify", "cli"),
             background="Reproduces and verifies an evidence bundle.",
             goal="A library + CLI that verifies a .invoicekit bundle.",
             acceptance=(
                "- [ ] `invoicekit verify bundle.invoicekit` succeeds for valid bundles.\n"
                "- [ ] Tampered bundles fail verification with clear error.\n"
                "- [ ] Signature, timestamp, content addresses all verified."),
             references="PLAN.md §6 Track 6 T-084."),
        Bead("T-085", "t-085-replay-from-bundle", "Replay-from-bundle (audit/debug feature)",
             "feature", 1, ["T-080", "T-084"], lbl("track-6", "evidence", "replay", "dx"),
             background="Re-runs the entire pipeline from a bundle and asserts byte-equality with originally-recorded outputs.",
             goal="`invoicekit replay bundle.invoicekit --against=<snapshot>` works end-to-end.",
             acceptance=(
                "- [ ] Replay produces byte-equal outputs when nothing has changed.\n"
                "- [ ] Replay produces structured diff when something has changed.\n"
                "- [ ] Lives in `crates/replay`."),
             references="PLAN.md §6 Track 6 T-085."),
    ]


def track7_peppol_live() -> List[Bead]:
    return [
        Bead("T-090", "t-090-smp-sml-lookup", "Peppol participant lookup (SMP/SML client)",
             "task", 1, ["T-042"], lbl("track-7", "peppol", "smp"),
             background="Find the right access point for a Peppol participant ID.",
             goal="An SMP/SML client that resolves participant IDs to access points.",
             acceptance=(
                "- [ ] DNS-based SML lookup.\n"
                "- [ ] SMP HTTP fetch + parse.\n"
                "- [ ] Cache with TTL respect.\n"
                "- [ ] Tests against the OpenPeppol test SML."),
             references="PLAN.md §6 Track 7 T-090."),
        Bead("T-091", "t-091-partner-peppol-ap-adapter", "Partner Peppol access point adapter",
             "feature", 1, ["T-072", "T-090"], lbl("track-7", "peppol", "partner"),
             background="Wraps a chosen partner AP (Storecove / ecosio / B2BRouter). Selection decided in Phase 2.5 manifests.",
             goal="A partner AP adapter implementing GatewayAdapter.",
             acceptance=(
                "- [ ] Adapter passes T-074b contract tests.\n"
                "- [ ] Sandbox round-trip with a real partner.\n"
                "- [ ] Configurable via env vars + secret manager."),
             references="PLAN.md §2.7, §6 Track 7 T-091."),
        Bead("T-092", "t-092-phase4-reference-adapter", "phase4 reference adapter (validator-phase4 sidecar)",
             "task", 1, ["T-091"], lbl("track-7", "peppol", "phase4"),
             background="phase4 runs as a JVM sidecar; we call it as a conformance oracle for our native AS4 work.",
             goal="`validator-phase4` sidecar + Rust adapter.",
             acceptance=(
                "- [ ] phase4 sidecar deployed.\n"
                "- [ ] Adapter sends/receives via phase4.\n"
                "- [ ] Used as the oracle in T-094 differential testing."),
             references="PLAN.md §2.7, §4.8."),
        Bead("T-093", "t-093-peppol-inbound-receiver", "Peppol inbound receiver service",
             "task", 1, ["T-091"], lbl("track-7", "peppol", "inbound"),
             background="Inbound Peppol traffic: parse, validate, archive.",
             goal="A receiver service that ingests Peppol-delivered invoices.",
             acceptance=(
                "- [ ] Receives via partner AP webhook OR native AS4 receiver.\n"
                "- [ ] Validates via T-031 + JVM sidecars.\n"
                "- [ ] Archives via T-081.\n"
                "- [ ] Tests with at least 10 inbound fixtures."),
             references="PLAN.md §4.8."),
        Bead("T-094", "t-094-native-rust-as4-sender", "Native Rust AS4 sender",
             "feature", 1, ["T-090", "T-092"], lbl("track-7", "peppol", "as4", "native"),
             background="Native Rust AS4 sender, differentially tested against phase4. Promoted per-route as it passes conformance.",
             goal="A Rust AS4 sender that passes OpenPeppol conformance.",
             acceptance=(
                "- [ ] ebMS3 + WS-Security implemented.\n"
                "- [ ] SBDH wrapping correct.\n"
                "- [ ] Differential test vs phase4: byte-equal envelopes for at least 20 fixtures.\n"
                "- [ ] OpenPeppol conformance suite passes."),
             references="PLAN.md §2.7, §6 Track 7 T-094."),
        Bead("T-095", "t-095-native-rust-as4-receiver", "Native Rust AS4 receiver",
             "feature", 1, ["T-093", "T-094"], lbl("track-7", "peppol", "as4", "native"),
             background="The harder half. WS-Security validation, receipts (NRR), MEPs.",
             goal="A Rust AS4 receiver that passes inbound conformance.",
             acceptance=(
                "- [ ] WS-Security signature validation.\n"
                "- [ ] Reliability messaging receipts (Non-repudiation of Receipt).\n"
                "- [ ] Tested against phase4 sender."),
             references="PLAN.md §2.7."),
    ]


def track7_5_country_manifests() -> List[Bead]:
    """Phase 2.5 country feasibility manifests, one per country."""
    countries = [
        ("T-770", "poland", "Poland"),
        ("T-771", "saudi-arabia", "Saudi Arabia"),
        ("T-772", "belgium", "Belgium"),
        ("T-773", "italy", "Italy"),
        ("T-774", "france", "France"),
        ("T-775", "spain", "Spain"),
        ("T-776", "greece", "Greece"),
        ("T-777", "uae", "United Arab Emirates"),
        ("T-778", "india", "India"),
        ("T-779", "mexico", "Mexico"),
        ("T-780", "brazil", "Brazil (multiple sub-manifests for NF-e and per-municipal NFS-e)"),
        ("T-781", "malaysia", "Malaysia"),
        ("T-782", "turkey", "Turkey"),
        ("T-783", "romania", "Romania"),
        ("T-784", "hungary", "Hungary"),
        ("T-785", "japan", "Japan (Qualified Invoice System)"),
        ("T-786", "chile", "Chile"),
        ("T-787", "colombia", "Colombia"),
        ("T-788", "peru", "Peru"),
        ("T-789", "argentina", "Argentina"),
        ("T-790", "ecuador", "Ecuador"),
        ("T-791", "costa-rica", "Costa Rica"),
        ("T-792", "dominican-republic", "Dominican Republic"),
        ("T-793", "egypt", "Egypt"),
        ("T-794", "israel", "Israel"),
        ("T-795", "indonesia", "Indonesia"),
        ("T-796", "philippines", "Philippines"),
        ("T-797", "vietnam", "Vietnam"),
        ("T-798", "thailand", "Thailand"),
        ("T-799", "south-korea", "South Korea"),
        ("T-7A0", "china", "China"),
        ("T-7A1", "taiwan", "Taiwan"),
        ("T-7A2", "kenya", "Kenya"),
        ("T-7A3", "nigeria", "Nigeria"),
        ("T-7A4", "south-africa", "South Africa"),
        ("T-7A5", "portugal", "Portugal (national reporting alongside Peppol)"),
    ]
    beads = []
    for tid, slug_base, country_name in countries:
        beads.append(Bead(
            tid=tid,
            slug=f"{tid.lower().replace('-', '-')}-manifest-{slug_base}",
            title=f"Country feasibility manifest: {country_name}",
            type="task",
            priority=2,
            deps=["T-006", "T-074a"],
            labels=lbl("track-7-5", "manifest", slug_base),  # NOT "country" — that label triggers gates that don't apply to manifests
            background=(
                f"Phase 2.5 country feasibility manifest for {country_name}. "
                "Mandatory input before the country's Track 8 crate can start. See PLAN.md §3.3 Phase 2.5."
            ),
            goal=f"A signed manifest documenting all {country_name} feasibility data plus baseline fixture set and sandbox cassettes.",
            acceptance=(
                "- [ ] Source URLs and retrieval dates documented.\n"
                "- [ ] Sandbox availability documented (public / partner-gated / requires local tax ID).\n"
                "- [ ] Qualified electronic seal / HSM / smart-card requirements documented.\n"
                "- [ ] Local fiscal representative / in-country PAC / ASP requirements documented.\n"
                "- [ ] Validator backend selected (rust-native / jvm:* / rest:official / partner / cli / none).\n"
                "- [ ] Partner options documented with per-envelope pricing if disclosed.\n"
                "- [ ] Go / no-go flag set.\n"
                "- [ ] Initial fixture set: at least 5 valid + 5 invalid invoices in the country's required format.\n"
                "- [ ] Baseline sandbox cassettes recorded (at least one success and one canonical error) when a sandbox exists.\n"
                "- [ ] Manifest signed (sigstore or minisign)."
            ),
            notes="Manifest format documented in `data/country-manifests/SCHEMA.md`.",
            references="PLAN.md §3.3 Phase 2.5; §6 Track 7.5.",
        ))
    return beads


def track8_country_crates() -> List[Bead]:
    """Track 8 = archetype lock-in + Wave 1/2/3 country crates."""
    beads = []

    # Archetype lock-in (sequential)
    beads.append(Bead(
        tid="T-800",
        slug="t-800-archetype-async-clearance-ksef",
        title="Archetype: Poland KSeF (async clearance archetype)",
        type="feature",
        priority=1,
        deps=["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-080", "T-083", "T-770", "T-083b3"],
        labels=lbl("track-8", "archetype", "poland", "wave-0"),  # NOT "country" — archetypes DEFINE the country trait, they don't consume it
        background=(
            "First archetype: async clearance state machine (submit → reserve → poll → commit → cancel → correct). "
            "Locks the trait surface every later async-clearance country uses. See PLAN.md §3.3 archetype lock-in."
        ),
        goal="Poland KSeF FA(3) reported live via partner; archetype trait stable and documented.",
        acceptance=(
            "- [ ] Async-clearance archetype trait defined and documented in `crates/report-archetype-async/`.\n"
            "- [ ] Poland KSeF FA(3) format serializer.\n"
            "- [ ] Clearance state machine implements: draft → signed → reserved → submitted → committed → cancelled? → corrected?.\n"
            "- [ ] Cassette set covers at least: successful submission, schema error, certificate rejection, KSeF maintenance, peak-hour latency.\n"
            "- [ ] Sandbox-proven status with KSeF sandbox.\n"
            "- [ ] Signing via T-083b3 (does NOT re-implement signing).\n"
            "- [ ] Reconciliation API returns correct state for tested transmissions."
        ),
        notes="THE archetype for every async-clearance country in Waves 1/2/3.",
        references="PLAN.md §3.3, §6 Track 8 T-800.",
    ))
    beads.append(Bead(
        tid="T-801",
        slug="t-801-archetype-cryptographic-zatca",
        title="Archetype: Saudi Arabia ZATCA (cryptographic archetype)",
        type="feature",
        priority=1,
        deps=["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-080", "T-083", "T-771", "T-083b1"],
        labels=lbl("track-8", "archetype", "saudi-arabia", "wave-0"),  # NOT "country" — archetype DEFINES the trait
        background=(
            "Second archetype: cryptographic. Heaviest. ECDSA secp256k1 stamping, custom UBL canonicalization, TLV QR generation, "
            "certificate management orchestration. See PLAN.md §3.3."
        ),
        goal="Saudi Arabia ZATCA Phase 2 reported live; cryptographic archetype trait stable.",
        acceptance=(
            "- [ ] Cryptographic archetype trait defined in `crates/report-archetype-cryptographic/`.\n"
            "- [ ] ZATCA Phase 2 UBL canonicalization.\n"
            "- [ ] TLV QR code generation per spec.\n"
            "- [ ] Hash-chain across invoices (previous_invoice_hash) handled.\n"
            "- [ ] Cassette set: clearance success, clearance error, certificate rejection, hash-chain mismatch.\n"
            "- [ ] Sandbox-proven status with ZATCA Fatoora sandbox.\n"
            "- [ ] Crypto via T-083b1 (does NOT re-implement signing)."
        ),
        notes="6–8 weeks: the longest archetype. THE archetype for every cryptographic-clearance country (CFDI, NF-e, China Fapiao).",
        references="PLAN.md §3.3, §6 Track 8 T-801.",
    ))
    beads.append(Bead(
        tid="T-802",
        slug="t-802-archetype-peppol-overlay-belgium",
        title="Archetype: Belgium Peppol overlay (Peppol overlay archetype)",
        type="feature",
        priority=1,
        deps=["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-080", "T-091", "T-772"],
        labels=lbl("track-8", "archetype", "belgium", "wave-0"),  # NOT "country" — archetype DEFINES the trait
        background=(
            "Third archetype: Peppol-mandate / CIUS overlay. Thin wrapper over Family A. See PLAN.md §3.3."
        ),
        goal="Belgium Peppol mandate path implemented; Peppol overlay archetype trait stable.",
        acceptance=(
            "- [ ] Peppol overlay archetype trait defined in `crates/report-archetype-peppol/`.\n"
            "- [ ] Belgium CIUS overlay implemented (using Family A T-042 base).\n"
            "- [ ] Cassette set: successful delivery via partner AP, malformed acknowledgment, recipient not on Peppol.\n"
            "- [ ] Partner-live status."
        ),
        notes="1 week. THE archetype for every Peppol-mandate country.",
        references="PLAN.md §3.3, §6 Track 8 T-802.",
    ))

    # Wave 1
    wave1 = [
        ("T-810", "italy", "Italy SDI", "T-773", "async-clearance", "T-083b4", "Italy SDI clearance and receipts (FatturaPA 1.2.2)"),
        ("T-811", "france", "France PA-PDP", "T-774", "async-clearance + peppol-overlay", None, "France PA / PDP e-invoicing and e-reporting"),
        ("T-812", "spain", "Spain VeriFactu", "T-775", "async-clearance", None, "Spain VeriFactu and FacturaE"),
        ("T-813", "greece", "Greece myDATA", "T-776", "async-clearance", None, "Greece myDATA reporting"),
        ("T-814", "uae", "UAE PINT-AE", "T-777", "peppol-overlay", None, "UAE PINT-AE national onboarding"),
    ]
    for tid, slug_country, short_title, manifest_tid, lineage, signer, desc in wave1:
        deps = ["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-074b", "T-080", "T-083", manifest_tid]
        if "peppol" in lineage:
            deps.append("T-802")
            deps.append("T-091")
        if "async" in lineage:
            deps.append("T-800")
        if "cryptographic" in lineage:
            deps.append("T-801")
        if signer:
            deps.append(signer)
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-country-{slug_country}",
            title=f"Country crate: {short_title}", type="task", priority=2,
            deps=deps,
            labels=lbl("track-8", "country", "wave-1", slug_country),
            background=desc,
            goal=f"{short_title} general-availability per the matrix in PLAN.md §3.4.",
            acceptance=(
                "- [ ] Country format serializer + parser implemented.\n"
                "- [ ] Validator backend wired up per the manifest.\n"
                "- [ ] State machine transitions tested against cassette set.\n"
                "- [ ] Partner-live or native-live status achieved.\n"
                "- [ ] Inbound flow tested with at least 10 fixtures.\n"
                "- [ ] Archive + correction flow tested.\n"
                "- [ ] Maturity matrix cell updated to GA."),
            notes=f"Archetype lineage: {lineage}.",
            references=f"PLAN.md §6 Track 8 {tid}; §3.2 Family B.",
        ))

    # Wave 2
    wave2 = [
        ("T-820", "india", "India IRP / GST", "T-778", "async-clearance"),
        ("T-821", "mexico", "Mexico CFDI 4.0", "T-779", "async-clearance + cryptographic"),
        ("T-822", "brazil", "Brazil NF-e + NFS-e", "T-780", "async-clearance + cryptographic"),
        ("T-823", "malaysia", "Malaysia MyInvois", "T-781", "async-clearance"),
        ("T-824", "turkey", "Turkey e-Fatura", "T-782", "async-clearance"),
        ("T-825", "romania", "Romania RO e-Factura", "T-783", "async-clearance"),
        ("T-826", "hungary", "Hungary NAV", "T-784", "async-clearance (reporting variant)"),
        ("T-827", "japan", "Japan Qualified Invoice System", "T-785", "peppol-overlay"),
    ]
    for tid, slug_country, short_title, manifest_tid, lineage in wave2:
        deps = ["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-074b", "T-080", "T-083", manifest_tid]
        if "peppol" in lineage:
            deps.append("T-802"); deps.append("T-091")
        if "async" in lineage:
            deps.append("T-800")
        if "cryptographic" in lineage:
            deps.append("T-801")
        if "T-821" == tid:
            deps.append("T-083b2")
        if "T-822" == tid:
            deps.append("T-083b5")
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-country-{slug_country}",
            title=f"Country crate: {short_title}", type="task", priority=2,
            deps=deps,
            labels=lbl("track-8", "country", "wave-2", slug_country),
            background=f"{short_title} country crate. Wave 2.",
            goal=f"{short_title} general-availability per the matrix in PLAN.md §3.4.",
            acceptance=(
                "- [ ] Country format implemented.\n"
                "- [ ] Validator wired.\n"
                "- [ ] Cassette set covers success + at least two error paths.\n"
                "- [ ] Partner-live status.\n"
                "- [ ] Maturity matrix cell updated."),
            notes=f"Archetype lineage: {lineage}.",
            references=f"PLAN.md §6 Track 8 {tid}.",
        ))

    # Wave 3
    wave3 = [
        ("T-830", "chile", "Chile SII DTE", "T-786"),
        ("T-831", "colombia", "Colombia DIAN", "T-787"),
        ("T-832", "peru", "Peru SUNAT", "T-788"),
        ("T-833", "argentina", "Argentina AFIP", "T-789"),
        ("T-834", "ecuador", "Ecuador SRI", "T-790"),
        ("T-835", "costa-rica", "Costa Rica Hacienda", "T-791"),
        ("T-836", "dominican-republic", "Dominican Republic DGII", "T-792"),
        ("T-837", "egypt", "Egypt ETA", "T-793"),
        ("T-838", "israel", "Israel Tax Authority", "T-794"),
        ("T-839", "indonesia", "Indonesia DJP Online", "T-795"),
        ("T-840", "philippines", "Philippines BIR EIS", "T-796"),
        ("T-841", "vietnam", "Vietnam GDT", "T-797"),
        ("T-842", "thailand", "Thailand RD", "T-798"),
        ("T-843", "south-korea", "South Korea NTS", "T-799"),
        ("T-844", "china", "China Golden Tax / Fapiao", "T-7A0"),
        ("T-845", "taiwan", "Taiwan MOF", "T-7A1"),
        ("T-846", "kenya", "Kenya eTIMS", "T-7A2"),
        ("T-847", "nigeria", "Nigeria FIRS", "T-7A3"),
        ("T-848", "south-africa", "South Africa SARS", "T-7A4"),
        ("T-849", "portugal", "Portugal national reporting", "T-7A5"),
    ]
    for tid, slug_country, short_title, manifest_tid in wave3:
        deps = ["T-010", "T-017", "T-070", "T-070a", "T-073", "T-074a", "T-074b", "T-080", "T-083", "T-800", manifest_tid]
        # China Fapiao requires cryptographic
        if tid == "T-844":
            deps.append("T-801")
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-country-{slug_country}",
            title=f"Country crate: {short_title}", type="task", priority=3,
            deps=deps,
            labels=lbl("track-8", "country", "wave-3", slug_country),
            background=f"{short_title}. Wave 3.",
            goal=f"{short_title} general-availability per the matrix in §3.4.",
            acceptance=(
                "- [ ] Country format implemented.\n"
                "- [ ] Validator wired.\n"
                "- [ ] Cassette set covers success + two error paths.\n"
                "- [ ] Partner-live status (or simulated where credentials unavailable).\n"
                "- [ ] Maturity matrix cell updated."),
            notes="Archetype lineage: async-clearance.",
            references=f"PLAN.md §6 Track 8 {tid}.",
        ))

    return beads


def track9_dx_surface() -> List[Bead]:
    return [
        Bead("T-100", "t-100-invoicekit-cli", "`invoicekit` command-line binary",
             "feature", 1, ["T-031"], lbl("track-9", "dx", "cli"),
             background="Top-level CLI binary that exposes every subcommand.",
             goal="The `invoicekit` CLI binary with all documented subcommands wired up.",
             acceptance=(
                "- [ ] `invoicekit --help` documents every subcommand.\n"
                "- [ ] All subcommands implemented and tested.\n"
                "- [ ] Cross-platform: builds and runs on Linux, macOS, Windows."),
             references="PLAN.md §5.3."),
        Bead("T-100a", "t-100a-invoicekit-repl", "`invoicekit repl` interactive session",
             "feature", 2, ["T-100"], lbl("track-9", "dx", "cli", "repl"),
             background="Interactive REPL wrapping CLI commands in a rustyline shell.",
             goal="`invoicekit repl` opens an interactive session.",
             acceptance=(
                "- [ ] rustyline-backed shell.\n"
                "- [ ] Subcommands available without re-typing the binary name.\n"
                "- [ ] State (current invoice draft, current tenant) persists in the session."),
             references="PLAN.md §6 Track 9 T-100a."),
        Bead("T-101", "t-101-invoicekit-doctor", "`invoicekit doctor`",
             "task", 1, ["T-100"], lbl("track-9", "dx", "cli"),
             background="Diagnostic command. See PLAN.md §5.1.",
             goal="`invoicekit doctor` reports engine, validator availability, rulepack freshness, etc.",
             acceptance=(
                "- [ ] Reports each diagnostic with pass/fail and remediation hint."),
             references="PLAN.md §5.1."),
        Bead("T-102", "t-102-invoicekit-init", "`invoicekit init` interactive",
             "task", 1, ["T-100"], lbl("track-9", "dx", "cli"),
             background="Interactive scaffolding for first-touch developer experience.",
             goal="`invoicekit init` walks user through first invoice setup.",
             acceptance=(
                "- [ ] Detects host language/framework.\n"
                "- [ ] Auto-detects country from package.json or env.\n"
                "- [ ] VIES lookup confirms the supplier VAT."),
             references="PLAN.md §5.1."),
        Bead("T-103", "t-103-typescript-sdk", "TypeScript SDK (@invoicekit/core, /render, /managed)",
             "feature", 1, ["T-023", "T-024"], lbl("track-9", "dx", "sdk", "typescript"),
             background="Three packages: core (pure), render, managed.",
             goal="All three packages published to npm.",
             acceptance=(
                "- [ ] Each package publishes to npm under `@invoicekit/{core,render,managed}`.\n"
                "- [ ] TypeScript types from T-012.\n"
                "- [ ] Tests pass on Node, Deno, Bun."),
             references="PLAN.md §5.2."),
        Bead("T-104", "t-104-python-sdk", "Python SDK (pyo3 + maturin)",
             "feature", 1, ["T-023", "T-024"], lbl("track-9", "dx", "sdk", "python"),
             background="Python wheel via pyo3 + maturin.",
             goal="`invoicekit` Python package on PyPI.",
             acceptance=(
                "- [ ] Wheels built for cpython 3.10, 3.11, 3.12.\n"
                "- [ ] Tested with pytest.\n"
                "- [ ] Published to PyPI."),
             references="PLAN.md §5.2."),
        Bead("T-105", "t-105-java-sdk", "Java SDK (JNI/FFM over C ABI, with REST sidecar fallback)",
             "feature", 1, ["T-023", "T-024"], lbl("track-9", "dx", "sdk", "java"),
             background="Java SDK with Foreign Function and Memory API (Java 22+) or JNI fallback.",
             goal="Maven artifact published.",
             acceptance=(
                "- [ ] Builds for Java 17, 21, 22.\n"
                "- [ ] REST sidecar fallback if native loading fails.\n"
                "- [ ] Published to Maven Central."),
             references="PLAN.md §5.2."),
        Bead("T-106", "t-106-dotnet-sdk", ".NET SDK (P/Invoke over C ABI, with REST sidecar fallback)",
             "feature", 1, ["T-023", "T-024"], lbl("track-9", "dx", "sdk", "dotnet"),
             background=".NET SDK via P/Invoke.",
             goal="NuGet package published.",
             acceptance=(
                "- [ ] Builds for .NET 8 LTS.\n"
                "- [ ] REST fallback.\n"
                "- [ ] Published to NuGet."),
             references="PLAN.md §5.2."),
        Bead("T-107", "t-107-go-sdk", "Go SDK (cgo with REST sidecar fallback)",
             "feature", 1, ["T-023", "T-024"], lbl("track-9", "dx", "sdk", "go"),
             background="Go SDK via cgo.",
             goal="Go module published.",
             acceptance=(
                "- [ ] cgo binding works on linux, darwin, windows.\n"
                "- [ ] REST fallback for pure-Go contexts.\n"
                "- [ ] Module published."),
             references="PLAN.md §5.2."),
        Bead("T-108", "t-108-wasm-browser-bundle", "Browser bundle (wasm-bindgen)",
             "feature", 1, ["T-025"], lbl("track-9", "dx", "sdk", "wasm"),
             background="WebAssembly browser bundle.",
             goal="`@invoicekit/wasm` published to npm.",
             acceptance=(
                "- [ ] Bundle size < 5 MB with default features.\n"
                "- [ ] Works in Cloudflare Workers, Deno, Bun, browsers.\n"
                "- [ ] Published."),
             references="PLAN.md §5.2."),
        Bead("T-109", "t-109-rest-shim-axum", "REST shim (Axum)",
             "feature", 1, ["T-023", "T-031"], lbl("track-9", "dx", "rest"),
             background="Thin HTTP shim over the engine. See PLAN.md §5.5.",
             goal="Axum-based REST shim with the endpoints from §5.5.",
             acceptance=(
                "- [ ] All endpoints from §5.5 implemented.\n"
                "- [ ] OpenAPI 3.1 specification generated.\n"
                "- [ ] Tests via http client."),
             references="PLAN.md §5.5."),
        Bead("T-109a", "t-109a-openapi-3-1-spec", "OpenAPI 3.1 specification auto-generated from Rust types",
             "task", 1, ["T-109"], lbl("track-9", "dx", "openapi"),
             background="Auto-generated from Rust types via utoipa. The spec is the contract.",
             goal="OpenAPI 3.1 spec served at `https://api.invoicekit.org/openapi.json`.",
             acceptance=(
                "- [ ] Generated on every release with content hash.\n"
                "- [ ] Validates with openapi-spec-validator.\n"
                "- [ ] Customers can generate bindings from it."),
             references="PLAN.md §6 Track 9 T-109a."),
        Bead("T-110", "t-110-reverse-proxy-sidecar", "Reverse-proxy sidecar container",
             "task", 1, ["T-109"], lbl("track-9", "dx", "sidecar"),
             background="For JVM/.NET enterprises that won't accept native binding.",
             goal="A containerized HTTP sidecar that wraps the engine.",
             acceptance=(
                "- [ ] Dockerfile in `bindings/rest-shim/`.\n"
                "- [ ] Health check endpoint.\n"
                "- [ ] Tests."),
             references="PLAN.md §2.1."),
        Bead("T-111", "t-111-invoice-language-server", "Invoice language server (LSP)",
             "feature", 1, ["T-031", "T-032"], lbl("track-9", "dx", "lsp"),
             background="Language Server Protocol implementation. Hover BT terms, diagnostics, autocomplete code lists.",
             goal="A working LSP server.",
             acceptance=(
                "- [ ] Hover on BT-* terms shows the EN 16931 explanatory text.\n"
                "- [ ] Diagnostics on save.\n"
                "- [ ] Code-action quick fixes."),
             references="PLAN.md §5.4."),
        Bead("T-112", "t-112-ide-extensions", "VS Code, Cursor, Neovim, Helix extensions",
             "task", 2, ["T-111"], lbl("track-9", "dx", "lsp", "ide"),
             background="IDE integrations for the LSP.",
             goal="Extensions published in the respective marketplaces.",
             acceptance=(
                "- [ ] VS Code extension published.\n"
                "- [ ] Cursor extension published.\n"
                "- [ ] Neovim and Helix configuration documented."),
             references="PLAN.md §5.4."),
        Bead("T-113", "t-113-nextra-docs-site", "Documentation site (Nextra) with per-rule and per-country pages",
             "feature", 1, ["T-031"], lbl("track-9", "dx", "docs"),
             background="The docs site. Per-rule pages for SEO; per-country guides.",
             goal="`docs.invoicekit.org` live with all rule and country pages.",
             acceptance=(
                "- [ ] Per-rule pages for every EN 16931 rule.\n"
                "- [ ] Per-country guides for every country we support.\n"
                "- [ ] Search works.\n"
                "- [ ] Live."),
             references="PLAN.md §5.4."),
        Bead("T-114", "t-114-storybook-templates", "Storybook for templates",
             "task", 2, ["T-051"], lbl("track-9", "dx", "templates"),
             background="Storybook preview for invoice templates.",
             goal="Storybook with all default templates.",
             acceptance=(
                "- [ ] Every template visible in Storybook.\n"
                "- [ ] Variants: with allowances, with reverse charge, etc."),
             references="PLAN.md §6 Track 9 T-114."),
        Bead("T-115", "t-115-github-actions", "GitHub Actions for invoice validation",
             "task", 2, ["T-035"], lbl("track-9", "dx", "ci"),
             background="GitHub Action customers can drop into their CI.",
             goal="`invoicekit/validate-action@v1` published.",
             acceptance=(
                "- [ ] Action validates any invoice files committed to a repo.\n"
                "- [ ] Published on the GitHub Marketplace."),
             references="PLAN.md §6 Track 9 T-115."),
        Bead("T-116", "t-116-mcp-server", "Model Context Protocol server",
             "task", 2, ["T-031"], lbl("track-9", "dx", "mcp", "ai"),
             background="MCP server for AI dev tools (Claude Code, Cursor, Aider, Continue).",
             goal="An MCP server exposing every engine operation.",
             acceptance=(
                "- [ ] Server implements the MCP spec.\n"
                "- [ ] Documented operations: validate, render, send, verify, capabilities."),
             references="PLAN.md §5.5."),
    ]


def track10_conformance() -> List[Bead]:
    return [
        Bead("T-120", "t-120-corpus-licensing-policy", "Corpus licensing/redaction policy, fixture metadata schema",
             "task", 1, ["T-002"], lbl("track-10", "conformance", "licensing"),
             background="Policy doc that gates how we collect, redact, license, and publish fixtures.",
             goal="A policy doc + schema for fixture metadata.",
             acceptance=(
                "- [ ] Policy doc in `docs/CONFORMANCE-LICENSING.md`.\n"
                "- [ ] Fixture metadata schema (JSON Schema) committed.\n"
                "- [ ] At least 3 sample fixtures with full metadata."),
             references="PLAN.md §3.3 Phase 7."),
        Bead("T-121", "t-121-adversarial-generator", "Adversarial generator (Rust)",
             "feature", 1, ["T-010", "T-040", "T-041"], lbl("track-10", "conformance", "fuzzing"),
             background="Generates pathological invoices in IR and emits via every serializer for differential testing.",
             goal="A Rust crate that emits adversarial invoices.",
             acceptance=(
                "- [ ] Covers edge cases: zero amount, negative amount, allowances > totals, mixed VAT.\n"
                "- [ ] Emits via every serializer.\n"
                "- [ ] Plug into T-123 differential harness."),
             references="PLAN.md §6 Track 10 T-121."),
        Bead("T-122", "t-122-synthetic-public-corpus", "Synthetic public corpus version 0.5 (500+ adversarial invoices)",
             "task", 1, ["T-121"], lbl("track-10", "conformance", "corpus"),
             background="500+ adversarial invoices generated from T-121, committed under CC0 / Apache 2.0.",
             goal="At least 500 fixtures committed in `conformance-corpus/synthetic/`.",
             acceptance=(
                "- [ ] 500+ fixtures across all Family A profiles.\n"
                "- [ ] Each with metadata.\n"
                "- [ ] CC0 or Apache 2.0 licensed."),
             references="PLAN.md §3.3 Phase 7."),
        Bead("T-123", "t-123-differential-test-harness", "Differential test harness",
             "feature", 1, ["T-030", "T-031", "T-032", "T-040", "T-041"], lbl("track-10", "conformance", "differential"),
             background="Runs all serializers + both pure-Rust and reference-worker validators against the corpus; diffs results; publishes parity dashboard.",
             goal="Differential harness with public parity dashboard.",
             acceptance=(
                "- [ ] Compares pure-Rust validator vs JVM sidecars.\n"
                "- [ ] Parity dashboard published to `parity.invoicekit.org`.\n"
                "- [ ] Tracks parity over time."),
             references="PLAN.md §6 Track 10 T-123."),
        Bead("T-124", "t-124-benchmark-dashboard", "Public benchmark dashboard",
             "task", 2, ["T-123"], lbl("track-10", "performance", "dashboard"),
             background="Public benchmark dashboard for engine operations.",
             goal="`benchmark.invoicekit.org` is live and updated on every release.",
             acceptance=(
                "- [ ] Dashboard live.\n"
                "- [ ] Per-operation history visible."),
             references="PLAN.md §6 Track 10 T-124, §6 T-007."),
    ]


def track11_managed_layer() -> List[Bead]:
    return [
        Bead("T-130", "t-130-tenant-model", "Tenant model, scoped API keys, OIDC, RBAC, audit-event schema",
             "feature", 0, ["T-001"], lbl("track-11", "managed", "auth", "critical-path", "p0"),
             background="Per-tenant isolation from day one.",
             goal="Full tenant model + scoped keys + OIDC + RBAC + audit event schema.",
             acceptance=(
                "- [ ] Tenant ID propagates everywhere.\n"
                "- [ ] Scoped API keys with explicit scopes.\n"
                "- [ ] OIDC SSO works with at least one provider (Google).\n"
                "- [ ] RBAC with at least 3 roles (admin, member, viewer).\n"
                "- [ ] Audit event schema documented and tested."),
             references="PLAN.md §3.3 Phase 6, §6 Track 11 T-130."),
        Bead("T-131", "t-131-envelope-encryption-kms", "Envelope encryption with KMS-per-tenant",
             "task", 1, ["T-130"], lbl("track-11", "managed", "encryption", "kms"),
             background="Per-tenant KMS, key rotation, data residency tags.",
             goal="Envelope encryption with KMS per tenant.",
             acceptance=(
                "- [ ] AWS KMS adapter; future: Azure KMS, GCP KMS.\n"
                "- [ ] Key rotation tested.\n"
                "- [ ] Data residency tag (EU / US / global) honored at write time."),
             references="PLAN.md §6 Track 11 T-131."),
        Bead("T-132", "t-132-webhook-signing", "Webhook signing (HMAC-SHA256, Stripe-shape)",
             "task", 1, ["T-130"], lbl("track-11", "managed", "webhooks", "security"),
             background="HMAC-SHA256, `InvoiceKit-Signature: t=<unix>,v1=<hex>` header.",
             goal="Webhook signing and replay protection.",
             acceptance=(
                "- [ ] Signature format matches the documented schema.\n"
                "- [ ] Replay protection via timestamp + 5-min window.\n"
                "- [ ] Tested against a sample receiver."),
             references="PLAN.md §6 Track 11 T-132."),
        Bead("T-133", "t-133-sbom-signed-releases", "SBOM, dependency scanning, signed releases, security advisory process",
             "task", 1, ["T-002"], lbl("track-11", "managed", "security", "sbom"),
             background="Hosted-layer operational security baseline (distinct from T-002 which is repo-level).",
             goal="Hosted layer has its own SBOM pipeline + dependency scans + signing.",
             acceptance=(
                "- [ ] SBOM generated on every deploy.\n"
                "- [ ] Dependency scans gate deploys.\n"
                "- [ ] Security advisory process documented."),
             references="PLAN.md §6 Track 11 T-133."),
        Bead("T-134", "t-134-api-gateway-rate-limiting", "API gateway, authentication, rate limiting",
             "feature", 1, ["T-130"], lbl("track-11", "managed", "api"),
             background="The customer-facing API gateway.",
             goal="An Axum-based API gateway with auth and rate limiting.",
             acceptance=(
                "- [ ] All `/v1/*` endpoints behind the gateway.\n"
                "- [ ] Auth via scoped API keys or OIDC.\n"
                "- [ ] Per-tenant rate limits."),
             references="PLAN.md §6 Track 11 T-134."),
        Bead("T-135", "t-135-customer-dashboard", "Customer dashboard (audit log, usage, errors)",
             "feature", 1, ["T-130"], lbl("track-11", "managed", "dashboard", "ui"),
             background="The customer-facing dashboard.",
             goal="A web dashboard for customers to see their data.",
             acceptance=(
                "- [ ] Audit log view.\n"
                "- [ ] Usage view (envelopes, errors).\n"
                "- [ ] Search and filter."),
             references="PLAN.md §6 Track 11 T-135."),
        Bead("T-136", "t-136-opentelemetry", "OpenTelemetry tracing, metrics, log redaction, per-gateway dashboards",
             "feature", 1, ["T-072"], lbl("track-11", "observability", "telemetry"),
             background="Full observability stack.",
             goal="Per-tenant + per-gateway tracing and metrics.",
             acceptance=(
                "- [ ] OTel traces on every request.\n"
                "- [ ] Metrics for SLO operations: validate, render, transmit-enqueue, gateway-accepted, archive-write, webhook-deliver.\n"
                "- [ ] PII redaction in logs."),
             references="PLAN.md §4.12."),
        Bead("T-137", "t-137-replay-admin-tooling", "Replay and admin tooling for stuck transmissions",
             "task", 1, ["T-136"], lbl("track-11", "observability", "admin"),
             background="Ops tooling for stuck transmissions and dead-letter queues.",
             goal="Admin CLI + UI to inspect and replay stuck transmissions.",
             acceptance=(
                "- [ ] CLI: `invoicekit-admin stuck`, `invoicekit-admin replay <id>`.\n"
                "- [ ] UI in the customer dashboard."),
             references="PLAN.md §6 Track 11 T-137."),
        Bead("T-138", "t-138-status-page", "Status page and incident tooling",
             "task", 2, ["T-136"], lbl("track-11", "ops", "status"),
             background="Public status page with per-country and per-gateway uptime.",
             goal="`status.invoicekit.org` live.",
             acceptance=(
                "- [ ] Live status page.\n"
                "- [ ] Per-country and per-gateway uptime.\n"
                "- [ ] Incident posting integrated."),
             references="PLAN.md §6 Track 11 T-138."),
        Bead("T-139", "t-139-support-ticket-integration", "Support ticket integration",
             "task", 2, ["T-135"], lbl("track-11", "ops", "support"),
             background="Customers can file tickets from the dashboard.",
             goal="In-app ticket filing.",
             acceptance=(
                "- [ ] In-app form to file a ticket.\n"
                "- [ ] Integration with a ticket system (Linear, Zendesk, or similar)."),
             references="PLAN.md §6 Track 11 T-139."),
        Bead("T-140", "t-140-stripe-for-our-own-billing", "Stripe integration for our own customer invoicing",
             "task", 2, ["T-130"], lbl("track-11", "ops", "billing"),
             background="We use Stripe for our own SaaS billing.",
             goal="Stripe wired up for our own customer subscriptions.",
             acceptance=(
                "- [ ] Subscription plans defined.\n"
                "- [ ] Webhook handler for invoice events.\n"
                "- [ ] Customer self-service portal."),
             references="PLAN.md §6 Track 11 T-140."),
        Bead("T-141", "t-141-hot-reloadable-rule-packs", "Hot-reloadable rule packs",
             "task", 1, ["T-017", "T-018"], lbl("track-11", "managed", "ops"),
             background="Managed service picks up signed rule pack updates without restart.",
             goal="Rule pack hot-reload via inotify + atomic file swap.",
             acceptance=(
                "- [ ] Inotify watcher implemented.\n"
                "- [ ] Atomic file swap tested.\n"
                "- [ ] No transmission interrupted by reload."),
             references="PLAN.md §6 Track 11 T-141."),
        Bead("T-142", "t-142-customer-audit-log-api", "Customer-facing audit log API",
             "feature", 1, ["T-130", "T-136"], lbl("track-11", "audit", "api"),
             background="Customers query every action taken on their data, exportable as CSV or JSON, signed.",
             goal="`GET /v1/audit/events` with pagination + filtering.",
             acceptance=(
                "- [ ] Endpoint implemented.\n"
                "- [ ] Pagination + filtering.\n"
                "- [ ] Signed export verified."),
             references="PLAN.md §6 Track 11 T-142."),
    ]


def track12_billing_bridges() -> List[Bead]:
    bridges = [
        ("T-1200", "stripe-invoicing", "Stripe Invoicing", "Listens to Stripe invoice events; translates to CommercialDocument; renders + transmits via engine; writes the receipt back to Stripe metadata."),
        ("T-1201", "lago", "Lago", "Lago invoice event bridge."),
        ("T-1202", "maxio", "Maxio (Chargify + SaaSOptics)", "Maxio bridge."),
        ("T-1203", "chargebee", "Chargebee", "Chargebee bridge."),
        ("T-1204", "recurly", "Recurly", "Recurly bridge."),
    ]
    beads = []
    for tid, slug_b, name, desc in bridges:
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-bridge-{slug_b}",
            title=f"Billing-platform bridge: {name}",
            type="feature", priority=2,
            deps=["T-031", "T-072", "T-091"],
            labels=lbl("track-12", "bridge", "distribution", slug_b),
            background=f"{desc} See PLAN.md §6 Track 12.",
            goal=f"A working {name} → InvoiceKit bridge.",
            acceptance=(
                "- [ ] Webhook listener for the platform's invoice events.\n"
                "- [ ] Transformer to CommercialDocument with LossinessLedger entries.\n"
                "- [ ] Sends via the engine + receives receipt.\n"
                "- [ ] Receipt writes back to the platform (where supported).\n"
                "- [ ] End-to-end test against a sandbox account."),
            notes="Lives in `bridges/`.",
            references=f"PLAN.md §6 Track 12 {tid}.",
        ))
    return beads


def track13_deployment() -> List[Bead]:
    return [
        Bead("T-1300", "t-1300-docker-compose", "Single-host docker-compose for the full managed stack",
             "feature", 1, ["T-130", "T-030", "T-083"], lbl("track-13", "deploy", "docker"),
             background="One file brings up the entire stack: Postgres, all JVM validator sidecars, signer-agent, archive backend, managed API.",
             goal="`docker compose up` works for the entire stack.",
             acceptance=(
                "- [ ] All services start.\n"
                "- [ ] Smoke tests pass against the running stack.\n"
                "- [ ] Documented in `deploy/README.md`."),
             references="PLAN.md §6 Track 13."),
        Bead("T-1301", "t-1301-helm-chart", "Kubernetes Helm chart",
             "feature", 1, ["T-1300"], lbl("track-13", "deploy", "kubernetes", "helm"),
             background="Production-grade multi-node deployment.",
             goal="`helm install invoicekit ./deploy/helm` works.",
             acceptance=(
                "- [ ] Chart validates with helm lint.\n"
                "- [ ] Smoke test in a kind cluster.\n"
                "- [ ] Configurable replicas, resources, storage."),
             references="PLAN.md §6 Track 13."),
        Bead("T-1302", "t-1302-terraform-module", "Terraform module for managed-cloud provisioning",
             "feature", 2, ["T-1301"], lbl("track-13", "deploy", "terraform"),
             background="AWS / Azure / GCP terraform module.",
             goal="A terraform module that provisions the full stack on a cloud.",
             acceptance=(
                "- [ ] Module published in the terraform registry.\n"
                "- [ ] AWS example runs end-to-end."),
             references="PLAN.md §6 Track 13."),
    ]


def track14_demo_apps() -> List[Bead]:
    demos = [
        ("T-1400", "nextjs", "Next.js", "T-103"),
        ("T-1401", "django", "Django", "T-104"),
        ("T-1402", "rails", "Rails (via REST shim)", "T-109"),
        ("T-1403", "spring-boot", "Spring Boot", "T-105"),
        ("T-1404", "asp-net", "ASP.NET", "T-106"),
        ("T-1405", "laravel", "Laravel (via REST shim)", "T-109"),
        ("T-1406", "fastapi", "FastAPI", "T-104"),
        ("T-1407", "go-chi", "Go (chi)", "T-107"),
    ]
    beads = []
    for tid, slug_d, name, dep in demos:
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-demo-{slug_d}",
            title=f"Reference demo app: {name}",
            type="task", priority=3,
            deps=[dep],
            labels=lbl("track-14", "demo", "distribution", slug_d),
            background=f"Reference {name} app that lands a German XRechnung in under 5 minutes from clone.",
            goal=f"A working {name} demo app.",
            acceptance=(
                "- [ ] Repo lives in `examples/`.\n"
                "- [ ] README has copy-paste setup that works in under 5 minutes.\n"
                "- [ ] At least 3 example invoices issued + validated end-to-end.\n"
                "- [ ] CI runs the demo on every PR."),
            references=f"PLAN.md §6 Track 14 {tid}.",
        ))
    return beads


def track15_erp_connectors() -> List[Bead]:
    connectors = [
        ("T-1500", "odoo", "Odoo", "T-104", 2),
        ("T-1501", "ms-dynamics", "Microsoft Dynamics 365", "T-106", 3),
        ("T-1502", "sap-b1", "SAP Business One", "T-105", 3),
        ("T-1503", "lexware", "Lexware (German market)", "T-109", 2),
        ("T-1504", "sage", "Sage", "T-109", 2),
        ("T-1505", "sevdesk", "sevDesk (German market)", "T-109", 1),
    ]
    beads = []
    for tid, slug_c, name, dep, weeks in connectors:
        beads.append(Bead(
            tid=tid, slug=f"{tid.lower().replace('-', '-')}-connector-{slug_c}",
            title=f"ERP connector: {name}",
            type="feature", priority=3,
            deps=[dep],
            labels=lbl("track-15", "connector", "distribution", slug_c),
            background=f"{name} addon/extension that lets non-technical users adopt e-invoicing in {name}.",
            goal=f"A working {name} connector packaged for the host marketplace.",
            acceptance=(
                "- [ ] Connector packaged for the host platform.\n"
                "- [ ] Published in the host marketplace.\n"
                f"- [ ] At least one demo invoice issued + transmitted via the connector.\n"
                "- [ ] Tested end-to-end."),
            notes=f"Lives in `connectors/{slug_c}/`. Effort: ~{weeks} weeks.",
            references=f"PLAN.md §6 Track 15 {tid}.",
        ))
    return beads


# ────────────────────────────────────────────────────────────────────────────
# br invocation glue.
# ────────────────────────────────────────────────────────────────────────────


def br_create(bead: Bead) -> str:
    """Create one bead. Returns the assigned br-id."""
    cmd = [
        "br", "create",
        "--silent",
        "--slug", bead.slug,
        "--type", bead.type,
        "--priority", str(bead.priority),
        "--description", bead.render_body(),
    ]
    if bead.labels:
        cmd += ["--labels", ",".join(bead.labels)]
    cmd.append(bead.title)
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def br_dep_add(child: str, parent: str) -> None:
    subprocess.run(["br", "dep", "add", child, parent], capture_output=True, text=True, check=True)


def br_update_description(brid: str, body: str) -> None:
    subprocess.run(["br", "update", brid, "--description", body], capture_output=True, text=True, check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="Don't create; print what would be created.")
    parser.add_argument("--validate-only", action="store_true", help="Run validation only.")
    parser.add_argument("--update-bodies", action="store_true", help="Re-render existing beads' bodies with the current template; calls br update.")
    args = parser.parse_args()

    if args.validate_only:
        cycles = subprocess.run(["br", "dep", "cycles"], capture_output=True, text=True).stdout
        print(f"=== br dep cycles ===\n{cycles}")
        insights = subprocess.run(["bv", "--robot-insights"], capture_output=True, text=True).stdout
        print(f"=== bv insights ===\n{insights[:2000]}")
        return 0

    all_beads: List[Bead] = []
    all_beads.extend(track0_foundation())
    all_beads.extend(track1_engine_primitives())
    all_beads.extend(track2_reference_validator())
    all_beads.extend(track3_format_family_a())
    all_beads.extend(track4_rendering())
    all_beads.extend(track5_intake())
    all_beads.extend(track6_reconciliation_evidence())
    all_beads.extend(track7_peppol_live())
    all_beads.extend(track7_5_country_manifests())
    all_beads.extend(track8_country_crates())
    all_beads.extend(track9_dx_surface())
    all_beads.extend(track10_conformance())
    all_beads.extend(track11_managed_layer())
    all_beads.extend(track12_billing_bridges())
    all_beads.extend(track13_deployment())
    all_beads.extend(track14_demo_apps())
    all_beads.extend(track15_erp_connectors())

    if args.check:
        for b in all_beads:
            print(f"{b.tid:<10} {b.type:<8} p{b.priority} {b.title}")
        print(f"\nTotal: {len(all_beads)} beads")
        return 0

    tid_to_brid: dict = {}
    if MAPPING_FILE.exists():
        tid_to_brid = json.loads(MAPPING_FILE.read_text())

    if args.update_bodies:
        # Update mode: re-render every existing bead with the current template.
        print(f"--- updating {len(all_beads)} bead bodies with the latest template ---")
        updated = 0
        for bead in all_beads:
            if bead.tid not in tid_to_brid:
                print(f"[skip] {bead.tid} not in mapping; skipping")
                continue
            try:
                br_update_description(tid_to_brid[bead.tid], bead.render_body())
                updated += 1
                if updated % 25 == 0:
                    print(f"  updated {updated} / {len(all_beads)}")
            except subprocess.CalledProcessError as e:
                print(f"[fail] {bead.tid}: {e.stderr}", file=sys.stderr)
                return 1
        print(f"Updated {updated} beads.")
        return 0

    # Phase 1: create all beads.
    for bead in all_beads:
        if bead.tid in tid_to_brid:
            print(f"[skip] {bead.tid} already created as {tid_to_brid[bead.tid]}")
            continue
        try:
            brid = br_create(bead)
        except subprocess.CalledProcessError as e:
            print(f"[fail] {bead.tid}: {e.stderr}", file=sys.stderr)
            return 1
        tid_to_brid[bead.tid] = brid
        print(f"{bead.tid:<10} → {brid}: {bead.title}")
        MAPPING_FILE.parent.mkdir(parents=True, exist_ok=True)
        MAPPING_FILE.write_text(json.dumps(tid_to_brid, indent=2))

    # Phase 2: wire deps.
    print("\n--- wiring dependencies ---")
    dep_count = 0
    for bead in all_beads:
        for dep_tid in bead.deps:
            if dep_tid not in tid_to_brid:
                print(f"[warn] {bead.tid} depends on unknown {dep_tid}; skipping", file=sys.stderr)
                continue
            child = tid_to_brid[bead.tid]
            parent = tid_to_brid[dep_tid]
            try:
                br_dep_add(child, parent)
                dep_count += 1
            except subprocess.CalledProcessError as e:
                if "already exists" in (e.stderr or ""):
                    continue
                print(f"[fail dep] {bead.tid} → {dep_tid}: {e.stderr}", file=sys.stderr)
                return 1
    print(f"Wired {dep_count} dependency edges.")

    # Phase 3: validate.
    print("\n--- validation ---")
    cycles = subprocess.run(["br", "dep", "cycles"], capture_output=True, text=True).stdout
    print(f"Cycles: {cycles.strip() or '(none)'}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
