// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

use crate::{
    validate_identifier, GatewayAdapter, GatewayError, GatewayErrorKind, GatewayOperation,
    GatewayReceipt, GatewayRoute, GatewayStatus, OutboxEnvelope, OutboxState, ReconcileError,
    RetryDecision, SubmitRequest,
};
use invoicekit_ir::CommercialDocument;
use serde::{Deserialize, Serialize};
use std::cmp;

/// Bead identifier attached to transmission worker logs.
pub const TRANSMISSION_WORKER_BEAD_ID: &str = "invoices-t-072-transmission-worker-8gt";

/// Per-gateway rate limit enforced before adapter calls.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayRateLimit {
    /// Minimum number of seconds between attempts for this gateway.
    pub min_interval_seconds: u64,
}

impl GatewayRateLimit {
    /// Builds a per-gateway rate limit.
    ///
    /// A value of `0` disables local spacing and leaves throttling entirely to
    /// the gateway adapter.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::GatewayRateLimit;
    ///
    /// let limit = GatewayRateLimit::new(60);
    /// assert_eq!(limit.min_interval_seconds, 60);
    /// ```
    #[must_use]
    pub const fn new(min_interval_seconds: u64) -> Self {
        Self {
            min_interval_seconds,
        }
    }
}

/// Circuit-breaker policy for persistent gateway failures.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CircuitBreakerPolicy {
    /// Number of consecutive persistent failures that opens the circuit.
    pub failure_threshold: u16,
    /// Number of seconds the circuit remains open before a new probe.
    pub open_seconds: u64,
}

impl CircuitBreakerPolicy {
    /// Builds a validated circuit-breaker policy.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidTransmissionWorkerConfig`] when the
    /// threshold or open duration would make the breaker ineffective.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::CircuitBreakerPolicy;
    ///
    /// let policy = CircuitBreakerPolicy::new(3, 300).unwrap();
    /// assert_eq!(policy.failure_threshold, 3);
    /// ```
    pub const fn new(failure_threshold: u16, open_seconds: u64) -> Result<Self, ReconcileError> {
        if failure_threshold == 0 {
            return Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "failure_threshold",
                message: "must be at least one",
                remediation: "set the number of consecutive failures required to open the circuit",
            });
        }
        if open_seconds == 0 {
            return Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "open_seconds",
                message: "must be greater than zero",
                remediation: "set a positive circuit-open duration",
            });
        }
        Ok(Self {
            failure_threshold,
            open_seconds,
        })
    }
}

/// Configuration for one gateway transmission worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionWorkerConfig {
    /// Stable gateway key used in logs, metrics, and breaker state.
    pub gateway_key: String,
    /// Maximum jobs processed by one batch drain.
    pub max_batch_size: usize,
    /// Per-gateway rate limit.
    pub rate_limit: GatewayRateLimit,
    /// Circuit-breaker policy.
    pub circuit_breaker: CircuitBreakerPolicy,
}

impl TransmissionWorkerConfig {
    /// Builds a validated worker configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`],
    /// [`ReconcileError::InvalidIdentifier`], or
    /// [`ReconcileError::InvalidTransmissionWorkerConfig`] when configuration
    /// fields are unsafe.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CircuitBreakerPolicy, GatewayRateLimit, TransmissionWorkerConfig,
    /// };
    ///
    /// let config = TransmissionWorkerConfig::new(
    ///     "mock-peppol",
    ///     10,
    ///     GatewayRateLimit::new(1),
    ///     CircuitBreakerPolicy::new(2, 60).unwrap(),
    /// )
    /// .unwrap();
    /// assert_eq!(config.gateway_key, "mock-peppol");
    /// ```
    pub fn new(
        gateway_key: impl Into<String>,
        max_batch_size: usize,
        rate_limit: GatewayRateLimit,
        circuit_breaker: CircuitBreakerPolicy,
    ) -> Result<Self, ReconcileError> {
        let gateway_key = gateway_key.into();
        validate_identifier(&gateway_key, "gateway_key")?;
        if max_batch_size == 0 {
            return Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "max_batch_size",
                message: "must be at least one",
                remediation: "allow the worker to drain at least one outbox row per batch",
            });
        }
        Ok(Self {
            gateway_key,
            max_batch_size,
            rate_limit,
            circuit_breaker,
        })
    }
}

/// Ready outbox job submitted by the transmission worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionJob {
    /// Durable outbox envelope.
    pub envelope: OutboxEnvelope,
    /// Gateway route selected by routing policy.
    pub route: GatewayRoute,
    /// Validated invoice document being submitted.
    pub document: CommercialDocument,
}

impl TransmissionJob {
    /// Builds a transmission job from an outbox envelope and invoice document.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_ir::CommercialDocument;
    /// # use invoicekit_reconcile::{GatewayRoute, OutboxEnvelope, TransmissionJob};
    /// # fn envelope() -> OutboxEnvelope { loop {} }
    /// # fn document() -> CommercialDocument { loop {} }
    /// let job = TransmissionJob::new(
    ///     envelope(),
    ///     GatewayRoute::new("peppol", "peppol-bis-3", Some("DE")).unwrap(),
    ///     document(),
    /// );
    /// assert_eq!(job.route.route, "peppol");
    /// ```
    #[must_use]
    pub const fn new(
        envelope: OutboxEnvelope,
        route: GatewayRoute,
        document: CommercialDocument,
    ) -> Self {
        Self {
            envelope,
            route,
            document,
        }
    }
}

/// Stable outcome names used in worker logs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransmissionWorkerOutcomeKind {
    /// Adapter accepted the submit request.
    Submitted,
    /// Gateway failure was retryable and the worker scheduled another attempt.
    RetryScheduled,
    /// Gateway failure was terminal or exhausted the retry policy.
    DeadLettered,
    /// Local per-gateway rate limit deferred the job before adapter dispatch.
    RateLimited,
    /// Circuit breaker deferred the job before adapter dispatch.
    CircuitOpen,
}

impl TransmissionWorkerOutcomeKind {
    /// Returns the stable outcome name.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::TransmissionWorkerOutcomeKind;
    ///
    /// assert_eq!(TransmissionWorkerOutcomeKind::CircuitOpen.as_str(), "circuit_open");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::RetryScheduled => "retry_scheduled",
            Self::DeadLettered => "dead_lettered",
            Self::RateLimited => "rate_limited",
            Self::CircuitOpen => "circuit_open",
        }
    }
}

/// Serializable log event emitted for every worker decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TransmissionWorkerLogEvent {
    /// Stable log event name.
    pub event: &'static str,
    /// Bead that introduced the log contract.
    pub bead_id: &'static str,
    /// Gateway key from [`TransmissionWorkerConfig`].
    pub gateway_key: String,
    /// Outbox row identifier.
    pub outbox_id: String,
    /// Tenant ID copied from the gateway context.
    pub tenant_id: String,
    /// Trace ID copied from the gateway context.
    pub trace_id: String,
    /// Idempotency key copied from the gateway context.
    pub idempotency_key: String,
    /// Gateway attempt ID copied from the gateway context.
    pub gateway_attempt_id: String,
    /// Gateway operation attempted by this worker.
    pub operation: GatewayOperation,
    /// Worker decision.
    pub outcome: TransmissionWorkerOutcomeKind,
    /// Retry delay or deferred-until window, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    /// Normalized gateway error kind, when the adapter returned an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_error_kind: Option<GatewayErrorKind>,
}

impl TransmissionWorkerLogEvent {
    /// Serializes the structured event as one JSON log line.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] only if serialization of this static
    /// event shape fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::TransmissionWorkerLogEvent;
    /// # fn event() -> TransmissionWorkerLogEvent { loop {} }
    /// let line = event().to_json_line().unwrap();
    /// assert!(line.contains("tenant_id"));
    /// ```
    pub fn to_json_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Result of one transmission worker decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransmissionWorkerResult {
    /// Gateway accepted the submit request.
    Submitted {
        /// Updated outbox envelope.
        envelope: OutboxEnvelope,
        /// Normalized gateway receipt.
        receipt: GatewayReceipt,
        /// Structured log event.
        log_event: TransmissionWorkerLogEvent,
    },
    /// Gateway failure was retryable and another attempt was scheduled.
    RetryScheduled {
        /// Updated outbox envelope with failed-attempt count.
        envelope: OutboxEnvelope,
        /// Normalized gateway error.
        error: GatewayError,
        /// Delay before the next attempt.
        retry_after_seconds: u64,
        /// Circuit-open deadline if this failure opened the breaker.
        circuit_open_until_seconds: Option<u64>,
        /// Structured log event.
        log_event: TransmissionWorkerLogEvent,
    },
    /// Gateway failure was terminal or exhausted the retry policy.
    DeadLettered {
        /// Updated outbox envelope.
        envelope: OutboxEnvelope,
        /// Durable dead-letter row.
        dead_letter: crate::DeadLetterRecord,
        /// Normalized gateway error.
        error: GatewayError,
        /// Circuit-open deadline if this failure opened the breaker.
        circuit_open_until_seconds: Option<u64>,
        /// Structured log event.
        log_event: TransmissionWorkerLogEvent,
    },
    /// Local rate limit deferred the job before adapter dispatch.
    RateLimited {
        /// Deferred job, unchanged and safe to requeue.
        job: TransmissionJob,
        /// Delay before the job is eligible again.
        retry_after_seconds: u64,
        /// Structured log event.
        log_event: TransmissionWorkerLogEvent,
    },
    /// Circuit breaker deferred the job before adapter dispatch.
    CircuitOpen {
        /// Deferred job, unchanged and safe to requeue.
        job: TransmissionJob,
        /// Delay before the circuit allows a probe.
        retry_after_seconds: u64,
        /// Structured log event.
        log_event: TransmissionWorkerLogEvent,
    },
}

impl TransmissionWorkerResult {
    /// Returns the worker outcome kind for this result.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::{TransmissionWorkerOutcomeKind, TransmissionWorkerResult};
    /// # fn result() -> TransmissionWorkerResult { loop {} }
    /// assert!(matches!(
    ///     result().outcome(),
    ///     TransmissionWorkerOutcomeKind::Submitted
    ///         | TransmissionWorkerOutcomeKind::RetryScheduled
    ///         | TransmissionWorkerOutcomeKind::DeadLettered
    ///         | TransmissionWorkerOutcomeKind::RateLimited
    ///         | TransmissionWorkerOutcomeKind::CircuitOpen
    /// ));
    /// ```
    #[must_use]
    pub const fn outcome(&self) -> TransmissionWorkerOutcomeKind {
        match self {
            Self::Submitted { .. } => TransmissionWorkerOutcomeKind::Submitted,
            Self::RetryScheduled { .. } => TransmissionWorkerOutcomeKind::RetryScheduled,
            Self::DeadLettered { .. } => TransmissionWorkerOutcomeKind::DeadLettered,
            Self::RateLimited { .. } => TransmissionWorkerOutcomeKind::RateLimited,
            Self::CircuitOpen { .. } => TransmissionWorkerOutcomeKind::CircuitOpen,
        }
    }

    /// Returns the structured log event for this result.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::TransmissionWorkerResult;
    /// # fn result() -> TransmissionWorkerResult { loop {} }
    /// assert_eq!(result().log_event().event, "invoicekit.transmission_worker.decision");
    /// ```
    #[must_use]
    pub const fn log_event(&self) -> &TransmissionWorkerLogEvent {
        match self {
            Self::Submitted { log_event, .. }
            | Self::RetryScheduled { log_event, .. }
            | Self::DeadLettered { log_event, .. }
            | Self::RateLimited { log_event, .. }
            | Self::CircuitOpen { log_event, .. } => log_event,
        }
    }
}

/// Deterministic transmission worker for one gateway adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransmissionWorker {
    config: TransmissionWorkerConfig,
    last_attempt_started_at_seconds: Option<u64>,
    consecutive_circuit_failures: u16,
    circuit_open_until_seconds: Option<u64>,
}

impl TransmissionWorker {
    /// Builds a worker from validated configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CircuitBreakerPolicy, GatewayRateLimit, TransmissionWorker,
    ///     TransmissionWorkerConfig,
    /// };
    ///
    /// let worker = TransmissionWorker::new(
    ///     TransmissionWorkerConfig::new(
    ///         "mock-peppol",
    ///         10,
    ///         GatewayRateLimit::new(1),
    ///         CircuitBreakerPolicy::new(2, 60).unwrap(),
    ///     )
    ///     .unwrap(),
    /// );
    /// assert_eq!(worker.config().gateway_key, "mock-peppol");
    /// ```
    #[must_use]
    pub const fn new(config: TransmissionWorkerConfig) -> Self {
        Self {
            config,
            last_attempt_started_at_seconds: None,
            consecutive_circuit_failures: 0,
            circuit_open_until_seconds: None,
        }
    }

    /// Returns this worker's configuration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::TransmissionWorker;
    /// # fn worker() -> TransmissionWorker { loop {} }
    /// assert!(!worker().config().gateway_key.is_empty());
    /// ```
    #[must_use]
    pub const fn config(&self) -> &TransmissionWorkerConfig {
        &self.config
    }

    /// Returns the current circuit-open deadline, if any.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::TransmissionWorker;
    /// # fn worker() -> TransmissionWorker { loop {} }
    /// let _deadline = worker().circuit_open_until_seconds();
    /// ```
    #[must_use]
    pub const fn circuit_open_until_seconds(&self) -> Option<u64> {
        self.circuit_open_until_seconds
    }

    /// Returns consecutive failures that count toward the circuit breaker.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_reconcile::TransmissionWorker;
    /// # fn worker() -> TransmissionWorker { loop {} }
    /// let _failures = worker().consecutive_circuit_failures();
    /// ```
    #[must_use]
    pub const fn consecutive_circuit_failures(&self) -> u16 {
        self.consecutive_circuit_failures
    }

    /// Processes one ready outbox job through the gateway adapter.
    ///
    /// `now_seconds` is supplied by the caller so production code can use a
    /// real clock while tests use deterministic timestamps without sleeping.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError`] when the job is not ready or the submit
    /// request cannot be constructed from its context and document.
    pub async fn process_once(
        &mut self,
        adapter: &dyn GatewayAdapter,
        now_seconds: u64,
        job: TransmissionJob,
    ) -> Result<TransmissionWorkerResult, ReconcileError> {
        Self::process_ready_state(&job.envelope)?;

        if let Some(open_until) = self.circuit_open_until_seconds {
            if now_seconds < open_until {
                let retry_after_seconds = open_until - now_seconds;
                let log_event = self.log_event(
                    &job.envelope,
                    TransmissionWorkerOutcomeKind::CircuitOpen,
                    Some(retry_after_seconds),
                    None,
                );
                Self::emit_log(&log_event);
                return Ok(TransmissionWorkerResult::CircuitOpen {
                    job,
                    retry_after_seconds,
                    log_event,
                });
            }
            self.circuit_open_until_seconds = None;
        }

        if let Some(retry_after_seconds) = self.rate_limit_delay(now_seconds) {
            let log_event = self.log_event(
                &job.envelope,
                TransmissionWorkerOutcomeKind::RateLimited,
                Some(retry_after_seconds),
                None,
            );
            Self::emit_log(&log_event);
            return Ok(TransmissionWorkerResult::RateLimited {
                job,
                retry_after_seconds,
                log_event,
            });
        }

        let mut envelope = job.envelope;
        let request = SubmitRequest::new(envelope.context.clone(), job.route, job.document)?;
        self.last_attempt_started_at_seconds = Some(now_seconds);
        match adapter.submit(request).await {
            Ok(receipt) => {
                self.consecutive_circuit_failures = 0;
                self.circuit_open_until_seconds = None;
                envelope.state = outbox_state_for_receipt(receipt.status);
                let log_event = self.log_event(
                    &envelope,
                    TransmissionWorkerOutcomeKind::Submitted,
                    None,
                    None,
                );
                Self::emit_log(&log_event);
                Ok(TransmissionWorkerResult::Submitted {
                    envelope,
                    receipt,
                    log_event,
                })
            }
            Err(error) => self.handle_gateway_error(envelope, error, now_seconds),
        }
    }

    /// Processes up to `max_batch_size` jobs in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError`] when any selected job is not ready or cannot
    /// be converted into a gateway submit request.
    pub async fn process_batch(
        &mut self,
        adapter: &dyn GatewayAdapter,
        now_seconds: u64,
        jobs: impl IntoIterator<Item = TransmissionJob>,
    ) -> Result<Vec<TransmissionWorkerResult>, ReconcileError> {
        let selected: Vec<TransmissionJob> =
            jobs.into_iter().take(self.config.max_batch_size).collect();

        let mut results = Vec::with_capacity(selected.len());
        for job in selected {
            results.push(self.process_once(adapter, now_seconds, job).await?);
        }
        Ok(results)
    }

    fn process_ready_state(envelope: &OutboxEnvelope) -> Result<(), ReconcileError> {
        if matches!(envelope.state, OutboxState::Reserved) {
            Ok(())
        } else {
            Err(ReconcileError::InvalidOutboxState {
                outbox_id: envelope.outbox_id.clone(),
                state: envelope.state,
                remediation: "only reserved outbox rows are ready for transmission",
            })
        }
    }

    fn rate_limit_delay(&self, now_seconds: u64) -> Option<u64> {
        let min_interval = self.config.rate_limit.min_interval_seconds;
        if min_interval == 0 {
            return None;
        }
        let last = self.last_attempt_started_at_seconds?;
        let next_allowed = last.saturating_add(min_interval);
        (now_seconds < next_allowed).then_some(next_allowed - now_seconds)
    }

    fn handle_gateway_error(
        &mut self,
        mut envelope: OutboxEnvelope,
        error: GatewayError,
        now_seconds: u64,
    ) -> Result<TransmissionWorkerResult, ReconcileError> {
        let circuit_open_until_seconds = self.record_circuit_failure(&error, now_seconds);
        if is_retryable_gateway_error(error.kind) {
            match envelope.record_failed_attempt()? {
                RetryDecision::RetryAfterSeconds(policy_delay) => {
                    let retry_after_seconds =
                        cmp::max(policy_delay, error.retry_after_seconds.unwrap_or(0));
                    let log_event = self.log_event(
                        &envelope,
                        TransmissionWorkerOutcomeKind::RetryScheduled,
                        Some(retry_after_seconds),
                        Some(error.kind),
                    );
                    Self::emit_log(&log_event);
                    Ok(TransmissionWorkerResult::RetryScheduled {
                        envelope,
                        error,
                        retry_after_seconds,
                        circuit_open_until_seconds,
                        log_event,
                    })
                }
                RetryDecision::MoveToDeadLetter => {
                    self.dead_letter_result(envelope, error, circuit_open_until_seconds)
                }
            }
        } else {
            let _decision = envelope.record_failed_attempt()?;
            envelope.state = OutboxState::DeadLetter;
            self.dead_letter_result(envelope, error, circuit_open_until_seconds)
        }
    }

    fn dead_letter_result(
        &self,
        envelope: OutboxEnvelope,
        error: GatewayError,
        circuit_open_until_seconds: Option<u64>,
    ) -> Result<TransmissionWorkerResult, ReconcileError> {
        let dead_letter_id = format!("dead_{}", envelope.outbox_id);
        let dead_letter = envelope.to_dead_letter(dead_letter_id, &error)?;
        let log_event = self.log_event(
            &envelope,
            TransmissionWorkerOutcomeKind::DeadLettered,
            None,
            Some(error.kind),
        );
        Self::emit_log(&log_event);
        Ok(TransmissionWorkerResult::DeadLettered {
            envelope,
            dead_letter,
            error,
            circuit_open_until_seconds,
            log_event,
        })
    }

    fn record_circuit_failure(&mut self, error: &GatewayError, now_seconds: u64) -> Option<u64> {
        if !counts_toward_circuit_breaker(error.kind) {
            return self.circuit_open_until_seconds;
        }
        self.consecutive_circuit_failures = self.consecutive_circuit_failures.saturating_add(1);
        if self.consecutive_circuit_failures >= self.config.circuit_breaker.failure_threshold {
            let open_until = now_seconds.saturating_add(self.config.circuit_breaker.open_seconds);
            self.circuit_open_until_seconds = Some(open_until);
        }
        self.circuit_open_until_seconds
    }

    fn log_event(
        &self,
        envelope: &OutboxEnvelope,
        outcome: TransmissionWorkerOutcomeKind,
        retry_after_seconds: Option<u64>,
        gateway_error_kind: Option<GatewayErrorKind>,
    ) -> TransmissionWorkerLogEvent {
        TransmissionWorkerLogEvent {
            event: "invoicekit.transmission_worker.decision",
            bead_id: TRANSMISSION_WORKER_BEAD_ID,
            gateway_key: self.config.gateway_key.clone(),
            outbox_id: envelope.outbox_id.clone(),
            tenant_id: envelope.context.tenant_id.as_str().to_owned(),
            trace_id: envelope.context.trace_id.as_str().to_owned(),
            idempotency_key: envelope.context.idempotency_key.as_str().to_owned(),
            gateway_attempt_id: envelope.context.gateway_attempt_id.as_str().to_owned(),
            operation: GatewayOperation::Submit,
            outcome,
            retry_after_seconds,
            gateway_error_kind,
        }
    }

    fn emit_log(event: &TransmissionWorkerLogEvent) {
        tracing::info!(
            bead_id = event.bead_id,
            gateway_key = event.gateway_key.as_str(),
            outbox_id = event.outbox_id.as_str(),
            tenant_id = event.tenant_id.as_str(),
            trace_id = event.trace_id.as_str(),
            idempotency_key = event.idempotency_key.as_str(),
            gateway_attempt_id = event.gateway_attempt_id.as_str(),
            operation = event.operation.as_str(),
            outcome = event.outcome.as_str(),
            retry_after_seconds = event.retry_after_seconds,
            gateway_error_kind = event.gateway_error_kind.map(GatewayErrorKind::as_str),
            "transmission worker decision"
        );
    }
}

fn outbox_state_for_receipt(status: GatewayStatus) -> OutboxState {
    match status {
        GatewayStatus::Pending => OutboxState::Sent,
        GatewayStatus::Rejected => OutboxState::Rejected,
        GatewayStatus::Accepted | GatewayStatus::Cancelled | GatewayStatus::Corrected => {
            OutboxState::Delivered
        }
    }
}

fn is_retryable_gateway_error(kind: GatewayErrorKind) -> bool {
    matches!(
        kind,
        GatewayErrorKind::RateLimited
            | GatewayErrorKind::GatewayMaintenance
            | GatewayErrorKind::Timeout
            | GatewayErrorKind::NetworkFailure
            | GatewayErrorKind::PartnerError
            | GatewayErrorKind::UnexpectedResponse
    )
}

fn counts_toward_circuit_breaker(kind: GatewayErrorKind) -> bool {
    matches!(
        kind,
        GatewayErrorKind::GatewayMaintenance
            | GatewayErrorKind::Timeout
            | GatewayErrorKind::NetworkFailure
            | GatewayErrorKind::PartnerError
            | GatewayErrorKind::UnexpectedResponse
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GatewayAttemptId, GatewayContext, GatewayFuture, GatewaySubmissionId, IdempotencyKey,
        RetryPolicy, TenantId, TraceId,
    };
    use invoicekit_ir::CommercialDocument;
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Mutex;
    use std::task::{Context, Poll};

    use futures_task::noop_waker_ref;

    #[test]
    fn config_validates_batch_and_breaker_fields() {
        assert!(matches!(
            CircuitBreakerPolicy::new(0, 60),
            Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "failure_threshold",
                ..
            })
        ));
        assert!(matches!(
            CircuitBreakerPolicy::new(1, 0),
            Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "open_seconds",
                ..
            })
        ));
        assert!(matches!(
            TransmissionWorkerConfig::new(
                "mock",
                0,
                GatewayRateLimit::new(0),
                CircuitBreakerPolicy::new(1, 60).unwrap(),
            ),
            Err(ReconcileError::InvalidTransmissionWorkerConfig {
                field: "max_batch_size",
                ..
            })
        ));
    }

    #[test]
    fn worker_submits_reserved_job_and_marks_pending_as_sent() -> Result<(), String> {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Ok(receipt(GatewayStatus::Pending))]);
        let mut worker = worker(0, 2, 60);

        let result = block_on_ready(worker.process_once(&adapter, 1_000, job("outbox_001")))
            .map_err(|err| err.to_string())?;

        match result {
            TransmissionWorkerResult::Submitted {
                envelope,
                receipt,
                log_event,
            } => {
                assert_eq!(envelope.state, OutboxState::Sent);
                assert_eq!(receipt.status, GatewayStatus::Pending);
                assert_eq!(log_event.tenant_id, "tenant_123");
                assert_eq!(log_event.trace_id, "trace_abc");
                assert_eq!(log_event.outcome, TransmissionWorkerOutcomeKind::Submitted);
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        assert_eq!(adapter.calls(), 1);
        assert_eq!(worker.consecutive_circuit_failures(), 0);
        Ok(())
    }

    #[test]
    fn worker_schedules_retry_with_policy_and_gateway_retry_after() -> Result<(), String> {
        let error = GatewayError::new(
            GatewayErrorKind::RateLimited,
            GatewayOperation::Submit,
            "quota exceeded",
            "retry after gateway window",
        )
        .with_retry_after_seconds(90);
        let adapter = ScriptedGatewayAdapter::with_outcomes([Err(error)]);
        let mut worker = worker(0, 2, 60);

        let result = block_on_ready(worker.process_once(&adapter, 1_000, job("outbox_001")))
            .map_err(|err| err.to_string())?;

        match result {
            TransmissionWorkerResult::RetryScheduled {
                envelope,
                retry_after_seconds,
                log_event,
                ..
            } => {
                assert_eq!(envelope.attempt_count, 1);
                assert_eq!(envelope.state, OutboxState::Reserved);
                assert_eq!(retry_after_seconds, 90);
                assert_eq!(
                    log_event.gateway_error_kind,
                    Some(GatewayErrorKind::RateLimited)
                );
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn worker_moves_exhausted_retry_policy_to_dead_letter() -> Result<(), String> {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Err(error(
            GatewayErrorKind::GatewayMaintenance,
        ))]);
        let mut worker = worker(0, 2, 60);
        let mut job = job("outbox_001");
        job.envelope.retry_policy = RetryPolicy::new(1, 30, 30, 0).unwrap();

        let result = block_on_ready(worker.process_once(&adapter, 1_000, job))
            .map_err(|err| err.to_string())?;

        match result {
            TransmissionWorkerResult::DeadLettered {
                envelope,
                dead_letter,
                error,
                ..
            } => {
                assert_eq!(envelope.state, OutboxState::DeadLetter);
                assert_eq!(envelope.attempt_count, 1);
                assert_eq!(dead_letter.failure_code, "gateway_maintenance");
                assert_eq!(error.kind, GatewayErrorKind::GatewayMaintenance);
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        Ok(())
    }

    #[test]
    fn worker_dead_letters_non_retryable_gateway_errors() -> Result<(), String> {
        let adapter =
            ScriptedGatewayAdapter::with_outcomes([Err(error(GatewayErrorKind::Rejected))]);
        let mut worker = worker(0, 2, 60);

        let result = block_on_ready(worker.process_once(&adapter, 1_000, job("outbox_001")))
            .map_err(|err| err.to_string())?;

        match result {
            TransmissionWorkerResult::DeadLettered {
                envelope, error, ..
            } => {
                assert_eq!(envelope.state, OutboxState::DeadLetter);
                assert_eq!(envelope.attempt_count, 1);
                assert_eq!(error.kind, GatewayErrorKind::Rejected);
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        assert_eq!(worker.consecutive_circuit_failures(), 0);
        Ok(())
    }

    #[test]
    fn worker_honors_per_gateway_rate_limit_before_adapter_call() -> Result<(), String> {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Ok(receipt(GatewayStatus::Accepted))]);
        let mut worker = worker(10, 2, 60);

        let first = block_on_ready(worker.process_once(&adapter, 100, job("outbox_001")))
            .map_err(|err| err.to_string())?;
        assert_eq!(first.outcome(), TransmissionWorkerOutcomeKind::Submitted);

        let second = block_on_ready(worker.process_once(&adapter, 105, job("outbox_002")))
            .map_err(|err| err.to_string())?;

        match second {
            TransmissionWorkerResult::RateLimited {
                job,
                retry_after_seconds,
                log_event,
            } => {
                assert_eq!(job.envelope.outbox_id, "outbox_002");
                assert_eq!(retry_after_seconds, 5);
                assert_eq!(
                    log_event.outcome,
                    TransmissionWorkerOutcomeKind::RateLimited
                );
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        assert_eq!(adapter.calls(), 1);
        Ok(())
    }

    #[test]
    fn worker_does_not_rate_limit_after_local_request_validation_failure() {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Ok(receipt(GatewayStatus::Accepted))]);
        let mut worker = worker(10, 2, 60);
        let mut invalid_job = job("outbox_bad");
        invalid_job.envelope.context = GatewayContext::new(
            TenantId::new("tenant_other").unwrap(),
            TraceId::new("trace_abc").unwrap(),
            IdempotencyKey::new("idem_invoice_123").unwrap(),
            GatewayAttemptId::new("attempt_001").unwrap(),
        );

        let err = block_on_ready(worker.process_once(&adapter, 100, invalid_job)).unwrap_err();
        assert!(matches!(
            err,
            ReconcileError::ContextMismatch {
                field: "tenant_id",
                ..
            }
        ));
        assert_eq!(adapter.calls(), 0);

        let result = block_on_ready(worker.process_once(&adapter, 100, job("outbox_001")))
            .expect("valid job result");

        assert_eq!(result.outcome(), TransmissionWorkerOutcomeKind::Submitted);
        assert_eq!(adapter.calls(), 1);
    }

    #[test]
    fn worker_opens_circuit_after_persistent_failures() -> Result<(), String> {
        let adapter = ScriptedGatewayAdapter::with_outcomes([
            Err(error(GatewayErrorKind::GatewayMaintenance)),
            Err(error(GatewayErrorKind::GatewayMaintenance)),
        ]);
        let mut worker = worker(0, 2, 120);

        let first = block_on_ready(worker.process_once(&adapter, 100, job("outbox_001")))
            .map_err(|err| err.to_string())?;
        assert_eq!(
            first.outcome(),
            TransmissionWorkerOutcomeKind::RetryScheduled
        );
        assert_eq!(worker.circuit_open_until_seconds(), None);

        let second = block_on_ready(worker.process_once(&adapter, 101, job("outbox_002")))
            .map_err(|err| err.to_string())?;
        match second {
            TransmissionWorkerResult::RetryScheduled {
                circuit_open_until_seconds,
                ..
            } => {
                assert_eq!(circuit_open_until_seconds, Some(221));
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }

        let third = block_on_ready(worker.process_once(&adapter, 150, job("outbox_003")))
            .map_err(|err| err.to_string())?;
        match third {
            TransmissionWorkerResult::CircuitOpen {
                job,
                retry_after_seconds,
                log_event,
            } => {
                assert_eq!(job.envelope.outbox_id, "outbox_003");
                assert_eq!(retry_after_seconds, 71);
                assert_eq!(
                    log_event.outcome,
                    TransmissionWorkerOutcomeKind::CircuitOpen
                );
            }
            other => return Err(format!("unexpected result: {other:?}")),
        }
        assert_eq!(adapter.calls(), 2);
        Ok(())
    }

    #[test]
    fn worker_rejects_non_reserved_outbox_rows() {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Ok(receipt(GatewayStatus::Accepted))]);
        let mut worker = worker(0, 2, 60);
        let mut job = job("outbox_001");
        job.envelope.state = OutboxState::Sent;

        let err = block_on_ready(worker.process_once(&adapter, 100, job)).unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::InvalidOutboxState {
                state: OutboxState::Sent,
                ..
            }
        ));
        assert_eq!(adapter.calls(), 0);
    }

    #[test]
    fn worker_process_batch_respects_max_batch_size() {
        let adapter = ScriptedGatewayAdapter::with_outcomes([
            Ok(receipt(GatewayStatus::Accepted)),
            Ok(receipt(GatewayStatus::Accepted)),
            Ok(receipt(GatewayStatus::Accepted)),
        ]);
        let mut worker = worker_with_batch(0, 2, 60, 2);

        let results = block_on_ready(worker.process_batch(
            &adapter,
            100,
            [job("outbox_001"), job("outbox_002"), job("outbox_003")],
        ))
        .expect("batch results");

        assert_eq!(results.len(), 2);
        assert_eq!(adapter.calls(), 2);
    }

    #[test]
    fn worker_log_event_serializes_as_json_with_trace_and_tenant() {
        let adapter = ScriptedGatewayAdapter::with_outcomes([Ok(receipt(GatewayStatus::Accepted))]);
        let mut worker = worker(0, 2, 60);

        let result = block_on_ready(worker.process_once(&adapter, 100, job("outbox_001")))
            .expect("worker result");
        let line = result.log_event().to_json_line().unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();

        assert_eq!(value["tenant_id"], "tenant_123");
        assert_eq!(value["trace_id"], "trace_abc");
        assert_eq!(value["gateway_attempt_id"], "attempt_001");
        assert_eq!(value["outcome"], "submitted");
    }

    struct ScriptedGatewayAdapter {
        outcomes: Mutex<VecDeque<Result<GatewayReceipt, GatewayError>>>,
        calls: Mutex<u32>,
    }

    impl ScriptedGatewayAdapter {
        fn with_outcomes(
            outcomes: impl IntoIterator<Item = Result<GatewayReceipt, GatewayError>>,
        ) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into_iter().collect()),
                calls: Mutex::new(0),
            }
        }

        fn calls(&self) -> u32 {
            *self.calls.lock().expect("test call lock is not poisoned")
        }

        fn next_outcome(&self) -> Result<GatewayReceipt, GatewayError> {
            *self.calls.lock().expect("test call lock is not poisoned") += 1;
            self.outcomes
                .lock()
                .expect("test adapter lock is not poisoned")
                .pop_front()
                .expect("test adapter configured with enough outcomes")
        }
    }

    impl GatewayAdapter for ScriptedGatewayAdapter {
        fn submit(&self, _request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(self.next_outcome()))
        }

        fn poll(&self, _request: crate::PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(Err(error(
                GatewayErrorKind::UnsupportedOperation,
            ))))
        }

        fn cancel(&self, _request: crate::CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(Err(error(
                GatewayErrorKind::UnsupportedOperation,
            ))))
        }

        fn correct(&self, _request: crate::CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(Err(error(
                GatewayErrorKind::UnsupportedOperation,
            ))))
        }
    }

    fn block_on_ready<T>(future: impl Future<Output = T>) -> T {
        let mut future = pin!(future);
        let mut context = Context::from_waker(noop_waker_ref());
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                break value;
            }
            std::thread::yield_now();
        }
    }

    fn worker(rate_limit_seconds: u64, threshold: u16, open_seconds: u64) -> TransmissionWorker {
        worker_with_batch(rate_limit_seconds, threshold, open_seconds, 10)
    }

    fn worker_with_batch(
        rate_limit_seconds: u64,
        threshold: u16,
        open_seconds: u64,
        max_batch_size: usize,
    ) -> TransmissionWorker {
        TransmissionWorker::new(
            TransmissionWorkerConfig::new(
                "mock-peppol",
                max_batch_size,
                GatewayRateLimit::new(rate_limit_seconds),
                CircuitBreakerPolicy::new(threshold, open_seconds).unwrap(),
            )
            .unwrap(),
        )
    }

    fn job(outbox_id: &str) -> TransmissionJob {
        TransmissionJob::new(
            OutboxEnvelope::new(
                outbox_id,
                gateway_context(),
                blake3::hash(b"invoice"),
                "{}",
                RetryPolicy::new(3, 30, 120, 0).unwrap(),
            )
            .unwrap(),
            GatewayRoute::new("peppol", "peppol-bis-3", Some("DE")).unwrap(),
            synthetic_document(),
        )
    }

    fn gateway_context() -> crate::GatewayContext {
        GatewayContext::new(
            TenantId::new("tenant_123").unwrap(),
            TraceId::new("trace_abc").unwrap(),
            IdempotencyKey::new("idem_invoice_123").unwrap(),
            GatewayAttemptId::new("attempt_001").unwrap(),
        )
    }

    fn receipt(status: GatewayStatus) -> GatewayReceipt {
        GatewayReceipt::new(
            GatewayOperation::Submit,
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            status,
            "2026-05-27T05:30:00Z",
        )
        .unwrap()
    }

    fn error(kind: GatewayErrorKind) -> GatewayError {
        GatewayError::new(
            kind,
            GatewayOperation::Submit,
            format!("{kind} test failure"),
            "apply the normalized adapter remediation",
        )
    }

    fn synthetic_document() -> CommercialDocument {
        CommercialDocument::try_from_value(json!({
            "schema_version": "1.0",
            "id": "doc_2026_0001",
            "document_type": "invoice",
            "issue_date": "2026-05-26",
            "due_date": "2026-06-25",
            "document_number": "INV-2026-0001",
            "currency": "EUR",
            "supplier": party_json("supplier-1", "InvoiceKit GmbH", "DE"),
            "customer": party_json("customer-1", "ACME SAS", "FR"),
            "lines": [{
                "id": "1",
                "description": "Validation subscription",
                "quantity": "1",
                "unit_code": "EA",
                "unit_price": "100.00",
                "line_extension_amount": "100.00",
                "tax_category": "S"
            }],
            "tax_summary": [{
                "category_code": "S",
                "taxable_amount": "100.00",
                "tax_amount": "19.00",
                "tax_rate": "19.00"
            }],
            "monetary_total": {
                "line_extension_amount": "100.00",
                "tax_exclusive_amount": "100.00",
                "tax_inclusive_amount": "119.00",
                "payable_amount": "119.00"
            },
            "meta": {
                "tenant_id": "tenant_123",
                "trace_id": "trace_abc",
                "source_system": "unit-test"
            }
        }))
        .unwrap()
    }

    fn party_json(id: &str, name: &str, country: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "tax_ids": [{
                "scheme": "vat",
                "value": format!("{country}123456789")
            }],
            "address": {
                "lines": ["Main Street 1"],
                "city": "Sample City",
                "postal_code": "10115",
                "country": country
            }
        })
    }
}
