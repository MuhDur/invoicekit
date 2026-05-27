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
import { baseInvoice } from "./shared.ts";

export const data = baseInvoice;

export const template = defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "tax-breakdown",
    title: "Tax breakdown",
  },
  (invoice) => [
    heading(1, text("Tax breakdown for "), strong(invoice.documentNumber)),
    table(
      ["Category", "Rate", "Taxable amount", "Tax amount"],
      invoice.taxSubtotals.map((tax) => [
        tax.category,
        `${tax.rate}%`,
        money(tax.taxableAmount),
        money(tax.taxAmount),
      ]),
    ),
    paragraph(strong("Net"), text(` ${money(invoice.totals.net)}`)),
    paragraph(strong("Tax"), text(` ${money(invoice.totals.tax)}`)),
    paragraph(strong("Payable"), text(` ${money(invoice.totals.payable)}`)),
  ],
);
