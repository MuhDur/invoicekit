// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2022",
    sourcemap: true,
  },
});
