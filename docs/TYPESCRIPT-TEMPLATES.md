# TypeScript Templates

InvoiceKit templates are authored in TypeScript and compiled to Typst. The DSL
keeps template authors on typed invoice fields and builder functions instead of
raw Typst syntax.

From a clean clone:

```bash
cd templates/typescript
bun install
bun run verify
```

Use the compiler from TypeScript:

```ts
import {
  compileTemplate,
  defineTemplate,
  heading,
  money,
  paragraph,
  strong,
  text,
  type CommercialDocumentTemplateData,
} from "./src/index.ts";
import { data as invoice } from "./examples/basic-invoice.ts";

const template = defineTemplate<CommercialDocumentTemplateData>(
  { name: "docs-example", title: "Docs example" },
  (invoice) => [
    heading(1, text("Invoice "), strong(invoice.documentNumber)),
    paragraph(strong("Payable"), text(` ${money(invoice.totals.payable)}`)),
  ],
);

const typst = compileTemplate(template, invoice);
```

The package ships five example templates under `templates/typescript/examples`
and freezes their compiled Typst in `templates/typescript/golden`.
