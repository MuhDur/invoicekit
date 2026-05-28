import { queryOptions, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { Download, ExternalLink, FileText, RefreshCw, ReceiptText } from "lucide-react";
import { Button } from "../components/ui/button";
import { StatusPill } from "../components/ui/status-pill";
import {
  createHttpDashboardClient,
  type DashboardEngineClient,
  type TransmissionPage,
  type TransmissionState,
  type TransmissionSummary
} from "../src/engine";

const dashboardClient = createHttpDashboardClient();

export function transmissionsQueryOptions(client: DashboardEngineClient) {
  return queryOptions({
    queryKey: ["transmissions", { limit: 25 }],
    queryFn: () => client.listTransmissions({ limit: 25 })
  });
}

export function DocumentsRoute() {
  const queryClient = useQueryClient();
  const { data } = useSuspenseQuery(transmissionsQueryOptions(dashboardClient));

  return (
    <DocumentsContent
      transmissions={data}
      onRefresh={() => {
        void queryClient.invalidateQueries({ queryKey: ["transmissions"] });
      }}
    />
  );
}

interface DocumentsContentProps {
  readonly transmissions: TransmissionPage;
  readonly onRefresh?: () => void;
}

export function DocumentsContent({ onRefresh, transmissions }: DocumentsContentProps) {
  return (
    <div className="documents-page">
      <header className="overview-header">
        <div>
          <p className="eyebrow">Documents</p>
          <h1>Transmissions</h1>
          <p className="muted">Gateway state, recipient, receipts, and evidence bundle access.</p>
        </div>
        <Button aria-label="Refresh transmissions" onClick={onRefresh}>
          <RefreshCw size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <section className="table-panel" aria-label="Document transmissions">
        <div className="panel-heading">
          <div>
            <h2>Recent transmissions</h2>
            <p className="muted">{transmissions.pageInfo.limit} per page</p>
          </div>
          <StatusPill tone={transmissions.pageInfo.hasNextPage ? "warning" : "good"}>
            {transmissions.pageInfo.hasNextPage ? "more available" : "current page"}
          </StatusPill>
        </div>
        <div className="table-scroll">
          <table className="documents-table">
            <thead>
              <tr>
                <th scope="col">Document</th>
                <th scope="col">State</th>
                <th scope="col">Gateway</th>
                <th scope="col">Recipient</th>
                <th scope="col">Amount</th>
                <th scope="col">Updated</th>
                <th scope="col">Artifacts</th>
              </tr>
            </thead>
            <tbody>
              {transmissions.items.length > 0 ? (
                transmissions.items.map((item) => <TransmissionRow item={item} key={item.id} />)
              ) : (
                <tr>
                  <td className="empty-table-cell" colSpan={7}>
                    No transmissions yet.
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

function TransmissionRow({ item }: { readonly item: TransmissionSummary }) {
  const updatedAt = new Intl.DateTimeFormat("en", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(item.updatedAt));

  return (
    <tr>
      <td>
        <div className="document-cell">
          <FileText size={18} aria-hidden="true" />
          <span>
            <strong>{item.documentId}</strong>
            <small>{item.issueDate}</small>
          </span>
        </div>
      </td>
      <td>
        <StatusPill tone={transmissionTone(item.state)}>{item.state}</StatusPill>
      </td>
      <td>{item.gateway}</td>
      <td>
        <span className="recipient-cell">
          {item.recipient}
          <small>{item.recipientCountry}</small>
        </span>
      </td>
      <td>
        {item.amount} {item.currency}
      </td>
      <td>{updatedAt}</td>
      <td>
        <div className="artifact-links">
          {item.receiptUrl ? (
            <a href={item.receiptUrl}>
              <ReceiptText size={16} aria-hidden="true" />
              Receipt
            </a>
          ) : null}
          {item.evidenceBundleUrl ? (
            <a href={item.evidenceBundleUrl}>
              <Download size={16} aria-hidden="true" />
              Bundle
            </a>
          ) : null}
          {!item.receiptUrl && !item.evidenceBundleUrl ? (
            <span className="muted">
              <ExternalLink size={16} aria-hidden="true" />
              Pending
            </span>
          ) : null}
        </div>
      </td>
    </tr>
  );
}

function transmissionTone(state: TransmissionState): "critical" | "good" | "neutral" | "warning" {
  switch (state) {
    case "accepted":
    case "archived":
      return "good";
    case "failed":
    case "rejected":
      return "critical";
    case "queued":
    case "sent":
    case "validating":
      return "warning";
  }
}
