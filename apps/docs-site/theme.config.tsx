// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import React from "react";

const config = {
  logo: <span style={{ fontWeight: 700 }}>InvoiceKit</span>,
  project: { link: "https://github.com/MuhDur/invoicekit" },
  docsRepositoryBase:
    "https://github.com/MuhDur/invoicekit/tree/main/apps/docs-site",
  footer: {
    text: "InvoiceKit — open source under Apache 2.0",
  },
  head: () => (
    <>
      <meta name="description" content="InvoiceKit documentation — EN 16931 rules, country guides, integration walkthroughs." />
      <link rel="icon" href="data:," />
    </>
  ),
  primaryHue: 218,
  search: { placeholder: "Search rules, countries, guides…" },
};

export default config;
