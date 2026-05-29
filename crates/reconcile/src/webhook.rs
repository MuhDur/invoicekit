// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-076 webhook dispatcher: HMAC-SHA256 signing, replay protection,
//! idempotency, and at-least-once retry semantics.
//!
//! Signature format follows the InvoiceKit T-132 convention (which itself
//! mirrors the Stripe webhook header shape used industry-wide so customer
//! integrations port their existing verification code):
//!
//! ```text
//! Invoicekit-Signature: t=1716821123,v1=<hex(HMAC-SHA256(secret, "t.payload"))>
//! ```
//!
//! Verification reproduces the HMAC over `"<timestamp>.<body>"` and
//! constant-time compares the two digests. The verifier also enforces a
//! 5-minute replay window (`abs(now - t) <= 300s`) and consults an
//! [`EventIdLedger`] for idempotency: a previously-seen event ID is
//! rejected with [`WebhookVerifyError::DuplicateEvent`] so a retried POST
//! cannot double-fire a downstream side effect.
//!
//! The [`WebhookDispatcher`] drives the at-least-once side: it asks an
//! injected `WebhookTransport` to send the signed envelope, treats
//! transport errors and 5xx responses as retryable, and applies the
//! exponential backoff policy in [`WebhookRetryPolicy`]. After
//! [`WebhookRetryPolicy::max_attempts`] failures the envelope is reported
//! as [`DeliveryOutcome::Exhausted`]; downstream policy (typically the
//! private `outbox` module's dead-letter queue) takes over.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

/// Bead identifier carried alongside emitted log events for diagnostic correlation.
pub const WEBHOOK_BEAD_ID: &str = "invoices-t-076-webhook-dispatcher-h5r";

/// HTTP header name carrying the [`WebhookHeaders::signature`] value.
///
/// Lowercase to match HTTP/2 conventions; receivers should compare
/// case-insensitively per RFC 7230.
pub const SIGNATURE_HEADER: &str = "invoicekit-signature";
/// HTTP header name carrying the [`WebhookHeaders::event_id`] value.
pub const EVENT_ID_HEADER: &str = "invoicekit-event-id";
/// Signature scheme version tag prefix inside [`WebhookHeaders::signature`].
pub const VERSION_TAG: &str = "v1";
/// Replay-window tolerance (seconds) on either side of `now`.
pub const REPLAY_WINDOW_SECONDS: i64 = 300;

type HmacSha256 = Hmac<Sha256>;

/// One webhook event, ready to be signed and dispatched.
///
/// `event_id` must be stable across retries — it's the only thing the
/// receiver-side [`WebhookVerifier::verify`] uses to dedupe replays. The
/// state-machine emitter (T-073) picks a `UUIDv7` today; any stable string
/// works.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookEvent {
    /// Stable identifier for this event (idempotency key).
    pub event_id: String,
    /// Event topic, e.g. `invoice.delivered` or `invoice.rejected`.
    pub topic: String,
    /// Tenant identifier owning the event.
    pub tenant_id: String,
    /// JSON payload (already serialized by the caller).
    pub payload: String,
}

/// Envelope produced by [`WebhookSigner::sign`] and carried to the
/// transport layer. Fields map 1:1 to HTTP request shape so a transport
/// implementation only has to wire them onto its `POST`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebhookEnvelope {
    /// Event being delivered.
    pub event: WebhookEvent,
    /// Headers to attach to the outgoing request.
    pub headers: WebhookHeaders,
    /// Body bytes (UTF-8 text matching `event.payload`).
    pub body: String,
    /// UNIX timestamp the signer used.
    pub timestamp: i64,
}

/// Headers `WebhookEnvelope` will set on the outgoing HTTP request.
///
/// Modelled as concrete fields rather than `BTreeMap<String, String>` so
/// downstream transports cannot accidentally rename them and break
/// signature verification.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WebhookHeaders {
    /// `Invoicekit-Signature: t=<ts>,v1=<hex>`.
    pub signature: String,
    /// `Invoicekit-Event-Id: <event-id>`.
    pub event_id: String,
}

/// HMAC-SHA256 signer for outgoing webhooks. Stateless; safe to share.
pub struct WebhookSigner {
    secret: Vec<u8>,
}

impl WebhookSigner {
    /// Build a signer from a shared secret.
    ///
    /// # Examples
    ///
    /// ```
    /// let signer = invoicekit_reconcile::WebhookSigner::new(b"secret");
    /// let event = invoicekit_reconcile::WebhookEvent {
    ///     event_id: "evt-1".to_owned(),
    ///     topic: "invoice.delivered".to_owned(),
    ///     tenant_id: "tenant-1".to_owned(),
    ///     payload: r#"{"hello":"world"}"#.to_owned(),
    /// };
    /// let env = signer.sign(event, 1_700_000_000);
    /// assert!(env.headers.signature.starts_with("t=1700000000,v1="));
    /// ```
    #[must_use]
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Sign `event` with the given `timestamp` (UNIX seconds, UTC).
    #[must_use]
    pub fn sign(&self, event: WebhookEvent, timestamp: i64) -> WebhookEnvelope {
        let body = event.payload.clone();
        let signature = compute_signature(&self.secret, timestamp, &body);
        let headers = WebhookHeaders {
            signature: format!("t={timestamp},{VERSION_TAG}={signature}"),
            event_id: event.event_id.clone(),
        };
        WebhookEnvelope {
            event,
            headers,
            body,
            timestamp,
        }
    }
}

/// Errors emitted by [`WebhookVerifier::verify`].
#[derive(Debug, Error, Eq, PartialEq)]
pub enum WebhookVerifyError {
    /// Signature header is missing or malformed.
    #[error("webhook signature header is missing or malformed (expected `t=<ts>,v1=<hex>`); hint: forward the `Invoicekit-Signature` header verbatim")]
    BadSignatureHeader,
    /// Signature header is present but the hex digest is the wrong length.
    #[error(
        "webhook signature digest is the wrong length; hint: HMAC-SHA256 hex is exactly 64 chars"
    )]
    BadSignatureLength,
    /// Computed signature does not match the header (tampered body or wrong secret).
    #[error("webhook signature does not match; hint: confirm the receiver shares the same secret and the body has not been re-serialized")]
    SignatureMismatch,
    /// Timestamp is outside the replay window.
    #[error("webhook timestamp is outside the {REPLAY_WINDOW_SECONDS}-second replay window (delta={delta_seconds}s); hint: synchronize clocks (NTP) or refresh the request")]
    ReplayWindowExceeded {
        /// Absolute seconds between `now` and the signed timestamp.
        delta_seconds: i64,
    },
    /// Event ID header is missing.
    #[error("webhook event-id header is missing; hint: forward the `Invoicekit-Event-Id` header verbatim")]
    MissingEventId,
    /// Event ID has been seen before (idempotency).
    #[error("webhook event-id `{event_id}` was already processed (replay or duplicate delivery); hint: return 200 and skip the side effect")]
    DuplicateEvent {
        /// Event identifier that was already in the ledger.
        event_id: String,
    },
}

/// In-memory idempotency ledger for already-processed event IDs.
///
/// Real deployments persist the ledger (Postgres unique index, Redis set,
/// etc.) so a restart does not re-accept replays. This in-memory ledger is
/// the default for tests and lightweight embedded use.
#[derive(Debug, Default)]
pub struct EventIdLedger {
    seen: Mutex<HashSet<String>>,
}

impl EventIdLedger {
    /// Build an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically record `event_id` as seen. Returns `true` when this is
    /// the first time the ledger has seen the id (caller should run the
    /// side effect) and `false` when it is a duplicate.
    ///
    /// # Panics
    ///
    /// Panics if the internal `Mutex` is poisoned. Poisoning only happens
    /// when a different thread panicked while holding the lock; in
    /// production deployments where the dispatcher is the only writer this
    /// is unreachable.
    pub fn record(&self, event_id: &str) -> bool {
        let mut seen = self.seen.lock().expect("EventIdLedger mutex poisoned");
        seen.insert(event_id.to_owned())
    }

    /// True when the ledger has previously recorded `event_id`.
    ///
    /// # Panics
    ///
    /// Panics if the internal `Mutex` is poisoned; see [`Self::record`]
    /// for the conditions under which that can happen.
    pub fn contains(&self, event_id: &str) -> bool {
        let seen = self.seen.lock().expect("EventIdLedger mutex poisoned");
        seen.contains(event_id)
    }
}

/// HMAC-SHA256 receiver-side verifier with replay-window + idempotency.
pub struct WebhookVerifier<'a> {
    secret: &'a [u8],
    ledger: &'a EventIdLedger,
}

impl<'a> WebhookVerifier<'a> {
    /// Build a verifier from a shared secret and an idempotency ledger.
    #[must_use]
    pub const fn new(secret: &'a [u8], ledger: &'a EventIdLedger) -> Self {
        Self { secret, ledger }
    }

    /// Verify a delivered webhook against the configured policy.
    ///
    /// # Errors
    ///
    /// See [`WebhookVerifyError`] for the variants. On success the event
    /// id is recorded in the ledger so a subsequent retried delivery
    /// returns [`WebhookVerifyError::DuplicateEvent`].
    pub fn verify(
        &self,
        headers: &WebhookHeaders,
        body: &str,
        now: i64,
    ) -> Result<(), WebhookVerifyError> {
        let (timestamp, signature) = parse_signature_header(&headers.signature)?;

        // 5-minute replay window: reject anything older or newer than that.
        // `timestamp` is attacker-controlled (parsed from the signature
        // header), so the magnitude must be computed without `i64::abs`,
        // which panics on `i64::MIN` (reachable via a crafted timestamp).
        // `unsigned_abs` returns the magnitude as a `u64` and never overflows.
        let delta = now.saturating_sub(timestamp);
        let abs_delta = delta.unsigned_abs();
        // `REPLAY_WINDOW_SECONDS` is a small positive constant, so the cast is exact.
        if abs_delta > REPLAY_WINDOW_SECONDS as u64 {
            return Err(WebhookVerifyError::ReplayWindowExceeded {
                delta_seconds: delta,
            });
        }

        // HMAC check using constant-time comparison.
        if signature.len() != 64 {
            return Err(WebhookVerifyError::BadSignatureLength);
        }
        let expected = compute_signature(self.secret, timestamp, body);
        // Compare the hex strings byte-wise in constant time.
        if !constant_time_eq(signature.as_bytes(), expected.as_bytes()) {
            return Err(WebhookVerifyError::SignatureMismatch);
        }

        // Idempotency check: header must be present and unseen.
        if headers.event_id.is_empty() {
            return Err(WebhookVerifyError::MissingEventId);
        }
        if !self.ledger.record(&headers.event_id) {
            return Err(WebhookVerifyError::DuplicateEvent {
                event_id: headers.event_id.clone(),
            });
        }
        Ok(())
    }
}

/// Decision the dispatcher hands back to the caller after one delivery
/// attempt completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookRetryDecision {
    /// Delivery completed successfully; no further attempts needed.
    Done,
    /// Delivery failed but the policy allows another attempt after the
    /// recommended backoff.
    Retry {
        /// Sleep this long before the next attempt.
        backoff: Duration,
    },
    /// Delivery failed and `max_attempts` is exhausted; hand off to the
    /// dead-letter queue.
    Exhausted,
}

/// Exponential backoff retry policy.
///
/// Schedule: `min(base * factor^attempt, max)`. The 5xx + transport-error
/// arms feed into this; 4xx responses are *not* retried because the
/// receiver gave a deterministic answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebhookRetryPolicy {
    /// Initial backoff before the first retry.
    pub base: Duration,
    /// Multiplier applied per retry attempt.
    pub factor: u32,
    /// Hard ceiling regardless of `factor.pow(attempt)`.
    pub max: Duration,
    /// Total number of delivery attempts (1 = no retry).
    pub max_attempts: u32,
}

impl WebhookRetryPolicy {
    /// Default Stripe-shape policy: 1s base, 2× factor, 60s ceiling, 6 attempts.
    #[must_use]
    pub const fn stripe_default() -> Self {
        Self {
            base: Duration::from_secs(1),
            factor: 2,
            max: Duration::from_secs(60),
            max_attempts: 6,
        }
    }

    /// Compute the backoff for `attempt` (0-indexed: 0 = before second try).
    #[must_use]
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        let multiplier = self.factor.saturating_pow(attempt);
        let scaled = self.base.saturating_mul(multiplier);
        if scaled > self.max {
            self.max
        } else {
            scaled
        }
    }
}

impl Default for WebhookRetryPolicy {
    fn default() -> Self {
        Self::stripe_default()
    }
}

/// Outcome reported by one full [`WebhookDispatcher::deliver`] call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    /// Successful delivery after `attempts` attempts.
    Delivered {
        /// Number of attempts spent (1 = succeeded on first try).
        attempts: u32,
    },
    /// Permanent client error (4xx). Not retried.
    PermanentFailure {
        /// HTTP status code returned by the receiver.
        status: u16,
        /// Number of attempts spent.
        attempts: u32,
    },
    /// Retry budget exhausted; transport will hand off to the dead-letter
    /// queue.
    Exhausted {
        /// Total attempts before giving up (matches `policy.max_attempts`).
        attempts: u32,
    },
}

/// Receiver-side response classification used by [`WebhookDispatcher`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchResult {
    /// HTTP 2xx — delivery succeeded.
    Success,
    /// HTTP 4xx (other than 408/429) — permanent client error, do not retry.
    PermanentClientError {
        /// HTTP status the receiver returned.
        status: u16,
    },
    /// HTTP 5xx, 408, 429, or any transport-level failure — retry-eligible.
    TransientFailure,
}

/// Trait the dispatcher uses to send envelopes. Implementations can use
/// `reqwest`, `hyper`, `ureq`, or — in tests — a closure that records
/// every attempt.
pub trait WebhookTransport {
    /// Deliver `envelope` synchronously, returning the receiver's reply.
    fn deliver(&mut self, envelope: &WebhookEnvelope) -> DispatchResult;
}

/// Loop-driving dispatcher that combines signing + transport + retry
/// policy into a single `dispatcher.deliver(event)` call.
pub struct WebhookDispatcher<'a> {
    signer: &'a WebhookSigner,
    policy: WebhookRetryPolicy,
}

impl<'a> WebhookDispatcher<'a> {
    /// Build a dispatcher from a signer + retry policy.
    #[must_use]
    pub const fn new(signer: &'a WebhookSigner, policy: WebhookRetryPolicy) -> Self {
        Self { signer, policy }
    }

    /// Drive delivery of `event` through `transport`. `clock` returns the
    /// UNIX timestamp the signer should stamp each attempt with; a
    /// monotonic mock clock makes the test arms reproducible.
    pub fn deliver<T, C>(
        &self,
        event: &WebhookEvent,
        transport: &mut T,
        mut clock: C,
        mut sleeper: impl FnMut(Duration),
    ) -> DeliveryOutcome
    where
        T: WebhookTransport,
        C: FnMut() -> i64,
    {
        let mut attempts = 0_u32;
        loop {
            attempts = attempts.saturating_add(1);
            let envelope = self.signer.sign(event.clone(), clock());
            let result = transport.deliver(&envelope);
            match result {
                DispatchResult::Success => return DeliveryOutcome::Delivered { attempts },
                DispatchResult::PermanentClientError { status } => {
                    return DeliveryOutcome::PermanentFailure { status, attempts };
                }
                DispatchResult::TransientFailure => {
                    if attempts >= self.policy.max_attempts {
                        return DeliveryOutcome::Exhausted { attempts };
                    }
                    sleeper(self.policy.backoff_for(attempts - 1));
                }
            }
        }
    }
}

fn compute_signature(secret: &[u8], timestamp: i64, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let bytes = mac.finalize().into_bytes();
    hex_lower(&bytes)
}

fn parse_signature_header(value: &str) -> Result<(i64, String), WebhookVerifyError> {
    let mut timestamp: Option<i64> = None;
    let mut signature: Option<String> = None;
    for part in value.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("t=") {
            timestamp = rest.parse::<i64>().ok();
        } else if let Some(rest) = part.strip_prefix("v1=") {
            signature = Some(rest.to_owned());
        }
    }
    match (timestamp, signature) {
        (Some(ts), Some(sig)) => Ok((ts, sig)),
        _ => Err(WebhookVerifyError::BadSignatureHeader),
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn fixture_event() -> WebhookEvent {
        WebhookEvent {
            event_id: "evt-1".to_owned(),
            topic: "invoice.delivered".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            payload: r#"{"invoice_id":"INV-1"}"#.to_owned(),
        }
    }

    #[test]
    fn signature_header_uses_v1_prefix() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        assert!(env.headers.signature.starts_with("t=1700000000,v1="));
        assert_eq!(env.headers.event_id, "evt-1");
    }

    #[test]
    fn verifier_accepts_freshly_signed_webhook() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        assert!(verifier
            .verify(&env.headers, &env.body, 1_700_000_000)
            .is_ok());
    }

    #[test]
    fn verifier_rejects_tampered_body() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        let err = verifier
            .verify(&env.headers, "{\"invoice_id\":\"INV-99\"}", 1_700_000_000)
            .unwrap_err();
        assert_eq!(err, WebhookVerifyError::SignatureMismatch);
    }

    #[test]
    fn verifier_rejects_wrong_secret() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"different", &ledger);
        let err = verifier
            .verify(&env.headers, &env.body, 1_700_000_000)
            .unwrap_err();
        assert_eq!(err, WebhookVerifyError::SignatureMismatch);
    }

    #[test]
    fn verifier_rejects_replay_outside_window() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        // 5 minutes + 1 second beyond the window.
        let now = 1_700_000_000 + 301;
        let err = verifier.verify(&env.headers, &env.body, now).unwrap_err();
        assert!(matches!(
            err,
            WebhookVerifyError::ReplayWindowExceeded { delta_seconds: 301 }
        ));
    }

    #[test]
    fn verifier_handles_extreme_timestamp_without_overflow_panic() {
        // Regression: a crafted `t=` header can drive `now - timestamp` to
        // `i64::MIN` under saturating subtraction. The old `delta.abs()`
        // panicked on `i64::MIN` (debug overflow), giving an attacker a
        // denial-of-service via a single malformed webhook. The verifier
        // must instead reject the timestamp as outside the replay window.
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        // 64-char hex placeholder so we reach the window check first.
        let signature = "0".repeat(64);
        let headers = WebhookHeaders {
            signature: format!("t={},v1={signature}", i64::MAX),
            event_id: "evt-overflow".to_owned(),
        };
        // now = -1, timestamp = i64::MAX => saturating_sub == i64::MIN.
        let err = verifier
            .verify(&headers, "body", -1)
            .expect_err("extreme timestamp must be rejected, not panic");
        assert!(matches!(
            err,
            WebhookVerifyError::ReplayWindowExceeded { .. }
        ));
    }

    #[test]
    fn verifier_rejects_duplicate_event_id() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        verifier
            .verify(&env.headers, &env.body, 1_700_000_000)
            .expect("first delivery accepted");
        let err = verifier
            .verify(&env.headers, &env.body, 1_700_000_000)
            .unwrap_err();
        assert!(matches!(err, WebhookVerifyError::DuplicateEvent { .. }));
    }

    #[test]
    fn verifier_rejects_malformed_signature_header() {
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        let headers = WebhookHeaders {
            signature: "garbage".to_owned(),
            event_id: "evt-x".to_owned(),
        };
        let err = verifier
            .verify(&headers, "body", 1_700_000_000)
            .unwrap_err();
        assert_eq!(err, WebhookVerifyError::BadSignatureHeader);
    }

    #[test]
    fn verifier_rejects_missing_event_id() {
        let signer = WebhookSigner::new(b"shh");
        let env = signer.sign(fixture_event(), 1_700_000_000);
        let ledger = EventIdLedger::new();
        let verifier = WebhookVerifier::new(b"shh", &ledger);
        let mut headers = env.headers.clone();
        headers.event_id.clear();
        let err = verifier
            .verify(&headers, &env.body, 1_700_000_000)
            .unwrap_err();
        assert_eq!(err, WebhookVerifyError::MissingEventId);
    }

    #[test]
    fn retry_policy_backoff_caps_at_max() {
        let policy = WebhookRetryPolicy {
            base: Duration::from_secs(1),
            factor: 10,
            max: Duration::from_secs(5),
            max_attempts: 4,
        };
        assert_eq!(policy.backoff_for(0), Duration::from_secs(1));
        assert_eq!(policy.backoff_for(1), Duration::from_secs(5)); // 10 capped to 5
        assert_eq!(policy.backoff_for(10), Duration::from_secs(5)); // huge capped
    }

    #[test]
    fn dispatcher_succeeds_on_first_try() {
        let signer = WebhookSigner::new(b"shh");
        let dispatcher = WebhookDispatcher::new(&signer, WebhookRetryPolicy::stripe_default());
        let mut transport = RecordingTransport::new(vec![DispatchResult::Success]);
        let mut clock_value = 1_700_000_000_i64;
        let outcome = dispatcher.deliver(
            &fixture_event(),
            &mut transport,
            || {
                let v = clock_value;
                clock_value += 1;
                v
            },
            |_| {},
        );
        assert_eq!(outcome, DeliveryOutcome::Delivered { attempts: 1 });
        assert_eq!(transport.attempts(), 1);
    }

    #[test]
    fn dispatcher_retries_until_success() {
        let signer = WebhookSigner::new(b"shh");
        let dispatcher = WebhookDispatcher::new(
            &signer,
            WebhookRetryPolicy {
                base: Duration::from_millis(1),
                factor: 2,
                max: Duration::from_millis(8),
                max_attempts: 5,
            },
        );
        let mut transport = RecordingTransport::new(vec![
            DispatchResult::TransientFailure,
            DispatchResult::TransientFailure,
            DispatchResult::Success,
        ]);
        let mut clock = 1_700_000_000_i64;
        let outcome = dispatcher.deliver(
            &fixture_event(),
            &mut transport,
            || {
                clock += 1;
                clock
            },
            |_| {},
        );
        assert_eq!(outcome, DeliveryOutcome::Delivered { attempts: 3 });
    }

    #[test]
    fn dispatcher_exhausts_retry_budget() {
        let signer = WebhookSigner::new(b"shh");
        let dispatcher = WebhookDispatcher::new(
            &signer,
            WebhookRetryPolicy {
                base: Duration::from_millis(1),
                factor: 2,
                max: Duration::from_millis(8),
                max_attempts: 3,
            },
        );
        let mut transport = RecordingTransport::new(vec![
            DispatchResult::TransientFailure,
            DispatchResult::TransientFailure,
            DispatchResult::TransientFailure,
        ]);
        let mut clock = 1_700_000_000_i64;
        let outcome = dispatcher.deliver(
            &fixture_event(),
            &mut transport,
            || {
                clock += 1;
                clock
            },
            |_| {},
        );
        assert_eq!(outcome, DeliveryOutcome::Exhausted { attempts: 3 });
    }

    #[test]
    fn dispatcher_does_not_retry_permanent_failure() {
        let signer = WebhookSigner::new(b"shh");
        let dispatcher = WebhookDispatcher::new(&signer, WebhookRetryPolicy::stripe_default());
        let mut transport =
            RecordingTransport::new(vec![DispatchResult::PermanentClientError { status: 410 }]);
        let mut clock = 1_700_000_000_i64;
        let outcome = dispatcher.deliver(
            &fixture_event(),
            &mut transport,
            || {
                clock += 1;
                clock
            },
            |_| panic!("sleeper must not be called for permanent failures"),
        );
        assert_eq!(
            outcome,
            DeliveryOutcome::PermanentFailure {
                status: 410,
                attempts: 1
            }
        );
    }

    proptest! {
        /// Signing and verification round-trip across arbitrary payloads
        /// and timestamps inside the replay window.
        #[test]
        fn signed_envelopes_verify(
            ts in 1_700_000_000_i64..1_710_000_000,
            offset in -250_i64..=250_i64,
            payload in "[a-zA-Z0-9 _-]{0,256}",
            event_id in "evt-[a-z0-9]{1,8}",
        ) {
            let signer = WebhookSigner::new(b"secret");
            let event = WebhookEvent {
                event_id,
                topic: "invoice.delivered".to_owned(),
                tenant_id: "tenant".to_owned(),
                payload,
            };
            let env = signer.sign(event, ts);
            let ledger = EventIdLedger::new();
            let verifier = WebhookVerifier::new(b"secret", &ledger);
            prop_assert!(verifier.verify(&env.headers, &env.body, ts + offset).is_ok());
        }

        /// Any timestamp outside the replay window is rejected.
        #[test]
        fn replay_window_is_enforced(
            ts in 1_700_000_000_i64..1_710_000_000,
            outside in prop_oneof![300_i64..=10_000, -10_000_i64..=-300],
        ) {
            let signer = WebhookSigner::new(b"secret");
            let env = signer.sign(fixture_event(), ts);
            let ledger = EventIdLedger::new();
            let verifier = WebhookVerifier::new(b"secret", &ledger);
            let now = ts + outside;
            // Skip cases where outside happens to be exactly within ±300s.
            prop_assume!((now - ts).abs() > REPLAY_WINDOW_SECONDS);
            let result = verifier.verify(&env.headers, &env.body, now);
            let is_replay = matches!(
                result,
                Err(WebhookVerifyError::ReplayWindowExceeded { .. })
            );
            prop_assert!(is_replay);
        }
    }

    /// Test transport that returns canned results from a script.
    struct RecordingTransport {
        script: std::vec::IntoIter<DispatchResult>,
        attempts: u32,
    }

    impl RecordingTransport {
        fn new(script: Vec<DispatchResult>) -> Self {
            Self {
                script: script.into_iter(),
                attempts: 0,
            }
        }
        fn attempts(&self) -> u32 {
            self.attempts
        }
    }

    impl WebhookTransport for RecordingTransport {
        fn deliver(&mut self, _envelope: &WebhookEnvelope) -> DispatchResult {
            self.attempts = self.attempts.saturating_add(1);
            self.script.next().expect("transport script exhausted")
        }
    }
}
