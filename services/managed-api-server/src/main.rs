// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-managed-api-server` — InvoiceKit workspace binary.
//!
//! See [`plans/PLAN.md`](../../../plans/PLAN.md) for the architectural role
//! of this binary. The no-arg entry point remains the workspace-identity
//! handshake every InvoiceKit binary exposes; the request dispatcher below is
//! the managed service boundary that routes every handled request through the
//! T-136 observation path before returning.

use invoicekit_managed_api::{
    ManagedApiError, ManagedRequestObservation, ManagedRequestObservationInput,
    TenantRequestContext,
};
use serde_json::Value;

const CRATE_NAME: &str = "invoicekit-managed-api-server";

fn main() {
    // Workspace-identity handshake. Returning silently is the documented
    // contract for the no-arg invocation; downstream beads add subcommands.
    let _ = CRATE_NAME;
}

/// Request handed to the managed API dispatcher after routing and auth.
#[derive(Clone, Debug)]
pub struct ManagedApiServerRequest {
    context: TenantRequestContext,
    observation: ManagedRequestObservationInput,
    log_fields: Value,
}

impl ManagedApiServerRequest {
    /// Build a dispatcher request with prevalidated tenant context and
    /// operation observation inputs.
    #[must_use]
    pub fn new(
        context: TenantRequestContext,
        observation: ManagedRequestObservationInput,
        log_fields: Value,
    ) -> Self {
        Self {
            context,
            observation,
            log_fields,
        }
    }
}

/// Response returned by the managed API dispatcher.
#[derive(Clone, Debug)]
pub struct ManagedApiServerResponse {
    /// Final HTTP status code selected by the route handler.
    pub status_code: u16,
    /// W3C `traceparent` propagated to downstream calls and response metadata.
    pub traceparent: String,
    /// Span, metric, and redacted log emitted for this request.
    pub observation: ManagedRequestObservation,
}

/// Handle one managed request and emit its T-136 observation.
///
/// # Errors
///
/// Returns [`ManagedApiError`] when the request observation inputs fail
/// validation.
pub fn handle_request(
    request: ManagedApiServerRequest,
) -> Result<ManagedApiServerResponse, ManagedApiError> {
    let observation = request
        .context
        .observe_request(request.observation.with_log_fields(request.log_fields))?;
    observation.emit_tracing_log();
    Ok(ManagedApiServerResponse {
        status_code: observation.span.http_status_code,
        traceparent: observation.span.traceparent(),
        observation,
    })
}

#[cfg(test)]
mod tests {
    use super::{handle_request, ManagedApiServerRequest, CRATE_NAME};
    use invoicekit_managed_api::{
        Actor, ManagedRequestObservationInput, OpenTelemetryIds, SloOperation, TelemetryOutcome,
        TenantId, TenantRequestContext, TraceId, LOG_REDACTION_PLACEHOLDER,
    };
    use serde_json::json;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(CRATE_NAME, "invoicekit-managed-api-server");
    }

    #[test]
    fn main_returns_without_panic() {
        super::main();
    }

    #[test]
    fn main_is_idempotent() {
        super::main();
        super::main();
    }

    #[test]
    fn dispatcher_observes_every_slo_operation() {
        for operation in SloOperation::ALL {
            let response = handle_request(request_for(operation)).unwrap();

            assert_eq!(response.status_code, 200);
            assert!(response.traceparent.starts_with("00-"));
            assert_eq!(response.observation.span.operation, operation);
            assert_eq!(response.observation.metric.operation, operation);
            assert_eq!(
                response.observation.redacted_log["fields"]["customerEmail"],
                LOG_REDACTION_PLACEHOLDER
            );
            assert_eq!(
                response.observation.redacted_log["fields"]["status"],
                "accepted"
            );
        }
    }

    #[test]
    fn dispatcher_preserves_gateway_observation_fields() {
        let response = handle_request(ManagedApiServerRequest::new(
            context("trace_gateway"),
            input_for(SloOperation::GatewayAccepted)
                .with_gateway_key("mock-peppol")
                .unwrap()
                .with_failure_kind("timeout")
                .unwrap(),
            json!({"authorization": "Bearer secret", "status": "retry"}),
        ))
        .unwrap();

        assert_eq!(
            response.observation.span.gateway_key.as_deref(),
            Some("mock-peppol")
        );
        assert_eq!(
            response.observation.metric.failure_kind.as_deref(),
            Some("timeout")
        );
        assert_eq!(
            response.observation.redacted_log["fields"]["authorization"],
            LOG_REDACTION_PLACEHOLDER
        );
    }

    fn request_for(operation: SloOperation) -> ManagedApiServerRequest {
        ManagedApiServerRequest::new(
            context(&format!("trace_{}", operation.as_str().replace('-', "_"))),
            input_for(operation),
            json!({
                "customerEmail": "billing@example.com",
                "status": "accepted"
            }),
        )
    }

    fn input_for(operation: SloOperation) -> ManagedRequestObservationInput {
        ManagedRequestObservationInput::new(
            OpenTelemetryIds::new("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7", None)
                .unwrap(),
            operation,
            "POST",
            format!("/v1/{}", operation.as_str()),
            200,
            TelemetryOutcome::Succeeded,
            12,
            "2026-05-27T06:00:00Z",
        )
        .unwrap()
    }

    fn context(trace_id: &str) -> TenantRequestContext {
        TenantRequestContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new(trace_id).unwrap(),
            Actor::System {
                name: "managed-api-server".to_owned(),
            },
        )
    }
}
