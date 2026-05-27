import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { DocumentsContent, transmissionsQueryOptions } from "../routes/documents";
import { createHttpDashboardClient, type TransmissionPage } from "../src/engine";

const transmissionPageFixture: TransmissionPage = {
  items: [
    {
      id: "tx_01",
      documentId: "INV-2026-0501",
      state: "accepted",
      gateway: "Peppol partner AP",
      recipient: "Atlas Components GmbH",
      recipientCountry: "DE",
      issueDate: "2026-05-27",
      updatedAt: "2026-05-27T15:05:00Z",
      amount: "1200.00",
      currency: "EUR",
      receiptUrl: "/documents/tx_01/receipt",
      evidenceBundleUrl: "/documents/tx_01/bundle.invoicekit"
    },
    {
      id: "tx_02",
      documentId: "INV-2026-0502",
      state: "queued",
      gateway: "ZATCA sandbox",
      recipient: "Riyadh Parts Co.",
      recipientCountry: "SA",
      issueDate: "2026-05-27",
      updatedAt: "2026-05-27T15:10:00Z",
      amount: "840.50",
      currency: "SAR"
    }
  ],
  pageInfo: {
    endCursor: "cursor_02",
    hasNextPage: true,
    limit: 25
  }
};

describe("documents adapter", () => {
  test("calls engine.list_transmissions with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: transmissionPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = transmissionsQueryOptions(client);
    const page = await client.listTransmissions({ cursor: "after_01", limit: 50 });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("transmissions");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_transmissions");
    expect(params.cursor).toBe("after_01");
    expect(params.limit).toBe(50);
    expect(page.items[0]?.receiptUrl).toBe("/documents/tx_01/receipt");
    expect(page.pageInfo.hasNextPage).toBe(true);
  });

  test("rejects unsupported transmission states from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...transmissionPageFixture,
            items: [
              {
                ...transmissionPageFixture.items[0],
                state: "invented"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listTransmissions()).rejects.toThrow("unsupported transmission state");
  });
});

describe("documents route rendering", () => {
  test("renders the transmission table and artifact links", () => {
    const markup = renderToStaticMarkup(<DocumentsContent transmissions={transmissionPageFixture} />);

    expect(markup).toContain("Transmissions");
    expect(markup).toContain("INV-2026-0501");
    expect(markup).toContain("Peppol partner AP");
    expect(markup).toContain("Atlas Components GmbH");
    expect(markup).toContain("1200.00 EUR");
    expect(markup).toContain("/documents/tx_01/receipt");
    expect(markup).toContain("Pending");
  });

  test("renders an empty state when no transmissions exist", () => {
    const markup = renderToStaticMarkup(
      <DocumentsContent
        transmissions={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 25
          }
        }}
      />
    );

    expect(markup).toContain("No transmissions yet.");
  });
});
