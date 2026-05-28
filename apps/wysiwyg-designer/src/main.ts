// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Wire the canvas + inspector + preview together over a single
// mutable `DesignerState`. The scaffold runs in any browser
// environment that has `document` available; tests don't import
// this module.

import { Canvas } from "./canvas.ts";
import { Inspector } from "./inspector.ts";
import { Preview } from "./preview.ts";
import { EMPTY_STATE, type BlockStyle, type DesignerState } from "./types.ts";

declare const document: Document | undefined;

const SAMPLE_INVOICE_JSON = `{
  "supplier": {"name": "Acme GmbH", "address": "Berlin"},
  "customer": {"name": "Globex SRL", "address": "Milano"},
  "documentNumber": "INV-2026-001",
  "issueDate": "2026-05-27",
  "totals": {"payable": "1234.56"}
}`;

function bootstrap(): void {
  if (typeof document === "undefined") return;
  const canvasHost = document.getElementById("canvas-host");
  const inspectorHost = document.getElementById("inspector-host");
  const previewHost = document.getElementById("preview-host") as HTMLIFrameElement | null;
  if (canvasHost === null || inspectorHost === null || previewHost === null) {
    throw new Error("designer host elements missing from index.html");
  }
  let state: DesignerState = EMPTY_STATE;
  const preview = new Preview(previewHost, SAMPLE_INVOICE_JSON);
  const inspector = new Inspector(inspectorHost, state, {
    onBindChange: (id, bind) => {
      state = patchBlock(state, id, (b) => ({ ...b, bind }));
      sync();
    },
    onStyleChange: (id, style: BlockStyle) => {
      state = patchBlock(state, id, (b) => ({ ...b, style }));
      sync();
    },
  });
  const canvas = new Canvas(canvasHost, state, {
    onSelect: (id) => inspector.select(id),
    onMove: (id, x, y) => {
      state = patchBlock(state, id, (b) => ({ ...b, x, y }));
      sync();
    },
  });
  void preview.refresh(state);

  function sync(): void {
    canvas.setState(state);
    inspector.setState(state);
    void preview.refresh(state);
  }
}

function patchBlock(
  state: DesignerState,
  id: string,
  patch: (b: DesignerState["blocks"][number]) => DesignerState["blocks"][number],
): DesignerState {
  return {
    ...state,
    blocks: state.blocks.map((b) => (b.id === id ? patch(b) : b)),
  };
}

bootstrap();
