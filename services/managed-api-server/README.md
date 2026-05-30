# invoicekit-managed-api-server

The managed-layer API gateway for InvoiceKit: an `axum` `Router` builder with
per-tenant API-key authentication and a per-tenant token-bucket rate limiter.
This is the optional hosted-service front end; the open core does not depend on it.

## Capabilities

- `build_router(AppState) -> axum::Router` mounts a `/v1` subrouter. One route
  is wired today: `GET /v1/audit/events`.
- API-key authentication. `parse_bearer_token` extracts the token from an
  `Authorization: Bearer <token>` header; a `TenantApiKeyStore` resolves it to
  an `AuthenticatedTenant` (`TenantId`, `ApiKeyId`, `Actor::ApiKey`). Missing,
  malformed, or unknown tokens map to HTTP 401.
- `InMemoryTenantApiKeyStore` — a `Mutex<HashMap>` token store for tests and
  lightweight deployments. `insert` / `resolve` / `len` / `is_empty`.
- `TokenBucketRateLimiter` — a synchronous `Mutex<HashMap<(TenantId, route), bucket>>`
  token bucket. `take` returns the remaining token count or `RateLimited { retry_after_ms }`.
  `RateLimitPolicy` carries `capacity` (burst) and `refill_per_second`. Per-route
  overrides via `with_route_override`; `take_at` accepts a caller-supplied
  `Instant` for deterministic tests. The default policy is capacity 30, refill
  6.0/sec. Exhausted buckets map to HTTP 429 with a `Retry-After` header
  (seconds, ceil of the millisecond value, minimum 1).
- A stable JSON error envelope (`ApiErrorBody` / `ApiErrorInner`) with a
  `code` string (`unauthorized`, `rate_limited`, `bad_request`, `forbidden`,
  `internal_error`) and a `message`. `ApiError` maps `AuditQueryError` variants
  onto 403 / 400 / 500 as appropriate.
- The `GET /v1/audit/events` handler authenticates, rate-limits, builds a
  `TenantRequestContext`, and delegates to `handle_audit_query` from
  `invoicekit-managed-api`. The query string accepts `page_size` and `cursor`.
- A standalone dispatcher in `main.rs`: `handle_request(ManagedApiServerRequest)`
  runs an `invoicekit-managed-api` request observation (span, metric, redacted
  log), emits it, and returns the status code and W3C `traceparent`.

## Mode / Residuals

- **The binary does not serve HTTP.** `main()` is the workspace no-arg identity
  handshake; it returns silently and does not bind a TCP listener. To serve the
  router you must call `build_router` and wire it to a listener yourself. The
  library doc-comment's claim that "the binary wires to a TCP listener" is not
  realized in this crate.
- **Auth, rate-limit, and tracing are not tower middleware layers.** Despite the
  library doc-comment describing a stack of "three middlewares before any
  handler," the route handler calls `authenticate` and `rate_limit_check`
  inline. There is no shared tower layer, and no `X-Invoicekit-Trace-Id`
  response header is emitted by the router. A `traceparent` is produced only by
  the separate `handle_request` dispatcher in `main.rs`, which the router does
  not call.
- **One live route.** `/v1/reconcile`, `/v1/events/sse`, and `/v1/capabilities`
  are named as future work in the doc-comment but are not implemented. The
  `AuditQuery` filter axes (action, outcome, target kind, target id, since,
  until) are hardcoded to `None`; only `page_size` and `cursor` are wired.
- **In-memory stores only.** Both `InMemoryTenantApiKeyStore` and the rate
  limiter hold state in process memory. The `TenantApiKeyStore` trait and the
  `Arc<dyn ...>` fields in `AppState` exist so a SQL-backed store can be swapped
  in later, but no such store is provided here. The audit store comes from
  `invoicekit-managed-api`.
- **No OIDC / scoped permissions.** Only opaque bearer tokens resolving to
  `Actor::ApiKey` are handled. `Actor::Principal` and scope enforcement are
  deferred to a future unified token validator.
- **`invoicekit-reconcile` is a declared dependency but unused** by the library
  (it backs the deferred reconcile route).
- The rate limiter is single-process and not durable; the doc-comment notes a
  Redis-backed implementation as a future swap.

## References

- W3C Trace Context `traceparent` — produced by the `main.rs` dispatcher (no
  URL in source).
- `Retry-After` header on 429 — the doc-comment notes this matches the shape
  Stripe and GitHub return (no URL in source).
- Bead and plan identifiers in source: T-134 (this gateway), T-142 (audit
  events), T-136 (request observation), and `plans/PLAN.md`. T-075, T-077, and
  T-006a are named as future routes.

## License

Apache-2.0
