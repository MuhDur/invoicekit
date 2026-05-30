<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-cli

The InvoiceKit workspace command-line surface. Every `invoicekit <subcommand>` runner lives here as a reusable library function so the published `invoicekit` binary and the per-subcommand shim binaries dispatch through the same code path.

The crate is `publish = false`; it ships the binaries, not a library API for downstream crates. The only public library item is `crate_name()`, a `const fn` returning `"invoicekit-cli"` used by release tooling and bead-correlation reports.

## Binaries

- `invoicekit` — the umbrella binary; dispatches `argv[1]` to a subcommand runner.
- `migrate-archive` — thin shim onto the same runner as `invoicekit migrate-archive`.
- `gen-schema` — prints the JSON Schema for `invoicekit_ir::CommercialDocument` to stdout (CI re-derives and diffs it).
- `invoicekit-admin` — operator tooling over a SQLite outbox database.

## Subcommands (`invoicekit <command>`)

- `capabilities` — resolve which e-invoice profiles a route/scenario/date accepts, against a compile-time-bundled matrix (`data/capabilities/matrix.json`). Filters on route, scenario, and validity window; flags stale matches via `warnings[]`; attempts a scenario auto-downgrade; otherwise returns `status: "no_data"`. Output is a stable JSON envelope, with a `--format=pretty` printer.
- `codelist-update` — refresh a code-list manifest from a locally-staged upstream file. Network-free by design (the nightly workflow does the `curl`); atomic write via temp-file + `fsync` + `rename`, with a "no change" branch that leaves the file untouched.
- `diff` — compare two `.ikb` evidence bundles artefact-by-artefact (`byte-equal`, `changed`, `only-in-left`, `only-in-right`) using BLAKE3 content hashes. Exit `0` when byte-equal, `1` when any artefact differs.
- `doctor` — local environment diagnostics: Rust toolchain present, workspace layout, code-list data tree, and TCP reachability probes against the validator and signer sidecar ports on `127.0.0.1`.
- `init` — scaffold a starter `invoicekit/` directory (a typed `draft.json` plus `config.toml`), detecting the host language/framework from marker files (Node, Python, Go, Java, .NET, Rust).
- `migrate-archive` — walk a directory of invoice JSON files, check each `schema_version` against `--from-version`, lift to `--to-version` via `invoicekit_migration::migrate`, and rewrite in place.
- `pack` — walk an input directory and pack every regular file into a deterministic `.ikb` evidence bundle.
- `unpack` — inverse of `pack`: extract every artefact from a `.ikb` bundle to disk.
- `show` — read-only manifest + per-artefact summary of a `.ikb` bundle (human or `--json`).
- `peppol doctor` / `peppol show` — load a bring-your-own-key Peppol credentials JSON file and run `PeppolDoctor` checks (cert/key existence, PEM shape, endpoint URL, participant id), or pretty-print the credentials shape without revealing passphrases.
- `timestamp` — request an RFC 3161 timestamp for a bundle's manifest. See Mode below.
- `validate` — validate UBL 2.1 Invoice/CreditNote or UN/CEFACT CII XML with the native EN 16931 rule set; `--explain` emits an ordered rule-evaluation trace with paths, inputs, decisions, and citations.
- `verify` — verify a `.ikb` / `.invoicekit` evidence bundle. See Mode below.
- `replay` — replay a bundle through the replayer and report drift. See Mode below.
- `repl` — open an interactive `rustyline` shell that dispatches the same subcommand runners without retyping the binary name; keeps lightweight session state for the current tenant and draft directory. `--eval <line>` runs a single line and exits.
- `version` — print the binary's name, version, and build profile (`--json` for structured output).

`init` and `peppol` are dispatched by the binary but are not listed in the `invoicekit --help` usage text (see Residuals).

## Admin subcommands (`invoicekit-admin <command>`)

SQLite-only today (Postgres / MySQL adapters are deferred to follow-up beads).

- `stuck` — list outbox rows needing attention, bucketed: `dead-letter`, `retry-overdue` (past `next_attempt_at` by more than `--overdue-mins`, default 15), and `stale-reserved` (held by `reserved_until` longer than `--reserved-mins`, default 5). Output is JSON-lines or `--format=table`.
- `replay` — re-enqueue a dead-letter row into the live outbox. Inserts a fresh `invoicekit_outbox` row reusing the original tenant/trace/idempotency triple with a new `outbox_id` and `gateway_attempt_id`, attempt count zero, `next_attempt_at = now`. The dead-letter row stays as audit trail; the replay is refused if a live row with the same `(tenant_id, idempotency_key)` already exists. `--dry-run` reports without mutating.

## Mode / Residuals

This crate is wiring; the honesty caveats live in the runners that intentionally call placeholder or baseline backends.

- **`timestamp` is a mock.** The only backend wired in is `invoicekit_timestamping::MockTimestampClient`, a deterministic in-process mock that pins `genTime` so replay tests stay byte-identical. It is **not** a real RFC 3161 Time-Stamp Authority. A real signing/timestamping path requires a real TSA HTTP client (named in the source as a T-082 follow-up); the subcommand is meant to switch to real tokens without flag changes once that lands.
- **`verify` is content-only.** It calls `verify_packed` with `VerifyOptions::content_only()` — the BLAKE3 content-address check runs, but detached-signature, DSSE manifest-envelope, and timestamp checks are skipped because the CLI does not yet wire a signer or TSA client (T-100 / T-083a / T-082 follow-ups).
- **`replay` defaults to an identity baseline.** The default `IdentityReplayer` always replays byte-equal — it does not re-run the real pipeline yet (T-100 follow-up). `--mutate <id>` deliberately drifts named artefacts via `MutatingReplayer` so CI can prove its drift-detection wiring exits non-zero.
- **`init` does not call VIES.** Supplier VAT lookup is stubbed (no network); the printed report flags it as `Skipped { reason: "no VIES client wired yet" }` rather than silently passing.
- **`version` does not report commit or rustc.** The emitted `VersionInfo` carries only `name`, `version`, and `build_profile` (`dev` vs `release`, derived from `debug_assertions`). There is no build script to capture a commit hash or rustc version, so neither is surfaced.
- **Admin tooling is SQLite-only.** `open_sqlite` is the only database adapter; other engines are deferred.

The `signer`/`timestamping`/`verify` placeholder details are documented in those crates' own READMEs; this crate inherits their limitations because it calls them.

## References

Specs and standards actually named in the source:

- EN 16931 — European e-invoicing semantic model (the `validate` rule set).
- UBL 2.1 and UN/CEFACT Cross Industry Invoice (CII) — the document syntaxes `validate` accepts.
- RFC 3161 — Time-Stamp Protocol (the format `timestamp` targets; currently served by a mock).
- ISO 3166-1 alpha-2 — country codes used by the `capabilities` matrix.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
