import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { Crown, Eye, RefreshCw, UserCheck, UserRoundCog } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type DashboardEngineClient,
  type TeamMember,
  type TeamMemberPage,
  type TeamMemberRole,
  type TeamMemberStatus
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function teamMembersQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["team-members", { limit: 50 }],
    queryFn: () => client.listTeamMembers({ limit: 50 })
  });
}

export function TeamRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(teamMembersQueryOptions(dashboardClient));

  return (
    <TeamContent
      teamMembers={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["team-members"] });
      }}
    />
  );
}

interface TeamContentProps {
  readonly onRefresh?: () => void;
  readonly teamMembers: TeamMemberPage;
}

export function TeamContent({ onRefresh, teamMembers }: TeamContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Settings</p>
          <h1>Team members</h1>
          <p className="muted">Tenant users, roles, access status, and last activity.</p>
        </div>
        <Button aria-label="Refresh team members" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Tenant team members">
        <div className="panel-heading">
          <div>
            <h2>Members</h2>
            <p className="muted">{teamMembers.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={teamMembers.pageInfo.hasNextPage ? "warning" : "good"}>
            {teamMembers.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Member</th>
                <th scope="col">Role</th>
                <th scope="col">Status</th>
                <th scope="col">Last active</th>
                <th scope="col">Invited</th>
              </tr>
            </thead>
            <tbody>
              {teamMembers.items.length > 0 ? (
                teamMembers.items.map((member) => <TeamMemberRow key={member.id} member={member} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={5}>
                    No team members yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

function TeamMemberRow({ member }: { readonly member: TeamMember }) {
  return (
    <tr>
      <td>
        <div className="document-cell">
          <UserCheck size={18} aria-hidden="true" />
          <span>
            <strong>{member.displayName}</strong>
            <small>{member.email}</small>
          </span>
        </div>
      </td>
      <td>
        <div className="document-cell">
          {roleIcon(member.role)}
          <span>{roleLabel(member.role)}</span>
        </div>
      </td>
      <td>
        <StatusPill tone={statusTone(member.status)}>{statusLabel(member.status)}</StatusPill>
      </td>
      <td>{formatOptionalDate(member.lastActiveAt, "Never active")}</td>
      <td>{formatOptionalDate(member.invitedAt, "Already joined")}</td>
    </tr>
  );
}

function roleIcon(role: TeamMemberRole) {
  switch (role) {
    case "admin":
      return <Crown size={18} aria-hidden="true" />;
    case "operator":
      return <UserRoundCog size={18} aria-hidden="true" />;
    case "read_only":
      return <Eye size={18} aria-hidden="true" />;
  }
}

function roleLabel(role: TeamMemberRole): string {
  switch (role) {
    case "admin":
      return "Admin";
    case "operator":
      return "Operator";
    case "read_only":
      return "Read-only";
  }
}

function statusLabel(status: TeamMemberStatus): string {
  switch (status) {
    case "active":
      return "Active";
    case "disabled":
      return "Disabled";
    case "invited":
      return "Invited";
  }
}

function statusTone(status: TeamMemberStatus): "critical" | "good" | "neutral" | "warning" {
  switch (status) {
    case "active":
      return "good";
    case "disabled":
      return "critical";
    case "invited":
      return "warning";
  }
}

function formatOptionalDate(value: string | undefined, fallback: string): string {
  if (value === undefined) {
    return fallback;
  }

  return new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(value));
}
