// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-transmit-mock` — InvoiceKit workspace member.
//!
//! This crate owns the deterministic cassette format used by the mock
//! transmission gateway. T-074 wires these types into the
//! `GatewayAdapter`; T-074a provides the recorder, scrubber, matcher, and
//! scenario metadata contract that downstream country crates can build on.
//!
//! Cassettes are JSON `.vcr` documents. Request matching is keyed by
//! method, path, and a BLAKE3 body fingerprint so a fixture never silently
//! reuses the wrong gateway response.

use std::collections::{BTreeMap, BTreeSet};

use invoicekit_ir::CommercialDocument;
use invoicekit_reconcile::{
    CancelRequest, CorrectRequest, GatewayAdapter, GatewayAttemptId, GatewayContext, GatewayError,
    GatewayErrorKind, GatewayFuture, GatewayOperation, GatewayReceipt, GatewayRoute, GatewayStatus,
    GatewaySubmissionId, IdempotencyKey, PollRequest, ReconcileError, SubmitRequest, TenantId,
    TraceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Request path used for mock submit cassette matching.
pub const MOCK_SUBMIT_PATH: &str = "/mock/submit";
/// Request path used for mock poll cassette matching.
pub const MOCK_POLL_PATH: &str = "/mock/poll";
/// Request path used for mock cancel cassette matching.
pub const MOCK_CANCEL_PATH: &str = "/mock/cancel";
/// Request path used for mock correction cassette matching.
pub const MOCK_CORRECT_PATH: &str = "/mock/correct";

/// JSON Schema document for `scenario.json` files stored beside cassettes.
///
/// The schema is intentionally small: each cassette directory names one
/// scenario, the source sandbox, the route being exercised, and the
/// scrubber profile used before committing the recording.
pub const SCENARIO_METADATA_SCHEMA_JSON: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://invoicekit.dev/schemas/cassette-scenario-v1.json",
  "title": "InvoiceKit cassette scenario metadata",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema_version",
    "scenario_id",
    "title",
    "country",
    "route",
    "source",
    "scrubber_profile"
  ],
  "properties": {
    "schema_version": { "const": "1.0" },
    "scenario_id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9._/-]*$" },
    "title": { "type": "string", "minLength": 1 },
    "country": { "type": "string", "pattern": "^[A-Z]{2}$" },
    "route": { "type": "string", "minLength": 1 },
    "source": {
      "type": "string",
      "enum": ["official-sandbox", "partner-sandbox", "synthetic"]
    },
    "scrubber_profile": { "type": "string", "minLength": 1 },
    "description": { "type": "string" },
    "tags": {
      "type": "array",
      "items": { "type": "string", "minLength": 1 },
      "uniqueItems": true
    }
  }
}"#;

/// Errors returned by cassette recording, scrubbing, and matching.
#[derive(Debug, Error)]
pub enum CassetteError {
    /// Required field was blank or absent.
    #[error(
        "{field} is required; hint: cassette records must be complete before they are committed"
    )]
    MissingRequiredField {
        /// Field name.
        field: &'static str,
    },
    /// Field failed a format rule.
    #[error("{field} is invalid: {reason}; hint: normalize the cassette input before recording")]
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Human-readable validation failure.
        reason: String,
    },
    /// Two interactions had the same matcher key.
    #[error(
        "cassette collision for {method} {path} body={body_fingerprint}; hint: split scenarios or change the recorded request body"
    )]
    MatcherCollision {
        /// HTTP method.
        method: String,
        /// Request path.
        path: String,
        /// BLAKE3 fingerprint of the request body.
        body_fingerprint: String,
    },
    /// No interaction matched the requested key.
    #[error(
        "no cassette interaction matched {method} {path} body={body_fingerprint}; hint: record a cassette for this request"
    )]
    NoMatch {
        /// HTTP method.
        method: String,
        /// Request path.
        path: String,
        /// BLAKE3 fingerprint of the request body.
        body_fingerprint: String,
    },
    /// A cassette still contains text that looks like personal data.
    #[error(
        "cassette contains {finding_count} unscrubbed personal-data pattern(s); hint: add scrub rules before committing the cassette"
    )]
    UnscrubbedPii {
        /// Number of matched high-risk patterns.
        finding_count: usize,
    },
    /// JSON serialization or parsing failed.
    #[error("cassette JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Cassette-backed implementation of [`GatewayAdapter`].
///
/// The adapter turns gateway operations into deterministic internal requests,
/// matches those requests against committed cassettes, and normalizes replayed
/// cassette bodies into [`GatewayReceipt`] or [`GatewayError`] values.
#[derive(Debug)]
pub struct MockGatewayAdapter {
    cassettes: Vec<Cassette>,
}

impl MockGatewayAdapter {
    /// Builds a mock gateway from deterministic cassettes.
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::MatcherCollision`] when two interactions across
    /// the provided cassettes share method, path, and body fingerprint.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, MockGatewayAdapter, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/empty",
    ///     "Synthetic empty cassette",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let cassette = CassetteRecorder::new(scenario).finish();
    /// let adapter = MockGatewayAdapter::new([cassette])?;
    ///
    /// assert_eq!(adapter.cassette_count(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(cassettes: impl IntoIterator<Item = Cassette>) -> Result<Self, CassetteError> {
        let cassettes: Vec<Cassette> = cassettes.into_iter().collect();
        let mut keys = BTreeSet::new();
        for cassette in &cassettes {
            CassetteMatcher::new(cassette)?;
            for interaction in &cassette.interactions {
                let key = interaction.request.match_key();
                if keys.contains(&key) {
                    return Err(CassetteError::MatcherCollision {
                        method: key.method,
                        path: key.path,
                        body_fingerprint: key.body_fingerprint,
                    });
                }
                keys.insert(key);
            }
        }
        Ok(Self { cassettes })
    }

    /// Returns the number of cassettes loaded by this adapter.
    #[must_use]
    pub fn cassette_count(&self) -> usize {
        self.cassettes.len()
    }

    fn recorded_submit_request(request: &SubmitRequest) -> Result<RecordedRequest, GatewayError> {
        let envelope = MockRequestEnvelope {
            operation: GatewayOperation::Submit,
            context: &request.context,
            route: Some(&request.route),
            submission_id: None,
            document_id: Some(request.document.id.as_str()),
            document_number: Some(request.document.document_number.as_str()),
            document_fingerprint: Some(document_fingerprint(
                &request.document,
                GatewayOperation::Submit,
            )?),
            reason: None,
        };
        let body = serialize_request_envelope(&envelope)?;
        recorded_gateway_request(GatewayOperation::Submit, "POST", MOCK_SUBMIT_PATH, body)
    }

    fn recorded_poll_request(request: &PollRequest) -> Result<RecordedRequest, GatewayError> {
        let envelope = MockRequestEnvelope {
            operation: GatewayOperation::Poll,
            context: &request.context,
            route: None,
            submission_id: Some(&request.submission_id),
            document_id: None,
            document_number: None,
            document_fingerprint: None,
            reason: None,
        };
        let body = serialize_request_envelope(&envelope)?;
        recorded_gateway_request(GatewayOperation::Poll, "GET", MOCK_POLL_PATH, body)
    }

    fn recorded_cancel_request(request: &CancelRequest) -> Result<RecordedRequest, GatewayError> {
        let envelope = MockRequestEnvelope {
            operation: GatewayOperation::Cancel,
            context: &request.context,
            route: None,
            submission_id: Some(&request.submission_id),
            document_id: None,
            document_number: None,
            document_fingerprint: None,
            reason: Some(&request.reason),
        };
        let body = serialize_request_envelope(&envelope)?;
        recorded_gateway_request(GatewayOperation::Cancel, "POST", MOCK_CANCEL_PATH, body)
    }

    fn recorded_correct_request(request: &CorrectRequest) -> Result<RecordedRequest, GatewayError> {
        let envelope = MockRequestEnvelope {
            operation: GatewayOperation::Correct,
            context: &request.context,
            route: None,
            submission_id: Some(&request.submission_id),
            document_id: Some(request.corrected_document.id.as_str()),
            document_number: Some(request.corrected_document.document_number.as_str()),
            document_fingerprint: Some(document_fingerprint(
                &request.corrected_document,
                GatewayOperation::Correct,
            )?),
            reason: Some(&request.reason),
        };
        let body = serialize_request_envelope(&envelope)?;
        recorded_gateway_request(GatewayOperation::Correct, "POST", MOCK_CORRECT_PATH, body)
    }

    fn replay(
        &self,
        operation: GatewayOperation,
        request: &RecordedRequest,
        context: GatewayContext,
    ) -> Result<GatewayReceipt, GatewayError> {
        tracing::debug!(
            operation = operation.as_str(),
            tenant_id = context.tenant_id.as_str(),
            trace_id = context.trace_id.as_str(),
            gateway_attempt_id = context.gateway_attempt_id.as_str(),
            "replaying mock gateway cassette"
        );
        let response = self.match_response(request).map_err(|error| {
            cassette_error_to_gateway_error(
                operation,
                &error,
                "record or select a matching cassette",
            )
        })?;
        if (200..=299).contains(&response.status) {
            receipt_from_response(operation, context, response)
        } else {
            Err(error_from_response(operation, response))
        }
    }

    fn match_response<'a>(
        &'a self,
        request: &RecordedRequest,
    ) -> Result<&'a RecordedResponse, CassetteError> {
        let mut miss = None;
        for cassette in &self.cassettes {
            let matcher = CassetteMatcher::new(cassette)?;
            match matcher.match_request(request) {
                Ok(response) => return Ok(response),
                Err(error @ CassetteError::NoMatch { .. }) => {
                    miss = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(miss.unwrap_or_else(|| CassetteError::NoMatch {
            method: request.method.clone(),
            path: request.path.clone(),
            body_fingerprint: body_fingerprint(request.body.as_bytes()),
        }))
    }
}

impl GatewayAdapter for MockGatewayAdapter {
    fn submit(&self, request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let recorded = Self::recorded_submit_request(&request);
        let context = request.context;
        let result =
            recorded.and_then(|recorded| self.replay(GatewayOperation::Submit, &recorded, context));
        Box::pin(std::future::ready(result))
    }

    fn poll(&self, request: PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let recorded = Self::recorded_poll_request(&request);
        let context = request.context;
        let result =
            recorded.and_then(|recorded| self.replay(GatewayOperation::Poll, &recorded, context));
        Box::pin(std::future::ready(result))
    }

    fn cancel(&self, request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let recorded = Self::recorded_cancel_request(&request);
        let context = request.context;
        let result =
            recorded.and_then(|recorded| self.replay(GatewayOperation::Cancel, &recorded, context));
        Box::pin(std::future::ready(result))
    }

    fn correct(&self, request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let recorded = Self::recorded_correct_request(&request);
        let context = request.context;
        let result = recorded
            .and_then(|recorded| self.replay(GatewayOperation::Correct, &recorded, context));
        Box::pin(std::future::ready(result))
    }
}

#[derive(Serialize)]
struct MockRequestEnvelope<'a> {
    operation: GatewayOperation,
    context: &'a GatewayContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<&'a GatewayRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_id: Option<&'a GatewaySubmissionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_number: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
}

#[derive(Deserialize)]
struct MockGatewayReceiptBody {
    submission_id: String,
    status: GatewayStatus,
    received_at: String,
    #[serde(default)]
    gateway_reference: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Deserialize)]
struct MockGatewayErrorBody {
    kind: GatewayErrorKind,
    message: String,
    remediation: String,
    #[serde(default)]
    gateway_code: Option<String>,
    #[serde(default)]
    submission_id: Option<String>,
    #[serde(default)]
    retry_after_seconds: Option<u64>,
}

fn serialize_request_envelope(envelope: &MockRequestEnvelope<'_>) -> Result<String, GatewayError> {
    serde_json::to_string(&envelope).map_err(|error| {
        GatewayError::new(
            GatewayErrorKind::InvalidRequest,
            envelope.operation,
            format!("mock gateway request serialization failed: {error}"),
            "ensure gateway request fields are serializable before replay",
        )
    })
}

fn document_fingerprint(
    document: &impl Serialize,
    operation: GatewayOperation,
) -> Result<String, GatewayError> {
    let value = serde_json::to_value(document).map_err(|error| {
        GatewayError::new(
            GatewayErrorKind::InvalidRequest,
            operation,
            format!("mock gateway document serialization failed: {error}"),
            "ensure the document is serializable before cassette replay",
        )
    })?;
    let canonical = invoicekit_canonical::canonicalize_value(&value).map_err(|error| {
        GatewayError::new(
            GatewayErrorKind::InvalidRequest,
            operation,
            format!("mock gateway document canonicalization failed: {error}"),
            "ensure the document is I-JSON compatible before cassette replay",
        )
    })?;
    Ok(body_fingerprint(canonical.as_bytes()))
}

fn recorded_gateway_request(
    operation: GatewayOperation,
    method: &str,
    path: &str,
    body: String,
) -> Result<RecordedRequest, GatewayError> {
    RecordedRequest::new(method, path, BTreeMap::new(), body).map_err(|error| {
        cassette_error_to_gateway_error(
            operation,
            &error,
            "fix the mock gateway operation-to-cassette request mapping",
        )
    })
}

fn receipt_from_response(
    operation: GatewayOperation,
    context: GatewayContext,
    response: &RecordedResponse,
) -> Result<GatewayReceipt, GatewayError> {
    let body: MockGatewayReceiptBody = serde_json::from_str(&response.body).map_err(|error| {
        malformed_receipt_error(operation, format!("invalid receipt JSON: {error}"))
    })?;
    let submission_id = GatewaySubmissionId::new(body.submission_id).map_err(|error| {
        malformed_receipt_error(operation, format!("invalid receipt submission_id: {error}"))
    })?;
    let mut receipt = GatewayReceipt::new(
        operation,
        context,
        submission_id,
        body.status,
        body.received_at,
    )
    .map_err(|error| malformed_receipt_error(operation, error.to_string()))?;
    receipt.gateway_reference = body.gateway_reference;
    receipt.raw_receipt_hash = Some(body_fingerprint(response.body.as_bytes()));
    receipt.detail = body.detail;
    Ok(receipt)
}

fn error_from_response(operation: GatewayOperation, response: &RecordedResponse) -> GatewayError {
    let body: MockGatewayErrorBody = match serde_json::from_str(&response.body) {
        Ok(body) => body,
        Err(error) => {
            if is_gateway_maintenance_page(response) {
                return GatewayError::new(
                    GatewayErrorKind::GatewayMaintenance,
                    operation,
                    format!(
                        "gateway returned an HTML maintenance page with HTTP status {}",
                        response.status
                    ),
                    "retry after the maintenance window or replay a normalized gateway error cassette",
                )
                .with_gateway_code(format!("HTTP_{}", response.status));
            }
            return malformed_receipt_error(operation, format!("invalid error JSON: {error}"));
        }
    };
    let mut error = GatewayError::new(body.kind, operation, body.message, body.remediation);
    if let Some(code) = body.gateway_code {
        error = error.with_gateway_code(code);
    }
    if let Some(submission_id) = body.submission_id {
        let submission_id = match GatewaySubmissionId::new(submission_id) {
            Ok(submission_id) => submission_id,
            Err(error) => {
                return malformed_receipt_error(
                    operation,
                    format!("invalid error submission_id: {error}"),
                );
            }
        };
        error = error.with_submission_id(submission_id);
    }
    if let Some(seconds) = body.retry_after_seconds {
        error = error.with_retry_after_seconds(seconds);
    }
    error
}

fn is_gateway_maintenance_page(response: &RecordedResponse) -> bool {
    if !(500..=599).contains(&response.status) {
        return false;
    }

    let content_type = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.to_ascii_lowercase());
    let body = response.body.to_ascii_lowercase();
    let looks_like_html = content_type
        .as_deref()
        .is_some_and(|value| value.contains("text/html"))
        || body.contains("<!doctype html")
        || body.contains("<html");
    let looks_like_maintenance = body.contains("maintenance")
        || body.contains("temporarily unavailable")
        || body.contains("service unavailable");

    looks_like_html && looks_like_maintenance
}

fn malformed_receipt_error(operation: GatewayOperation, message: String) -> GatewayError {
    GatewayError::new(
        GatewayErrorKind::MalformedReceipt,
        operation,
        message,
        "fix or re-record the malformed mock gateway cassette response",
    )
}

fn cassette_error_to_gateway_error(
    operation: GatewayOperation,
    error: &CassetteError,
    remediation: &'static str,
) -> GatewayError {
    let kind = match error {
        CassetteError::NoMatch { .. } => GatewayErrorKind::NotFound,
        CassetteError::MatcherCollision { .. } => GatewayErrorKind::UnexpectedResponse,
        CassetteError::MissingRequiredField { .. }
        | CassetteError::InvalidField { .. }
        | CassetteError::UnscrubbedPii { .. }
        | CassetteError::Json(_) => GatewayErrorKind::InvalidRequest,
    };
    GatewayError::new(kind, operation, error.to_string(), remediation)
}

/// Stable scenario identifiers that every gateway contract suite run must cover.
pub const GATEWAY_CONTRACT_SCENARIO_IDS: &[&str] = &[
    "idempotent-replay",
    "duplicate-submission",
    "timeout",
    "malformed-receipt",
    "auth-failure",
    "certificate-rejection",
    "rate-limit",
    "delayed-async-receipt",
    "unknown-response-field",
    "gateway-maintenance-page",
    "partner-error-translation",
];

/// Errors raised while building reusable gateway contract scenarios.
#[derive(Debug, Error)]
pub enum GatewayContractError {
    /// Cassette metadata or interaction material was invalid.
    #[error("gateway contract cassette is invalid: {0}")]
    Cassette(#[from] CassetteError),
    /// Synthetic contract invoice IR was invalid.
    #[error("gateway contract invoice is invalid: {0}")]
    Ir(#[from] invoicekit_ir::IrError),
    /// Reconciliation request material was invalid.
    #[error("gateway contract request is invalid: {0}")]
    Reconcile(#[from] ReconcileError),
    /// Mock gateway request recording failed.
    #[error("gateway contract adapter recording failed: {0}")]
    Gateway(#[from] GatewayError),
}

/// Mismatch between a gateway adapter result and the contract expectation.
#[derive(Debug, Error)]
#[error("gateway contract scenario {scenario_id} failed: {message}")]
pub struct GatewayContractMismatch {
    scenario_id: &'static str,
    message: String,
}

impl GatewayContractMismatch {
    /// Returns the scenario identifier that failed.
    #[must_use]
    pub const fn scenario_id(&self) -> &'static str {
        self.scenario_id
    }

    /// Returns the human-readable mismatch description.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Expected normalized adapter result for one gateway contract operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayContractExpectation {
    /// Adapter should return a normalized receipt.
    Receipt {
        /// Expected receipt status.
        status: GatewayStatus,
        /// Expected gateway reference.
        gateway_reference: &'static str,
    },
    /// Adapter should return a normalized gateway error.
    Error {
        /// Expected error kind.
        kind: GatewayErrorKind,
        /// Expected gateway-specific error code.
        gateway_code: Option<&'static str>,
        /// Expected retry delay in seconds.
        retry_after_seconds: Option<u64>,
        /// Text that must be present in the normalized error message.
        message_contains: &'static str,
    },
}

/// One reusable, cassette-backed `GatewayAdapter` contract scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayContractScenario {
    id: &'static str,
    title: &'static str,
    submit_case: &'static str,
    submit_response: RecordedResponse,
    submit_expectation: GatewayContractExpectation,
    poll_submission_id: Option<&'static str>,
    poll_response: Option<RecordedResponse>,
    poll_expectation: Option<GatewayContractExpectation>,
}

impl GatewayContractScenario {
    /// Returns the stable scenario identifier.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// Returns the human-readable scenario title.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        self.title
    }

    /// Builds the submit request that adapter implementations must handle.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayContractError`] if synthetic contract fixture data no
    /// longer builds a valid invoice or gateway request.
    pub fn submit_request(&self) -> Result<SubmitRequest, GatewayContractError> {
        contract_submit_request(self.submit_case)
    }

    /// Returns the expected submit result.
    #[must_use]
    pub const fn submit_expectation(&self) -> GatewayContractExpectation {
        self.submit_expectation
    }

    /// Builds the optional poll request for asynchronous scenarios.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayContractError`] if the synthetic poll context or
    /// submission identifier is invalid.
    pub fn poll_request(&self) -> Result<Option<PollRequest>, GatewayContractError> {
        let Some(submission_id) = self.poll_submission_id else {
            return Ok(None);
        };
        Ok(Some(PollRequest::new(
            contract_gateway_context(self.submit_case)?,
            GatewaySubmissionId::new(submission_id)?,
        )))
    }

    /// Returns the optional expected poll result for asynchronous scenarios.
    #[must_use]
    pub const fn poll_expectation(&self) -> Option<GatewayContractExpectation> {
        self.poll_expectation
    }

    /// Builds a deterministic mock cassette for this contract scenario.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayContractError`] if the generated scenario metadata,
    /// recorded request, or recorded response is invalid.
    pub fn cassette(&self) -> Result<Cassette, GatewayContractError> {
        let mut metadata = ScenarioMetadata::new(
            format!("mock/contract/{}", self.id),
            self.title,
            "DE",
            "mock",
            ScenarioSource::Synthetic,
            "none",
        )?;
        metadata.description = Some(format!(
            "Synthetic GatewayAdapter contract scenario: {}.",
            self.id
        ));
        metadata.tags = vec!["gateway-contract".to_owned(), self.id.to_owned()];

        let submit_request = self.submit_request()?;
        let mut recorder = CassetteRecorder::new(metadata);
        recorder.record(
            MockGatewayAdapter::recorded_submit_request(&submit_request)?,
            self.submit_response.clone(),
        );

        if let Some(poll_request) = self.poll_request()? {
            let poll_response =
                self.poll_response
                    .clone()
                    .ok_or(GatewayContractError::Cassette(
                        CassetteError::MissingRequiredField {
                            field: "poll_response",
                        },
                    ))?;
            recorder.record(
                MockGatewayAdapter::recorded_poll_request(&poll_request)?,
                poll_response,
            );
        }

        Ok(recorder.finish())
    }

    /// Verifies a submit operation result against this scenario.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayContractMismatch`] when the adapter result does not
    /// match the scenario expectation.
    pub fn verify_submit_result(
        &self,
        result: &Result<GatewayReceipt, GatewayError>,
    ) -> Result<(), GatewayContractMismatch> {
        verify_contract_result(
            self.id,
            GatewayOperation::Submit,
            self.submit_expectation,
            result,
        )
    }

    /// Verifies a poll operation result against this scenario.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayContractMismatch`] when the scenario has no poll
    /// expectation or when the adapter result does not match it.
    pub fn verify_poll_result(
        &self,
        result: &Result<GatewayReceipt, GatewayError>,
    ) -> Result<(), GatewayContractMismatch> {
        let expectation = self.poll_expectation.ok_or_else(|| {
            contract_mismatch(self.id, "scenario does not define a poll expectation")
        })?;
        verify_contract_result(self.id, GatewayOperation::Poll, expectation, result)
    }
}

/// Builds every reusable gateway contract scenario.
///
/// # Examples
///
/// ```
/// use invoicekit_transmit_mock::{
///     gateway_contract_scenarios, GATEWAY_CONTRACT_SCENARIO_IDS,
/// };
///
/// let scenarios = gateway_contract_scenarios().unwrap();
/// let ids: Vec<&str> = scenarios.iter().map(|scenario| scenario.id()).collect();
/// assert_eq!(ids, GATEWAY_CONTRACT_SCENARIO_IDS);
/// ```
///
/// # Errors
///
/// Returns [`GatewayContractError`] if any synthetic cassette response is no
/// longer valid according to the cassette schema.
pub fn gateway_contract_scenarios() -> Result<Vec<GatewayContractScenario>, GatewayContractError> {
    Ok(vec![
        contract_idempotent_replay()?,
        contract_duplicate_submission()?,
        contract_timeout()?,
        contract_malformed_receipt()?,
        contract_auth_failure()?,
        contract_certificate_rejection()?,
        contract_rate_limit()?,
        contract_delayed_async_receipt()?,
        contract_unknown_response_field()?,
        contract_gateway_maintenance_page()?,
        contract_partner_error_translation()?,
    ])
}

/// Builds deterministic mock cassettes for every gateway contract scenario.
///
/// # Examples
///
/// ```
/// use invoicekit_transmit_mock::{
///     gateway_contract_cassettes, GATEWAY_CONTRACT_SCENARIO_IDS,
/// };
///
/// let cassettes = gateway_contract_cassettes().unwrap();
/// assert_eq!(cassettes.len(), GATEWAY_CONTRACT_SCENARIO_IDS.len());
/// ```
///
/// # Errors
///
/// Returns [`GatewayContractError`] if a generated scenario or cassette is
/// invalid.
pub fn gateway_contract_cassettes() -> Result<Vec<Cassette>, GatewayContractError> {
    gateway_contract_scenarios()?
        .iter()
        .map(GatewayContractScenario::cassette)
        .collect()
}

fn verify_contract_result(
    scenario_id: &'static str,
    operation: GatewayOperation,
    expectation: GatewayContractExpectation,
    result: &Result<GatewayReceipt, GatewayError>,
) -> Result<(), GatewayContractMismatch> {
    match (expectation, result) {
        (
            GatewayContractExpectation::Receipt {
                status,
                gateway_reference,
            },
            Ok(receipt),
        ) => verify_contract_receipt(scenario_id, operation, status, gateway_reference, receipt),
        (
            GatewayContractExpectation::Error {
                kind,
                gateway_code,
                retry_after_seconds,
                message_contains,
            },
            Err(error),
        ) => verify_contract_error(
            scenario_id,
            operation,
            kind,
            gateway_code,
            retry_after_seconds,
            message_contains,
            error,
        ),
        (GatewayContractExpectation::Receipt { .. }, Err(error)) => Err(contract_mismatch(
            scenario_id,
            format!("expected receipt, got error kind {}", error.kind),
        )),
        (GatewayContractExpectation::Error { .. }, Ok(receipt)) => Err(contract_mismatch(
            scenario_id,
            format!("expected error, got receipt status {:?}", receipt.status),
        )),
    }
}

fn verify_contract_receipt(
    scenario_id: &'static str,
    operation: GatewayOperation,
    status: GatewayStatus,
    gateway_reference: &str,
    receipt: &GatewayReceipt,
) -> Result<(), GatewayContractMismatch> {
    if receipt.operation != operation {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected operation {operation}, got {}",
                receipt.operation.as_str()
            ),
        ));
    }
    if receipt.status != status {
        return Err(contract_mismatch(
            scenario_id,
            format!("expected status {status:?}, got {:?}", receipt.status),
        ));
    }
    if receipt.gateway_reference.as_deref() != Some(gateway_reference) {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected gateway_reference {gateway_reference}, got {:?}",
                receipt.gateway_reference
            ),
        ));
    }
    Ok(())
}

fn verify_contract_error(
    scenario_id: &'static str,
    operation: GatewayOperation,
    kind: GatewayErrorKind,
    gateway_code: Option<&str>,
    retry_after_seconds: Option<u64>,
    message_contains: &str,
    error: &GatewayError,
) -> Result<(), GatewayContractMismatch> {
    if error.operation != operation {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected operation {operation}, got {}",
                error.operation.as_str()
            ),
        ));
    }
    if error.kind != kind {
        return Err(contract_mismatch(
            scenario_id,
            format!("expected error kind {kind}, got {}", error.kind),
        ));
    }
    if error.gateway_code.as_deref() != gateway_code {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected gateway_code {gateway_code:?}, got {:?}",
                error.gateway_code
            ),
        ));
    }
    if error.retry_after_seconds != retry_after_seconds {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected retry_after_seconds {retry_after_seconds:?}, got {:?}",
                error.retry_after_seconds
            ),
        ));
    }
    if !error.message.contains(message_contains) {
        return Err(contract_mismatch(
            scenario_id,
            format!(
                "expected message containing {message_contains:?}, got {:?}",
                error.message
            ),
        ));
    }
    Ok(())
}

fn contract_mismatch(
    scenario_id: &'static str,
    message: impl Into<String>,
) -> GatewayContractMismatch {
    GatewayContractMismatch {
        scenario_id,
        message: message.into(),
    }
}

fn contract_idempotent_replay() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "idempotent-replay",
        title: "Gateway contract idempotent replay",
        submit_case: "contract_idempotent",
        submit_response: recorded_response(
            202,
            r#"{"submission_id":"mock_sub_contract_idempotent","status":"accepted","gateway_reference":"MOCK-CONTRACT-IDEMPOTENT","received_at":"2026-05-27T01:00:00Z","detail":"Replayable accepted receipt."}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Receipt {
            status: GatewayStatus::Accepted,
            gateway_reference: "MOCK-CONTRACT-IDEMPOTENT",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_duplicate_submission() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "duplicate-submission",
        title: "Gateway contract duplicate submission",
        submit_case: "contract_duplicate",
        submit_response: recorded_response(
            409,
            r#"{"kind":"duplicate_submission","message":"gateway already has this invoice","remediation":"poll the existing submission instead of submitting again","gateway_code":"DUPLICATE_SUBMISSION","submission_id":"mock_sub_contract_duplicate"}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::DuplicateSubmission,
            gateway_code: Some("DUPLICATE_SUBMISSION"),
            retry_after_seconds: None,
            message_contains: "already has",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_timeout() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "timeout",
        title: "Gateway contract timeout",
        submit_case: "contract_timeout",
        submit_response: recorded_response(
            504,
            r#"{"kind":"timeout","message":"gateway timed out before a final receipt","remediation":"retry with the same idempotency key or poll the submission","gateway_code":"GATEWAY_TIMEOUT","retry_after_seconds":30}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::Timeout,
            gateway_code: Some("GATEWAY_TIMEOUT"),
            retry_after_seconds: Some(30),
            message_contains: "timed out",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_malformed_receipt() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "malformed-receipt",
        title: "Gateway contract malformed receipt",
        submit_case: "contract_malformed",
        submit_response: recorded_response(202, "<not-json>")?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::MalformedReceipt,
            gateway_code: None,
            retry_after_seconds: None,
            message_contains: "invalid receipt JSON",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_auth_failure() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "auth-failure",
        title: "Gateway contract auth failure",
        submit_case: "contract_auth",
        submit_response: recorded_response(
            401,
            r#"{"kind":"auth_failure","message":"gateway rejected the credentials","remediation":"refresh credentials before replaying the cassette","gateway_code":"AUTH_FAILED"}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::AuthFailure,
            gateway_code: Some("AUTH_FAILED"),
            retry_after_seconds: None,
            message_contains: "credentials",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_certificate_rejection() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "certificate-rejection",
        title: "Gateway contract certificate rejection",
        submit_case: "contract_certificate",
        submit_response: recorded_response(
            495,
            r#"{"kind":"certificate_rejected","message":"gateway rejected the signing certificate chain","remediation":"renew or replace the certificate before retrying","gateway_code":"CERT_CHAIN_REJECTED"}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::CertificateRejected,
            gateway_code: Some("CERT_CHAIN_REJECTED"),
            retry_after_seconds: None,
            message_contains: "certificate",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_rate_limit() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "rate-limit",
        title: "Gateway contract rate limit",
        submit_case: "contract_rate_limit",
        submit_response: recorded_response(
            429,
            r#"{"kind":"rate_limited","message":"gateway rate limit exceeded","remediation":"retry after the supplied backoff window","gateway_code":"RATE_LIMIT","retry_after_seconds":60}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::RateLimited,
            gateway_code: Some("RATE_LIMIT"),
            retry_after_seconds: Some(60),
            message_contains: "rate limit",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_delayed_async_receipt() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "delayed-async-receipt",
        title: "Gateway contract delayed async receipt",
        submit_case: "contract_delayed_async",
        submit_response: recorded_response(
            202,
            r#"{"submission_id":"mock_sub_contract_delayed","status":"pending","gateway_reference":"MOCK-CONTRACT-PENDING","received_at":"2026-05-27T01:07:00Z","detail":"Gateway accepted the request for asynchronous processing."}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Receipt {
            status: GatewayStatus::Pending,
            gateway_reference: "MOCK-CONTRACT-PENDING",
        },
        poll_submission_id: Some("mock_sub_contract_delayed"),
        poll_response: Some(recorded_response(
            200,
            r#"{"submission_id":"mock_sub_contract_delayed","status":"accepted","gateway_reference":"MOCK-CONTRACT-DELAYED-RECEIPT","received_at":"2026-05-27T01:09:00Z","detail":"Gateway completed asynchronous processing."}"#,
        )?),
        poll_expectation: Some(GatewayContractExpectation::Receipt {
            status: GatewayStatus::Accepted,
            gateway_reference: "MOCK-CONTRACT-DELAYED-RECEIPT",
        }),
    })
}

fn contract_unknown_response_field() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "unknown-response-field",
        title: "Gateway contract unknown response field",
        submit_case: "contract_unknown_field",
        submit_response: recorded_response(
            202,
            r#"{"submission_id":"mock_sub_contract_unknown_field","status":"accepted","gateway_reference":"MOCK-CONTRACT-UNKNOWN-FIELD","received_at":"2026-05-27T01:08:00Z","detail":"Unknown response members are ignored.","future_gateway_member":"kept out of the normalized receipt"}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Receipt {
            status: GatewayStatus::Accepted,
            gateway_reference: "MOCK-CONTRACT-UNKNOWN-FIELD",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_gateway_maintenance_page() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "gateway-maintenance-page",
        title: "Gateway contract maintenance page",
        submit_case: "contract_maintenance",
        submit_response: html_response(
            503,
            "<!doctype html><html><title>Maintenance</title><body>Service unavailable for maintenance.</body></html>",
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::GatewayMaintenance,
            gateway_code: Some("HTTP_503"),
            retry_after_seconds: None,
            message_contains: "maintenance page",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn contract_partner_error_translation() -> Result<GatewayContractScenario, GatewayContractError> {
    Ok(GatewayContractScenario {
        id: "partner-error-translation",
        title: "Gateway contract partner error translation",
        submit_case: "contract_partner_error",
        submit_response: recorded_response(
            502,
            r#"{"kind":"partner_error","message":"partner access point returned an opaque failure","remediation":"inspect the partner incident reference and retry after recovery","gateway_code":"PARTNER_PEP_503","retry_after_seconds":120}"#,
        )?,
        submit_expectation: GatewayContractExpectation::Error {
            kind: GatewayErrorKind::PartnerError,
            gateway_code: Some("PARTNER_PEP_503"),
            retry_after_seconds: Some(120),
            message_contains: "partner",
        },
        poll_submission_id: None,
        poll_response: None,
        poll_expectation: None,
    })
}

fn recorded_response(status: u16, body: &'static str) -> Result<RecordedResponse, CassetteError> {
    RecordedResponse::new(status, BTreeMap::new(), body)
}

fn html_response(status: u16, body: &'static str) -> Result<RecordedResponse, CassetteError> {
    let mut headers = BTreeMap::new();
    headers.insert(
        "content-type".to_owned(),
        "text/html; charset=utf-8".to_owned(),
    );
    RecordedResponse::new(status, headers, body)
}

fn contract_submit_request(case: &str) -> Result<SubmitRequest, GatewayContractError> {
    Ok(SubmitRequest::new(
        contract_gateway_context(case)?,
        GatewayRoute::new("mock", "mock-profile", Some("DE"))?,
        contract_synthetic_document(case)?,
    )?)
}

fn contract_gateway_context(case: &str) -> Result<GatewayContext, ReconcileError> {
    Ok(GatewayContext::new(
        TenantId::new("tenant_mock")?,
        TraceId::new(format!("trace_mock_{case}"))?,
        IdempotencyKey::new(format!("idem_mock_{case}"))?,
        GatewayAttemptId::new(format!("attempt_mock_{case}"))?,
    ))
}

fn contract_synthetic_document(case: &str) -> Result<CommercialDocument, invoicekit_ir::IrError> {
    CommercialDocument::try_from_value(serde_json::json!({
        "schema_version": "1.0",
        "id": format!("doc_mock_{case}"),
        "document_type": "invoice",
        "issue_date": "2026-05-27",
        "document_number": format!("INV-MOCK-{}", case.to_ascii_uppercase()),
        "currency": "EUR",
        "supplier": contract_party_json("supplier_mock", "Mock Supplier GmbH", "DE"),
        "customer": contract_party_json("customer_mock", "Mock Buyer SAS", "FR"),
        "lines": [{
            "id": "1",
            "description": "Mock gateway fixture",
            "quantity": "1",
            "unit_price": "100.00",
            "line_extension_amount": "100.00"
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
            "tenant_id": "tenant_mock",
            "trace_id": format!("trace_mock_{case}"),
            "source_system": "transmit-mock-contract"
        }
    }))
}

fn contract_party_json(id: &str, name: &str, country: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "tax_ids": [{
            "scheme": "test",
            "value": format!("{country}-MOCK-TAX")
        }],
        "address": {
            "lines": ["Mock Street 1"],
            "city": "Mock City",
            "postal_code": "10000",
            "country": country
        }
    })
}

/// Metadata stored in each cassette directory as `scenario.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScenarioMetadata {
    /// Scenario schema version.
    pub schema_version: String,
    /// Stable scenario identifier, for example `pl/ksef/accepted`.
    pub scenario_id: String,
    /// Human-readable title.
    pub title: String,
    /// ISO 3166-1 alpha-2 country code.
    pub country: String,
    /// Transmission route or gateway family, for example `peppol`.
    pub route: String,
    /// Recording source.
    pub source: ScenarioSource,
    /// Scrubber profile applied before commit.
    pub scrubber_profile: String,
    /// Optional longer description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Searchable scenario labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl ScenarioMetadata {
    /// Builds validated scenario metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScenarioMetadata, ScenarioSource};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "de/ksef/success",
    ///     "KSeF accepted invoice",
    ///     "DE",
    ///     "ksef",
    ///     ScenarioSource::OfficialSandbox,
    ///     "default-de",
    /// )?;
    ///
    /// assert_eq!(scenario.schema_version, "1.0");
    /// assert_eq!(scenario.country, "DE");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when identifiers are blank or when
    /// `country` is not an ISO 3166-1 alpha-2 uppercase code.
    pub fn new(
        scenario_id: impl Into<String>,
        title: impl Into<String>,
        country: impl Into<String>,
        route: impl Into<String>,
        source: ScenarioSource,
        scrubber_profile: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        let scenario_id = non_empty(scenario_id.into(), "scenario_id")?;
        let title = non_empty(title.into(), "title")?;
        let country = country.into();
        validate_country(&country)?;
        let route = non_empty(route.into(), "route")?;
        let scrubber_profile = non_empty(scrubber_profile.into(), "scrubber_profile")?;
        Ok(Self {
            schema_version: "1.0".to_owned(),
            scenario_id,
            title,
            country,
            route,
            source,
            scrubber_profile,
            description: None,
            tags: Vec::new(),
        })
    }
}

/// Source used to create a cassette.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioSource {
    /// Official regulator sandbox.
    OfficialSandbox,
    /// Partner access point or partner gateway sandbox.
    PartnerSandbox,
    /// Fully synthetic scenario.
    Synthetic,
}

/// A full deterministic `.vcr` cassette.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cassette {
    /// Cassette format version.
    pub schema_version: String,
    /// Scenario metadata.
    pub scenario: ScenarioMetadata,
    /// Recorded request/response interactions.
    pub interactions: Vec<CassetteInteraction>,
}

impl Cassette {
    /// Serializes this cassette to byte-stable JSON `.vcr` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert!(cassette.to_vcr_bytes()?.ends_with(b"\n"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::Json`] if serialization fails.
    pub fn to_vcr_bytes(&self) -> Result<Vec<u8>, CassetteError> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Parses a cassette from JSON `.vcr` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     Cassette, CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let original = CassetteRecorder::new(scenario).finish();
    /// let parsed = Cassette::from_vcr_bytes(&original.to_vcr_bytes()?)?;
    ///
    /// assert_eq!(parsed, original);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::Json`] if parsing fails.
    pub fn from_vcr_bytes(bytes: &[u8]) -> Result<Self, CassetteError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// One request/response pair inside a cassette.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CassetteInteraction {
    /// Recorded request.
    pub request: RecordedRequest,
    /// Recorded response.
    pub response: RecordedResponse,
}

/// Recorded gateway request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedRequest {
    /// Uppercase HTTP-like method or AS4 operation name.
    pub method: String,
    /// Request path, starting with `/`.
    pub path: String,
    /// Deterministically ordered request headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Request body as UTF-8 text.
    pub body: String,
    /// BLAKE3 fingerprint of `body`.
    pub body_fingerprint: String,
}

impl RecordedRequest {
    /// Builds a recorded request and computes its body fingerprint.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{body_fingerprint, RecordedRequest};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let request = RecordedRequest::new(
    ///     "post",
    ///     "/sandbox/invoices",
    ///     BTreeMap::new(),
    ///     "<Invoice />",
    /// )?;
    ///
    /// assert_eq!(request.method, "POST");
    /// assert_eq!(
    ///     request.body_fingerprint,
    ///     body_fingerprint(b"<Invoice />")
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `method` is blank or `path` does not
    /// start with `/`.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        headers: BTreeMap<String, String>,
        body: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        let method = non_empty(method.into(), "method")?.to_ascii_uppercase();
        let path = path.into();
        validate_path(&path)?;
        let body = body.into();
        let body_fingerprint = body_fingerprint(body.as_bytes());
        Ok(Self {
            method,
            path,
            headers,
            body,
            body_fingerprint,
        })
    }

    fn match_key(&self) -> MatchKey {
        MatchKey {
            method: self.method.clone(),
            path: self.path.clone(),
            body_fingerprint: body_fingerprint(self.body.as_bytes()),
        }
    }
}

/// Recorded gateway response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedResponse {
    /// HTTP-like status code.
    pub status: u16,
    /// Deterministically ordered response headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Response body as UTF-8 text.
    pub body: String,
}

impl RecordedResponse {
    /// Builds a recorded response.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::RecordedResponse;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let response = RecordedResponse::new(202, BTreeMap::new(), r#"{"ok":true}"#)?;
    ///
    /// assert_eq!(response.status, 202);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `status` is outside `100..=599`.
    pub fn new(
        status: u16,
        headers: BTreeMap<String, String>,
        body: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        if !(100..=599).contains(&status) {
            return Err(CassetteError::InvalidField {
                field: "status",
                reason: "expected HTTP status code in 100..=599".to_owned(),
            });
        }
        Ok(Self {
            status,
            headers,
            body: body.into(),
        })
    }
}

/// Deterministic recorder for `.vcr` cassettes.
pub struct CassetteRecorder {
    scenario: ScenarioMetadata,
    interactions: Vec<CassetteInteraction>,
}

impl CassetteRecorder {
    /// Starts recording a scenario.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert!(cassette.interactions.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(scenario: ScenarioMetadata) -> Self {
        Self {
            scenario,
            interactions: Vec::new(),
        }
    }

    /// Appends one request/response interaction.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, RecordedRequest, RecordedResponse, ScenarioMetadata,
    ///     ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?;
    /// let response = RecordedResponse::new(202, BTreeMap::new(), "{}")?;
    ///
    /// recorder.record(request, response);
    ///
    /// assert_eq!(recorder.finish().interactions.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn record(&mut self, request: RecordedRequest, response: RecordedResponse) {
        self.interactions
            .push(CassetteInteraction { request, response });
    }

    /// Finishes the recording and returns a cassette.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    ///
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert_eq!(cassette.schema_version, "1.0");
    /// # Ok(())
    /// # }
    /// ```
    pub fn finish(self) -> Cassette {
        Cassette {
            schema_version: "1.0".to_owned(),
            scenario: self.scenario,
            interactions: self.interactions,
        }
    }
}

/// Part of an interaction a scrub rule applies to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrubScope {
    /// Request paths.
    Path,
    /// Request and response bodies.
    Body,
    /// Header values.
    Headers,
    /// Request paths, bodies, and header values.
    All,
}

/// One country-scoped scrubber rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScrubRule {
    /// ISO country code or `*` for all countries.
    pub country: String,
    /// Sensitive literal to remove.
    pub find: String,
    /// Deterministic replacement token.
    pub replacement: String,
    /// Interaction scope.
    pub scope: ScrubScope,
}

impl ScrubRule {
    /// Builds a scrub rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScrubRule, ScrubScope};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let rule = ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::All)?;
    ///
    /// assert_eq!(rule.country, "DE");
    /// assert_eq!(rule.replacement, "[VAT-DE-1]");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `country`, `find`, or
    /// `replacement` is blank, or when `country` is neither `*` nor an ISO
    /// uppercase alpha-2 code.
    pub fn new(
        country: impl Into<String>,
        find: impl Into<String>,
        replacement: impl Into<String>,
        scope: ScrubScope,
    ) -> Result<Self, CassetteError> {
        let country = country.into();
        if country != "*" {
            validate_country(&country)?;
        }
        Ok(Self {
            country,
            find: non_empty(find.into(), "find")?,
            replacement: non_empty(replacement.into(), "replacement")?,
            scope,
        })
    }

    fn applies_to(&self, country: &str) -> bool {
        self.country == "*" || self.country == country
    }
}

/// Configurable country-aware cassette scrubber.
#[derive(Default)]
pub struct Scrubber {
    rules: Vec<ScrubRule>,
}

impl Scrubber {
    /// Creates a scrubber from pre-validated rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScrubRule, ScrubScope, Scrubber};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scrubber = Scrubber::new(vec![
    ///     ScrubRule::new("*", "secret@example.com", "[EMAIL-1]", ScrubScope::All)?,
    /// ]);
    ///
    /// let _ = scrubber;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(rules: Vec<ScrubRule>) -> Self {
        Self { rules }
    }

    /// Applies all rules for `country` and returns a scrubbed cassette.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, RecordedRequest, RecordedResponse, ScenarioMetadata,
    ///     ScenarioSource, ScrubRule, ScrubScope, Scrubber,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/pii",
    ///     "Synthetic PII case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "default-de",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "DE123456789")?;
    /// let response = RecordedResponse::new(202, BTreeMap::new(), "{}")?;
    /// recorder.record(request, response);
    ///
    /// let scrubber = Scrubber::new(vec![
    ///     ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::Body)?,
    /// ]);
    /// let scrubbed = scrubber.scrub_cassette("DE", &recorder.finish());
    ///
    /// assert_eq!(scrubbed.interactions[0].request.body, "[VAT-DE-1]");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn scrub_cassette(&self, country: &str, cassette: &Cassette) -> Cassette {
        let mut out = cassette.clone();
        for interaction in &mut out.interactions {
            for rule in self.rules.iter().filter(|rule| rule.applies_to(country)) {
                match rule.scope {
                    ScrubScope::Path => scrub_path(interaction, rule),
                    ScrubScope::Body => scrub_bodies(interaction, rule),
                    ScrubScope::Headers => scrub_headers(interaction, rule),
                    ScrubScope::All => {
                        scrub_path(interaction, rule);
                        scrub_bodies(interaction, rule);
                        scrub_headers(interaction, rule);
                    }
                }
            }
            interaction.request.body_fingerprint =
                body_fingerprint(interaction.request.body.as_bytes());
        }
        out
    }
}

/// Counts high-risk unsanitized personal-data patterns in a cassette.
///
/// This deliberately errs on the side of false positives for CI use. It
/// catches country-prefixed tax IDs such as `DE123456789`, IBAN-like
/// account numbers, and email addresses in request paths, request/response
/// bodies, and header values.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// use invoicekit_transmit_mock::{
///     count_unscrubbed_pii_patterns, CassetteRecorder, RecordedRequest,
///     RecordedResponse, ScenarioMetadata, ScenarioSource,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let scenario = ScenarioMetadata::new(
///     "synthetic/pii",
///     "Synthetic PII case",
///     "DE",
///     "mock",
///     ScenarioSource::Synthetic,
///     "none",
/// )?;
/// let mut recorder = CassetteRecorder::new(scenario);
/// recorder.record(
///     RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "DE123456789")?,
///     RecordedResponse::new(202, BTreeMap::new(), "{}")?,
/// );
///
/// assert_eq!(count_unscrubbed_pii_patterns(&recorder.finish()), 1);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub fn count_unscrubbed_pii_patterns(cassette: &Cassette) -> usize {
    cassette
        .interactions
        .iter()
        .map(|interaction| {
            count_text_pii_patterns(&interaction.request.path)
                + count_text_pii_patterns(&interaction.request.body)
                + count_text_pii_patterns(&interaction.response.body)
                + count_header_pii_patterns(&interaction.request.headers)
                + count_header_pii_patterns(&interaction.response.headers)
        })
        .sum()
}

/// Fails when a cassette still contains high-risk personal-data patterns.
///
/// # Errors
///
/// Returns [`CassetteError::UnscrubbedPii`] when request paths, bodies, or
/// headers still contain values that look like tax identifiers, IBANs, or
/// email addresses.
///
/// # Examples
///
/// ```
/// use invoicekit_transmit_mock::{
///     assert_no_unscrubbed_pii_patterns, CassetteRecorder, ScenarioMetadata,
///     ScenarioSource,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let scenario = ScenarioMetadata::new(
///     "synthetic/clean",
///     "Synthetic clean case",
///     "DE",
///     "mock",
///     ScenarioSource::Synthetic,
///     "none",
/// )?;
/// let cassette = CassetteRecorder::new(scenario).finish();
///
/// assert_no_unscrubbed_pii_patterns(&cassette)?;
/// # Ok(())
/// # }
/// ```
pub fn assert_no_unscrubbed_pii_patterns(cassette: &Cassette) -> Result<(), CassetteError> {
    let finding_count = count_unscrubbed_pii_patterns(cassette);
    if finding_count == 0 {
        Ok(())
    } else {
        Err(CassetteError::UnscrubbedPii { finding_count })
    }
}

/// Matcher keyed by method + path + body fingerprint.
pub struct CassetteMatcher<'a> {
    responses: BTreeMap<MatchKey, &'a RecordedResponse>,
}

impl<'a> CassetteMatcher<'a> {
    /// Builds a matcher and rejects duplicate request keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteMatcher, CassetteRecorder, RecordedRequest, RecordedResponse,
    ///     ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/replay",
    ///     "Synthetic replay case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// recorder.record(
    ///     RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?,
    ///     RecordedResponse::new(202, BTreeMap::new(), r#"{"accepted":true}"#)?,
    /// );
    /// let cassette = recorder.finish();
    ///
    /// let matcher = CassetteMatcher::new(&cassette)?;
    ///
    /// assert_eq!(
    ///     matcher.match_request(&cassette.interactions[0].request)?.status,
    ///     202
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::MatcherCollision`] if two interactions
    /// share method, path, and body fingerprint.
    pub fn new(cassette: &'a Cassette) -> Result<Self, CassetteError> {
        let mut responses = BTreeMap::new();
        for interaction in &cassette.interactions {
            let key = interaction.request.match_key();
            if responses.contains_key(&key) {
                return Err(CassetteError::MatcherCollision {
                    method: key.method,
                    path: key.path,
                    body_fingerprint: key.body_fingerprint,
                });
            }
            responses.insert(key, &interaction.response);
        }
        Ok(Self { responses })
    }

    /// Returns the response for `request`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteMatcher, CassetteRecorder, RecordedRequest, RecordedResponse,
    ///     ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/replay",
    ///     "Synthetic replay case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// recorder.record(
    ///     request.clone(),
    ///     RecordedResponse::new(202, BTreeMap::new(), r#"{"accepted":true}"#)?,
    /// );
    /// let cassette = recorder.finish();
    /// let matcher = CassetteMatcher::new(&cassette)?;
    ///
    /// assert_eq!(matcher.match_request(&request)?.body, r#"{"accepted":true}"#);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::NoMatch`] when no recorded interaction has
    /// the same method, path, and body fingerprint.
    pub fn match_request(
        &self,
        request: &RecordedRequest,
    ) -> Result<&'a RecordedResponse, CassetteError> {
        let key = request.match_key();
        self.responses
            .get(&key)
            .copied()
            .ok_or(CassetteError::NoMatch {
                method: key.method,
                path: key.path,
                body_fingerprint: key.body_fingerprint,
            })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MatchKey {
    method: String,
    path: String,
    body_fingerprint: String,
}

/// Computes the BLAKE3 body fingerprint used by [`CassetteMatcher`].
///
/// # Examples
///
/// ```
/// let first = invoicekit_transmit_mock::body_fingerprint(b"{}");
/// let second = invoicekit_transmit_mock::body_fingerprint(b"{}");
///
/// assert_eq!(first, second);
/// assert_ne!(first, invoicekit_transmit_mock::body_fingerprint(b"{\"x\":1}"));
/// ```
#[must_use]
pub fn body_fingerprint(body: &[u8]) -> String {
    blake3::hash(body).to_hex().to_string()
}

/// Parses the scenario metadata JSON Schema.
///
/// # Errors
///
/// Returns [`CassetteError::Json`] if the embedded schema is malformed.
///
/// # Examples
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = invoicekit_transmit_mock::scenario_metadata_schema()?;
///
/// assert_eq!(schema["properties"]["schema_version"]["const"], "1.0");
/// # Ok(())
/// # }
/// ```
pub fn scenario_metadata_schema() -> Result<serde_json::Value, CassetteError> {
    Ok(serde_json::from_str(SCENARIO_METADATA_SCHEMA_JSON)?)
}

fn scrub_path(interaction: &mut CassetteInteraction, rule: &ScrubRule) {
    interaction.request.path = interaction
        .request
        .path
        .replace(rule.find.as_str(), rule.replacement.as_str());
}

fn scrub_bodies(interaction: &mut CassetteInteraction, rule: &ScrubRule) {
    interaction.request.body = interaction
        .request
        .body
        .replace(rule.find.as_str(), rule.replacement.as_str());
    interaction.response.body = interaction
        .response
        .body
        .replace(rule.find.as_str(), rule.replacement.as_str());
}

fn scrub_headers(interaction: &mut CassetteInteraction, rule: &ScrubRule) {
    for value in interaction.request.headers.values_mut() {
        *value = value.replace(rule.find.as_str(), rule.replacement.as_str());
    }
    for value in interaction.response.headers.values_mut() {
        *value = value.replace(rule.find.as_str(), rule.replacement.as_str());
    }
}

fn count_header_pii_patterns(headers: &BTreeMap<String, String>) -> usize {
    headers
        .values()
        .map(|value| count_text_pii_patterns(value))
        .sum()
}

fn count_text_pii_patterns(value: &str) -> usize {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '+')))
        .filter(|token| {
            looks_like_country_tax_id(token) || looks_like_iban(token) || looks_like_email(token)
        })
        .count()
}

fn looks_like_country_tax_id(token: &str) -> bool {
    if !(10..=14).contains(&token.len()) {
        return false;
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(second) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase() && second.is_ascii_uppercase() && chars.all(|c| c.is_ascii_digit())
}

fn looks_like_iban(token: &str) -> bool {
    if !(15..=34).contains(&token.len()) {
        return false;
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(second) = chars.next() else {
        return false;
    };
    let Some(third) = chars.next() else {
        return false;
    };
    let Some(fourth) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && second.is_ascii_uppercase()
        && third.is_ascii_digit()
        && fourth.is_ascii_digit()
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn looks_like_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

fn non_empty(value: String, field: &'static str) -> Result<String, CassetteError> {
    if value.trim().is_empty() {
        return Err(CassetteError::MissingRequiredField { field });
    }
    Ok(value)
}

fn validate_country(country: &str) -> Result<(), CassetteError> {
    if country.len() == 2 && country.chars().all(|c| c.is_ascii_uppercase()) {
        return Ok(());
    }
    Err(CassetteError::InvalidField {
        field: "country",
        reason: "expected ISO 3166-1 alpha-2 uppercase code".to_owned(),
    })
}

fn validate_path(path: &str) -> Result<(), CassetteError> {
    if path.starts_with('/') && !path.trim().is_empty() {
        return Ok(());
    }
    Err(CassetteError::InvalidField {
        field: "path",
        reason: "expected an absolute request path starting with `/`".to_owned(),
    })
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_transmit_mock::crate_name(), "invoicekit-transmit-mock");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-mock"
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use invoicekit_ir::CommercialDocument;
    use invoicekit_reconcile::{
        GatewayAdapter, GatewayAttemptId, GatewayContext, GatewayErrorKind, GatewayOperation,
        GatewayRoute, GatewayStatus, GatewaySubmissionId, IdempotencyKey, PollRequest,
        SubmitRequest, TenantId, TraceId,
    };
    use serde_json::json;

    use super::{
        assert_no_unscrubbed_pii_patterns, body_fingerprint, count_unscrubbed_pii_patterns,
        crate_name, scenario_metadata_schema, Cassette, CassetteMatcher, CassetteRecorder,
        MockGatewayAdapter, RecordedRequest, RecordedResponse, ScenarioMetadata, ScenarioSource,
        ScrubRule, ScrubScope, Scrubber,
    };

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-transmit-mock");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn recorder_emits_byte_stable_vcr_bytes() {
        let first = sample_cassette_bytes();
        let second = sample_cassette_bytes();

        assert_eq!(first, second);
        assert!(String::from_utf8(first)
            .unwrap()
            .contains("\"schema_version\": \"1.0\""));
    }

    #[test]
    fn scrubber_removes_country_scoped_personal_data() {
        let cassette = sample_cassette();
        let scrubber = Scrubber::new(vec![
            ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::All).unwrap(),
            ScrubRule::new("FR", "FR123456789", "[VAT-FR-1]", ScrubScope::All).unwrap(),
        ]);

        let redacted = scrubber.scrub_cassette("DE", &cassette);
        let bytes = String::from_utf8(redacted.to_vcr_bytes().unwrap()).unwrap();

        assert!(!bytes.contains("DE123456789"));
        assert!(bytes.contains("[VAT-DE-1]"));
        assert!(bytes.contains("FR123456789"));
        let interaction = first_interaction(&redacted);
        assert_eq!(
            interaction.request.body_fingerprint,
            body_fingerprint(interaction.request.body.as_bytes())
        );
    }

    #[test]
    fn pii_scan_reports_unscrubbed_patterns() {
        let cassette = sample_cassette();

        assert!(count_unscrubbed_pii_patterns(&cassette) >= 2);
        assert!(assert_no_unscrubbed_pii_patterns(&cassette).is_err());
    }

    #[test]
    fn pii_scan_reports_unscrubbed_request_path() {
        let cassette = path_pii_cassette();

        assert_eq!(count_unscrubbed_pii_patterns(&cassette), 1);
        assert!(assert_no_unscrubbed_pii_patterns(&cassette).is_err());
    }

    #[test]
    fn scrubber_removes_path_personal_data_with_all_scope() {
        let cassette = path_pii_cassette();
        let scrubber = Scrubber::new(vec![ScrubRule::new(
            "*",
            "DE123456789",
            "[VAT-DE-1]",
            ScrubScope::All,
        )
        .unwrap()]);

        let redacted = scrubber.scrub_cassette("DE", &cassette);

        assert_eq!(
            first_interaction(&redacted).request.path,
            "/taxpayer/[VAT-DE-1]"
        );
        assert_eq!(count_unscrubbed_pii_patterns(&redacted), 0);
    }

    #[test]
    fn pii_scan_passes_after_all_sensitive_values_are_scrubbed() {
        let cassette = sample_cassette();
        let scrubber = Scrubber::new(vec![
            ScrubRule::new("*", "DE123456789", "[VAT-DE-1]", ScrubScope::All).unwrap(),
            ScrubRule::new("*", "FR123456789", "[VAT-FR-1]", ScrubScope::All).unwrap(),
        ]);

        let redacted = scrubber.scrub_cassette("DE", &cassette);

        assert_eq!(count_unscrubbed_pii_patterns(&redacted), 0);
        assert!(assert_no_unscrubbed_pii_patterns(&redacted).is_ok());
    }

    #[test]
    fn matcher_routes_by_method_path_and_body_fingerprint() {
        let cassette = sample_cassette();
        let matcher = CassetteMatcher::new(&cassette).unwrap();
        let response = matcher
            .match_request(&first_interaction(&cassette).request)
            .unwrap();

        assert_eq!(response.status, 202);

        let different_body = RecordedRequest::new(
            "POST",
            "/ksef/invoices",
            BTreeMap::new(),
            "<Invoice>different</Invoice>",
        )
        .unwrap();
        assert!(matcher.match_request(&different_body).is_err());
    }

    #[test]
    fn matcher_derives_fingerprint_from_body_not_stored_field() {
        let mut cassette = sample_cassette();
        first_interaction_mut(&mut cassette)
            .request
            .body_fingerprint = body_fingerprint(b"stale");
        let matcher = CassetteMatcher::new(&cassette).unwrap();
        let request = RecordedRequest::new(
            "POST",
            "/ksef/invoices",
            BTreeMap::new(),
            "<Invoice><Seller>DE123456789</Seller></Invoice>",
        )
        .unwrap();

        assert_eq!(matcher.match_request(&request).unwrap().status, 202);
    }

    #[test]
    fn matcher_rejects_colliding_interactions() {
        let mut cassette = sample_cassette();
        let repeated = first_interaction(&cassette).clone();
        cassette.interactions.push(repeated);

        assert!(CassetteMatcher::new(&cassette).is_err());
    }

    #[test]
    fn scenario_metadata_schema_is_valid_json() {
        let schema = scenario_metadata_schema().unwrap();

        assert_eq!(schema["title"], "InvoiceKit cassette scenario metadata");
        assert_eq!(schema["properties"]["schema_version"]["const"], "1.0");
    }

    #[test]
    fn invalid_country_is_rejected() {
        let err = ScenarioMetadata::new(
            "de/ksef/success",
            "KSeF success",
            "de",
            "ksef",
            ScenarioSource::OfficialSandbox,
            "default-de",
        )
        .unwrap_err();

        assert!(err.to_string().contains("country"));
    }

    #[test]
    fn baseline_cassettes_match_recorder_output_and_are_scrubbed() {
        let accepted = accepted_cassette();
        let rejected = rejected_cassette();

        assert_eq!(
            String::from_utf8(accepted.to_vcr_bytes().unwrap()).unwrap(),
            include_str!(
                "../../../conformance-corpus/cassettes/mock/accepted/interaction.vcr.json"
            )
        );
        assert_eq!(
            String::from_utf8(rejected.to_vcr_bytes().unwrap()).unwrap(),
            include_str!(
                "../../../conformance-corpus/cassettes/mock/rejected/interaction.vcr.json"
            )
        );
        assert_eq!(
            accepted.to_vcr_bytes().unwrap(),
            accepted_cassette().to_vcr_bytes().unwrap()
        );
        assert_eq!(
            rejected.to_vcr_bytes().unwrap(),
            rejected_cassette().to_vcr_bytes().unwrap()
        );
        assert_no_unscrubbed_pii_patterns(&accepted).unwrap();
        assert_no_unscrubbed_pii_patterns(&rejected).unwrap();
    }

    #[test]
    fn baseline_scenario_metadata_files_match_cassettes() {
        let accepted: ScenarioMetadata = serde_json::from_str(include_str!(
            "../../../conformance-corpus/cassettes/mock/accepted/scenario.json"
        ))
        .unwrap();
        let rejected: ScenarioMetadata = serde_json::from_str(include_str!(
            "../../../conformance-corpus/cassettes/mock/rejected/scenario.json"
        ))
        .unwrap();

        assert_eq!(accepted, accepted_cassette().scenario);
        assert_eq!(rejected, rejected_cassette().scenario);
    }

    #[test]
    fn mock_gateway_replays_successful_submit_cassette() {
        let adapter = MockGatewayAdapter::new([accepted_cassette()]).unwrap();
        let receipt = block_on_ready(adapter.submit(submit_request("success"))).unwrap();

        assert_eq!(receipt.operation, GatewayOperation::Submit);
        assert_eq!(receipt.status, GatewayStatus::Accepted);
        assert_eq!(receipt.submission_id.as_str(), "mock_sub_success");
        assert_eq!(
            receipt.gateway_reference.as_deref(),
            Some("MOCK-ACCEPTED-1")
        );
        assert!(receipt.raw_receipt_hash.is_some());
    }

    #[test]
    fn mock_gateway_replays_gateway_failure_cassette() {
        let adapter = MockGatewayAdapter::new([rejected_cassette()]).unwrap();
        let err = block_on_ready(adapter.submit(submit_request("rejected"))).unwrap_err();

        assert_eq!(err.kind, GatewayErrorKind::Rejected);
        assert_eq!(err.gateway_code.as_deref(), Some("MOCK_REJECTED"));
        assert_eq!(err.submission_id.unwrap().as_str(), "mock_sub_rejected");
        assert!(err.remediation.contains("fix"));
    }

    #[test]
    fn mock_gateway_reports_no_matching_cassette() {
        let adapter = MockGatewayAdapter::new([accepted_cassette()]).unwrap();
        let err = block_on_ready(adapter.submit(submit_request("unknown"))).unwrap_err();

        assert_eq!(err.kind, GatewayErrorKind::NotFound);
        assert!(err.message.contains("no cassette interaction matched"));
    }

    #[test]
    fn mock_gateway_document_content_changes_cassette_key() {
        let adapter = MockGatewayAdapter::new([accepted_cassette()]).unwrap();
        let changed_document = synthetic_document_with_payable_amount("success", "118.00");
        let changed_request = SubmitRequest::new(
            gateway_context("success"),
            gateway_route(),
            changed_document,
        )
        .unwrap();

        let err = block_on_ready(adapter.submit(changed_request)).unwrap_err();

        assert_eq!(err.kind, GatewayErrorKind::NotFound);
        assert!(err.message.contains("no cassette interaction matched"));
    }

    #[test]
    fn mock_gateway_rejects_duplicate_cassette_keys() {
        let first = accepted_cassette();
        let second = accepted_cassette();

        let err = MockGatewayAdapter::new([first, second]).unwrap_err();

        assert!(matches!(err, super::CassetteError::MatcherCollision { .. }));
    }

    #[test]
    fn mock_gateway_replays_poll_operation() {
        let request = poll_request("poll-success");
        let mut recorder = CassetteRecorder::new(
            ScenarioMetadata::new(
                "mock/poll/accepted",
                "Mock poll accepted",
                "DE",
                "mock",
                ScenarioSource::Synthetic,
                "none",
            )
            .unwrap(),
        );
        recorder.record(
            MockGatewayAdapter::recorded_poll_request(&request).unwrap(),
            RecordedResponse::new(
                200,
                BTreeMap::new(),
                r#"{"submission_id":"mock_sub_success","status":"accepted","gateway_reference":"MOCK-POLL-1","received_at":"2026-05-27T00:05:00Z","detail":"Synthetic mock gateway poll accepted."}"#,
            )
            .unwrap(),
        );
        let adapter = MockGatewayAdapter::new([recorder.finish()]).unwrap();

        let receipt = block_on_ready(adapter.poll(request)).unwrap();

        assert_eq!(receipt.operation, GatewayOperation::Poll);
        assert_eq!(receipt.status, GatewayStatus::Accepted);
        assert_eq!(receipt.gateway_reference.as_deref(), Some("MOCK-POLL-1"));
    }

    #[test]
    fn gateway_contract_required_scenarios_pass_for_mock_adapter() {
        let scenarios = super::gateway_contract_scenarios().unwrap();
        let actual_scenarios: Vec<&str> = scenarios
            .iter()
            .map(super::GatewayContractScenario::id)
            .collect();
        assert_eq!(
            actual_scenarios.as_slice(),
            super::GATEWAY_CONTRACT_SCENARIO_IDS
        );
        let adapter =
            MockGatewayAdapter::new(super::gateway_contract_cassettes().unwrap()).unwrap();

        for scenario in &scenarios {
            let request = scenario.submit_request().unwrap();
            let submit_result = block_on_ready(adapter.submit(request));
            scenario.verify_submit_result(&submit_result).unwrap();

            if let Some(request) = scenario.poll_request().unwrap() {
                let poll_result = block_on_ready(adapter.poll(request));
                scenario.verify_poll_result(&poll_result).unwrap();
            }
        }
    }

    #[test]
    fn gateway_contract_idempotent_replay_returns_same_receipt() {
        let scenario = super::gateway_contract_scenarios()
            .unwrap()
            .into_iter()
            .find(|scenario| scenario.id() == "idempotent-replay")
            .unwrap();
        let adapter = MockGatewayAdapter::new([scenario.cassette().unwrap()]).unwrap();
        let request = scenario.submit_request().unwrap();

        let first_result = block_on_ready(adapter.submit(request.clone()));
        scenario.verify_submit_result(&first_result).unwrap();
        let second_result = block_on_ready(adapter.submit(request));
        scenario.verify_submit_result(&second_result).unwrap();
        let first = first_result.unwrap();
        let second = second_result.unwrap();

        assert_eq!(first, second);
        assert_eq!(first.status, GatewayStatus::Accepted);
        assert_eq!(
            first.gateway_reference.as_deref(),
            Some("MOCK-CONTRACT-IDEMPOTENT")
        );
    }

    #[test]
    fn gateway_contract_cassettes_are_byte_stable_and_scrubbed() {
        let first = gateway_contract_cassette_bytes();
        let second = gateway_contract_cassette_bytes();

        assert_eq!(first, second);

        for cassette in super::gateway_contract_cassettes().unwrap() {
            assert!(cassette
                .scenario
                .tags
                .iter()
                .any(|tag| tag == "gateway-contract"));
            assert_no_unscrubbed_pii_patterns(&cassette).unwrap();
        }
    }

    #[test]
    fn mock_gateway_preserves_structured_error_with_incorrect_html_header() {
        let request = submit_request("structured-html-header");
        let mut recorder = CassetteRecorder::new(
            ScenarioMetadata::new(
                "mock/contract/structured-html-header",
                "Structured error with incorrect HTML header",
                "DE",
                "mock",
                ScenarioSource::Synthetic,
                "none",
            )
            .unwrap(),
        );
        let mut headers = BTreeMap::new();
        headers.insert(
            "content-type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        );
        recorder.record(
            MockGatewayAdapter::recorded_submit_request(&request).unwrap(),
            RecordedResponse::new(
                503,
                headers,
                r#"{"kind":"partner_error","message":"partner service unavailable","remediation":"retry after partner recovery","gateway_code":"PARTNER_STRUCTURED","retry_after_seconds":90}"#,
            )
            .unwrap(),
        );
        let adapter = MockGatewayAdapter::new([recorder.finish()]).unwrap();

        let error = block_on_ready(adapter.submit(request)).unwrap_err();

        assert_eq!(error.kind, GatewayErrorKind::PartnerError);
        assert_eq!(error.gateway_code.as_deref(), Some("PARTNER_STRUCTURED"));
        assert_eq!(error.retry_after_seconds, Some(90));
    }

    #[test]
    fn cassette_corpus_has_no_unscrubbed_pii() {
        let repo = repo_root();
        let cassette_root = repo.join("conformance-corpus").join("cassettes");
        if !cassette_root.is_dir() {
            return;
        }

        let mut checked = 0;
        scan_cassette_dir(&cassette_root, &mut checked);
    }

    fn gateway_contract_cassette_bytes() -> Vec<Vec<u8>> {
        super::gateway_contract_cassettes()
            .unwrap()
            .iter()
            .map(|cassette| cassette.to_vcr_bytes().unwrap())
            .collect()
    }

    fn sample_cassette_bytes() -> Vec<u8> {
        sample_cassette().to_vcr_bytes().unwrap()
    }

    fn path_pii_cassette() -> super::Cassette {
        let scenario = ScenarioMetadata::new(
            "synthetic/path-pii",
            "Synthetic path PII case",
            "DE",
            "mock",
            ScenarioSource::Synthetic,
            "default-de",
        )
        .unwrap();
        let mut recorder = CassetteRecorder::new(scenario);
        recorder.record(
            RecordedRequest::new("GET", "/taxpayer/DE123456789", BTreeMap::new(), "{}").unwrap(),
            RecordedResponse::new(200, BTreeMap::new(), "{}").unwrap(),
        );
        recorder.finish()
    }

    fn first_interaction(cassette: &super::Cassette) -> &super::CassetteInteraction {
        cassette.interactions.first().unwrap()
    }

    fn first_interaction_mut(cassette: &mut super::Cassette) -> &mut super::CassetteInteraction {
        cassette.interactions.first_mut().unwrap()
    }

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
    }

    fn scan_cassette_dir(dir: &Path, checked: &mut usize) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                scan_cassette_dir(&path, checked);
            } else if is_vcr_path(&path) {
                assert_vcr_file_is_scrubbed(&path);
                *checked += 1;
            }
        }
    }

    fn is_vcr_path(path: &Path) -> bool {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vcr"))
        {
            return true;
        }

        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && path
                .file_stem()
                .and_then(|stem| Path::new(stem).extension())
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("vcr"))
    }

    fn assert_vcr_file_is_scrubbed(path: &Path) {
        let bytes = fs::read(path).unwrap();
        let cassette = Cassette::from_vcr_bytes(&bytes);
        assert!(
            cassette.is_ok(),
            "{}: invalid cassette JSON: {}",
            path.display(),
            cassette.unwrap_err()
        );
        let cassette = Cassette::from_vcr_bytes(&bytes).unwrap();
        let scan_result = assert_no_unscrubbed_pii_patterns(&cassette);
        assert!(
            scan_result.is_ok(),
            "{}: {}",
            path.display(),
            scan_result.unwrap_err()
        );
    }

    fn accepted_cassette() -> super::Cassette {
        let mut recorder = CassetteRecorder::new(
            ScenarioMetadata::new(
                "mock/submit/accepted",
                "Mock submit accepted",
                "DE",
                "mock",
                ScenarioSource::Synthetic,
                "none",
            )
            .unwrap(),
        );
        recorder.record(
            MockGatewayAdapter::recorded_submit_request(&submit_request("success")).unwrap(),
            RecordedResponse::new(
                202,
                BTreeMap::new(),
                r#"{"submission_id":"mock_sub_success","status":"accepted","gateway_reference":"MOCK-ACCEPTED-1","received_at":"2026-05-27T00:00:00Z","detail":"Synthetic mock gateway accepted the invoice."}"#,
            )
            .unwrap(),
        );
        recorder.finish()
    }

    fn rejected_cassette() -> super::Cassette {
        let mut recorder = CassetteRecorder::new(
            ScenarioMetadata::new(
                "mock/submit/rejected",
                "Mock submit rejected",
                "DE",
                "mock",
                ScenarioSource::Synthetic,
                "none",
            )
            .unwrap(),
        );
        recorder.record(
            MockGatewayAdapter::recorded_submit_request(&submit_request("rejected")).unwrap(),
            RecordedResponse::new(
                422,
                BTreeMap::new(),
                r#"{"kind":"rejected","message":"mock gateway rejected the invoice","remediation":"fix the invoice payload and replay the matching cassette","gateway_code":"MOCK_REJECTED","submission_id":"mock_sub_rejected"}"#,
            )
            .unwrap(),
        );
        recorder.finish()
    }

    fn submit_request(case: &str) -> SubmitRequest {
        SubmitRequest::new(
            gateway_context(case),
            gateway_route(),
            synthetic_document(case),
        )
        .unwrap()
    }

    fn poll_request(case: &str) -> PollRequest {
        PollRequest::new(
            gateway_context(case),
            GatewaySubmissionId::new("mock_sub_success").unwrap(),
        )
    }

    fn gateway_context(case: &str) -> GatewayContext {
        GatewayContext::new(
            TenantId::new("tenant_mock").unwrap(),
            TraceId::new(format!("trace_mock_{case}")).unwrap(),
            IdempotencyKey::new(format!("idem_mock_{case}")).unwrap(),
            GatewayAttemptId::new(format!("attempt_mock_{case}")).unwrap(),
        )
    }

    fn gateway_route() -> GatewayRoute {
        GatewayRoute::new("mock", "mock-profile", Some("DE")).unwrap()
    }

    fn synthetic_document(case: &str) -> CommercialDocument {
        synthetic_document_with_payable_amount(case, "119.00")
    }

    fn synthetic_document_with_payable_amount(
        case: &str,
        payable_amount: &str,
    ) -> CommercialDocument {
        CommercialDocument::try_from_value(json!({
            "schema_version": "1.0",
            "id": format!("doc_mock_{case}"),
            "document_type": "invoice",
            "issue_date": "2026-05-27",
            "document_number": format!("INV-MOCK-{}", case.to_ascii_uppercase()),
            "currency": "EUR",
            "supplier": party_json("supplier_mock", "Mock Supplier GmbH", "DE"),
            "customer": party_json("customer_mock", "Mock Buyer SAS", "FR"),
            "lines": [{
                "id": "1",
                "description": "Mock gateway fixture",
                "quantity": "1",
                "unit_price": "100.00",
                "line_extension_amount": "100.00"
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
                "payable_amount": payable_amount
            },
            "meta": {
                "tenant_id": "tenant_mock",
                "trace_id": format!("trace_mock_{case}"),
                "source_system": "transmit-mock-test"
            }
        }))
        .unwrap()
    }

    fn party_json(id: &str, name: &str, country: &str) -> serde_json::Value {
        json!({
            "id": id,
            "name": name,
            "tax_ids": [{
                "scheme": "test",
                "value": format!("{country}-MOCK-TAX")
            }],
            "address": {
                "lines": ["Mock Street 1"],
                "city": "Mock City",
                "postal_code": "10000",
                "country": country
            }
        })
    }

    fn block_on_ready<T>(future: impl Future<Output = T>) -> T {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                break value;
            }
            std::thread::yield_now();
        }
    }

    fn sample_cassette() -> super::Cassette {
        let scenario = ScenarioMetadata::new(
            "de/ksef/success",
            "KSeF accepted invoice",
            "DE",
            "ksef",
            ScenarioSource::OfficialSandbox,
            "default-de",
        )
        .unwrap();
        let mut recorder = CassetteRecorder::new(scenario);
        let mut request_headers = BTreeMap::new();
        request_headers.insert("x-tax-id".to_owned(), "DE123456789".to_owned());
        let request = RecordedRequest::new(
            "post",
            "/ksef/invoices",
            request_headers,
            "<Invoice><Seller>DE123456789</Seller></Invoice>",
        )
        .unwrap();
        let response = RecordedResponse::new(
            202,
            BTreeMap::new(),
            "{\"status\":\"accepted\",\"buyer\":\"FR123456789\"}",
        )
        .unwrap();
        recorder.record(request, response);
        recorder.finish()
    }
}
