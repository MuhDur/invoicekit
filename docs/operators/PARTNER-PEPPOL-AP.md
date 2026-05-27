# Partner Peppol Access Point runbook (T-091)

Per AGENTS.md commitment #7, Year 1 live Peppol delivery rides on a
partner access point (Storecove / ecosio / B2BRouter) rather than
native AS4. This runbook covers vendor selection, env-var contract,
and how to plug into the `GatewayAdapter` trait that
`crates/reconcile` defines.

## Vendor selection (Phase 2.5 decision)

| Candidate | Why pick | Why skip |
| --- | --- | --- |
| **Storecove** | Modern REST API, 30-day trial, sandbox is generous, supports both Peppol BIS 3.0 and PINT, EU + SG + AU + NZ + JP coverage. | Pricing is per-document at low volumes; document-type list is curated, so an exotic profile may need a custom mapping ticket. |
| **ecosio** | Battle-tested in DACH; strong B2B + B2G coverage; the German Leitweg-ID flow is first-class. | Older XML-RPC-flavoured API; the sandbox requires a sales call to provision. |
| **B2BRouter** | Iberia-focused; explicit support for Spain's Verifactu, France's CTC, and Portugal's SAF-T overlay. | Sparse documentation in English; the SOAP layer is harder to wrap from Rust without a code generator. |

Decision criteria, in order:
1. Coverage of the country list at the top of `plans/PLAN.md`
   Section 3 (must hit ≥ 80% of the Year-1 country list).
2. Sandbox that can be provisioned without a sales call (so the
   contract tests in `crates/transmit-mock` keep working without a
   human-in-the-loop).
3. Pricing scales linearly with documents transmitted (the trust
   toolkit positioning rejects per-seat fees).

Default recommendation: **Storecove for the first ship**; ecosio for
DACH-heavy customers as a per-tenant override; B2BRouter as a
follow-up bead once Iberia volume justifies the integration cost.

## Adapter shape

`crates/transmit-peppol-partner` (follow-up scaffold) implements
the `GatewayAdapter` trait declared in `crates/reconcile/src/lib.rs`:

```rust
pub trait GatewayAdapter: Send + Sync {
    fn submit(&self, request: SubmitRequest)   -> GatewayFuture<'_, GatewayReceipt>;
    fn poll(&self, request: PollRequest)       -> GatewayFuture<'_, GatewayReceipt>;
    fn cancel(&self, request: CancelRequest)   -> GatewayFuture<'_, GatewayReceipt>;
    fn correct(&self, request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt>;
}
```

The partner-specific implementation maps each call to the chosen
vendor's REST endpoint, attaches the canonical invoice XML, and
translates the vendor's HTTP response into a `GatewayReceipt`
populated with `gateway_attempt_id` + `tracking_id` + the receipt
PDF (when available).

## Env-var configuration

All credentials live in environment variables (read at process
start, never logged):

| Variable | Purpose |
| --- | --- |
| `INVOICEKIT_PEPPOL_PARTNER` | `storecove` \| `ecosio` \| `b2brouter` — selects the adapter at runtime. |
| `INVOICEKIT_PEPPOL_API_BASE` | Vendor REST endpoint (defaults to the production URL for the chosen vendor; override for sandbox). |
| `INVOICEKIT_PEPPOL_API_KEY` | Long-lived API key OR (preferred) the OIDC client ID for vendors that support it. |
| `INVOICEKIT_PEPPOL_API_SECRET` | Paired secret when the vendor uses HMAC auth. Empty when the vendor accepts a bearer token only. |
| `INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID` | Vendor-assigned ID for the sending legal entity; surfaced in the Peppol SBDH header. |
| `INVOICEKIT_PEPPOL_SANDBOX` | `true` to route through the vendor's sandbox; defaults to `false`. The state-machine guard in `reconcile` refuses sandbox + production-tagged invoices. |

Secret manager integration: when the env-var is not set, the
adapter delegates to a `SecretResolver` trait. The repo ships a
`SecretResolver::Env` (the env-var fallback) and a
`SecretResolver::Stdin` (for interactive operator setup); a future
bead adds `SecretResolver::Vault` and `SecretResolver::Sops`.

## Strict-gate waivers (in this PR)

- **T-074b contract tests** — the contract-test suite isn't yet
  shipped (T-074b is open). The partner adapter ships behind a
  feature flag; CI runs the existing `transmit-mock` contract
  tests against the trait surface so future-T-074b can plug in
  without further trait churn.
- **Sandbox round-trip with a real partner** — out of scope for
  this PR. Operator provisions the sandbox following the per-vendor
  steps below.

## Per-vendor sandbox provisioning

### Storecove (recommended)

1. Sign up at <https://app.storecove.com> with a corporate email.
2. Create a *Legal Entity* under your account; copy its ID.
3. Generate an API key under *Security → API Keys*.
4. Set:
   ```
   export INVOICEKIT_PEPPOL_PARTNER=storecove
   export INVOICEKIT_PEPPOL_API_BASE=https://api.storecove.com/api/v2
   export INVOICEKIT_PEPPOL_API_KEY=<key>
   export INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID=<entity-id>
   export INVOICEKIT_PEPPOL_SANDBOX=true
   ```
5. Send a test invoice via
   `invoicekit transmit --gateway=peppol-partner --target=<recipient-participant>`.
   Watch the document appear in the Storecove admin UI's "Sent"
   tab within ~30 seconds.

### ecosio

1. Email `sales@ecosio.com` requesting a Peppol sandbox account.
   Mention "B2B Peppol BIS Billing 3.0" so they provision the
   right product family.
2. They return a SOAP endpoint + a username + a per-message
   signing certificate. Stash the certificate under
   `~/.config/invoicekit/ecosio-signing.p12` and set:
   ```
   export INVOICEKIT_PEPPOL_PARTNER=ecosio
   export INVOICEKIT_PEPPOL_API_BASE=<sales-provided URL>
   export INVOICEKIT_PEPPOL_API_KEY=<ecosio username>
   export INVOICEKIT_PEPPOL_API_SECRET=<ecosio password>
   export INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID=<your-Peppol participant id>
   export INVOICEKIT_PEPPOL_SANDBOX=true
   ```

### B2BRouter

1. Register at <https://b2brouter.net> as a developer.
2. Activate the *Send & Receive* sandbox sandbox.
3. Set:
   ```
   export INVOICEKIT_PEPPOL_PARTNER=b2brouter
   export INVOICEKIT_PEPPOL_API_BASE=https://app.b2brouter.net/projects/-/api
   export INVOICEKIT_PEPPOL_API_KEY=<token from your account settings>
   export INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID=<your participant id>
   export INVOICEKIT_PEPPOL_SANDBOX=true
   ```

## When to fall back to `phase4`

`phase4` (the Apache reference AS4 stack) remains the in-tree
reference adapter under `crates/transmit-peppol`. Use it when:

- A regulator demands an in-country access point and the vendor
  list above doesn't have one (e.g. when a new market opens that
  none of the three vendors serve).
- A high-volume tenant's marginal cost on the partner crosses the
  break-even point with self-hosting AS4.
- T-074b's contract tests reveal a partner-specific deviation
  that the vendor refuses to fix on their side.

The phase4 adapter is the long-term direction; the partner adapter
buys time while T-074b and the in-tree AS4 work continue.
