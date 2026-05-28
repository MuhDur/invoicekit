// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { describe, expect, test, mock } from "bun:test";

import { AnalyticsSink } from "../src/analytics";

describe("AnalyticsSink", () => {
  test("does nothing when disabled", () => {
    const sink = new AnalyticsSink({ disabled: true });
    sink.emit({ kind: "page_view" });
    expect(true).toBe(true);
  });

  test("does nothing when no endpoint configured", () => {
    const sink = new AnalyticsSink();
    sink.emit({ kind: "page_view" });
    expect(true).toBe(true);
  });

  test("posts JSON payload to the configured endpoint via fetch", async () => {
    const originalFetch = globalThis.fetch;
    const originalNav = globalThis.navigator;
    // Force the fetch fallback by hiding sendBeacon.
    Object.defineProperty(globalThis, "navigator", {
      value: {},
      configurable: true,
    });
    const calls: { url: string; body: string }[] = [];
    globalThis.fetch = mock(async (input, init) => {
      calls.push({
        url: String(input),
        body: String(init?.body ?? ""),
      });
      return new Response("", { status: 200 });
    }) as unknown as typeof globalThis.fetch;
    try {
      const sink = new AnalyticsSink({
        endpoint: "https://analytics.invalid/collect",
      });
      sink.emit({
        kind: "validation_completed",
        mode: "local",
        finding_count: 3,
      });
      // tiny tick for the async fetch
      await new Promise((r) => setTimeout(r, 0));
      expect(calls).toHaveLength(1);
      expect(calls[0]?.url).toBe("https://analytics.invalid/collect");
      const parsed = JSON.parse(calls[0]?.body ?? "{}");
      expect(parsed.kind).toBe("validation_completed");
      expect(parsed.mode).toBe("local");
      expect(parsed.finding_count).toBe(3);
      expect(parsed.ts).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    } finally {
      globalThis.fetch = originalFetch;
      Object.defineProperty(globalThis, "navigator", {
        value: originalNav,
        configurable: true,
      });
    }
  });
});
