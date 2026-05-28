// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Preview pipeline. Loads the wasm renderer from T-026
// (`bindings/wasm-browser/pkg/invoicekit.js`) and re-renders the
// PDF whenever the designer state changes.
//
// This scaffold ships a no-op fallback when the wasm bundle is
// missing — the iframe is left blank with a stub message — so
// the editor still boots without the wasm artefact present. The
// wiring contract here matches the runbook
// (docs/operators/WYSIWYG-TEMPLATE-DESIGNER.md §Preview pipeline).

import type { DesignerState } from "./types.ts";
import { emitHeader } from "./template-emit.ts";

const WASM_BASE_URL =
  (globalThis as { INVOICEKIT_WASM_BASE_URL?: string }).INVOICEKIT_WASM_BASE_URL ??
  "/bindings/wasm-browser/pkg/invoicekit.js";

interface WasmModule {
  render_template(templateSource: string, sampleInvoiceJson: string): Uint8Array;
}

interface WasmLoader {
  default?: (input?: string) => Promise<unknown>;
  render_template?: WasmModule["render_template"];
}

export class Preview {
  private readonly iframe: HTMLIFrameElement;
  private readonly sampleInvoice: string;
  private wasm: WasmModule | undefined;
  private loadAttempted = false;

  constructor(iframe: HTMLIFrameElement, sampleInvoice: string) {
    this.iframe = iframe;
    this.sampleInvoice = sampleInvoice;
  }

  async refresh(state: DesignerState): Promise<void> {
    const templateSource = stubTemplateSource(state);
    const wasm = await this.loadWasm();
    if (wasm === undefined) {
      this.renderStubMessage();
      return;
    }
    const bytes = wasm.render_template(templateSource, this.sampleInvoice);
    // Slice into a fresh ArrayBuffer-backed Uint8Array; some wasm
    // toolchains return Uint8Array<SharedArrayBuffer> which is not
    // a valid `BlobPart`.
    const owned = new Uint8Array(bytes);
    const blob = new Blob([owned.buffer as ArrayBuffer], { type: "application/pdf" });
    this.iframe.src = URL.createObjectURL(blob);
  }

  private async loadWasm(): Promise<WasmModule | undefined> {
    if (this.wasm !== undefined) return this.wasm;
    if (this.loadAttempted) return undefined;
    this.loadAttempted = true;
    try {
      const mod = (await import(/* @vite-ignore */ WASM_BASE_URL)) as WasmLoader;
      if (typeof mod.default === "function") await mod.default();
      if (typeof mod.render_template !== "function") return undefined;
      this.wasm = { render_template: mod.render_template };
      return this.wasm;
    } catch {
      return undefined;
    }
  }

  private renderStubMessage(): void {
    const html = `<!doctype html><meta charset="utf-8"><body style="font-family:sans-serif;padding:1rem;">
      <h3>Preview unavailable</h3>
      <p>The InvoiceKit wasm bundle was not found at <code>${WASM_BASE_URL}</code>.</p>
      <p>Run <code>cargo build -p invoicekit-wasm --target wasm32-unknown-unknown</code> or
      set <code>INVOICEKIT_WASM_BASE_URL</code>.</p>
    </body>`;
    this.iframe.srcdoc = html;
  }
}

function stubTemplateSource(state: DesignerState): string {
  const header = emitHeader(state, {
    copyrightLines: ["Copyright 2026 The InvoiceKit Authors"],
  });
  return `${header}
import { defineTemplate, type CommercialDocumentTemplateData } from "@invoicekit/template-typst";

export const template = defineTemplate<CommercialDocumentTemplateData>(
  { name: "designer-output", title: "Designer output" },
  () => [],
);
`;
}
