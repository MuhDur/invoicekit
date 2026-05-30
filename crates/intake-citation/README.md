# invoicekit-intake-citation

The bounding-box citation taxonomy: a serializable schema for recording where each extracted invoice field came from. This crate defines data types and validators only — it does no extraction, OCR, PDF parsing, or model inference.

## What it does

InvoiceKit's intake layers (digital-PDF text extraction, server-side OCR, vision-language model) emit a citation alongside every field they extract into the intermediate representation. A citation names the source region the layer read — a page rectangle, a PDF object id, an OCR span id, or a model id — so the audit UI can highlight the exact spot and the evidence bundle can archive the provenance next to the canonical document.

This crate is the schema for those citations. It provides the types, the validated constructors, and a ledger that groups citations by field path. The actual extraction lives in sibling crates (`invoicekit-intake-pdf`, `invoicekit-intake-ocr`, `invoicekit-intake-vlm`); this crate has no dependency on any extractor, OCR engine, or model and runs none of them. Its only runtime dependencies are `serde`, `serde_json`, and `thiserror`.

## Capabilities

Types:

- `BoundingBox` — a page rectangle in PDF user-space coordinates (origin bottom-left, points = 1/72 inch): `page` (1-indexed), `x`, `y`, `width`, `height` as `f32`. Helpers: `right()`, `top()`, and `overlaps(other)` (same-page rectangle intersection). `BoundingBox::validated` rejects page `0`, non-finite coordinates, and non-positive width/height.
- `ExtractionLayer` — the layer that produced a citation: `DigitalPdfText`, `DigitalPdfXml`, `ServerOcr`, `VisionLanguageModel`, `HumanOverride`. `slug()` returns the kebab-case wire name; `default_confidence()` returns a fixed per-layer floor (`1.0` for digital-PDF/XML/human-override, `0.75` for OCR, `0.55` for VLM) used by the audit UI when an extractor supplies no per-citation confidence.
- `Confidence` — an `f32` in `[0.0, 1.0]`. `Confidence::new` clamps out-of-range input and maps `NaN` to `0.0`.
- `CitationSource` — a discriminated union of source pointers: `PdfObject { object_id, bounding_box? }`, `BoundingBox { bounding_box }`, `OcrSpan { span_id, bounding_box? }`, `Model { model_id, bounding_box? }`. `bounding_box()` returns the rectangle the variant carries, if any. Serialized with an internal `kind` tag, kebab-case.
- `BoundingBoxCitation` — one rectangle citation for a field: `path` (JSON-pointer-style), `bounding_box`, `layer`, `source_text` (verbatim text the layer read), `confidence`, `extractor_id`. `validated` rejects empty path or extractor id.
- `FieldCitation` — the `{value, source, confidence}` shape with `path` and `layer`: a field value paired with a typed `CitationSource`. `validated` rejects empty path or value, and empty `span_id` / `model_id` inside an `OcrSpan` / `Model` source.
- `CitationLedger` — citations grouped by field path in a `BTreeMap<String, Vec<BoundingBoxCitation>>`, giving stable iteration order for deterministic bundle output. Methods: `record`, `is_empty`, `len`, `iter`, `for_path`, `winner_for` (last-recorded citation for a path — later layers override earlier ones), `has_low_confidence(path, floor)`.
- `CitationError` — the validation error enum (`InvalidPage`, `NonPositiveExtent`, `NonFiniteCoordinate`, `EmptyPath`, `EmptyExtractorId`, `EmptyValue`, `EmptyOcrSpanId`, `EmptyModelId`).

Functions:

- `crate_name() -> &'static str` — returns `"invoicekit-intake-citation"`.

All public types derive `serde::Serialize` / `Deserialize` and round-trip through JSON.

## Mode / Residuals

- **Schema-only.** This crate validates and serializes citation records. It does not produce them. There is no PDF reader, no OCR, no model here. Bounding boxes, OCR span ids, model ids, and confidence scores are values the calling extractor must supply.
- **Confidence is a passthrough.** `Confidence` clamps and stores the value an extractor gives. `default_confidence()` is a fixed per-layer constant, not a measurement. Nothing in this crate estimates extraction accuracy.
- **No spatial geometry beyond same-page rectangle overlap.** `overlaps` is an axis-aligned same-page intersection test. There is no rotation handling (the doc note states rotation belongs in the PDF page `/Rotate` field, not the box), no coordinate-space transform, and no page-size awareness.
- **No text encoding logic.** `source_text` and `value` are stored verbatim as Rust `String`s (UTF-8). The crate performs no normalization, no right-to-left or bidirectional reshaping, no CJK handling, and no Unicode processing. Any such handling, if needed, lives in the extractor that fills these fields.
- **Validation is shallow.** `validated` constructors check page bounds, coordinate finiteness, positive extent, and non-empty strings. They do not check that a `path` is a well-formed JSON pointer, that a bounding box lies within a real page, or that `source_text` matches the cited region.
- **No cryptography.** This crate hashes nothing, signs nothing, and contains no placeholder crypto. Hashing and signing of the evidence bundle that carries these citations happen in `invoicekit-evidence` and the signer crates.

## References

- PDF user-space coordinate convention (origin bottom-left, units of 1/72 inch; 1-indexed page numbers; 0-indexed object ids; page rotation in the `/Rotate` field) — referenced in the source doc-comments; no external URL is cited in the code.
- `invoicekit-ir` `CommercialDocument` — the extracted document model the `path` fields point into (`https://docs.rs/invoicekit-ir`, the only URL present in the source).
- Internal plan references in the source: `PLAN.md §3.4 / §4.4` (layered intake) and task ids `T-062`, `T-063`, `T-066`.

Engine names appearing in doc-comments and examples (`PaddleOCR`, `SmolDocling`-256M) are illustrative `extractor_id` / `model_id` string values, not dependencies of this crate.

## License

Apache-2.0. Part of the InvoiceKit workspace.
