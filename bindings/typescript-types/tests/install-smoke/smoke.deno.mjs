// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// apc8 Deno-specific install smoke. Deno reads npm: specifiers
// directly without an install step, so we point at the locally
// packed tarball via a temporary node_modules tree that
// `npm install ./invoicekit-types-*.tgz` produced.
//
// The runtime check is identical to smoke.mjs but the entry
// uses `npm:@invoicekit/types` so Deno resolves through its npm
// compatibility layer instead of its raw URL loader.

import * as types from "npm:@invoicekit/types@*";

if (typeof types !== "object" || types === null) {
  console.error("apc8 smoke (deno): @invoicekit/types did not resolve to an object");
  Deno.exit(1);
}

console.log(`apc8 smoke (deno): @invoicekit/types import OK`);
