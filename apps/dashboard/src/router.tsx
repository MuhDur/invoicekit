import { Link, Outlet, createRootRoute, createRoute, createRouter, useRouterState } from "@tanstack/react-router";
import { Suspense } from "react";
import { Activity, Gauge, Inbox, Settings } from "lucide-react";
import { OverviewRoute } from "../routes/overview";

function AppShell() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const overviewActive = pathname === "/" || pathname === "/overview";

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
          <span className="nav-item nav-item-disabled">
            <Inbox size={18} aria-hidden="true" />
            Documents
          </span>
          <span className="nav-item nav-item-disabled">
            <Activity size={18} aria-hidden="true" />
            Activity
          </span>
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

const routeTree = rootRoute.addChildren([overviewIndexRoute, overviewRoute]);

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
      <h1>Overview unavailable</h1>
      <p className="muted">{message}</p>
      <pre>{JSON.stringify({ route: "overview", message }, null, 2)}</pre>
    </div>
  );
}
