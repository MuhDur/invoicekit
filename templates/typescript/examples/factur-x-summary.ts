// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import {
  defineTemplate,
  grid,
  heading,
  money,
  paragraph,
  strong,
  table,
  text,
  type CommercialDocumentTemplateData,
} from "../src/index.ts";
import { baseInvoice } from "./shared.ts";

export const data = baseInvoice;

export const template = defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "factur-x-summary",
    title: "Factur-X summary",
  },
  (invoice) => [
    heading(1, text("Factur-X summary")),
    grid(2, [
      [strong("Profile"), text("EN 16931")],
      [strong("Document"), text(invoice.documentNumber)],
      [strong("Supplier VAT"), text(invoice.supplier.vatId)],
      [strong("Customer VAT"), text(invoice.customer.vatId)],
    ]),
    table(
      ["VAT category", "Rate", "Taxable", "Tax"],
      invoice.taxSubtotals.map((tax) => [
        tax.category,
        `${tax.rate}%`,
        money(tax.taxableAmount),
        money(tax.taxAmount),
      ]),
    ),
    paragraph(text(invoice.note ?? "No note")),
  ],
);
