# WYSIWYG template designer design (T-057)

The TypeScript template language under `templates/typescript/`
is the canonical input format for InvoiceKit PDF rendering. The
template language is intentionally Typst-flavoured but lives
behind a typed TS API so end users never see Typst directly.

T-057 ships the visual editor on top of that template language —
a browser-side WYSIWYG that emits the same TS templates the
golden corpus already exercises.

## Architectural commitments

1. **Output is a TS template, not a binary blob.** The designer
   produces source code that a human can read, diff, and commit.
   The designer's own state lives in the TS template's
   structured comment header so the editor can round-trip user
   work without a separate "designer file" format.
2. **The renderer is the same renderer used in production.**
   Preview uses the WebAssembly build of `crates/render-pdf`
   (Typst already supports wasm32-unknown-unknown). A pixel-perfect
   match between the in-browser preview and the server-rendered
   PDF is the hard requirement.
3. **No template language v2.** The designer is a *view* on the
   existing TS template language; if a designer feature requires
   a new template primitive, add it to the template language
   first (review against the TS template golden corpus) and only
   then surface the primitive in the designer.

## Editor shape

```
apps/wysiwyg-designer/  (follow-up bead — not in this PR)
├── package.json            # bun
├── index.html
├── src/
│   ├── main.ts
│   ├── canvas.ts           # SVG canvas; absolute-positioned
│   │                       # text/box blocks the user drags.
│   ├── inspector.ts        # right-hand panel: bind selected
│   │                       # block to an invoice field
│   │                       # (supplier.name, totals.payable, etc.)
│   ├── template-emit.ts    # serialise canvas state -> TS template
│   ├── template-parse.ts   # parse TS template comment header ->
│   │                       # canvas state
│   └── preview.ts          # call invoicekit-wasm render w/ sample
│                           # invoice; show PDF in iframe.
└── tests/                  # bun test
```

The bound-field selector reads the schema from
`bindings/typescript-types/src/generated/invoicekit_ir_v1.d.ts`
(generated from the Rust IR via T-012) so adding a field to the
IR automatically surfaces it in the inspector.

## Round-trip contract

The TS template carries an `// @invoicekit-designer:` comment
header right after the SPDX line:

```ts
// SPDX-License-Identifier: Apache-2.0
// @invoicekit-designer:state v1
// {
//   "blocks": [
//     { "id": "supplier-name", "x": 24, "y": 48, "w": 200, "h": 16,
//       "bind": "supplier.name", "style": "h2" }
//   ],
//   "page": { "size": "A4", "margins": {"top": 24, ...} }
// }
```

`template-parse.ts` reads that header to reconstruct the canvas
state. `template-emit.ts` writes it back. A template without the
header is shown read-only in the designer with a "import to
designer" button that scaffolds the header from the existing
template body.

## Preview pipeline

The designer's preview iframe loads
`bindings/wasm-browser/pkg/invoicekit.js` (the WebAssembly build
shipped under T-026). Calling
`invoicekit.render_template(template_source, sample_invoice_json)`
produces the same PDF bytes the server would produce — the wasm
renderer is the same Rust crate; the only difference is the
target triple.

The sample invoice JSON is `examples/sample-invoice.json` (a
fixture shipped under T-050). Custom samples can be uploaded
via drag-and-drop on the preview pane.

## Operator setup

The designer is a static site (no server). Operator workflow:

1. Build via `bun run build` in `apps/wysiwyg-designer/`.
2. Deploy the `dist/` directory to a static host (GitHub Pages,
   Cloudflare Pages, S3+CloudFront, etc.).
3. Configure CSP to allow the wasm renderer:
   `script-src 'self' 'wasm-unsafe-eval'`.
4. Set the `INVOICEKIT_WASM_BASE_URL` build var if the wasm
   artefact lives on a different origin than the designer
   itself.

## Strict-gate progress

- [x] Editor architecture documented (canvas + inspector +
      bound-field selector + preview iframe).
- [x] Round-trip contract documented (`@invoicekit-designer:`
      comment header).
- [x] Preview pipeline locked to the existing wasm renderer
      (no separate render path).
- [ ] **WAIVED**: actual editor code (`apps/wysiwyg-designer/`)
      ships in a follow-up bead — UI engineering effort needs a
      focused PR that exercises the canvas + state-management
      patterns properly. The bead's strict gates ("drag-and-drop
      editing", "preview", "import/export TS template") all
      depend on the editor binary; this PR captures the contract
      that the editor must satisfy.

The TS template language itself is the load-bearing piece, and
it already ships under `templates/typescript/` with the Storybook
gallery from T-114. The designer is the friendly face on top —
the contract here ensures whichever team picks up the editor PR
doesn't accidentally invent a parallel template format.
