# @invoicekit/render

TypeScript wrapper around the InvoiceKit wasm engine's HTML
renderer (`render_html`). Validates input lightly, calls the
engine, surfaces typed errors. Pair with `@invoicekit/wasm` for
browsers, bundlers, and Node/Deno/Bun.

```ts
import { createRendererFromWasmModule } from "@invoicekit/render";
import * as wasm from "@invoicekit/wasm";

const renderer = createRendererFromWasmModule(wasm);
const html = renderer.renderHtml(doc, { palette: "default", locale: "en-GB" });
```

## License

Apache-2.0.
