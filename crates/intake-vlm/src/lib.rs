// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! InvoiceKit intake **VLM** substrate (Layer 5 — Qwen2.5-VL-7B).
//!
//! The vision-language-model layer of the intake pipeline.
//! Layer 5 is the most expensive layer per PLAN.md §3.5;
//! the engine routes to it only when Layers 1, 2, 3, and 4
//! (digital PDF, Factur-X, PaddleOCR, SmolDocling) failed
//! to reach acceptable confidence.
//!
//! This crate ships the typed surface, an injectable
//! [`VlmTransport`] trait so a follow-up `intake-vlm-http`
//! crate can plug in a real `reqwest` client without
//! changing the engine, a retry-aware [`Qwen25Vl7bProvider`]
//! with cost telemetry, and deterministic mocks for tests.

#![allow(clippy::doc_markdown)]

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

/// Which VLM model the engine targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VlmModel {
    /// Qwen2.5-VL-7B (the L5 default).
    Qwen25Vl7b,
    /// Mock — test stub.
    Mock,
}

/// One typed extraction the VLM emits per invoice field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VlmField {
    /// EN16931 BT/BG term id (e.g. `BT-1`, `BG-7`).
    pub term: String,
    /// Extracted value as a UTF-8 string. Numeric fields are
    /// strings to preserve the issuer's formatting.
    pub value: String,
    /// Model self-reported confidence in [0.0, 1.0].
    pub confidence: f32,
}

/// Aggregate VLM extraction result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VlmResult {
    /// Model that produced the result.
    pub model: VlmModel,
    /// Extracted fields in document order.
    pub fields: Vec<VlmField>,
    /// Mean field confidence (0.0 when no fields).
    pub mean_confidence: f32,
    /// Tokens billed by the provider (live impl populates;
    /// mock returns 0).
    pub billed_tokens: u64,
    /// Per-call cost in micro-USD (1e-6 USD units, so
    /// `1_500` = $0.0015). Lossless across the wire and
    /// the engine's accounting integer.
    pub cost_micro_usd: u64,
}

/// Typed errors raised by [`VlmProvider`] implementations.
#[derive(Debug, Error)]
pub enum VlmError {
    /// Source bytes were not a parseable PDF/image.
    #[error("source bytes rejected: {0}")]
    BadSource(String),
    /// Cloud inference provider refused.
    #[error("provider failure: {0}")]
    Provider(String),
    /// Authentication / authorization failed.
    #[error("auth failure: {0}")]
    Auth(String),
    /// Request exceeded the deadline.
    #[error("timeout after {0}ms")]
    Timeout(u64),
    /// Rate-limited; the retry policy gave up.
    #[error("rate limited; retry-after {0}s")]
    RateLimited(u32),
}

/// VLM extraction surface.
pub trait VlmProvider: Send + Sync {
    /// Which model this provider implements.
    fn model(&self) -> VlmModel;

    /// Extract typed invoice fields from `source_bytes`
    /// (PDF/PNG/JPEG).
    ///
    /// # Errors
    ///
    /// Returns one of the [`VlmError`] variants on parse
    /// failure, cloud-provider failure, auth failure,
    /// timeout, or rate limit exhaustion.
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError>;
}

/// Single response from a [`VlmTransport`].
#[derive(Clone, Debug, PartialEq)]
pub struct TransportResponse {
    /// Extracted fields the upstream model returned.
    pub fields: Vec<VlmField>,
    /// Tokens billed by the provider.
    pub billed_tokens: u64,
    /// Per-call cost in micro-USD.
    pub cost_micro_usd: u64,
}

/// Outcome of a single transport attempt.
#[derive(Debug)]
pub enum TransportOutcome {
    /// Successful response.
    Ok(TransportResponse),
    /// HTTP 429 (or equivalent) with the upstream's
    /// `Retry-After` hint in seconds.
    RateLimited {
        /// Server-suggested seconds to wait before retry.
        retry_after_secs: u32,
    },
    /// Request did not complete inside the deadline.
    Timeout {
        /// Effective deadline in milliseconds.
        elapsed_ms: u64,
    },
    /// HTTP 401/403 (or equivalent) — credentials are bad.
    Auth(String),
    /// Any other provider-side failure.
    Provider(String),
}

/// Injectable transport seam.
///
/// The live impl lives in a follow-up `intake-vlm-http`
/// crate and wraps `reqwest` with the operator's chosen
/// endpoint URL + API key. Tests pass a scripted in-process
/// transport with a sequence of [`TransportOutcome`] values.
pub trait VlmTransport: Send + Sync {
    /// Execute one attempt against the upstream endpoint.
    fn call(&self, source_bytes: &[u8]) -> TransportOutcome;
}

/// Retry/backoff policy for [`Qwen25Vl7bProvider`].
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Maximum attempts (including the first). Must be ≥ 1.
    pub max_attempts: u32,
    /// Maximum total seconds to honour upstream
    /// `Retry-After` hints across the whole call.
    pub max_total_backoff_secs: u32,
}

impl RetryPolicy {
    /// Production default: 3 attempts, total backoff capped
    /// at 60 seconds.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_attempts: 3,
            max_total_backoff_secs: 60,
        }
    }

    /// Single-attempt policy for tests that must not retry.
    #[must_use]
    pub const fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            max_total_backoff_secs: 0,
        }
    }
}

/// One emitted telemetry row per successful provider call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostTelemetry {
    /// Model that ran.
    pub model: VlmModel,
    /// Billed tokens.
    pub billed_tokens: u64,
    /// Per-call cost in micro-USD.
    pub cost_micro_usd: u64,
    /// Attempts taken (≥ 1).
    pub attempts: u32,
}

/// Pluggable telemetry sink. The default implementation
/// emits a `tracing::info` event so operators can scrape it
/// with their existing pipeline; tests use
/// [`InMemoryTelemetry`] to assert the row directly.
pub trait CostTelemetrySink: Send + Sync {
    /// Record one billed call.
    fn record(&self, row: CostTelemetry);
}

/// `tracing::info` sink used in production.
#[derive(Default)]
pub struct TracingTelemetry;

impl CostTelemetrySink for TracingTelemetry {
    fn record(&self, row: CostTelemetry) {
        tracing::info!(
            target: "invoicekit_intake_vlm::cost",
            model = ?row.model,
            billed_tokens = row.billed_tokens,
            cost_micro_usd = row.cost_micro_usd,
            attempts = row.attempts,
            "vlm-call",
        );
    }
}

/// In-memory sink for assertions.
#[derive(Default)]
pub struct InMemoryTelemetry {
    rows: Mutex<Vec<CostTelemetry>>,
}

impl InMemoryTelemetry {
    /// Snapshot the rows recorded so far.
    ///
    /// # Panics
    ///
    /// Panics if another thread panicked while holding the
    /// internal mutex. Single-threaded callers (the common
    /// case) cannot trigger this.
    #[must_use]
    pub fn snapshot(&self) -> Vec<CostTelemetry> {
        self.rows.lock().unwrap().clone()
    }
}

impl CostTelemetrySink for InMemoryTelemetry {
    fn record(&self, row: CostTelemetry) {
        self.rows.lock().unwrap().push(row);
    }
}

/// Live-bound Qwen2.5-VL-7B provider.
///
/// Wraps a [`VlmTransport`] with retry-on-rate-limit, a
/// hard deadline, auth-failure surfacing, and cost
/// telemetry.
pub struct Qwen25Vl7bProvider {
    /// HTTPS endpoint the live transport will POST to. Kept
    /// here so operators can validate config before wiring a
    /// transport.
    pub endpoint_url: String,
    /// Operator's per-tenant API key (kept opaque). Kept
    /// here for the same reason.
    pub api_key_ref: String,
    transport: Box<dyn VlmTransport>,
    telemetry: Box<dyn CostTelemetrySink>,
    retry_policy: RetryPolicy,
}

impl Qwen25Vl7bProvider {
    /// Build a production-shaped provider.
    #[must_use]
    pub fn new(
        endpoint_url: String,
        api_key_ref: String,
        transport: Box<dyn VlmTransport>,
    ) -> Self {
        Self {
            endpoint_url,
            api_key_ref,
            transport,
            telemetry: Box::new(TracingTelemetry),
            retry_policy: RetryPolicy::production(),
        }
    }

    /// Swap in a custom telemetry sink.
    #[must_use]
    pub fn with_telemetry(mut self, sink: Box<dyn CostTelemetrySink>) -> Self {
        self.telemetry = sink;
        self
    }

    /// Override the retry policy.
    #[must_use]
    pub const fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }
}

impl VlmProvider for Qwen25Vl7bProvider {
    fn model(&self) -> VlmModel {
        VlmModel::Qwen25Vl7b
    }
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError> {
        if source_bytes.is_empty() {
            return Err(VlmError::BadSource("source is empty".to_owned()));
        }
        if self.endpoint_url.is_empty() {
            return Err(VlmError::Provider("endpoint_url is empty".to_owned()));
        }
        if self.api_key_ref.is_empty() {
            return Err(VlmError::Auth("api_key_ref is empty".to_owned()));
        }

        let max_attempts = self.retry_policy.max_attempts.max(1);
        let max_total_backoff = u64::from(self.retry_policy.max_total_backoff_secs);
        let mut backoff_used_secs: u64 = 0;
        let mut last_rate_limit: u32 = 0;

        for attempt in 1..=max_attempts {
            match self.transport.call(source_bytes) {
                TransportOutcome::Ok(resp) => {
                    let mean = mean_confidence(&resp.fields);
                    let row = CostTelemetry {
                        model: VlmModel::Qwen25Vl7b,
                        billed_tokens: resp.billed_tokens,
                        cost_micro_usd: resp.cost_micro_usd,
                        attempts: attempt,
                    };
                    self.telemetry.record(row);
                    return Ok(VlmResult {
                        model: VlmModel::Qwen25Vl7b,
                        fields: resp.fields,
                        mean_confidence: mean,
                        billed_tokens: resp.billed_tokens,
                        cost_micro_usd: resp.cost_micro_usd,
                    });
                }
                TransportOutcome::RateLimited { retry_after_secs } => {
                    last_rate_limit = retry_after_secs;
                    debug!(
                        attempt,
                        retry_after_secs, "qwen25-vl rate limited; considering retry"
                    );
                    let next = backoff_used_secs.saturating_add(u64::from(retry_after_secs));
                    if attempt >= max_attempts || next > max_total_backoff {
                        warn!(
                            attempt,
                            retry_after_secs,
                            max_attempts,
                            max_total_backoff,
                            "qwen25-vl rate-limit retries exhausted",
                        );
                        return Err(VlmError::RateLimited(retry_after_secs));
                    }
                    backoff_used_secs = next;
                    sleep_seconds(retry_after_secs);
                }
                TransportOutcome::Timeout { elapsed_ms } => {
                    return Err(VlmError::Timeout(elapsed_ms));
                }
                TransportOutcome::Auth(msg) => {
                    return Err(VlmError::Auth(msg));
                }
                TransportOutcome::Provider(msg) => {
                    return Err(VlmError::Provider(msg));
                }
            }
        }
        Err(VlmError::RateLimited(last_rate_limit))
    }
}

fn sleep_seconds(secs: u32) {
    if cfg!(test) {
        // Tests assert on the retry path without actually
        // sleeping; the policy itself is what they verify.
        return;
    }
    std::thread::sleep(Duration::from_secs(u64::from(secs)));
}

fn mean_confidence(fields: &[VlmField]) -> f32 {
    if fields.is_empty() {
        return 0.0;
    }
    let count = u32::try_from(fields.len()).unwrap_or(1).max(1);
    #[allow(clippy::cast_precision_loss)]
    let denom = count as f32;
    fields.iter().map(|f| f.confidence).sum::<f32>() / denom
}

/// Deterministic mock provider used in engine wiring tests.
pub struct MockVlmProvider;

impl VlmProvider for MockVlmProvider {
    fn model(&self) -> VlmModel {
        VlmModel::Mock
    }
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError> {
        if source_bytes.is_empty() {
            return Err(VlmError::BadSource("source is empty".to_owned()));
        }
        Ok(stub_extract(VlmModel::Mock))
    }
}

fn stub_extract(model: VlmModel) -> VlmResult {
    let fields = vec![
        VlmField {
            term: "BT-1".to_owned(),
            value: "INV-MOCK-1".to_owned(),
            confidence: 0.95,
        },
        VlmField {
            term: "BT-2".to_owned(),
            value: "2026-05-28".to_owned(),
            confidence: 0.92,
        },
        VlmField {
            term: "BT-5".to_owned(),
            value: "EUR".to_owned(),
            confidence: 0.99,
        },
    ];
    let mean = mean_confidence(&fields);
    VlmResult {
        model,
        fields,
        mean_confidence: mean,
        billed_tokens: 0,
        cost_micro_usd: 0,
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_intake_vlm::crate_name(), "invoicekit-intake-vlm");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-vlm"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Test transport scripted by a sequence of outcomes.
    struct ScriptedTransport {
        script: StdMutex<Vec<TransportOutcome>>,
    }

    impl ScriptedTransport {
        fn new(outcomes: Vec<TransportOutcome>) -> Self {
            Self {
                script: StdMutex::new(outcomes),
            }
        }
    }

    impl VlmTransport for ScriptedTransport {
        fn call(&self, _source: &[u8]) -> TransportOutcome {
            let mut q = self.script.lock().unwrap();
            if q.is_empty() {
                TransportOutcome::Provider("script exhausted".to_owned())
            } else {
                q.remove(0)
            }
        }
    }

    fn ok_response() -> TransportResponse {
        TransportResponse {
            fields: vec![
                VlmField {
                    term: "BT-1".to_owned(),
                    value: "INV-LIVE-7".to_owned(),
                    confidence: 0.91,
                },
                VlmField {
                    term: "BT-2".to_owned(),
                    value: "2026-05-28".to_owned(),
                    confidence: 0.88,
                },
            ],
            billed_tokens: 1_280,
            cost_micro_usd: 1_500,
        }
    }

    #[test]
    fn mock_provider_returns_three_fields() {
        let r = MockVlmProvider.extract(b"%PDF-1.4").unwrap();
        assert_eq!(r.model, VlmModel::Mock);
        assert_eq!(r.fields.len(), 3);
        assert!(r.mean_confidence > 0.9);
        assert_eq!(r.cost_micro_usd, 0);
    }

    #[test]
    fn mock_provider_rejects_empty_source() {
        let err = MockVlmProvider.extract(b"").unwrap_err();
        assert!(matches!(err, VlmError::BadSource(_)));
    }

    struct ArcSink(std::sync::Arc<InMemoryTelemetry>);

    impl CostTelemetrySink for ArcSink {
        fn record(&self, row: CostTelemetry) {
            self.0.record(row);
        }
    }

    #[test]
    fn qwen_provider_happy_path_records_cost_telemetry() {
        let transport = ScriptedTransport::new(vec![TransportOutcome::Ok(ok_response())]);
        let recorded = std::sync::Arc::new(InMemoryTelemetry::default());
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz/v1/chat/completions".to_owned(),
            "secret-ref:tenant-1".to_owned(),
            Box::new(transport),
        )
        .with_telemetry(Box::new(ArcSink(recorded.clone())));
        let r = p.extract(b"%PDF-1.4").unwrap();
        assert_eq!(r.model, VlmModel::Qwen25Vl7b);
        assert_eq!(r.fields.len(), 2);
        assert_eq!(r.billed_tokens, 1_280);
        assert_eq!(r.cost_micro_usd, 1_500);
        let rows = recorded.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].billed_tokens, 1_280);
        assert_eq!(rows[0].cost_micro_usd, 1_500);
    }

    #[test]
    fn qwen_provider_retries_rate_limit_then_succeeds() {
        let transport = ScriptedTransport::new(vec![
            TransportOutcome::RateLimited {
                retry_after_secs: 1,
            },
            TransportOutcome::Ok(ok_response()),
        ]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            "x".to_owned(),
            Box::new(transport),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            max_total_backoff_secs: 10,
        });
        let r = p.extract(b"%PDF-1.4").unwrap();
        assert_eq!(r.billed_tokens, 1_280);
    }

    #[test]
    fn qwen_provider_gives_up_when_rate_limit_exceeds_budget() {
        let transport = ScriptedTransport::new(vec![
            TransportOutcome::RateLimited {
                retry_after_secs: 30,
            },
            TransportOutcome::RateLimited {
                retry_after_secs: 30,
            },
            TransportOutcome::RateLimited {
                retry_after_secs: 30,
            },
        ]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            "x".to_owned(),
            Box::new(transport),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 5,
            max_total_backoff_secs: 20, // < 30
        });
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::RateLimited(30)));
    }

    #[test]
    fn qwen_provider_surfaces_timeout_immediately() {
        let transport =
            ScriptedTransport::new(vec![TransportOutcome::Timeout { elapsed_ms: 30_000 }]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            "x".to_owned(),
            Box::new(transport),
        );
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Timeout(30_000)));
    }

    #[test]
    fn qwen_provider_surfaces_auth_failure_immediately() {
        let transport =
            ScriptedTransport::new(vec![TransportOutcome::Auth("401 unauthorized".to_owned())]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            "x".to_owned(),
            Box::new(transport),
        );
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Auth(_)));
    }

    #[test]
    fn qwen_provider_rejects_empty_endpoint_before_transport() {
        let transport = ScriptedTransport::new(vec![TransportOutcome::Ok(ok_response())]);
        let p = Qwen25Vl7bProvider::new(
            String::new(),
            "secret-ref:tenant-1".to_owned(),
            Box::new(transport),
        );
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Provider(_)));
    }

    #[test]
    fn qwen_provider_rejects_empty_api_key_ref_as_auth() {
        let transport = ScriptedTransport::new(vec![TransportOutcome::Ok(ok_response())]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            String::new(),
            Box::new(transport),
        );
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Auth(_)));
    }

    #[test]
    fn qwen_provider_rejects_empty_source() {
        let transport = ScriptedTransport::new(vec![TransportOutcome::Ok(ok_response())]);
        let p = Qwen25Vl7bProvider::new(
            "https://api.together.xyz".to_owned(),
            "x".to_owned(),
            Box::new(transport),
        );
        let err = p.extract(b"").unwrap_err();
        assert!(matches!(err, VlmError::BadSource(_)));
    }

    #[test]
    fn vlm_result_round_trips_through_serde() {
        let r = VlmResult {
            model: VlmModel::Qwen25Vl7b,
            fields: vec![VlmField {
                term: "BT-1".to_owned(),
                value: "X".to_owned(),
                confidence: 1.0,
            }],
            mean_confidence: 1.0,
            billed_tokens: 42,
            cost_micro_usd: 1_234,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: VlmResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn vlm_error_rate_limited_carries_retry_after() {
        let err = VlmError::RateLimited(30);
        assert!(err.to_string().contains("30"));
    }

    #[test]
    fn in_memory_telemetry_collects_rows_for_assertions() {
        let sink = InMemoryTelemetry::default();
        sink.record(CostTelemetry {
            model: VlmModel::Qwen25Vl7b,
            billed_tokens: 99,
            cost_micro_usd: 7,
            attempts: 2,
        });
        let rows = sink.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].billed_tokens, 99);
    }

    #[test]
    fn retry_policy_production_defaults_are_sane() {
        let p = RetryPolicy::production();
        assert!(p.max_attempts >= 1);
        assert!(p.max_total_backoff_secs > 0);
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-intake-vlm");
    }
}
