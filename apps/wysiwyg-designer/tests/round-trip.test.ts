// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { describe, expect, test } from "bun:test";
import {
  emitHeader,
  emitIntoTemplate,
} from "../src/template-emit.ts";
import {
  DesignerParseError,
  parseHeader,
} from "../src/template-parse.ts";
import {
  DEFAULT_PAGE,
  EMPTY_STATE,
  type DesignerState,
} from "../src/types.ts";

const SAMPLE: DesignerState = {
  page: DEFAULT_PAGE,
  blocks: [
    { id: "supplier-name", x: 24, y: 48, w: 200, h: 16, bind: "supplier.name", style: "h2" },
    { id: "payable", x: 24, y: 700, w: 200, h: 20, bind: "totals.payable", style: "amount" },
  ],
};

const SPDX = "// SPDX-License-Identifier: Apache-2.0";

describe("designer header round-trip", () => {
  test("emitHeader followed by parseHeader recovers the state", () => {
    const header = emitHeader(SAMPLE);
    const parsed = parseHeader(header);
    expect(parsed.hadHeader).toBe(true);
    expect(parsed.state).toEqual(SAMPLE);
  });

  test("emitIntoTemplate injects after SPDX when no header exists", () => {
    const template = `${SPDX}\n\nexport const template = null;\n`;
    const next = emitIntoTemplate(SAMPLE, template);
    expect(next.startsWith(SPDX + "\n")).toBe(true);
    expect(next).toContain("@invoicekit-designer:state v1");
    const parsed = parseHeader(next);
    expect(parsed.state).toEqual(SAMPLE);
  });

  test("emitIntoTemplate replaces an existing header in place", () => {
    const initial = emitIntoTemplate(SAMPLE, `${SPDX}\n\nexport const template = null;\n`);
    const updated: DesignerState = {
      ...SAMPLE,
      blocks: [
        ...SAMPLE.blocks,
        { id: "issue-date", x: 24, y: 90, w: 200, h: 16, bind: "issueDate", style: "body" },
      ],
    };
    const next = emitIntoTemplate(updated, initial);
    const parsed = parseHeader(next);
    expect(parsed.state).toEqual(updated);
    expect(next.match(/@invoicekit-designer:state v1/g)?.length).toBe(1);
  });

  test("parseHeader returns empty state with hadHeader=false when no tag is present", () => {
    const parsed = parseHeader(`${SPDX}\nexport const template = null;\n`);
    expect(parsed.hadHeader).toBe(false);
    expect(parsed.state).toEqual(EMPTY_STATE);
  });

  test("copyright lines are placed between SPDX and the designer tag", () => {
    const header = emitHeader(SAMPLE, {
      copyrightLines: ["Copyright 2026 The InvoiceKit Authors"],
    });
    const lines = header.split("\n");
    const spdxIdx = lines.indexOf(SPDX);
    const copyIdx = lines.indexOf("// Copyright 2026 The InvoiceKit Authors");
    const tagIdx = lines.findIndex((l) => l.includes("@invoicekit-designer:state"));
    expect(spdxIdx).toBe(0);
    expect(copyIdx).toBe(spdxIdx + 1);
    expect(tagIdx).toBe(copyIdx + 1);
  });
});

describe("designer header validation", () => {
  test("rejects malformed JSON body", () => {
    const broken = `${SPDX}\n// @invoicekit-designer:state v1\n// not json\n`;
    expect(() => parseHeader(broken)).toThrow(DesignerParseError);
  });

  test("rejects unknown block style", () => {
    const json = JSON.stringify({
      page: DEFAULT_PAGE,
      blocks: [{ id: "x", x: 0, y: 0, w: 10, h: 10, bind: "supplier.name", style: "wat" }],
    }, null, 2);
    const body = json.split("\n").map((l) => `// ${l}`).join("\n");
    const broken = `${SPDX}\n// @invoicekit-designer:state v1\n${body}\n`;
    expect(() => parseHeader(broken)).toThrow(/style is "wat"/);
  });

  test("rejects non-finite coordinates", () => {
    const broken =
      `${SPDX}\n// @invoicekit-designer:state v1\n` +
      `// {"blocks":[{"id":"x","x":1,"y":null,"w":10,"h":10,"bind":"a","style":"body"}],"page":{"size":"A4","margins":{"top":0,"right":0,"bottom":0,"left":0}}}\n`;
    expect(() => parseHeader(broken)).toThrow(/blocks\[0\]\.y/);
  });

  test("rejects unsupported page size", () => {
    const json = JSON.stringify({
      page: { size: "A0", margins: DEFAULT_PAGE.margins },
      blocks: [],
    }, null, 2);
    const body = json.split("\n").map((l) => `// ${l}`).join("\n");
    const broken = `${SPDX}\n// @invoicekit-designer:state v1\n${body}\n`;
    expect(() => parseHeader(broken)).toThrow(/page\.size is "A0"/);
  });
});
