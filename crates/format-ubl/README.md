# invoicekit-format-ubl

UBL 2.1 `Invoice` and `CreditNote` read/write for the InvoiceKit commercial document model.

## What it does

This crate is one of InvoiceKit's format adapters. It reads an OASIS UBL 2.1 `Invoice` or `CreditNote` document into the shared `invoicekit_ir::CommercialDocument`, and serializes that model back out as deterministic UBL XML.

It is deliberately scoped to the IR fields that exist today. The parser maps the core commercial surface (identifiers, dates, currency, parties, lines, tax summary, monetary totals, notes, payment terms and instructions). Top-level UBL elements that have no core IR home are not dropped: they are captured verbatim into a UBL-specific `JurisdictionExtension` so a later profile pass can round-trip them. Where even that does not apply, the crate is honest about it through a coverage matrix and a lossiness ledger rather than silently discarding data.

Two things make the round-trip auditable:

- **A coverage matrix** (`mapping` module). Every top-level element in the UBL 2.1 maindoc sequence has a row stating how InvoiceKit represents it today: core IR, the metadata extension, a preserved document-field extension, a profile/customization payload, or an unimplemented gap.
- **A lossiness ledger.** `from_xml` parses, re-serializes, re-parses, and compares the two IR documents using `invoicekit_ir::LossinessLedger::from_roundtrip_comparison`. The returned ledger lists what survived and what did not, so callers can refuse a lossy import.

Serializer output is run through `invoicekit_canonical::canonicalize_xml`, so serializing the same document twice on the same platform yields identical bytes.

This is a working adapter for the current core IR, not full UBL 2.1 coverage. Many aggregate references (order, billing, despatch, contract references), allowances/charges, delivery, and multi-currency roles are preserved as opaque fragments rather than modeled. Read the coverage matrix before assuming a field is understood.

## Where it sits in the pipeline

```
engine → ir → canonical → [format/profile] → validate → render/intake → transmit → evidence
                                  ▲
                          invoicekit-format-ubl
```

It is a peer of `invoicekit-format-cii` and `invoicekit-format-gobl`. It depends on `invoicekit-ir` (the document model and lossiness ledger) and `invoicekit-canonical` (deterministic XML output). Profile crates such as Peppol BIS, PINT, Factur-X, and XRechnung build on top of UBL serialization; this crate emits a neutral InvoiceKit core customization and profile URN and leaves profile-specific values to those layers.

## Public API

Top-level functions:

- `to_xml(document: &CommercialDocument) -> Result<String, UblError>` — serialize to canonical UBL XML. Only `DocumentType::Invoice` and `DocumentType::CreditNote` are supported; other types return `UblError::UnsupportedDocumentType`.
- `from_xml(input: &str) -> Result<(CommercialDocument, LossinessLedger), UblError>` — parse and return the IR plus the round-trip lossiness ledger.
- `crate_name() -> &'static str`.

Error type: `UblError` (a `thiserror` enum) with variants for malformed XML, an unsupported root, missing required elements, invalid decimals, IR validation failure, canonicalization failure, and an invalid preserved fragment.

Coverage matrix (`mapping` module, re-exported at the crate root):

- `UblDocumentKind`, `UblCoverageClass`, `UblElementCoverage`.
- `top_level_coverage(kind)`, `coverage_for(kind, element)`.
- `INVOICE_ELEMENT_COVERAGE`, `CREDIT_NOTE_ELEMENT_COVERAGE`.
- URN and schema-URI constants: `INVOICEKIT_METADATA_EXTENSION_URN`, `UBL_DOCUMENT_FIELDS_EXTENSION_URN`, `UBL_2_1_OS_SPEC_URI`, `UBL_2_1_INVOICE_SCHEMA_URI`, `UBL_2_1_CREDIT_NOTE_SCHEMA_URI`.

Offline schema harness (`schema` module, re-exported):

- `validate_oasis_ubl_2_1_schema(xml) -> Result<UblSchemaValidationReport, UblError>` — checks XML against the vendored OASIS UBL 2.1 XSDs. It resolves all imports from `schemas/ubl-2.1` and never fetches over the network. It also re-checks the maindoc top-level sequence order, because the pure-Rust XSD validator otherwise accepts globally declared UBL elements in the wrong document position.
- `UblSchemaValidationReport` (with `is_valid()`), `UblSchemaValidationFinding`, `UblSchemaValidatedFixture`, `OASIS_UBL_2_1_SCHEMA_VALIDATED_FIXTURES`.

The vendored schemas and their provenance (upstream URLs and SHA-256 sums) are documented in `schemas/ubl-2.1/PROVENANCE.md`.

## Binary

`invoicekit-ubl-normalize` reads UBL XML, runs it through `from_xml` then `to_xml`, and writes canonical UBL XML to stdout. It refuses to emit output if the lossiness ledger reports any lost fields or warnings.

```
invoicekit-ubl-normalize <fixture.xml>
invoicekit-ubl-normalize --stdin <fixture-label>
```

## Usage

Parse a UBL invoice and check that nothing was lost:

```rust
use invoicekit_format_ubl::{from_xml, to_xml};

let (document, ledger) = from_xml(ubl_xml)?;
assert!(ledger.lost.is_empty(), "lossy import: {:?}", ledger.lost);

// Re-serialize as canonical UBL XML.
let xml = to_xml(&document)?;
# Ok::<(), invoicekit_format_ubl::UblError>(())
```

## License

Apache-2.0. The vendored OASIS UBL 2.1 XSD files retain their upstream OASIS copyright and permission notice; see `schemas/ubl-2.1/PROVENANCE.md`.
