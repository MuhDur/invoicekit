# PDF/A-3 post-processing — decision rule

This document records how InvoiceKit decides between (a) patching
Typst's PDF output downstream in `crates/render-pdf-postproc` via
`lopdf`, and (b) upstreaming a PR to the Typst project.

## The rule

| Missing feature | Where it should live | Rationale |
| --- | --- | --- |
| XMP metadata that any PDF/A producer needs (Title, Author, CreateDate, ModDate). | **Upstream Typst.** | Every Typst user benefits; the change is uncontroversial within the PDF spec. |
| PDF spec correctness fixes (missing `MarkInfo`, wrong `OutputIntent`, malformed Filespec dictionary). | **Upstream Typst.** | A PDF that fails veraPDF on a Typst-only output is a Typst bug. |
| ZUGFeRD / Factur-X attachment (the `factur-x.xml` Filespec + `AF` array + `fx:` XMP namespace). | **`render-pdf-postproc` (this crate).** | Factur-X is an e-invoicing-specific overlay. Pushing it upstream would either bloat Typst with a domain-specific feature flag or invite an unrelated dependency on InvoiceKit. |
| XRechnung-in-PDF naming (`xrechnung.xml` attachment + same XMP block with `XRECHNUNG` conformance level). | **`render-pdf-postproc` (this crate).** | Same rationale; the only difference is the canonical filename. |
| Hybrid-PDF profiles (PDF/A-3 with a non-Factur-X attachment, e.g. CEN/TC 251 hybrid HL7-FHIR-in-PDF). | **`render-pdf-postproc` (this crate).** | Same shape; any hybrid scheme that needs an `AF` + `Filespec` overlay gets a sibling helper inside this crate. |
| PDF/UA accessibility tags. | **Upstream Typst (preferred) or here as a fallback.** | Typst is actively working on accessibility tags; we prefer to wait and rebase on the upstream feature when it lands. Adding a custom tag tree here would be expensive to maintain. |

The principle: anything that lives in the PDF spec without
reference to e-invoicing belongs upstream. Anything that lives in
ZUGFeRD / Factur-X / XRechnung belongs here.

## Upstream-PR queue

The features below have been identified as upstream-PR candidates
during T-053. Each entry lists what InvoiceKit needs and the
upstream conversation pointer.

- **XMP packet emission options.** Typst's `PdfOptions` does not
  yet allow overriding the XMP packet; today we have to rewrite
  the catalog's `Metadata` stream from outside. Track the upstream
  issue and link the PR here when it lands.
- **AssociatedFile knowledge.** Typst could accept an
  `attachments: Vec<...>` field on `PdfOptions` that emits the
  `EmbeddedFiles` + `AF` plumbing on the way out. This would let
  InvoiceKit's post-processor drop most of `embed_factur_x`.

When an upstream PR is merged + Typst is rebased, the
corresponding code in `render-pdf-postproc` becomes deletable —
follow-up bead should remove it and prove the round-trip still
passes via the Typst path alone.

## Why `lopdf` is the in-tree patch tool

`lopdf` parses + re-serializes PDFs without re-encoding streams.
That preserves Typst's careful PDF/A-3b byte layout, including
font subsetting and ICC profile placement. A heavier PDF library
(`pdf-rs`, `printpdf`) would re-emit the page tree from scratch
and risk breaking the PDF/A-3 conformance we inherit from Typst.

The trade-off: `lopdf`'s API is low-level. Each new overlay
feature (Factur-X attachment, AssociatedFile pointer, XMP packet)
takes ~30 lines of object-graph manipulation. The verbosity is the
right side of the trade because the post-processor's contract is
narrow ("add Factur-X plumbing without touching anything else") —
expressive APIs would tempt us to mutate things we shouldn't.

## What this post-processor does NOT cover

- It does not validate that the PDF is already PDF/A-3b. Typst
  handles that; the post-processor inherits whatever Typst emits.
- It does not run veraPDF. The conformance gate is owned by the
  release pipeline (a follow-up bead installs veraPDF in a
  release-only workflow and asserts `--profile=3b` / `--profile=3u`
  pass for every emitted invoice).
- It does not extract the XML — that is `intake-pdf::extract_factur_x_xml`
  (T-060), which the unit tests here use as their round-trip
  inverse so a producer/consumer drift is caught at compile time.
