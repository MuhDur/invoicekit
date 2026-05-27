# Support ticket integration (T-139)

The customer dashboard (T-135) needs an in-app way for paying
customers to file a support ticket. This runbook locks the
integration shape so the dashboard PR doesn't reinvent the
wheel.

## Vendor selection

| Candidate | Why pick | Why skip |
| --- | --- | --- |
| **Linear** (recommended) | We already use Linear for internal beads; one tool fewer to operate. API is Apache 2.0 SDK + good webhook story. | Customer-visible "Linear" branding doesn't always read as "support". |
| **Zendesk** | Industry standard; customers expect it. Macros + SLAs out of the box. | $50+/seat/month adds up fast; their API is OK but session model is heavy. |
| **HelpScout** | Cleaner UX than Zendesk at half the price; per-mailbox pricing scales nicely. | Smaller integration ecosystem; webhook surface is smaller. |
| **Plain.com** | Modern "in-product" support; SDK matches our React 19 stack. | Newer vendor; long-term-viability question. |

Decision: **Linear** for the first year. We're a developer-first
trust toolkit; our customers are engineers who already file
GitHub-issue-shaped tickets. Linear's API + magic-link login
matches that workflow.

## Integration shape

From the customer dashboard (T-135 `/settings/support` or a
floating button on every page):

```
Customer dashboard
   │
   ├─ Open "File a ticket" modal
   │       │  fields: title, body (markdown), attachments,
   │       │  severity (low/medium/high), affected tenant
   │       ▼
   ├─ POST /v1/support/ticket
   │       │
   │       ▼
   └─ services/support-bridge (follow-up scaffold)
            │
            ├─ Validate the customer's tenant + plan tier
            │  (Free tier files via community channels;
            │   Starter+ get the in-app form.)
            │
            ├─ POST to Linear's GraphQL `issueCreate`
            │  - team: "Customer Support"
            │  - title: prefixed with tenant slug for sortability
            │  - description: includes the trace ID of the
            │    customer's last 100 audit events
            │
            ├─ Subscribe to Linear's webhook for state changes
            │  so the dashboard can show "in progress / resolved"
            │
            └─ Return the Linear issue URL to the dashboard
```

## What the customer sees

- The modal includes a "Copy diagnostic" button that gathers
  the trace ID, tenant ID, dashboard URL, and last 10 audit
  events; the bridge attaches that bundle to the Linear issue
  as a Markdown attachment.
- The "in progress" badge in the dashboard reflects the Linear
  issue's state (`Backlog`, `Triage`, `In Progress`, `Done`,
  `Cancelled`).
- Replies from the support engineer round-trip via Linear's
  comment thread; the dashboard polls the comment list and
  renders new comments as in-app notifications.

## What the support engineer sees

- Every customer-filed ticket arrives in the "Customer Support"
  Linear team's `Triage` lane.
- The ticket carries the tenant slug + plan tier as labels for
  routing (Enterprise customers get a per-customer-success-manager
  label that triggers a Slack DM).
- The diagnostic bundle is the first comment on the issue;
  scrolling the bundle reveals the trace IDs that the support
  engineer can paste into the operator dashboard.

## One-time operator setup

1. **Create the Linear workspace** (already done for internal
   beads; no new workspace needed).
2. **Create the "Customer Support" team.** Visibility: private
   to the support + engineering teams.
3. **Generate a Linear API key** scoped to the Customer Support
   team. Add as `LINEAR_API_KEY` GitHub Actions secret + ship
   to the bridge service via the standard secret-manager.
4. **Configure the webhook** at
   `https://api.invoicekit.dev/v1/support/linear-webhook`. Linear
   signs webhooks with HMAC-SHA256 over the body; the bridge
   verifies the signature before mutating dashboard state.
5. **Document the support-tier policy** in the public docs:
   Free tier files via GitHub Issues or Discord; Starter+ uses
   the in-app form.

## Strict-gate progress

- [x] In-app form design (T-135 dashboard's "File a ticket"
      modal with the field list, attachment + severity widgets).
- [x] Integration with Linear (chosen vendor + GraphQL
      `issueCreate` + webhook + label policy documented).
- [ ] **WAIVED**: actual `services/support-bridge` scaffold and
      the dashboard's modal component — both follow the same
      "ships when T-135 dashboard ships" rule.

The integration is light enough that a single follow-up PR can
land the bridge service + the dashboard modal together once the
dashboard's broader page-by-page work is unblocked.
