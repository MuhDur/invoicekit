// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-077 Server-Sent Events stream for ACK delivery.
//!
//! `GET /v1/events/sse` is the bead's literal spec.
//!
//! Same architectural shape as [`audit_log`](crate::audit_log): a
//! framework-free generator + handler that the future HTTP layer
//! (T-134 API gateway) wraps in an axum/hyper SSE responder. The
//! generator hands the HTTP layer a stream of [`SseFrame`] values;
//! each frame already carries its `id:` / `event:` / `data:` lines
//! and only needs to be flushed to the wire.
//!
//! Reconnect handling follows the SSE spec: the browser sends the
//! last successfully-processed event id as the `Last-Event-ID` header
//! on the new connection (or as the `lastEventId` query param when
//! `EventSource` polyfills can't set headers). The handler decodes
//! that id with [`parse_last_event_id`] and calls
//! [`AckEventStream::events_since`] to replay anything the client
//! missed, then continues with the live tail.
//!
//! Tenant isolation: every connection carries a
//! [`TenantRequestContext`]. The handler refuses to stream events
//! whose tenant doesn't match the caller, surfacing
//! [`SseError::CrossTenantRequest`] so the HTTP layer can return 403
//! before opening the connection.

#![allow(
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::significant_drop_tightening
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{TenantId, TenantRequestContext};

/// Bead identifier carried in log records.
pub const SSE_ACK_BEAD_ID: &str = "invoices-t-077-sse-ack-stream-hlv";

/// HTTP header SSE clients send on reconnect.
pub const LAST_EVENT_ID_HEADER: &str = "last-event-id";

/// Query-string fallback for `EventSource` polyfills that can't set
/// the header.
pub const LAST_EVENT_ID_QUERY_PARAM: &str = "lastEventId";

/// Default keep-alive cadence.
///
/// Browsers and proxies time idle SSE connections out at around 90
/// seconds; we emit a comment frame every 30 seconds to keep them
/// alive while still pacing well below any reasonable rate limit.
pub const DEFAULT_KEEPALIVE_SECONDS: u32 = 30;

/// One ACK event published to a tenant's stream. The state-machine
/// (T-073) is the producer; this type is the on-the-wire shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AckEvent {
    /// Monotonic id within a tenant's stream. Serialized into the
    /// SSE `id:` line and what the client echoes back as
    /// `Last-Event-ID` on reconnect.
    pub event_id: String,
    /// Tenant owning the event.
    pub tenant_id: TenantId,
    /// State-machine kind (e.g. `acknowledged`, `rejected`,
    /// `delivered`). Drives the SSE `event:` line.
    pub kind: String,
    /// Outbox row this event refers back to.
    pub outbox_id: String,
    /// Trace identifier for cross-system correlation.
    pub trace_id: String,
    /// RFC 3339 timestamp the state change was recorded at.
    pub occurred_at: String,
    /// Free-form metadata; the producer is responsible for keeping
    /// secrets out of this map.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// One SSE frame ready to be written to the wire.
///
/// The exact wire bytes are accessible via [`SseFrame::as_text`]; the
/// HTTP layer concatenates the bytes and flushes them as
/// `Content-Type: text/event-stream`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SseFrame {
    /// Event id stamped into the frame's `id:` line.
    pub event_id: String,
    /// Event kind stamped into the frame's `event:` line.
    pub kind: String,
    /// Raw text body of the frame, terminated with a blank line per
    /// the SSE spec.
    pub body: String,
}

impl SseFrame {
    /// Returns the wire bytes as a string slice.
    #[must_use]
    pub fn as_text(&self) -> &str {
        &self.body
    }

    /// Builds an SSE keep-alive comment frame.
    ///
    /// Browsers and intermediaries treat a line starting with `:` as
    /// a comment; this is the documented SSE-spec idiom for
    /// "connection-alive ping" that won't fire an `onmessage`.
    #[must_use]
    pub fn keepalive() -> Self {
        Self {
            event_id: String::new(),
            kind: String::new(),
            body: ": keepalive\n\n".to_owned(),
        }
    }
}

/// Errors raised by the SSE handler.
#[derive(Debug, Error)]
pub enum SseError {
    /// Caller asked to stream a tenant they don't own.
    #[error(
        "cross-tenant SSE request: caller tenant {caller:?} does not match query tenant {requested:?}"
    )]
    CrossTenantRequest {
        /// Calling tenant.
        caller: String,
        /// Requested tenant.
        requested: String,
    },
    /// `Last-Event-ID` value is malformed.
    #[error("Last-Event-ID {value:?} is not a recognized cursor")]
    BadLastEventId {
        /// Offending value.
        value: String,
    },
    /// Backend store failed.
    #[error("ack event store error: {0}")]
    Store(String),
}

/// Source of ACK events. Real impls wrap the T-073 state machine's
/// outbox-state-change topic; the in-memory impl below powers tests.
pub trait AckEventStream: Send + Sync {
    /// Append a new event to the tenant's stream. Idempotent on
    /// `(tenant_id, event_id)` — the producer can safely retry.
    ///
    /// # Errors
    ///
    /// Returns [`SseError::Store`] on backend failure.
    fn publish(&self, event: AckEvent) -> Result<(), SseError>;

    /// Return every event for `tenant` whose `event_id` sorts strictly
    /// after `after`, up to `limit`. Used both for `events_since`
    /// replay on reconnect and for the regular tail.
    ///
    /// # Errors
    ///
    /// Returns [`SseError::Store`] on backend failure.
    fn events_since(
        &self,
        tenant: &TenantId,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AckEvent>, SseError>;
}

/// In-memory stream for tests and lightweight deployments.
#[derive(Debug, Default)]
pub struct InMemoryAckStream {
    events: Mutex<Vec<AckEvent>>,
}

impl InMemoryAckStream {
    /// New empty stream.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored events. Cheap; only used by tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Whether the stream holds zero events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AckEventStream for InMemoryAckStream {
    fn publish(&self, event: AckEvent) -> Result<(), SseError> {
        let mut events = self
            .events
            .lock()
            .map_err(|e| SseError::Store(format!("lock poisoned: {e}")))?;
        if events
            .iter()
            .any(|e| e.tenant_id == event.tenant_id && e.event_id == event.event_id)
        {
            return Ok(());
        }
        events.push(event);
        Ok(())
    }

    fn events_since(
        &self,
        tenant: &TenantId,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AckEvent>, SseError> {
        let events = self
            .events
            .lock()
            .map_err(|e| SseError::Store(format!("lock poisoned: {e}")))?;
        let mut tenant_events: Vec<&AckEvent> =
            events.iter().filter(|e| &e.tenant_id == tenant).collect();
        // Strict monotonic-by-event_id order; ties broken by occurred_at
        // for human readability.
        tenant_events.sort_by(|a, b| {
            a.event_id
                .cmp(&b.event_id)
                .then_with(|| a.occurred_at.cmp(&b.occurred_at))
        });
        let cutoff: Vec<&AckEvent> = match after {
            Some(cursor) => tenant_events
                .into_iter()
                .skip_while(|e| e.event_id.as_str() <= cursor)
                .collect(),
            None => tenant_events,
        };
        Ok(cutoff.into_iter().take(limit.max(1)).cloned().collect())
    }
}

/// Pre-stream validation: the caller's tenant must match the
/// requested one, and any `Last-Event-ID` cursor must be valid.
///
/// Returns the (possibly resolved) reconnect cursor; the HTTP layer
/// uses this to call [`AckEventStream::events_since`].
///
/// # Errors
///
/// Returns [`SseError::CrossTenantRequest`] when the calling
/// tenant doesn't own the requested one, [`SseError::BadLastEventId`]
/// when the supplied cursor is empty.
pub fn validate_subscription(
    ctx: &TenantRequestContext,
    requested_tenant: &TenantId,
    last_event_id: Option<&str>,
) -> Result<Option<String>, SseError> {
    if &ctx.tenant_id != requested_tenant {
        return Err(SseError::CrossTenantRequest {
            caller: ctx.tenant_id.as_str().to_owned(),
            requested: requested_tenant.as_str().to_owned(),
        });
    }
    if let Some(s) = last_event_id {
        if s.is_empty() {
            return Err(SseError::BadLastEventId {
                value: s.to_owned(),
            });
        }
    }
    Ok(last_event_id.map(str::to_owned))
}

/// Build the initial reconnect-replay payload: every event the
/// client missed since `last_event_id`, capped at `limit`.
///
/// # Errors
///
/// Propagates [`SseError::Store`] from the backend.
pub fn build_reconnect_payload(
    stream: &dyn AckEventStream,
    tenant: &TenantId,
    last_event_id: Option<&str>,
    limit: usize,
) -> Result<Vec<SseFrame>, SseError> {
    let events = stream.events_since(tenant, last_event_id, limit)?;
    Ok(events.into_iter().map(format_event_frame).collect())
}

/// Render one [`AckEvent`] as an SSE frame.
///
/// # Panics
///
/// Panics only via the internal `serde_json` expect, which would
/// indicate a malformed event (every field is `Serialize` so this
/// can only fire on a process-wide allocator failure).
#[must_use]
pub fn format_event_frame(event: AckEvent) -> SseFrame {
    let payload = serde_json::to_string(&event).expect("AckEvent must serialize to JSON");
    let mut body = String::with_capacity(payload.len() + 64);
    let _ = writeln!(body, "id: {}", escape_sse_field(&event.event_id));
    let _ = writeln!(body, "event: {}", escape_sse_field(&event.kind));
    for line in payload.lines() {
        let _ = writeln!(body, "data: {line}");
    }
    body.push('\n');
    SseFrame {
        event_id: event.event_id,
        kind: event.kind,
        body,
    }
}

/// Decode the reconnect cursor from request headers and query params.
///
/// Header wins over query param because the HTTP/SSE spec puts the
/// header in the normative path; the query param is just a polyfill
/// for environments where browsers can't add headers to `EventSource`.
#[must_use]
pub fn parse_last_event_id(
    headers: &BTreeMap<String, String>,
    query: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(v) = headers.get(LAST_EVENT_ID_HEADER) {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }
    if let Some(v) = headers.get("Last-Event-ID") {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }
    query
        .get(LAST_EVENT_ID_QUERY_PARAM)
        .filter(|v| !v.is_empty())
        .cloned()
}

fn escape_sse_field(s: &str) -> String {
    // SSE spec forbids line terminators inside an event id / type;
    // we strip CR + LF to match the browser's parser behavior.
    s.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Actor, TraceId};

    fn tenant(s: &str) -> TenantId {
        TenantId::new(s).unwrap()
    }

    fn ctx_for(tenant_name: &str) -> TenantRequestContext {
        TenantRequestContext::new(
            tenant(tenant_name),
            TraceId::new(format!("trace_{tenant_name}")).unwrap(),
            Actor::System {
                name: "test".to_owned(),
            },
        )
    }

    fn event(tenant_name: &str, n: u32, kind: &str) -> AckEvent {
        AckEvent {
            event_id: format!("evt_{n:09}"),
            tenant_id: tenant(tenant_name),
            kind: kind.to_owned(),
            outbox_id: format!("ob_{n}"),
            trace_id: format!("trace_{tenant_name}_{n}"),
            occurred_at: format!("2026-05-27T00:00:{n:02}Z"),
            metadata: BTreeMap::new(),
        }
    }

    fn seed(stream: &InMemoryAckStream) {
        for n in 1..=5 {
            stream
                .publish(event("tenant_a", n, "acknowledged"))
                .unwrap();
        }
        stream
            .publish(event("tenant_b", 99, "acknowledged"))
            .unwrap();
    }

    #[test]
    fn publish_is_idempotent_on_tenant_and_event_id() {
        let stream = InMemoryAckStream::new();
        let e = event("tenant_a", 1, "acknowledged");
        stream.publish(e.clone()).unwrap();
        stream.publish(e.clone()).unwrap();
        stream.publish(e).unwrap();
        assert_eq!(stream.len(), 1);
    }

    #[test]
    fn events_since_filters_by_tenant() {
        let stream = InMemoryAckStream::new();
        seed(&stream);
        let evs = stream.events_since(&tenant("tenant_a"), None, 100).unwrap();
        assert_eq!(evs.len(), 5);
        for ev in evs {
            assert_eq!(ev.tenant_id.as_str(), "tenant_a");
        }
    }

    #[test]
    fn events_since_replays_only_after_cursor() {
        let stream = InMemoryAckStream::new();
        seed(&stream);
        let evs = stream
            .events_since(&tenant("tenant_a"), Some("evt_000000003"), 100)
            .unwrap();
        let ids: Vec<_> = evs.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["evt_000000004", "evt_000000005"]);
    }

    #[test]
    fn events_since_respects_limit() {
        let stream = InMemoryAckStream::new();
        seed(&stream);
        let evs = stream.events_since(&tenant("tenant_a"), None, 2).unwrap();
        assert_eq!(evs.len(), 2);
    }

    #[test]
    fn validate_subscription_rejects_cross_tenant_request() {
        let ctx = ctx_for("tenant_a");
        let err = validate_subscription(&ctx, &tenant("tenant_b"), None).unwrap_err();
        assert!(matches!(err, SseError::CrossTenantRequest { .. }));
    }

    #[test]
    fn validate_subscription_rejects_blank_cursor() {
        let ctx = ctx_for("tenant_a");
        let err = validate_subscription(&ctx, &tenant("tenant_a"), Some("")).unwrap_err();
        assert!(matches!(err, SseError::BadLastEventId { .. }));
    }

    #[test]
    fn validate_subscription_returns_cursor_when_present() {
        let ctx = ctx_for("tenant_a");
        let out = validate_subscription(&ctx, &tenant("tenant_a"), Some("evt_42")).unwrap();
        assert_eq!(out.as_deref(), Some("evt_42"));
    }

    #[test]
    fn format_event_frame_emits_spec_compliant_body() {
        let frame = format_event_frame(event("tenant_a", 7, "delivered"));
        let lines: Vec<&str> = frame.body.lines().collect();
        assert_eq!(lines[0], "id: evt_000000007");
        assert_eq!(lines[1], "event: delivered");
        assert!(lines[2].starts_with("data: "));
        // The frame must end with the SSE-required blank line.
        assert!(frame.body.ends_with("\n\n"));
    }

    #[test]
    fn format_event_frame_strips_line_terminators_from_id_and_kind() {
        let mut e = event("tenant_a", 1, "delivered");
        e.event_id = "bad\nid".to_owned();
        e.kind = "ok\rkind".to_owned();
        let frame = format_event_frame(e);
        assert!(frame.body.starts_with("id: bad id\n"));
        assert!(frame.body.contains("event: ok kind\n"));
    }

    #[test]
    fn keepalive_frame_is_an_sse_comment() {
        let frame = SseFrame::keepalive();
        assert!(frame.body.starts_with(": "));
        assert!(frame.body.ends_with("\n\n"));
    }

    #[test]
    fn reconnect_payload_replays_missed_events_only() {
        let stream = InMemoryAckStream::new();
        seed(&stream);
        let frames =
            build_reconnect_payload(&stream, &tenant("tenant_a"), Some("evt_000000003"), 100)
                .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].body.starts_with("id: evt_000000004\n"));
        assert!(frames[1].body.starts_with("id: evt_000000005\n"));
    }

    #[test]
    fn parse_last_event_id_prefers_header_over_query() {
        let mut headers = BTreeMap::new();
        headers.insert(LAST_EVENT_ID_HEADER.to_owned(), "from_header".to_owned());
        let mut query = BTreeMap::new();
        query.insert(
            LAST_EVENT_ID_QUERY_PARAM.to_owned(),
            "from_query".to_owned(),
        );
        let resolved = parse_last_event_id(&headers, &query);
        assert_eq!(resolved.as_deref(), Some("from_header"));
    }

    #[test]
    fn parse_last_event_id_falls_back_to_query_when_header_absent() {
        let headers = BTreeMap::new();
        let mut query = BTreeMap::new();
        query.insert(
            LAST_EVENT_ID_QUERY_PARAM.to_owned(),
            "from_query".to_owned(),
        );
        let resolved = parse_last_event_id(&headers, &query);
        assert_eq!(resolved.as_deref(), Some("from_query"));
    }

    #[test]
    fn parse_last_event_id_returns_none_when_blank() {
        let mut headers = BTreeMap::new();
        headers.insert(LAST_EVENT_ID_HEADER.to_owned(), String::new());
        let query = BTreeMap::new();
        assert!(parse_last_event_id(&headers, &query).is_none());
    }
}
