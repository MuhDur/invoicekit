<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-format-gobl

Bidirectional adapter between InvoiceKit's `CommercialDocument` and [invopop/gobl](https://github.com/invopop/gobl) JSON.

## What it does

GOBL is the closest open-source neighbor to InvoiceKit: Apache 2.0, written in Go, with a well-defined JSON shape for bills. Rather than reinvent its schema, InvoiceKit interoperates with it, so data can move between the two ecosystems. This crate is the bridge. It projects an InvoiceKit intermediate representation document out to GOBL JSON, and parses GOBL JSON back into the intermediate representation.

Both directions return a lossiness report alongside the converted document. When a field has no home in the target shape, the conversion does not silently drop it — it records the path and the reason. That is the trust-toolkit stance: report lossiness, don't paper over it.

This is a first-cut adapter, not a complete GOBL implementation. It covers the core invoice surface and round-trips it reliably; fields GOBL carries that InvoiceKit does not model yet (for example `addons`, `$regime`, per-line discounts) are treated as expected losses rather than errors.

## Scope

The adapter maps:

- document id, type, code, issue/due/tax-point dates, currency
- supplier and customer parties — tax identifier, postal address, contact
- one or more lines — id, description, quantity, unit price, line total, tax category
- per-category tax summary and monetary totals
- payment terms and payment instructions
- preceding document references, notes, attachments
- jurisdiction extensions, stamped into the GOBL `ext` map keyed by URN and round-tripped back into `JurisdictionExtension`

GOBL has no first-class payee distinct from the supplier; a payee is emitted as an auxiliary field and the loss is noted in the ledger. A GOBL document with no `meta` block gets synthesized placeholder tenant and trace identifiers, again with a ledger note.

## Public API

| Item | Purpose |
| --- | --- |
| `to_gobl(&CommercialDocument) -> Result<GoblEnvelope, GoblError>` | Project the intermediate representation out to GOBL JSON. Validates the input first. |
| `from_gobl(&Value) -> Result<GoblEnvelope, GoblError>` | Parse GOBL JSON into a serialized intermediate-representation document. |
| `GoblEnvelope` | The result: `document` (the converted JSON) plus `ledger` (a `LossinessLedger`). |
| `GoblError` | Conversion errors: `NotAnObject`, `MissingField`, `BadValue`, `BadDecimal`, `Ir`. |

On the forward path `GoblEnvelope::document` holds the GOBL payload. On the reverse path it holds the serialized intermediate-representation JSON, so it can be fed straight into `CommercialDocument::try_from_value`.

There are also a few constants — `GOBL_BILL_SCHEMA_PREFIX` (the `https://gobl.org/draft-0/bill/` schema URL prefix) and `GOBL_ADAPTER_BEAD_ID` for diagnostic correlation.

## Where it sits

This is a format crate in the InvoiceKit pipeline:

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

It lives at the `format` stage, beside `format-ubl` and `format-cii`. It reads and writes `invoicekit-ir::CommercialDocument`, the layered intermediate representation. It does not touch validation, rendering, or transmission — it is purely a translation layer between InvoiceKit and GOBL.

## Usage

Round-trip an intermediate-representation document through GOBL and back:

```rust
use invoicekit_format_gobl::{from_gobl, to_gobl};
use invoicekit_ir::CommercialDocument;

// Project an IR document out to GOBL JSON.
let forward = to_gobl(&doc)?;
let gobl_json = forward.document;
for entry in &forward.ledger.lost {
    eprintln!("lossy: {} — {}", entry.path, entry.reason);
}

// Parse GOBL JSON back into a validated IR document.
let backward = from_gobl(&gobl_json)?;
let recovered = CommercialDocument::try_from_value(backward.document)?;
assert_eq!(recovered.id, doc.id);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Conformance

`tests/upstream_corpus.rs` round-trips a fixed set of 20 GOBL upstream fixtures (under `conformance-corpus/gobl-upstream/`) and asserts the outcome against a byte-stable `coverage-matrix.json` snapshot. Each fixture lands as one of: clean round trip, lossy round trip, or a documented skip with a reason. A codec change that improves or degrades coverage shows up as a snapshot diff in review. The floor is that at least 10 of the 20 fixtures survive the round trip without an inbound parse failure.

To regenerate the snapshot after an intentional codec change:

```sh
cargo test -p invoicekit-format-gobl --test upstream_corpus \
    ignored_bless_coverage_matrix -- --ignored --nocapture
```

## License

Apache-2.0, like the rest of InvoiceKit.
