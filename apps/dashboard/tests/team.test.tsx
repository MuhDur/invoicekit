import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { TeamContent, teamMembersQueryOptions } from "../routes/team";
import { createHttpDashboardClient, type TeamMemberPage } from "../src/engine";

const teamMemberPageFixture: TeamMemberPage = {
  items: [
    {
      id: "tm_01",
      email: "ada@example.com",
      displayName: "Ada Lovelace",
      role: "admin",
      status: "active",
      lastActiveAt: "2026-05-27T16:24:00Z",
      invitedAt: "2026-04-02T09:00:00Z"
    },
    {
      id: "tm_02",
      email: "lin@example.com",
      displayName: "Lin Chen",
      role: "read_only",
      status: "invited",
      invitedAt: "2026-05-26T11:12:00Z"
    }
  ],
  pageInfo: {
    endCursor: "tm_02",
    hasNextPage: false,
    limit: 50
  }
};

describe("team adapter", () => {
  test("calls engine.list_team_members with pagination params", async () => {
    const requests: Array<{ readonly input: RequestInfo | URL; readonly init: RequestInit | undefined }> = [];
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async (input, init) => {
        requests.push({ input, init });
        return Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: teamMemberPageFixture
        });
      },
      requestIdFactory: () => "test-request"
    });
    const query = teamMembersQueryOptions(client);
    const page = await client.listTeamMembers({ limit: 50, role: "admin" });
    const firstRequest = requests[0];

    if (firstRequest?.init?.body === undefined) {
      throw new Error("Expected Engine ABI request body");
    }

    const body = (await new Response(firstRequest.init.body).json()) as Record<string, unknown>;
    const params = body.params as Record<string, unknown>;

    expect(query.queryKey[0]).toBe("team-members");
    expect(firstRequest.input).toBe("/engine");
    expect(body.method).toBe("engine.list_team_members");
    expect(params.limit).toBe(50);
    expect(params.role).toBe("admin");
    expect(page.items[0]?.email).toBe("ada@example.com");
  });

  test("rejects unsupported team member roles from the Engine ABI", async () => {
    const client = createHttpDashboardClient({
      endpoint: "/engine",
      fetcher: async () =>
        Response.json({
          jsonrpc: "2.0",
          id: "test-request",
          result: {
            ...teamMemberPageFixture,
            items: [
              {
                ...teamMemberPageFixture.items[0],
                role: "owner"
              }
            ]
          }
        }),
      requestIdFactory: () => "test-request"
    });

    await expect(client.listTeamMembers()).rejects.toThrow("unsupported team member role");
  });
});

describe("team route rendering", () => {
  test("renders team members with role and activity metadata", () => {
    const markup = renderToStaticMarkup(<TeamContent teamMembers={teamMemberPageFixture} />);

    expect(markup).toContain("Team members");
    expect(markup).toContain("Ada Lovelace");
    expect(markup).toContain("ada@example.com");
    expect(markup).toContain("Admin");
    expect(markup).toContain("Active");
    expect(markup).toContain("Lin Chen");
    expect(markup).toContain("Read-only");
    expect(markup).toContain("Never active");
  });

  test("renders an empty state when no team members exist", () => {
    const markup = renderToStaticMarkup(
      <TeamContent
        teamMembers={{
          items: [],
          pageInfo: {
            hasNextPage: false,
            limit: 50
          }
        }}
      />
    );

    expect(markup).toContain("No team members yet.");
  });
});
