// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1400 reference Next.js demo: root layout.

import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: "InvoiceKit Next.js Demo",
  description: "Issue a German XRechnung in under 5 minutes using @invoicekit/core.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body
        style={{
          fontFamily: "system-ui, -apple-system, sans-serif",
          margin: 0,
          padding: "2rem",
          background: "#fafafa",
          color: "#111",
        }}
      >
        {children}
      </body>
    </html>
  );
}
