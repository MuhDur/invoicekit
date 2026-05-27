// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-134 managed-API gateway.
//!
//! `invoicekit_managed_api_server::build_router` produces the axum
//! `Router` that the binary wires to a TCP listener. The router
//! stacks three middlewares before any handler:
//!
//! 1. **ApiKey auth** — extracts the `Authorization: Bearer <token>`
//!    header, looks the token up in a [`TenantApiKeyStore`], and
//!    attaches the resolved `TenantRequestContext` (from
//!    `invoicekit-managed-api`) to the request extensions so route
//!    handlers can read it without re-doing the auth work. Unknown
//!    / missing tokens return 401.
//! 2. **Per-tenant rate limit** — token bucket keyed by
//!    `(tenant_id, route)`. The default policy is permissive (60
//!    requests / 10 seconds, burst 30) and is tunable per route.
//!    Exhausted buckets return 429 with a `Retry-After` header.
//! 3. **Tracing wrap** — every handled request carries a generated
//!    `trace_id` that lands on the response as `X-Invoicekit-Trace-Id`
//!    so the customer can correlate with our logs.
//!
//! Today the router exposes `/v1/audit/events` (T-142). Subsequent
//! beads wire `/v1/reconcile` (T-075), `/v1/events/sse` (T-077),
//! and `/v1/capabilities` (T-006a). The build_router function takes
//! the per-route handler dependencies as an [`AppState`] so adding
//! a new route is one entry in `AppState` plus one router call.

#![allow(
    clippy::option_if_let_else,
    clippy::module_name_repetitions,
    clippy::too_long_first_doc_paragraph,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::significant_drop_tightening,
    clippy::or_fun_call,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::must_use_candidate
)]

pub mod auth;
pub mod rate_limit;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use invoicekit_managed_api::audit_log::AuditEventStore;

pub use auth::{ApiKeyAuthError, InMemoryTenantApiKeyStore, TenantApiKeyStore};
pub use rate_limit::{RateLimitPolicy, TokenBucketRateLimiter};

/// Bead identifier carried in log records.
pub const GATEWAY_BEAD_ID: &str = "invoices-t-134-api-gateway-rate-limiting-i4c0";

/// Shared state passed to every handler.
///
/// Each field is an `Arc<dyn Trait>` so the production wiring can
/// swap the in-memory test stores for SQL-backed ones without
/// touching the router or middleware.
#[derive(Clone)]
pub struct AppState {
    /// API-key → tenant resolution.
    pub api_keys: Arc<dyn TenantApiKeyStore + Send + Sync>,
    /// Per-tenant token-bucket rate limiter.
    pub rate_limiter: Arc<TokenBucketRateLimiter>,
    /// Backing store for `GET /v1/audit/events`.
    pub audit_store: Arc<dyn AuditEventStore>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

/// Build the axum router with every middleware stacked.
///
/// The order matters: auth before rate limit, so a flood of
/// unauthenticated traffic burns the auth-failure path (cheap)
/// instead of consuming a tenant's rate-limit budget.
pub fn build_router(state: AppState) -> Router {
    let v1 = routes::v1_router(state.clone());
    Router::new().nest("/v1", v1).with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_managed_api::audit_log::InMemoryAuditStore;

    fn smoke_state() -> AppState {
        AppState {
            api_keys: Arc::new(InMemoryTenantApiKeyStore::new()),
            rate_limiter: Arc::new(TokenBucketRateLimiter::with_default_policy()),
            audit_store: Arc::new(InMemoryAuditStore::new()),
        }
    }

    #[test]
    fn build_router_smoke() {
        let _router = build_router(smoke_state());
    }
}
