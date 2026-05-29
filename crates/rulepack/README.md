<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 The InvoiceKit Authors
-->

# invoicekit-rulepack

Signed, effective-dated validation rule packs, selected by country, profile, and date.

## What it does

Every InvoiceKit validator needs to know *which* rules apply to a given invoice: the EN 16931 business rules, the Peppol BIS restrictions, the German XRechnung extensions, and so on. Those rule sets change over time and differ by jurisdiction. This crate is the registry that holds them.

A rule pack is a `Manifest`: a JSON envelope carrying the upstream version it tracks, retrieval provenance, the code list versions it pins, an integrity checksum over the body, a pointer to the parity fixtures continuous integration uses to grade it, and a signature. The loader verifies the signature before any rule is handed to a consumer. You ask the `Registry` for "the pack covering Germany, XRechnung 3.0, on 2026-05-26" and it returns the one manifest whose effective window contains that date, preferring an exact country match over a `"global"` fallback.

The manifest itself does not interpret the rules. The `body` field is opaque JSON; the consuming crate (the hand-written `invoicekit-validate-ubl-cii` validator, a JVM validator sidecar, a per-country reporter) decides what the rules mean. This crate's job is selection and integrity.

### Signing status

The signature scheme is pluggable via the `signature_alg` field. Production packs are intended to be signed with Sigstore keyless OIDC or minisign once that operator-owned key setup lands. Until then, the only implemented scheme is `"blake3:identity"`, where the signature is the BLAKE3 digest of the canonical body bytes. That catches accidental tampering of the embedded JSON. It is not a real signature and the code says so plainly. The three built-in seed packs ship with an all-zero placeholder digest over an empty rule body, a deliberate "no tamper, no real signature yet" sentinel pending upstream artifact ingestion.

This is honest about its maturity: the date-selection, integrity-check, and hot-reload machinery is real and tested; the rule bodies are empty seeds and the signing is a placeholder scheme.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
                                               ^
                                               |
                                          rulepack
```

It feeds the **validate** stage. Validators pull their rule set through this crate so that what gets enforced is versioned, date-scoped, and integrity-checked rather than hard-coded.

## Key public API

Crate root (`invoicekit_rulepack`):

- `Manifest` — one rule pack envelope. `Manifest::from_json(raw)` decodes and verifies in one step; `Manifest::verify()` checks the signature scheme, the signature, and the effective window; `Manifest::covers(on_date)` reports whether a date falls inside the window.
- `Registry` — the in-memory set of loaded manifests. `Registry::seeded()` loads the workspace-embedded seed packs; `Registry::insert(manifest)` adds a manifest (verified at insert time); `Registry::pack_for(country, profile, on_date)` performs the selection; plus `iter`, `len`, `is_empty`, and `rulepack_ids`.
- `ParityFixtures`, `GeneratedMetadata` — the typed sub-records on a manifest.
- `RulepackError` — load and verification failures: `UnknownSignatureScheme`, `SignatureMismatch`, `InvalidEffectiveWindow`, `SeedManifestInvalid`, `Json`.

Hot reload (`invoicekit_rulepack::hot_reload`, re-exported at the crate root):

- `HotReloadRegistry` — a file-backed registry behind an `ArcSwap`. `load_from_dir(dir)` reads every `*.json` file; `snapshot()` returns a cheap lock-free `Snapshot` (an `Arc<Registry>`); `reload_now()` re-reads the directory and swaps atomically; `spawn_watcher()` starts a background thread that reloads on filesystem changes via the `notify` crate.
- `Snapshot`, `WatchHandle`, `HotReloadError`.

A failed reload (malformed JSON, bad signature) leaves the previous snapshot in place and surfaces an error, so a long-running service keeps serving from the last known-good set.

## Usage

Select the rule pack that applies to a German XRechnung invoice dated today:

```rust
use invoicekit_rulepack::Registry;

let registry = Registry::seeded()?;

let pack = registry
    .pack_for("DE", "urn:xoev-de:kosit:standard:xrechnung_3.0", "2026-05-26")
    .expect("a pack covers this country/profile/date");

assert!(pack.rulepack_id.contains("xrechnung"));
assert_eq!(pack.parity_fixtures.oracle, "jvm:kosit");
# Ok::<(), invoicekit_rulepack::RulepackError>(())
```

For a long-running service that reloads packs from disk without restarting:

```rust,no_run
use invoicekit_rulepack::HotReloadRegistry;

let registry = HotReloadRegistry::load_from_dir("/etc/invoicekit/rulepacks")?;
let _watcher = std::sync::Arc::clone(&registry).spawn_watcher()?;

// Readers take a cheap snapshot; reloads never block them.
let snapshot = registry.snapshot();
for id in snapshot.rulepack_ids() {
    println!("{id}");
}
# Ok::<(), invoicekit_rulepack::HotReloadError>(())
```

## Status

Workspace member crate (`publish = false`). The selection, integrity, effective-date, and hot-reload logic is implemented and covered by unit and property tests. The shipped rule bodies are seed placeholders and the signing scheme is the placeholder `blake3:identity`; real upstream artifact ingestion and keyless signing are tracked follow-up work.

## License

Apache-2.0. See the workspace root for the full text.
