import { Link, Outlet, createRootRoute, createRoute, createRouter, useRouterState } from "@tanstack/react-router";
import { Suspense } from "react";
import {
  Activity,
  AlertTriangle,
  BarChart3,
  CreditCard,
  Gauge,
  Inbox,
  KeyRound,
  Settings,
  Users,
  Webhook
} from "lucide-react";
import { ApiKeysRoute } from "../routes/api-keys";
import { AuditRoute } from "../routes/audit";
import { BillingRoute } from "../routes/billing";
import { DocumentsRoute } from "../routes/documents";
import { ErrorsRoute } from "../routes/errors";
import { OverviewRoute } from "../routes/overview";
import { TeamRoute } from "../routes/team";
import { UsageRoute } from "../routes/usage";
import { WebhooksRoute } from "../routes/webhooks";

function AppShell() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const auditActive = pathname.startsWith("/audit");
  const overviewActive = pathname === "/" || pathname === "/overview";
  const documentsActive = pathname.startsWith("/documents");
  const usageActive = pathname.startsWith("/usage");
  const errorsActive = pathname.startsWith("/errors");
  const teamActive = pathname.startsWith("/settings/team");
  const apiKeysActive = pathname.startsWith("/settings/api-keys");
  const webhooksActive = pathname.startsWith("/settings/webhooks");
  const billingActive = pathname.startsWith("/settings/billing");

  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="Dashboard navigation">
        <div className="brand">
          <span className="brand-mark">IK</span>
          <span>
            <strong>InvoiceKit</strong>
            <small>Managed console</small>
          </span>
        </div>
        <nav className="nav-list">
          <Link className="nav-item" data-active={overviewActive ? true : undefined} to="/overview">
            <Gauge size={18} aria-hidden="true" />
            Overview
          </Link>
          <Link className="nav-item" data-active={documentsActive ? true : undefined} to="/documents">
            <Inbox size={18} aria-hidden="true" />
            Documents
          </Link>
          <Link className="nav-item" data-active={auditActive ? true : undefined} to="/audit">
            <Activity size={18} aria-hidden="true" />
            Audit
          </Link>
          <Link className="nav-item" data-active={usageActive ? true : undefined} to="/usage">
            <BarChart3 size={18} aria-hidden="true" />
            Usage
          </Link>
          <Link className="nav-item" data-active={errorsActive ? true : undefined} to="/errors">
            <AlertTriangle size={18} aria-hidden="true" />
            Errors
          </Link>
          <Link className="nav-item" data-active={teamActive ? true : undefined} to="/settings/team">
            <Users size={18} aria-hidden="true" />
            Team
          </Link>
          <Link className="nav-item" data-active={apiKeysActive ? true : undefined} to="/settings/api-keys">
            <KeyRound size={18} aria-hidden="true" />
            API keys
          </Link>
          <Link className="nav-item" data-active={webhooksActive ? true : undefined} to="/settings/webhooks">
            <Webhook size={18} aria-hidden="true" />
            Webhooks
          </Link>
          <Link className="nav-item" data-active={billingActive ? true : undefined} to="/settings/billing">
            <CreditCard size={18} aria-hidden="true" />
            Billing
          </Link>
          <span className="nav-item nav-item-disabled">
            <Settings size={18} aria-hidden="true" />
            Settings
          </span>
        </nav>
      </aside>
      <main className="main-pane">
        <Suspense fallback={<div className="loading-panel">Loading overview</div>}>
          <Outlet />
        </Suspense>
      </main>
    </div>
  );
}

const rootRoute = createRootRoute({
  component: AppShell,
  errorComponent: DashboardError
});

const overviewIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: OverviewRoute
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/overview",
  component: OverviewRoute
});

const documentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/documents",
  component: DocumentsRoute
});

const auditRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/audit",
  component: AuditRoute
});

const usageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/usage",
  component: UsageRoute
});

const errorsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/errors",
  component: ErrorsRoute
});

const teamRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/team",
  component: TeamRoute
});

const apiKeysRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/api-keys",
  component: ApiKeysRoute
});

const webhooksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/webhooks",
  component: WebhooksRoute
});

const billingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/billing",
  component: BillingRoute
});

const routeTree = rootRoute.addChildren([
  overviewIndexRoute,
  overviewRoute,
  documentsRoute,
  auditRoute,
  usageRoute,
  errorsRoute,
  teamRoute,
  apiKeysRoute,
  webhooksRoute,
  billingRoute
]);

export const router = createRouter({
  defaultPreload: "intent",
  routeTree
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

function DashboardError({ error }: { readonly error: unknown }) {
  const message = error instanceof Error ? error.message : "Unknown dashboard error";

  return (
    <div className="error-panel" role="alert">
      <p className="eyebrow">Dashboard error</p>
      <h1>Dashboard unavailable</h1>
      <p className="muted">{message}</p>
      <pre>{JSON.stringify({ route: "dashboard", message }, null, 2)}</pre>
    </div>
  );
}
