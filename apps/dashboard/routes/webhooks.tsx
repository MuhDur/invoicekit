import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { RefreshCw, Webhook } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type DashboardEngineClient,
  type WebhookEndpoint,
  type WebhookEndpointPage,
  type WebhookStatus
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function webhooksQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["webhooks", { limit: 50 }],
    queryFn: () => client.listWebhooks({ limit: 50 })
  });
}

export function WebhooksRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(webhooksQueryOptions(dashboardClient));

  return (
    <WebhooksContent
      webhooks={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      }}
    />
  );
}

interface WebhooksContentProps {
  readonly onRefresh?: () => void;
  readonly webhooks: WebhookEndpointPage;
}

export function WebhooksContent({ onRefresh, webhooks }: WebhooksContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Settings</p>
          <h1>Webhooks</h1>
          <p className="muted">Outbound endpoint URLs, event subscriptions, signing prefixes, and delivery health.</p>
        </div>
        <Button aria-label="Refresh webhooks" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Tenant webhooks">
        <div className="panel-heading">
          <div>
            <h2>Endpoints</h2>
            <p className="muted">{webhooks.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={webhooks.pageInfo.hasNextPage ? "warning" : "good"}>
            {webhooks.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Endpoint</th>
                <th scope="col">Status</th>
                <th scope="col">Events</th>
                <th scope="col">Signing prefix</th>
                <th scope="col">Last delivery</th>
                <th scope="col">Failures</th>
              </tr>
            </thead>
            <tbody>
              {webhooks.items.length > 0 ? (
                webhooks.items.map((webhook) => <WebhookRow key={webhook.id} webhook={webhook} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={6}>
                    No webhooks yet.
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

function WebhookRow({ webhook }: { readonly webhook: WebhookEndpoint }) {
  return (
    <tr>
      <td>
        <div className="document-cell">
          <Webhook size={18} aria-hidden="true" />
          <span>
            <strong>{webhook.name}</strong>
            <small>{webhook.url}</small>
          </span>
        </div>
      </td>
      <td>
        <StatusPill tone={statusTone(webhook.status)}>{statusLabel(webhook.status)}</StatusPill>
      </td>
      <td>{webhook.eventTypes.join(", ")}</td>
      <td>{webhook.signingSecretPrefix}</td>
      <td>{formatOptionalDate(webhook.lastDeliveredAt, "Never delivered")}</td>
      <td>{webhook.failureCount.toLocaleString()}</td>
    </tr>
  );
}

function statusLabel(status: WebhookStatus): string {
  switch (status) {
    case "active":
      return "Active";
    case "disabled":
      return "Disabled";
    case "failing":
      return "Failing";
  }
}

function statusTone(status: WebhookStatus): "critical" | "good" | "neutral" {
  switch (status) {
    case "active":
      return "good";
    case "disabled":
      return "neutral";
    case "failing":
      return "critical";
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
