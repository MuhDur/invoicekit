// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with `bun run generate` from
// bindings/typescript-types/. Source of truth: schemas/.
//
//
// Public TypeScript surface for @invoicekit/types. Each module under
// ./generated/ corresponds 1:1 to a committed JSON Schema in
// schemas/. Re-exports stay deliberately namespace-flat: callers
// import { CommercialDocument } from "@invoicekit/types".

// apc8: `export type *` (TypeScript 5.0+) marks each re-export
// as types-only so tsc erases the statement at emit time.
// Without this, dist/index.js would contain `export * from
// "./generated/*.js"` — but the target src/generated/*.d.ts
// files are declaration-only, so npm/pnpm/yarn/deno consumers
// would hit "Cannot find module" at runtime.

export type * from "./generated/invoicekit_capabilities_v1.js";
export type * from "./generated/invoicekit_ir_v1.js";
export type * from "./generated/validation_result.js";
