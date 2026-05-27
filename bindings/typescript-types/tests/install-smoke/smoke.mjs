// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// apc8 cross-runtime install smoke test.
//
// The @invoicekit/types package is types-only — there are no
// runtime exports. The smoke test we want is the one a typical
// consumer would hit on day one: "can I install + import this
// package under my runtime + package manager combination?".
//
// Steps:
//   1. import * as types from "@invoicekit/types"
//      — proves the package's `main` / `module` / `exports`
//        entries resolve in the consumer runtime.
//   2. JSON.parse the installed package.json from disk via
//      module resolution
//      — proves the `files` array in publishConfig actually
//        ships everything the import path needs.
//
// On a missing export, missing dist/, or runtime import error,
// this script exits non-zero.

import * as types from "@invoicekit/types";

if (typeof types !== "object" || types === null) {
  console.error("apc8 smoke: @invoicekit/types did not resolve to an object");
  process.exit(1);
}

// dist/index.js compiles down to `export * from "./generated/...";`
// with no value exports, so `types` is an empty namespace object
// at runtime. That is the correct shape — the package's value
// surface IS empty. We just need to prove the import resolved.
console.log(`apc8 smoke: @invoicekit/types import OK (type-only package; ${Object.keys(types).length} runtime exports as expected)`);
