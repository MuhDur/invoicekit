<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-render-factur-x-acceptance

CI acceptance binary that renders a fixed set of Factur-X PDFs into an output directory so the `.github/workflows/pdfa3-verapdf.yml` job can validate them with `verapdf --profile=3b` and `--profile=3u`. A test fixture generator, not a shipped artifact.

## Capabilities

Renders 30 Factur-X PDFs (5 per profile × 6 profiles). Each PDF:

1. Starts as the byte-stable hello-world invoice from `invoicekit-render-pdf::render_hello_world_invoice` (Typst-rendered, PDF/A-3b conformant by construction).
2. Has a profile-tagged Factur-X CII XML embedded via `invoicekit-render-pdf-postproc::embed_factur_x`, which writes the `Names.EmbeddedFiles` name tree, the `AF` array on the catalog, and a profile-aware XMP packet.

## Mode / Residuals

- Produces PDFs for an external veraPDF gate to check; this binary does **not** itself run veraPDF or assert PDF/A conformance — that is the CI job's responsibility.
- The invoice content is the fixed hello-world fixture; this is an acceptance-corpus generator, not a general renderer.

## License

Apache-2.0.
