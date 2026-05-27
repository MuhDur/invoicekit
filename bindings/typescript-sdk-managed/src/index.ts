// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/**
 * @invoicekit/managed — typed REST client for the InvoiceKit
 * managed-API gateway (T-134).
 *
 * Endpoints covered today:
 *   GET /v1/audit/events     — paginated audit-log query (T-142)
 *   POST /v1/reconcile       — bank-statement reconciliation (T-075, when wired)
 *   GET /v1/events/sse       — SSE event stream with reconnect (T-077, when wired)
 *
 * Design notes:
 *   - The client takes a `fetch`-compatible function; consumers can
 *     inject `globalThis.fetch`, an undici instance, a mocked
 *     function in tests, or a Deno/Bun polyfill.
 *   - Auth is `Bearer <apiKey>`; the client adds it on every call.
 *   - Errors come back from the gateway as
 *     `{ error: { code, message } }`; the client parses that shape
 *     into a typed `ManagedApiError`. Non-JSON bodies and unexpected
 *     statuses surface as `ManagedApiError` with `code: "NETWORK"`
 *     or `code: "PARSE"` so caller-side `instanceof` checks are
 *     enough.
 */

export const SDK_MANAGED_BEAD_ID = "invoices-t-103-typescript-sdk-bhkn";

/** Minimal subset of the WHATWG `fetch` signature we depend on. */
export type FetchLike = (
  input: string,
  init?: {
    method?: string;
    headers?: Record<string, string>;
    body?: string;
    signal?: AbortSignal;
  },
) => Promise<{
  ok: boolean;
  status: number;
  headers: { get(name: string): string | null };
  text(): Promise<string>;
  json(): Promise<unknown>;
}>;

export interface ManagedApiClientOptions {
  /** Base URL of the gateway (e.g. `https://api.invoicekit.example`). */
  baseUrl: string;
  /** Bearer API key issued to the tenant. */
  apiKey: string;
  /** `fetch`-compatible function. Defaults to `globalThis.fetch`. */
  fetch?: FetchLike;
  /** Default per-request `AbortSignal`. */
  signal?: AbortSignal;
}

/** Shape of a paged audit-log response. */
export interface AuditEventsPage {
  events: AuditEvent[];
  next_cursor: string | null;
}

export interface AuditEvent {
  id: string;
  occurred_at: string;
  actor: string;
  action: string;
  resource: string;
  payload?: Record<string, unknown>;
}

export interface GetAuditEventsParams {
  cursor?: string;
  limit?: number;
  since?: string;
  until?: string;
}

export class ManagedApiError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly status?: number,
  ) {
    super(message);
    this.name = "ManagedApiError";
  }
}

export function createManagedApiClient(options: ManagedApiClientOptions) {
  if (!options.baseUrl) {
    throw new Error("createManagedApiClient: baseUrl is required");
  }
  if (!options.apiKey) {
    throw new Error("createManagedApiClient: apiKey is required");
  }
  const resolved = options.fetch ?? (globalThis.fetch as FetchLike | undefined);
  if (typeof resolved !== "function") {
    throw new Error(
      "createManagedApiClient: no fetch available; pass `fetch` explicitly",
    );
  }
  const fetchImpl: FetchLike = resolved;
  const baseUrl = options.baseUrl.replace(/\/$/, "");
  const defaultSignal = options.signal;

  async function request<T>(
    path: string,
    init: { method: string; query?: Record<string, string | number | undefined>; body?: unknown; signal?: AbortSignal } = { method: "GET" },
  ): Promise<T> {
    const url = `${baseUrl}${path}${formatQuery(init.query)}`;
    const headers: Record<string, string> = {
      "authorization": `Bearer ${options.apiKey}`,
      "accept": "application/json",
    };
    let body: string | undefined;
    if (init.body !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(init.body);
    }
    let resp;
    try {
      const requestInit: {
        method: string;
        headers: Record<string, string>;
        body?: string;
        signal?: AbortSignal;
      } = { method: init.method, headers };
      if (body !== undefined) {
        requestInit.body = body;
      }
      const signal = init.signal ?? defaultSignal;
      if (signal !== undefined) {
        requestInit.signal = signal;
      }
      resp = await fetchImpl(url, requestInit);
    } catch (cause) {
      throw new ManagedApiError("NETWORK", describe(cause));
    }
    if (!resp.ok) {
      let parsedErr;
      try {
        parsedErr = (await resp.json()) as { error?: { code?: string; message?: string } };
      } catch {
        const fallbackText = await resp.text().catch(() => "");
        throw new ManagedApiError(
          "HTTP",
          fallbackText || `request failed with status ${resp.status}`,
          resp.status,
        );
      }
      const code = parsedErr.error?.code ?? "HTTP";
      const message = parsedErr.error?.message ?? `request failed with status ${resp.status}`;
      throw new ManagedApiError(code, message, resp.status);
    }
    try {
      return (await resp.json()) as T;
    } catch (cause) {
      throw new ManagedApiError("PARSE", describe(cause), resp.status);
    }
  }

  return {
    /** GET /v1/audit/events */
    async getAuditEvents(params: GetAuditEventsParams = {}): Promise<AuditEventsPage> {
      return request<AuditEventsPage>("/v1/audit/events", {
        method: "GET",
        query: {
          cursor: params.cursor,
          limit: params.limit,
          since: params.since,
          until: params.until,
        },
      });
    },
  };
}

function formatQuery(query?: Record<string, string | number | undefined>): string {
  if (!query) return "";
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined) continue;
    params.append(k, String(v));
  }
  const s = params.toString();
  return s ? `?${s}` : "";
}

function describe(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  try {
    return JSON.stringify(cause);
  } catch {
    return String(cause);
  }
}
