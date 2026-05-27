import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { BillingContent, tenantBillingQueryOptions } from "../routes/billing";
import { createHttpDashboardClient, type TenantBilling } from "../src/engine";

const tenantBillingFixture: TenantBilling = {
  planName: "Hosted Team",
  planSlug: "hosted-team",
  state: "active",
  billingEmail: "finance@example.com",
  currentPeriodEnd: "2026-06-01T00:00:00Z",
  documentsIncluded: 5000,
  documentsUsed: 1842,
  currency: "EUR",
  monthlyBasePrice: "199.00",
  portalSessionUrl: "https://billing.example.com/session/test",
  upcomingInvoice: {
    id: "in_upcoming_01",
    amountDue: "214.80",
    currency: "EUR",
    dueDate: "2026-06-03T00:00:00Z",
    status: "open"
  }
};

describe("billing adapter", () => {
  test("calls engine.tenant_billing over the Engine ABI JSON-RPC endpoint", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: tenantBillingFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = tenantBillingQueryOptions(client);
    const billing = await client.tenantBilling();
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("tenant-billing");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.tenant_billing");
    expect(billing.planSlug).toBe("hosted-team");
    expect(billing.upcomingInvoice?.amountDue).toBe("214.80");
  });

  test("rejects unsupported upcoming invoice statuses from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...tenantBillingFixture,
            upcomingInvoice: {
              ...tenantBillingFixture.upcomingInvoice,
              status: "collecting"
            }
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.tenantBilling()).rejects.toThrow("unsupported upcoming invoice status");
  });
});

describe("billing route rendering", () => {
  test("renders plan status, portal link, and upcoming invoice", () => {
    const markup = renderToStaticMarkup(<BillingContent billing={tenantBillingFixture} />);

    expect(markup).toContain("Billing");
    expect(markup).toContain("Hosted Team");
    expect(markup).toContain("finance@example.com");
    expect(markup).toContain("1,842");
    expect(markup).toContain("199.00 EUR");
    expect(markup).toContain("Billing portal");
    expect(markup).toContain("in_upcoming_01");
    expect(markup).toContain("214.80 EUR");
  });

  test("renders no-upcoming-invoice state", () => {
    const billingWithoutUpcoming: TenantBilling = {
      planName: "Hosted Team",
      planSlug: "hosted-team",
      state: "active",
      billingEmail: "finance@example.com",
      currentPeriodEnd: "2026-06-01T00:00:00Z",
      documentsIncluded: 5000,
      documentsUsed: 1842,
      currency: "EUR",
      monthlyBasePrice: "199.00"
    };
    const markup = renderToStaticMarkup(
      <BillingContent billing={billingWithoutUpcoming} />
    );

    expect(markup).toContain("No upcoming invoice");
    expect(markup).not.toContain("Billing portal");
  });
});
