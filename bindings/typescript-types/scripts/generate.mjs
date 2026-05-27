// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-012 type generator. Reads every JSON Schema under ../../schemas/
// and emits a corresponding .d.ts file under src/generated/. The
// produced files are committed to the repo so contributors can review
// the type surface in PRs; CI re-runs this script and fails the
// build if the committed files differ from the freshly-generated
// output, guaranteeing the .d.ts stays in lockstep with the Rust
// source of truth.
//
// Usage:
//
//     bun run scripts/generate.mjs              # write
//     bun run scripts/generate.mjs --check      # write to temp + diff committed
//
// `--check` is what CI runs; the local default is to write so a
// developer can `bun run generate && git add -p` after changing a
// Rust IR type. Uses `bun` per AGENTS.md ("Use bun for everything
// JavaScript/TypeScript. Never use npm, yarn, or pnpm in our own
// development scripts.").

import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative, resolve } from "node:path";
import { compile } from "json-schema-to-typescript";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..");
const REPO = resolve(ROOT, "..", "..");
const SCHEMA_DIR = join(REPO, "schemas");
const OUT_DIR = join(ROOT, "src", "generated");
const INDEX_FILE = join(ROOT, "src", "index.ts");

const HEADER = `// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with \`bun run generate\` from
// bindings/typescript-types/. Source of truth: schemas/.
//
`;

const COMPILE_OPTS = {
  bannerComment: "",
  style: { semi: true, singleQuote: false, trailingComma: "all" },
  format: false,
  additionalProperties: false,
  ignoreMinAndMaxItems: true,
  unreachableDefinitions: true,
};

function moduleNameFor(schemaFile) {
  // schemas/invoicekit-ir-v1.json -> invoicekit_ir_v1
  return schemaFile.replace(/\.schema\.json$/, "").replace(/\.json$/, "").replace(/[^a-z0-9]+/gi, "_");
}

async function listSchemas() {
  const entries = await readdir(SCHEMA_DIR);
  return entries
    .filter((f) => f.endsWith(".json"))
    .sort();
}

async function generate({ check }) {
  const schemas = await listSchemas();
  if (schemas.length === 0) {
    throw new Error(`no schemas found under ${SCHEMA_DIR}`);
  }

  const fresh = new Map();
  for (const file of schemas) {
    const path = join(SCHEMA_DIR, file);
    const raw = await readFile(path, "utf8");
    const schema = JSON.parse(raw);
    const ts = await compile(schema, moduleNameFor(file), COMPILE_OPTS);
    fresh.set(moduleFor(file), HEADER + ts);
  }
  const indexContent = HEADER + buildIndex([...fresh.keys()]);

  if (check) {
    const issues = [];
    if (!existsSync(OUT_DIR)) {
      issues.push(`missing directory ${relative(REPO, OUT_DIR)}`);
    } else {
      for (const [mod, expected] of fresh) {
        const path = join(OUT_DIR, `${mod}.d.ts`);
        const actual = existsSync(path) ? await readFile(path, "utf8") : "";
        if (actual !== expected) {
          issues.push(
            `committed ${relative(REPO, path)} does not match freshly-generated output`,
          );
        }
      }
      const indexPath = INDEX_FILE;
      const actualIndex = existsSync(indexPath) ? await readFile(indexPath, "utf8") : "";
      if (actualIndex !== indexContent) {
        issues.push(
          `committed ${relative(REPO, indexPath)} does not match freshly-generated index`,
        );
      }
      const committed = (await readdir(OUT_DIR)).filter((n) => n.endsWith(".d.ts"));
      const expectedSet = new Set([...fresh.keys()].map((m) => `${m}.d.ts`));
      for (const name of committed) {
        if (!expectedSet.has(name)) {
          issues.push(`extra ${name} in src/generated/ has no schema; delete it`);
        }
      }
    }
    if (issues.length > 0) {
      console.error("T-012 type-generation drift detected:");
      for (const i of issues) {
        console.error(`  - ${i}`);
      }
      console.error(
        "\nrun `bun --cwd bindings/typescript-types run generate` and commit the diff",
      );
      process.exit(1);
    }
    console.log("T-012 type-generation: committed types match schema-driven generation");
    return;
  }

  await rm(OUT_DIR, { recursive: true, force: true });
  await mkdir(OUT_DIR, { recursive: true });
  for (const [mod, contents] of fresh) {
    await writeFile(join(OUT_DIR, `${mod}.d.ts`), contents);
  }
  await writeFile(INDEX_FILE, indexContent);
  console.log(`T-012 type-generation: wrote ${fresh.size} module(s) to src/generated/`);
}

function moduleFor(schemaFile) {
  return moduleNameFor(schemaFile);
}

function buildIndex(modules) {
  const lines = [
    "//",
    "// Public TypeScript surface for @invoicekit/types. Each module under",
    "// ./generated/ corresponds 1:1 to a committed JSON Schema in",
    "// schemas/. Re-exports stay deliberately namespace-flat: callers",
    "// import { CommercialDocument } from \"@invoicekit/types\".",
    "",
    "// apc8: `export type *` (TypeScript 5.0+) marks each re-export",
    "// as types-only so tsc erases the statement at emit time.",
    "// Without this, dist/index.js would contain `export * from",
    "// \"./generated/*.js\"` — but the target src/generated/*.d.ts",
    "// files are declaration-only, so npm/pnpm/yarn/deno consumers",
    "// would hit \"Cannot find module\" at runtime.",
    "",
  ];
  for (const mod of modules) {
    lines.push(`export type * from "./generated/${mod}.js";`);
  }
  lines.push("");
  return lines.join("\n");
}

const check = process.argv.includes("--check");
generate({ check }).catch((e) => {
  console.error(e);
  process.exit(1);
});
