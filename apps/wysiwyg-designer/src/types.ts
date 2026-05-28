// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Shared types for the designer state. Mirrors the
// `@invoicekit-designer:state v1` header schema in
// docs/operators/WYSIWYG-TEMPLATE-DESIGNER.md.

export const DESIGNER_STATE_VERSION = "v1" as const;

export type BlockStyle =
  | "h1"
  | "h2"
  | "h3"
  | "body"
  | "caption"
  | "amount";

export type PageSize = "A4" | "Letter" | "Legal";

export interface PageMargins {
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
  readonly left: number;
}

export interface Page {
  readonly size: PageSize;
  readonly margins: PageMargins;
}

export interface Block {
  readonly id: string;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
  /** Dotted-path binding into the IR, e.g. `supplier.name`. */
  readonly bind: string;
  readonly style: BlockStyle;
}

export interface DesignerState {
  readonly blocks: readonly Block[];
  readonly page: Page;
}

export const DEFAULT_PAGE: Page = {
  size: "A4",
  margins: { top: 24, right: 24, bottom: 24, left: 24 },
};

export const EMPTY_STATE: DesignerState = {
  blocks: [],
  page: DEFAULT_PAGE,
};
