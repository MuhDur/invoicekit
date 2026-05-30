# invoicekit-intake-pdf

Layer-1/Layer-2 PDF intake: deterministic text extraction from digital PDFs, plus Factur-X / ZUGFeRD embedded-XML extraction. Best-effort, no OCR.

## What it does

This crate reads two things out of a PDF byte stream:

1. The embedded structured-text layer of a "digital" PDF — a PDF that carries text show-operators, as opposed to a scanned image. `extract_pdf_text` walks every page's content stream, tracks the text-positioning state, and emits one `TextFragment` per `Tj`/`TJ`/`'`/`"` operator with its `(x, y)` origin in PDF user-space units (1 unit = 1/72 inch, origin at the lower-left of the page) and the font size set by the most-recent `Tf`. Fragments are then sorted into reading order per page.

2. The Factur-X / ZUGFeRD / XRechnung XML invoice carried as a PDF/A-3 file attachment. `extract_factur_x_xml` walks the embedded-files name tree and the `AF` associated-files array, finds the first `Filespec` whose name matches a canonical attachment name, and returns the decoded stream bytes.

Extraction is best-effort. Scanned PDFs (no text layer) and PDFs whose fonts use custom subset encodings produce no usable text here and are the documented hand-off to the OCR / vision-language fallback (a separate crate); this crate does not OCR and does not render.

## Capabilities

- **Digital text extraction** via `extract_pdf_text`, returning `StructuredText { pages: Vec<PageText> }`. Each `PageText` carries its 0-based index, `width_pt`/`height_pt` from the page `MediaBox` (inherited from a parent `Pages` node when the page does not declare one; defaults to US-Letter 612×792 if absent), and reading-order `TextFragment`s. `StructuredText::plain_text()` concatenates fragment text in reading order, form-feed-separated between pages.
- **Factur-X / ZUGFeRD attachment extraction** via `extract_factur_x_xml`, returning `Ok(Some(bytes))` on a match, `Ok(None)` for any other PDF (including a well-formed PDF with no XML attachment). Canonical names are exposed as `FACTUR_X_ATTACHMENT_NAMES`: `factur-x.xml`, `ZUGFeRD-invoice.xml`, `zugferd-invoice.xml`, `xrechnung.xml`. Name matching is **case-sensitive and byte-exact** by design — a mis-cased name (`Factur-X.xml`) is treated as not-Factur-X, not auto-corrected.
- **String decoding**: UTF-16BE with a `FE FF` byte-order mark (including correct combination of surrogate pairs for supplementary-plane characters such as emoji), then UTF-8, then a best-effort Latin-1 fallback. There is no `CMap`/`ToUnicode` reverse-lookup — custom-encoded subset fonts decode to whatever bytes the run stores, which may be mangled.
- **Script-aware reading-order reconstruction** (`script_order` module, heuristic):
  - Left-to-right lines: grouped into visual lines top-to-bottom, sorted left-to-right within a line.
  - Right-to-left lines (Arabic, Hebrew): detected with the Unicode Bidirectional Algorithm (UAX #9, via `unicode-bidi`) using a dominant-strong-class rule, then reordered into logical reading order at run granularity.
  - CJK vertical pages: detected by clustering near-constant-`x` columns of stacked CJK glyphs, then emitted right-to-left, each column top-to-bottom.
- **Resource ceilings** on untrusted input. Over-limit input is refused with `PdfTextError::TooLarge` or `PdfTextError::Page` rather than processed to exhaustion: max 4096 pages, max 1,000,000 text fragments across the document, a 64 MiB cap on a page's decompressed content stream, and a 16 MiB cap on a decompressed embedded-file stream. `FlateDecode` content is decoded through a size-bounded streaming reader so a decompression bomb stops at the ceiling instead of inflating fully. The embedded-files name tree is cycle-guarded (visited-set plus a 256-deep secondary cap).
- **Encrypted PDFs are rejected** with `PdfTextError::Encrypted`. The crate does not decrypt; supplying credentials is a future bead.
- **Errors are typed and total**: malformed input surfaces `PdfTextError::Parse`, a bad page content stream surfaces `PdfTextError::Page { page, detail }`; arbitrary binary input does not panic.

## Mode / Residuals

This is real text extraction over `lopdf`, not a stub or placeholder — the public functions return the actual decoded contents of the PDF. It is best-effort, not a guaranteed extractor, and the limits are deliberate:

- **No OCR, no rendering, no glyph rasterization.** Scanned PDFs yield no text and are routed elsewhere.
- **No `CMap`/`ToUnicode` reverse-lookup, no kerning, no text-rendering-mode awareness.** Standard Type1 / `WinAnsiEncoding` / `MacRomanEncoding` fonts (the common shape of invoice PDFs from Typst, LaTeX, `wkhtmltopdf`, LibreOffice, Word/Apache POI) extract to readable text; custom-encoded subset fonts produce mangled text — the documented gap for the OCR / vision fallback.
- **Reading-order reconstruction is heuristic, not exact.** Documented residuals (see the `script_order` module): each show-text run is assumed to store glyphs in *logical* order (the universal producer behaviour) — a run with baked-in *visual*-order glyphs is not reconstructed and routes to the OCR / vision fallback; RTL reordering is at run granularity, so a multi-run embedded left-to-right phrase on an RTL line has its runs reversed among themselves; mirrored bracket glyphs are reordered, never substituted; mixed vertical/horizontal lines are classified by majority and fall back to the horizontal path.
- **`MediaBox` fallback is a default, not a measurement.** A page with no resolvable `MediaBox` reports 612×792, which can be wrong for non-Letter pages; downstream code that only needs relative layout is unaffected.

## References

Specifications and standards named in the source:

- PDF content-stream text-positioning operators (`BT`/`ET`/`Tf`/`TL`/`Td`/`TD`/`Tm`/`T*`/`Tj`/`TJ`/`'`/`"`) and user-space coordinates (1 unit = 1/72 inch).
- PDF/A-3 associated-file attachment mechanism: catalog `Names.EmbeddedFiles` name tree and the `AF` associated-files array; `Filespec` `F`/`UF` and `EF.F`/`EF.UF` streams.
- Factur-X / ZUGFeRD / XRechnung embedded-invoice attachment naming.
- Unicode Bidirectional Algorithm, UAX #9 (via the `unicode-bidi` crate).
- UTF-16BE / UTF-8 string decoding.

No external URLs are cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
