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

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use invoicekit_ir::{CommercialDocument, Party};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod outbox;
pub mod redact;
mod worker;

pub use outbox::{
    all_outbox_migrations, outbox_migration, DatabaseDialect, DeadLetterRecord, OutboxEnvelope,
    OutboxMigration, OutboxState, RetryDecision, RetryPolicy, OUTBOX_BEAD_ID,
};
pub use redact::{redact_for_support, RedactedBundle, RedactionReport, REDACT_BEAD_ID};
pub use worker::{
    CircuitBreakerPolicy, GatewayRateLimit, TransmissionJob, TransmissionWorker,
    TransmissionWorkerConfig, TransmissionWorkerLogEvent, TransmissionWorkerOutcomeKind,
    TransmissionWorkerResult, TRANSMISSION_WORKER_BEAD_ID,
};

/// Boxed future returned by gateway adapter operations.
///
/// The boxed shape keeps [`GatewayAdapter`] object-safe, so the transmission
/// worker can store partner, native, and mock adapters behind one trait object.
pub type GatewayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, GatewayError>> + Send + 'a>>;

/// BLAKE3 hash returned by [`fingerprint`].
///
/// This is the stable 32-byte digest used by reconciliation and evidence
/// lookups. Use [`blake3::Hash::to_hex`] when a textual representation is
/// needed.
pub type Blake3Hash = blake3::Hash;

const FINGERPRINT_DOMAIN: &[u8] = b"invoicekit:reconcile:fingerprint:v1";

/// Computes the deterministic invoice fingerprint used for deduplication.
///
/// The fingerprint follows PLAN.md section 4.6:
/// `blake3(supplier_VAT || customer_VAT || issue_date || document_number ||
/// total_amount || currency)`. InvoiceKit prepends a domain tag and serializes
/// each component with a length prefix before hashing, so adjacent fields cannot
/// collide by concatenation ambiguity. `total_amount` is the normalized payable
/// amount from [`invoicekit_ir::MonetaryTotal`].
///
/// If a party has multiple VAT tax IDs, the first tax ID whose scheme is `vat`
/// (case-insensitive) is used. If no VAT ID is present, the component is the
/// empty string; this preserves the pure no-error function shape required by
/// the reconciliation contract while keeping the missing-value choice explicit.
///
/// Test vector committed in T-022:
///
/// * supplier VAT: `DE123456789`
/// * customer VAT: `FR123456789`
/// * issue date: `2026-05-26`
/// * document number: `INV-2026-0001`
/// * total amount: `119`
/// * currency: `EUR`
/// * fingerprint hex: `437ccffe5449042844eef1adb2181c7e9bfed6b097810145189c1f872ca58bde`
///
/// # Examples
///
/// ```
/// use invoicekit_ir::CommercialDocument;
/// use invoicekit_reconcile::fingerprint;
/// use serde_json::json;
///
/// let document = CommercialDocument::try_from_value(json!({
///     "schema_version": "1.0",
///     "id": "doc_fingerprint_vector",
///     "document_type": "invoice",
///     "issue_date": "2026-05-26",
///     "due_date": "2026-06-25",
///     "document_number": "INV-2026-0001",
///     "currency": "EUR",
///     "supplier": {
///         "id": "supplier-fingerprint",
///         "name": "InvoiceKit GmbH",
///         "tax_ids": [{ "scheme": "vat", "value": "DE123456789" }],
///         "address": {
///             "lines": ["Main Street 1"],
///             "city": "Sample City",
///             "postal_code": "10115",
///             "country": "DE"
///         }
///     },
///     "customer": {
///         "id": "customer-fingerprint",
///         "name": "ACME SAS",
///         "tax_ids": [{ "scheme": "vat", "value": "FR123456789" }],
///         "address": {
///             "lines": ["Main Street 1"],
///             "city": "Sample City",
///             "postal_code": "10115",
///             "country": "FR"
///         }
///     },
///     "lines": [{
///         "id": "1",
///         "description": "Validation subscription",
///         "quantity": "1",
///         "unit_code": "EA",
///         "unit_price": "119.00",
///         "line_extension_amount": "119.00",
///         "tax_category": "S"
///     }],
///     "tax_summary": [{
///         "category_code": "S",
///         "taxable_amount": "119.00",
///         "tax_amount": "0.00",
///         "tax_rate": "0.00"
///     }],
///     "monetary_total": {
///         "line_extension_amount": "119.00",
///         "tax_exclusive_amount": "119.00",
///         "tax_inclusive_amount": "119.00",
///         "payable_amount": "119.00"
///     },
///     "meta": {
///         "tenant_id": "tenant_fingerprint",
///         "trace_id": "trace_fingerprint",
///         "source_system": "docs"
///     }
/// }))
/// .unwrap();
///
/// assert_eq!(
///     fingerprint(&document).to_hex().to_string(),
///     "437ccffe5449042844eef1adb2181c7e9bfed6b097810145189c1f872ca58bde"
/// );
/// ```
#[must_use]
pub fn fingerprint(doc: &CommercialDocument) -> Blake3Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(FINGERPRINT_DOMAIN);

    update_fingerprint_field(&mut hasher, party_vat(&doc.supplier));
    update_fingerprint_field(&mut hasher, party_vat(&doc.customer));
    update_fingerprint_field(&mut hasher, doc.issue_date.as_str());
    update_fingerprint_field(&mut hasher, doc.document_number.as_str());
    let total_amount = doc
        .monetary_total
        .payable_amount
        .inner()
        .normalize()
        .to_string();
    update_fingerprint_field(&mut hasher, &total_amount);
    update_fingerprint_field(&mut hasher, doc.currency.as_str());

    hasher.finalize()
}

fn update_fingerprint_field(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn party_vat(party: &Party) -> &str {
    party
        .tax_ids
        .iter()
        .find(|tax_id| tax_id.scheme.eq_ignore_ascii_case("vat"))
        .map_or("", |tax_id| tax_id.value.as_str())
}

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

/// A permitted country-specific sub-state transition layered on a base move.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CountrySubStateTransition {
    /// Gateway, authority, or network namespace this rule applies to.
    pub system: String,
    /// Required base state before the transition.
    pub from_base: TransmissionBaseState,
    /// Required country-specific code before the transition, if one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_code: Option<String>,
    /// Required base state after the transition.
    pub to_base: TransmissionBaseState,
    /// Required country-specific code after the transition.
    pub to_code: String,
}

impl CountrySubStateTransition {
    /// Builds a rule for the first country-specific code in a flow.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidTransition`] if the base states are
    /// not a valid transition, or identifier errors for blank/unsafe values.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{CountrySubStateTransition, TransmissionBaseState};
    ///
    /// let rule = CountrySubStateTransition::initial(
    ///     "KSEF",
    ///     TransmissionBaseState::Reserved,
    ///     TransmissionBaseState::Sent,
    ///     "reserved",
    /// )
    /// .unwrap();
    /// assert_eq!(rule.to_code, "reserved");
    /// ```
    pub fn initial(
        system: impl Into<String>,
        from_base: TransmissionBaseState,
        to_base: TransmissionBaseState,
        to_code: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        Self::build(system, from_base, None, to_base, to_code)
    }

    /// Builds a rule from one country-specific code to the next.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidTransition`] if the base states are
    /// not a valid transition, or identifier errors for blank/unsafe values.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{CountrySubStateTransition, TransmissionBaseState};
    ///
    /// let rule = CountrySubStateTransition::from_code(
    ///     "ZATCA",
    ///     TransmissionBaseState::Delivered,
    ///     "cleared",
    ///     TransmissionBaseState::Acknowledged,
    ///     "reported",
    /// )
    /// .unwrap();
    /// assert_eq!(rule.from_code.as_deref(), Some("cleared"));
    /// ```
    pub fn from_code(
        system: impl Into<String>,
        from_base: TransmissionBaseState,
        from_code: impl Into<String>,
        to_base: TransmissionBaseState,
        to_code: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        Self::build(system, from_base, Some(from_code.into()), to_base, to_code)
    }

    fn build(
        system: impl Into<String>,
        from_base: TransmissionBaseState,
        from_code: Option<String>,
        to_base: TransmissionBaseState,
        to_code: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let system = system.into();
        let to_code = to_code.into();
        validate_identifier(&system, "country_substate_transition.system")?;
        if let Some(from_code) = &from_code {
            validate_identifier(from_code, "country_substate_transition.from_code")?;
        }
        validate_identifier(&to_code, "country_substate_transition.to_code")?;
        if !from_base.can_transition_to(to_base) {
            return Err(ReconcileError::InvalidTransition {
                from: from_base,
                to: to_base,
            });
        }
        Ok(Self {
            system,
            from_base,
            from_code,
            to_base,
            to_code,
        })
    }

    fn matches_states(&self, from: &TransmissionState, to: &TransmissionState) -> bool {
        if self.from_base != from.base || self.to_base != to.base {
            return false;
        }
        let Some(to_substate) = &to.country_substate else {
            return false;
        };
        let system_matches = to_substate.system == self.system;
        let code_matches = to_substate.code == self.to_code;
        if !(system_matches && code_matches) {
            return false;
        }
        match (&self.from_code, &from.country_substate) {
            (None, None) => true,
            (Some(expected), Some(from_substate)) => {
                from_substate.system == self.system && from_substate.code == *expected
            }
            _ => false,
        }
    }

    fn rule_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.system,
            self.from_base,
            self.from_code.as_deref().unwrap_or(""),
            self.to_base,
            self.to_code
        )
    }
}

/// Registry of country-specific sub-state transition rules.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CountrySubStateRegistry {
    transitions: Vec<CountrySubStateTransition>,
}

impl CountrySubStateRegistry {
    /// Builds a registry from country-specific transition rules.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::DuplicateCountrySubStateTransition`] when two
    /// rules describe the same system/from/to edge.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CountrySubStateRegistry, CountrySubStateTransition, TransmissionBaseState,
    /// };
    ///
    /// let registry = CountrySubStateRegistry::new(vec![
    ///     CountrySubStateTransition::initial(
    ///         "SDI",
    ///         TransmissionBaseState::Sent,
    ///         TransmissionBaseState::Delivered,
    ///         "accepted",
    ///     )
    ///     .unwrap(),
    /// ])
    /// .unwrap();
    /// assert_eq!(registry.transitions().len(), 1);
    /// ```
    pub fn new(transitions: Vec<CountrySubStateTransition>) -> Result<Self, ReconcileError> {
        let mut seen = BTreeSet::new();
        for transition in &transitions {
            if !seen.insert(transition.rule_key()) {
                return Err(ReconcileError::DuplicateCountrySubStateTransition {
                    system: transition.system.clone(),
                    from: format_country_rule_endpoint(
                        transition.from_base,
                        transition.from_code.as_deref(),
                    ),
                    to: format_country_rule_endpoint(transition.to_base, Some(&transition.to_code)),
                });
            }
        }
        Ok(Self { transitions })
    }

    /// Returns the configured country-specific transition rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::CountrySubStateRegistry;
    ///
    /// let registry = CountrySubStateRegistry::default();
    /// assert!(registry.transitions().is_empty());
    /// ```
    #[must_use]
    pub fn transitions(&self) -> &[CountrySubStateTransition] {
        &self.transitions
    }

    /// Validates a country-specific sub-state transition.
    ///
    /// Systems with no configured rules remain open extension points. Once a
    /// system has at least one configured rule, transitions for that system
    /// must match a configured rule.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::CountrySubStateSystemMismatch`] when a move
    /// changes country systems, or
    /// [`ReconcileError::InvalidCountrySubStateTransition`] when a configured
    /// system does not allow the requested country code transition.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CountrySubState, CountrySubStateRegistry, CountrySubStateTransition,
    ///     TransmissionBaseState, TransmissionState,
    /// };
    ///
    /// let registry = CountrySubStateRegistry::new(vec![
    ///     CountrySubStateTransition::initial(
    ///         "KSEF",
    ///         TransmissionBaseState::Reserved,
    ///         TransmissionBaseState::Sent,
    ///         "reserved",
    ///     )
    ///     .unwrap(),
    /// ])
    /// .unwrap();
    /// let from = TransmissionState::new(TransmissionBaseState::Reserved);
    /// let to = TransmissionState::new(TransmissionBaseState::Sent)
    ///     .with_country_substate(CountrySubState::new("KSEF", "reserved", "reserved").unwrap());
    /// registry.validate_transition(&from, &to).unwrap();
    /// ```
    pub fn validate_transition(
        &self,
        from: &TransmissionState,
        to: &TransmissionState,
    ) -> Result<(), ReconcileError> {
        let Some(system) = transition_country_system(from, to)? else {
            return Ok(());
        };
        let has_rules = self
            .transitions
            .iter()
            .any(|transition| transition.system == system);
        if !has_rules {
            return Ok(());
        }
        if self
            .transitions
            .iter()
            .any(|transition| transition.matches_states(from, to))
        {
            Ok(())
        } else {
            Err(ReconcileError::InvalidCountrySubStateTransition {
                system,
                from: describe_country_state(from),
                to: describe_country_state(to),
            })
        }
    }
}

/// Executable transmission state machine with transition history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionStateMachine {
    current: TransmissionState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    history: Vec<TransmissionTransition>,
    #[serde(default)]
    country_registry: CountrySubStateRegistry,
}

impl TransmissionStateMachine {
    /// Builds a state machine with no configured country-specific rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState, TransmissionStateMachine};
    ///
    /// let machine = TransmissionStateMachine::new(
    ///     TransmissionState::new(TransmissionBaseState::Draft),
    /// );
    /// assert_eq!(machine.current().base, TransmissionBaseState::Draft);
    /// ```
    #[must_use]
    pub fn new(initial: TransmissionState) -> Self {
        Self {
            current: initial,
            history: Vec::new(),
            country_registry: CountrySubStateRegistry::default(),
        }
    }

    /// Builds a state machine with configured country-specific rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     CountrySubStateRegistry, TransmissionBaseState, TransmissionState, TransmissionStateMachine,
    /// };
    ///
    /// let machine = TransmissionStateMachine::with_country_registry(
    ///     TransmissionState::new(TransmissionBaseState::Draft),
    ///     CountrySubStateRegistry::default(),
    /// );
    /// assert!(machine.history().is_empty());
    /// ```
    #[must_use]
    pub fn with_country_registry(
        initial: TransmissionState,
        country_registry: CountrySubStateRegistry,
    ) -> Self {
        Self {
            current: initial,
            history: Vec::new(),
            country_registry,
        }
    }

    /// Returns the current transmission state.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState, TransmissionStateMachine};
    ///
    /// let machine = TransmissionStateMachine::new(
    ///     TransmissionState::new(TransmissionBaseState::Validated),
    /// );
    /// assert_eq!(machine.current().base, TransmissionBaseState::Validated);
    /// ```
    #[must_use]
    pub const fn current(&self) -> &TransmissionState {
        &self.current
    }

    /// Returns the validated transition history.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState, TransmissionStateMachine};
    ///
    /// let machine = TransmissionStateMachine::new(
    ///     TransmissionState::new(TransmissionBaseState::Draft),
    /// );
    /// assert!(machine.history().is_empty());
    /// ```
    #[must_use]
    pub fn history(&self) -> &[TransmissionTransition] {
        &self.history
    }

    /// Applies a validated transition and records it in history.
    ///
    /// # Errors
    ///
    /// Returns typed transition errors when the base state or configured
    /// country-specific state does not allow the requested move.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{TransmissionBaseState, TransmissionState, TransmissionStateMachine};
    ///
    /// let mut machine = TransmissionStateMachine::new(
    ///     TransmissionState::new(TransmissionBaseState::Draft),
    /// );
    /// machine
    ///     .apply(
    ///         TransmissionState::new(TransmissionBaseState::Validated),
    ///         "validation passed",
    ///     )
    ///     .unwrap();
    /// assert_eq!(machine.history().len(), 1);
    /// ```
    pub fn apply(
        &mut self,
        next: TransmissionState,
        reason: impl Into<String>,
    ) -> Result<TransmissionTransition, ReconcileError> {
        let transition = TransmissionTransition::new(self.current.clone(), next, reason)?;
        self.country_registry
            .validate_transition(&transition.from, &transition.to)?;
        self.current = transition.to.clone();
        self.history.push(transition.clone());
        Ok(transition)
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
    /// A country-specific sub-state transition was not allowed.
    #[error("invalid country sub-state transition for `{system}` from `{from}` to `{to}`")]
    InvalidCountrySubStateTransition {
        /// Country, authority, or network namespace.
        system: String,
        /// State before the rejected transition.
        from: String,
        /// State after the rejected transition.
        to: String,
    },
    /// A transition attempted to move between two country-specific systems.
    #[error("country sub-state system mismatch from `{from}` to `{to}`")]
    CountrySubStateSystemMismatch {
        /// System carried by the current state.
        from: String,
        /// System carried by the next state.
        to: String,
    },
    /// Two country-specific sub-state rules described the same edge.
    #[error("duplicate country sub-state transition for `{system}` from `{from}` to `{to}`")]
    DuplicateCountrySubStateTransition {
        /// Country, authority, or network namespace.
        system: String,
        /// Duplicate source edge.
        from: String,
        /// Duplicate target edge.
        to: String,
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
    /// Retry policy parameters would make scheduling ambiguous or unsafe.
    #[error("invalid retry policy field `{field}`: {message}; remediation: {remediation}")]
    InvalidRetryPolicy {
        /// Field that failed validation.
        field: &'static str,
        /// Human-readable diagnostic.
        message: &'static str,
        /// Human-readable remediation hint.
        remediation: &'static str,
    },
    /// Retry attempt number was outside the configured retry policy.
    #[error("invalid retry attempt `{attempt}`: {message}; remediation: {remediation}")]
    InvalidRetryAttempt {
        /// Attempt number supplied by the caller.
        attempt: u16,
        /// Human-readable diagnostic.
        message: &'static str,
        /// Human-readable remediation hint.
        remediation: &'static str,
    },
    /// Transmission worker configuration is unsafe or ambiguous.
    #[error(
        "invalid transmission worker config field `{field}`: {message}; remediation: {remediation}"
    )]
    InvalidTransmissionWorkerConfig {
        /// Configuration field that failed validation.
        field: &'static str,
        /// Human-readable diagnostic.
        message: &'static str,
        /// Human-readable remediation hint.
        remediation: &'static str,
    },
    /// An outbox row was not ready for the transmission worker.
    #[error("invalid outbox state for `{outbox_id}`: `{state}`; remediation: {remediation}")]
    InvalidOutboxState {
        /// Outbox row identifier.
        outbox_id: String,
        /// State that was rejected.
        state: OutboxState,
        /// Human-readable remediation hint.
        remediation: &'static str,
    },
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

fn transition_country_system(
    from: &TransmissionState,
    to: &TransmissionState,
) -> Result<Option<String>, ReconcileError> {
    match (&from.country_substate, &to.country_substate) {
        (Some(from_substate), Some(to_substate)) if from_substate.system != to_substate.system => {
            Err(ReconcileError::CountrySubStateSystemMismatch {
                from: from_substate.system.clone(),
                to: to_substate.system.clone(),
            })
        }
        (Some(from_substate), _) => Ok(Some(from_substate.system.clone())),
        (_, Some(to_substate)) => Ok(Some(to_substate.system.clone())),
        (None, None) => Ok(None),
    }
}

fn describe_country_state(state: &TransmissionState) -> String {
    state.country_substate.as_ref().map_or_else(
        || state.base.to_string(),
        |substate| format!("{}:{}/{}", state.base, substate.system, substate.code),
    )
}

fn format_country_rule_endpoint(base: TransmissionBaseState, code: Option<&str>) -> String {
    code.map_or_else(|| base.to_string(), |code| format!("{base}/{code}"))
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
    use proptest::prelude::*;
    use serde_json::{json, Value};

    use super::{
        crate_name, fingerprint, Blake3Hash, CancelRequest, CorrectRequest, CountrySubState,
        CountrySubStateRegistry, CountrySubStateTransition, GatewayAdapter, GatewayAttemptId,
        GatewayContext, GatewayError, GatewayErrorKind, GatewayFuture, GatewayOperation,
        GatewayReceipt, GatewayRoute, GatewayStatus, GatewaySubmissionId, IdempotencyKey,
        PollRequest, ReconcileError, SubmitRequest, TenantId, TraceId, TransmissionBaseState,
        TransmissionState, TransmissionStateMachine,
    };

    const FINGERPRINT_TEST_VECTOR_HEX: &str =
        "437ccffe5449042844eef1adb2181c7e9bfed6b097810145189c1f872ca58bde";

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
    fn fingerprint_matches_committed_test_vector() {
        let document = fingerprint_document(
            "DE123456789",
            "FR123456789",
            "2026-05-26",
            "INV-2026-0001",
            "119.00",
            "EUR",
        );

        assert_eq!(
            fingerprint_hex(fingerprint(&document)),
            FINGERPRINT_TEST_VECTOR_HEX
        );
    }

    #[test]
    fn fingerprint_is_deterministic_for_same_input() {
        let document = fingerprint_document(
            "DE123456789",
            "FR123456789",
            "2026-05-26",
            "INV-2026-0001",
            "119.00",
            "EUR",
        );

        assert_eq!(fingerprint(&document), fingerprint(&document));
    }

    #[test]
    fn fingerprint_changes_when_any_formula_field_changes() {
        let base = fingerprint_document_value(
            "DE123456789",
            "FR123456789",
            "2026-05-26",
            "INV-2026-0001",
            "119.00",
            "EUR",
        );
        let base_hash = fingerprint(&CommercialDocument::try_from_value(base.clone()).unwrap());

        let mut variants: Vec<(&str, Value)> = Vec::new();
        let mut changed = base.clone();
        changed["supplier"]["tax_ids"][0]["value"] = json!("DE987654321");
        variants.push(("supplier VAT", changed));

        let mut changed = base.clone();
        changed["customer"]["tax_ids"][0]["value"] = json!("FR987654321");
        variants.push(("customer VAT", changed));

        let mut changed = base.clone();
        changed["issue_date"] = json!("2026-05-27");
        variants.push(("issue date", changed));

        let mut changed = base.clone();
        changed["document_number"] = json!("INV-2026-0002");
        variants.push(("document number", changed));

        let mut changed = base.clone();
        changed["monetary_total"]["payable_amount"] = json!("120.00");
        variants.push(("total amount", changed));

        let mut changed = base;
        changed["currency"] = json!("USD");
        variants.push(("currency", changed));

        for (field, value) in variants {
            let changed_hash = fingerprint(&CommercialDocument::try_from_value(value).unwrap());
            assert_ne!(
                base_hash, changed_hash,
                "{field} change should alter fingerprint"
            );
        }
    }

    proptest! {
        #[test]
        fn generated_fingerprint_is_deterministic(
            supplier_digits in "[0-9]{9}",
            customer_digits in "[0-9]{9}",
            invoice_suffix in 1_u32..1_000_000_u32,
            day in 1_u8..=28,
            cents in 1_u64..10_000_000_u64,
            currency in prop_oneof![Just("EUR"), Just("USD"), Just("GBP")]
        ) {
            let supplier_vat = format!("DE{supplier_digits}");
            let customer_vat = format!("FR{customer_digits}");
            let issue_date = format!("2026-05-{day:02}");
            let document_number = format!("INV-2026-{invoice_suffix:06}");
            let amount = format!("{}.{:02}", cents / 100, cents % 100);
            let document = fingerprint_document(
                &supplier_vat,
                &customer_vat,
                &issue_date,
                &document_number,
                &amount,
                currency,
            );

            prop_assert_eq!(fingerprint(&document), fingerprint(&document));
        }
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
    fn state_machine_implements_every_base_transition() {
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
            (TransmissionBaseState::Sent, TransmissionBaseState::Rejected),
            (
                TransmissionBaseState::Delivered,
                TransmissionBaseState::Acknowledged,
            ),
            (
                TransmissionBaseState::Delivered,
                TransmissionBaseState::Rejected,
            ),
            (
                TransmissionBaseState::Acknowledged,
                TransmissionBaseState::Archived,
            ),
            (
                TransmissionBaseState::Rejected,
                TransmissionBaseState::Archived,
            ),
        ];

        for (from, to) in transitions {
            let mut machine = TransmissionStateMachine::new(TransmissionState::new(from));
            let transition = machine
                .apply(TransmissionState::new(to), "allowed transition")
                .unwrap();

            assert_eq!(machine.current().base, to);
            assert_eq!(machine.history().len(), 1);
            assert_eq!(machine.history()[0], transition);
        }
    }

    #[test]
    fn state_machine_rejects_invalid_base_transition_without_advancing() {
        let mut machine =
            TransmissionStateMachine::new(TransmissionState::new(TransmissionBaseState::Draft));
        let err = machine
            .apply(
                TransmissionState::new(TransmissionBaseState::Sent),
                "skip required states",
            )
            .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::InvalidTransition {
                from: TransmissionBaseState::Draft,
                to: TransmissionBaseState::Sent,
            }
        ));
        assert_eq!(machine.current().base, TransmissionBaseState::Draft);
        assert!(machine.history().is_empty());
    }

    #[test]
    fn country_substate_registry_drives_ksef_sdi_and_zatca_extensions() {
        let registry = country_registry();

        let mut ksef = TransmissionStateMachine::with_country_registry(
            TransmissionState::new(TransmissionBaseState::Reserved),
            registry.clone(),
        );
        ksef.apply(
            TransmissionState::new(TransmissionBaseState::Sent)
                .with_country_substate(country_substate("KSEF", "reserved")),
            "KSeF submission reserved",
        )
        .unwrap();
        ksef.apply(
            TransmissionState::new(TransmissionBaseState::Delivered)
                .with_country_substate(country_substate("KSEF", "committed")),
            "KSeF committed UPO",
        )
        .unwrap();
        assert_eq!(
            ksef.current().country_substate.as_ref().unwrap().code,
            "committed"
        );

        let mut sdi = TransmissionStateMachine::with_country_registry(
            TransmissionState::new(TransmissionBaseState::Sent),
            registry.clone(),
        );
        sdi.apply(
            TransmissionState::new(TransmissionBaseState::Rejected)
                .with_country_substate(country_substate("SDI", "rejected")),
            "SDI scarto receipt",
        )
        .unwrap();
        assert_eq!(
            sdi.current().country_substate.as_ref().unwrap().system,
            "SDI"
        );

        let mut zatca = TransmissionStateMachine::with_country_registry(
            TransmissionState::new(TransmissionBaseState::Sent),
            registry,
        );
        zatca
            .apply(
                TransmissionState::new(TransmissionBaseState::Delivered)
                    .with_country_substate(country_substate("ZATCA", "cleared")),
                "ZATCA cleared invoice",
            )
            .unwrap();
        zatca
            .apply(
                TransmissionState::new(TransmissionBaseState::Acknowledged)
                    .with_country_substate(country_substate("ZATCA", "reported")),
                "ZATCA reported clearance",
            )
            .unwrap();
        assert_eq!(
            zatca.current().country_substate.as_ref().unwrap().code,
            "reported"
        );
    }

    #[test]
    fn country_substate_registry_rejects_unconfigured_code_transition() {
        let registry = country_registry();
        let mut machine = TransmissionStateMachine::with_country_registry(
            TransmissionState::new(TransmissionBaseState::Reserved),
            registry,
        );

        let err = machine
            .apply(
                TransmissionState::new(TransmissionBaseState::Sent)
                    .with_country_substate(country_substate("KSEF", "unknown")),
                "unknown KSeF code",
            )
            .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::InvalidCountrySubStateTransition { system, .. } if system == "KSEF"
        ));
        assert_eq!(machine.current().base, TransmissionBaseState::Reserved);
    }

    #[test]
    fn country_substate_registry_rejects_system_switches() {
        let registry = country_registry();
        let mut machine = TransmissionStateMachine::with_country_registry(
            TransmissionState::new(TransmissionBaseState::Sent)
                .with_country_substate(country_substate("KSEF", "reserved")),
            registry,
        );

        let err = machine
            .apply(
                TransmissionState::new(TransmissionBaseState::Delivered)
                    .with_country_substate(country_substate("SDI", "accepted")),
                "switch systems",
            )
            .unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::CountrySubStateSystemMismatch { from, to }
                if from == "KSEF" && to == "SDI"
        ));
    }

    #[test]
    fn country_substate_registry_rejects_duplicate_rules() {
        let rule = CountrySubStateTransition::initial(
            "KSEF",
            TransmissionBaseState::Reserved,
            TransmissionBaseState::Sent,
            "reserved",
        )
        .unwrap();

        let err = CountrySubStateRegistry::new(vec![rule.clone(), rule]).unwrap_err();

        assert!(matches!(
            err,
            ReconcileError::DuplicateCountrySubStateTransition { system, .. } if system == "KSEF"
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

    fn country_registry() -> CountrySubStateRegistry {
        CountrySubStateRegistry::new(vec![
            CountrySubStateTransition::initial(
                "KSEF",
                TransmissionBaseState::Reserved,
                TransmissionBaseState::Sent,
                "reserved",
            )
            .unwrap(),
            CountrySubStateTransition::from_code(
                "KSEF",
                TransmissionBaseState::Sent,
                "reserved",
                TransmissionBaseState::Delivered,
                "committed",
            )
            .unwrap(),
            CountrySubStateTransition::initial(
                "SDI",
                TransmissionBaseState::Sent,
                TransmissionBaseState::Delivered,
                "accepted",
            )
            .unwrap(),
            CountrySubStateTransition::initial(
                "SDI",
                TransmissionBaseState::Sent,
                TransmissionBaseState::Rejected,
                "rejected",
            )
            .unwrap(),
            CountrySubStateTransition::initial(
                "ZATCA",
                TransmissionBaseState::Sent,
                TransmissionBaseState::Delivered,
                "cleared",
            )
            .unwrap(),
            CountrySubStateTransition::from_code(
                "ZATCA",
                TransmissionBaseState::Delivered,
                "cleared",
                TransmissionBaseState::Acknowledged,
                "reported",
            )
            .unwrap(),
        ])
        .unwrap()
    }

    fn fingerprint_hex(hash: Blake3Hash) -> String {
        hash.to_hex().to_string()
    }

    fn fingerprint_document(
        supplier_vat: &str,
        customer_vat: &str,
        issue_date: &str,
        document_number: &str,
        total_amount: &str,
        currency: &str,
    ) -> CommercialDocument {
        CommercialDocument::try_from_value(fingerprint_document_value(
            supplier_vat,
            customer_vat,
            issue_date,
            document_number,
            total_amount,
            currency,
        ))
        .unwrap()
    }

    fn fingerprint_document_value(
        supplier_vat: &str,
        customer_vat: &str,
        issue_date: &str,
        document_number: &str,
        total_amount: &str,
        currency: &str,
    ) -> Value {
        json!({
            "schema_version": "1.0",
            "id": "doc_fingerprint_vector",
            "document_type": "invoice",
            "issue_date": issue_date,
            "due_date": "2026-06-25",
            "document_number": document_number,
            "currency": currency,
            "supplier": party_with_vat_json(
                "supplier-fingerprint",
                "InvoiceKit GmbH",
                "DE",
                supplier_vat
            ),
            "customer": party_with_vat_json(
                "customer-fingerprint",
                "ACME SAS",
                "FR",
                customer_vat
            ),
            "lines": [{
                "id": "1",
                "description": "Validation subscription",
                "quantity": "1",
                "unit_code": "EA",
                "unit_price": total_amount,
                "line_extension_amount": total_amount,
                "tax_category": "S"
            }],
            "tax_summary": [{
                "category_code": "S",
                "taxable_amount": total_amount,
                "tax_amount": "0.00",
                "tax_rate": "0.00"
            }],
            "monetary_total": {
                "line_extension_amount": total_amount,
                "tax_exclusive_amount": total_amount,
                "tax_inclusive_amount": total_amount,
                "payable_amount": total_amount
            },
            "meta": {
                "tenant_id": "tenant_fingerprint",
                "trace_id": "trace_fingerprint",
                "source_system": "unit-test"
            }
        })
    }

    fn party_with_vat_json(id: &str, name: &str, country: &str, vat: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "tax_ids": [{
                "scheme": "vat",
                "value": vat
            }],
            "address": {
                "lines": ["Main Street 1"],
                "city": "Sample City",
                "postal_code": "10115",
                "country": country
            }
        })
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
