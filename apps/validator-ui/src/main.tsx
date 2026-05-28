// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import React from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";

const container = document.getElementById("root");
if (!container) {
  throw new Error("validator-ui: #root mount node missing from index.html");
}
createRoot(container).render(<App />);
