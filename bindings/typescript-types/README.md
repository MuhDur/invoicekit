# @invoicekit/types

Generated TypeScript types for the InvoiceKit JSON Schemas. The Rust source of truth lives in `crates/ir` and friends; the committed schemas under `schemas/*.json` are the contract this package exposes to TypeScript / JavaScript consumers.

## What's in the box

Each module under [`src/generated/`](./src/generated) corresponds 1:1 to a schema in [`schemas/`](../../schemas):

- [`schemas/invoicekit-ir-v1.json`](../../schemas/invoicekit-ir-v1.json) → [`src/generated/invoicekit_ir_v1.d.ts`](./src/generated/invoicekit_ir_v1.d.ts) — `CommercialDocument` and the rest of the layered IR.
- [`schemas/validation-result.schema.json`](../../schemas/validation-result.schema.json) → [`src/generated/validation_result.d.ts`](./src/generated/validation_result.d.ts) — `ValidationResult`, `Finding`, severity tags.

Future schemas (capabilities, codelists, …) get a generated module automatically the next time `scripts/generate.mjs` runs.

`src/index.ts` re-exports everything namespace-flat so consumers can do:

```ts
import { CommercialDocument, ValidationResult } from "@invoicekit/types";
```

## Regenerating after a Rust IR change

```bash
cd bindings/typescript-types
npm install
npm run generate
npm run check         # tsc --noEmit
npm test              # round-trip + drift checks
git add src/generated src/index.ts
```

CI runs `node scripts/generate.mjs --check` and fails when the committed files differ from a fresh generation — i.e. you cannot land a Rust IR change without also re-running the generator and committing the diff.

## Why types live in this repo

The TypeScript SDK ([T-103](../../plans/PLAN.md)) depends on this package. The schemas are the contract between Rust and TypeScript, and `@invoicekit/types` is the artifact that turns "the schema accepts this" into a compile-time guarantee for downstream TypeScript consumers.

## Publishing (not yet wired)

`publishConfig.access` is `public`. The actual `npm publish` is gated on T-103 / T-104 SDK release work and an `NPM_TOKEN` secret that does not exist today. Until that lands the package is consumed via local path resolution from the InvoiceKit monorepo.
