// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { describe, expect, test, mock } from "bun:test";

import {
  DEFAULT_RULE_PACK_VERSION,
  validateLocal,
  validateReference,
} from "../src/validator";

describe("validateLocal", () => {
  test("returns the empty-input finding on whitespace-only input", async () => {
    const r = await validateLocal("   \n  ");
    expect(r.mode).toBe("local");
    expect(r.rule_pack_version).toBe(DEFAULT_RULE_PACK_VERSION);
    expect(r.backend).toContain("wasm");
    expect(r.findings).toHaveLength(1);
    expect(r.findings[0]?.rule_id).toBe("ui.input.empty");
  });

  test("returns the scaffold finding for any non-empty input", async () => {
    const r = await validateLocal("<Invoice/>");
    expect(r.mode).toBe("local");
    expect(r.findings[0]?.rule_id).toBe("ui.scaffold.wasm-pending");
    expect(r.findings[0]?.severity).toBe("warning");
  });
});

describe("validateReference", () => {
  test("surfaces HTTP errors as ui.transport.http findings", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async () =>
      new Response("nope", { status: 500, statusText: "Server Error" })
    ) as unknown as typeof globalThis.fetch;
    try {
      const r = await validateReference("<x/>", {
        baseUrl: "https://example.invalid",
      });
      expect(r.mode).toBe("reference");
      expect(r.findings[0]?.rule_id).toBe("ui.transport.http");
      expect(r.findings[0]?.severity).toBe("error");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  test("surfaces network failures as ui.transport.error findings", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async () => {
      throw new Error("network down");
    }) as unknown as typeof globalThis.fetch;
    try {
      const r = await validateReference("<x/>", {
        baseUrl: "https://example.invalid",
      });
      expect(r.findings[0]?.rule_id).toBe("ui.transport.error");
      expect(r.findings[0]?.message).toContain("network down");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  test("parses well-formed reference JSON responses", async () => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async () =>
      new Response(
        JSON.stringify({
          rule_pack_version: "en16931-2017+peppol-bis-3.0.20",
          backend: "validator-kosit-1.5.0",
          findings: [
            {
              rule_id: "BR-CO-10",
              severity: "error",
              message: "VAT subtotals do not close",
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      )
    ) as unknown as typeof globalThis.fetch;
    try {
      const r = await validateReference("<x/>", {
        baseUrl: "https://example.invalid/",
      });
      expect(r.backend).toBe("validator-kosit-1.5.0");
      expect(r.rule_pack_version).toBe("en16931-2017+peppol-bis-3.0.20");
      expect(r.findings).toHaveLength(1);
      expect(r.findings[0]?.rule_id).toBe("BR-CO-10");
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
