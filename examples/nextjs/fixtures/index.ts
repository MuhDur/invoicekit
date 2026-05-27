// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1400 demo fixtures: three German XRechnung shapes the builder
// API can construct. Shared by the API route and the smoke test.

import type { InvoiceInput } from "@invoicekit/core";

const SELLER = {
  name: "Acme GmbH",
  taxId: { scheme: "vat", value: "DE123456789" },
  address: {
    lines: ["Hauptstraße 42"],
    city: "Berlin",
    postalCode: "10115",
    country: "DE",
  },
};

const BUYER = {
  name: "Beispielkunde AG",
  taxId: { scheme: "vat", value: "DE987654321" },
  address: {
    lines: ["Friedrichstraße 10"],
    city: "München",
    postalCode: "80331",
    country: "DE",
  },
};

export const fixtures: Record<string, InvoiceInput> = {
  basic: {
    id: "doc-de-basic-2026-0001",
    documentNumber: "RE-2026-0001",
    documentType: "invoice",
    issueDate: "2026-05-27",
    dueDate: "2026-06-26",
    currency: "EUR",
    supplier: SELLER,
    customer: BUYER,
    lines: [
      {
        description: "Software-Lizenz Q3/2026",
        quantity: "1",
        unitPrice: "1000.00",
        taxCategory: "S",
      },
    ],
    tenantId: "tenant-demo",
    traceId: "trace-de-basic-2026-0001",
  },

  "with-allowance": {
    id: "doc-de-allowance-2026-0002",
    documentNumber: "RE-2026-0002",
    documentType: "invoice",
    issueDate: "2026-05-27",
    dueDate: "2026-06-26",
    currency: "EUR",
    supplier: SELLER,
    customer: BUYER,
    lines: [
      {
        description: "Beratungsleistung März 2026",
        quantity: "10",
        unitPrice: "150.00",
        taxCategory: "S",
      },
      {
        // A documented 10% allowance applied per-line. The
        // builder treats negative quantities as discounts so the
        // total subtracts naturally on the engine side.
        description: "Mengenrabatt 10%",
        quantity: "-1",
        unitPrice: "150.00",
        taxCategory: "S",
      },
    ],
    tenantId: "tenant-demo",
    traceId: "trace-de-allowance-2026-0002",
  },

  "reverse-charge": {
    id: "doc-de-rc-2026-0003",
    documentNumber: "RE-2026-0003",
    documentType: "invoice",
    issueDate: "2026-05-27",
    dueDate: "2026-06-26",
    currency: "EUR",
    supplier: SELLER,
    // For reverse charge B2B the customer must be in another EU
    // member state — here, Austria.
    customer: {
      ...BUYER,
      taxId: { scheme: "vat", value: "ATU12345678" },
      address: {
        lines: ["Stephansplatz 1"],
        city: "Wien",
        postalCode: "1010",
        country: "AT",
      },
    },
    lines: [
      {
        description: "Wartungsvertrag Q3/2026",
        quantity: "1",
        unitPrice: "5000.00",
        // "AE" = VAT Reverse Charge per EN 16931 BT-118
        // category code list (UNTDID 5305).
        taxCategory: "AE",
      },
    ],
    tenantId: "tenant-demo",
    traceId: "trace-de-rc-2026-0003",
  },
};
