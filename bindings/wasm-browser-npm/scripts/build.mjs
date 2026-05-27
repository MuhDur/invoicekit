// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-108 build driver: runs wasm-pack for the three runtime
// targets (web, bundler, nodejs) so a single `bun run build`
// produces the multi-target dist/ tree that package.json's
// exports map references.
//
// The script intentionally calls `wasm-pack` directly rather
// than going through `bun run build:*` so it produces one
// process per target instead of three nested shells — cleaner
// log output, identical semantics.

import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync, mkdirSync, rmSync } from "node:fs";

const HERE = dirname(fileURLToPath(import.meta.url));
const PKG_ROOT = resolve(HERE, "..");
const CRATE_DIR = resolve(PKG_ROOT, "..", "..", "crates", "invoicekit-wasm");

const TARGETS = [
  { name: "web", outDir: "dist/web" },
  { name: "bundler", outDir: "dist/bundler" },
  { name: "nodejs", outDir: "dist/node" },
];

async function runWasmPack(target) {
  const outDir = resolve(PKG_ROOT, target.outDir);
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  console.log(`[T-108] wasm-pack build --target ${target.name} → ${target.outDir}`);
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(
      "wasm-pack",
      [
        "build",
        CRATE_DIR,
        "--release",
        "--target",
        target.name,
        "--out-dir",
        outDir,
        "--out-name",
        "invoicekit_wasm",
        "--no-default-features",
      ],
      { stdio: "inherit" },
    );
    child.on("exit", (code) => {
      if (code === 0) {
        resolvePromise();
      } else {
        rejectPromise(new Error(`wasm-pack exited with code ${code}`));
      }
    });
    child.on("error", rejectPromise);
  });
}

async function main() {
  if (!existsSync(CRATE_DIR)) {
    throw new Error(`invoicekit-wasm crate not found at ${CRATE_DIR}`);
  }
  for (const target of TARGETS) {
    await runWasmPack(target);
  }
  console.log("[T-108] build complete; dist/web + dist/bundler + dist/node populated");
}

main().catch((err) => {
  console.error(`[T-108] build failed: ${err}`);
  process.exit(1);
});
