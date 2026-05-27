// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/** @type {import('next').NextConfig} */
const nextConfig = {
  // The demo links against the local @invoicekit/core via a file:
  // dependency in package.json. Next.js's webpack picks the
  // package's main entry from there without any extra config.
  reactStrictMode: true,
};

export default nextConfig;
