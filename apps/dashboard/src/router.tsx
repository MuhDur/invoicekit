import { Link, Outlet, createRootRoute, createRoute, createRouter, useRouterState } from "@tanstack/react-router";
import { Suspense } from "react";
import { Activity, AlertTriangle, BarChart3, Gauge, Inbox, Settings, Users } from "lucide-react";
import { AuditRoute } from "../routes/audit";
import { DocumentsRoute } from "../routes/documents";
import { ErrorsRoute } from "../routes/errors";
import { OverviewRoute } from "../routes/overview";
import { TeamRoute } from "../routes/team";
import { UsageRoute } from "../routes/usage";

function AppShell() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const auditActive = pathname.startsWith("/audit");
  const overviewActive = pathname === "/" || pathname === "/overview";
  const documentsActive = pathname.startsWith("/documents");
  const usageActive = pathname.startsWith("/usage");
  const errorsActive = pathname.startsWith("/errors");
  const teamActive = pathname.startsWith("/settings/team");

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

const routeTree = rootRoute.addChildren([
  overviewIndexRoute,
  overviewRoute,
  documentsRoute,
  auditRoute,
  usageRoute,
  errorsRoute,
  teamRoute
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
