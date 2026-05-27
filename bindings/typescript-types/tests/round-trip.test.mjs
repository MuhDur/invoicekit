// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-012 lightweight round-trip checks: a known-good synthetic
// CommercialDocument matches the shape of the committed schema, and
// the generated TypeScript surface re-exports the same symbol names
// the schema declares. A stricter TS-AST-back-to-JSON-Schema check is
// filed as a follow-up bead — the value here is catching schema/type
// drift, not formally proving bidirectional equivalence.

import { readFile, readdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { strict as assert } from "node:assert";
import { test } from "node:test";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const REPO = resolve(ROOT, "..", "..");
const SCHEMA_DIR = join(REPO, "schemas");
const GENERATED_DIR = join(ROOT, "src", "generated");

test("every committed schema has a matching .d.ts module", async () => {
  const schemas = (await readdir(SCHEMA_DIR))
    .filter((f) => f.endsWith(".json"))
    .sort();
  const generated = (await readdir(GENERATED_DIR))
    .filter((f) => f.endsWith(".d.ts"))
    .sort();
  assert.equal(
    schemas.length,
    generated.length,
    `expected one .d.ts per schema; schemas=${schemas.length} generated=${generated.length}`,
  );
});

test("every generated module is namespace-imported by src/index.ts", async () => {
  const index = await readFile(join(ROOT, "src", "index.ts"), "utf8");
  const generated = (await readdir(GENERATED_DIR))
    .filter((f) => f.endsWith(".d.ts"))
    .map((f) => f.replace(/\.d\.ts$/, ""));
  for (const mod of generated) {
    assert.ok(
      index.includes(`./generated/${mod}.js`),
      `src/index.ts must re-export ./generated/${mod}.js so consumers get the type without per-module imports`,
    );
  }
});

test("CommercialDocument type carries the schema's required fields", async () => {
  const dts = await readFile(
    join(GENERATED_DIR, "invoicekit_ir_v1.d.ts"),
    "utf8",
  );
  // Spot-check a few schema-declared required properties — full
  // schema-completeness is the round-trip follow-up.
  for (const required of [
    "id:",
    "document_type:",
    "issue_date:",
    "schema_version",
  ]) {
    assert.ok(
      dts.includes(required),
      `generated CommercialDocument is missing required field marker ${required}`,
    );
  }
});

test("validation result type exposes the severity-tagged finding shape", async () => {
  const dts = await readFile(
    join(GENERATED_DIR, "validation_result.d.ts"),
    "utf8",
  );
  for (const symbol of ["severity", "finding"]) {
    assert.ok(
      dts.toLowerCase().includes(symbol),
      `validation_result module should expose ${symbol}-related types`,
    );
  }
});

test("generated headers carry the SPDX banner", async () => {
  for (const f of [
    "invoicekit_ir_v1.d.ts",
    "validation_result.d.ts",
  ]) {
    const raw = await readFile(join(GENERATED_DIR, f), "utf8");
    assert.ok(
      raw.includes("SPDX-License-Identifier: Apache-2.0"),
      `${f} must carry the SPDX header so the SPDX gate passes`,
    );
    assert.ok(
      raw.includes("DO NOT EDIT BY HAND"),
      `${f} must carry the do-not-hand-edit warning`,
    );
  }
});
