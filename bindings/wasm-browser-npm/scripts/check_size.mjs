// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-108 bundle-size gate: walks dist/*/invoicekit_wasm_bg.wasm
// and asserts every produced bundle is under 5 MB. This is the
// load-bearing acceptance gate the bead specifies; the build
// fails loudly if a new feature pushes the bundle over.

import { statSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_ROOT = resolve(HERE, "..");

const MAX_BYTES = 5 * 1024 * 1024;
const TARGETS = ["web", "bundler", "node"];

let failures = 0;
for (const target of TARGETS) {
  const path = resolve(PKG_ROOT, "dist", target, "invoicekit_wasm_bg.wasm");
  if (!existsSync(path)) {
    console.error(`[T-108] FAIL: missing ${path} (did you run \`bun run build\`?)`);
    failures += 1;
    continue;
  }
  const bytes = statSync(path).size;
  const mb = (bytes / (1024 * 1024)).toFixed(2);
  if (bytes > MAX_BYTES) {
    console.error(
      `[T-108] FAIL: ${target} bundle is ${mb} MB (${bytes} bytes); cap is 5 MB`,
    );
    failures += 1;
  } else {
    console.log(
      `[T-108] OK: ${target} bundle is ${mb} MB (${bytes} bytes); under the 5 MB cap`,
    );
  }
}

if (failures > 0) {
  console.error(`[T-108] ${failures} bundle(s) exceeded the 5 MB cap`);
  process.exit(1);
}
console.log("[T-108] every bundle is under the 5 MB cap");
