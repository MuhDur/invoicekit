// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, expect, test } from "bun:test";

import { compileTemplate, grid, table, TemplateCompileError } from "../src/index.ts";
import { examples } from "../examples/index.ts";

const goldenRoot = join(import.meta.dir, "..", "golden");

describe("TypeScript template compiler", () => {
  for (const [name, example] of Object.entries(examples)) {
    test(`${name} compiles to a deterministic Typst golden`, () => {
      const first = compileTemplate(example.template, example.data);
      const second = compileTemplate(example.template, example.data);

      expect(first).toBe(second);
      assertGolden(`${name}.typ`, first);
    });
  }

  test("Typst escaping is deterministic and keeps user text inert", () => {
    const escaped = compileTemplate(examples["basic-invoice"].template, {
      ...examples["basic-invoice"].data,
      documentNumber: "INV-[#bad]*_$",
    });

    expect(escaped).toContain("INV-\\[\\#bad\\]\\*\\_\\$");
  });

  test("invalid grid row shape returns a typed error", () => {
    expect(() => grid(2, [[{ kind: "text", value: "only one cell" }]])).toThrow(
      TemplateCompileError,
    );
  });

  test("invalid table row shape returns a typed error", () => {
    expect(() => table(["A", "B"], [["only one cell"]])).toThrow(TemplateCompileError);
  });
});

function assertGolden(name: string, actual: string): void {
  const path = join(goldenRoot, name);
  if (process.env.UPDATE_GOLDENS === "1") {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, actual, "utf8");
  }

  const expected = readFileSync(path, "utf8");
  expect(actual).toBe(expected);
}
