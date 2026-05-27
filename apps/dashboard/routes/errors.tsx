import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { BellRing, RefreshCw, RotateCw, ServerCog, ShieldAlert } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type DashboardEngineClient,
  type RecentError,
  type RecentErrorPage,
  type RecentErrorSeverity,
  type RecentErrorSource
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function recentErrorsQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["recent-errors", { limit: 50 }],
    queryFn: () => client.listRecentErrors({ limit: 50 })
  });
}

export function ErrorsRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(recentErrorsQueryOptions(dashboardClient));

  return (
    <ErrorsContent
      errors={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["recent-errors"] });
      }}
    />
  );
}

interface ErrorsContentProps {
  readonly errors: RecentErrorPage;
  readonly onRefresh?: () => void;
}

export function ErrorsContent({ errors, onRefresh }: ErrorsContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Errors</p>
          <h1>Recent failures</h1>
          <p className="muted">Gateway errors, validator findings, retries, and remediation traces.</p>
        </div>
        <Button aria-label="Refresh recent errors" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Recent gateway and validator errors">
        <div className="panel-heading">
          <div>
            <h2>Recent errors</h2>
            <p className="muted">{errors.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={errors.pageInfo.hasNextPage ? "warning" : "good"}>
            {errors.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Error</th>
                <th scope="col">Severity</th>
                <th scope="col">Source</th>
                <th scope="col">Document</th>
                <th scope="col">Trace</th>
                <th scope="col">Occurred</th>
              </tr>
            </thead>
            <tbody>
              {errors.items.length > 0 ? (
                errors.items.map((error) => <RecentErrorRow error={error} key={error.id} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={6}>
                    No recent errors.
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

function RecentErrorRow({ error }: { readonly error: RecentError }) {
  const occurredAt = new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(error.occurredAt));

  return (
    <tr>
      <td>
        <div className="document-cell">
          {sourceIcon(error.source)}
          <span>
            <strong>{error.summary}</strong>
            <small>{error.remediation}</small>
          </span>
        </div>
      </td>
      <td>
        <StatusPill tone={severityTone(error.severity)}>{error.severity}</StatusPill>
      </td>
      <td>{sourceLabel(error.source)}</td>
      <td>{error.documentId}</td>
      <td>{error.traceId}</td>
      <td>{occurredAt}</td>
    </tr>
  );
}

function sourceIcon(source: RecentErrorSource) {
  switch (source) {
    case "gateway":
      return <BellRing size={18} aria-hidden="true" />;
    case "retry":
      return <RotateCw size={18} aria-hidden="true" />;
    case "system":
      return <ServerCog size={18} aria-hidden="true" />;
    case "validator":
      return <ShieldAlert size={18} aria-hidden="true" />;
  }
}

function sourceLabel(source: RecentErrorSource): string {
  switch (source) {
    case "gateway":
      return "Gateway";
    case "retry":
      return "Retry";
    case "system":
      return "System";
    case "validator":
      return "Validator";
  }
}

function severityTone(severity: RecentErrorSeverity): "critical" | "neutral" | "warning" {
  switch (severity) {
    case "critical":
      return "critical";
    case "info":
      return "neutral";
    case "warning":
      return "warning";
  }
}
