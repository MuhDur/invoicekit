// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { defineTemplate, paragraph, text, type CommercialDocumentTemplateData } from "../src/index.ts";

defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "type-safety-positive",
  },
  (invoice) => [
    paragraph(text(invoice.supplier.legalName)),
    paragraph(text(invoice.totals.payable.amount)),
  ],
);

defineTemplate<CommercialDocumentTemplateData>(
  {
    name: "type-safety-negative",
  },
  (invoice) => [
    // @ts-expect-error missing fields must be caught by TypeScript.
    paragraph(text(invoice.supplier.missingLegalName)),
  ],
);

// @ts-expect-error required invoice totals must not be optional.
const missingTotals: CommercialDocumentTemplateData = {
  documentKind: "invoice",
  documentNumber: "INV-1",
  issueDate: "2026-01-01",
  supplier: {
    legalName: "Supplier",
    vatId: "DE123456789",
    address: {
      street: "Street 1",
      city: "Berlin",
      country: "DE",
    },
  },
  customer: {
    legalName: "Customer",
    vatId: "GB987654321",
    address: {
      street: "Street 2",
      city: "London",
      country: "GB",
    },
  },
  lines: [],
  taxSubtotals: [],
};
