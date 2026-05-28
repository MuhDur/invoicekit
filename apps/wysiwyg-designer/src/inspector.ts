// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Minimal inspector. Real version reads field paths from the
// generated `invoicekit_ir_v1.d.ts` via T-012 and renders a tree
// picker. The scaffold ships a flat dropdown over a hard-coded
// allowlist so the round-trip story still works.

import type { Block, BlockStyle, DesignerState } from "./types.ts";

const FIELD_PATHS: readonly string[] = [
  "supplier.name",
  "supplier.address",
  "supplier.taxId",
  "customer.name",
  "customer.address",
  "customer.taxId",
  "documentNumber",
  "issueDate",
  "dueDate",
  "totals.payable",
  "totals.taxExclusive",
  "totals.taxInclusive",
];

const STYLES: readonly BlockStyle[] = ["h1", "h2", "h3", "body", "caption", "amount"];

export interface InspectorEvents {
  readonly onBindChange: (blockId: string, bind: string) => void;
  readonly onStyleChange: (blockId: string, style: BlockStyle) => void;
}

export class Inspector {
  private readonly host: HTMLElement;
  private readonly events: InspectorEvents;
  private state: DesignerState;
  private selectedId: string | undefined;

  constructor(host: HTMLElement, initial: DesignerState, events: InspectorEvents) {
    this.host = host;
    this.state = initial;
    this.events = events;
    this.render();
  }

  setState(state: DesignerState): void {
    this.state = state;
    this.render();
  }

  select(blockId: string | undefined): void {
    this.selectedId = blockId;
    this.render();
  }

  private render(): void {
    this.host.innerHTML = "";
    const block = this.findSelected();
    if (block === undefined) {
      const empty = document.createElement("p");
      empty.textContent = "Select a block to edit its binding and style.";
      this.host.appendChild(empty);
      return;
    }
    this.host.appendChild(this.renderRow("Bind to field", this.renderBindSelect(block)));
    this.host.appendChild(this.renderRow("Style", this.renderStyleSelect(block)));
  }

  private findSelected(): Block | undefined {
    if (this.selectedId === undefined) return undefined;
    return this.state.blocks.find((b) => b.id === this.selectedId);
  }

  private renderRow(label: string, control: HTMLElement): HTMLElement {
    const row = document.createElement("label");
    row.style.display = "block";
    row.style.marginBottom = "0.75rem";
    const labelEl = document.createElement("span");
    labelEl.style.display = "block";
    labelEl.style.fontWeight = "600";
    labelEl.textContent = label;
    row.appendChild(labelEl);
    row.appendChild(control);
    return row;
  }

  private renderBindSelect(block: Block): HTMLSelectElement {
    const select = document.createElement("select");
    for (const path of FIELD_PATHS) {
      const opt = document.createElement("option");
      opt.value = path;
      opt.textContent = path;
      if (path === block.bind) opt.selected = true;
      select.appendChild(opt);
    }
    select.addEventListener("change", () => {
      this.events.onBindChange(block.id, select.value);
    });
    return select;
  }

  private renderStyleSelect(block: Block): HTMLSelectElement {
    const select = document.createElement("select");
    for (const style of STYLES) {
      const opt = document.createElement("option");
      opt.value = style;
      opt.textContent = style;
      if (style === block.style) opt.selected = true;
      select.appendChild(opt);
    }
    select.addEventListener("change", () => {
      this.events.onStyleChange(block.id, select.value as BlockStyle);
    });
    return select;
  }
}
