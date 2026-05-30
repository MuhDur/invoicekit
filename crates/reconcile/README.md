<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-reconcile

Gateway contracts, transmission lifecycle, durable outbox, webhook delivery, and reconciliation primitives for the InvoiceKit transmission path.

This crate is the stable boundary between InvoiceKit's core and the country/network gateways that actually transmit invoices. It defines the identifiers, request/receipt shapes, state machine, retry policy, and the customer-facing reconcile endpoint that every gateway integration speaks through, instead of letting each gateway invent its own error language.

## Capabilities

### Invoice fingerprint

- `fingerprint(doc) -> blake3::Hash` — deterministic deduplication digest over `blake3(supplier_VAT || customer_VAT || issue_date || document_number || total_amount || currency)`, with a domain tag and per-field length prefixes so adjacent fields cannot collide by concatenation ambiguity. The VAT component is the first `tax_id` whose scheme is `vat` (case-insensitive), or the empty string when absent. `total_amount` is the normalized `payable_amount`. A committed test vector pins the hex output. This is a content fingerprint for dedup and lookup, not a signature or a security token.

### Gateway contract

- Validated newtype identifiers: `TenantId`, `TraceId`, `IdempotencyKey`, `GatewayAttemptId`, `GatewaySubmissionId` — reject blank, whitespace-padded, and control-character values.
- `GatewayContext` — tenant, trace, idempotency, and attempt IDs carried into every gateway call.
- `GatewayRoute` — `route` / `profile` / optional uppercase ISO 3166-1 alpha-2 `country`.
- `GatewayOperation` — `Submit`, `Poll`, `Cancel`, `Correct`.
- Request types `SubmitRequest`, `PollRequest`, `CancelRequest`, `CorrectRequest`. `SubmitRequest` and `CorrectRequest` run `CommercialDocument::validate()` and check that the document's `meta.tenant_id` / `meta.trace_id` match the gateway context.
- `GatewayReceipt` / `GatewayStatus` — normalized gateway response (`Accepted`, `Pending`, `Rejected`, `Cancelled`, `Corrected`).
- `GatewayError` / `GatewayErrorKind` — fourteen normalized failure categories (auth, rate-limit, malformed receipt, maintenance, certificate, duplicate, timeout, network, rejected, not-found, invalid-request, unsupported-operation, partner, unexpected) with message, remediation hint, optional gateway code, submission handle, and `retry_after_seconds`.
- `GatewayAdapter` trait — object-safe `submit` / `poll` / `cancel` / `correct` returning boxed `GatewayFuture`s. The crate defines the trait only; it ships no concrete adapter (partner access point, native protocol, or mock implement it elsewhere).

### Transmission state machine

- `TransmissionBaseState` — `Draft -> Validated -> Signed -> Reserved -> Sent -> (Delivered | Rejected) -> ...`, with `Delivered -> (Acknowledged | Rejected)`, both `Acknowledged` and `Rejected -> Archived`, and `Archived` terminal. Illegal moves are rejected by `can_transition_to`.
- `CountrySubState`, `CountrySubStateTransition`, `CountrySubStateRegistry` — optional country/network sub-states (for example `KSEF`, `SDI`, `ZATCA`) layered on a base move. A system with no configured rules is an open extension point; once it has any rule, sub-state moves into that system must match a configured rule. Shedding a sub-state (an archive with no target sub-state) is always allowed. Duplicate rules are rejected.
- `TransmissionState`, `TransmissionTransition`, `TransmissionStateMachine` — build, validate, apply transitions, and record transition history.

### Durable outbox (`OUTBOX_BEAD_ID`)

- `OutboxState` — the ten persisted states (`draft` … `archived`, plus `dead_letter`); `from_transmission_base` maps the lifecycle state, `is_terminal` flags `delivered`/`acknowledged`/`archived`/`dead_letter`.
- `OutboxEnvelope` — typed pre-persist row in the `Reserved` state; `record_failed_attempt` advances the attempt counter and returns a `RetryDecision`; `to_dead_letter` builds a `DeadLetterRecord` from a `GatewayError`.
- `RetryPolicy` — validated exponential backoff (`max_attempts`, `base_delay_seconds`, `max_delay_seconds`, `jitter_percent`). `delay_for_attempt` is deterministic per `(idempotency_key, attempt)`: jitter is derived from a BLAKE3 hash so tests and replay-from-evidence reproduce the same schedule. Backoff is bounded by `max_delay_seconds`.
- `DatabaseDialect`, `OutboxMigration`, `outbox_migration`, `all_outbox_migrations` — embedded `001_invoicekit_outbox` up/down SQL for Postgres, MySQL, and SQLite (`include_str!` from `migrations/`). `is_idempotent` checks that up SQL guards with `IF NOT EXISTS` and down SQL with `IF EXISTS`.

### Transmission worker (`TRANSMISSION_WORKER_BEAD_ID`)

- `TransmissionWorker` / `TransmissionWorkerConfig` — drains a ready outbox job through a `GatewayAdapter`. `process_once(adapter, now_seconds, job)` takes the clock as a parameter so production uses a real clock and tests pass deterministic timestamps without sleeping.
- `GatewayRateLimit` — minimum seconds between attempts per gateway (`0` disables local spacing).
- `CircuitBreakerPolicy` — opens after N consecutive persistent failures for a configured open window.
- `TransmissionWorkerResult` / `TransmissionWorkerOutcomeKind` — `Submitted`, `RetryScheduled`, `DeadLettered`, `RateLimited`, `CircuitOpen`.
- `TransmissionWorkerLogEvent::to_json_line` — one structured JSON log line per worker decision.

### Webhook dispatch (`WEBHOOK_BEAD_ID`)

Real HMAC-SHA256, not a placeholder. Signing and verification use the `hmac` + `sha2` crates; digest comparison is constant-time via `subtle`.

- `WebhookSigner::sign` — computes `HMAC-SHA256(secret, "<timestamp>.<body>")` and emits an `Invoicekit-Signature: t=<ts>,v1=<hex>` header (the Stripe-shape convention, so existing customer verification code ports).
- `WebhookVerifier::verify` — re-derives the HMAC, constant-time-compares the hex digests, enforces a 300-second replay window (computed with `unsigned_abs` so a crafted `i64::MIN` timestamp cannot panic), and consults an `EventIdLedger` for idempotency.
- `EventIdLedger` — in-memory seen-event-ID set. Documented as the test/lightweight default; the doc states real deployments persist the ledger (unique index / Redis set) so a restart does not re-accept replays.
- `WebhookDispatcher` — drives at-least-once delivery over an injected `WebhookTransport`: 2xx is success, 4xx (other than 408/429) is a permanent failure that is not retried, 5xx/408/429/transport errors are retried under `WebhookRetryPolicy` exponential backoff until `max_attempts`, then `DeliveryOutcome::Exhausted`.
- `WebhookEvent`, `WebhookEnvelope`, `WebhookHeaders`, `DeliveryOutcome`, `WebhookRetryDecision`, `WebhookRetryPolicy`, `WebhookVerifyError`.

### Reconcile API (`reconcile_api`, `RECONCILE_API_BEAD_ID`)

- `handle_reconcile_request(store, request)` — framework-free handler for `POST /v1/reconcile`. Takes a batch of `(internal_id, fingerprint_hex)` pairs and buckets each internal ID into `delivered`, `failed`, `pending`, or `unknown` based on the outbox row's state. Validates internal IDs (non-empty, ≤ 256 bytes), validates fingerprints (64 hex chars), de-duplicates fingerprints before querying, de-duplicates internal IDs in the response, and rejects batches over `MAX_BATCH_ENTRIES` (10 000) with `BatchTooLarge`. Empty batches short-circuit without touching the store.
- `OutboxLookup` trait — tenant-scoped bucket lookup. Tenant isolation is enforced by the store query (`buckets_for` is tenant-scoped and its impl MUST scope every query by tenant); the handler delegates isolation to the store and does not re-check the per-row tenant. `InMemoryOutboxLookup` is provided for tests; real Postgres/MySQL/SQLite impls live with the outbox migrations.

### PII redaction (`redact`, `REDACT_BEAD_ID`)

- `redact_for_support(document) -> RedactedBundle` — produces a copy of a `CommercialDocument` with every personal-data field replaced by the literal `<REDACTED>` placeholder, plus a `RedactionReport` listing the JSON-Pointer-style path of each redacted field. Redacted: party names, address lines / city / subdivision / postal code, contact name / email / phone, payment-instruction account and reference, tax-id value. Kept verbatim (reproducibility-critical, not personal data per the documented policy): document id / number, dates, currency, line descriptions, monetary totals, tax category codes, extensions, meta tenant/trace IDs, party IDs, country code, and tax-id scheme. The redacted document still passes IR validation and the operation is idempotent.

## Mode / Residuals

- **Webhook crypto is real.** Signing and verification are genuine HMAC-SHA256 with constant-time comparison. There is no mock or placeholder MAC in this crate.
- **`fingerprint` and retry jitter use real BLAKE3, for non-security purposes.** The fingerprint is a deduplication/lookup digest and the retry jitter is a deterministic spreading function. Neither is presented as, nor usable as, a signature, MAC, or secret.
- **`EventIdLedger` is in-memory by default.** It does not survive a process restart; persistent idempotency is the deployment's responsibility (the source documents this).
- **No gateway adapter ships here.** `GatewayAdapter` is a trait; the partner access point, native protocol, and cassette-backed mock adapters live in other crates. `OutboxLookup` likewise has only an in-memory test impl in this crate.
- **PII redaction is one-way.** v1 is not reversible; the source notes a sealed-envelope unredaction key is a future addition that would not change the `redact_for_support` signature.

## References

- PLAN.md section 2.5 — gateway adapter boundary identifiers (tenant / trace / idempotency / attempt).
- PLAN.md section 4.6 — invoice fingerprint formula.
- BLAKE3 — `fingerprint` and the deterministic retry-jitter source (`blake3` crate).
- HMAC-SHA256 — webhook signature scheme (`hmac` + `sha2`); constant-time compare via `subtle`.
- Webhook signature header — `Invoicekit-Signature: t=<ts>,v1=<hex>`, mirroring the Stripe webhook header shape; 300-second replay window.
- RFC 7230 — receivers should compare the lowercase `invoicekit-signature` header name case-insensitively.
- GDPR Article 4(1) and Recital 26 — basis for the redaction policy's PII-vs-non-PII field split.
- Embedded outbox migrations — `migrations/{postgres,mysql,sqlite}/001_invoicekit_outbox.{up,down}.sql`.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
