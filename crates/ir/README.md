# invoicekit-ir

The layered invoice data model. The Rust source of truth for what an InvoiceKit document *is*.

## What it does

`invoicekit-ir` defines the in-memory shape of a commercial document and the rules
for whether that shape is well-formed. It keeps two layers separate on purpose:

- **`CommercialDocument`** — jurisdiction-agnostic invoice and credit-note
  semantics. Parties, lines, tax summaries, monetary totals, payment
  instructions, attachments, references, notes, and metadata. This is the root.
- **`ProfileView`** — a projection of a commercial document onto one standard or
  country profile (Peppol BIS, XRechnung, Factur-X, a national report). The view
  records *what the projection did* to the data through a `LossinessLedger`, so a
  lossy mapping is auditable instead of silent.

The model carries no float anywhere. Money and quantities are `DecimalValue`,
which serializes to and from a fixed decimal *string* at the JSON boundary, so
`119.00` stays `119.00` and never becomes `119` or `119.0000001`. Country,
currency, and date values are newtypes that validate on construction.

This crate is the model and its validation envelope. It does not parse XML, talk
to validators, render PDFs, or transmit anything. Those live in the format,
profile, validate, render, and transmit crates that build on this one.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

`ir` is the second stage. Everything downstream operates on a `CommercialDocument`
or a `ProfileView` defined here. The `canonical` crate serializes the IR
deterministically for signing; the `format-*` and `profile-*` crates project it
onto wire formats; `validate` checks it against rule packs.

## Key public API

Document model and construction:

- `CommercialDocument` — the root document.
- `CommercialDocumentParts` — typed input struct; `CommercialDocument::new(parts)`
  builds and validates in one step.
- `CommercialDocument::try_from_value(value)` — deserialize from a
  `serde_json::Value` and validate.
- `CommercialDocument::to_value()` — serialize back to JSON. The round trip
  `to_value -> try_from_value -> to_value` is byte-stable.
- `CommercialDocument::validate()` — run the validation envelope on an existing
  document.

Profile projection:

- `ProfileView` / `ProfileView::new(...)` — a projection onto one profile.
- `ProfileIdentifier` — the target profile URN and version.
- `LossinessLedger`, `LossinessEntry` — the record of what survived a projection
  and what did not. `LossinessLedger::from_roundtrip_comparison(source, reparsed, adapter)`
  builds a field-level ledger by diffing a document against its reparsed self.

Validating value objects (each validates on construction):

- `DocumentId`, `DocumentNumber` — non-empty identifiers.
- `DateOnly` — a real `YYYY-MM-DD` calendar date (rejects `2026-02-29`, `2026-13-01`).
- `Iso4217Code` — three uppercase ASCII letters.
- `CountryCode` — two uppercase ASCII letters (ISO 3166-1 alpha-2).
- `DecimalValue` (aliased as `MoneyAmount` and `Quantity`) — decimal-as-string.

Supporting types: `Party`, `PartyTaxId`, `PostalAddress`, `Contact`,
`DocumentLine`, `TaxCategorySummary`, `MonetaryTotal`, `PaymentTerms`,
`PaymentInstruction` / `PaymentInstructionKind`, `Attachment`,
`DocumentReference`, `LocalizedString`, `DocumentMeta`, `DocumentType`,
`SchemaVersion`.

Extensions and schema:

- `JurisdictionExtension` — a polymorphic `{ urn, payload }` envelope for
  profile- or country-specific data. The URN scheme prefix is canonicalized to
  lowercase `urn:` per RFC 8141 on both construction and deserialization; the
  namespace identifier and namespace-specific string are preserved byte-for-byte
  so canonical signing payloads stay stable.
- `commercial_document_schema()` — returns the JSON Schema (Draft 2020-12) for
  `CommercialDocument`. This is what the TypeScript, Python, Java, and .NET
  binding generators consume. CI re-derives it and asserts byte-equality against
  the committed `schemas/invoicekit-ir-v1.json`.
- `IrError` — the error type returned by every constructor and validator.

## Usage

```rust
use invoicekit_ir::CommercialDocument;
use serde_json::json;

let document = CommercialDocument::try_from_value(json!({
    "schema_version": "1.0",
    "id": "doc_2026_0001",
    "document_type": "invoice",
    "issue_date": "2026-05-26",
    "due_date": "2026-06-25",
    "document_number": "INV-2026-0001",
    "currency": "EUR",
    "supplier": {
        "name": "InvoiceKit GmbH",
        "tax_ids": [{ "scheme": "vat", "value": "DE123456789" }],
        "address": { "lines": ["Main Street 1"], "city": "Berlin", "postal_code": "10115", "country": "DE" }
    },
    "customer": {
        "name": "ACME SAS",
        "address": { "lines": ["Rue 1"], "city": "Paris", "postal_code": "75001", "country": "FR" }
    },
    "lines": [{
        "id": "1",
        "description": "Validation subscription",
        "quantity": "1",
        "unit_price": "100.00",
        "line_extension_amount": "100.00",
        "tax_category": "S"
    }],
    "monetary_total": {
        "line_extension_amount": "100.00",
        "tax_exclusive_amount": "100.00",
        "tax_inclusive_amount": "119.00",
        "payable_amount": "119.00"
    },
    "meta": { "tenant_id": "tenant_123", "trace_id": "trace_abc" }
}))?;

assert_eq!(document.id.as_str(), "doc_2026_0001");
assert_eq!(document.monetary_total.payable_amount.inner().scale(), 2);
# Ok::<(), invoicekit_ir::IrError>(())
```

## Stability

`SchemaVersion` is at `V1_0` and every serialized document carries it. The model
covers the global commercial-document core; profile-specific structure lives in
`JurisdictionExtension` payloads rather than in typed fields here, so a new
profile does not require changing this crate.

## License

Apache-2.0. Part of the InvoiceKit workspace.
