import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  Archive,
  CheckCircle2,
  CreditCard,
  FileCheck2,
  Gauge,
  RefreshCw,
  Send
} from "lucide-react";
import { Button } from "../components/ui/button";
import { MetricPanel } from "../components/ui/metric-panel";
import { StatusPill } from "../components/ui/status-pill";
import {
  billingTone,
  createHttpDashboardClient,
  type ActivityKind,
  type DashboardEngineClient,
  type RecentActivityItem,
  type TenantOverview,
  usagePercent
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function tenantOverviewQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["tenant-overview"],
    queryFn: () => client.tenantOverview()
  });
}

export function OverviewRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(tenantOverviewQueryOptions(dashboardClient));

  return (
    <OverviewContent
      overview={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["tenant-overview"] });
      }}
    />
  );
}

interface OverviewContentProps {
  readonly overview: TenantOverview;
  readonly onRefresh?: () => void;
}

export function OverviewContent({ onRefresh, overview }: OverviewContentProps) {
  const percent = usagePercent(overview.documentsSent);
  const billingStatusTone = billingTone(overview.billing.state);
  const lastGenerated = new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(overview.generatedAt));

  return (
    <div className="overview">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Tenant overview</p>
          <h1>{overview.tenantName}</h1>
          <p className="muted">Current period delivery, billing, and evidence activity.</p>
        </div>
        <Button aria-label="Refresh tenant overview" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="billing-banner" data-tone={billingStatusTone} aria-label="Billing state">
        <div className="billing-copy">
          <CreditCard size={24} aria-hidden="true" />
          <div>
            <h2>{overview.billing.headline}</h2>
            <p className="muted">{overview.billing.detail}</p>
          </div>
        </div>
        <StatusPill tone={billingStatusTone}>{overview.billing.state.replace("_", " ")}</StatusPill>
      </section>

      <section className="metric-grid" aria-label="Overview metrics">
        <MetricPanel
          foot={`${overview.documentsSent.periodLabel} usage`}
          icon={<FileCheck2 size={20} aria-hidden="true" />}
          label="Documents sent"
          value={`${overview.documentsSent.used.toLocaleString()} / ${overview.documentsSent.limit.toLocaleString()}`}
        >
          <div
            className="progress-track"
            aria-label={`${percent}% of document allowance used`}
            aria-valuemax={100}
            aria-valuemin={0}
            aria-valuenow={percent}
            role="progressbar"
          >
            <div className="progress-fill" style={{ width: `${percent}%` }} />
          </div>
        </MetricPanel>
        <MetricPanel
          foot="Needs operator review"
          icon={<AlertTriangle size={20} aria-hidden="true" />}
          label="Validation failures"
          value={overview.validationFailures.toLocaleString()}
        />
        <MetricPanel
          foot={`Generated ${lastGenerated}`}
          icon={<Gauge size={20} aria-hidden="true" />}
          label="Tenant"
          value={overview.tenantId}
        />
      </section>

      <section className="activity-panel" aria-label="Recent activity">
        <div className="panel-heading">
          <h2>Recent activity</h2>
          <StatusPill tone="neutral">{overview.recentActivity.length} events</StatusPill>
        </div>
        <div className="activity-list">
          {overview.recentActivity.map((item) => (
            <ActivityRow item={item} key={item.id} />
          ))}
        </div>
      </section>
    </div>
  );
}

function ActivityRow({ item }: { readonly item: RecentActivityItem }) {
  const occurredAt = new Intl.DateTimeFormat("en", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(item.occurredAt));

  return (
    <article className="activity-row" data-kind={item.kind}>
      <span className="activity-icon">{activityIcon(item.kind)}</span>
      <div>
        <div className="activity-title">
          <span>{item.documentId}</span>
          <StatusPill tone={item.kind === "failed" ? "critical" : "good"}>{activityLabel(item.kind)}</StatusPill>
        </div>
        <p className="activity-summary">
          {item.summary} <span className="muted">{item.counterparty}</span>
        </p>
      </div>
      <div className="activity-meta">
        <span>{occurredAt}</span>
        <span>{item.traceId}</span>
      </div>
    </article>
  );
}

function activityIcon(kind: ActivityKind) {
  switch (kind) {
    case "archived":
      return <Archive size={18} aria-hidden="true" />;
    case "failed":
      return <AlertTriangle size={18} aria-hidden="true" />;
    case "sent":
      return <Send size={18} aria-hidden="true" />;
    case "validated":
      return <CheckCircle2 size={18} aria-hidden="true" />;
  }
}

function activityLabel(kind: ActivityKind): string {
  switch (kind) {
    case "archived":
      return "archived";
    case "failed":
      return "failed";
    case "sent":
      return "sent";
    case "validated":
      return "validated";
  }
}
