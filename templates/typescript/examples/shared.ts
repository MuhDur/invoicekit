// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import type { CommercialDocumentTemplateData, MoneyAmount, Party } from "../src/index.ts";

export const supplier: Party = {
  legalName: "InvoiceKit Trust Toolkit GmbH",
  vatId: "DE123456789",
  address: {
    street: "Reference Street 1",
    city: "Berlin",
    country: "DE",
  },
};

export const customer: Party = {
  legalName: "Deterministic Buyer Ltd",
  vatId: "GB987654321",
  address: {
    street: "Audit Road 8",
    city: "London",
    country: "GB",
  },
};

export function eur(amount: string): MoneyAmount {
  return {
    amount,
    currency: "EUR",
  };
}

export const baseInvoice: CommercialDocumentTemplateData = {
  documentKind: "invoice",
  documentNumber: "INV-2026-0001",
  issueDate: "2026-01-15",
  dueDate: "2026-02-14",
  supplier,
  customer,
  lines: [
    {
      description: "Format correctness subscription",
      quantity: "1",
      unitPrice: eur("100.00"),
      lineTotal: eur("100.00"),
    },
    {
      description: "Evidence bundle verification",
      quantity: "2",
      unitPrice: eur("25.00"),
      lineTotal: eur("50.00"),
    },
  ],
  taxSubtotals: [
    {
      category: "S",
      rate: "19",
      taxableAmount: eur("150.00"),
      taxAmount: eur("28.50"),
    },
  ],
  totals: {
    net: eur("150.00"),
    tax: eur("28.50"),
    payable: eur("178.50"),
  },
  paymentTerms: "Payable within 30 days by bank transfer.",
  note: "Generated from the typed InvoiceKit template language.",
};
