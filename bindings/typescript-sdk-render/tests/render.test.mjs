// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  createRenderer,
  createRendererFromWasmModule,
  RenderEngineError,
  RenderInputError,
} from "../src/index.ts";

function fakeBridge(impl) {
  return { renderHtml: impl };
}

const minimalDoc = { schema_version: "1.0", id: "doc-001" };

test("renders to HTML via injected bridge", () => {
  const calls = [];
  const renderer = createRenderer(
    fakeBridge((json, opts) => {
      calls.push({ json, opts });
      return "<html><body>ok</body></html>";
    }),
  );
  const out = renderer.renderHtml(minimalDoc);
  assert.equal(out, "<html><body>ok</body></html>");
  assert.equal(calls.length, 1);
  assert.equal(JSON.parse(calls[0].json).schema_version, "1.0");
  assert.equal(calls[0].opts.palette, "default");
  assert.equal(calls[0].opts.locale, "en-GB");
  assert.equal(calls[0].opts.strict, true);
});

test("forwards explicit options", () => {
  let captured;
  const renderer = createRenderer(
    fakeBridge((_json, opts) => {
      captured = opts;
      return "";
    }),
  );
  renderer.renderHtml(minimalDoc, { palette: "muted", locale: "de-DE", strict: false });
  assert.equal(captured.palette, "muted");
  assert.equal(captured.locale, "de-DE");
  assert.equal(captured.strict, false);
});

test("rejects non-object document with RenderInputError", () => {
  const renderer = createRenderer(fakeBridge(() => ""));
  assert.throws(() => renderer.renderHtml(null), RenderInputError);
  assert.throws(() => renderer.renderHtml([]), RenderInputError);
  assert.throws(() => renderer.renderHtml("hi"), RenderInputError);
});

test("rejects document missing schema_version", () => {
  const renderer = createRenderer(fakeBridge(() => ""));
  assert.throws(
    () => renderer.renderHtml({ id: "no-version" }),
    RenderInputError,
  );
});

test("wraps bridge errors as RenderEngineError", () => {
  const renderer = createRenderer(
    fakeBridge(() => {
      throw new Error("wasm panic: invalid line");
    }),
  );
  assert.throws(
    () => renderer.renderHtml(minimalDoc),
    (err) => err instanceof RenderEngineError && /wasm panic/.test(err.message),
  );
});

test("createRendererFromWasmModule adapts wasm exports", () => {
  let received;
  const fakeWasm = {
    render_html(documentJson, optionsJson) {
      received = { documentJson, optionsJson };
      return "<html>via wasm</html>";
    },
  };
  const renderer = createRendererFromWasmModule(fakeWasm);
  const out = renderer.renderHtml(minimalDoc, { locale: "fr-FR" });
  assert.equal(out, "<html>via wasm</html>");
  assert.equal(JSON.parse(received.documentJson).schema_version, "1.0");
  assert.equal(JSON.parse(received.optionsJson).locale, "fr-FR");
});
