import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ErrorsContent, recentErrorsQueryOptions } from "../routes/errors";
import { createHttpDashboardClient, type RecentErrorPage } from "../src/engine";

const recentErrorPageFixture: RecentErrorPage = {
  items: [
    {
      id: "err_01",
      occurredAt: "2026-05-27T16:04:00Z",
      source: "gateway",
      severity: "critical",
      documentId: "INV-2026-0042",
      summary: "Peppol access point rejected the envelope",
      remediation: "Retry after refreshing the SMP route.",
      traceId: "trc_gateway_01"
    },
    {
      id: "err_02",
      occurredAt: "2026-05-27T16:18:00Z",
      source: "validator",
      severity: "warning",
      documentId: "INV-2026-0043",
      summary: "EN 16931 warning on buyer reference",
      remediation: "Collect a buyer reference before resubmitting.",
      traceId: "trc_validator_02"
    }
  ],
  pageInfo: {
    endCursor: "err_02",
    hasNextPage: true,
    limit: 50
  }
};

describe("errors adapter", () => {
  test("calls engine.list_recent_errors with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: recentErrorPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = recentErrorsQueryOptions(client);
    const page = await client.listRecentErrors({ limit: 50, severity: "critical" });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("recent-errors");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_recent_errors");
    expect(params.limit).toBe(50);
    expect(params.severity).toBe("critical");
    expect(page.items[0]?.traceId).toBe("trc_gateway_01");
  });

  test("rejects unsupported error sources from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...recentErrorPageFixture,
            items: [
              {
                ...recentErrorPageFixture.items[0],
                source: "mailbox"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listRecentErrors()).rejects.toThrow("unsupported error source");
  });
});

describe("errors route rendering", () => {
  test("renders recent failures with remediation and trace metadata", () => {
    const markup = renderToStaticMarkup(<ErrorsContent errors={recentErrorPageFixture} />);

    expect(markup).toContain("Recent failures");
    expect(markup).toContain("Peppol access point rejected the envelope");
    expect(markup).toContain("Retry after refreshing the SMP route.");
    expect(markup).toContain("Gateway");
    expect(markup).toContain("INV-2026-0042");
    expect(markup).toContain("trc_gateway_01");
    expect(markup).toContain("warning");
  });

  test("renders an empty state when no recent errors exist", () => {
    const markup = renderToStaticMarkup(
      <ErrorsContent
        errors={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 50
          }
        }}
      />
    );

    expect(markup).toContain("No recent errors.");
  });
});
