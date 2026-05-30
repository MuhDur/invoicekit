<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-pdf-visual-regression

Developer/CI tool that catches unintended visual changes in rendered invoice PDFs by rasterising them and diffing against committed baseline images. Not part of the engine or any shipped artifact.

## Capabilities

- **`diff`** — the full pipeline: walks `conformance-corpus/pdf-snapshots/MANIFEST.json`, rasterises each declared PDF candidate at 144 DPI via pdfium (requires the `pdfium` cargo feature), diffs each against its committed baseline PNG with a fast per-pixel RGBA delta plus an anti-aliasing tolerance, and emits a markdown report.
- **`diff-png`** — diffs a pair of already-rasterised PNGs without pdfium (handy for unit tests and local debugging).
- Additional CLI mode(s) per the binary's `--help`.

## Mode / Residuals

- Rasterisation needs the `pdfium` feature (and a pdfium library available); the `diff-png` path works without it.
- Comparison is a per-pixel RGBA delta with an anti-aliasing tolerance — not a semantic/structural PDF diff.
- Baselines are committed PNGs under the conformance corpus; a diff flags drift, it does not decide intent.

## License

Apache-2.0.
