// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-134 ApiKey-based authentication layer.
//!
//! Tokens follow the `Authorization: Bearer <token>` shape that
//! every InvoiceKit SDK already produces. The token resolves through
//! a [`TenantApiKeyStore`] to a `(TenantId, Actor::ApiKey)` pair the
//! handler chain reads off the request extensions. The store is a
//! trait so the in-memory test impl below + a future Postgres-backed
//! impl share the same middleware code.
//!
//! OIDC and the more elaborate scoped-permission flow live on the
//! existing `invoicekit-managed-api` types (`Actor::Principal`,
//! `ApiKeyRecord::require_scope`); a follow-up bead replaces the
//! middleware here with a unified token validator that recognizes
//! both bearer schemes.

use std::collections::HashMap;
use std::sync::Mutex;

use invoicekit_managed_api::{Actor, ApiKeyId, TenantId};
use thiserror::Error;

/// Resolved auth context the middleware stashes on the request.
#[derive(Clone, Debug)]
pub struct AuthenticatedTenant {
    /// Tenant the API key is scoped to.
    pub tenant_id: TenantId,
    /// API key id (for log correlation; the secret is never logged).
    pub api_key_id: ApiKeyId,
    /// Pre-built `Actor::ApiKey` value for downstream
    /// `TenantRequestContext` construction.
    pub actor: Actor,
}

/// Errors surfaced by the middleware.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApiKeyAuthError {
    /// Header is absent.
    #[error("missing Authorization header")]
    MissingHeader,
    /// Header didn't start with `Bearer `.
    #[error("malformed Authorization header: {0}")]
    MalformedHeader(String),
    /// Token isn't registered with the store.
    #[error("unknown API key")]
    UnknownKey,
    /// Store backend failed.
    #[error("auth store error: {0}")]
    Store(String),
}

/// Trait the middleware queries to resolve an opaque bearer token.
pub trait TenantApiKeyStore: Send + Sync {
    /// Look up `token`. Returns `Ok(None)` for an unknown token
    /// (mapped to [`ApiKeyAuthError::UnknownKey`] by the
    /// middleware) and `Err(...)` for backend failure.
    ///
    /// # Errors
    ///
    /// Returns [`ApiKeyAuthError::Store`] on backend failure.
    fn resolve(&self, token: &str) -> Result<Option<AuthenticatedTenant>, ApiKeyAuthError>;
}

/// In-memory store for tests and lightweight deployments.
#[derive(Debug, Default)]
pub struct InMemoryTenantApiKeyStore {
    rows: Mutex<HashMap<String, AuthenticatedTenant>>,
}

impl InMemoryTenantApiKeyStore {
    /// New empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a token under `(tenant, key_id)`. Idempotent on the
    /// token; re-registering the same token overrides the prior
    /// `(tenant, key_id)` pair.
    pub fn insert(&self, token: impl Into<String>, tenant: TenantId, key_id: ApiKeyId) {
        let mut rows = self.rows.lock().expect("test lock poisoned");
        rows.insert(
            token.into(),
            AuthenticatedTenant {
                tenant_id: tenant,
                api_key_id: key_id.clone(),
                actor: Actor::ApiKey { key_id },
            },
        );
    }

    /// Number of stored tokens. Cheap.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.lock().expect("test lock poisoned").len()
    }

    /// Whether the store holds zero tokens.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TenantApiKeyStore for InMemoryTenantApiKeyStore {
    fn resolve(&self, token: &str) -> Result<Option<AuthenticatedTenant>, ApiKeyAuthError> {
        let rows = self
            .rows
            .lock()
            .map_err(|e| ApiKeyAuthError::Store(format!("lock poisoned: {e}")))?;
        Ok(rows.get(token).cloned())
    }
}

/// Extract the bearer token from an HTTP `Authorization` header.
///
/// # Errors
///
/// Returns [`ApiKeyAuthError::MissingHeader`] when `value` is
/// `None`, and [`ApiKeyAuthError::MalformedHeader`] when the
/// header doesn't start with `Bearer ` or carries an empty token.
pub fn parse_bearer_token(value: Option<&str>) -> Result<&str, ApiKeyAuthError> {
    let raw = value.ok_or(ApiKeyAuthError::MissingHeader)?;
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiKeyAuthError::MalformedHeader("expected 'Bearer <token>'".into()))?;
    if token.trim().is_empty() {
        return Err(ApiKeyAuthError::MalformedHeader("empty token".into()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(s: &str) -> TenantId {
        TenantId::new(s).unwrap()
    }

    fn key(s: &str) -> ApiKeyId {
        ApiKeyId::new(s).unwrap()
    }

    #[test]
    fn parse_bearer_token_handles_happy_path() {
        let token = parse_bearer_token(Some("Bearer secret123")).unwrap();
        assert_eq!(token, "secret123");
    }

    #[test]
    fn parse_bearer_token_missing_header_is_typed() {
        let err = parse_bearer_token(None).unwrap_err();
        assert_eq!(err, ApiKeyAuthError::MissingHeader);
    }

    #[test]
    fn parse_bearer_token_malformed_scheme_is_typed() {
        let err = parse_bearer_token(Some("Basic abc")).unwrap_err();
        assert!(matches!(err, ApiKeyAuthError::MalformedHeader(_)));
    }

    #[test]
    fn parse_bearer_token_empty_token_is_typed() {
        let err = parse_bearer_token(Some("Bearer   ")).unwrap_err();
        assert!(matches!(err, ApiKeyAuthError::MalformedHeader(_)));
    }

    #[test]
    fn in_memory_store_round_trips_a_registered_token() {
        let store = InMemoryTenantApiKeyStore::new();
        store.insert("tok_acme_1", tenant("tenant_acme"), key("key_1"));
        let resolved = store.resolve("tok_acme_1").unwrap().unwrap();
        assert_eq!(resolved.tenant_id, tenant("tenant_acme"));
        assert_eq!(resolved.api_key_id, key("key_1"));
        assert!(matches!(resolved.actor, Actor::ApiKey { .. }));
    }

    #[test]
    fn in_memory_store_returns_none_for_unknown_token() {
        let store = InMemoryTenantApiKeyStore::new();
        store.insert("tok_acme_1", tenant("tenant_acme"), key("key_1"));
        assert!(store.resolve("tok_other").unwrap().is_none());
    }

    #[test]
    fn re_registering_a_token_overrides_the_tenant() {
        let store = InMemoryTenantApiKeyStore::new();
        store.insert("tok", tenant("tenant_a"), key("key_a"));
        store.insert("tok", tenant("tenant_b"), key("key_b"));
        let resolved = store.resolve("tok").unwrap().unwrap();
        assert_eq!(resolved.tenant_id, tenant("tenant_b"));
    }
}
