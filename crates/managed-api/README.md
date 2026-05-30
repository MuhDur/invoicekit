<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-managed-api

The deterministic tenant, authentication, authorization, audit, and observability contract for the hosted managed layer. Types only — no HTTP server, no database, no token exchange.

This crate owns the shared model that the future managed-layer services depend on so tenant identity and audit evidence do not drift across them. The module doc-comment is explicit that HTTP routing, token exchange, database persistence, dashboards, and deployment artefacts are left to later Track 11 and Track 13 crates; this crate is the contract, not the running service.

## Capabilities

Identity and request context:

- Validated identifier newtypes — `TenantId`, `PrincipalId`, `ApiKeyId`, `TraceId`, `AuditEventId`. Each is a `#[serde(transparent)]` string with a checked constructor: 1-128 ASCII bytes from `A-Z a-z 0-9 _ . : -`, no surrounding whitespace.
- `TenantRequestContext` — per-request tenant, trace id, and `Actor` (`ApiKey`, `Principal`, or `System`). `require_same_tenant` rejects any `TenantScoped` value that belongs to a different tenant with `ManagedApiError::TenantMismatch`. `TenantScoped` is the trait that makes that check uniform.

Authorization:

- `ApiScope` — explicit API-key scopes (`tenant:admin`, `invoice:read`, `invoice:write`, `invoice:validate`, `invoice:render`, `invoice:transmit`, `archive:read`, `audit:read`). `permits(Permission)` maps a scope to a permission; `tenant:admin` permits everything.
- `ApiKeyRecord` — tenant-owned key with a name, a stored `ApiKeySecretDigest` (algorithm + encoded digest; the raw secret is never stored here), a non-secret preview, an explicit scope set (at least one required), an `ApiKeyStatus` (`Active` / `Revoked`), and a creation timestamp. `allows_scope` / `require_scope` enforce that a revoked key grants nothing and that `tenant:admin` widens to any scope.
- `Role` (`Admin`, `Member`, `Viewer`), `Permission`, and `Membership` — role-based access control. `Role::allows` / `require_permission` define a fixed permission table; `Membership` binds a principal to one role inside a tenant.

OpenID Connect (Google), claim-validation only:

- `GoogleOidcConfig` — typed client config. `new` requires a non-empty client id and an HTTPS redirect URI. `authorization_url` builds an Authorization Code + PKCE (`code_challenge_method=S256`) request URL with its own percent-encoder. `discovery_document_uri()` returns Google's fixed discovery URL.
- `accept_verified_claims` — validates tenant-model invariants over `GoogleIdTokenClaims` (issuer, audience contains the client id, expiry, issued-at skew of at most 300 seconds, verified email, non-empty subject, optional hosted-domain match) and returns an `OidcIdentity`. See Mode below — this does not verify the token signature.

Audit:

- `AuditEvent` (with `AuditEventId`, `AuditAction`, `AuditOutcome`, `AuditTarget`, RFC 3339 timestamp, and a non-secret string `metadata` map), built from a `TenantRequestContext` so every event carries tenant, trace id, and actor. `audit_event_json_schema()` returns the hand-authored JSON Schema (Draft 2020-12) for the public event contract.
- `audit_log` module — a framework-free customer-facing query API. `AuditEventStore` trait (with `InMemoryAuditStore`), `AuditQuery` filters (action, outcome, target kind/id, RFC 3339 `since`/`until`, opaque cursor pagination), and `handle_audit_query`, which enforces tenant isolation (cross-tenant queries fail with `AuditQueryError::CrossTenantRequest`) and clamps page size to `[1, MAX_PAGE_SIZE]`.
- Signed export — `signed_export` renders a page as line-delimited JSON or CSV and attaches a real hex HMAC-SHA256 (`hmac` + `sha2`) over the body bytes; `verify_signed_export` recomputes and compares it in constant time (`subtle::ConstantTimeEq`). `SIGNATURE_ALG` is `hmac-sha256`. The caller supplies the signing key bytes.

Server-Sent Events ACK stream (`sse_ack` module):

- `AckEvent`, `SseFrame`, the `AckEventStream` trait (with `InMemoryAckStream`), `validate_subscription` (cross-tenant rejection plus cursor check), `build_reconnect_payload`, `format_event_frame`, and `parse_last_event_id`. Framework-free: it hands the future HTTP layer SSE-spec-shaped frames (`id:` / `event:` / `data:`, keep-alive comment frames) with `Last-Event-ID` reconnect replay. It produces frames; it does not open or hold a connection.

Observability (`observability` module):

- `OpenTelemetryIds` — W3C trace/span/parent-span/flags with strict validation (32 and 16 lowercase hex characters, not all-zero) and a `traceparent()` header renderer.
- `SloOperation`, `TelemetryOutcome`, `ObservedRequestSpan`, `SloMetricEvent`, and `ManagedRequestObservation` — one `TenantRequestContext::observe_request` call emits a span, an SLO metric, and a redacted log together so the three channels cannot drift. `to_otel_json` renders deterministic OpenTelemetry-shaped JSON; `emit_tracing_log` emits through the `tracing` facade.
- `GatewayDashboardSnapshot::from_metrics` — deterministic per-gateway aggregation over a half-open RFC 3339 window: counts, availability and gateway-acceptance in parts-per-million, p95 latency by nearest-rank, and failure counts by kind.
- `redact_log_value` / `LOG_REDACTION_PLACEHOLDER` — recursive log redaction that preserves object/array shape while masking values under known personal-data or secret keys, and scalar strings that look like bearer/basic auth, common key prefixes, JWTs, or email addresses. See Mode for its scope.

## Mode / Residuals

This crate is a **type and policy contract**. It performs no network input/output, opens no database, runs no HTTP server, and exchanges no tokens. The in-memory stores (`InMemoryAuditStore`, `InMemoryAckStream`) are for tests and lightweight deployments; production storage is a future backend behind the same traits.

- **OIDC token signatures are not verified here.** `accept_verified_claims` is documented and named to accept claims *after* a gateway has already verified the JWT signature against Google's `jwks_uri`. It checks issuer, audience, expiry, issued-at skew, verified email, subject, and hosted domain — claim-shape invariants only. There is no JWKS fetch, no signature check, and no PKCE-verifier exchange in this crate. A real login path needs the signature-verifying gateway in front of this function.
- **The signed-export HMAC is real, but key management is not.** Export signing and verification use a genuine keyed HMAC-SHA256 with constant-time comparison — this is not a placeholder MAC. However, the signing key is caller-supplied raw bytes; this crate holds no key store, no rotation, and no hardware-security-module or key-management-service path. It is a message-authentication code for export-tamper detection, not a public-verifiable digital signature.
- **The API-key digest is a descriptor, not a hasher.** `ApiKeySecretDigest` records an algorithm name and an encoded digest string. This crate does not hash secrets, compare them, or authenticate a presented key against a digest; it only carries the stored descriptor.
- **Log redaction is a heuristic denylist.** `redact_log_value` matches against a fixed set of sensitive key names (plus substring and suffix rules) and a fixed set of secret-looking scalar shapes. It is biased toward over-redaction and is not a complete personal-data detector; producers are still responsible for keeping secrets out of `metadata` maps.
- **Timestamps are compared lexically.** `since`/`until` audit filters and the dashboard window rely on normalized RFC 3339 strings (for example UTC `Z`) so lexical order matches chronological order; the crate does not parse calendars.
- Audit cursors are opaque hex offsets over the current in-memory ordering; an out-of-range cursor yields an empty final page rather than a panic.

## References

- W3C Trace Context — `traceparent` and trace/span identifier format (`OpenTelemetryIds`).
- OpenTelemetry — span and metric attribute shapes emitted by `to_otel_json`.
- OpenID Connect / Google OpenID Connect — discovery document `https://accounts.google.com/.well-known/openid-configuration`, issuer `https://accounts.google.com`, authorization endpoint `https://accounts.google.com/o/oauth2/v2/auth`, and the `openid email profile` scope used by `authorization_url`.
- OAuth 2.0 Authorization Code with PKCE (`code_challenge_method=S256`).
- RFC 3339 — timestamp format used for event and metric times.
- HMAC-SHA256 — export signature scheme (`SIGNATURE_ALG = "hmac-sha256"`), via the `hmac` and `sha2` crates with `subtle` constant-time comparison.
- JSON Schema 2020-12 — `$schema` of `audit_event_json_schema()`.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
