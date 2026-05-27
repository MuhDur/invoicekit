// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1400 smoke test: build every fixture and assert the
// CommercialDocument shape the @invoicekit/core builder emits.
// Runs the builder directly (no Next.js process required) so the
// CI gate stays fast.

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { buildCommercialDocument } from "@invoicekit/core";

import { fixtures } from "../fixtures/index.ts";

test("basic fixture builds a single-line German XRechnung", () => {
  const doc = buildCommercialDocument(fixtures.basic);
  assert.equal(doc.currency, "EUR");
  assert.equal(doc.supplier.address.country, "DE");
  assert.equal(doc.customer.address.country, "DE");
  assert.equal(doc.lines.length, 1);
  assert.equal(doc.lines[0].line_extension_amount, "1000.00");
  assert.equal(doc.monetary_total.payable_amount, "1000.00");
});

test("with-allowance fixture sums a 10% discount via signed-quantity line", () => {
  const doc = buildCommercialDocument(fixtures["with-allowance"]);
  assert.equal(doc.lines.length, 2);
  // 10 × 150 + (-1) × 150 = 1500 - 150 = 1350
  assert.equal(doc.monetary_total.line_extension_amount, "1350.00");
});

test("reverse-charge fixture carries AE tax category on the line", () => {
  const doc = buildCommercialDocument(fixtures["reverse-charge"]);
  assert.equal(doc.customer.address.country, "AT");
  assert.equal(doc.lines[0].tax_category, "AE");
  assert.equal(doc.monetary_total.payable_amount, "5000.00");
});

test("every fixture issues a non-empty document_number, id, and meta", () => {
  for (const [name, input] of Object.entries(fixtures)) {
    const doc = buildCommercialDocument(input);
    assert.ok(doc.id, `${name}: id must be non-empty`);
    assert.ok(doc.document_number, `${name}: document_number must be non-empty`);
    assert.ok(doc.meta?.tenant_id, `${name}: meta.tenant_id must be non-empty`);
    assert.ok(doc.meta?.trace_id, `${name}: meta.trace_id must be non-empty`);
  }
});
