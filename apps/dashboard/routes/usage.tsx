import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { CreditCard, FileCheck2, Inbox, RefreshCw } from "lucide-react";
import { Button } from "../components/ui/button";
import { MetricPanel } from "../components/ui/metric-panel";
import { StatusPill } from "../components/ui/status-pill";
import { createHttpDashboardClient, type DashboardEngineClient, type TenantUsage, type UsageMonth } from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function tenantUsageQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["tenant-usage"],
    queryFn: () => client.tenantUsage()
  });
}

export function UsageRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(tenantUsageQueryOptions(dashboardClient));

  return (
    <UsageContent
      usage={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["tenant-usage"] });
      }}
    />
  );
}

interface UsageContentProps {
  readonly onRefresh?: () => void;
  readonly usage: TenantUsage;
}

export function UsageContent({ onRefresh, usage }: UsageContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Usage</p>
          <h1>{usage.periodLabel}</h1>
          <p className="muted">Documents sent, documents received, and partner access point cost.</p>
        </div>
        <Button aria-label="Refresh tenant usage" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="metric-grid" aria-label="Usage totals">
        <MetricPanel
          foot="Outbound documents"
          icon={<FileCheck2 size={20} aria-hidden="true" />}
          label="Documents sent"
          value={usage.totalSent.toLocaleString()}
        />
        <MetricPanel
          foot="Inbound documents"
          icon={<Inbox size={20} aria-hidden="true" />}
          label="Documents received"
          value={usage.totalReceived.toLocaleString()}
        />
        <MetricPanel
          foot="Partner AP pass-through"
          icon={<CreditCard size={20} aria-hidden="true" />}
          label="Access point cost"
          value={`${usage.partnerApCostTotal} ${usage.currency}`}
        />
      </section>

      <section className="table-panel" aria-label="Monthly usage">
        <div className="panel-heading">
          <div>
            <h2>Monthly breakdown</h2>
            <p className="muted">{usage.months.length} months</p>
          </div>
          <StatusPill tone="neutral">{usage.currency}</StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Month</th>
                <th scope="col">Sent</th>
                <th scope="col">Received</th>
                <th scope="col">Partner AP cost</th>
              </tr>
            </thead>
            <tbody>
              {usage.months.length > 0 ? (
                usage.months.map((month) => <UsageMonthRow currency={usage.currency} key={month.month} month={month} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={4}>
                    No usage recorded yet.
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

function UsageMonthRow({ currency, month }: { readonly currency: string; readonly month: UsageMonth }) {
  return (
    <tr>
      <td>
        <strong>{month.month}</strong>
      </td>
      <td>{month.documentsSent.toLocaleString()}</td>
      <td>{month.documentsReceived.toLocaleString()}</td>
      <td>
        {month.partnerApCost} {currency}
      </td>
    </tr>
  );
}
