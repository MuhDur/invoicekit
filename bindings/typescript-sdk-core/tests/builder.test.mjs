// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { buildCommercialDocument, InvalidInvoiceError } from "../src/index.ts";

function sampleInput(overrides) {
  return {
    id: "doc-001",
    documentNumber: "F-2026-001",
    documentType: "invoice",
    issueDate: "2026-05-27",
    currency: "EUR",
    supplier: {
      name: "Acme Corp",
      taxId: { scheme: "vat", value: "ES12345678" },
      address: {
        lines: ["Calle Mayor 1"],
        city: "Madrid",
        postalCode: "28013",
        country: "ES",
      },
    },
    customer: {
      name: "Widget Buyer",
      address: {
        lines: ["12 rue de la Paix"],
        city: "Paris",
        postalCode: "75001",
        country: "FR",
      },
    },
    lines: [
      {
        description: "Widget",
        quantity: "2",
        unitPrice: "100.00",
      },
    ],
    tenantId: "tenant-x",
    traceId: "trace-001",
    ...overrides,
  };
}

test("builds a minimal valid CommercialDocument", () => {
  const doc = buildCommercialDocument(sampleInput());
  assert.equal(doc.schema_version, "1.0");
  assert.equal(doc.id, "doc-001");
  assert.equal(doc.currency, "EUR");
  assert.equal(doc.lines.length, 1);
  assert.equal(doc.lines[0].id, "L1");
  assert.equal(doc.lines[0].line_extension_amount, "200.00");
  assert.equal(doc.monetary_total.payable_amount, "200.00");
});

test("auto-generates line IDs when omitted", () => {
  const doc = buildCommercialDocument(
    sampleInput({
      lines: [
        { description: "First", quantity: "1", unitPrice: "10.00" },
        { description: "Second", quantity: "1", unitPrice: "10.00" },
      ],
    }),
  );
  assert.equal(doc.lines[0].id, "L1");
  assert.equal(doc.lines[1].id, "L2");
});

test("preserves explicit line IDs when provided", () => {
  const doc = buildCommercialDocument(
    sampleInput({
      lines: [
        { id: "ITEM-A", description: "First", quantity: "1", unitPrice: "10.00" },
      ],
    }),
  );
  assert.equal(doc.lines[0].id, "ITEM-A");
});

test("sums line subtotals into monetary_total.line_extension_amount", () => {
  const doc = buildCommercialDocument(
    sampleInput({
      lines: [
        { description: "A", quantity: "2", unitPrice: "100.00" },
        { description: "B", quantity: "1", unitPrice: "50.00" },
      ],
    }),
  );
  assert.equal(doc.monetary_total.line_extension_amount, "250.00");
});

test("rejects blank id with InvalidInvoiceError", () => {
  assert.throws(
    () => buildCommercialDocument(sampleInput({ id: "" })),
    (err) => err instanceof InvalidInvoiceError && err.field === "id",
  );
});

test("rejects non-ISO currency", () => {
  assert.throws(
    () => buildCommercialDocument(sampleInput({ currency: "EURO" })),
    (err) => err instanceof InvalidInvoiceError && err.field === "currency",
  );
});

test("rejects empty lines array", () => {
  assert.throws(
    () => buildCommercialDocument(sampleInput({ lines: [] })),
    (err) => err instanceof InvalidInvoiceError && err.field === "lines",
  );
});

test("rejects non-decimal quantity", () => {
  assert.throws(
    () =>
      buildCommercialDocument(
        sampleInput({
          lines: [{ description: "x", quantity: "abc", unitPrice: "1.00" }],
        }),
      ),
    (err) =>
      err instanceof InvalidInvoiceError && err.field.startsWith("lines[0].quantity"),
  );
});

test("rejects non-ISO country code in address", () => {
  assert.throws(
    () =>
      buildCommercialDocument(
        sampleInput({
          supplier: {
            ...sampleInput().supplier,
            address: { ...sampleInput().supplier.address, country: "ESP" },
          },
        }),
      ),
    (err) =>
      err instanceof InvalidInvoiceError &&
      err.field === "supplier.address.country",
  );
});

test("preserves optional payment_instructions and tax_summary as empty arrays", () => {
  const doc = buildCommercialDocument(sampleInput());
  assert.deepEqual(doc.payment_instructions, []);
  assert.deepEqual(doc.tax_summary, []);
  assert.deepEqual(doc.extensions, []);
});

test("includes meta.tenant_id and meta.trace_id", () => {
  const doc = buildCommercialDocument(sampleInput());
  assert.equal(doc.meta.tenant_id, "tenant-x");
  assert.equal(doc.meta.trace_id, "trace-001");
});
