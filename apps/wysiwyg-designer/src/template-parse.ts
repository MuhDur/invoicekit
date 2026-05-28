// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Parse the `// @invoicekit-designer:state v1` comment header
// out of a TS template source string into a `DesignerState`.
// Round-trip stable with `template-emit.ts`.

import {
  DESIGNER_STATE_VERSION,
  type Block,
  type BlockStyle,
  type DesignerState,
  type Page,
  type PageSize,
  EMPTY_STATE,
} from "./types.ts";

const HEADER_TAG = `@invoicekit-designer:state ${DESIGNER_STATE_VERSION}`;

export class DesignerParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DesignerParseError";
  }
}

export interface ParseResult {
  readonly state: DesignerState;
  /** True if the template carried a designer header; false if a
   *  default empty state was returned because the template has
   *  no designer state yet. The UI shows an "import to designer"
   *  button in the latter case. */
  readonly hadHeader: boolean;
}

export function parseHeader(templateSource: string): ParseResult {
  const lines = templateSource.split("\n");
  const tagIdx = lines.findIndex((l) => l.includes(HEADER_TAG));
  if (tagIdx === -1) {
    return { state: EMPTY_STATE, hadHeader: false };
  }
  const bodyLines: string[] = [];
  for (let i = tagIdx + 1; i < lines.length; i += 1) {
    const line = lines[i];
    if (line === undefined) break;
    if (!line.startsWith("//")) break;
    bodyLines.push(stripCommentPrefix(line));
  }
  if (bodyLines.length === 0) {
    throw new DesignerParseError(
      "designer header tag found but no JSON body follows it",
    );
  }
  const jsonText = bodyLines.join("\n");
  let raw: unknown;
  try {
    raw = JSON.parse(jsonText);
  } catch (err) {
    throw new DesignerParseError(
      `designer header JSON is malformed: ${(err as Error).message}`,
    );
  }
  const state = coerceState(raw);
  return { state, hadHeader: true };
}

function stripCommentPrefix(line: string): string {
  // Comment lines are either `// {payload}` or `//{payload}`.
  if (line.startsWith("// ")) return line.slice(3);
  if (line.startsWith("//")) return line.slice(2);
  return line;
}

const VALID_STYLES: ReadonlySet<BlockStyle> = new Set<BlockStyle>([
  "h1",
  "h2",
  "h3",
  "body",
  "caption",
  "amount",
]);
const VALID_PAGE_SIZES: ReadonlySet<PageSize> = new Set<PageSize>([
  "A4",
  "Letter",
  "Legal",
]);

function coerceState(raw: unknown): DesignerState {
  if (typeof raw !== "object" || raw === null) {
    throw new DesignerParseError("designer header body must be a JSON object");
  }
  const obj = raw as Record<string, unknown>;
  const blocks = coerceBlocks(obj["blocks"]);
  const page = coercePage(obj["page"]);
  return { blocks, page };
}

function coerceBlocks(raw: unknown): readonly Block[] {
  if (!Array.isArray(raw)) {
    throw new DesignerParseError("designer header `blocks` must be an array");
  }
  return raw.map((entry, idx) => coerceBlock(entry, idx));
}

function coerceBlock(raw: unknown, idx: number): Block {
  if (typeof raw !== "object" || raw === null) {
    throw new DesignerParseError(`blocks[${idx}] must be an object`);
  }
  const obj = raw as Record<string, unknown>;
  const id = expectString(obj["id"], `blocks[${idx}].id`);
  const x = expectNumber(obj["x"], `blocks[${idx}].x`);
  const y = expectNumber(obj["y"], `blocks[${idx}].y`);
  const w = expectNumber(obj["w"], `blocks[${idx}].w`);
  const h = expectNumber(obj["h"], `blocks[${idx}].h`);
  const bind = expectString(obj["bind"], `blocks[${idx}].bind`);
  const styleRaw = expectString(obj["style"], `blocks[${idx}].style`);
  if (!VALID_STYLES.has(styleRaw as BlockStyle)) {
    throw new DesignerParseError(
      `blocks[${idx}].style is "${styleRaw}", must be one of: ${[...VALID_STYLES].join(", ")}`,
    );
  }
  return { id, x, y, w, h, bind, style: styleRaw as BlockStyle };
}

function coercePage(raw: unknown): Page {
  if (typeof raw !== "object" || raw === null) {
    throw new DesignerParseError("designer header `page` must be an object");
  }
  const obj = raw as Record<string, unknown>;
  const sizeRaw = expectString(obj["size"], "page.size");
  if (!VALID_PAGE_SIZES.has(sizeRaw as PageSize)) {
    throw new DesignerParseError(
      `page.size is "${sizeRaw}", must be one of: ${[...VALID_PAGE_SIZES].join(", ")}`,
    );
  }
  const marginsRaw = obj["margins"];
  if (typeof marginsRaw !== "object" || marginsRaw === null) {
    throw new DesignerParseError("page.margins must be an object");
  }
  const m = marginsRaw as Record<string, unknown>;
  return {
    size: sizeRaw as PageSize,
    margins: {
      top: expectNumber(m["top"], "page.margins.top"),
      right: expectNumber(m["right"], "page.margins.right"),
      bottom: expectNumber(m["bottom"], "page.margins.bottom"),
      left: expectNumber(m["left"], "page.margins.left"),
    },
  };
}

function expectString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new DesignerParseError(`${field} must be a string`);
  }
  return value;
}

function expectNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new DesignerParseError(`${field} must be a finite number`);
  }
  return value;
}
