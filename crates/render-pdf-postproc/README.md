# invoicekit-render-pdf-postproc

Post-processes a Typst-rendered PDF into a Factur-X / ZUGFeRD PDF/A-3: it injects the invoice XML as a file attachment, wires the associated-file relationship, and merges a profile-aware XMP metadata packet. It does not render or validate PDFs.

## What it does

Typst emits PDF/A-3b-compliant bytes, but does not write the XMP metadata or the associated-file relationship that Factur-X / ZUGFeRD readers look for. This crate is the post-processing pass. Given Typst's PDF output, the Factur-X XML payload, and a profile, the single public function `embed_factur_x(pdf, xml, profile)` returns patched PDF bytes where:

1. The XML lives in the catalog's `Names.EmbeddedFiles` name tree under the canonical filename for the profile, as an `EmbeddedFile` stream behind a `Filespec` with `AFRelationship = Alternative`.
2. The catalog's `AF` array references that same `Filespec`, so PDF/A-3 readers find the attachment regardless of which path they walk first.
3. The catalog `Metadata` stream is an XMP packet that declares the `fx:` extension schema and the chosen ZUGFeRD profile, built by preserving Typst's existing PDF/A identification metadata and appending the Factur-X block inside the same RDF packet.

Existing catalog entries are preserved, not overwritten: a prior embedded file, a prior `AF` entry, and a prior extension-schema bag all survive the merge.

## Capabilities

- **Attachment injection** via `embed_factur_x`. Adds the `EmbeddedFile` stream (`Subtype text/xml`), the `Filespec` (`F`/`UF` set to the profile filename, `AFRelationship = Alternative`, `EF.F`/`EF.UF` pointing at the stream), and patches the catalog's `Names`, `AF`, and `Metadata`.
- **Profile model** via `ZugferdProfile` (`Minimum`, `BasicWl`, `Basic`, `En16931`, `Extended`, `Xrechnung`). The variant selects the attachment filename (`attachment_filename` returns `xrechnung.xml` for `Xrechnung`, `factur-x.xml` for all others) and the `fx:ConformanceLevel` value (`xmp_conformance_level`; `name` is an alias of it). `ZugferdProfile::all()` returns the six-profile acceptance matrix.
- **Merge-preserving catalog patch.** Pre-existing `EmbeddedFiles` name-tree entries are retained, with the new `(filename, filespec)` pair added and the same filename de-duplicated. `Names`, the `EmbeddedFiles` sub-dictionary, and its inner `Names` array are resolved whether stored inline or as indirect references (ISO 32000 permits any of these to be a reference). A pre-existing `AF` entry is kept and the new `Filespec` reference appended only if absent.
- **Name-tree ordering.** The leaf `Names` array is re-sorted into ascending lexical byte-string key order, as required by ISO 32000 7.9.6. A non-string key or a trailing unpaired element is left at the end so a malformed input never loses data.
- **XMP merge.** When Typst's existing XMP is present, the Factur-X PDF/A extension schema is inserted into the existing `pdfaExtension:schemas` bag (or a fresh bag/packet is synthesized if none exists), the `fx:` value description is appended before `</rdf:RDF>`, and an existing `pdfaid:conformance` of `B` is promoted to `U`. A compressed (`FlateDecode`) `Metadata` stream is decoded before merging.
- **Byte-deterministic output.** The same `(pdf, xml, profile)` triple produces byte-identical output across runs, so a release pipeline can hash the result and store the digest as conformance evidence.
- **Typed errors.** `PostprocError::Parse` for unparseable input or a missing/non-dictionary `/Root` catalog; `PostprocError::Serialize` when the patched document cannot be written back out.
- **Acceptance-gate constants.** `ACCEPTANCE_FIXTURES_PER_PROFILE = 5` and `REQUIRED_VERAPDF_PROFILE_ARGS = ["3b", "3u"]` describe the T-053 acceptance matrix (30 fixtures across two veraPDF profiles).
- Re-exports `FACTUR_X_ATTACHMENT_NAMES` from `invoicekit-intake-pdf` so callers can match canonical attachment names without a second dependency.

## Mode / Residuals

This is real PDF object manipulation over `lopdf`, not a stub. `embed_factur_x` returns the actual patched bytes, and the in-crate test suite round-trips all six profiles (five fixtures each) back out through `invoicekit-intake-pdf`'s extractor and asserts byte-for-byte XML equality. The boundaries are deliberate:

- **It does not render and does not produce PDF/A from scratch.** It assumes the input is already valid PDF/A-3b (Typst's output). It only overlays the Factur-X / ZUGFeRD attachment, the `AF` pointer, and the `fx:` XMP block.
- **It does not validate the result.** The XMP packet is shaped so the attachment is visible to veraPDF's `--profile=3b` / `--profile=3u` checks, but this crate does not run veraPDF or any other PDF/A conformance oracle. A real veraPDF run against the two profiles remains a mandatory external gate before the conformance claim can be closed; the constants here only name that gate.
- **It does not validate the XML.** The XML payload is embedded as-is; this crate does not parse, schema-check, or confirm that the XML matches the declared `profile`.
- **It does not encrypt, sign, or hash.** Determinism enables an external pipeline to hash the output; the crate itself ships no cryptography.

## References

Specifications and standards named in the source:

- ISO 32000 7.9.6 — PDF name trees (leaf `Names` array lexical key ordering).
- PDF/A-3 associated-file mechanism — catalog `Names.EmbeddedFiles` name tree, the `AF` associated-files array, and `Filespec` (`F`/`UF`, `EF.F`/`EF.UF`, `AFRelationship`).
- XMP / RDF metadata, including the PDF/A identification schema (`pdfaid:part`, `pdfaid:conformance`) and the PDF/A extension-schema mechanism (`pdfaExtension`, `pdfaSchema`, `pdfaProperty`).
- Factur-X 1.0 / ZUGFeRD 2.1 — profile set and attachment naming (`factur-x.xml`, `xrechnung.xml`); `fx:` extension-schema namespace `urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#`.
- veraPDF profile arguments `3b` / `3u` (named as the external validation gate, not invoked here).
- `typst_pdf::PdfOptions` (named as the upstream producer of the input PDF).

The decision rule for fixing missing PDF features here versus upstreaming to Typst is recorded in `plans/PDF-A-3-POSTPROC.md`. No external URLs are cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently (`publish = false`).
