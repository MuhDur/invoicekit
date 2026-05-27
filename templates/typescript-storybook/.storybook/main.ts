// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-114 Storybook configuration for the InvoiceKit Typst
// template family.

import type { StorybookConfig } from "@storybook/html-vite";

const config: StorybookConfig = {
  framework: "@storybook/html-vite",
  stories: ["../src/**/*.stories.ts"],
  addons: [],
  docs: {
    autodocs: false,
  },
};

export default config;
