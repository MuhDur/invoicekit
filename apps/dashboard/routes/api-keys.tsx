import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { KeyRound, RefreshCw } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type ApiKey,
  type ApiKeyPage,
  type ApiKeyStatus,
  type DashboardEngineClient
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function apiKeysQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["api-keys", { limit: 50 }],
    queryFn: () => client.listApiKeys({ limit: 50 })
  });
}

export function ApiKeysRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(apiKeysQueryOptions(dashboardClient));

  return (
    <ApiKeysContent
      apiKeys={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
      }}
    />
  );
}

interface ApiKeysContentProps {
  readonly apiKeys: ApiKeyPage;
  readonly onRefresh?: () => void;
}

export function ApiKeysContent({ apiKeys, onRefresh }: ApiKeysContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Settings</p>
          <h1>API keys</h1>
          <p className="muted">Scoped tokens, key prefixes, status, and usage timestamps.</p>
        </div>
        <Button aria-label="Refresh API keys" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Tenant API keys">
        <div className="panel-heading">
          <div>
            <h2>Keys</h2>
            <p className="muted">{apiKeys.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={apiKeys.pageInfo.hasNextPage ? "warning" : "good"}>
            {apiKeys.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Key</th>
                <th scope="col">Status</th>
                <th scope="col">Scopes</th>
                <th scope="col">Created</th>
                <th scope="col">Last used</th>
                <th scope="col">Expires</th>
              </tr>
            </thead>
            <tbody>
              {apiKeys.items.length > 0 ? (
                apiKeys.items.map((apiKey) => <ApiKeyRow apiKey={apiKey} key={apiKey.id} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={6}>
                    No API keys yet.
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

function ApiKeyRow({ apiKey }: { readonly apiKey: ApiKey }) {
  return (
    <tr>
      <td>
        <div className="document-cell">
          <KeyRound size={18} aria-hidden="true" />
          <span>
            <strong>{apiKey.name}</strong>
            <small>{apiKey.prefix}</small>
          </span>
        </div>
      </td>
      <td>
        <StatusPill tone={statusTone(apiKey.status)}>{statusLabel(apiKey.status)}</StatusPill>
      </td>
      <td>{apiKey.scopes.join(", ")}</td>
      <td>{formatDate(apiKey.createdAt)}</td>
      <td>{formatOptionalDate(apiKey.lastUsedAt, "Never used")}</td>
      <td>{formatOptionalDate(apiKey.expiresAt, "No expiry")}</td>
    </tr>
  );
}

function statusLabel(status: ApiKeyStatus): string {
  switch (status) {
    case "active":
      return "Active";
    case "revoked":
      return "Revoked";
  }
}

function statusTone(status: ApiKeyStatus): "critical" | "good" {
  switch (status) {
    case "active":
      return "good";
    case "revoked":
      return "critical";
  }
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(value));
}

function formatOptionalDate(value: string | undefined, fallback: string): string {
  if (value === undefined) {
    return fallback;
  }

  return formatDate(value);
}
