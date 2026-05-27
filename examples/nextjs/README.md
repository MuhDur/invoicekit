# examples/nextjs

Reference Next.js demo for InvoiceKit. Issues three German XRechnung shapes
(basic, with allowance, reverse charge) using `@invoicekit/core` — the
pure-TypeScript builder that runs on the server side of any Next.js app.

## 5-minute setup

```bash
# 1. Clone + install
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit/examples/nextjs

# 2. The @invoicekit/core dependency is a workspace link;
#    bun resolves it automatically.
bun install

# 3. Run the dev server
bun run dev
# → http://localhost:3000
```

Pick one of the three fixtures, click **Issue**, and the
server-side `app/api/issue/route.ts` handler builds a validated
`CommercialDocument` JSON and returns it to the page.

## Smoke test

```bash
bun run smoke
```

Builds every fixture through the same builder the API route uses and
asserts the canonical shape (line subtotals, payable amount, customer
country, tax category for reverse-charge).

## Files

| Path                              | Purpose                                  |
|-----------------------------------|------------------------------------------|
| `app/layout.tsx`                  | Root layout.                             |
| `app/page.tsx`                    | Landing page + Issue button.             |
| `app/api/issue/route.ts`          | POST endpoint that calls the builder.    |
| `fixtures/index.ts`               | Three German XRechnung fixtures.         |
| `tests/smoke.test.mjs`            | Builder smoke test (CI gate).            |

## Architecture

The demo deliberately uses only `@invoicekit/core` (the pure-TypeScript
builder). The wasm engine (`@invoicekit/wasm`) and REST gateway
(`@invoicekit/managed`) are not required for issuing a `CommercialDocument`
JSON — those packages handle rendering, validation, and submission, which
this demo intentionally leaves as exercises.

To extend the demo to call the wasm engine for full XSD validation, add
`@invoicekit/wasm` to dependencies and wire it into the API route per the
package README.

## License

Apache-2.0.

Implemented by `invoices-t-1400-demo-nextjs-dkjl`.
