# @invoicekit/core

Pure-TypeScript builder API on top of `@invoicekit/types`. No
runtime dependencies, no wasm. Builds `CommercialDocument` JSON
values that any InvoiceKit runtime (wasm, native binding, or
managed-API gateway) accepts.

```ts
import { buildCommercialDocument } from "@invoicekit/core";

const doc = buildCommercialDocument({
  id: "doc-001",
  documentNumber: "F-2026-001",
  documentType: "invoice",
  issueDate: "2026-05-27",
  currency: "EUR",
  supplier: {
    name: "Acme",
    address: { lines: ["1 Main St"], city: "Madrid", postalCode: "28013", country: "ES" },
  },
  customer: {
    name: "Buyer",
    address: { lines: ["12 rue de la Paix"], city: "Paris", postalCode: "75001", country: "FR" },
  },
  lines: [{ description: "Widget", quantity: "2", unitPrice: "100.00" }],
  tenantId: "tenant-x",
  traceId: "trace-001",
});
```

Tested on Node, Deno, and Bun. See `tests/builder.test.mjs` for
the full surface.

## License

Apache-2.0.
