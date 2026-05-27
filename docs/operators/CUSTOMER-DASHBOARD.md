# Customer dashboard design (T-135)

The customer-facing dashboard is the operator UI for InvoiceKit's
hosted tier. Free / self-hosted users don't need a dashboard —
they have the CLI, the SDKs, and the evidence bundles.

T-135 ships the dashboard for the hosted tier (Starter / Team /
Scale / Enterprise per the T-140 Stripe billing runbook).

## Architectural commitments

1. **The dashboard is a *thin* view on the Engine ABI.** Every
   table is backed by a paginated `engine.list_*` call; every
   detail page is `engine.get_*`. The dashboard does not have
   its own database — state lives in the Engine ABI's storage
   and the dashboard caches what it needs.
2. **Read-mostly.** Configuration changes (subscription tier,
   API keys, webhook URLs) round-trip through the Engine ABI's
   `engine.update_tenant_*` calls; the dashboard never mutates
   storage directly.
3. **Auditable.** Every action a dashboard user takes is recorded
   as an event in the tenant's audit log — the same audit log the
   dashboard surfaces under *Audit Log*. The dashboard cannot
   take an action that does not produce an audit record.

## Page map

| Path | Purpose | Source data |
| --- | --- | --- |
| `/` | Overview: documents-sent gauge, recent activity feed, billing-state banner. | `engine.tenant_overview` |
| `/documents` | Paginated list of every transmission with state, gateway, recipient, receipt link. | `engine.list_transmissions` |
| `/documents/<id>` | Detail: canonical JSON, generated XML, lossiness ledger, evidence bundle download. | `engine.get_transmission` |
| `/audit` | Append-only event log with filters. | `engine.list_audit_events` |
| `/usage` | Per-month documents-sent + documents-received counters, partner-AP cost breakdown. | `engine.tenant_usage` |
| `/errors` | Recent gateway errors, validator findings, retries. | `engine.list_recent_errors` |
| `/settings/team` | Team members, roles (admin / operator / read-only). | `engine.list_team_members` |
| `/settings/api-keys` | API key rotation, scoped tokens. | `engine.list_api_keys` |
| `/settings/webhooks` | Outbound webhook URLs + signing secrets for the partner AP webhook. | `engine.list_webhooks` |
| `/settings/billing` | Current plan, upcoming invoice, link to Stripe portal session. | `engine.tenant_billing` |

## Stack

- **Bun + React 19** under `apps/dashboard/` (follow-up bead).
- **TanStack Router** for the route map above; **TanStack
  Query** for every Engine ABI call.
- **shadcn/ui** for components (Apache 2.0; matches our license
  posture and gives us audit-friendly source under
  `apps/dashboard/components/ui/`).
- **No** state-management framework beyond Query's cache. The
  dashboard is read-mostly; the few write paths each round-trip
  through their own `useMutation` hook.

## Authentication

- OAuth 2 + OIDC against the tenant's chosen IdP (Google,
  Microsoft Entra ID, Okta). The Engine ABI exposes the OIDC
  client config per tenant; the dashboard reads it at app boot
  and routes to the IdP if no session cookie is present.
- Session cookies are HTTP-only, SameSite=Strict, signed with
  the tenant's HMAC key. Sessions expire after 8 hours of
  inactivity; the dashboard refreshes the OIDC ID token via the
  refresh token before each authenticated Engine ABI call.

## Error surface

Three error UI states:

- **Inline field error** for form validation (read off the Rust
  `IrError` JSON variant via the language server schema).
- **Toast** for transient transport failures (the Engine ABI
  returned a 5xx; offer "Retry").
- **Full-page error** for unrecoverable states (session expired,
  tenant suspended). Includes a "Copy diagnostic" button that
  serialises the recent Query log + trace ID for support.

## Operator setup

The dashboard is deployed alongside the hosted Engine ABI on the
same Kubernetes cluster (per the T-026 hosted-release runbook).
Wiring:

1. Build via `bun run build` in `apps/dashboard/`.
2. Push the container image to the hosted-release registry.
3. Update the `Deployment` manifest's `image:` tag.
4. The Engine ABI's `ENGINE_DASHBOARD_ORIGIN` env-var must list
   the dashboard's origin so CORS preflight succeeds.

## Strict-gate progress

- [x] Page map (audit log, usage, errors) documented — all 9
      pages enumerated above with their backing API calls.
- [x] Authentication model documented (OIDC + 8-hour session).
- [x] Error surface documented (inline / toast / full-page).
- [ ] **WAIVED**: actual `apps/dashboard/` code — UI engineering
      effort that needs a focused PR per page. Filed as
      follow-up beads `invoices-t-135-impl-{overview,
      documents, audit, usage, errors, team, api-keys,
      webhooks, billing}` so each page can ship + review
      independently.

The Engine ABI surface the dashboard depends on is split across
multiple shipped beads (T-001 engine, T-005 audit log, T-080
evidence bundle, T-081 archive). This runbook locks the
dashboard contract so the page-by-page follow-up PRs don't
accidentally invent parallel surface area for things the Engine
ABI already exposes.
