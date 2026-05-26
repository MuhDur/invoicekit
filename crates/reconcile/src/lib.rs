// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-reconcile` - gateway contracts, transmission identity, and
//! reconciliation primitives.
//!
//! Gateway integrations enter InvoiceKit through [`GatewayAdapter`]. The
//! adapter boundary carries the identifiers required by PLAN.md section 2.5:
//! tenant ID, trace ID, idempotency key, and gateway attempt ID. The state
//! machine and outbox beads build on this stable contract instead of allowing
//! each country gateway to invent its own error language.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use invoicekit_ir::CommercialDocument;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Boxed future returned by gateway adapter operations.
///
/// The boxed shape keeps [`GatewayAdapter`] object-safe, so the transmission
/// worker can store partner, native, and mock adapters behind one trait object.
pub type GatewayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, GatewayError>> + Send + 'a>>;

macro_rules! gateway_id_type {
    ($type_name:ident, $field_name:literal, $summary:literal, $example_value:literal) => {
        #[doc = $summary]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $type_name(String);

        impl $type_name {
            #[doc = concat!("Builds a validated ", $field_name, ".")]
            ///
            /// # Errors
            ///
            #[doc = concat!(
                "Returns [`ReconcileError::MissingRequiredField`] when `value` is blank, ",
                "or [`ReconcileError::InvalidIdentifier`] when it has leading or trailing ",
                "whitespace or control characters."
            )]
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("use invoicekit_reconcile::", stringify!($type_name), ";")]
            ///
            #[doc = concat!("let id = ", stringify!($type_name), "::new(\"", $example_value, "\").unwrap();")]
            #[doc = concat!("assert_eq!(id.as_str(), \"", $example_value, "\");")]
            /// ```
            pub fn new(value: impl Into<String>) -> Result<Self, ReconcileError> {
                let value = value.into();
                validate_identifier(&value, $field_name)?;
                Ok(Self(value))
            }

            #[doc = concat!("Returns the ", $field_name, " as text.")]
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("use invoicekit_reconcile::", stringify!($type_name), ";")]
            ///
            #[doc = concat!("let id = ", stringify!($type_name), "::new(\"", $example_value, "\").unwrap();")]
            #[doc = concat!("assert_eq!(id.as_str(), \"", $example_value, "\");")]
            /// ```
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $type_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

gateway_id_type!(
    TenantId,
    "tenant_id",
    "Tenant identifier attached to every transmission.",
    "tenant_acme"
);
gateway_id_type!(
    TraceId,
    "trace_id",
    "Trace identifier propagated from API edge to gateway attempts.",
    "trace_123"
);
gateway_id_type!(
    IdempotencyKey,
    "idempotency_key",
    "Idempotency key used to make gateway submission retries safe.",
    "idem_invoice_123"
);
gateway_id_type!(
    GatewayAttemptId,
    "gateway_attempt_id",
    "Identifier for one concrete gateway attempt.",
    "attempt_001"
);
gateway_id_type!(
    GatewaySubmissionId,
    "gateway_submission_id",
    "Gateway-visible submission identifier or receipt handle.",
    "sub_001"
);

/// Per-attempt context required for every gateway operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayContext {
    /// Tenant whose invoice is being transmitted.
    pub tenant_id: TenantId,
    /// Trace identifier for logs, evidence bundles, and receipts.
    pub trace_id: TraceId,
    /// Retry-safe idempotency key.
    pub idempotency_key: IdempotencyKey,
    /// Attempt identifier for this concrete gateway call.
    pub gateway_attempt_id: GatewayAttemptId,
}

impl GatewayContext {
    /// Builds gateway operation context from validated identifiers.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, IdempotencyKey, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// assert_eq!(context.trace_id.as_str(), "trace_123");
    /// ```
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        trace_id: TraceId,
        idempotency_key: IdempotencyKey,
        gateway_attempt_id: GatewayAttemptId,
    ) -> Self {
        Self {
            tenant_id,
            trace_id,
            idempotency_key,
            gateway_attempt_id,
        }
    }
}

/// Gateway route selected by routing policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayRoute {
    /// Routing target, such as `peppol`, `ksef`, or `sdi`.
    pub route: String,
    /// Invoice profile or clearance profile expected by the gateway.
    pub profile: String,
    /// Optional country code for country-specific adapters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl GatewayRoute {
    /// Builds and validates a gateway route.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] when `route` or
    /// `profile` is blank. Returns [`ReconcileError::InvalidCountryCode`]
    /// when a country code is present but is not uppercase ISO 3166-1 alpha-2
    /// shaped text.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::GatewayRoute;
    ///
    /// let route = GatewayRoute::new("peppol", "peppol-bis-3", Some("DE")).unwrap();
    /// assert_eq!(route.route, "peppol");
    /// ```
    pub fn new(
        route: impl Into<String>,
        profile: impl Into<String>,
        country: Option<impl Into<String>>,
    ) -> Result<Self, ReconcileError> {
        let route = route.into();
        let profile = profile.into();
        validate_non_empty(&route, "route")?;
        validate_non_empty(&profile, "profile")?;
        let country = country.map(Into::into);
        if let Some(country) = &country {
            validate_country_code(country)?;
        }
        Ok(Self {
            route,
            profile,
            country,
        })
    }
}

/// Gateway operation kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayOperation {
    /// Submit a new invoice to a gateway.
    Submit,
    /// Poll for the status of a previous submission.
    Poll,
    /// Cancel a previous submission where the gateway supports cancellation.
    Cancel,
    /// Correct or replace a previous submission.
    Correct,
}

impl GatewayOperation {
    /// Returns the stable operation name.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::GatewayOperation;
    ///
    /// assert_eq!(GatewayOperation::Submit.as_str(), "submit");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Poll => "poll",
            Self::Cancel => "cancel",
            Self::Correct => "correct",
        }
    }
}

impl fmt::Display for GatewayOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Normalized gateway receipt status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayStatus {
    /// The gateway accepted the request synchronously.
    Accepted,
    /// The gateway accepted the request for asynchronous processing.
    Pending,
    /// The gateway rejected the request.
    Rejected,
    /// The gateway acknowledged cancellation.
    Cancelled,
    /// The gateway accepted a correction.
    Corrected,
}

/// Canonical InvoiceKit transmission lifecycle state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransmissionBaseState {
    /// Invoice exists but has not passed validation.
    Draft,
    /// Invoice passed deterministic validation.
    Validated,
    /// Invoice payload and evidence material were signed.
    Signed,
    /// Idempotency and outbox slot were reserved before transmission.
    Reserved,
    /// Invoice was sent to a gateway adapter.
    Sent,
    /// Gateway accepted or delivered the invoice.
    Delivered,
    /// Recipient, authority, or partner acknowledged the delivery.
    Acknowledged,
    /// Gateway, authority, or recipient rejected the invoice.
    Rejected,
    /// Final evidence and receipts were archived.
    Archived,
}

impl TransmissionBaseState {
    /// Returns the stable serialized state name.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::TransmissionBaseState;
    ///
    /// assert_eq!(TransmissionBaseState::Delivered.as_str(), "delivered");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Validated => "validated",
            Self::Signed => "signed",
            Self::Reserved => "reserved",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Acknowledged => "acknowledged",
            Self::Rejected => "rejected",
            Self::Archived => "archived",
        }
    }

    /// Returns true when `next` is a valid base-state transition.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::TransmissionBaseState;
    ///
    /// assert!(TransmissionBaseState::Sent.can_transition_to(TransmissionBaseState::Delivered));
    /// assert!(!TransmissionBaseState::Draft.can_transition_to(TransmissionBaseState::Sent));
    /// ```
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Validated)
                | (Self::Validated, Self::Signed)
                | (Self::Signed, Self::Reserved)
                | (Self::Reserved, Self::Sent)
                | (Self::Sent, Self::Delivered | Self::Rejected)
                | (Self::Delivered, Self::Acknowledged | Self::Rejected)
                | (Self::Acknowledged | Self::Rejected, Self::Archived)
        )
    }

    /// Returns true when no further state transition is valid.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::TransmissionBaseState;
    ///
    /// assert!(TransmissionBaseState::Archived.is_terminal());
    /// assert!(!TransmissionBaseState::Sent.is_terminal());
    /// ```
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Archived)
    }
}

impl fmt::Display for TransmissionBaseState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Country or network-specific state layered on top of the canonical state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CountrySubState {
    /// Gateway, authority, or network namespace, such as `KSEF`, `SDI`, or
    /// `ZATCA`.
    pub system: String,
    /// Stable country-system state code.
    pub code: String,
    /// Human-readable label for logs and support views.
    pub label: String,
}

impl CountrySubState {
    /// Builds a validated country-specific sub-state.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] for blank fields, or
    /// [`ReconcileError::InvalidIdentifier`] when a field contains control
    /// characters or identifier whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::CountrySubState;
    ///
    /// let substate = CountrySubState::new("KSEF", "session_opened", "KSeF session opened").unwrap();
    /// assert_eq!(substate.system, "KSEF");
    /// ```
    pub fn new(
        system: impl Into<String>,
        code: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let system = system.into();
        let code = code.into();
        let label = label.into();
        validate_identifier(&system, "country_substate.system")?;
        validate_identifier(&code, "country_substate.code")?;
        validate_text(&label, "country_substate.label")?;
        Ok(Self {
            system,
            code,
            label,
        })
    }
}

/// Full transmission state, including optional country-specific sub-state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionState {
    /// Canonical state used by the outbox and evidence bundle.
    pub base: TransmissionBaseState,
    /// Optional country, authority, or network state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_substate: Option<CountrySubState>,
}

impl TransmissionState {
    /// Builds a state with no country-specific sub-state.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState};
    ///
    /// let state = TransmissionState::new(TransmissionBaseState::Draft);
    /// assert_eq!(state.base, TransmissionBaseState::Draft);
    /// ```
    #[must_use]
    pub const fn new(base: TransmissionBaseState) -> Self {
        Self {
            base,
            country_substate: None,
        }
    }

    /// Attaches a country-specific sub-state.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{CountrySubState, TransmissionBaseState, TransmissionState};
    ///
    /// let state = TransmissionState::new(TransmissionBaseState::Sent)
    ///     .with_country_substate(CountrySubState::new("SDI", "RC", "Ricevuta consegna").unwrap());
    /// assert_eq!(state.country_substate.unwrap().system, "SDI");
    /// ```
    #[must_use]
    pub fn with_country_substate(mut self, country_substate: CountrySubState) -> Self {
        self.country_substate = Some(country_substate);
        self
    }

    /// Validates and builds a transition to `next`.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidTransition`] when the base-state move
    /// is not allowed, or [`ReconcileError::MissingRequiredField`] when
    /// `reason` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState};
    ///
    /// let transition = TransmissionState::new(TransmissionBaseState::Draft)
    ///     .transition_to(
    ///         TransmissionState::new(TransmissionBaseState::Validated),
    ///         "validation passed",
    ///     )
    ///     .unwrap();
    /// assert_eq!(transition.to.base, TransmissionBaseState::Validated);
    /// ```
    pub fn transition_to(
        self,
        next: Self,
        reason: impl Into<String>,
    ) -> Result<TransmissionTransition, ReconcileError> {
        TransmissionTransition::new(self, next, reason)
    }
}

/// Validated transition between two transmission states.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionTransition {
    /// State before the transition.
    pub from: TransmissionState,
    /// State after the transition.
    pub to: TransmissionState,
    /// Operator or system reason for the transition.
    pub reason: String,
}

impl TransmissionTransition {
    /// Builds and validates a state transition.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidTransition`] when the base-state move
    /// is not allowed, or [`ReconcileError::MissingRequiredField`] when
    /// `reason` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     TransmissionBaseState, TransmissionState, TransmissionTransition,
    /// };
    ///
    /// let transition = TransmissionTransition::new(
    ///     TransmissionState::new(TransmissionBaseState::Sent),
    ///     TransmissionState::new(TransmissionBaseState::Delivered),
    ///     "gateway accepted",
    /// )
    /// .unwrap();
    /// assert_eq!(transition.from.base, TransmissionBaseState::Sent);
    /// ```
    pub fn new(
        from: TransmissionState,
        to: TransmissionState,
        reason: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let reason = reason.into();
        validate_non_empty(&reason, "transition.reason")?;
        if !from.base.can_transition_to(to.base) {
            return Err(ReconcileError::InvalidTransition {
                from: from.base,
                to: to.base,
            });
        }
        Ok(Self { from, to, reason })
    }
}

/// Receipt normalized from a gateway response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayReceipt {
    /// Operation that produced this receipt.
    pub operation: GatewayOperation,
    /// Context carried into the gateway call.
    pub context: GatewayContext,
    /// Stable gateway submission handle.
    pub submission_id: GatewaySubmissionId,
    /// Normalized receipt status.
    pub status: GatewayStatus,
    /// Gateway-specific reference number, if one was returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_reference: Option<String>,
    /// Hash of the raw receipt bytes, if raw bytes were captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_receipt_hash: Option<String>,
    /// Timestamp supplied by the adapter in RFC 3339 or gateway-native form.
    pub received_at: String,
    /// Human-readable normalized receipt details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl GatewayReceipt {
    /// Builds a normalized gateway receipt.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] when `received_at` is
    /// blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, GatewayOperation, GatewayReceipt,
    ///     GatewayStatus, GatewaySubmissionId, IdempotencyKey, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// let receipt = GatewayReceipt::new(
    ///     GatewayOperation::Submit,
    ///     context,
    ///     GatewaySubmissionId::new("sub_001").unwrap(),
    ///     GatewayStatus::Accepted,
    ///     "2026-05-26T18:00:00Z",
    /// )
    /// .unwrap();
    /// assert_eq!(receipt.status, GatewayStatus::Accepted);
    /// ```
    pub fn new(
        operation: GatewayOperation,
        context: GatewayContext,
        submission_id: GatewaySubmissionId,
        status: GatewayStatus,
        received_at: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let received_at = received_at.into();
        validate_non_empty(&received_at, "received_at")?;
        Ok(Self {
            operation,
            context,
            submission_id,
            status,
            gateway_reference: None,
            raw_receipt_hash: None,
            received_at,
            detail: None,
        })
    }
}

/// Submit operation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubmitRequest {
    /// Required per-attempt context.
    pub context: GatewayContext,
    /// Route selected by routing policy.
    pub route: GatewayRoute,
    /// Validated invoice document being submitted.
    pub document: CommercialDocument,
}

impl SubmitRequest {
    /// Builds a submit request and verifies document context propagation.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidDocument`] when the IR document does
    /// not validate, or [`ReconcileError::ContextMismatch`] when the document
    /// metadata tenant or trace ID differs from the gateway context.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_ir::CommercialDocument;
    /// # use invoicekit_reconcile::{GatewayContext, GatewayRoute, SubmitRequest};
    /// # fn build_context() -> GatewayContext { loop {} }
    /// # fn build_document() -> CommercialDocument { loop {} }
    /// let request = SubmitRequest::new(
    ///     build_context(),
    ///     GatewayRoute::new("peppol", "peppol-bis-3", Some("DE")).unwrap(),
    ///     build_document(),
    /// );
    /// assert!(request.is_ok());
    /// ```
    pub fn new(
        context: GatewayContext,
        route: GatewayRoute,
        document: CommercialDocument,
    ) -> Result<Self, ReconcileError> {
        document
            .validate()
            .map_err(ReconcileError::InvalidDocument)?;
        ensure_document_context(&context, &document)?;
        Ok(Self {
            context,
            route,
            document,
        })
    }
}

/// Poll operation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PollRequest {
    /// Required per-attempt context.
    pub context: GatewayContext,
    /// Submission to poll.
    pub submission_id: GatewaySubmissionId,
}

impl PollRequest {
    /// Builds a poll request.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, GatewaySubmissionId, IdempotencyKey,
    ///     PollRequest, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_poll_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_poll_001").unwrap(),
    /// );
    /// let request = PollRequest::new(context, GatewaySubmissionId::new("sub_001").unwrap());
    /// assert_eq!(request.submission_id.as_str(), "sub_001");
    /// ```
    #[must_use]
    pub const fn new(context: GatewayContext, submission_id: GatewaySubmissionId) -> Self {
        Self {
            context,
            submission_id,
        }
    }
}

/// Cancel operation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelRequest {
    /// Required per-attempt context.
    pub context: GatewayContext,
    /// Submission to cancel.
    pub submission_id: GatewaySubmissionId,
    /// Gateway-facing cancellation reason.
    pub reason: String,
}

impl CancelRequest {
    /// Builds a cancel request.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] when `reason` is
    /// blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CancelRequest, GatewayAttemptId, GatewayContext, GatewaySubmissionId,
    ///     IdempotencyKey, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_cancel_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_cancel_001").unwrap(),
    /// );
    /// let request = CancelRequest::new(
    ///     context,
    ///     GatewaySubmissionId::new("sub_001").unwrap(),
    ///     "customer requested cancellation",
    /// )
    /// .unwrap();
    /// assert!(request.reason.contains("customer"));
    /// ```
    pub fn new(
        context: GatewayContext,
        submission_id: GatewaySubmissionId,
        reason: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let reason = reason.into();
        validate_non_empty(&reason, "reason")?;
        Ok(Self {
            context,
            submission_id,
            reason,
        })
    }
}

/// Correct operation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrectRequest {
    /// Required per-attempt context.
    pub context: GatewayContext,
    /// Submission being corrected.
    pub submission_id: GatewaySubmissionId,
    /// Replacement or corrective invoice document.
    pub corrected_document: CommercialDocument,
    /// Gateway-facing correction reason.
    pub reason: String,
}

impl CorrectRequest {
    /// Builds a correction request and verifies document context propagation.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] when `reason` is
    /// blank, [`ReconcileError::InvalidDocument`] when the IR document does not
    /// validate, or [`ReconcileError::ContextMismatch`] when the document
    /// metadata tenant or trace ID differs from the gateway context.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use invoicekit_ir::CommercialDocument;
    /// # use invoicekit_reconcile::{CorrectRequest, GatewayContext, GatewaySubmissionId};
    /// # fn build_context() -> GatewayContext { loop {} }
    /// # fn build_document() -> CommercialDocument { loop {} }
    /// let request = CorrectRequest::new(
    ///     build_context(),
    ///     GatewaySubmissionId::new("sub_001").unwrap(),
    ///     build_document(),
    ///     "corrected buyer reference",
    /// );
    /// assert!(request.is_ok());
    /// ```
    pub fn new(
        context: GatewayContext,
        submission_id: GatewaySubmissionId,
        corrected_document: CommercialDocument,
        reason: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let reason = reason.into();
        validate_non_empty(&reason, "reason")?;
        corrected_document
            .validate()
            .map_err(ReconcileError::InvalidDocument)?;
        ensure_document_context(&context, &corrected_document)?;
        Ok(Self {
            context,
            submission_id,
            corrected_document,
            reason,
        })
    }
}

/// Normalized categories for gateway failures.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayErrorKind {
    /// Credentials, token, API key, or account permission failed.
    AuthFailure,
    /// Gateway throttled the call.
    RateLimited,
    /// Gateway returned a receipt that could not be parsed or authenticated.
    MalformedReceipt,
    /// Gateway was down, under maintenance, or returned a maintenance page.
    GatewayMaintenance,
    /// Client certificate, signing certificate, or certificate chain failed.
    CertificateRejected,
    /// Gateway reported that the same invoice was already submitted.
    DuplicateSubmission,
    /// Request timed out before a reliable gateway receipt was available.
    Timeout,
    /// Network transport failed before a gateway-level response existed.
    NetworkFailure,
    /// Gateway rejected the invoice payload for business or validation reasons.
    Rejected,
    /// Gateway did not know the requested submission.
    NotFound,
    /// Adapter detected an invalid request before calling the gateway.
    InvalidRequest,
    /// Gateway or adapter does not support the requested operation.
    UnsupportedOperation,
    /// Partner access point returned a provider-specific error.
    PartnerError,
    /// Gateway response was syntactically valid but semantically unexpected.
    UnexpectedResponse,
}

impl GatewayErrorKind {
    /// Returns the stable error-kind name.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::GatewayErrorKind;
    ///
    /// assert_eq!(GatewayErrorKind::RateLimited.as_str(), "rate_limited");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthFailure => "auth_failure",
            Self::RateLimited => "rate_limited",
            Self::MalformedReceipt => "malformed_receipt",
            Self::GatewayMaintenance => "gateway_maintenance",
            Self::CertificateRejected => "certificate_rejected",
            Self::DuplicateSubmission => "duplicate_submission",
            Self::Timeout => "timeout",
            Self::NetworkFailure => "network_failure",
            Self::Rejected => "rejected",
            Self::NotFound => "not_found",
            Self::InvalidRequest => "invalid_request",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::PartnerError => "partner_error",
            Self::UnexpectedResponse => "unexpected_response",
        }
    }
}

impl fmt::Display for GatewayErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by gateway adapters after normalizing gateway-specific
/// failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{operation} failed with {kind}: {message}; remediation: {remediation}")]
pub struct GatewayError {
    /// Normalized error kind.
    pub kind: GatewayErrorKind,
    /// Operation that failed.
    pub operation: GatewayOperation,
    /// Human-readable diagnostic message.
    pub message: String,
    /// Human-readable remediation hint.
    pub remediation: String,
    /// Gateway or partner-specific code, if one was present.
    pub gateway_code: Option<String>,
    /// Gateway submission handle, if the failure is tied to an existing
    /// submission.
    pub submission_id: Option<GatewaySubmissionId>,
    /// Retry delay in seconds, when the gateway provided one or the adapter
    /// computed one.
    pub retry_after_seconds: Option<u64>,
}

impl GatewayError {
    /// Builds a normalized gateway error.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayError, GatewayErrorKind, GatewayOperation,
    /// };
    ///
    /// let error = GatewayError::new(
    ///     GatewayErrorKind::RateLimited,
    ///     GatewayOperation::Submit,
    ///     "gateway quota exceeded",
    ///     "retry after the returned backoff window",
    /// );
    /// assert_eq!(error.kind, GatewayErrorKind::RateLimited);
    /// ```
    #[must_use]
    pub fn new(
        kind: GatewayErrorKind,
        operation: GatewayOperation,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            operation,
            message: message.into(),
            remediation: remediation.into(),
            gateway_code: None,
            submission_id: None,
            retry_after_seconds: None,
        }
    }

    /// Attaches a gateway-specific error code.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{GatewayError, GatewayErrorKind, GatewayOperation};
    ///
    /// let error = GatewayError::new(
    ///     GatewayErrorKind::PartnerError,
    ///     GatewayOperation::Poll,
    ///     "partner returned an opaque failure",
    ///     "inspect the partner support incident",
    /// )
    /// .with_gateway_code("PARTNER-42");
    /// assert_eq!(error.gateway_code.as_deref(), Some("PARTNER-42"));
    /// ```
    #[must_use]
    pub fn with_gateway_code(mut self, gateway_code: impl Into<String>) -> Self {
        self.gateway_code = Some(gateway_code.into());
        self
    }

    /// Attaches a submission handle to the error.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayError, GatewayErrorKind, GatewayOperation, GatewaySubmissionId,
    /// };
    ///
    /// let error = GatewayError::new(
    ///     GatewayErrorKind::DuplicateSubmission,
    ///     GatewayOperation::Submit,
    ///     "gateway already has this invoice",
    ///     "poll the existing submission instead of resubmitting",
    /// )
    /// .with_submission_id(GatewaySubmissionId::new("sub_001").unwrap());
    /// assert_eq!(error.submission_id.unwrap().as_str(), "sub_001");
    /// ```
    #[must_use]
    pub fn with_submission_id(mut self, submission_id: GatewaySubmissionId) -> Self {
        self.submission_id = Some(submission_id);
        self
    }

    /// Attaches retry guidance in seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{GatewayError, GatewayErrorKind, GatewayOperation};
    ///
    /// let error = GatewayError::new(
    ///     GatewayErrorKind::RateLimited,
    ///     GatewayOperation::Submit,
    ///     "too many requests",
    ///     "retry later",
    /// )
    /// .with_retry_after_seconds(60);
    /// assert_eq!(error.retry_after_seconds, Some(60));
    /// ```
    #[must_use]
    pub const fn with_retry_after_seconds(mut self, seconds: u64) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }
}

/// Gateway adapter interface implemented by partner access points, native
/// protocol adapters, and the cassette-backed mock gateway.
pub trait GatewayAdapter: Send + Sync {
    /// Submits a new invoice to the gateway.
    fn submit(&self, request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt>;

    /// Polls a prior gateway submission.
    fn poll(&self, request: PollRequest) -> GatewayFuture<'_, GatewayReceipt>;

    /// Cancels a prior gateway submission where the gateway supports it.
    fn cancel(&self, request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt>;

    /// Corrects or replaces a prior gateway submission.
    fn correct(&self, request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt>;
}

/// Errors produced by reconciliation and gateway-contract constructors.
#[derive(Debug, Error)]
pub enum ReconcileError {
    /// A required string field was blank.
    #[error("missing required field `{0}`")]
    MissingRequiredField(&'static str),
    /// An identifier had leading or trailing whitespace or control characters.
    #[error("invalid identifier `{field}`: `{value}`")]
    InvalidIdentifier {
        /// Field that failed validation.
        field: &'static str,
        /// Rejected value.
        value: String,
    },
    /// A transmission state transition was not allowed.
    #[error("invalid transmission transition from `{from}` to `{to}`")]
    InvalidTransition {
        /// State before the rejected transition.
        from: TransmissionBaseState,
        /// State after the rejected transition.
        to: TransmissionBaseState,
    },
    /// A gateway country code was not uppercase ISO 3166-1 alpha-2 shaped text.
    #[error("invalid ISO 3166-1 alpha-2 gateway country code `{0}`")]
    InvalidCountryCode(String),
    /// Gateway context did not match the invoice document metadata.
    #[error("gateway context `{field}` mismatch: expected `{expected}`, got `{actual}`")]
    ContextMismatch {
        /// Field that differed.
        field: &'static str,
        /// Value carried by the gateway context.
        expected: String,
        /// Value carried by the document metadata.
        actual: String,
    },
    /// The invoice document failed IR validation.
    #[error("invalid invoice document: {0}")]
    InvalidDocument(invoicekit_ir::IrError),
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
/// assert_eq!(invoicekit_reconcile::crate_name(), "invoicekit-reconcile");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-reconcile"
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), ReconcileError> {
    validate_non_empty(value, field)?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(ReconcileError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), ReconcileError> {
    if value.trim().is_empty() {
        Err(ReconcileError::MissingRequiredField(field))
    } else {
        Ok(())
    }
}

fn validate_text(value: &str, field: &'static str) -> Result<(), ReconcileError> {
    validate_non_empty(value, field)?;
    if value.chars().any(char::is_control) {
        return Err(ReconcileError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_country_code(value: &str) -> Result<(), ReconcileError> {
    if value.len() == 2 && value.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ReconcileError::InvalidCountryCode(value.to_owned()))
    }
}

fn ensure_document_context(
    context: &GatewayContext,
    document: &CommercialDocument,
) -> Result<(), ReconcileError> {
    ensure_same(
        "tenant_id",
        context.tenant_id.as_str(),
        &document.meta.tenant_id,
    )?;
    ensure_same(
        "trace_id",
        context.trace_id.as_str(),
        &document.meta.trace_id,
    )
}

fn ensure_same(field: &'static str, expected: &str, actual: &str) -> Result<(), ReconcileError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ReconcileError::ContextMismatch {
            field,
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Mutex;
    use std::task::{Context, Poll};

    use futures_task::noop_waker_ref;
    use invoicekit_ir::CommercialDocument;
    use serde_json::{json, Value};

    use super::{
        crate_name, CancelRequest, CorrectRequest, CountrySubState, GatewayAdapter,
        GatewayAttemptId, GatewayContext, GatewayError, GatewayErrorKind, GatewayFuture,
        GatewayOperation, GatewayReceipt, GatewayRoute, GatewayStatus, GatewaySubmissionId,
        IdempotencyKey, PollRequest, ReconcileError, SubmitRequest, TenantId, TraceId,
        TransmissionBaseState, TransmissionState,
    };

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-reconcile");
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
    fn identifier_constructors_reject_blank_or_unsafe_values() {
        assert!(matches!(
            TenantId::new(" "),
            Err(ReconcileError::MissingRequiredField("tenant_id"))
        ));
        assert!(matches!(
            TraceId::new(" trace_123"),
            Err(ReconcileError::InvalidIdentifier {
                field: "trace_id",
                ..
            })
        ));
        assert!(matches!(
            IdempotencyKey::new("idem\n123"),
            Err(ReconcileError::InvalidIdentifier {
                field: "idempotency_key",
                ..
            })
        ));
    }

    #[test]
    fn route_validation_rejects_blank_profile_and_bad_country() {
        assert!(matches!(
            GatewayRoute::new("peppol", "", Some("DE")),
            Err(ReconcileError::MissingRequiredField("profile"))
        ));
        assert!(matches!(
            GatewayRoute::new("peppol", "peppol-bis-3", Some("de")),
            Err(ReconcileError::InvalidCountryCode(country)) if country == "de"
        ));
    }

    #[test]
    fn transmission_base_state_allows_canonical_happy_path() {
        let transitions = [
            (
                TransmissionBaseState::Draft,
                TransmissionBaseState::Validated,
            ),
            (
                TransmissionBaseState::Validated,
                TransmissionBaseState::Signed,
            ),
            (
                TransmissionBaseState::Signed,
                TransmissionBaseState::Reserved,
            ),
            (TransmissionBaseState::Reserved, TransmissionBaseState::Sent),
            (
                TransmissionBaseState::Sent,
                TransmissionBaseState::Delivered,
            ),
            (
                TransmissionBaseState::Delivered,
                TransmissionBaseState::Acknowledged,
            ),
            (
                TransmissionBaseState::Acknowledged,
                TransmissionBaseState::Archived,
            ),
        ];

        for (from, to) in transitions {
            let transition = TransmissionState::new(from)
                .transition_to(TransmissionState::new(to), "canonical transition")
                .unwrap();

            assert_eq!(transition.from.base, from);
            assert_eq!(transition.to.base, to);
        }
    }

    #[test]
    fn rejected_transmission_can_be_archived() {
        let transition = TransmissionState::new(TransmissionBaseState::Sent)
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Rejected),
                "gateway rejection",
            )
            .unwrap()
            .to
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Archived),
                "archive rejection evidence",
            )
            .unwrap();

        assert_eq!(transition.from.base, TransmissionBaseState::Rejected);
        assert_eq!(transition.to.base, TransmissionBaseState::Archived);
    }

    #[test]
    fn invalid_transition_returns_typed_error() {
        let err = TransmissionState::new(TransmissionBaseState::Draft)
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Sent),
                "skip validation",
            )
            .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::InvalidTransition {
                from: TransmissionBaseState::Draft,
                to: TransmissionBaseState::Sent,
            }
        ));
    }

    #[test]
    fn archived_state_is_terminal() {
        assert!(TransmissionBaseState::Archived.is_terminal());
        assert!(!TransmissionBaseState::Sent.is_terminal());

        let err = TransmissionState::new(TransmissionBaseState::Archived)
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Sent),
                "attempt reopen",
            )
            .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::InvalidTransition {
                from: TransmissionBaseState::Archived,
                to: TransmissionBaseState::Sent,
            }
        ));
    }

    #[test]
    fn country_substate_hook_covers_ksef_sdi_and_zatca_cases() {
        let ksef_sent = TransmissionState::new(TransmissionBaseState::Sent)
            .with_country_substate(country_substate("KSEF", "session_opened"));
        let ksef_delivered = TransmissionState::new(TransmissionBaseState::Delivered)
            .with_country_substate(country_substate("KSEF", "upo_received"));
        let ksef_transition = ksef_sent
            .transition_to(ksef_delivered, "KSeF UPO received")
            .unwrap();
        assert_eq!(
            ksef_transition.to.country_substate.as_ref().unwrap().code,
            "upo_received"
        );

        let sdi_transition = TransmissionState::new(TransmissionBaseState::Sent)
            .with_country_substate(country_substate("SDI", "presa_in_carico"))
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Delivered)
                    .with_country_substate(country_substate("SDI", "ricevuta_consegna")),
                "SDI delivery receipt",
            )
            .unwrap();
        assert_eq!(
            sdi_transition.to.country_substate.as_ref().unwrap().system,
            "SDI"
        );

        let zatca_transition = TransmissionState::new(TransmissionBaseState::Delivered)
            .with_country_substate(country_substate("ZATCA", "cleared"))
            .transition_to(
                TransmissionState::new(TransmissionBaseState::Acknowledged)
                    .with_country_substate(country_substate("ZATCA", "reported")),
                "ZATCA clearance acknowledged",
            )
            .unwrap();
        assert_eq!(
            zatca_transition.to.country_substate.as_ref().unwrap().code,
            "reported"
        );
    }

    #[test]
    fn country_substate_rejects_blank_or_control_values() {
        assert!(matches!(
            CountrySubState::new("KSEF", "", "KSeF state"),
            Err(ReconcileError::MissingRequiredField(
                "country_substate.code"
            ))
        ));
        assert!(matches!(
            CountrySubState::new("SDI", "ricevuta", "bad\nlabel"),
            Err(ReconcileError::InvalidIdentifier {
                field: "country_substate.label",
                ..
            })
        ));
    }

    #[test]
    fn submit_request_requires_document_context_to_match_gateway_context() {
        let context = gateway_context();
        let document = synthetic_document();
        let route = gateway_route();

        let request = SubmitRequest::new(context, route, document).unwrap();

        assert_eq!(request.context.tenant_id.as_str(), "tenant_123");
        assert_eq!(request.document.meta.trace_id, "trace_abc");
    }

    #[test]
    fn submit_request_rejects_mismatched_tenant() {
        let context = GatewayContext::new(
            TenantId::new("tenant_other").unwrap(),
            TraceId::new("trace_abc").unwrap(),
            IdempotencyKey::new("idem_invoice_123").unwrap(),
            GatewayAttemptId::new("attempt_001").unwrap(),
        );

        let err = SubmitRequest::new(context, gateway_route(), synthetic_document()).unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::ContextMismatch {
                field: "tenant_id",
                expected,
                actual,
            } if expected == "tenant_other" && actual == "tenant_123"
        ));
    }

    #[test]
    fn correct_request_validates_reason_and_document_context() {
        let err = CorrectRequest::new(
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            synthetic_document(),
            " ",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::MissingRequiredField("reason")
        ));
    }

    #[test]
    fn cancel_request_validates_reason() {
        let err = CancelRequest::new(
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            " ",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::MissingRequiredField("reason")
        ));
    }

    #[test]
    fn gateway_receipt_rejects_blank_received_at() {
        let err = GatewayReceipt::new(
            GatewayOperation::Submit,
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            GatewayStatus::Accepted,
            " ",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::MissingRequiredField("received_at")
        ));
    }

    #[test]
    fn gateway_error_carries_normalized_remediation_and_retry_metadata() {
        let error = GatewayError::new(
            GatewayErrorKind::RateLimited,
            GatewayOperation::Submit,
            "gateway quota exceeded",
            "retry after the returned backoff window",
        )
        .with_gateway_code("429")
        .with_submission_id(GatewaySubmissionId::new("sub_001").unwrap())
        .with_retry_after_seconds(120);

        assert_eq!(error.kind, GatewayErrorKind::RateLimited);
        assert_eq!(error.gateway_code.as_deref(), Some("429"));
        assert_eq!(error.retry_after_seconds, Some(120));
        assert!(error.to_string().contains("remediation"));
    }

    #[test]
    fn gateway_adapter_trait_dispatches_all_successful_operations() {
        let adapter = ScriptedGatewayAdapter::with_outcomes([
            Ok(receipt(GatewayOperation::Submit, GatewayStatus::Pending)),
            Ok(receipt(GatewayOperation::Poll, GatewayStatus::Accepted)),
            Ok(receipt(GatewayOperation::Cancel, GatewayStatus::Cancelled)),
            Ok(receipt(GatewayOperation::Correct, GatewayStatus::Corrected)),
        ]);

        let submit = block_on_ready(adapter.submit(submit_request()));
        let poll = block_on_ready(adapter.poll(poll_request()));
        let cancel = block_on_ready(adapter.cancel(cancel_request()));
        let correct = block_on_ready(adapter.correct(correct_request()));

        assert_eq!(submit.unwrap().status, GatewayStatus::Pending);
        assert_eq!(poll.unwrap().operation, GatewayOperation::Poll);
        assert_eq!(cancel.unwrap().status, GatewayStatus::Cancelled);
        assert_eq!(correct.unwrap().status, GatewayStatus::Corrected);
    }

    #[test]
    fn gateway_adapter_trait_normalizes_required_failure_modes() {
        let failure_modes = [
            GatewayErrorKind::AuthFailure,
            GatewayErrorKind::RateLimited,
            GatewayErrorKind::MalformedReceipt,
            GatewayErrorKind::GatewayMaintenance,
            GatewayErrorKind::CertificateRejected,
            GatewayErrorKind::DuplicateSubmission,
            GatewayErrorKind::NotFound,
            GatewayErrorKind::UnsupportedOperation,
        ];

        for kind in failure_modes {
            let adapter = ScriptedGatewayAdapter::with_outcomes([Err(error(kind))]);
            let err = block_on_ready(adapter.submit(submit_request())).unwrap_err();

            assert_eq!(err.kind, kind);
            assert!(!err.remediation.is_empty());
        }
    }

    struct ScriptedGatewayAdapter {
        outcomes: Mutex<VecDeque<Result<GatewayReceipt, GatewayError>>>,
    }

    impl ScriptedGatewayAdapter {
        fn with_outcomes(
            outcomes: impl IntoIterator<Item = Result<GatewayReceipt, GatewayError>>,
        ) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into_iter().collect()),
            }
        }

        fn next_outcome(&self) -> Result<GatewayReceipt, GatewayError> {
            self.outcomes
                .lock()
                .expect("test adapter lock is not poisoned")
                .pop_front()
                .expect("test adapter configured with at least one outcome")
        }
    }

    impl GatewayAdapter for ScriptedGatewayAdapter {
        fn submit(&self, _request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(self.next_outcome()))
        }

        fn poll(&self, _request: PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(self.next_outcome()))
        }

        fn cancel(&self, _request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(self.next_outcome()))
        }

        fn correct(&self, _request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
            Box::pin(std::future::ready(self.next_outcome()))
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

    fn gateway_context() -> GatewayContext {
        GatewayContext::new(
            TenantId::new("tenant_123").unwrap(),
            TraceId::new("trace_abc").unwrap(),
            IdempotencyKey::new("idem_invoice_123").unwrap(),
            GatewayAttemptId::new("attempt_001").unwrap(),
        )
    }

    fn gateway_route() -> GatewayRoute {
        GatewayRoute::new("peppol", "peppol-bis-3", Some("DE")).unwrap()
    }

    fn submit_request() -> SubmitRequest {
        SubmitRequest::new(gateway_context(), gateway_route(), synthetic_document()).unwrap()
    }

    fn poll_request() -> PollRequest {
        PollRequest::new(
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
        )
    }

    fn cancel_request() -> CancelRequest {
        CancelRequest::new(
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            "customer requested cancellation",
        )
        .unwrap()
    }

    fn correct_request() -> CorrectRequest {
        CorrectRequest::new(
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            synthetic_document(),
            "correct buyer reference",
        )
        .unwrap()
    }

    fn receipt(operation: GatewayOperation, status: GatewayStatus) -> GatewayReceipt {
        GatewayReceipt::new(
            operation,
            gateway_context(),
            GatewaySubmissionId::new("sub_001").unwrap(),
            status,
            "2026-05-26T18:00:00Z",
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

    fn country_substate(system: &str, code: &str) -> CountrySubState {
        CountrySubState::new(system, code, format!("{system} {code}")).unwrap()
    }

    fn synthetic_document() -> CommercialDocument {
        CommercialDocument::try_from_value(synthetic_document_json()).unwrap()
    }

    fn synthetic_document_json() -> Value {
        json!({
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
        })
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
