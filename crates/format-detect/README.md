<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-format-detect

Byte-prefix format sniffer that maps an opaque payload to an InvoiceKit `FormatId`.

## What it does

A customer hands InvoiceKit a blob of bytes and asks "what is this?". This crate
answers that question by reading a bounded prefix — never the whole file — and
returning the document family it belongs to. It does not parse the document. It
looks for file signatures, XML namespace URIs, and JSON schema URLs, then returns
a stable enum value.

The detector is conservative on purpose. It would rather return `Unknown` than
make a wrong claim. Every named variant exists because a real fixture proves the
rule, and the conformance corpus enforces a false-positive ceiling. Anything that
does not match a registered signature comes back as `FormatId::Unknown` with a
short note describing what the sniffer actually saw.

How matches are decided:

- File magic first. `%PDF-` and the ZIP local-file header (`PK\x03\x04`) are
  checked before anything else.
- For PDF, an extra scan looks for the Factur-X / ZUGFeRD attachment markers
  (`/AFRelationship`, `factur-x.xml`, `ZUGFeRD-invoice.xml`) so a hybrid PDF/A-3
  is distinguished from a plain PDF.
- For XML, the primary namespace URI is the authority. Namespace URIs are
  immutable per-standard tokens, so a literal substring match is a sound signal.
- For JSON, the `$schema` URL — or the GOBL / InvoiceKit-IR field shape — decides.
- A UTF-8 byte-order mark is tolerated, since Office tooling sometimes prepends one.

## Where it sits in the pipeline

`engine -> ir -> canonical -> format/profile -> validate -> render/`**`intake`**` -> transmit -> evidence`

This crate is an intake-side gate. It runs at the very start of intake, before
any format-specific parser is chosen. Given unidentified bytes it picks the lane:
a UBL document goes to the UBL reader, a GOBL envelope to the GOBL reader, a PDF
to the PDF/OCR path, and so on. `inbound-peppol` is a real consumer — it calls
`detect_format` on the inbound payload and branches on the returned `FormatId`.

## Public API

| Item | Purpose |
| --- | --- |
| `FormatId` | Enum of detected document families (see below). `Copy`, `serde`-serializable as kebab-case. |
| `Detection` | A `FormatId` plus a `notes` string. Notes are empty for confident matches and populated for `Unknown` / ambiguous cases. |
| `detect_format(input: &[u8]) -> FormatId` | The common entry point. Returns just the identifier. |
| `detect_format_with_notes(input: &[u8]) -> Detection` | Same detection, but keeps the diagnostic breadcrumb for the `Unknown` case. |
| `crate_name() -> &'static str` | Returns `"invoicekit-format-detect"`. Housekeeping. |

Both detection functions are total: they never panic and never read past a
bounded prefix (16 kB for XML/JSON, extended to 64 kB when a PDF signature is
present so the attachment marker can be found).

### Formats currently recognised

XML namespaces: UBL 2.1 invoice and credit note (`Ubl21`), UN/CEFACT Cross
Industry Invoice D16B (`CiiD16B`), FatturaPA 1.2.x (`FatturaPa`), SAT CFDI 4.0
(`Cfdi40`), Polish KSeF FA(3) (`KsefFa3`), Saudi ZATCA Phase 2 (`ZatcaPhase2`),
Greek myDATA v1.0 (`MyDataV10`), Spanish Verifactu v1.0 (`VerifactuV10`),
Brazilian NF-e v4.00 (`NfeV400`), Indian GST IRN (`GstIrn`).

JSON shapes: GOBL envelope (`GoblEnvelope`), GOBL bill (`GoblBill`), InvoiceKit
internal IR v1 (`InvoicekitIrV1`).

Binary containers: Factur-X / ZUGFeRD PDF (`PdfWithFacturX`), plain PDF (`Pdf`),
ZIP container (`ZipContainer`). The sniffer does not recurse into a ZIP; the
caller decides what to do with it.

Everything else is `Unknown`.

## Usage

```rust
use invoicekit_format_detect::{detect_format, detect_format_with_notes, FormatId};

// Confident match — a plain PDF.
assert_eq!(detect_format(b"%PDF-1.7\n"), FormatId::Pdf);

// XML by namespace.
let ubl = br#"<?xml version="1.0"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#;
assert_eq!(detect_format(ubl), FormatId::Ubl21);

// JSON by $schema.
let gobl = br#"{"$schema":"https://gobl.org/draft-0/envelope","head":{},"doc":{}}"#;
assert_eq!(detect_format(gobl), FormatId::GoblEnvelope);

// Unknown input keeps a diagnostic breadcrumb.
let d = detect_format_with_notes(b"not-a-known-format");
assert_eq!(d.format, FormatId::Unknown);
assert!(!d.notes.is_empty());
```

## Tests

Unit tests in `src/lib.rs` cover one or more cases per `FormatId`, BOM
tolerance, pretty-printed JSON, and crash safety on random bytes. The
`tests/conformance_corpus.rs` harness loads the workspace `conformance-corpus/`
fixtures and asserts the UBL, CII, and GOBL samples classify correctly while the
overall false-positive rate stays under one percent. Corpus tests skip cleanly
when the fixtures are not present.

## Status

Working and tested for the formats listed above. The variant list grows only
when a fixture exists to prove the rule, so treat anything not in `FormatId` as
out of scope today. This crate is a workspace member (`publish = false`); it is
not published to crates.io on its own.

Licensed under Apache-2.0.
