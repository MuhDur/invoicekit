import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { UsageContent, tenantUsageQueryOptions } from "../routes/usage";
import { createHttpDashboardClient, type TenantUsage } from "../src/engine";

const tenantUsageFixture: TenantUsage = {
  periodLabel: "Last 6 months",
  totalSent: 2480,
  totalReceived: 312,
  partnerApCostTotal: "184.20",
  currency: "EUR",
  months: [
    {
      month: "2026-05",
      documentsSent: 842,
      documentsReceived: 91,
      partnerApCost: "63.15"
    },
    {
      month: "2026-04",
      documentsSent: 731,
      documentsReceived: 84,
      partnerApCost: "54.82"
    }
  ]
};

describe("usage adapter", () => {
  test("calls engine.tenant_usage over the Engine ABI JSON-RPC endpoint", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: tenantUsageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = tenantUsageQueryOptions(client);
    const usage = await client.tenantUsage();
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("tenant-usage");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.tenant_usage");
    expect(usage.partnerApCostTotal).toBe("184.20");
    expect(usage.months[0]?.documentsReceived).toBe(91);
  });

  test("rejects malformed month counters from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...tenantUsageFixture,
            months: [
              {
                ...tenantUsageFixture.months[0],
                documentsSent: "842"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.tenantUsage()).rejects.toThrow("documentsSent must be a finite number");
  });
});

describe("usage route rendering", () => {
  test("renders totals and monthly usage rows", () => {
    const markup = renderToStaticMarkup(<UsageContent usage={tenantUsageFixture} />);

    expect(markup).toContain("Last 6 months");
    expect(markup).toContain("2,480");
    expect(markup).toContain("312");
    expect(markup).toContain("184.20 EUR");
    expect(markup).toContain("2026-05");
    expect(markup).toContain("63.15 EUR");
  });

  test("renders an empty state when no usage exists", () => {
    const markup = renderToStaticMarkup(
      <UsageContent
        usage={{
          currency: "EUR",
          months: [],
          partnerApCostTotal: "0.00",
          periodLabel: "Last 6 months",
          totalReceived: 0,
          totalSent: 0
        }}
      />
    );

    expect(markup).toContain("No usage recorded yet.");
  });
});
