<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-intake-ocr

Typed surface for the optical-character-recognition (OCR) layer of the InvoiceKit intake pipeline. The OCR backends are not implemented here: every provider in this crate returns a fixed placeholder token, not text read from the source document.

## What it does

This crate defines the contract that the intake pipeline uses to call an OCR backend, plus three provider implementations that satisfy that contract without doing any recognition. It exists so the engine's intake wiring is stable before the real runtimes land. The live OCR backends ship in a follow-up crate (the Cargo description names `intake-ocr-onnx`); this crate carries only the typed surface and stubs.

The OCR layer is one rung of a larger intake stack described in PLAN.md §3.5. Digital PDF text extraction and Factur-X embedded-XML extraction live in `intake-pdf`; cloud vision-language inference lives in `intake-vlm`. This crate is the slot for server-side OCR (PaddleOCR) and a small on-device model (SmolDocling), neither of which is wired up yet.

## Capabilities

- `OcrProvider` trait: `layer() -> OcrLayer` and `recognise(&[u8]) -> Result<OcrResult, OcrError>`. The trait is `Send + Sync`.
- Typed result model that round-trips through serde: `OcrResult` (layer, tokens in reading order, `page_count`, `mean_confidence`), `OcrToken` (UTF-8 `text`, `bbox`, `confidence` in `[0.0, 1.0]`), `BoundingBox` (page index plus x/y/width/height in PDF points).
- `OcrLayer` enum (`PaddleOcr`, `SmolDocling`, `Mock`) with kebab-case serde, and `OcrError` (`BadSource`, `Backend`).
- Three provider structs, all of which reject empty source bytes with `OcrError::BadSource`:
  - `MockOcrProvider` — returns a single fixed `INV-MOCK-1` token at the top-left of page 0, confidence 1.0.
  - `PaddleOcrProvider { sidecar_url }` — returns a `STUB-paddle-len-<n>` token at confidence 0.9. The `sidecar_url` field is stored but never used.
  - `SmolDoclingProvider { model_path }` — rejects an empty `model_path` with `OcrError::Backend`, otherwise returns a `STUB-smoldocling-len-<n>` token at confidence 0.9. The `model_path` is checked for emptiness but no model is loaded.
- `crate_name() -> &'static str` — returns `"invoicekit-intake-ocr"`.

## Mode / Residuals

This crate is a substrate, not a working OCR engine. No provider here reads, decodes, rasterizes, or recognizes any text from the input bytes.

- `MockOcrProvider` always returns the literal token `INV-MOCK-1` regardless of input content. It is a deterministic test baseline.
- `PaddleOcrProvider` and `SmolDoclingProvider` ignore the document content entirely. Each returns one synthetic token whose text encodes only the layer name and the input byte length. The token text, bounding box, and 0.9 confidence are constants, not measurements.
- There is no PDF parsing, no image decoding, no PaddleOCR sidecar call, no ONNX runtime, and no model load. The only validation performed is the empty-source check (all providers) and the empty-`model_path` check (`SmolDoclingProvider`).
- No script-specific handling exists. There is no right-to-left, Chinese/Japanese/Korean, or other Unicode-script processing — the providers never produce text derived from the source, so there is nothing to handle. The `text` field type is `String`, so any future real backend can carry UTF-8.

What the real path needs: an actual OCR backend. For Layer 3, a PaddleOCR sidecar process reachable at `sidecar_url` that the provider POSTs document bytes to. For Layer 4, an ONNX runtime that loads the SmolDocling-256M model at `model_path` and runs inference. When extraction is real it will be best-effort — OCR output is inherently approximate and confidence-scored.

## References

- PLAN.md §3.5 — intake pipeline layering (cited in the crate module documentation).

The names PaddleOCR, SmolDocling-256M, and ONNX appear in the source as labels for the intended backends. They are not dependencies of this crate and no such runtime is invoked.

## License

Apache-2.0.
