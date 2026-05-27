// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/**
 * @invoicekit/render — thin TypeScript surface for rendering a
 * CommercialDocument JSON value to HTML (and, when wired, PDF) via
 * the InvoiceKit wasm engine (T-108 @invoicekit/wasm).
 *
 * Why thin: every byte of HTML/PDF output is produced by the Rust
 * engine in render-html / render-pdf so that browser, server, and
 * CLI surfaces produce byte-identical output. This package's job is
 * to:
 *
 *  - Accept an injected wasm bridge (so tests / Node / Deno / Bun
 *    can pick the binding flavor that fits their runtime).
 *  - Validate the JSON shape just enough to fail fast with a clear
 *    error before crossing the wasm boundary.
 *  - Surface stable error types so consumers can catch by class
 *    instead of string-matching.
 *
 * The bridge protocol is intentionally small: one method,
 * `renderHtml(documentJson, options)`, that returns a string. The
 * concrete wasm package (`@invoicekit/wasm`) exposes a matching
 * `render_html` export; the consumer calls
 * `createRendererFromWasmModule(wasmModule)` to wrap it.
 */

export const SDK_RENDER_BEAD_ID = "invoices-t-103-typescript-sdk-bhkn";

/** Options forwarded to the engine renderer. */
export interface RenderHtmlOptions {
  /** Palette name registered in render-html (defaults to "default"). */
  palette?: string;
  /** Locale tag (e.g. "en-GB", "de-DE"). Defaults to "en-GB". */
  locale?: string;
  /** Force WCAG AA strict mode. Defaults to true. */
  strict?: boolean;
}

/** Minimal bridge contract a wasm binding must satisfy. */
export interface WasmRendererBridge {
  renderHtml(documentJson: string, options: RenderHtmlOptions): string;
}

/** Errors the renderer surfaces. */
export class RenderInputError extends Error {
  constructor(message: string) {
    super(`render input invalid: ${message}`);
    this.name = "RenderInputError";
  }
}

export class RenderEngineError extends Error {
  constructor(cause: unknown) {
    super(`render engine failed: ${describe(cause)}`);
    this.name = "RenderEngineError";
  }
}

/** Create a renderer bound to an injected bridge. */
export function createRenderer(bridge: WasmRendererBridge) {
  return {
    /** Render the document to HTML and return it as a string. */
    renderHtml(
      document: Record<string, unknown>,
      options: RenderHtmlOptions = {},
    ): string {
      requirePlainObject(document);
      const json = JSON.stringify(document);
      try {
        return bridge.renderHtml(json, normalizeOptions(options));
      } catch (cause) {
        throw new RenderEngineError(cause);
      }
    },
  };
}

/**
 * Wrap a `@invoicekit/wasm` module into the bridge shape. The wasm
 * package's `render_html(documentJson: string, optionsJson: string)`
 * export is the only contact point — this is a one-line adapter so
 * tests can inject a fake bridge.
 */
export function createRendererFromWasmModule(wasmModule: {
  render_html(documentJson: string, optionsJson: string): string;
}): ReturnType<typeof createRenderer> {
  const bridge: WasmRendererBridge = {
    renderHtml(documentJson, options) {
      return wasmModule.render_html(documentJson, JSON.stringify(options));
    },
  };
  return createRenderer(bridge);
}

function normalizeOptions(options: RenderHtmlOptions): RenderHtmlOptions {
  return {
    palette: options.palette ?? "default",
    locale: options.locale ?? "en-GB",
    strict: options.strict ?? true,
  };
}

function requirePlainObject(value: unknown): void {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new RenderInputError("document must be a plain object");
  }
  if (!("schema_version" in value)) {
    throw new RenderInputError("document missing schema_version");
  }
}

function describe(cause: unknown): string {
  if (cause instanceof Error) {
    return cause.message;
  }
  try {
    return JSON.stringify(cause);
  } catch {
    return String(cause);
  }
}
