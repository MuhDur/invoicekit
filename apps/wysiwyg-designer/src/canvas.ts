// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Minimal absolute-positioned DOM canvas. The real designer will
// add SVG rulers, snap-grid, multi-select, etc.; the scaffold
// here ships just enough to verify the round-trip in a browser:
//   - renders one page rectangle per `Page`
//   - one absolutely-positioned `.designer-block` per `Block`
//   - tracks selection + drag updates and notifies via callback

import type { Block, DesignerState } from "./types.ts";

const PAGE_SIZES_MM: Record<DesignerState["page"]["size"], { w: number; h: number }> = {
  A4: { w: 210, h: 297 },
  Letter: { w: 216, h: 279 },
  Legal: { w: 216, h: 356 },
};

const MM_TO_PX = 3.78; // 96dpi approximation; cosmetic only.

export interface CanvasEvents {
  readonly onSelect: (blockId: string | undefined) => void;
  readonly onMove: (blockId: string, x: number, y: number) => void;
}

export class Canvas {
  private readonly host: HTMLElement;
  private readonly events: CanvasEvents;
  private state: DesignerState;
  private selectedId: string | undefined;

  constructor(host: HTMLElement, initial: DesignerState, events: CanvasEvents) {
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
    this.events.onSelect(blockId);
  }

  private render(): void {
    this.host.innerHTML = "";
    const pageEl = document.createElement("div");
    pageEl.className = "designer-page";
    const { w, h } = PAGE_SIZES_MM[this.state.page.size];
    pageEl.style.width = `${w * MM_TO_PX}px`;
    pageEl.style.height = `${h * MM_TO_PX}px`;
    for (const block of this.state.blocks) {
      pageEl.appendChild(this.renderBlock(block));
    }
    this.host.appendChild(pageEl);
  }

  private renderBlock(block: Block): HTMLElement {
    const el = document.createElement("div");
    el.className = "designer-block";
    if (block.id === this.selectedId) el.classList.add("selected");
    el.style.left = `${block.x}px`;
    el.style.top = `${block.y}px`;
    el.style.width = `${block.w}px`;
    el.style.height = `${block.h}px`;
    el.dataset["blockId"] = block.id;
    el.textContent = `${block.bind} (${block.style})`;
    el.addEventListener("mousedown", (e) => this.beginDrag(e, block));
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      this.select(block.id);
    });
    return el;
  }

  private beginDrag(start: MouseEvent, block: Block): void {
    start.preventDefault();
    const startX = start.clientX - block.x;
    const startY = start.clientY - block.y;
    const onMove = (e: MouseEvent) => {
      this.events.onMove(block.id, e.clientX - startX, e.clientY - startY);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }
}
