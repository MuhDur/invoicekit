// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import nextra from "nextra";

const withNextra = nextra({
  theme: "nextra-theme-docs",
  themeConfig: "./theme.config.tsx",
  // Nextra ships flexsearch out of the box; the search bar in the
  // header reads every MDX file under pages/ and indexes it.
  search: { codeblocks: false },
});

export default withNextra({
  reactStrictMode: true,
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
});
