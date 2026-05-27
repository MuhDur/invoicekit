import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { CalendarDays, CreditCard, ExternalLink, FileCheck2, RefreshCw, ReceiptText } from "lucide-react";
import { Button } from "../components/ui/button";
import { MetricPanel } from "../components/ui/metric-panel";
import { StatusPill } from "../components/ui/status-pill";
import {
  billingTone,
  createHttpDashboardClient,
  type BillingState,
  type DashboardEngineClient,
  type TenantBilling,
  type UpcomingInvoice
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function tenantBillingQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["tenant-billing"],
    queryFn: () => client.tenantBilling()
  });
}

export function BillingRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(tenantBillingQueryOptions(dashboardClient));

  return (
    <BillingContent
      billing={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["tenant-billing"] });
      }}
    />
  );
}

interface BillingContentProps {
  readonly billing: TenantBilling;
  readonly onRefresh?: () => void;
}

export function BillingContent({ billing, onRefresh }: BillingContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Settings</p>
          <h1>Billing</h1>
          <p className="muted">Plan status, usage allowance, billing email, and upcoming Stripe invoice.</p>
        </div>
        <div className="artifact-links">
          {billing.portalSessionUrl ? (
            <a className="button" href={billing.portalSessionUrl}>
              <ExternalLink size={16} aria-hidden="true" />
              Billing portal
            </a>
          ) : null}
          <Button aria-label="Refresh billing" onClick={onRefresh}>
            <RefreshCw size={16} aria-hidden="true" />
            Refresh
          </Button>
        </div>
      </header>

      <section className="metric-grid" aria-label="Billing summary">
        <MetricPanel
          foot={billing.planSlug}
          icon={<CreditCard size={20} aria-hidden="true" />}
          label="Current plan"
          value={billing.planName}
        />
        <MetricPanel
          foot={`${billing.documentsIncluded.toLocaleString()} included`}
          icon={<FileCheck2 size={20} aria-hidden="true" />}
          label="Documents used"
          value={billing.documentsUsed.toLocaleString()}
        />
        <MetricPanel
          foot={billing.currency}
          icon={<ReceiptText size={20} aria-hidden="true" />}
          label="Monthly base"
          value={`${billing.monthlyBasePrice} ${billing.currency}`}
        />
      </section>

      <section className="billing-banner" data-tone={billingTone(billing.state)}>
        <div className="billing-copy">
          <CreditCard size={22} aria-hidden="true" />
          <div>
            <h2>{billingStateLabel(billing.state)}</h2>
            <p className="muted">
              {billing.billingEmail} · Period ends {formatDate(billing.currentPeriodEnd)}
            </p>
          </div>
        </div>
        <StatusPill tone={billingTone(billing.state)}>{billing.state}</StatusPill>
      </section>

      <section className="table-panel" aria-label="Upcoming invoice">
        <div className="panel-heading">
          <div>
            <h2>Upcoming invoice</h2>
            <p className="muted">Stripe-hosted customer billing</p>
          </div>
          <CalendarDays size={20} aria-hidden="true" />
        </div>
        {billing.upcomingInvoice ? <UpcomingInvoicePanel invoice={billing.upcomingInvoice} /> : <NoInvoicePanel />}
      </section>
    </div>
  );
}

function UpcomingInvoicePanel({ invoice }: { readonly invoice: UpcomingInvoice }) {
  return (
    <div className="activity-row">
      <div className="activity-icon">
        <ReceiptText size={18} aria-hidden="true" />
      </div>
      <div>
        <div className="activity-title">
          <span>{invoice.id}</span>
          <StatusPill tone={invoice.status === "open" ? "warning" : "neutral"}>{invoice.status}</StatusPill>
        </div>
        <p className="activity-summary">
          {invoice.amountDue} {invoice.currency} due {formatDate(invoice.dueDate)}
        </p>
      </div>
      <div className="activity-meta">
        <span>{invoice.currency}</span>
      </div>
    </div>
  );
}

function NoInvoicePanel() {
  return (
    <div className="activity-row">
      <div className="activity-icon">
        <ReceiptText size={18} aria-hidden="true" />
      </div>
      <div>
        <div className="activity-title">
          <span>No upcoming invoice</span>
        </div>
        <p className="activity-summary">Stripe has not emitted an upcoming invoice for this tenant.</p>
      </div>
    </div>
  );
}

function billingStateLabel(state: BillingState): string {
  switch (state) {
    case "active":
      return "Subscription active";
    case "past_due":
      return "Payment past due";
    case "suspended":
      return "Tenant suspended";
    case "trial":
      return "Trial active";
  }
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(value));
}
