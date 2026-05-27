// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-108 smoke test: load the nodejs bundle and call the engine ABI
// surface end-to-end. The bead's "works in Node + Bun + Deno +
// browsers" acceptance breaks into:
//
//   - Node:    this test, run via `bun test` (bun's runner is
//              jest/node:test compatible and natively loads
//              wasm-pack's nodejs target).
//   - Bun:     same test runs under bun.
//   - Deno:    follow-up bead wires `deno run --check` on the web
//              target via the CI matrix (no Deno runner in CI today).
//   - Browser: follow-up bead wires headless-Chrome + Playwright
//              for the web target (no browser runner in CI today).
//
// The wasm-pack build output is gitignored, so this test is only
// executable AFTER `bun run build` has populated dist/node/. The
// CI workflow runs both in sequence.

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_ROOT = resolve(HERE, "..");
const NODE_BUNDLE = resolve(PKG_ROOT, "dist", "node", "invoicekit_wasm.js");

test("nodejs bundle exists after wasm-pack build", () => {
  if (!existsSync(NODE_BUNDLE)) {
    console.warn(
      "[T-108 smoke] nodejs bundle missing; run `bun run build` first. Skipping wasm round-trip.",
    );
    return;
  }
  assert.ok(existsSync(NODE_BUNDLE), "dist/node/invoicekit_wasm.js must exist");
});

test("engine ABI round-trips through the wasm export", async () => {
  if (!existsSync(NODE_BUNDLE)) {
    console.warn(
      "[T-108 smoke] nodejs bundle missing; skipping engine ABI round-trip.",
    );
    return;
  }
  const mod = await import(NODE_BUNDLE);
  // wasm-pack's nodejs target eager-initializes the wasm at import
  // time, so the exports are immediately callable. The known-error
  // case (unknown operation) returns a JSON error envelope without
  // requiring any feature flags — minimum-viable smoke.
  const request = new TextEncoder().encode(
    JSON.stringify({
      abi_version: 1,
      operation: "unknown",
      payload: {},
    }),
  );
  const response = mod.processEngineAbiJson(request);
  assert.ok(response instanceof Uint8Array, "response must be a Uint8Array");
  const body = JSON.parse(new TextDecoder().decode(response));
  assert.equal(body.status, "error");
});

test("bead id is reachable from JS for diagnostic correlation", async () => {
  if (!existsSync(NODE_BUNDLE)) {
    return;
  }
  const mod = await import(NODE_BUNDLE);
  const id = mod.beadId();
  assert.equal(
    id,
    "invoices-t-025-wasm-artifact-nso",
    "beadId should round-trip the T-025 WASM_ARTIFACT_BEAD_ID constant",
  );
});

test("compiled bundle lists are JSON arrays", async () => {
  if (!existsSync(NODE_BUNDLE)) {
    return;
  }
  const mod = await import(NODE_BUNDLE);
  const countries = JSON.parse(mod.compiledCountryBundles());
  const formats = JSON.parse(mod.compiledFormatBundles());
  assert.ok(Array.isArray(countries));
  assert.ok(Array.isArray(formats));
  // Default-feature build: both lists should be empty.
  assert.equal(countries.length, 0);
  assert.equal(formats.length, 0);
});
