// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createManagedApiClient, ManagedApiError } from "../src/index.ts";

function mockFetch(responder) {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    return responder(url, init);
  };
  return { fetchImpl, calls };
}

function jsonResponse(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: () => null },
    text: async () => JSON.stringify(body),
    json: async () => body,
  };
}

test("getAuditEvents issues a GET with bearer auth", async () => {
  const { fetchImpl, calls } = mockFetch(() =>
    jsonResponse(200, { events: [], next_cursor: null }),
  );
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  const page = await client.getAuditEvents();
  assert.deepEqual(page, { events: [], next_cursor: null });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://api.example.com/v1/audit/events");
  assert.equal(calls[0].init.method, "GET");
  assert.equal(calls[0].init.headers.authorization, "Bearer test-key");
});

test("getAuditEvents serializes pagination params", async () => {
  const { fetchImpl, calls } = mockFetch(() =>
    jsonResponse(200, { events: [], next_cursor: null }),
  );
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  await client.getAuditEvents({ cursor: "abc", limit: 25, since: "2026-01-01T00:00:00Z" });
  assert.match(calls[0].url, /\?cursor=abc&limit=25&since=2026-01-01T00%3A00%3A00Z$/);
});

test("strips trailing slash from baseUrl", async () => {
  const { fetchImpl, calls } = mockFetch(() =>
    jsonResponse(200, { events: [], next_cursor: null }),
  );
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com/",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  await client.getAuditEvents();
  assert.equal(calls[0].url, "https://api.example.com/v1/audit/events");
});

test("parses gateway error envelope into ManagedApiError", async () => {
  const { fetchImpl } = mockFetch(() =>
    jsonResponse(401, { error: { code: "UNAUTHORIZED", message: "missing api key" } }),
  );
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  await assert.rejects(
    client.getAuditEvents(),
    (err) =>
      err instanceof ManagedApiError &&
      err.code === "UNAUTHORIZED" &&
      err.status === 401 &&
      /missing api key/.test(err.message),
  );
});

test("wraps non-JSON error body with HTTP code", async () => {
  const fetchImpl = async () => ({
    ok: false,
    status: 502,
    headers: { get: () => null },
    text: async () => "Bad Gateway",
    json: async () => {
      throw new Error("not json");
    },
  });
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  await assert.rejects(
    client.getAuditEvents(),
    (err) =>
      err instanceof ManagedApiError &&
      err.code === "HTTP" &&
      err.status === 502 &&
      /Bad Gateway/.test(err.message),
  );
});

test("wraps fetch-thrown errors with NETWORK code", async () => {
  const fetchImpl = async () => {
    throw new TypeError("connection refused");
  };
  const client = createManagedApiClient({
    baseUrl: "https://api.example.com",
    apiKey: "test-key",
    fetch: fetchImpl,
  });
  await assert.rejects(
    client.getAuditEvents(),
    (err) =>
      err instanceof ManagedApiError &&
      err.code === "NETWORK" &&
      /connection refused/.test(err.message),
  );
});

test("requires baseUrl and apiKey", () => {
  assert.throws(
    () => createManagedApiClient({ baseUrl: "", apiKey: "k", fetch: () => {} }),
    /baseUrl is required/,
  );
  assert.throws(
    () => createManagedApiClient({ baseUrl: "https://x", apiKey: "", fetch: () => {} }),
    /apiKey is required/,
  );
});

test("requires fetch to be available", () => {
  const original = globalThis.fetch;
  try {
    delete globalThis.fetch;
    assert.throws(
      () => createManagedApiClient({ baseUrl: "https://x", apiKey: "k" }),
      /no fetch available/,
    );
  } finally {
    if (original !== undefined) globalThis.fetch = original;
  }
});
