// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// Serialise a `DesignerState` to the canonical
// `// @invoicekit-designer:state v1` comment-header block that
// every designer-aware TS template carries right after its SPDX
// header. Round-trip stable with `template-parse.ts`.

import { DESIGNER_STATE_VERSION, type DesignerState } from "./types.ts";

const HEADER_TAG = `@invoicekit-designer:state ${DESIGNER_STATE_VERSION}`;
const SPDX_LINE = "// SPDX-License-Identifier: Apache-2.0";

export interface EmitOptions {
  /** Copyright line(s) injected between the SPDX line and the
   *  designer header. One string per line, no leading `// `. */
  readonly copyrightLines?: readonly string[];
}

export function emitHeader(state: DesignerState, opts: EmitOptions = {}): string {
  const json = JSON.stringify(state, null, 2);
  const headerBody = json
    .split("\n")
    .map((line) => `// ${line}`)
    .join("\n");
  const copyright = (opts.copyrightLines ?? [])
    .map((line) => `// ${line}`)
    .join("\n");
  const segments = [SPDX_LINE];
  if (copyright.length > 0) segments.push(copyright);
  segments.push(`// ${HEADER_TAG}`, headerBody);
  return segments.join("\n") + "\n";
}

/** Replace any existing designer header in `templateSource` with
 *  one freshly emitted from `state`. If no header is present, the
 *  new header is injected immediately after the SPDX line.
 *
 *  Roundtrip rule: `emitIntoTemplate(parseHeader(source), source)`
 *  must equal `source` modulo trailing-whitespace normalisation.
 */
export function emitIntoTemplate(
  state: DesignerState,
  templateSource: string,
  opts: EmitOptions = {},
): string {
  const lines = templateSource.split("\n");
  const headerStart = lines.findIndex((l) => l.includes(HEADER_TAG));
  if (headerStart === -1) {
    const spdxIdx = lines.findIndex((l) => l.startsWith(SPDX_LINE));
    const insertAt = spdxIdx === -1 ? 0 : spdxIdx + 1;
    const inserted = emitHeaderBlockOnly(state, opts);
    const next = [...lines.slice(0, insertAt), inserted, ...lines.slice(insertAt)];
    return next.join("\n");
  }
  const headerEnd = findHeaderEnd(lines, headerStart);
  const replaced = emitHeaderBlockOnly(state, opts);
  return [...lines.slice(0, headerStart), replaced, ...lines.slice(headerEnd + 1)].join("\n");
}

function emitHeaderBlockOnly(state: DesignerState, opts: EmitOptions): string {
  const json = JSON.stringify(state, null, 2);
  const headerBody = json
    .split("\n")
    .map((line) => `// ${line}`)
    .join("\n");
  const copyright = (opts.copyrightLines ?? [])
    .map((line) => `// ${line}`)
    .join("\n");
  const segments: string[] = [];
  if (copyright.length > 0) segments.push(copyright);
  segments.push(`// ${HEADER_TAG}`, headerBody);
  return segments.join("\n");
}

function findHeaderEnd(lines: readonly string[], headerStart: number): number {
  // Header body is the contiguous block of `// `-prefixed lines
  // immediately following the tag line. Stop at the first line
  // that is not a comment.
  let i = headerStart + 1;
  while (i < lines.length) {
    const line = lines[i];
    if (line === undefined) break;
    if (!line.startsWith("//")) break;
    i += 1;
  }
  return i - 1;
}
