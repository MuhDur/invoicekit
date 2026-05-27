# @invoicekit/wasm

The InvoiceKit engine, compiled to WebAssembly. Drop-in for browsers, Node, Deno, Bun, and Cloudflare Workers.

## Install

```sh
npm install @invoicekit/wasm
# or
bun add @invoicekit/wasm
```

## Use

```ts
import { processEngineAbiJson, compiledCountryBundles, beadId } from "@invoicekit/wasm";

const response = processEngineAbiJson(
  new TextEncoder().encode(
    JSON.stringify({ abi_version: 1, operation: "unknown", payload: {} }),
  ),
);
console.log(new TextDecoder().decode(response));

console.log("compiled country bundles:", JSON.parse(compiledCountryBundles()));
console.log("bead id:", beadId());
```

## What's in the box

Three wasm-pack target builds, all under 5 MB with default features:

- `dist/web/` — `--target web`. Use this for browsers, Cloudflare Workers, Deno.
- `dist/bundler/` — `--target bundler`. Use this with Vite / Rollup / Webpack / Bun.
- `dist/node/` — `--target nodejs`. Use this in Node.js scripts and CommonJS-shaped environments.

The `exports` map in `package.json` resolves the right target per runtime automatically; you don't normally need to pick one explicitly.

## Building from source

```sh
cd bindings/wasm-browser-npm
bun install
bun run build       # runs wasm-pack for all three targets
bun run check-size  # asserts every bundle is under 5 MB
bun test            # smoke test against the node bundle
```

`bun run build` is what CI runs in `.github/workflows/wasm-browser-bundle.yml`. The build artifact lives under `dist/` (gitignored); the published npm tarball carries it.

## Feature flags

The underlying [`invoicekit-wasm`](../../crates/invoicekit-wasm) crate carries country and format feature flags. The default-features build is the leanest (engine ABI surface only). To ship a bigger bundle with a specific country mix, fork the build script and pass `--features "country-de,country-fr,format-peppol"` to `wasm-pack build`.

## Publishing

`publishConfig.access = public`; the actual `npm publish` is gated on an `NPM_TOKEN` secret that doesn't exist today. Until that lands the package is consumed via local path resolution from the InvoiceKit monorepo or via a private tarball.
