// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-134 v1 router.
//!
//! The router today wires `GET /v1/audit/events` to the T-142
//! framework-free handler. The auth + rate-limit checks live in
//! `extract_auth` so adding a new route is a three-line change:
//! call `extract_auth`, call the relevant handler, map the
//! response.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use invoicekit_managed_api::audit_log::{
    handle_audit_query, AuditPage, AuditQuery, AuditQueryError,
};
use invoicekit_managed_api::{TenantId, TenantRequestContext, TraceId};
use serde::{Deserialize, Serialize};

use crate::auth::{parse_bearer_token, ApiKeyAuthError, AuthenticatedTenant};
use crate::rate_limit::RateLimited;
use crate::AppState;

/// Build the `/v1/*` subrouter. Mounted by `build_router`.
pub fn v1_router(_state: AppState) -> Router<AppState> {
    Router::new().route("/audit/events", get(get_audit_events))
}

/// Query-string shape for `GET /v1/audit/events`. Accepts an
/// optional `page_size` and `cursor`; the filter axes that
/// [`AuditQuery`] supports (action, outcome, target_kind, target_id,
/// since, until) can be added incrementally.
#[derive(Debug, Deserialize, Default)]
struct AuditQueryParams {
    #[serde(default)]
    page_size: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

async fn get_audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<AuditQueryParams>,
) -> Result<Json<AuditPage>, ApiError> {
    let auth = authenticate(&state, &headers)?;
    rate_limit_check(&state, &auth.tenant_id, "GET /v1/audit/events")?;
    let trace_id = TraceId::new(format!("trace_{}", auth.api_key_id))
        .map_err(|e| ApiError::internal(format!("trace id build failed: {e}")))?;
    let context = TenantRequestContext::new(auth.tenant_id.clone(), trace_id, auth.actor.clone());
    let query = AuditQuery {
        tenant_id: auth.tenant_id,
        action: None,
        outcome: None,
        target_kind: None,
        target_id: None,
        since: None,
        until: None,
        page_size: params.page_size.unwrap_or(0),
        cursor: params.cursor,
    };
    let page = handle_audit_query(&context, &*state.audit_store, query).map_err(ApiError::from)?;
    Ok(Json(page))
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthenticatedTenant, ApiError> {
    let token = parse_bearer_token(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
    .map_err(ApiError::from)?;
    let resolved = state.api_keys.resolve(token).map_err(ApiError::from)?;
    resolved.ok_or(ApiError::Unauthorized {
        reason: "unknown API key".into(),
    })
}

fn rate_limit_check(state: &AppState, tenant: &TenantId, route: &str) -> Result<(), ApiError> {
    match state.rate_limiter.take(tenant, route) {
        Ok(_remaining) => Ok(()),
        Err(RateLimited { retry_after_ms }) => Err(ApiError::RateLimited { retry_after_ms }),
    }
}

/// Standard error envelope every handler emits on failure. Stable
/// shape so SDK consumers can pattern-match on `error.code`.
#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    /// The error inner payload.
    pub error: ApiErrorInner,
}

/// Body of the error envelope returned to the API consumer.
#[derive(Debug, Serialize)]
pub struct ApiErrorInner {
    /// Stable error code (e.g. `unauthorized`, `rate_limited`).
    pub code: &'static str,
    /// Operator-readable message.
    pub message: String,
}

/// Errors the gateway maps onto HTTP status codes.
#[derive(Debug)]
pub enum ApiError {
    /// 401 — caller did not present a valid API key.
    Unauthorized {
        /// Reason logged with the rejection.
        reason: String,
    },
    /// 429 — caller exceeded the per-tenant rate budget.
    RateLimited {
        /// Milliseconds the caller should wait before retrying.
        retry_after_ms: u32,
    },
    /// 400 — caller's request was malformed.
    BadRequest {
        /// Reason logged with the rejection.
        reason: String,
    },
    /// 403 — caller asked for something they don't own.
    Forbidden {
        /// Reason logged with the rejection.
        reason: String,
    },
    /// 500 — backend failure.
    Internal {
        /// Reason logged with the rejection.
        reason: String,
    },
}

impl ApiError {
    fn internal(reason: impl Into<String>) -> Self {
        Self::Internal {
            reason: reason.into(),
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized { .. } => "unauthorized",
            Self::RateLimited { .. } => "rate_limited",
            Self::BadRequest { .. } => "bad_request",
            Self::Forbidden { .. } => "forbidden",
            Self::Internal { .. } => "internal_error",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Unauthorized { reason }
            | Self::BadRequest { reason }
            | Self::Forbidden { reason }
            | Self::Internal { reason } => reason.clone(),
            Self::RateLimited { retry_after_ms } => {
                format!("rate limited; retry after {retry_after_ms} ms")
            }
        }
    }
}

impl From<ApiKeyAuthError> for ApiError {
    fn from(err: ApiKeyAuthError) -> Self {
        Self::Unauthorized {
            reason: err.to_string(),
        }
    }
}

impl From<AuditQueryError> for ApiError {
    fn from(err: AuditQueryError) -> Self {
        match err {
            AuditQueryError::CrossTenantRequest { .. } => Self::Forbidden {
                reason: err.to_string(),
            },
            AuditQueryError::BadCursor { .. } | AuditQueryError::Filter(_) => Self::BadRequest {
                reason: err.to_string(),
            },
            AuditQueryError::Store(reason) => Self::Internal { reason },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ApiErrorBody {
            error: ApiErrorInner {
                code: self.code(),
                message: self.message(),
            },
        };
        let mut response = (status, Json(body)).into_response();
        if let Self::RateLimited { retry_after_ms } = self {
            let value: u64 = u64::from(retry_after_ms).div_ceil(1000).max(1);
            if let Ok(header) = axum::http::HeaderValue::from_str(&value.to_string()) {
                response.headers_mut().insert("retry-after", header);
            }
        }
        response
    }
}

// Keep `Arc<dyn TenantApiKeyStore>` callable through `state.api_keys`.
impl AppState {
    /// Returns the trait-object view of the auth store. Tests use
    /// this to register tokens without depending on the concrete
    /// in-memory impl name.
    #[must_use]
    pub fn api_keys(&self) -> Arc<dyn crate::auth::TenantApiKeyStore + Send + Sync> {
        Arc::clone(&self.api_keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::InMemoryTenantApiKeyStore;
    use crate::rate_limit::TokenBucketRateLimiter;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use invoicekit_managed_api::audit_log::{AuditEventStore, InMemoryAuditStore};
    use invoicekit_managed_api::{
        Actor, ApiKeyId, AuditAction, AuditEvent, AuditEventId, AuditOutcome, AuditTarget,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    fn build_app() -> (
        axum::Router,
        Arc<InMemoryTenantApiKeyStore>,
        Arc<InMemoryAuditStore>,
    ) {
        let api_keys = Arc::new(InMemoryTenantApiKeyStore::new());
        let audit_store = Arc::new(InMemoryAuditStore::new());
        let state = AppState {
            api_keys: api_keys.clone(),
            rate_limiter: Arc::new(TokenBucketRateLimiter::with_default_policy()),
            audit_store: audit_store.clone(),
        };
        (crate::build_router(state), api_keys, audit_store)
    }

    #[tokio::test]
    async fn missing_auth_returns_401() {
        let (app, _keys, _audit) = build_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audit/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_token_returns_401() {
        let (app, _keys, _audit) = build_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audit/events")
                    .header("Authorization", "Bearer tok_unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_returns_audit_page() {
        let (app, keys, audit) = build_app();
        keys.insert(
            "tok_acme",
            TenantId::new("tenant_acme").unwrap(),
            ApiKeyId::new("key_1").unwrap(),
        );
        let ctx = TenantRequestContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new("trace_seed").unwrap(),
            Actor::ApiKey {
                key_id: ApiKeyId::new("key_1").unwrap(),
            },
        );
        audit
            .append(
                AuditEvent::new(
                    AuditEventId::new("aud_1").unwrap(),
                    &ctx,
                    AuditAction::TenantCreated,
                    AuditOutcome::Succeeded,
                    AuditTarget::new("tenant", "tenant_acme").unwrap(),
                    "2026-05-27T00:00:00Z",
                )
                .unwrap(),
            )
            .unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audit/events")
                    .header("Authorization", "Bearer tok_acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["count"], 1);
    }

    #[tokio::test]
    async fn rate_limit_kicks_in_after_burst_and_returns_429_with_retry_after() {
        let api_keys = Arc::new(InMemoryTenantApiKeyStore::new());
        api_keys.insert(
            "tok_acme",
            TenantId::new("tenant_acme").unwrap(),
            ApiKeyId::new("key_1").unwrap(),
        );
        let state = AppState {
            api_keys: api_keys.clone(),
            rate_limiter: Arc::new(TokenBucketRateLimiter::with_policy(
                crate::rate_limit::RateLimitPolicy {
                    capacity: 2,
                    refill_per_second: 0.001,
                },
            )),
            audit_store: Arc::new(InMemoryAuditStore::new()),
        };
        let app = crate::build_router(state);
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/audit/events")
                        .header("Authorization", "Bearer tok_acme")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audit/events")
                    .header("Authorization", "Bearer tok_acme")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key("retry-after"));
    }
}
