# Contributing to InvoiceKit

Thanks for your interest in InvoiceKit. This guide is the short version; the
long version (architectural commitments, do/don't lists, agent rules) lives in
[`AGENTS.md`](./AGENTS.md). Read that first if you intend to write code.

## Before you start

1. Read [`README.md`](./README.md) for what the project is.
2. Read [`AGENTS.md`](./AGENTS.md) for the safety rules and architectural
   commitments. These are **not** suggestions.
3. Skim [`plans/PLAN.md`](./plans/PLAN.md) so you know which crate your change
   touches and which bead owns the area.

## How work is tracked

We track work as **beads** in a local-first issue graph using the `br` and
`bv` tooling. Open work and dependencies are computed from the graph; we do
not use a separate ticket tracker for engineering work.

```sh
br ready --json            # what is unblocked right now
bv --robot-next            # the single top-ranked unblocked bead
bv --robot-triage          # full triage view: scores, blockers, dependencies
br show <bead-id>          # full bead body, including its acceptance gates
```

Every code change must reference its bead. The bead body is the contract
the reviewer holds the change to.

## Pull request checklist

A change is ready for review when **all** of the following are true:

- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace` passes (happy path + ≥ 3 failure-mode tests
      where the change introduces a new public API surface).
- [ ] `cargo doc --workspace --no-deps` builds without warnings.
- [ ] Bead acceptance gates (universal + type-specific + bead-specific) are
      explicitly walked in the PR description. Waived gates carry a
      one-line rationale.
- [ ] The PR description quotes the bead ID and a one-line summary of what
      shipped.

CI runs the same set on every push and pull request.

## Local quickstart

```sh
just build           # cargo build --workspace --all-targets
just test            # cargo test --workspace
just lint            # cargo clippy --workspace --all-targets -- -D warnings
just fmt             # cargo fmt --all
just fmt-check       # cargo fmt --all --check
just audit           # cargo audit
just deny            # cargo deny check (requires cargo-deny)
just ci              # the whole CI matrix in one shot, locally
```

## Coding rules (short version — see `AGENTS.md` for the full list)

- **Apache 2.0 everywhere.** Do not introduce AGPL, SSPL, or other copyleft
  licenses.
- **No floating-point money.** Use the `invoicekit-money` crate.
- **No unsafe.** The workspace forbids `unsafe_code` at the lint level;
  any exception requires a documented rationale and review.
- **Deterministic output.** Anything that gets signed, hashed, archived, or
  compared elsewhere must be byte-identical across runs on the same input.
- **No bulk codemods.** Hand-edit; small, reviewable diffs.
- **No file deletion without explicit permission** in the session. See
  `AGENTS.md` Rule 1.

## Security disclosures

See [`SECURITY.md`](./SECURITY.md). Do not file public issues for security
vulnerabilities.

## License

By contributing, you agree that your contributions will be licensed under the
Apache License 2.0, the same license as the project (see [`LICENSE`](./LICENSE)).
