// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import {
  defineTemplate,
  grid,
  heading,
  money,
  paragraph,
  rule,
  strong,
  table,
  text,
  type CommercialDocumentTemplateData,
} from "../src/index.ts";
import { baseInvoice } from "./shared.ts";

export const data = baseInvoice;

export const template = defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "basic-invoice",
    title: "Basic invoice",
  },
  (invoice) => [
    heading(1, text("Invoice "), strong(invoice.documentNumber)),
    grid(2, [
      [strong("Supplier"), text(invoice.supplier.legalName)],
      [strong("Customer"), text(invoice.customer.legalName)],
      [strong("Issue date"), text(invoice.issueDate)],
      [strong("Due date"), text(invoice.dueDate ?? "due on receipt")],
    ]),
    rule(),
    table(
      ["Description", "Qty", "Unit price", "Line total"],
      invoice.lines.map((line) => [
        line.description,
        line.quantity,
        money(line.unitPrice),
        money(line.lineTotal),
      ]),
    ),
    paragraph(strong("Payable"), text(` ${money(invoice.totals.payable)}`)),
  ],
);
