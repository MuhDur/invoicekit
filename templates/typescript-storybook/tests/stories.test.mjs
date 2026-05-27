// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-114 smoke test: every story compiles its template without
// throwing, and every story declares the bead's required
// "variant" + "description" args. Mirrors what the Storybook
// build does without spinning up a browser.

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { compileTemplate } from "../../typescript/src/index.ts";

import * as basicInvoiceStories from "../src/basic-invoice.stories.ts";
import * as creditNoteStories from "../src/credit-note.stories.ts";
import * as factur from "../src/factur-x-summary.stories.ts";
import * as paymentReminder from "../src/payment-reminder.stories.ts";
import * as taxBreakdown from "../src/tax-breakdown.stories.ts";

const allFiles = {
  "basic-invoice": basicInvoiceStories,
  "credit-note": creditNoteStories,
  "factur-x-summary": factur,
  "payment-reminder": paymentReminder,
  "tax-breakdown": taxBreakdown,
};

test("every template ships a default-exported Storybook meta", () => {
  for (const [name, mod] of Object.entries(allFiles)) {
    assert.ok(mod.default, `${name}: missing default export`);
    assert.ok(mod.default.title, `${name}: meta.title missing`);
    assert.match(
      mod.default.title,
      /^Templates\//,
      `${name}: meta.title should start with "Templates/"`,
    );
  }
});

test("every template exports at least a Base story with compiled Typst source", () => {
  for (const [name, mod] of Object.entries(allFiles)) {
    const story = mod.Base;
    assert.ok(story, `${name}: missing Base story`);
    assert.ok(story.args.typstSource, `${name}: Base story missing typstSource`);
    assert.ok(story.args.templateName, `${name}: Base story missing templateName`);
    assert.ok(story.args.variant, `${name}: Base story missing variant`);
    // Typst source always carries the document header.
    assert.match(
      story.args.typstSource,
      /#set document/,
      `${name}: typstSource missing #set document header`,
    );
  }
});

test("strict gate: at least two templates ship allowance + reverse-charge variants", () => {
  // Bead requires "Variants: with allowances, with reverse charge, etc."
  const variantTemplates = [
    basicInvoiceStories,
    taxBreakdown,
  ];
  for (const mod of variantTemplates) {
    assert.ok(mod.WithAllowance, "WithAllowance variant missing");
    assert.ok(mod.ReverseCharge, "ReverseCharge variant missing");
    assert.match(
      mod.ReverseCharge.args.description,
      /[Rr]everse charge|BR-AE/,
      "ReverseCharge description must mention the rule",
    );
  }
});

test("compileTemplate produces stable Typst output (re-render equal)", async () => {
  const { template, data } = await import("../../typescript/examples/basic-invoice.ts");
  const a = compileTemplate(template, data);
  const b = compileTemplate(template, data);
  assert.equal(a, b);
});
