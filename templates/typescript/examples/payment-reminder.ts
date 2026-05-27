// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import {
  defineTemplate,
  heading,
  money,
  paragraph,
  strong,
  text,
  type CommercialDocumentTemplateData,
} from "../src/index.ts";
import { baseInvoice } from "./shared.ts";

export const data = baseInvoice;

export const template = defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "payment-reminder",
    title: "Payment reminder",
  },
  (invoice) => [
    heading(1, text("Payment reminder")),
    paragraph(text("Invoice "), strong(invoice.documentNumber), text(" remains open.")),
    paragraph(text("Customer: "), strong(invoice.customer.legalName)),
    paragraph(text("Due date: "), strong(invoice.dueDate ?? "due on receipt")),
    paragraph(strong("Outstanding amount"), text(` ${money(invoice.totals.payable)}`)),
    paragraph(text(invoice.paymentTerms ?? "Please settle the invoice promptly.")),
  ],
);
