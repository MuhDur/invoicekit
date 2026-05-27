import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { OverviewContent, tenantOverviewQueryOptions } from "../routes/overview";
import {
  createHttpDashboardClient,
  usagePercent,
  type TenantOverview
} from "../src/engine";

const tenantOverviewFixture: TenantOverview = {
  tenantId: "tenant_demo_eu",
  tenantName: "Nordwind Trading",
  documentsSent: {
    used: 842,
    limit: 1200,
    periodLabel: "May 2026"
  },
  billing: {
    state: "active",
    headline: "Billing active",
    detail: "Evidence bundles and managed delivery are available for this tenant.",
    actionLabel: "View plan"
  },
  recentActivity: [
    {
      id: "act_01",
      kind: "sent",
      documentId: "INV-2026-0418",
      counterparty: "Atlas Components GmbH",
      occurredAt: "2026-05-27T14:22:00Z",
      summary: "Peppol submission accepted by partner access point.",
      traceId: "trc_6c91af"
    },
    {
      id: "act_02",
      kind: "validated",
      documentId: "INV-2026-0417",
      counterparty: "Marin Logistics BV",
      occurredAt: "2026-05-27T13:48:00Z",
      summary: "EN 16931 and Peppol BIS validation completed.",
      traceId: "trc_b71233"
    }
  ],
  validationFailures: 1,
  generatedAt: "2026-05-27T14:30:00Z"
};

describe("tenant overview adapter", () => {
  test("calls engine.tenant_overview over the Engine ABI JSON-RPC endpoint", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: tenantOverviewFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = tenantOverviewQueryOptions(client);
    const overview = await client.tenantOverview();
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("tenant-overview");
    expect(firstRequest.input).toBe("/engine");
    expect(firstRequest.init?.credentials).toBe("include");
    expect(body.method).toBe("engine.tenant_overview");
    expect(overview.tenantId).toBe("tenant_demo_eu");
    expect(overview.recentActivity[0]?.traceId).toBe("trc_6c91af");
  });

  test("caps the document usage percentage at 100", () => {
    const overview: TenantOverview = {
      ...tenantOverviewFixture,
      documentsSent: {
        limit: 10,
        periodLabel: "May 2026",
        used: 19
      }
    };

    expect(usagePercent(overview.documentsSent)).toBe(100);
  });
});

describe("overview route rendering", () => {
  test("renders billing state, document gauge, and recent activity", () => {
    const markup = renderToStaticMarkup(<OverviewContent overview={tenantOverviewFixture} />);

    expect(markup).toContain("Nordwind Trading");
    expect(markup).toContain("842 / 1,200");
    expect(markup).toContain("role=\"progressbar\"");
    expect(markup).toContain("Billing active");
    expect(markup).toContain("Peppol submission accepted");
    expect(markup).toContain("trc_6c91af");
  });
});
