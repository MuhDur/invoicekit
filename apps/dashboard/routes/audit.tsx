import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type AuditEvent,
  type AuditEventPage,
  type AuditOutcome,
  type DashboardEngineClient
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function auditEventsQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["audit-events", { limit: 50 }],
    queryFn: () => client.listAuditEvents({ limit: 50 })
  });
}

export function AuditRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(auditEventsQueryOptions(dashboardClient));

  return (
    <AuditContent
      auditEvents={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["audit-events"] });
      }}
    />
  );
}

interface AuditContentProps {
  readonly auditEvents: AuditEventPage;
  readonly onRefresh?: () => void;
}

export function AuditContent({ auditEvents, onRefresh }: AuditContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Audit log</p>
          <h1>Tenant events</h1>
          <p className="muted">Append-only actions, resources, outcomes, and trace identifiers.</p>
        </div>
        <Button aria-label="Refresh audit log" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Tenant audit events">
        <div className="panel-heading">
          <div>
            <h2>Recent audit events</h2>
            <p className="muted">{auditEvents.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={auditEvents.pageInfo.hasNextPage ? "warning" : "good"}>
            {auditEvents.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Event</th>
                <th scope="col">Outcome</th>
                <th scope="col">Actor</th>
                <th scope="col">Resource</th>
                <th scope="col">Trace</th>
                <th scope="col">Occurred</th>
              </tr>
            </thead>
            <tbody>
              {auditEvents.items.length > 0 ? (
                auditEvents.items.map((event) => <AuditEventRow event={event} key={event.id} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={6}>
                    No audit events yet.
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

function AuditEventRow({ event }: { readonly event: AuditEvent }) {
  const occurredAt = new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(event.occurredAt));

  return (
    <tr>
      <td>
        <div className="document-cell">
          {auditIcon(event.outcome)}
          <span>
            <strong>{event.action}</strong>
            <small>{event.id}</small>
          </span>
        </div>
      </td>
      <td>
        <StatusPill tone={auditTone(event.outcome)}>{event.outcome}</StatusPill>
      </td>
      <td>{event.actor}</td>
      <td>
        <span className="recipient-cell">
          {event.resourceType}
          <small>{event.resourceId}</small>
        </span>
      </td>
      <td>{event.traceId}</td>
      <td>{occurredAt}</td>
    </tr>
  );
}

function auditIcon(outcome: AuditOutcome) {
  switch (outcome) {
    case "denied":
    case "failed":
      return <ShieldAlert size={18} aria-hidden="true" />;
    case "succeeded":
      return <ShieldCheck size={18} aria-hidden="true" />;
  }
}

function auditTone(outcome: AuditOutcome): "critical" | "good" | "neutral" | "warning" {
  switch (outcome) {
    case "denied":
    case "failed":
      return "critical";
    case "succeeded":
      return "good";
  }
}
