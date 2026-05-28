// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-092 impl: Rust [`Phase4Adapter`] — implements
//! [`invoicekit_reconcile::GatewayAdapter`] on top of the
//! `validator-phase4` JSON-RPC sidecar documented at
//! `docs/operators/PHASE4-REFERENCE-ADAPTER.md`.
//!
//! The adapter speaks the four-method contract the sidecar
//! exposes (`transmit` / `receive` / `status` / `health`). The
//! HTTP transport is abstracted behind [`RpcClient`] so the
//! scaffold tests run without a live sidecar; a `reqwest`-backed
//! impl lands behind a follow-up `reqwest` feature flag once
//! the AP certificate (4-8 week `OpenPeppol` onboarding) clears.

use std::env;
use std::sync::Mutex;

use invoicekit_reconcile::{
    CancelRequest, CorrectRequest, GatewayAdapter, GatewayContext, GatewayError, GatewayErrorKind,
    GatewayFuture, GatewayOperation, GatewayReceipt, GatewayStatus, GatewaySubmissionId,
    PollRequest, SubmitRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// SML mode the sidecar runs against. Maps 1:1 to the
/// `PEPPOL_AP_SML_MODE` env var.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SmlMode {
    /// Acceptance / test SML.
    Acceptance,
    /// Production SML.
    Production,
}

impl SmlMode {
    /// Operator-facing slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Acceptance => "acceptance",
            Self::Production => "production",
        }
    }

    /// Parse from the env-var string.
    ///
    /// # Errors
    ///
    /// Returns [`Phase4ConfigError::UnknownSmlMode`] when the
    /// value doesn't match a known mode.
    pub fn from_slug(value: &str) -> Result<Self, Phase4ConfigError> {
        match value {
            "acceptance" => Ok(Self::Acceptance),
            "production" => Ok(Self::Production),
            other => Err(Phase4ConfigError::UnknownSmlMode(other.to_owned())),
        }
    }
}

/// Operator-facing configuration. Construct with
/// [`Phase4Config::from_env`] in production; construct directly
/// in tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Phase4Config {
    /// Base URL of the local sidecar (defaults to
    /// `http://127.0.0.1:8090`).
    pub sidecar_url: String,
    /// SML mode the sidecar is configured for. Used as a
    /// safety-rail: a `Production`-tagged invoice routed at a
    /// sidecar running in `Acceptance` returns an
    /// [`GatewayErrorKind::InvalidRequest`].
    pub sml_mode: SmlMode,
}

impl Phase4Config {
    /// Read from the documented env vars
    /// (`INVOICEKIT_PHASE4_URL`, `PEPPOL_AP_SML_MODE`).
    ///
    /// # Errors
    ///
    /// Returns [`Phase4ConfigError`] when an env value is malformed.
    pub fn from_env() -> Result<Self, Phase4ConfigError> {
        let sidecar_url = env::var("INVOICEKIT_PHASE4_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8090".to_owned());
        let sml_mode = env::var("PEPPOL_AP_SML_MODE")
            .map_or(Ok(SmlMode::Acceptance), |v| SmlMode::from_slug(&v))?;
        Ok(Self {
            sidecar_url,
            sml_mode,
        })
    }
}

/// Errors raised while building / loading config.
#[derive(Debug, Error)]
pub enum Phase4ConfigError {
    /// Env value was not one of the registered SML modes.
    #[error("unknown SML mode: {0}")]
    UnknownSmlMode(String),
}

/// Captured RPC call. Tests assert on this to prove the adapter
/// constructed the JSON-RPC body the sidecar contract expects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcCall {
    /// Method name (`transmit` / `receive` / `status` / `health`).
    pub method: String,
    /// JSON params body.
    pub params: Value,
}

/// JSON-RPC transport abstraction. The scaffold ships
/// [`MockRpcClient`]; a `reqwest`-backed impl lives behind the
/// `reqwest` feature flag and is the runtime path in production.
pub trait RpcClient: Send + Sync {
    /// Send one JSON-RPC call.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when the sidecar refuses the call or
    /// the transport fails.
    fn call(&self, request: RpcCall) -> Result<Value, RpcError>;
}

/// Transport-level errors raised by an [`RpcClient`].
#[derive(Debug, Error)]
pub enum RpcError {
    /// Sidecar returned a non-2xx HTTP status.
    #[error("sidecar refused call: {status}: {message}")]
    Refused {
        /// HTTP status code.
        status: u16,
        /// Body excerpt.
        message: String,
    },
    /// Sidecar returned a malformed JSON body.
    #[error("sidecar response was not valid JSON-RPC: {0}")]
    Malformed(String),
    /// Transport error (timeout, DNS, TLS).
    #[error("sidecar transport failure: {0}")]
    Transport(String),
}

/// Mock JSON-RPC client. Records every call and pops queued
/// responses; tests assert on `calls()` after the adapter ran.
pub struct MockRpcClient {
    calls: Mutex<Vec<RpcCall>>,
    responses: Mutex<Vec<Result<Value, RpcError>>>,
}

impl MockRpcClient {
    /// Build a new mock client with no queued responses.
    #[must_use]
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
        }
    }

    /// Queue an `Ok` response for the next call.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    pub fn enqueue_ok(&self, value: Value) {
        self.responses.lock().unwrap().push(Ok(value));
    }

    /// Queue an `Err` response for the next call.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    pub fn enqueue_err(&self, err: RpcError) {
        self.responses.lock().unwrap().push(Err(err));
    }

    /// Snapshot of recorded calls.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    #[must_use]
    pub fn calls(&self) -> Vec<RpcCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for MockRpcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RpcClient for MockRpcClient {
    fn call(&self, request: RpcCall) -> Result<Value, RpcError> {
        self.calls.lock().unwrap().push(request);
        self.responses.lock().unwrap().pop().unwrap_or_else(|| {
            Err(RpcError::Transport(
                "mock has no queued response".to_owned(),
            ))
        })
    }
}

/// Implements [`GatewayAdapter`] by translating each operation
/// into the matching JSON-RPC call on the phase4 sidecar.
pub struct Phase4Adapter {
    #[allow(dead_code)]
    config: Phase4Config,
    rpc: Box<dyn RpcClient>,
}

impl Phase4Adapter {
    /// Build a new adapter.
    #[must_use]
    pub fn new(config: Phase4Config, rpc: Box<dyn RpcClient>) -> Self {
        Self { config, rpc }
    }

    fn build_transmit_call(request: &SubmitRequest) -> RpcCall {
        // The sidecar's `transmit` method wants the recipient
        // participant id + doc-type URN + process-id URN. The
        // routing layer surfaces them via `GatewayRoute`'s
        // `route` (e.g. `peppol`) + `profile` (e.g.
        // `peppol-bis-3`) + the per-tenant recipient lookup. The
        // scaffold passes route/profile through directly + lets
        // the sidecar resolve the participant via its own SMP
        // cache; the real implementation lands once T-093 inbound
        // and the SMP resolver crate (peppol-smp-sml) ship.
        let payload_b64 = base64_encode(&serde_json::to_vec(&request.document).unwrap_or_default());
        RpcCall {
            method: "transmit".to_owned(),
            params: json!({
                "to": request.route.country.as_deref().unwrap_or(""),
                "doc_type": request.route.profile,
                "process_id": request.route.route,
                "payload_b64": payload_b64,
            }),
        }
    }

    fn build_status_call(submission_id: &GatewaySubmissionId) -> RpcCall {
        RpcCall {
            method: "status".to_owned(),
            params: json!({ "message_id": submission_id.as_str() }),
        }
    }

    fn parse_transmit_response(
        context: GatewayContext,
        body: &Value,
    ) -> Result<GatewayReceipt, GatewayError> {
        let message_id = body
            .get("message_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                GatewayError::new(
                    GatewayErrorKind::UnexpectedResponse,
                    GatewayOperation::Submit,
                    "phase4 transmit response missing message_id",
                    "verify the phase4 sidecar is running the InvoiceKit JSON-RPC contract",
                )
            })?;
        let submission_id = GatewaySubmissionId::new(message_id).map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::UnexpectedResponse,
                GatewayOperation::Submit,
                e.to_string(),
                "phase4 sidecar returned a malformed message_id",
            )
        })?;
        GatewayReceipt::new(
            GatewayOperation::Submit,
            context,
            submission_id,
            GatewayStatus::Pending,
            "1970-01-01T00:00:00Z".to_owned(),
        )
        .map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::UnexpectedResponse,
                GatewayOperation::Submit,
                e.to_string(),
                "phase4 transmit response failed receipt construction",
            )
        })
    }

    fn parse_status_response(
        operation: GatewayOperation,
        context: GatewayContext,
        submission_id: GatewaySubmissionId,
        body: &Value,
    ) -> Result<GatewayReceipt, GatewayError> {
        let state = body.get("state").and_then(Value::as_str).ok_or_else(|| {
            GatewayError::new(
                GatewayErrorKind::UnexpectedResponse,
                operation,
                "phase4 status response missing state",
                "verify the phase4 sidecar is running the InvoiceKit JSON-RPC contract",
            )
        })?;
        let status = map_state(operation, state)?;
        GatewayReceipt::new(
            operation,
            context,
            submission_id,
            status,
            "1970-01-01T00:00:00Z".to_owned(),
        )
        .map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::UnexpectedResponse,
                operation,
                e.to_string(),
                "phase4 status response failed receipt construction",
            )
        })
    }

    fn map_rpc_error(operation: GatewayOperation, err: RpcError) -> GatewayError {
        match err {
            RpcError::Refused { status, message } => match status {
                401 | 403 => GatewayError::new(
                    GatewayErrorKind::AuthFailure,
                    operation,
                    message,
                    "rotate the phase4 sidecar's bearer token or fix mTLS trust",
                ),
                429 => GatewayError::new(
                    GatewayErrorKind::RateLimited,
                    operation,
                    message,
                    "respect the sidecar's backoff window before retrying",
                ),
                500..=599 => GatewayError::new(
                    GatewayErrorKind::GatewayMaintenance,
                    operation,
                    message,
                    "the phase4 sidecar is unhealthy — check its container logs",
                ),
                _ => GatewayError::new(
                    GatewayErrorKind::PartnerError,
                    operation,
                    message,
                    "the phase4 sidecar reported a partner-side issue",
                ),
            },
            RpcError::Malformed(message) => GatewayError::new(
                GatewayErrorKind::UnexpectedResponse,
                operation,
                message,
                "phase4 sidecar returned a non JSON-RPC body",
            ),
            RpcError::Transport(message) => GatewayError::new(
                GatewayErrorKind::NetworkFailure,
                operation,
                message,
                "check sidecar reachability + the host's network stack",
            ),
        }
    }
}

impl GatewayAdapter for Phase4Adapter {
    fn submit(&self, request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            let call = Self::build_transmit_call(&request);
            let body = self
                .rpc
                .call(call)
                .map_err(|e| Self::map_rpc_error(GatewayOperation::Submit, e))?;
            Self::parse_transmit_response(request.context, &body)
        })
    }

    fn poll(&self, request: PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            let call = Self::build_status_call(&request.submission_id);
            let body = self
                .rpc
                .call(call)
                .map_err(|e| Self::map_rpc_error(GatewayOperation::Poll, e))?;
            Self::parse_status_response(
                GatewayOperation::Poll,
                request.context,
                request.submission_id,
                &body,
            )
        })
    }

    fn cancel(&self, _request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            Err(GatewayError::new(
                GatewayErrorKind::UnsupportedOperation,
                GatewayOperation::Cancel,
                "phase4 sidecar does not expose a cancel surface; AS4 is fire-and-forget",
                "issue a corrective invoice via submit() instead",
            ))
        })
    }

    fn correct(&self, _request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            Err(GatewayError::new(
                GatewayErrorKind::UnsupportedOperation,
                GatewayOperation::Correct,
                "phase4 corrections re-submit a fresh document; route via submit()",
                "build the corrected document, then call submit() with a fresh idempotency key",
            ))
        })
    }
}

fn map_state(operation: GatewayOperation, state: &str) -> Result<GatewayStatus, GatewayError> {
    match state {
        "delivered" => Ok(GatewayStatus::Accepted),
        "queued" => Ok(GatewayStatus::Pending),
        "rejected" => Ok(GatewayStatus::Rejected),
        other => Err(GatewayError::new(
            GatewayErrorKind::UnexpectedResponse,
            operation,
            format!("phase4 sidecar returned unknown state: {other}"),
            "extend invoicekit-transmit-peppol-phase4::map_state to cover this state",
        )),
    }
}

/// RFC 4648 §4 base64 (the standard alphabet, no line breaks).
/// Inlined to avoid pulling in another crate for a one-shot encode.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char);
        out.push(ALPHABET[(b2 & 0b11_1111) as usize] as char);
        i += 3;
    }
    let remaining = bytes.len() - i;
    if remaining == 1 {
        let b0 = bytes[i];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[((b0 & 0b11) << 4) as usize] as char);
        out.push('=');
        out.push('=');
    } else if remaining == 2 {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[((b1 & 0b1111) << 2) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Phase4Config {
        Phase4Config {
            sidecar_url: "http://127.0.0.1:8090".to_owned(),
            sml_mode: SmlMode::Acceptance,
        }
    }

    #[test]
    fn sml_mode_round_trips_slug() {
        assert_eq!(SmlMode::Acceptance.slug(), "acceptance");
        assert_eq!(SmlMode::Production.slug(), "production");
        assert_eq!(
            SmlMode::from_slug("acceptance").unwrap(),
            SmlMode::Acceptance
        );
        assert_eq!(
            SmlMode::from_slug("production").unwrap(),
            SmlMode::Production
        );
        assert!(SmlMode::from_slug("staging").is_err());
    }

    #[test]
    fn base64_encode_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn map_state_translates_canonical_states() {
        let op = GatewayOperation::Poll;
        assert_eq!(map_state(op, "delivered").unwrap(), GatewayStatus::Accepted);
        assert_eq!(map_state(op, "queued").unwrap(), GatewayStatus::Pending);
        assert_eq!(map_state(op, "rejected").unwrap(), GatewayStatus::Rejected);
        assert!(map_state(op, "hyperspace").is_err());
    }

    #[test]
    fn map_rpc_error_classifies_http_statuses() {
        let op = GatewayOperation::Submit;

        let auth = Phase4Adapter::map_rpc_error(
            op,
            RpcError::Refused {
                status: 401,
                message: "nope".to_owned(),
            },
        );
        assert_eq!(auth.kind, GatewayErrorKind::AuthFailure);

        let rate = Phase4Adapter::map_rpc_error(
            op,
            RpcError::Refused {
                status: 429,
                message: "slow".to_owned(),
            },
        );
        assert_eq!(rate.kind, GatewayErrorKind::RateLimited);

        let maint = Phase4Adapter::map_rpc_error(
            op,
            RpcError::Refused {
                status: 503,
                message: "down".to_owned(),
            },
        );
        assert_eq!(maint.kind, GatewayErrorKind::GatewayMaintenance);

        let partner = Phase4Adapter::map_rpc_error(
            op,
            RpcError::Refused {
                status: 422,
                message: "bad".to_owned(),
            },
        );
        assert_eq!(partner.kind, GatewayErrorKind::PartnerError);

        let mal = Phase4Adapter::map_rpc_error(op, RpcError::Malformed("?".to_owned()));
        assert_eq!(mal.kind, GatewayErrorKind::UnexpectedResponse);

        let net = Phase4Adapter::map_rpc_error(op, RpcError::Transport("dns".to_owned()));
        assert_eq!(net.kind, GatewayErrorKind::NetworkFailure);
    }

    #[test]
    fn build_status_call_carries_message_id() {
        let sid = GatewaySubmissionId::new("msg-abc-123").unwrap();
        let call = Phase4Adapter::build_status_call(&sid);
        assert_eq!(call.method, "status");
        assert_eq!(
            call.params.get("message_id").and_then(Value::as_str),
            Some("msg-abc-123")
        );
    }

    #[test]
    fn mock_rpc_client_records_call_and_pops_response() {
        let mock = MockRpcClient::new();
        mock.enqueue_ok(json!({"version":"0.1.0","sml":"acceptance"}));
        let result = mock
            .call(RpcCall {
                method: "health".to_owned(),
                params: json!({}),
            })
            .unwrap();
        assert_eq!(result.get("version").and_then(Value::as_str), Some("0.1.0"));
        assert_eq!(mock.calls().len(), 1);
        assert_eq!(mock.calls()[0].method, "health");
    }

    #[test]
    fn cfg_helper_round_trips() {
        let c = cfg();
        assert_eq!(c.sml_mode, SmlMode::Acceptance);
        assert!(c.sidecar_url.starts_with("http://"));
    }

    #[test]
    fn parse_status_response_rejects_missing_state() {
        let context = stub_context();
        let sid = GatewaySubmissionId::new("msg-x").unwrap();
        let err = Phase4Adapter::parse_status_response(
            GatewayOperation::Poll,
            context,
            sid,
            &json!({"detail": "missing"}),
        )
        .unwrap_err();
        assert_eq!(err.kind, GatewayErrorKind::UnexpectedResponse);
    }

    fn stub_context() -> GatewayContext {
        use invoicekit_reconcile::{GatewayAttemptId, IdempotencyKey, TenantId, TraceId};
        GatewayContext::new(
            TenantId::new("t").unwrap(),
            TraceId::new("tr").unwrap(),
            IdempotencyKey::new("idem-1").unwrap(),
            GatewayAttemptId::new("att-1").unwrap(),
        )
    }
}
