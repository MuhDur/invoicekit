import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { AuditContent, auditEventsQueryOptions } from "../routes/audit";
import { createHttpDashboardClient, type AuditEventPage } from "../src/engine";

const auditEventPageFixture: AuditEventPage = {
  items: [
    {
      id: "aud_01",
      occurredAt: "2026-05-27T15:22:00Z",
      actor: "ada@example.com",
      action: "transmission.retry",
      resourceType: "transmission",
      resourceId: "tx_01",
      outcome: "succeeded",
      traceId: "trc_retry_01"
    },
    {
      id: "aud_02",
      occurredAt: "2026-05-27T15:29:00Z",
      actor: "sam@example.com",
      action: "api_key.rotate",
      resourceType: "api_key",
      resourceId: "key_live_01",
      outcome: "denied",
      traceId: "trc_key_02"
    }
  ],
  pageInfo: {
    endCursor: "aud_02",
    hasNextPage: true,
    limit: 50
  }
};

describe("audit adapter", () => {
  test("calls engine.list_audit_events with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: auditEventPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = auditEventsQueryOptions(client);
    const page = await client.listAuditEvents({ actor: "ada@example.com", limit: 50, outcome: "succeeded" });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("audit-events");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_audit_events");
    expect(params.actor).toBe("ada@example.com");
    expect(params.limit).toBe(50);
    expect(params.outcome).toBe("succeeded");
    expect(page.items[0]?.traceId).toBe("trc_retry_01");
  });

  test("rejects unsupported audit outcomes from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...auditEventPageFixture,
            items: [
              {
                ...auditEventPageFixture.items[0],
                outcome: "maybe"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listAuditEvents()).rejects.toThrow("unsupported audit outcome");
  });
});

describe("audit route rendering", () => {
  test("renders the audit table and trace metadata", () => {
    const markup = renderToStaticMarkup(<AuditContent auditEvents={auditEventPageFixture} />);

    expect(markup).toContain("Tenant events");
    expect(markup).toContain("transmission.retry");
    expect(markup).toContain("ada@example.com");
    expect(markup).toContain("tx_01");
    expect(markup).toContain("trc_retry_01");
    expect(markup).toContain("denied");
  });

  test("renders an empty state when no audit events exist", () => {
    const markup = renderToStaticMarkup(
      <AuditContent
        auditEvents={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 50
          }
        }}
      />
    );

    expect(markup).toContain("No audit events yet.");
  });
});
