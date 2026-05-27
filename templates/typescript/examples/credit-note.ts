// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import {
  defineTemplate,
  heading,
  money,
  paragraph,
  strong,
  table,
  text,
  type CommercialDocumentTemplateData,
} from "../src/index.ts";
import { baseInvoice, eur } from "./shared.ts";

const { dueDate: _dueDate, ...creditBase } = baseInvoice;

export const data: CommercialDocumentTemplateData = {
  ...creditBase,
  documentKind: "credit-note",
  documentNumber: "CN-2026-0001",
  lines: [
    {
      description: "Credit for cancelled verification run",
      quantity: "1",
      unitPrice: eur("-50.00"),
      lineTotal: eur("-50.00"),
    },
  ],
  taxSubtotals: [
    {
      category: "S",
      rate: "19",
      taxableAmount: eur("-50.00"),
      taxAmount: eur("-9.50"),
    },
  ],
  totals: {
    net: eur("-50.00"),
    tax: eur("-9.50"),
    payable: eur("-59.50"),
  },
};

export const template = defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "credit-note",
    title: "Credit note",
  },
  (invoice) => [
    heading(1, text("Credit note "), strong(invoice.documentNumber)),
    paragraph(text("Original customer: "), strong(invoice.customer.legalName)),
    table(
      ["Reason", "Amount"],
      invoice.lines.map((line) => [line.description, money(line.lineTotal)]),
    ),
    paragraph(strong("Credit total"), text(` ${money(invoice.totals.payable)}`)),
  ],
);
