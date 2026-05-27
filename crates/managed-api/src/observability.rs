// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Observability contracts for the managed InvoiceKit layer.
//!
//! This module keeps OpenTelemetry traces, SLO metrics, gateway dashboards,
//! and log redaction tied to the managed request boundary. Callers produce a
//! [`ManagedRequestObservation`] exactly once for each managed API operation so
//! the span, metric, and structured log fields cannot drift.

use crate::{require_non_empty, validate_identifier, ManagedApiError, TenantRequestContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Bead identifier attached to observability events introduced by T-136.
pub const OBSERVABILITY_BEAD_ID: &str = "invoices-t-136-opentelemetry-n3xq";

/// Placeholder used when a log field may contain personal or secret data.
pub const LOG_REDACTION_PLACEHOLDER: &str = "<REDACTED>";

/// Operations that have explicit service-level objectives.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SloOperation {
    /// Invoice validation request.
    Validate,
    /// Deterministic render request.
    Render,
    /// Outbox enqueue request before gateway delivery.
    TransmitEnqueue,
    /// Gateway accepted or rejected a submission attempt.
    GatewayAccepted,
    /// Evidence archive write request.
    ArchiveWrite,
    /// Webhook delivery attempt.
    WebhookDeliver,
}

impl SloOperation {
    /// Every operation that must be represented in T-136 metrics.
    pub const ALL: [Self; 6] = [
        Self::Validate,
        Self::Render,
        Self::TransmitEnqueue,
        Self::GatewayAccepted,
        Self::ArchiveWrite,
        Self::WebhookDeliver,
    ];

    /// Return the stable metric attribute value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::SloOperation;
    /// assert_eq!(SloOperation::TransmitEnqueue.as_str(), "transmit-enqueue");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Render => "render",
            Self::TransmitEnqueue => "transmit-enqueue",
            Self::GatewayAccepted => "gateway-accepted",
            Self::ArchiveWrite => "archive-write",
            Self::WebhookDeliver => "webhook-deliver",
        }
    }
}

/// Normalized outcome shared by request spans, SLO metrics, and dashboards.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryOutcome {
    /// Operation completed successfully.
    Succeeded,
    /// Operation was denied before execution.
    Denied,
    /// Operation failed after it started.
    Failed,
}

impl TelemetryOutcome {
    /// Return the stable outcome attribute value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::TelemetryOutcome;
    /// assert_eq!(TelemetryOutcome::Failed.as_str(), "failed");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// Validated OpenTelemetry trace/span identifiers.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OpenTelemetryIds {
    /// W3C trace identifier: 32 lowercase hexadecimal characters.
    pub trace_id: String,
    /// W3C span identifier: 16 lowercase hexadecimal characters.
    pub span_id: String,
    /// Optional W3C parent span identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// W3C trace flags, normally `01` for sampled or `00` for unsampled.
    pub trace_flags: String,
}

impl OpenTelemetryIds {
    /// Build validated OpenTelemetry identifiers.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when any identifier is
    /// not the exact W3C lowercase hexadecimal length or is all zeroes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::OpenTelemetryIds;
    /// let ids = OpenTelemetryIds::new(
    ///     "4bf92f3577b34da6a3ce929d0e0e4736",
    ///     "00f067aa0ba902b7",
    ///     None,
    /// ).unwrap();
    /// assert_eq!(
    ///     ids.traceparent(),
    ///     "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    /// );
    /// ```
    pub fn new(
        trace_id: impl Into<String>,
        span_id: impl Into<String>,
        parent_span_id: Option<String>,
    ) -> Result<Self, ManagedApiError> {
        Self::new_with_trace_flags(trace_id, span_id, parent_span_id, "01")
    }

    /// Build validated OpenTelemetry identifiers while preserving trace flags.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when any identifier is
    /// not the exact W3C lowercase hexadecimal length, is all zeroes, or when
    /// `trace_flags` is not two lowercase hexadecimal characters.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::OpenTelemetryIds;
    /// let ids = OpenTelemetryIds::new_with_trace_flags(
    ///     "4bf92f3577b34da6a3ce929d0e0e4736",
    ///     "00f067aa0ba902b7",
    ///     None,
    ///     "00",
    /// ).unwrap();
    /// assert!(ids.traceparent().ends_with("-00"));
    /// ```
    pub fn new_with_trace_flags(
        trace_id: impl Into<String>,
        span_id: impl Into<String>,
        parent_span_id: Option<String>,
        trace_flags: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        let trace_id = trace_id.into();
        let span_id = span_id.into();
        let trace_flags = trace_flags.into();
        Ok(Self {
            trace_id: validate_otel_trace_id(&trace_id)?,
            span_id: validate_otel_span_id("otel_span_id", &span_id)?,
            parent_span_id: parent_span_id
                .map(|id| validate_otel_span_id("otel_parent_span_id", &id))
                .transpose()?,
            trace_flags: validate_otel_trace_flags(&trace_flags)?,
        })
    }

    /// Return a W3C `traceparent` header value.
    #[must_use]
    pub fn traceparent(&self) -> String {
        format!("00-{}-{}-{}", self.trace_id, self.span_id, self.trace_flags)
    }
}

/// OpenTelemetry server span for one managed API request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObservedRequestSpan {
    /// OpenTelemetry span name, usually `METHOD route-template`.
    pub name: String,
    /// W3C trace/span identifiers.
    pub otel: OpenTelemetryIds,
    /// Tenant selected by the credential and route.
    pub tenant_id: String,
    /// InvoiceKit trace identifier propagated into audit and gateway records.
    pub invoicekit_trace_id: String,
    /// Operation being served.
    pub operation: SloOperation,
    /// HTTP method or equivalent request method.
    pub http_method: String,
    /// HTTP route template or equivalent request route.
    pub http_route: String,
    /// Final HTTP status code.
    pub http_status_code: u16,
    /// Normalized outcome.
    pub outcome: TelemetryOutcome,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// Optional gateway key for gateway-scoped requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_key: Option<String>,
    /// Bead that introduced this span contract.
    pub bead_id: &'static str,
}

impl ObservedRequestSpan {
    /// Build an OpenTelemetry-compatible request span record.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when method, route, or
    /// status is invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{
    /// #     Actor, ObservedRequestSpan, OpenTelemetryIds, SloOperation, TelemetryOutcome,
    /// #     TenantId, TenantRequestContext, TraceId,
    /// # };
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_request_1").unwrap(),
    ///     Actor::System { name: "managed-api".to_owned() },
    /// );
    /// let span = ObservedRequestSpan::new(
    ///     &ctx,
    ///     OpenTelemetryIds::new(
    ///         "4bf92f3577b34da6a3ce929d0e0e4736",
    ///         "00f067aa0ba902b7",
    ///         None,
    ///     ).unwrap(),
    ///     SloOperation::Validate,
    ///     "POST",
    ///     "/v1/invoices/validate",
    ///     200,
    ///     TelemetryOutcome::Succeeded,
    ///     12,
    /// ).unwrap();
    /// assert_eq!(span.name, "POST /v1/invoices/validate");
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        context: &TenantRequestContext,
        otel: OpenTelemetryIds,
        operation: SloOperation,
        http_method: impl Into<String>,
        http_route: impl Into<String>,
        http_status_code: u16,
        outcome: TelemetryOutcome,
        duration_ms: u64,
    ) -> Result<Self, ManagedApiError> {
        let http_method = require_non_empty("http_method", http_method.into())?;
        let http_route = require_non_empty("http_route", http_route.into())?;
        if !(100..=599).contains(&http_status_code) {
            return Err(ManagedApiError::InvalidIdentifier {
                field: "http_status_code",
                reason: "status code must be in the inclusive 100..599 range",
            });
        }

        Ok(Self {
            name: format!("{http_method} {http_route}"),
            otel,
            tenant_id: context.tenant_id.as_str().to_owned(),
            invoicekit_trace_id: context.trace_id.as_str().to_owned(),
            operation,
            http_method,
            http_route,
            http_status_code,
            outcome,
            duration_ms,
            gateway_key: None,
            bead_id: OBSERVABILITY_BEAD_ID,
        })
    }

    /// Attach a gateway key to a request span.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the gateway key is not
    /// a stable ASCII identifier.
    pub fn with_gateway_key(
        mut self,
        gateway_key: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        self.gateway_key = Some(validate_identifier("gateway_key", gateway_key.into())?);
        Ok(self)
    }

    /// Return the W3C traceparent header for this span.
    #[must_use]
    pub fn traceparent(&self) -> String {
        self.otel.traceparent()
    }

    /// Render the span as a deterministic OpenTelemetry-shaped JSON object.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_managed_api::ObservedRequestSpan;
    /// # fn span() -> ObservedRequestSpan { loop {} }
    /// assert_eq!(span().to_otel_json()["kind"], "server");
    /// ```
    #[must_use]
    pub fn to_otel_json(&self) -> Value {
        let mut attributes = Map::new();
        attributes.insert(
            "service.name".to_owned(),
            Value::String("invoicekit-managed-api".to_owned()),
        );
        attributes.insert(
            "invoicekit.bead_id".to_owned(),
            Value::String(self.bead_id.to_owned()),
        );
        attributes.insert(
            "invoicekit.tenant_id".to_owned(),
            Value::String(self.tenant_id.clone()),
        );
        attributes.insert(
            "invoicekit.trace_id".to_owned(),
            Value::String(self.invoicekit_trace_id.clone()),
        );
        attributes.insert(
            "invoicekit.slo_operation".to_owned(),
            Value::String(self.operation.as_str().to_owned()),
        );
        attributes.insert(
            "http.request.method".to_owned(),
            Value::String(self.http_method.clone()),
        );
        attributes.insert(
            "http.route".to_owned(),
            Value::String(self.http_route.clone()),
        );
        attributes.insert(
            "http.response.status_code".to_owned(),
            Value::from(self.http_status_code),
        );
        attributes.insert(
            "invoicekit.outcome".to_owned(),
            Value::String(self.outcome.as_str().to_owned()),
        );
        attributes.insert(
            "invoicekit.duration_ms".to_owned(),
            Value::from(self.duration_ms),
        );
        if let Some(gateway_key) = &self.gateway_key {
            attributes.insert(
                "invoicekit.gateway_key".to_owned(),
                Value::String(gateway_key.clone()),
            );
        }

        json!({
            "name": self.name,
            "kind": "server",
            "trace_id": self.otel.trace_id,
            "span_id": self.otel.span_id,
            "parent_span_id": self.otel.parent_span_id,
            "trace_flags": self.otel.trace_flags,
            "traceparent": self.traceparent(),
            "attributes": Value::Object(attributes),
        })
    }
}

/// One SLO metric sample for an InvoiceKit operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SloMetricEvent {
    /// Metric name for operation latency.
    pub metric_name: &'static str,
    /// Tenant selected by the credential and route.
    pub tenant_id: String,
    /// InvoiceKit trace identifier.
    pub trace_id: String,
    /// Operation measured by this event.
    pub operation: SloOperation,
    /// Normalized operation outcome.
    pub outcome: TelemetryOutcome,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// RFC 3339 timestamp for the sample.
    pub recorded_at: String,
    /// Optional gateway key for gateway-scoped metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_key: Option<String>,
    /// Optional normalized failure kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
}

impl SloMetricEvent {
    /// Build an SLO metric sample for one operation.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if `recorded_at` is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{
    /// #     Actor, SloMetricEvent, SloOperation, TelemetryOutcome,
    /// #     TenantId, TenantRequestContext, TraceId,
    /// # };
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_metric_1").unwrap(),
    ///     Actor::System { name: "managed-api".to_owned() },
    /// );
    /// let metric = SloMetricEvent::new(
    ///     &ctx,
    ///     SloOperation::ArchiveWrite,
    ///     TelemetryOutcome::Succeeded,
    ///     42,
    ///     "2026-05-27T06:00:00Z",
    /// ).unwrap();
    /// assert_eq!(metric.metric_name, "invoicekit.slo.operation.duration_ms");
    /// ```
    pub fn new(
        context: &TenantRequestContext,
        operation: SloOperation,
        outcome: TelemetryOutcome,
        duration_ms: u64,
        recorded_at: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        Ok(Self {
            metric_name: "invoicekit.slo.operation.duration_ms",
            tenant_id: context.tenant_id.as_str().to_owned(),
            trace_id: context.trace_id.as_str().to_owned(),
            operation,
            outcome,
            duration_ms,
            recorded_at: require_non_empty("recorded_at", recorded_at.into())?,
            gateway_key: None,
            failure_kind: None,
        })
    }

    /// Attach a gateway key to this metric sample.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the gateway key is not
    /// a stable ASCII identifier.
    pub fn with_gateway_key(
        mut self,
        gateway_key: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        self.gateway_key = Some(validate_identifier("gateway_key", gateway_key.into())?);
        Ok(self)
    }

    /// Attach a normalized failure kind.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the failure kind is
    /// not a stable ASCII identifier.
    pub fn with_failure_kind(
        mut self,
        failure_kind: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        self.failure_kind = Some(validate_identifier("failure_kind", failure_kind.into())?);
        Ok(self)
    }

    /// Render the event as a deterministic OpenTelemetry-shaped metric point.
    #[must_use]
    pub fn to_otel_json(&self) -> Value {
        let mut attributes = Map::new();
        attributes.insert(
            "invoicekit.bead_id".to_owned(),
            Value::String(OBSERVABILITY_BEAD_ID.to_owned()),
        );
        attributes.insert(
            "invoicekit.tenant_id".to_owned(),
            Value::String(self.tenant_id.clone()),
        );
        attributes.insert(
            "invoicekit.trace_id".to_owned(),
            Value::String(self.trace_id.clone()),
        );
        attributes.insert(
            "invoicekit.slo_operation".to_owned(),
            Value::String(self.operation.as_str().to_owned()),
        );
        attributes.insert(
            "invoicekit.outcome".to_owned(),
            Value::String(self.outcome.as_str().to_owned()),
        );
        if let Some(gateway_key) = &self.gateway_key {
            attributes.insert(
                "invoicekit.gateway_key".to_owned(),
                Value::String(gateway_key.clone()),
            );
        }
        if let Some(failure_kind) = &self.failure_kind {
            attributes.insert(
                "invoicekit.failure_kind".to_owned(),
                Value::String(failure_kind.clone()),
            );
        }

        json!({
            "name": self.metric_name,
            "unit": "ms",
            "value": self.duration_ms,
            "recorded_at": self.recorded_at,
            "attributes": Value::Object(attributes),
        })
    }
}

/// Deterministic per-gateway dashboard summary derived from SLO metrics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayDashboardSnapshot {
    /// Gateway key summarized by this snapshot.
    pub gateway_key: String,
    /// RFC 3339 inclusive window start.
    pub window_start: String,
    /// RFC 3339 exclusive window end.
    pub window_end: String,
    /// Total gateway-scoped metric samples in the window.
    pub total_events: u64,
    /// Successful gateway-scoped API operation samples.
    pub succeeded: u64,
    /// Failed or denied gateway-scoped samples.
    pub failed: u64,
    /// API availability in parts per million, if there are gateway events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_ppm: Option<u64>,
    /// Gateway legal-acceptance decision samples in the window.
    pub gateway_acceptance_events: u64,
    /// Gateway legal-acceptance successes in the window.
    pub gateway_accepted: u64,
    /// Gateway legal-acceptance failures or denials in the window.
    pub gateway_rejected: u64,
    /// Gateway legal acceptance in parts per million, separate from API uptime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_acceptance_ppm: Option<u64>,
    /// p95 latency across gateway-scoped samples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p95_duration_ms: Option<u64>,
    /// Failure counts grouped by normalized failure kind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub failure_counts: BTreeMap<String, u64>,
}

impl GatewayDashboardSnapshot {
    /// Build a per-gateway dashboard snapshot from SLO metrics.
    ///
    /// Metrics outside the half-open `[window_start, window_end)` range are
    /// ignored. Timestamps must use the same normalized RFC 3339 form, such as
    /// UTC `Z`, so lexical ordering matches chronological ordering.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when the gateway key is
    /// not a stable identifier or either window timestamp is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{GatewayDashboardSnapshot, SloMetricEvent};
    /// let snapshot = GatewayDashboardSnapshot::from_metrics(
    ///     "mock-peppol",
    ///     "2026-05-27T06:00:00Z",
    ///     "2026-05-27T07:00:00Z",
    ///     Vec::<SloMetricEvent>::new(),
    /// ).unwrap();
    /// assert_eq!(snapshot.total_events, 0);
    /// ```
    pub fn from_metrics(
        gateway_key: impl Into<String>,
        window_start: impl Into<String>,
        window_end: impl Into<String>,
        metrics: impl IntoIterator<Item = SloMetricEvent>,
    ) -> Result<Self, ManagedApiError> {
        let gateway_key = validate_identifier("gateway_key", gateway_key.into())?;
        let window_start = require_non_empty("window_start", window_start.into())?;
        let window_end = require_non_empty("window_end", window_end.into())?;
        if window_start >= window_end {
            return Err(ManagedApiError::InvalidIdentifier {
                field: "window",
                reason: "window_start must be before window_end",
            });
        }

        let mut durations = Vec::new();
        let mut succeeded = 0_u64;
        let mut failed = 0_u64;
        let mut gateway_acceptance_events = 0_u64;
        let mut gateway_accepted = 0_u64;
        let mut gateway_rejected = 0_u64;
        let mut failure_counts = BTreeMap::new();

        for metric in metrics.into_iter().filter(|metric| {
            metric
                .gateway_key
                .as_deref()
                .is_some_and(|metric_gateway| metric_gateway.eq(gateway_key.as_str()))
                && metric_in_window(metric, &window_start, &window_end)
        }) {
            durations.push(metric.duration_ms);
            let is_succeeded = matches!(metric.outcome, TelemetryOutcome::Succeeded);
            let is_gateway_acceptance = matches!(metric.operation, SloOperation::GatewayAccepted);
            if is_succeeded {
                succeeded += 1;
            }
            if matches!(
                metric.outcome,
                TelemetryOutcome::Denied | TelemetryOutcome::Failed
            ) {
                failed += 1;
                let key = metric
                    .failure_kind
                    .unwrap_or_else(|| metric.outcome.as_str().to_owned());
                *failure_counts.entry(key).or_insert(0) += 1;
            }
            if is_gateway_acceptance {
                gateway_acceptance_events += 1;
                if is_succeeded {
                    gateway_accepted += 1;
                } else {
                    gateway_rejected += 1;
                }
            }
        }

        durations.sort_unstable();
        let total_events = u64::try_from(durations.len()).unwrap_or(u64::MAX);
        let availability_ppm = parts_per_million(succeeded, total_events);
        let gateway_acceptance_ppm = parts_per_million(gateway_accepted, gateway_acceptance_events);
        let p95_duration_ms = percentile_nearest_rank(&durations, 95);

        Ok(Self {
            gateway_key,
            window_start,
            window_end,
            total_events,
            succeeded,
            failed,
            availability_ppm,
            gateway_acceptance_events,
            gateway_accepted,
            gateway_rejected,
            gateway_acceptance_ppm,
            p95_duration_ms,
            failure_counts,
        })
    }
}

/// Inputs required to observe one managed API request.
///
/// The type groups span, metric, and log inputs so request handlers cannot
/// update one observability channel while forgetting the others.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedRequestObservationInput {
    /// W3C OpenTelemetry identifiers for the request span.
    pub otel: OpenTelemetryIds,
    /// Managed operation being served.
    pub operation: SloOperation,
    /// HTTP method or equivalent request method.
    pub http_method: String,
    /// HTTP route template or equivalent request route.
    pub http_route: String,
    /// Final HTTP status code.
    pub http_status_code: u16,
    /// Normalized outcome.
    pub outcome: TelemetryOutcome,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// RFC 3339 timestamp for the metric sample.
    pub recorded_at: String,
    /// Optional gateway key for gateway-scoped operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_key: Option<String>,
    /// Optional normalized failure kind for failed metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<String>,
    /// Structured request log fields before redaction.
    #[serde(default)]
    pub log_fields: Value,
}

impl ManagedRequestObservationInput {
    /// Build observation inputs for one managed request.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when method, route,
    /// timestamp, or status is invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{
    /// #     ManagedRequestObservationInput, OpenTelemetryIds, SloOperation, TelemetryOutcome,
    /// # };
    /// let input = ManagedRequestObservationInput::new(
    ///     OpenTelemetryIds::new(
    ///         "4bf92f3577b34da6a3ce929d0e0e4736",
    ///         "00f067aa0ba902b7",
    ///         None,
    ///     ).unwrap(),
    ///     SloOperation::Validate,
    ///     "POST",
    ///     "/v1/invoices/validate",
    ///     200,
    ///     TelemetryOutcome::Succeeded,
    ///     15,
    ///     "2026-05-27T06:00:00Z",
    /// ).unwrap();
    /// assert_eq!(input.operation.as_str(), "validate");
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        otel: OpenTelemetryIds,
        operation: SloOperation,
        http_method: impl Into<String>,
        http_route: impl Into<String>,
        http_status_code: u16,
        outcome: TelemetryOutcome,
        duration_ms: u64,
        recorded_at: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        let http_method = require_non_empty("http_method", http_method.into())?;
        let http_route = require_non_empty("http_route", http_route.into())?;
        if !(100..=599).contains(&http_status_code) {
            return Err(ManagedApiError::InvalidIdentifier {
                field: "http_status_code",
                reason: "status code must be in the inclusive 100..599 range",
            });
        }
        Ok(Self {
            otel,
            operation,
            http_method,
            http_route,
            http_status_code,
            outcome,
            duration_ms,
            recorded_at: require_non_empty("recorded_at", recorded_at.into())?,
            gateway_key: None,
            failure_kind: None,
            log_fields: Value::Object(Map::new()),
        })
    }

    /// Attach a gateway key to both the span and metric emitted for the request.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the key is not a
    /// stable ASCII identifier.
    pub fn with_gateway_key(
        mut self,
        gateway_key: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        self.gateway_key = Some(validate_identifier("gateway_key", gateway_key.into())?);
        Ok(self)
    }

    /// Attach a normalized failure kind to the metric emitted for the request.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the failure kind is not
    /// a stable ASCII identifier.
    pub fn with_failure_kind(
        mut self,
        failure_kind: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        self.failure_kind = Some(validate_identifier("failure_kind", failure_kind.into())?);
        Ok(self)
    }

    /// Attach structured log fields that will be redacted before emission.
    #[must_use]
    pub fn with_log_fields(mut self, log_fields: Value) -> Self {
        self.log_fields = log_fields;
        self
    }
}

/// Atomic observation emitted for one managed API request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManagedRequestObservation {
    /// OpenTelemetry-compatible request span.
    pub span: ObservedRequestSpan,
    /// SLO metric sample for the request.
    pub metric: SloMetricEvent,
    /// PII-safe structured log payload.
    pub redacted_log: Value,
}

impl ManagedRequestObservation {
    /// Render the observation as deterministic OpenTelemetry-shaped JSON.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_managed_api::ManagedRequestObservation;
    /// # fn observation() -> ManagedRequestObservation { loop {} }
    /// let value = observation().to_otel_json();
    /// assert_eq!(value["kind"], "managed_request_observation");
    /// ```
    #[must_use]
    pub fn to_otel_json(&self) -> Value {
        json!({
            "kind": "managed_request_observation",
            "span": self.span.to_otel_json(),
            "metric": self.metric.to_otel_json(),
            "log": self.redacted_log,
        })
    }

    /// Emit the redacted structured log through the `tracing` facade.
    ///
    /// The emitted fields carry `trace_id`, `tenant_id`, and `bead_id` for the
    /// universal logging gate. The free-form payload is already redacted.
    pub fn emit_tracing_log(&self) {
        tracing::info!(
            bead_id = OBSERVABILITY_BEAD_ID,
            tenant_id = self.span.tenant_id.as_str(),
            trace_id = self.span.invoicekit_trace_id.as_str(),
            otel_trace_id = self.span.otel.trace_id.as_str(),
            otel_span_id = self.span.otel.span_id.as_str(),
            operation = self.span.operation.as_str(),
            outcome = self.span.outcome.as_str(),
            duration_ms = self.span.duration_ms,
            redacted_log = %self.redacted_log,
            "managed request observed"
        );
    }
}

impl TenantRequestContext {
    /// Observe one managed request as a span, SLO metric, and redacted log.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if any span, metric, or
    /// gateway field in `input` is invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{
    /// #     Actor, ManagedRequestObservationInput, OpenTelemetryIds, SloOperation,
    /// #     TelemetryOutcome, TenantId, TenantRequestContext, TraceId,
    /// # };
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_request_1").unwrap(),
    ///     Actor::System { name: "managed-api".to_owned() },
    /// );
    /// let observation = ctx.observe_request(
    ///     ManagedRequestObservationInput::new(
    ///         OpenTelemetryIds::new(
    ///             "4bf92f3577b34da6a3ce929d0e0e4736",
    ///             "00f067aa0ba902b7",
    ///             None,
    ///         ).unwrap(),
    ///         SloOperation::Validate,
    ///         "POST",
    ///         "/v1/invoices/validate",
    ///         200,
    ///         TelemetryOutcome::Succeeded,
    ///         15,
    ///         "2026-05-27T06:00:00Z",
    ///     ).unwrap(),
    /// ).unwrap();
    /// assert_eq!(observation.metric.operation.as_str(), "validate");
    /// ```
    pub fn observe_request(
        &self,
        input: ManagedRequestObservationInput,
    ) -> Result<ManagedRequestObservation, ManagedApiError> {
        let mut span = ObservedRequestSpan::new(
            self,
            input.otel,
            input.operation,
            input.http_method,
            input.http_route,
            input.http_status_code,
            input.outcome,
            input.duration_ms,
        )?;
        let mut metric = SloMetricEvent::new(
            self,
            input.operation,
            input.outcome,
            input.duration_ms,
            input.recorded_at,
        )?;
        if let Some(gateway_key) = input.gateway_key {
            span = span.with_gateway_key(gateway_key.clone())?;
            metric = metric.with_gateway_key(gateway_key)?;
        }
        if let Some(failure_kind) = input.failure_kind {
            metric = metric.with_failure_kind(failure_kind)?;
        }
        let redacted_log = redacted_request_log(self, &span, &input.log_fields);
        Ok(ManagedRequestObservation {
            span,
            metric,
            redacted_log,
        })
    }
}

/// Recursively redact sensitive values from a structured log payload.
///
/// The function preserves object keys and array shape while replacing values
/// under known PII or secret keys with [`LOG_REDACTION_PLACEHOLDER`].
///
/// # Examples
///
/// ```
/// # use invoicekit_managed_api::{redact_log_value, LOG_REDACTION_PLACEHOLDER};
/// # use serde_json::json;
/// let redacted = redact_log_value(&json!({"email": "user@example.com", "status": "ok"}));
/// assert_eq!(redacted["email"], LOG_REDACTION_PLACEHOLDER);
/// assert_eq!(redacted["status"], "ok");
/// ```
#[must_use]
pub fn redact_log_value(value: &Value) -> Value {
    redact_named_value(None, value)
}

fn redacted_request_log(
    context: &TenantRequestContext,
    span: &ObservedRequestSpan,
    log_fields: &Value,
) -> Value {
    let mut root = Map::new();
    root.insert(
        "event".to_owned(),
        Value::String("invoicekit.managed_api.request".to_owned()),
    );
    root.insert(
        "service.name".to_owned(),
        Value::String("invoicekit-managed-api".to_owned()),
    );
    root.insert(
        "invoicekit.bead_id".to_owned(),
        Value::String(OBSERVABILITY_BEAD_ID.to_owned()),
    );
    root.insert(
        "invoicekit.tenant_id".to_owned(),
        Value::String(context.tenant_id.as_str().to_owned()),
    );
    root.insert(
        "invoicekit.trace_id".to_owned(),
        Value::String(context.trace_id.as_str().to_owned()),
    );
    root.insert(
        "otel.trace_id".to_owned(),
        Value::String(span.otel.trace_id.clone()),
    );
    root.insert(
        "otel.span_id".to_owned(),
        Value::String(span.otel.span_id.clone()),
    );
    root.insert(
        "invoicekit.slo_operation".to_owned(),
        Value::String(span.operation.as_str().to_owned()),
    );
    root.insert(
        "invoicekit.outcome".to_owned(),
        Value::String(span.outcome.as_str().to_owned()),
    );
    root.insert(
        "invoicekit.duration_ms".to_owned(),
        Value::from(span.duration_ms),
    );
    if let Some(gateway_key) = &span.gateway_key {
        root.insert(
            "invoicekit.gateway_key".to_owned(),
            Value::String(gateway_key.clone()),
        );
    }
    root.insert("fields".to_owned(), redact_log_value(log_fields));
    Value::Object(root)
}

fn redact_named_value(key: Option<&str>, value: &Value) -> Value {
    if key.is_some_and(is_sensitive_key) {
        return redact_value_shape(value);
    }

    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(child_key, child_value)| {
                    (
                        child_key.clone(),
                        redact_named_value(Some(child_key), child_value),
                    )
                })
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_named_value(None, item))
                .collect(),
        ),
        Value::String(text) if looks_like_secret_scalar(text) => {
            Value::String(LOG_REDACTION_PLACEHOLDER.to_owned())
        }
        _ => value.clone(),
    }
}

fn redact_value_shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child_value)| (key.clone(), redact_value_shape(child_value)))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(redact_value_shape).collect::<Vec<Value>>())
        }
        _ => Value::String(LOG_REDACTION_PLACEHOLDER.to_owned()),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    matches!(
        normalized.as_str(),
        "authorization"
            | "apikey"
            | "privatekey"
            | "secretkey"
            | "secret"
            | "password"
            | "token"
            | "email"
            | "phone"
            | "iban"
            | "bankaccount"
            | "bankaccountnumber"
            | "accountnumber"
            | "taxid"
            | "taxidentifier"
            | "taxnumber"
            | "vatid"
            | "vatnumber"
            | "bic"
            | "swift"
            | "routingnumber"
            | "cardnumber"
            | "address"
            | "street"
            | "postalcode"
            | "zipcode"
            | "legalname"
            | "registrationname"
            | "name"
            | "suppliername"
            | "customername"
            | "customer"
            | "supplier"
            | "buyer"
            | "seller"
            | "party"
            | "counterparty"
            | "contact"
    ) || normalized.contains("email")
        || normalized.contains("phone")
        || normalized.contains("iban")
        || normalized.contains("taxid")
        || normalized.contains("taxidentifier")
        || normalized.contains("taxnumber")
        || normalized.contains("vatid")
        || normalized.contains("vatnumber")
        || normalized.contains("bankaccount")
        || normalized.contains("accountnumber")
        || normalized.contains("routingnumber")
        || normalized.contains("cardnumber")
        || normalized.contains("bic")
        || normalized.contains("swift")
        || normalized.contains("address")
        || normalized.contains("postalcode")
        || normalized.contains("zipcode")
        || normalized.contains("legalname")
        || normalized.contains("registrationname")
        || normalized.contains("customer")
        || normalized.contains("supplier")
        || normalized.contains("buyer")
        || normalized.contains("seller")
        || normalized.contains("counterparty")
        || normalized.contains("contact")
        || normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("name")
}

fn looks_like_secret_scalar(text: &str) -> bool {
    let trimmed = text.trim();
    let lowered = trimmed.to_ascii_lowercase();
    lowered.starts_with("bearer ")
        || lowered.starts_with("basic ")
        || lowered.starts_with("sk_")
        || lowered.starts_with("pk_")
        || lowered.starts_with("rk_")
        || lowered.starts_with("tok_")
        || lowered.starts_with("ghp_")
        || lowered.starts_with("key_")
        || looks_like_jwt(trimmed)
        || (trimmed.contains('@') && trimmed.contains('.'))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn looks_like_jwt(text: &str) -> bool {
    let mut parts = text.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(signature), None) => {
            header.len() >= 8
                && payload.len() >= 8
                && signature.len() >= 8
                && header.bytes().all(is_base64url_byte)
                && payload.bytes().all(is_base64url_byte)
                && signature.bytes().all(is_base64url_byte)
        }
        _ => false,
    }
}

fn is_base64url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn metric_in_window(metric: &SloMetricEvent, window_start: &str, window_end: &str) -> bool {
    metric.recorded_at.as_str() >= window_start && metric.recorded_at.as_str() < window_end
}

fn validate_otel_trace_id(value: &str) -> Result<String, ManagedApiError> {
    if value.len() != 32 {
        return Err(ManagedApiError::InvalidIdentifier {
            field: "otel_trace_id",
            reason: "OpenTelemetry trace_id must be 32 hexadecimal characters",
        });
    }
    validate_hex_not_zero("otel_trace_id", value.to_owned())
}

fn validate_otel_span_id(field: &'static str, value: &str) -> Result<String, ManagedApiError> {
    if value.len() != 16 {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "OpenTelemetry span_id must be 16 hexadecimal characters",
        });
    }
    validate_hex_not_zero(field, value.to_owned())
}

fn validate_otel_trace_flags(value: &str) -> Result<String, ManagedApiError> {
    if value.len() != 2 {
        return Err(ManagedApiError::InvalidIdentifier {
            field: "otel_trace_flags",
            reason: "OpenTelemetry trace flags must be two hexadecimal characters",
        });
    }
    if !value.bytes().all(is_lower_hex_byte) {
        return Err(ManagedApiError::InvalidIdentifier {
            field: "otel_trace_flags",
            reason: "OpenTelemetry trace flags must be lowercase hexadecimal",
        });
    }
    Ok(value.to_owned())
}

fn validate_hex_not_zero(field: &'static str, value: String) -> Result<String, ManagedApiError> {
    if !value.bytes().all(is_lower_hex_byte) {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "OpenTelemetry identifier must contain only lowercase hexadecimal characters",
        });
    }
    if value.bytes().all(|byte| matches!(byte, b'0')) {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "OpenTelemetry identifier must not be all zeroes",
        });
    }
    Ok(value)
}

fn is_lower_hex_byte(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}

fn parts_per_million(numerator: u64, denominator: u64) -> Option<u64> {
    (denominator > 0).then(|| {
        let value = u128::from(numerator).saturating_mul(1_000_000) / u128::from(denominator);
        u64::try_from(value).unwrap_or(u64::MAX)
    })
}

fn percentile_nearest_rank(sorted: &[u64], percentile: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = sorted.len().saturating_mul(percentile).saturating_add(99) / 100;
    sorted.get(rank.saturating_sub(1)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Actor, TenantId, TraceId};
    use serde_json::json;

    #[test]
    fn otel_request_span_serializes_traceparent_and_attributes() {
        let span = ObservedRequestSpan::new(
            &context("trace_request_1"),
            otel_ids(),
            SloOperation::Validate,
            "POST",
            "/v1/invoices/validate",
            200,
            TelemetryOutcome::Succeeded,
            17,
        )
        .unwrap()
        .with_gateway_key("mock-peppol")
        .unwrap();

        let value = span.to_otel_json();

        assert_eq!(
            span.traceparent(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
        assert_eq!(value["trace_flags"], "01");
        assert_eq!(value["kind"], "server");
        assert_eq!(value["attributes"]["invoicekit.tenant_id"], "tenant_acme");
        assert_eq!(
            value["attributes"]["invoicekit.trace_id"],
            "trace_request_1"
        );
        assert_eq!(value["attributes"]["invoicekit.slo_operation"], "validate");
        assert_eq!(value["attributes"]["invoicekit.gateway_key"], "mock-peppol");
    }

    #[test]
    fn otel_ids_reject_invalid_trace_and_span_identifiers() {
        assert!(matches!(
            OpenTelemetryIds::new("short", "00f067aa0ba902b7", None),
            Err(ManagedApiError::InvalidIdentifier {
                field: "otel_trace_id",
                ..
            })
        ));
        assert!(matches!(
            OpenTelemetryIds::new("00000000000000000000000000000000", "00f067aa0ba902b7", None,),
            Err(ManagedApiError::InvalidIdentifier {
                field: "otel_trace_id",
                ..
            })
        ));
        assert!(matches!(
            OpenTelemetryIds::new("4bf92f3577b34da6a3ce929d0e0e4736", "not-hex-span-id", None,),
            Err(ManagedApiError::InvalidIdentifier {
                field: "otel_span_id",
                ..
            })
        ));
        assert!(matches!(
            OpenTelemetryIds::new("4BF92F3577B34DA6A3CE929D0E0E4736", "00f067aa0ba902b7", None,),
            Err(ManagedApiError::InvalidIdentifier {
                field: "otel_trace_id",
                ..
            })
        ));
        let unsampled = OpenTelemetryIds::new_with_trace_flags(
            "4bf92f3577b34da6a3ce929d0e0e4736",
            "00f067aa0ba902b7",
            None,
            "00",
        )
        .unwrap();
        assert_eq!(
            unsampled.traceparent(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"
        );
    }

    #[test]
    fn slo_metric_contract_covers_required_operations() {
        let operations = SloOperation::ALL
            .into_iter()
            .map(SloOperation::as_str)
            .collect::<Vec<_>>();

        assert_eq!(
            operations,
            vec![
                "validate",
                "render",
                "transmit-enqueue",
                "gateway-accepted",
                "archive-write",
                "webhook-deliver",
            ]
        );

        let metric = SloMetricEvent::new(
            &context("trace_metric_1"),
            SloOperation::WebhookDeliver,
            TelemetryOutcome::Failed,
            250,
            "2026-05-27T06:00:00Z",
        )
        .unwrap()
        .with_gateway_key("mock-peppol")
        .unwrap()
        .with_failure_kind("timeout")
        .unwrap();

        let value = metric.to_otel_json();
        assert_eq!(value["name"], "invoicekit.slo.operation.duration_ms");
        assert_eq!(value["unit"], "ms");
        assert_eq!(
            value["attributes"]["invoicekit.slo_operation"],
            "webhook-deliver"
        );
        assert_eq!(value["attributes"]["invoicekit.failure_kind"], "timeout");
    }

    #[test]
    fn gateway_dashboard_aggregates_gateway_scoped_metrics() {
        let ctx = context("trace_metric_2");
        let metrics = vec![
            metric(
                &ctx,
                SloOperation::GatewayAccepted,
                TelemetryOutcome::Succeeded,
                10,
                None,
            ),
            metric(
                &ctx,
                SloOperation::GatewayAccepted,
                TelemetryOutcome::Succeeded,
                20,
                None,
            ),
            metric(
                &ctx,
                SloOperation::GatewayAccepted,
                TelemetryOutcome::Failed,
                200,
                Some("timeout"),
            ),
            metric(
                &ctx,
                SloOperation::TransmitEnqueue,
                TelemetryOutcome::Succeeded,
                30,
                None,
            ),
            SloMetricEvent::new(
                &ctx,
                SloOperation::GatewayAccepted,
                TelemetryOutcome::Succeeded,
                1,
                "2026-05-27T07:00:00Z",
            )
            .unwrap()
            .with_gateway_key("mock-peppol")
            .unwrap(),
            SloMetricEvent::new(
                &ctx,
                SloOperation::GatewayAccepted,
                TelemetryOutcome::Failed,
                999,
                "2026-05-27T06:00:00Z",
            )
            .unwrap()
            .with_gateway_key("other-gateway")
            .unwrap()
            .with_failure_kind("ignored")
            .unwrap(),
        ];

        let snapshot = GatewayDashboardSnapshot::from_metrics(
            "mock-peppol",
            "2026-05-27T06:00:00Z",
            "2026-05-27T07:00:00Z",
            metrics,
        )
        .unwrap();

        assert_eq!(snapshot.total_events, 4);
        assert_eq!(snapshot.succeeded, 3);
        assert_eq!(snapshot.failed, 1);
        assert_eq!(snapshot.availability_ppm, Some(750_000));
        assert_eq!(snapshot.gateway_acceptance_events, 3);
        assert_eq!(snapshot.gateway_accepted, 2);
        assert_eq!(snapshot.gateway_rejected, 1);
        assert_eq!(snapshot.gateway_acceptance_ppm, Some(666_666));
        assert_eq!(snapshot.p95_duration_ms, Some(200));
        assert_eq!(snapshot.failure_counts.get("timeout"), Some(&1));
    }

    #[test]
    fn log_redactor_masks_pii_and_preserves_shape() {
        let payload = json!({
            "tenant_id": "tenant_acme",
            "customer": {
                "name": "Alice Example",
                "email": "alice@example.com",
                "address": {
                    "street": "Main Street 1",
                    "postal_code": "10115"
                }
            },
            "metadata": {
                "customerEmail": "buyer@example.com",
                "vatId": "DE123456789",
                "vatNumber": "DE987654321",
                "bankAccountNumber": "DE89370400440532013000",
                "routingNumber": "021000021",
                "swift": "DEUTDEFF",
                "legal_name": "Example GmbH",
                "jwt": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
            },
            "lines": [
                {"description": "Validation service", "amount": "10.00"}
            ],
            "authorization": "bearer very-secret",
            "status": "accepted"
        });

        let redacted = redact_log_value(&payload);

        assert_eq!(redacted["tenant_id"], "tenant_acme");
        assert_eq!(redacted["customer"]["name"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(redacted["customer"]["email"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(
            redacted["customer"]["address"]["street"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(
            redacted["metadata"]["customerEmail"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(redacted["metadata"]["vatId"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(redacted["metadata"]["vatNumber"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(
            redacted["metadata"]["bankAccountNumber"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(
            redacted["metadata"]["routingNumber"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(redacted["metadata"]["swift"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(
            redacted["metadata"]["legal_name"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(redacted["metadata"]["jwt"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(redacted["lines"][0]["description"], "Validation service");
        assert_eq!(redacted["authorization"], LOG_REDACTION_PLACEHOLDER);
        assert_eq!(redacted["status"], "accepted");
    }

    #[test]
    fn managed_request_observation_emits_span_metric_and_redacted_log() {
        let observation = context("trace_request_2")
            .observe_request(
                ManagedRequestObservationInput::new(
                    otel_ids(),
                    SloOperation::ArchiveWrite,
                    "POST",
                    "/v1/archive/write",
                    201,
                    TelemetryOutcome::Succeeded,
                    44,
                    "2026-05-27T06:00:00Z",
                )
                .unwrap()
                .with_gateway_key("mock-peppol")
                .unwrap()
                .with_log_fields(json!({
                    "status": "stored",
                    "contact": {
                        "email": "billing@example.com",
                        "phone": "+49 30 123"
                    }
                })),
            )
            .unwrap();

        assert_eq!(observation.span.operation, SloOperation::ArchiveWrite);
        assert_eq!(observation.metric.operation, SloOperation::ArchiveWrite);
        assert_eq!(
            observation.redacted_log["invoicekit.bead_id"],
            OBSERVABILITY_BEAD_ID
        );
        assert_eq!(
            observation.redacted_log["fields"]["contact"]["email"],
            LOG_REDACTION_PLACEHOLDER
        );
        assert_eq!(observation.redacted_log["fields"]["status"], "stored");
        assert_eq!(
            observation.to_otel_json()["metric"]["attributes"]["invoicekit.gateway_key"],
            "mock-peppol"
        );
        observation.emit_tracing_log();
    }

    fn context(trace_id: &str) -> TenantRequestContext {
        TenantRequestContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new(trace_id).unwrap(),
            Actor::System {
                name: "managed-api".to_owned(),
            },
        )
    }

    fn otel_ids() -> OpenTelemetryIds {
        OpenTelemetryIds::new("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7", None).unwrap()
    }

    fn metric(
        context: &TenantRequestContext,
        operation: SloOperation,
        outcome: TelemetryOutcome,
        duration_ms: u64,
        failure_kind: Option<&str>,
    ) -> SloMetricEvent {
        let metric = SloMetricEvent::new(
            context,
            operation,
            outcome,
            duration_ms,
            "2026-05-27T06:00:00Z",
        )
        .unwrap()
        .with_gateway_key("mock-peppol")
        .unwrap();

        if let Some(failure_kind) = failure_kind {
            metric.with_failure_kind(failure_kind).unwrap()
        } else {
            metric
        }
    }
}
