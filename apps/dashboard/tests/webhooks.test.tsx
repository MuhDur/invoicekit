import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { WebhooksContent, webhooksQueryOptions } from "../routes/webhooks";
import { createHttpDashboardClient, type WebhookEndpointPage } from "../src/engine";

const webhookEndpointPageFixture: WebhookEndpointPage = {
  items: [
    {
      id: "wh_01",
      name: "Production listener",
      url: "https://hooks.example.com/invoicekit",
      eventTypes: ["transmission.accepted", "transmission.failed"],
      status: "active",
      signingSecretPrefix: signingPrefix("1234"),
      createdAt: "2026-05-02T08:00:00Z",
      lastDeliveredAt: "2026-05-27T16:50:00Z",
      failureCount: 0
    },
    {
      id: "wh_02",
      name: "Accounting bridge",
      url: "https://erp.example.com/hooks",
      eventTypes: ["evidence.created"],
      status: "failing",
      signingSecretPrefix: signingPrefix("9876"),
      createdAt: "2026-05-10T08:00:00Z",
      failureCount: 3
    }
  ],
  pageInfo: {
    hasNextPage: true,
    limit: 50
  }
};

describe("webhooks adapter", () => {
  test("calls engine.list_webhooks with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: webhookEndpointPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = webhooksQueryOptions(client);
    const page = await client.listWebhooks({ limit: 50, status: "active" });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("webhooks");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_webhooks");
    expect(params.limit).toBe(50);
    expect(params.status).toBe("active");
    expect(page.items[0]?.eventTypes[0]).toBe("transmission.accepted");
  });

  test("rejects unsupported webhook statuses from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...webhookEndpointPageFixture,
            items: [
              {
                ...webhookEndpointPageFixture.items[0],
                status: "pending"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listWebhooks()).rejects.toThrow("unsupported webhook status");
  });
});

describe("webhooks route rendering", () => {
  test("renders webhooks with event and delivery metadata", () => {
    const markup = renderToStaticMarkup(<WebhooksContent webhooks={webhookEndpointPageFixture} />);

    expect(markup).toContain("Webhooks");
    expect(markup).toContain("Production listener");
    expect(markup).toContain("https://hooks.example.com/invoicekit");
    expect(markup).toContain("transmission.accepted, transmission.failed");
    expect(markup).toContain(signingPrefix("1234"));
    expect(markup).toContain("Accounting bridge");
    expect(markup).toContain("Failing");
    expect(markup).toContain("Never delivered");
  });

  test("renders an empty state when no webhooks exist", () => {
    const markup = renderToStaticMarkup(
      <WebhooksContent
        webhooks={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 50
          }
        }}
      />
    );

    expect(markup).toContain("No webhooks yet.");
  });
});

function signingPrefix(suffix: string): string {
  return `prefix_live_${suffix}`;
}
