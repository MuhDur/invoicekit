# invoicekit-codelists

Signed, versioned, effective-dated registry for the code lists that invoice validation depends on.

## What it does

Code lists — country codes, currencies, unit codes, VAT category codes — are data, not constants baked into validation logic. They change over time, and a validator has to answer "was this code valid on the invoice date?", not just "is this code valid today". This crate holds those lists as signed manifests with effective-date windows and answers date-pinned lookups.

Each manifest is one snapshot of one list for a date range. It carries the list name, a version, an effective-from/effective-to window, the upstream source URL, a retrieval timestamp, and a signature over the whole payload. Lookups are pinned to a date: you ask for a code in a list *as of* a date, and you get back the entry only if both the manifest window and the entry's own validity window cover that date.

The crate ships a small but real set of seed manifests, embedded at compile time, so the load/verify/lookup contract is executable today. The seed covers the seven list families below. It is a starting set, not full upstream coverage — the data files are intentionally short.

Seed list families (each with a stable identifier constant):

- ISO 3166-1 alpha-2 country codes — `ISO_3166_1_ALPHA2`
- ISO 3166-2 subdivision codes — `ISO_3166_2`
- ISO 4217 currency codes — `ISO_4217`
- UN/ECE Recommendation 20 unit codes — `UNECE_REC20_UNITS`
- EN 16931 VAT category codes — `EN16931_VAT_CATEGORY`
- Peppol BIS Billing invoice type codes (UNCL1001) — `PEPPOL_INVOICE_TYPE`
- Peppol participant identifier schemes — `PEPPOL_PARTICIPANT_SCHEME`

## Signing

Manifests today use one signature algorithm: `sha256:identity`. The signature is a SHA-256 digest over a line-and-pipe-delimited rendering of the manifest payload (`Manifest::expected_signature`). `Manifest::verify` recomputes that digest and compares it in constant time. This is a content-integrity check, not a public-key signature — the algorithm name leaves room for a real detached signature later, and an unrecognized algorithm is rejected rather than ignored.

Because the signing payload is delimiter-based, fields that could contain a delimiter (`|`, `;`, `=`, newline, carriage return) are rejected during validation. That keeps two different manifests from ever producing the same digest payload.

## Public API

Loading and lookup (`lib.rs`):

- `Registry::seeded()` — build a registry from the embedded seed manifests; fails if any seed is malformed or its signature does not verify.
- `Registry::from_manifests(Vec<Manifest>)` — build a registry from caller-supplied manifests; verifies each one.
- `Registry::lookup(list, code, on_date)` — return the `Entry` for a code in a list as of a date, or `None` if the list is unknown, the date is malformed, no manifest covers it, the code is unknown, or the entry is outside its own window.
- `Registry::manifest(list, on_date)` — return the manifest that covers a list on a date.
- `Registry::list_names()` / `Registry::manifests()` — iterate the registry.
- `Manifest::from_json(raw)` — parse and verify a single manifest.
- `Manifest::verify()`, `Manifest::expected_signature()`, `Manifest::is_effective_on(on_date)`.
- `Entry::is_effective_on(on_date)`.
- `CodelistError` — load/validation errors (bad JSON, empty fields, ambiguous separators, bad dates, inverted date windows, empty manifests, duplicate codes, unsupported or mismatched signatures).

Updater inputs (`sources` module):

- `SourceSpec` and `BUILTIN_SOURCES` — the typed registry of upstream authorities.
- `source_for(list)` — look up a registered source by list name.
- `build_manifest(spec, raw_upstream, retrieved_at)` — normalize a raw upstream payload into a signed manifest that round-trips through `Manifest::verify`. `retrieved_at` is carried in verbatim and pins the version, so a re-run on the same input is byte-identical.
- `normalize_iso_4217_csv(raw)` — the per-list CSV normalizer for ISO 4217.
- `SourceFormat`, `Normalizer`, `SourceError`.

The `sources` module is network-free by design: it normalizes a raw payload read from a local path. The fetching happens elsewhere (the CLI `codelist-update` command and the nightly workflow that feeds it). Of the seven seed families, only ISO 4217 is wired end-to-end through `BUILTIN_SOURCES` today; the rest ship as seed manifests with their updaters tracked as follow-up work.

## Where it sits in the pipeline

This crate is a foundation dependency, not a stage. It sits to the side of the main flow:

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

The validation step consults this registry to confirm that codes carried in an invoice were valid on the invoice's issue date. The `cli` crate drives the updater path, and `bench-harness` exercises lookup performance.

## Usage

```rust
use invoicekit_codelists::{Registry, ISO_4217};

let registry = Registry::seeded()?;

// Look up a currency code as of a date.
let eur = registry
    .lookup(ISO_4217, "EUR", "2024-06-01")
    .expect("EUR is in the seed set");
assert_eq!(eur.label, "Euro");
assert_eq!(eur.attrs.get("minor_units").map(String::as_str), Some("2"));

// A date before the manifest's effective window resolves to nothing.
assert!(registry.lookup(ISO_4217, "EUR", "2023-12-31").is_none());
# Ok::<(), invoicekit_codelists::CodelistError>(())
```

Building a fresh manifest from an upstream payload, via the `sources` module:

```rust
use invoicekit_codelists::{ISO_4217, sources};

let csv = "code,label,numeric,minor_units\nEUR,Euro,978,2\nUSD,US Dollar,840,2\n";
let spec = sources::source_for(ISO_4217)?;
let manifest = sources::build_manifest(spec, csv, "2026-05-27")?;

assert_eq!(manifest.version, "iso-4217-2026-05-27");
manifest.verify()?; // freshly built manifests verify
# Ok::<(), Box<dyn std::error::Error>>(())
```

## License

Apache-2.0. Part of the InvoiceKit workspace.
