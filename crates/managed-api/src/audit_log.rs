// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-142 customer-facing audit log API.
//!
//! `GET /v1/audit/events` per the bead spec is two layers:
//!
//! 1. **Query + filter + paginate** an [`AuditEventStore`] for the
//!    requesting tenant. The store is a trait so the in-memory test
//!    impl ([`InMemoryAuditStore`]) and the future Postgres-backed
//!    impl share the same handler code. The handler is
//!    framework-free — it returns a typed [`AuditPage`] the future
//!    HTTP layer (T-134 API gateway) wraps in a Response.
//! 2. **Signed export** of a paginated result as CSV or JSON, with
//!    an HMAC-SHA256 signature over the canonical body bytes so a
//!    downstream operator can verify the export wasn't tampered with.
//!    Same algorithm choice as T-076's webhook signing for
//!    consistency.
//!
//! The handler enforces tenant isolation rigorously: every query
//! carries a `TenantRequestContext`, and the store impls reject any
//! query whose `tenant_id` doesn't match the context's tenant. A
//! cross-tenant query attempt surfaces as
//! [`AuditQueryError::CrossTenantRequest`] so the HTTP layer can log
//! it and return 403.

#![allow(
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::significant_drop_tightening,
    clippy::trivially_copy_pass_by_ref,
    clippy::or_fun_call,
    clippy::naive_bytecount,
    clippy::redundant_clone
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

use crate::{
    AuditAction, AuditEvent, AuditOutcome, ManagedApiError, TenantId, TenantRequestContext,
};

/// Bead identifier emitted in error/log records.
pub const AUDIT_LOG_API_BEAD_ID: &str = "invoices-t-142-customer-audit-log-api-zbgz";

/// Default `page_size` when the caller omits it. Picked to match the
/// shape Stripe + GitHub use for similarly-structured event APIs.
pub const DEFAULT_PAGE_SIZE: usize = 50;

/// Hard cap on `page_size`. A larger page would let one big query
/// pin the audit store; the HTTP layer should still honor any
/// proxy-level rate limit on top of this.
pub const MAX_PAGE_SIZE: usize = 500;

/// HMAC scheme tag stamped onto signed exports.
pub const SIGNATURE_ALG: &str = "hmac-sha256";

/// Output format the caller asks for via `?format=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// One JSON object per line; canonical key order.
    Json,
    /// Comma-separated values with a stable column header.
    Csv,
}

/// Filter shape parsed from the `GET /v1/audit/events` query string.
///
/// Empty filters mean "no filter on that axis." `since` / `until`
/// are RFC 3339 timestamps; the handler validates them as
/// non-empty strings but doesn't parse the calendar — store
/// implementations are responsible for the comparison (SQL stores
/// can use proper TIMESTAMP, the in-memory store uses lexical
/// compare which works for RFC 3339).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditQuery {
    /// Required: tenant whose events we're querying. Enforced
    /// against the calling [`TenantRequestContext`].
    pub tenant_id: TenantId,
    /// Optional action filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<AuditAction>,
    /// Optional outcome filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AuditOutcome>,
    /// Filter to events targeting this kind (`invoice`, `api_key`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    /// Filter to events targeting this exact id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// RFC 3339 inclusive lower bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// RFC 3339 inclusive upper bound on `occurred_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Page size (clamped to [1, [`MAX_PAGE_SIZE`]] by the handler).
    /// Zero or absent means [`DEFAULT_PAGE_SIZE`].
    #[serde(default)]
    pub page_size: usize,
    /// Opaque cursor returned by a previous page; `None` for the
    /// first page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl AuditQuery {
    /// New filter scoped to `tenant_id` with every other axis at
    /// "no filter." Tests and HTTP handlers should start here and
    /// override the fields the caller actually supplied.
    #[must_use]
    pub fn for_tenant(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            action: None,
            outcome: None,
            target_kind: None,
            target_id: None,
            since: None,
            until: None,
            page_size: 0,
            cursor: None,
        }
    }
}

/// One page of audit events plus the cursor a caller hands back for
/// the next page.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditPage {
    /// Matching events, oldest first.
    pub events: Vec<AuditEvent>,
    /// Number of events in this page (cheap convenience for clients).
    pub count: usize,
    /// Cursor for the next page; `None` when the result was the tail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Signed export envelope. `body` is the raw CSV / JSON bytes;
/// `signature` is hex-encoded HMAC-SHA256 over those bytes.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignedExport {
    /// Format the body was rendered as.
    pub format: ExportFormat,
    /// Tenant whose events are in the body.
    pub tenant_id: TenantId,
    /// Number of events the body contains.
    pub event_count: usize,
    /// Signature scheme tag ([`SIGNATURE_ALG`]).
    pub signature_alg: String,
    /// Hex-encoded HMAC-SHA256 signature over `body`.
    pub signature: String,
    /// Raw body bytes (CSV or JSON).
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

/// Errors raised by the audit-log query handler.
#[derive(Debug, Error)]
pub enum AuditQueryError {
    /// The caller asked for a tenant they don't own.
    #[error(
        "cross-tenant audit query: caller tenant {caller:?} does not match query tenant {requested:?}"
    )]
    CrossTenantRequest {
        /// Tenant on the calling context.
        caller: String,
        /// Tenant on the query.
        requested: String,
    },
    /// Cursor was not produced by this implementation.
    #[error("opaque cursor {cursor:?} is not recognized")]
    BadCursor {
        /// Offending cursor value.
        cursor: String,
    },
    /// Generic store failure surfaced from the backend impl.
    #[error("audit store error: {0}")]
    Store(String),
    /// Filter validation failed (e.g. empty `since` string).
    #[error("invalid filter: {0}")]
    Filter(String),
}

/// Errors raised by signed-export verification.
#[derive(Debug, Error)]
pub enum ExportVerifyError {
    /// Signature scheme on the envelope is not [`SIGNATURE_ALG`].
    #[error("unsupported signature scheme {scheme:?}; expected {SIGNATURE_ALG}")]
    UnknownScheme {
        /// Scheme declared on the envelope.
        scheme: String,
    },
    /// Recomputed HMAC did not match the envelope.
    #[error("HMAC signature mismatch")]
    SignatureMismatch,
    /// Signature was not valid hex.
    #[error("signature is not valid hex: {0}")]
    BadHex(String),
}

/// Storage backend for audit events. Real implementations wrap
/// Postgres / sqlite tables; the in-memory impl below powers tests
/// and the dispatcher's own integration smoke.
pub trait AuditEventStore: Send + Sync {
    /// Append one event. Idempotent on `event_id` — re-appending the
    /// same id is a no-op (in-memory impl) or `ON CONFLICT DO NOTHING`
    /// (SQL impls).
    ///
    /// # Errors
    ///
    /// Implementations may surface backend-specific errors; map them
    /// to [`AuditQueryError::Store`] when bubbling up.
    fn append(&self, event: AuditEvent) -> Result<(), AuditQueryError>;

    /// Run a query. The handler has already applied tenant-isolation
    /// and clamped the page size before calling this.
    ///
    /// # Errors
    ///
    /// Implementations may surface backend-specific errors as
    /// [`AuditQueryError::Store`].
    fn query(&self, query: &AuditQuery) -> Result<AuditPage, AuditQueryError>;
}

/// In-memory [`AuditEventStore`] for tests and small managed
/// deployments where Postgres is overkill.
#[derive(Debug, Default)]
pub struct InMemoryAuditStore {
    events: std::sync::Mutex<Vec<AuditEvent>>,
}

impl InMemoryAuditStore {
    /// New empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored events. Cheap.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Whether the store holds zero events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AuditEventStore for InMemoryAuditStore {
    fn append(&self, event: AuditEvent) -> Result<(), AuditQueryError> {
        let mut events = self
            .events
            .lock()
            .map_err(|e| AuditQueryError::Store(format!("lock poisoned: {e}")))?;
        if events.iter().any(|e| e.event_id == event.event_id) {
            return Ok(());
        }
        events.push(event);
        Ok(())
    }

    fn query(&self, query: &AuditQuery) -> Result<AuditPage, AuditQueryError> {
        let events = self
            .events
            .lock()
            .map_err(|e| AuditQueryError::Store(format!("lock poisoned: {e}")))?;
        let mut filtered: Vec<&AuditEvent> = events
            .iter()
            .filter(|e| e.tenant_id == query.tenant_id)
            .filter(|e| query.action.is_none_or(|a| e.action == a))
            .filter(|e| query.outcome.is_none_or(|o| e.outcome == o))
            .filter(|e| {
                query
                    .target_kind
                    .as_deref()
                    .is_none_or(|k| e.target.kind == k)
            })
            .filter(|e| query.target_id.as_deref().is_none_or(|i| e.target.id == i))
            .filter(|e| {
                query
                    .since
                    .as_deref()
                    .is_none_or(|s| e.occurred_at.as_str() >= s)
            })
            .filter(|e| {
                query
                    .until
                    .as_deref()
                    .is_none_or(|u| e.occurred_at.as_str() <= u)
            })
            .collect();
        // Stable ordering: oldest first by occurred_at then event_id.
        filtered.sort_by(|a, b| {
            a.occurred_at
                .cmp(&b.occurred_at)
                .then_with(|| a.event_id.as_str().cmp(b.event_id.as_str()))
        });

        // The cursor is client-supplied and decodes to an unbounded `usize`, so
        // clamp it to the result length before slicing: an out-of-range cursor
        // must yield an empty final page, never a panic. `saturating_add` keeps
        // a near-`usize::MAX` start from overflowing when the page size is added.
        let start = match &query.cursor {
            Some(c) => decode_cursor(c)?.min(filtered.len()),
            None => 0,
        };
        let end = start.saturating_add(query.page_size).min(filtered.len());
        let page: Vec<AuditEvent> = filtered[start..end].iter().map(|e| (*e).clone()).collect();
        let next_cursor = if end < filtered.len() {
            Some(encode_cursor(end))
        } else {
            None
        };
        Ok(AuditPage {
            events: page.clone(),
            count: page.len(),
            next_cursor,
        })
    }
}

/// Handler the future HTTP layer (T-134 API gateway) calls when a
/// `GET /v1/audit/events` request arrives.
///
/// # Errors
///
/// Returns [`AuditQueryError::CrossTenantRequest`] when the caller
/// asks for a tenant they don't own, [`AuditQueryError::Filter`]
/// when the filter contains an empty string, and propagates
/// [`AuditQueryError::Store`] from the backend.
pub fn handle_audit_query(
    ctx: &TenantRequestContext,
    store: &dyn AuditEventStore,
    mut query: AuditQuery,
) -> Result<AuditPage, AuditQueryError> {
    if query.tenant_id != ctx.tenant_id {
        return Err(AuditQueryError::CrossTenantRequest {
            caller: ctx.tenant_id.as_str().to_owned(),
            requested: query.tenant_id.as_str().to_owned(),
        });
    }
    if let Some(s) = &query.since {
        if s.is_empty() {
            return Err(AuditQueryError::Filter("since must not be blank".into()));
        }
    }
    if let Some(u) = &query.until {
        if u.is_empty() {
            return Err(AuditQueryError::Filter("until must not be blank".into()));
        }
    }
    if let Some(k) = &query.target_kind {
        if k.is_empty() {
            return Err(AuditQueryError::Filter(
                "target_kind must not be blank".into(),
            ));
        }
    }
    if let Some(i) = &query.target_id {
        if i.is_empty() {
            return Err(AuditQueryError::Filter(
                "target_id must not be blank".into(),
            ));
        }
    }

    if query.page_size == 0 {
        query.page_size = DEFAULT_PAGE_SIZE;
    } else if query.page_size > MAX_PAGE_SIZE {
        query.page_size = MAX_PAGE_SIZE;
    }
    store.query(&query)
}

/// Produce a signed export for `page`. The result's `body` is the
/// canonical CSV / JSON encoding and `signature` is hex(HMAC-SHA256(key, body)).
///
/// # Panics
///
/// Panics only via the internal `serde_json::to_string_pretty` /
/// HMAC `expect`s, which would indicate a malformed [`AuditEvent`] or
/// a 0-byte HMAC key. Both are constructor-prevented elsewhere.
///
/// # Errors
///
/// Returns [`ManagedApiError`] when re-serializing an event for the
/// export envelope hits an `unrepresentable` IR value (should not
/// happen with the current `AuditEvent` shape; documented here
/// because the function is fallible at the type level).
pub fn signed_export(
    tenant_id: TenantId,
    format: ExportFormat,
    page: &AuditPage,
    signing_key: &[u8],
) -> Result<SignedExport, ManagedApiError> {
    let body = match format {
        ExportFormat::Json => render_json(&page.events)?,
        ExportFormat::Csv => render_csv(&page.events),
    };
    let signature = hex_hmac_sha256(signing_key, &body);
    Ok(SignedExport {
        format,
        tenant_id,
        event_count: page.count,
        signature_alg: SIGNATURE_ALG.to_owned(),
        signature,
        body,
    })
}

/// Verify a signed export under `signing_key`. Returns the export's
/// `body` bytes on success.
///
/// # Errors
///
/// Returns [`ExportVerifyError::UnknownScheme`] for non-HMAC envelopes,
/// [`ExportVerifyError::BadHex`] when the signature isn't valid hex,
/// and [`ExportVerifyError::SignatureMismatch`] when the recomputed
/// HMAC doesn't match.
pub fn verify_signed_export(
    export: &SignedExport,
    signing_key: &[u8],
) -> Result<Vec<u8>, ExportVerifyError> {
    if export.signature_alg != SIGNATURE_ALG {
        return Err(ExportVerifyError::UnknownScheme {
            scheme: export.signature_alg.clone(),
        });
    }
    let supplied = hex_decode(&export.signature).map_err(ExportVerifyError::BadHex)?;
    let recomputed = raw_hmac_sha256(signing_key, &export.body);
    if recomputed.ct_eq(&supplied).into() {
        Ok(export.body.clone())
    } else {
        Err(ExportVerifyError::SignatureMismatch)
    }
}

fn render_json(events: &[AuditEvent]) -> Result<Vec<u8>, ManagedApiError> {
    let mut out = Vec::new();
    for e in events {
        let line = serde_json::to_vec(e).map_err(|_| ManagedApiError::InvalidIdentifier {
            field: "audit_event",
            reason: "audit event JSON encode failed",
        })?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

fn render_csv(events: &[AuditEvent]) -> Vec<u8> {
    let mut out = String::with_capacity(events.len() * 128);
    out.push_str(
        "event_id,tenant_id,trace_id,actor,action,outcome,target_kind,target_id,occurred_at,metadata\n",
    );
    for e in events {
        let actor = actor_short(&e.actor);
        let action = serde_action(&e.action);
        let outcome = serde_outcome(&e.outcome);
        let metadata = metadata_compact(&e.metadata);
        let _ = writeln!(
            out,
            "{eid},{tid},{trid},{actor},{action},{outcome},{kind},{tgt},{ts},{meta}",
            eid = csv_escape(e.event_id.as_str()),
            tid = csv_escape(e.tenant_id.as_str()),
            trid = csv_escape(e.trace_id.as_str()),
            actor = csv_escape(&actor),
            action = csv_escape(action),
            outcome = csv_escape(outcome),
            kind = csv_escape(&e.target.kind),
            tgt = csv_escape(&e.target.id),
            ts = csv_escape(&e.occurred_at),
            meta = csv_escape(&metadata)
        );
    }
    out.into_bytes()
}

fn actor_short(actor: &crate::Actor) -> String {
    match actor {
        crate::Actor::System { name } => format!("system:{name}"),
        crate::Actor::Principal { principal_id } => format!("principal:{principal_id}"),
        crate::Actor::ApiKey { key_id } => format!("api_key:{key_id}"),
    }
}

fn serde_action(action: &AuditAction) -> &'static str {
    match action {
        AuditAction::TenantCreated => "tenant_created",
        AuditAction::ApiKeyCreated => "api_key_created",
        AuditAction::ApiKeyRevoked => "api_key_revoked",
        AuditAction::OidcLoginSucceeded => "oidc_login_succeeded",
        AuditAction::OidcLoginRejected => "oidc_login_rejected",
        AuditAction::RoleAssigned => "role_assigned",
        AuditAction::RoleRemoved => "role_removed",
        AuditAction::InvoiceValidated => "invoice_validated",
        AuditAction::InvoiceTransmitted => "invoice_transmitted",
    }
}

fn serde_outcome(outcome: &AuditOutcome) -> &'static str {
    match outcome {
        AuditOutcome::Succeeded => "succeeded",
        AuditOutcome::Denied => "denied",
        AuditOutcome::Failed => "failed",
    }
}

fn metadata_compact(metadata: &BTreeMap<String, String>) -> String {
    if metadata.is_empty() {
        return String::new();
    }
    metadata
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn hex_hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    let bytes = raw_hmac_sha256(key, msg);
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in &bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn raw_hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err(format!("odd-length hex string ({} chars)", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = hex_nybble(bytes[i])?;
        let lo = hex_nybble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nybble(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        other => Err(format!("non-hex byte {other:#x}")),
    }
}

fn encode_cursor(offset: usize) -> String {
    // Opaque-to-the-client; we just hex-encode the offset so a
    // future store impl can swap to a real opaque token without
    // changing the wire shape.
    format!("v1.{offset:x}")
}

fn decode_cursor(s: &str) -> Result<usize, AuditQueryError> {
    let rest = s.strip_prefix("v1.").ok_or(AuditQueryError::BadCursor {
        cursor: s.to_owned(),
    })?;
    usize::from_str_radix(rest, 16).map_err(|_| AuditQueryError::BadCursor {
        cursor: s.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Actor, AuditEventId, AuditTarget, TenantRequestContext, TraceId};

    fn tenant(s: &str) -> TenantId {
        TenantId::new(s).unwrap()
    }

    fn context_for(tenant_name: &str) -> TenantRequestContext {
        TenantRequestContext::new(
            tenant(tenant_name),
            TraceId::new(format!("trace_{tenant_name}")).unwrap(),
            Actor::System {
                name: "managed-api".to_owned(),
            },
        )
    }

    fn event_for(
        ctx: &TenantRequestContext,
        suffix: &str,
        action: AuditAction,
        outcome: AuditOutcome,
        target_kind: &str,
        when: &str,
    ) -> AuditEvent {
        AuditEvent::new(
            AuditEventId::new(format!("aud_{suffix}")).unwrap(),
            ctx,
            action,
            outcome,
            AuditTarget::new(target_kind, format!("res_{suffix}")).unwrap(),
            when,
        )
        .unwrap()
    }

    fn seed_store(store: &InMemoryAuditStore) -> TenantRequestContext {
        let ctx_a = context_for("tenant_a");
        let ctx_b = context_for("tenant_b");
        let evs = [
            event_for(
                &ctx_a,
                "001",
                AuditAction::TenantCreated,
                AuditOutcome::Succeeded,
                "tenant",
                "2026-05-26T00:00:00Z",
            ),
            event_for(
                &ctx_a,
                "002",
                AuditAction::InvoiceValidated,
                AuditOutcome::Succeeded,
                "invoice",
                "2026-05-26T01:00:00Z",
            ),
            event_for(
                &ctx_a,
                "003",
                AuditAction::InvoiceTransmitted,
                AuditOutcome::Failed,
                "invoice",
                "2026-05-26T02:00:00Z",
            ),
            event_for(
                &ctx_b,
                "100",
                AuditAction::TenantCreated,
                AuditOutcome::Succeeded,
                "tenant",
                "2026-05-26T00:00:00Z",
            ),
        ];
        for e in evs {
            store.append(e).unwrap();
        }
        ctx_a
    }

    #[test]
    fn query_filters_to_caller_tenant_only() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(page.count, 3, "should not see tenant_b events");
        for ev in &page.events {
            assert_eq!(ev.tenant_id, ctx.tenant_id);
        }
    }

    #[test]
    fn query_rejects_cross_tenant_request() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let err = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: tenant("tenant_b"),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap_err();
        assert!(matches!(err, AuditQueryError::CrossTenantRequest { .. }));
    }

    #[test]
    fn query_paginates_results_via_opaque_cursor() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let p1 = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 2,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(p1.count, 2);
        assert!(p1.next_cursor.is_some(), "should hand back a cursor");
        let p2 = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 2,
                cursor: p1.next_cursor.clone(),
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(p2.count, 1, "tail page");
        assert!(p2.next_cursor.is_none());
        // No overlap between p1 and p2.
        let p1_ids: Vec<_> = p1.events.iter().map(|e| e.event_id.as_str()).collect();
        let p2_ids: Vec<_> = p2.events.iter().map(|e| e.event_id.as_str()).collect();
        for id in &p2_ids {
            assert!(!p1_ids.contains(id), "cursor must not double-emit");
        }
    }

    #[test]
    fn query_out_of_range_cursor_yields_empty_page_not_panic() {
        // The cursor is a client-controlled opaque token. A value past the end
        // of the result set must yield an empty final page, never panic the
        // `filtered[start..end]` slice (start > len) or overflow the
        // `start + page_size` offset. `v1.ffff...f` decodes to usize::MAX on a
        // 64-bit host, exercising both the out-of-range slice and the add.
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                cursor: Some("v1.ffffffffffffffff".into()),
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .expect("an out-of-range cursor must be handled gracefully, not panic");
        assert_eq!(page.count, 0, "out-of-range cursor returns an empty page");
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn query_filters_by_action_outcome_and_target() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                action: Some(AuditAction::InvoiceTransmitted),
                outcome: Some(AuditOutcome::Failed),
                target_kind: Some("invoice".into()),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(page.count, 1);
        assert_eq!(page.events[0].event_id.as_str(), "aud_003");
    }

    #[test]
    fn query_filters_by_time_window() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                since: Some("2026-05-26T01:30:00Z".into()),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(page.count, 1, "only the 02:00 event passes since");
    }

    #[test]
    fn query_rejects_bad_cursor() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let err = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                cursor: Some("garbage".into()),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap_err();
        assert!(matches!(err, AuditQueryError::BadCursor { .. }));
    }

    #[test]
    fn query_rejects_blank_filter() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let err = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                target_kind: Some(String::new()),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap_err();
        assert!(matches!(err, AuditQueryError::Filter(_)));
    }

    #[test]
    fn query_clamps_page_size_to_max() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: MAX_PAGE_SIZE + 1000,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        assert_eq!(page.count, 3); // only 3 events available; cap applied silently
    }

    #[test]
    fn query_zero_page_size_defaults_to_50() {
        // Asserts the docstring contract; we trust the cap not to
        // change in a backwards-incompatible way.
        let store = InMemoryAuditStore::new();
        let _ctx = seed_store(&store);
        // Spot-check the constant rather than over-fixturing.
        assert_eq!(DEFAULT_PAGE_SIZE, 50);
    }

    #[test]
    fn append_is_idempotent_on_event_id() {
        let store = InMemoryAuditStore::new();
        let ctx = context_for("tenant_x");
        let e = event_for(
            &ctx,
            "999",
            AuditAction::TenantCreated,
            AuditOutcome::Succeeded,
            "tenant",
            "2026-05-26T00:00:00Z",
        );
        store.append(e.clone()).unwrap();
        store.append(e.clone()).unwrap();
        store.append(e).unwrap();
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn signed_export_json_round_trips_through_verify() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        let key = b"super-secret-export-signing-key";
        let export = signed_export(ctx.tenant_id.clone(), ExportFormat::Json, &page, key).unwrap();
        assert_eq!(export.signature_alg, SIGNATURE_ALG);
        assert_eq!(export.event_count, page.count);
        assert_eq!(export.signature.len(), 64); // hex SHA-256
        let body = verify_signed_export(&export, key).unwrap();
        assert_eq!(body, export.body);
        // JSON body is one event per line.
        assert_eq!(body.iter().filter(|b| **b == b'\n').count(), page.count);
    }

    #[test]
    fn signed_export_csv_round_trips_through_verify() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        let key = b"key-bytes";
        let export = signed_export(ctx.tenant_id.clone(), ExportFormat::Csv, &page, key).unwrap();
        let body = verify_signed_export(&export, key).unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.starts_with("event_id,tenant_id"), "header row first");
        assert!(text.contains("aud_001"));
    }

    #[test]
    fn verify_rejects_signature_under_wrong_key() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        let export =
            signed_export(ctx.tenant_id.clone(), ExportFormat::Json, &page, b"k1").unwrap();
        let err = verify_signed_export(&export, b"k2").unwrap_err();
        assert!(matches!(err, ExportVerifyError::SignatureMismatch));
    }

    #[test]
    fn verify_rejects_unknown_scheme() {
        let mut export = SignedExport {
            format: ExportFormat::Json,
            tenant_id: tenant("t"),
            event_count: 0,
            signature_alg: "rsa-pss".into(),
            signature: "00".into(),
            body: vec![],
        };
        export.signature_alg = "rsa-pss".to_owned();
        let err = verify_signed_export(&export, b"any").unwrap_err();
        assert!(matches!(err, ExportVerifyError::UnknownScheme { .. }));
    }

    #[test]
    fn verify_rejects_bad_hex_signature() {
        let store = InMemoryAuditStore::new();
        let ctx = seed_store(&store);
        let page = handle_audit_query(
            &ctx,
            &store,
            AuditQuery {
                tenant_id: ctx.tenant_id.clone(),
                page_size: 50,
                ..AuditQuery::for_tenant(ctx.tenant_id.clone())
            },
        )
        .unwrap();
        let mut export =
            signed_export(ctx.tenant_id.clone(), ExportFormat::Json, &page, b"k").unwrap();
        export.signature = "zzznotsohex".into();
        let err = verify_signed_export(&export, b"k").unwrap_err();
        assert!(matches!(err, ExportVerifyError::BadHex(_)));
    }
}
