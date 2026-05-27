import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiKeysContent, apiKeysQueryOptions } from "../routes/api-keys";
import { createHttpDashboardClient, type ApiKeyPage } from "../src/engine";

const apiKeyPageFixture: ApiKeyPage = {
  items: [
    {
      id: "key_01",
      name: "Production gateway",
      prefix: "ik_live_1234",
      scopes: ["transmissions:write", "evidence:read"],
      status: "active",
      createdAt: "2026-05-01T08:00:00Z",
      lastUsedAt: "2026-05-27T16:40:00Z",
      expiresAt: "2026-08-01T08:00:00Z"
    },
    {
      id: "key_02",
      name: "Retired sandbox",
      prefix: "ik_test_9876",
      scopes: ["transmissions:read"],
      status: "revoked",
      createdAt: "2026-03-14T08:00:00Z"
    }
  ],
  pageInfo: {
    hasNextPage: false,
    limit: 50
  }
};

describe("api keys adapter", () => {
  test("calls engine.list_api_keys with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: apiKeyPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = apiKeysQueryOptions(client);
    const page = await client.listApiKeys({ limit: 50, status: "active" });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("api-keys");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_api_keys");
    expect(params.limit).toBe(50);
    expect(params.status).toBe("active");
    expect(page.items[0]?.prefix).toBe("ik_live_1234");
  });

  test("rejects malformed scopes from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...apiKeyPageFixture,
            items: [
              {
                ...apiKeyPageFixture.items[0],
                scopes: ["transmissions:write", 42]
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listApiKeys()).rejects.toThrow("scopes must contain only strings");
  });
});

describe("api keys route rendering", () => {
  test("renders API keys with scopes and usage metadata", () => {
    const markup = renderToStaticMarkup(<ApiKeysContent apiKeys={apiKeyPageFixture} />);

    expect(markup).toContain("API keys");
    expect(markup).toContain("Production gateway");
    expect(markup).toContain("ik_live_1234");
    expect(markup).toContain("transmissions:write, evidence:read");
    expect(markup).toContain("Active");
    expect(markup).toContain("Retired sandbox");
    expect(markup).toContain("Never used");
    expect(markup).toContain("No expiry");
  });

  test("renders an empty state when no API keys exist", () => {
    const markup = renderToStaticMarkup(
      <ApiKeysContent
        apiKeys={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 50
          }
        }}
      />
    );

    expect(markup).toContain("No API keys yet.");
  });
});
