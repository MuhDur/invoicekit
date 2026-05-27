// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-114 shared data fixtures + variants for the storybook
// stories. Builds on the templates package's `examples/shared`
// baseInvoice but adds the strict-gate variants:
//
//   * with allowances (negative-quantity line modelling a
//     volume discount)
//   * with reverse charge (BR-AE category code, customer in
//     another EU member state)

import type { CommercialDocumentTemplateData, MoneyAmount, Party } from "../../typescript/src/index.ts";

const eur = (amount: string): MoneyAmount => ({ amount, currency: "EUR" });

const supplier: Party = {
  legalName: "InvoiceKit Trust Toolkit GmbH",
  vatId: "DE123456789",
  address: {
    street: "Reference Street 1",
    city: "Berlin",
    country: "DE",
  },
};

const customerDE: Party = {
  legalName: "Deterministic Buyer Ltd",
  vatId: "DE987654321",
  address: {
    street: "Audit Road 8",
    city: "Munich",
    country: "DE",
  },
};

const customerAT: Party = {
  legalName: "Beispielkunde AG",
  vatId: "ATU12345678",
  address: {
    street: "Stephansplatz 1",
    city: "Vienna",
    country: "AT",
  },
};

const baseLines = [
  {
    description: "Conformance corpus review",
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
];

export const basicData: CommercialDocumentTemplateData = {
  documentKind: "invoice",
  documentNumber: "INV-2026-0001",
  issueDate: "2026-05-27",
  dueDate: "2026-06-26",
  supplier,
  customer: customerDE,
  lines: baseLines,
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

export const withAllowanceData: CommercialDocumentTemplateData = {
  ...basicData,
  documentNumber: "INV-2026-0002",
  lines: [
    ...baseLines,
    {
      description: "Volume rebate (-10%)",
      quantity: "-1",
      unitPrice: eur("15.00"),
      lineTotal: eur("-15.00"),
    },
  ],
  taxSubtotals: [
    {
      category: "S",
      rate: "19",
      taxableAmount: eur("135.00"),
      taxAmount: eur("25.65"),
    },
  ],
  totals: {
    net: eur("135.00"),
    tax: eur("25.65"),
    payable: eur("160.65"),
  },
};

export const reverseChargeData: CommercialDocumentTemplateData = {
  ...basicData,
  documentNumber: "INV-2026-0003",
  customer: customerAT,
  taxSubtotals: [
    {
      category: "AE",
      rate: "0",
      taxableAmount: eur("150.00"),
      taxAmount: eur("0.00"),
    },
  ],
  totals: {
    net: eur("150.00"),
    tax: eur("0.00"),
    payable: eur("150.00"),
  },
  note: "Reverse charge: VAT due by the customer (BR-AE).",
};
