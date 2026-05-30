<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-migration

Typed forward-migration framework for the InvoiceKit intermediate-representation (IR) schema version family.

When `invoicekit_ir::SchemaVersion` grows a new variant, invoices archived on customer disks under an older variant must keep loading. This crate carries the framework that takes a JSON document tagged with a known source version and lifts it to a requested target version, recording every field it could not lift cleanly in a `MigrationReport`.

## Capabilities

- `migrate(value, target)` — convenience entry point. Reads the document's root `schema_version`, looks up a migration path in the default seeded registry, applies it, and returns `(migrated_value, MigrationReport)`.
- `Registry` — in-memory store of `Migration` steps. `Registry::seeded()` returns a registry pre-loaded with the migrations InvoiceKit ships; `register` adds more; `migrate` runs one. Migrations are stored in a `Vec` and matched by linear scan on `(source_version, target_version)` (because `SchemaVersion` derives neither `Ord` nor `Hash`).
- `Migration` trait — one concrete step. Reports its `source_version`, `target_version`, whether it is `reversible`, and an `apply` that lifts the value and appends findings.
- `IdentityV1ToV1` — the only `Migration` registered today. `V1_0 → V1_0`, always reversible, returns the input value unchanged.
- `MigrationReport` — `from`, `to`, `reversible`, and a `Vec<MigrationFinding>`. `is_clean()` is true when no findings were recorded.
- `MigrationFinding` — per-field outcome: a JSON Pointer `path`, a machine code `kind` (for example `field-dropped`, `value-coerced`), a human `message`, and an optional `remediation` hint.
- `MigrationError` — `MissingSourceVersion`, `UnknownSourceVersion(String)`, `UnknownTargetVersion { from, to }`, `Json`.
- `crate_name()` — returns `"invoicekit-migration"`.

`MigrationReport`, `MigrationFinding`, and `SchemaVersion` are serde-serializable, so a report can be embedded in audit output.

## Mode / Residuals

This is a framework that ships ahead of the data it migrates. The IR currently exposes exactly one `SchemaVersion` variant, `V1_0`. No two-version pair exists yet, so today every well-formed `migrate` call is one of:

- An identity no-op (`V1_0 → V1_0`) handled by `IdentityV1ToV1`, returning the document byte-for-byte unchanged with a clean, reversible report.
- A `MigrationError::UnknownTargetVersion` when no registered step reaches the requested target.

There is **no** real field-transforming migration in this crate today, because there is no older or newer schema to transform between. The trait, the typed report, the reversibility marker, and the registry exist so that when a `V1_1` or `V2_0` variant lands, a single `Migration` implementation can be written and registered without reworking the surrounding machinery.

Other documented limitations:

- The registry picks the **first** registered migration matching `(source, target)`; it does not compose a chain of intermediate steps. Multi-hop migration (for example `V1_0 → V1_1 → V2_0`) would need to be added when more than two versions exist.
- `reversible` and the findings list are produced by each `Migration` implementation; the framework does not verify reversibility or check for information loss on its own. The identity migration is reversible by construction.
- Source version is read only from a root `schema_version` string field. A missing field yields `MissingSourceVersion`; an unparseable value yields `UnknownSourceVersion`.

This crate is `publish = false` and depends only on `invoicekit-ir`, `serde`, `serde_json`, and `thiserror`. It exposes no command-line interface and ships no binary.

## References

No external specifications or standards are referenced in the source. The crate's only domain dependency is the InvoiceKit IR (`invoicekit-ir`) and its `SchemaVersion` type.

## License

Apache-2.0.
