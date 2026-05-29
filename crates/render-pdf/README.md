# invoicekit-render-pdf

Deterministic, byte-stable PDF/A-3b rendering for InvoiceKit, built on Typst.

## What it does

This crate turns a validated InvoiceKit document into PDF bytes. Two renders of
the same input produce the same bytes — down to the SHA-256 — so the output can
go straight into a signed evidence bundle and be reproduced later. Determinism is
not best-effort: the document date is fixed, the PDF timestamp is fixed, only
embedded fonts are used (system fonts are never consulted), and the stable
document identifier is supplied by this crate rather than generated at runtime.

The PDF profile is PDF/A-3b. The archival profile matters because an invoice PDF
is meant to be the human-readable face of a long-lived legal record, and PDF/A-3b
is the profile that lets the structured XML ride along inside the PDF later.

Typst is used as the layout engine, but its source is treated as a trusted
template, not a sandbox. There is no public API for rendering user-authored Typst.
The internal renderer, the `RenderRequest` type, and the in-memory Typst `World`
are all private. The trust boundary for caller-supplied templates is a separate,
not-yet-landed task (T-051); until then, this crate owns every byte of Typst
source it executes.

## Public API

- `render_commercial_document_invoice(&CommercialDocument) -> Result<Vec<u8>, RenderPdfError>`
  — the document-aware path. Pass an already-validated IR document; the crate
  generates the Typst source and returns PDF/A-3b bytes. This is what REST and
  demo surfaces call.
- `render_hello_world_invoice() -> Result<Vec<u8>, RenderPdfError>` — renders the
  built-in smoke-test invoice. Used to prove the render path works end to end and
  to pin cross-platform byte stability.
- `HELLO_WORLD_INVOICE_TEMPLATE: &str` — the Typst source for that smoke invoice.
  Exposed mainly so tests can assert against it.
- `RenderPdfError` — the error type. Variants: `Compile` (Typst source failed to
  compile), `Profile` (the PDF profile is not supported by the exporter), `Export`
  (Typst failed turning the compiled document into PDF bytes), and
  `InvalidFixedTimestamp` (the fixed timestamp could not be constructed — a bug,
  not a user error). Every message carries a `Hint:`.
- `crate_name() -> &'static str` — returns `"invoicekit-render-pdf"`. Used by
  release tooling and log-correlation reports.

There is also a `#[doc(hidden)]` `render_for_fuzz(&str)` entry point. It exists
only so the libFuzzer target can drive the Typst compiler with adversarial source
without making `RenderRequest` public. Do not call it from application code.

## Determinism, in concrete terms

The document-independent rendering assets — the Typst standard library and the
font catalogue — do not depend on the invoice being rendered, but building them is
the dominant cost of a render. They are built exactly once into a process-wide
`LazyLock` (`RENDER_ASSETS`) and shared by every render; only the per-document
source changes between calls. This is a pure performance hoist: the library, the
font book contents, and the font index order are byte-for-byte identical to
building them per call.

The font catalogue is typst-kit's embedded faces (Libertinus Serif and friends)
with InvoiceKit's pinned fonts layered on top. The pinned set lives under
`fonts/<family>/` with a sibling license file per family. Today that is one face,
DejaVu Sans Mono. Adding another is a three-step diff: drop the `.ttf` under
`fonts/<family>/`, add its license, and add one entry to the `pinned_fonts!`
macro.

The byte-stability claim is enforced, not asserted. `tests/golden_render.rs` pins
the exact SHA-256 of both render paths; if a render-path change moves a single
byte, that test fails. A cross-platform CI job renders the hello-world invoice on
Linux and macOS and compares digests, which is why system-font discovery stays
off.

## Where it sits in the pipeline

```
engine → ir → canonical → format/profile → validate → render/intake → transmit → evidence
                                                          ▲
                                                  invoicekit-render-pdf
```

This crate is in the render stage. It consumes a validated `CommercialDocument`
from `invoicekit-ir` and produces PDF bytes. It is sibling to `render-html` (the
human-readable HTML face) and feeds `render-pdf-postproc` (which handles the
PDF/A-3 attachment of the structured XML). It does not validate, and it does not
embed XML itself — it produces the deterministic PDF/A-3b base that downstream
post-processing builds on.

## Usage

```rust
use invoicekit_ir::CommercialDocument;
use invoicekit_render_pdf::render_commercial_document_invoice;

// `document` is an already-validated InvoiceKit IR document.
let document: CommercialDocument = /* ... */;
let pdf: Vec<u8> = render_commercial_document_invoice(&document)?;
assert!(pdf.starts_with(b"%PDF-"));
```

The simplest possible call renders the built-in smoke invoice:

```rust
let pdf = invoicekit_render_pdf::render_hello_world_invoice()?;
assert!(pdf.starts_with(b"%PDF-"));
```

## Status

Working and tested for the two render paths above. The layout itself is plain:
title, supplier/customer block, a line-item table, and a total. It is a correct,
deterministic foundation, not a richly styled invoice template — styling depth and
caller-supplied templates are deliberately out of scope here. System fonts are
off; the pinned font set is one face.

## License

Apache-2.0. The bundled fonts ship with their own license files under `fonts/`.
