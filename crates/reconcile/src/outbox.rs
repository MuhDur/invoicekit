// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

use crate::{
    validate_identifier, validate_text, Blake3Hash, GatewayContext, GatewayError, IdempotencyKey,
    ReconcileError, TransmissionBaseState,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Bead identifier attached to outbox logs.
pub const OUTBOX_BEAD_ID: &str = "invoices-t-071-outbox-sql-schema-fao";

const RETRY_DOMAIN: &[u8] = b"invoicekit:reconcile:retry-jitter:v1";

const POSTGRES_MIGRATION: OutboxMigration = OutboxMigration {
    dialect: DatabaseDialect::Postgres,
    name: "001_invoicekit_outbox",
    up_sql: include_str!("../migrations/postgres/001_invoicekit_outbox.up.sql"),
    down_sql: include_str!("../migrations/postgres/001_invoicekit_outbox.down.sql"),
};

const MYSQL_MIGRATION: OutboxMigration = OutboxMigration {
    dialect: DatabaseDialect::Mysql,
    name: "001_invoicekit_outbox",
    up_sql: include_str!("../migrations/mysql/001_invoicekit_outbox.up.sql"),
    down_sql: include_str!("../migrations/mysql/001_invoicekit_outbox.down.sql"),
};

const SQLITE_MIGRATION: OutboxMigration = OutboxMigration {
    dialect: DatabaseDialect::Sqlite,
    name: "001_invoicekit_outbox",
    up_sql: include_str!("../migrations/sqlite/001_invoicekit_outbox.up.sql"),
    down_sql: include_str!("../migrations/sqlite/001_invoicekit_outbox.down.sql"),
};

/// SQL dialect supported by the InvoiceKit outbox migrations.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseDialect {
    /// `PostgreSQL` 14 or newer.
    Postgres,
    /// `MySQL` 8.0 or newer.
    Mysql,
    /// `SQLite` 3.38 or newer.
    Sqlite,
}

impl DatabaseDialect {
    /// Returns the stable dialect name.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::DatabaseDialect;
    ///
    /// assert_eq!(DatabaseDialect::Postgres.as_str(), "postgres");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Sqlite => "sqlite",
        }
    }
}

impl fmt::Display for DatabaseDialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reversible SQL migration for one outbox database dialect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutboxMigration {
    /// Database dialect this migration targets.
    pub dialect: DatabaseDialect,
    /// Stable migration name.
    pub name: &'static str,
    /// Idempotent forward SQL.
    pub up_sql: &'static str,
    /// Reversible down SQL.
    pub down_sql: &'static str,
}

impl OutboxMigration {
    /// Returns `true` when both migration directions include idempotency guards.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{outbox_migration, DatabaseDialect};
    ///
    /// let migration = outbox_migration(DatabaseDialect::Sqlite);
    /// assert!(migration.is_idempotent());
    /// ```
    #[must_use]
    pub fn is_idempotent(self) -> bool {
        self.up_sql.contains("IF NOT EXISTS") && self.down_sql.contains("IF EXISTS")
    }
}

/// Returns the outbox migration for one database dialect.
///
/// # Examples
///
/// ```
/// use invoicekit_reconcile::{outbox_migration, DatabaseDialect};
///
/// let migration = outbox_migration(DatabaseDialect::Postgres);
/// assert_eq!(migration.name, "001_invoicekit_outbox");
/// ```
#[must_use]
pub const fn outbox_migration(dialect: DatabaseDialect) -> OutboxMigration {
    match dialect {
        DatabaseDialect::Postgres => POSTGRES_MIGRATION,
        DatabaseDialect::Mysql => MYSQL_MIGRATION,
        DatabaseDialect::Sqlite => SQLITE_MIGRATION,
    }
}

/// Returns the complete migration set in deterministic dialect order.
///
/// # Examples
///
/// ```
/// use invoicekit_reconcile::all_outbox_migrations;
///
/// assert_eq!(all_outbox_migrations().len(), 3);
/// ```
#[must_use]
pub const fn all_outbox_migrations() -> [OutboxMigration; 3] {
    [POSTGRES_MIGRATION, MYSQL_MIGRATION, SQLITE_MIGRATION]
}

/// Durable outbox state stored in `invoicekit_outbox.state`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
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
    /// Gateway confirmed delivery.
    Delivered,
    /// Gateway or buyer acknowledged the delivered invoice.
    Acknowledged,
    /// Gateway rejected the invoice.
    Rejected,
    /// Invoice and evidence were archived.
    Archived,
    /// Retry policy has been exhausted or the failure is unrecoverable.
    DeadLetter,
}

impl OutboxState {
    /// Returns the stable database representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::OutboxState;
    ///
    /// assert_eq!(OutboxState::DeadLetter.as_str(), "dead_letter");
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
            Self::DeadLetter => "dead_letter",
        }
    }

    /// Maps a transmission lifecycle state into its persisted outbox state.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{OutboxState, TransmissionBaseState};
    ///
    /// assert_eq!(
    ///     OutboxState::from_transmission_base(TransmissionBaseState::Sent),
    ///     OutboxState::Sent
    /// );
    /// ```
    #[must_use]
    pub const fn from_transmission_base(state: TransmissionBaseState) -> Self {
        match state {
            TransmissionBaseState::Draft => Self::Draft,
            TransmissionBaseState::Validated => Self::Validated,
            TransmissionBaseState::Signed => Self::Signed,
            TransmissionBaseState::Reserved => Self::Reserved,
            TransmissionBaseState::Sent => Self::Sent,
            TransmissionBaseState::Delivered => Self::Delivered,
            TransmissionBaseState::Acknowledged => Self::Acknowledged,
            TransmissionBaseState::Rejected => Self::Rejected,
            TransmissionBaseState::Archived => Self::Archived,
        }
    }

    /// Returns whether the state will never be retried.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::OutboxState;
    ///
    /// assert!(OutboxState::DeadLetter.is_terminal());
    /// assert!(!OutboxState::Reserved.is_terminal());
    /// ```
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Delivered | Self::Acknowledged | Self::Archived | Self::DeadLetter
        )
    }
}

impl fmt::Display for OutboxState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Retry policy for at-least-once gateway delivery.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetryPolicy {
    /// Maximum number of attempts before the envelope moves to dead letter.
    pub max_attempts: u16,
    /// First retry delay in seconds.
    pub base_delay_seconds: u64,
    /// Upper bound for exponential backoff delay.
    pub max_delay_seconds: u64,
    /// Deterministic jitter as a percentage of the computed exponential delay.
    pub jitter_percent: u8,
}

impl RetryPolicy {
    /// Builds a validated exponential-backoff retry policy.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidRetryPolicy`] when any parameter would
    /// make retry scheduling ambiguous or unsafe.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::RetryPolicy;
    ///
    /// let policy = RetryPolicy::new(8, 30, 3_600, 25).unwrap();
    /// assert_eq!(policy.max_attempts, 8);
    /// ```
    pub const fn new(
        max_attempts: u16,
        base_delay_seconds: u64,
        max_delay_seconds: u64,
        jitter_percent: u8,
    ) -> Result<Self, ReconcileError> {
        if max_attempts == 0 {
            return Err(ReconcileError::InvalidRetryPolicy {
                field: "max_attempts",
                message: "must be at least one",
                remediation: "set max_attempts to the total number of delivery tries to allow",
            });
        }
        if base_delay_seconds == 0 {
            return Err(ReconcileError::InvalidRetryPolicy {
                field: "base_delay_seconds",
                message: "must be greater than zero",
                remediation: "set a positive first retry delay",
            });
        }
        if max_delay_seconds < base_delay_seconds {
            return Err(ReconcileError::InvalidRetryPolicy {
                field: "max_delay_seconds",
                message: "must be greater than or equal to base_delay_seconds",
                remediation: "raise max_delay_seconds or lower base_delay_seconds",
            });
        }
        if jitter_percent > 100 {
            return Err(ReconcileError::InvalidRetryPolicy {
                field: "jitter_percent",
                message: "must be between 0 and 100",
                remediation: "use a jitter percentage in the inclusive range 0..=100",
            });
        }
        Ok(Self {
            max_attempts,
            base_delay_seconds,
            max_delay_seconds,
            jitter_percent,
        })
    }

    /// Returns the default production retry policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::RetryPolicy;
    ///
    /// assert_eq!(RetryPolicy::default_policy().max_attempts, 8);
    /// ```
    #[must_use]
    pub const fn default_policy() -> Self {
        Self {
            max_attempts: 8,
            base_delay_seconds: 30,
            max_delay_seconds: 3_600,
            jitter_percent: 25,
        }
    }

    /// Computes the deterministic retry delay for a one-based attempt number.
    ///
    /// Attempt `1` is the first retry after the initial gateway attempt failed.
    /// Jitter is deterministic per `(idempotency_key, attempt)`, so tests and
    /// replay-from-evidence produce the same schedule while production traffic
    /// is still spread across a bounded window.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidRetryAttempt`] when `attempt_number` is
    /// zero or has reached the configured delivery-attempt budget.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{IdempotencyKey, RetryPolicy};
    ///
    /// let key = IdempotencyKey::new("idem_invoice_123").unwrap();
    /// let delay = RetryPolicy::new(8, 30, 3_600, 25)
    ///     .unwrap()
    ///     .delay_for_attempt(1, &key)
    ///     .unwrap();
    /// assert!((30..=37).contains(&delay));
    /// ```
    pub fn delay_for_attempt(
        self,
        attempt_number: u16,
        idempotency_key: &IdempotencyKey,
    ) -> Result<u64, ReconcileError> {
        if attempt_number == 0 {
            return Err(ReconcileError::InvalidRetryAttempt {
                attempt: attempt_number,
                message: "attempt numbers are one-based",
                remediation: "pass 1 for the first retry attempt",
            });
        }
        if attempt_number >= self.max_attempts {
            return Err(ReconcileError::InvalidRetryAttempt {
                attempt: attempt_number,
                message: "attempt has exhausted max_attempts",
                remediation: "move the outbox envelope to the dead-letter table",
            });
        }

        let exponent = u32::from(attempt_number.saturating_sub(1)).min(63);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let exponential_delay = self
            .base_delay_seconds
            .saturating_mul(multiplier)
            .min(self.max_delay_seconds);
        let jitter_cap = exponential_delay.saturating_mul(u64::from(self.jitter_percent)) / 100;

        if jitter_cap == 0 {
            return Ok(exponential_delay);
        }

        let jitter = deterministic_jitter(idempotency_key, attempt_number, jitter_cap);
        Ok(exponential_delay
            .saturating_add(jitter)
            .min(self.max_delay_seconds))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// Result of recording a failed delivery attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDecision {
    /// Retry the envelope after this many seconds.
    RetryAfterSeconds(u64),
    /// Move the envelope to `invoicekit_outbox_dead_letter`.
    MoveToDeadLetter,
}

/// Typed representation of an outbox row before it is persisted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutboxEnvelope {
    /// Stable outbox row identifier.
    pub outbox_id: String,
    /// Gateway context carrying tenant, trace, idempotency, and attempt IDs.
    pub context: GatewayContext,
    /// Deterministic invoice fingerprint as lowercase BLAKE3 hex.
    pub invoice_fingerprint_hex: String,
    /// Current durable outbox state.
    pub state: OutboxState,
    /// Canonical invoice payload or gateway command JSON.
    pub payload_json: String,
    /// Number of failed attempts already recorded.
    pub attempt_count: u16,
    /// Retry policy used by the transmission worker.
    pub retry_policy: RetryPolicy,
}

impl OutboxEnvelope {
    /// Builds a validated outbox envelope in the `reserved` state.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] or
    /// [`ReconcileError::InvalidIdentifier`] when text fields are blank or
    /// unsafe for durable identifiers.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, IdempotencyKey, OutboxEnvelope,
    ///     OutboxState, RetryPolicy, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// let envelope = OutboxEnvelope::new(
    ///     "outbox_001",
    ///     context,
    ///     blake3::hash(b"invoice"),
    ///     "{}",
    ///     RetryPolicy::default(),
    /// )
    /// .unwrap();
    /// assert_eq!(envelope.state, OutboxState::Reserved);
    /// ```
    pub fn new(
        outbox_id: impl Into<String>,
        context: GatewayContext,
        invoice_fingerprint: Blake3Hash,
        payload_json: impl Into<String>,
        retry_policy: RetryPolicy,
    ) -> Result<Self, ReconcileError> {
        let outbox_id = outbox_id.into();
        validate_identifier(&outbox_id, "outbox_id")?;
        let payload_json = payload_json.into();
        validate_text(&payload_json, "payload_json")?;

        tracing::debug!(
            bead_id = OUTBOX_BEAD_ID,
            tenant_id = context.tenant_id.as_str(),
            trace_id = context.trace_id.as_str(),
            idempotency_key = context.idempotency_key.as_str(),
            outbox_id = outbox_id.as_str(),
            "created outbox envelope"
        );

        Ok(Self {
            outbox_id,
            context,
            invoice_fingerprint_hex: invoice_fingerprint.to_hex().to_string(),
            state: OutboxState::Reserved,
            payload_json,
            attempt_count: 0,
            retry_policy,
        })
    }

    /// Records one failed attempt and returns the next retry decision.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::InvalidRetryAttempt`] only if the envelope's
    /// retry counters are internally inconsistent with its policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, IdempotencyKey, OutboxEnvelope,
    ///     RetryDecision, RetryPolicy, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// let mut envelope = OutboxEnvelope::new(
    ///     "outbox_001",
    ///     context,
    ///     blake3::hash(b"invoice"),
    ///     "{}",
    ///     RetryPolicy::new(2, 30, 60, 0).unwrap(),
    /// )
    /// .unwrap();
    /// assert_eq!(
    ///     envelope.record_failed_attempt().unwrap(),
    ///     RetryDecision::RetryAfterSeconds(30)
    /// );
    /// ```
    pub fn record_failed_attempt(&mut self) -> Result<RetryDecision, ReconcileError> {
        if self.attempt_count >= self.retry_policy.max_attempts {
            self.state = OutboxState::DeadLetter;
            return Ok(RetryDecision::MoveToDeadLetter);
        }

        self.attempt_count = self.attempt_count.saturating_add(1);
        if self.attempt_count >= self.retry_policy.max_attempts {
            self.state = OutboxState::DeadLetter;
            return Ok(RetryDecision::MoveToDeadLetter);
        }

        let delay = self
            .retry_policy
            .delay_for_attempt(self.attempt_count, &self.context.idempotency_key)?;
        Ok(RetryDecision::RetryAfterSeconds(delay))
    }

    /// Builds a dead-letter record from a normalized gateway error.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] or
    /// [`ReconcileError::InvalidIdentifier`] if the generated record text is
    /// not safe to persist.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     GatewayAttemptId, GatewayContext, GatewayError, GatewayErrorKind,
    ///     GatewayOperation, IdempotencyKey, OutboxEnvelope, RetryPolicy,
    ///     TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// let envelope = OutboxEnvelope::new(
    ///     "outbox_001",
    ///     context,
    ///     blake3::hash(b"invoice"),
    ///     "{}",
    ///     RetryPolicy::default(),
    /// )
    /// .unwrap();
    /// let error = GatewayError::new(
    ///     GatewayErrorKind::Rejected,
    ///     GatewayOperation::Submit,
    ///     "business rejection",
    ///     "inspect validation trace",
    /// );
    /// let dead = envelope.to_dead_letter("dead_001", &error).unwrap();
    /// assert_eq!(dead.failure_code, "rejected");
    /// ```
    pub fn to_dead_letter(
        &self,
        dead_letter_id: impl Into<String>,
        error: &GatewayError,
    ) -> Result<DeadLetterRecord, ReconcileError> {
        tracing::debug!(
            bead_id = OUTBOX_BEAD_ID,
            tenant_id = self.context.tenant_id.as_str(),
            trace_id = self.context.trace_id.as_str(),
            idempotency_key = self.context.idempotency_key.as_str(),
            outbox_id = self.outbox_id.as_str(),
            failure_code = error.kind.as_str(),
            "moving outbox envelope to dead letter"
        );

        DeadLetterRecord::new(
            dead_letter_id,
            self,
            error.kind.as_str(),
            error.message.as_str(),
        )
    }
}

/// Typed representation of a row in `invoicekit_outbox_dead_letter`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeadLetterRecord {
    /// Stable dead-letter row identifier.
    pub dead_letter_id: String,
    /// Original outbox row identifier.
    pub outbox_id: String,
    /// Gateway context copied from the failed envelope.
    pub context: GatewayContext,
    /// Deterministic invoice fingerprint copied from the failed envelope.
    pub invoice_fingerprint_hex: String,
    /// State at the time the envelope was dead-lettered.
    pub final_state: OutboxState,
    /// Normalized machine-readable failure code.
    pub failure_code: String,
    /// Human-readable failure message.
    pub failure_message: String,
    /// Number of attempts made before dead-lettering.
    pub attempt_count: u16,
    /// Canonical invoice payload or gateway command JSON.
    pub payload_json: String,
}

impl DeadLetterRecord {
    /// Builds a validated dead-letter record from an outbox envelope.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError::MissingRequiredField`] or
    /// [`ReconcileError::InvalidIdentifier`] when text fields are blank or
    /// unsafe for durable identifiers.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_reconcile::{
    ///     DeadLetterRecord, GatewayAttemptId, GatewayContext, IdempotencyKey,
    ///     OutboxEnvelope, RetryPolicy, TenantId, TraceId,
    /// };
    ///
    /// let context = GatewayContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     IdempotencyKey::new("idem_invoice_123").unwrap(),
    ///     GatewayAttemptId::new("attempt_001").unwrap(),
    /// );
    /// let envelope = OutboxEnvelope::new(
    ///     "outbox_001",
    ///     context,
    ///     blake3::hash(b"invoice"),
    ///     "{}",
    ///     RetryPolicy::default(),
    /// )
    /// .unwrap();
    /// let dead = DeadLetterRecord::new(
    ///     "dead_001",
    ///     &envelope,
    ///     "gateway_maintenance",
    ///     "maintenance window exceeded retry budget",
    /// )
    /// .unwrap();
    /// assert_eq!(dead.outbox_id, "outbox_001");
    /// ```
    pub fn new(
        dead_letter_id: impl Into<String>,
        envelope: &OutboxEnvelope,
        failure_code: impl Into<String>,
        failure_message: impl Into<String>,
    ) -> Result<Self, ReconcileError> {
        let dead_letter_id = dead_letter_id.into();
        validate_identifier(&dead_letter_id, "dead_letter_id")?;
        let failure_code = failure_code.into();
        validate_identifier(&failure_code, "failure_code")?;
        let failure_message = failure_message.into();
        validate_text(&failure_message, "failure_message")?;

        Ok(Self {
            dead_letter_id,
            outbox_id: envelope.outbox_id.clone(),
            context: envelope.context.clone(),
            invoice_fingerprint_hex: envelope.invoice_fingerprint_hex.clone(),
            final_state: envelope.state,
            failure_code,
            failure_message,
            attempt_count: envelope.attempt_count,
            payload_json: envelope.payload_json.clone(),
        })
    }
}

fn deterministic_jitter(
    idempotency_key: &IdempotencyKey,
    attempt_number: u16,
    jitter_cap: u64,
) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RETRY_DOMAIN);
    hasher.update(idempotency_key.as_str().as_bytes());
    hasher.update(&attempt_number.to_be_bytes());
    let hash = hasher.finalize();
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = *hash.as_bytes();
    let value = u64::from_be_bytes([b0, b1, b2, b3, b4, b5, b6, b7]);
    value % jitter_cap.saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GatewayAttemptId, GatewayErrorKind, GatewayOperation, TenantId, TraceId};
    use proptest::prelude::*;

    fn context(key: &str) -> GatewayContext {
        GatewayContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new("trace_123").unwrap(),
            IdempotencyKey::new(key).unwrap(),
            GatewayAttemptId::new("attempt_001").unwrap(),
        )
    }

    #[test]
    fn migrations_are_idempotent_and_reversible_for_all_dialects() {
        for migration in all_outbox_migrations() {
            assert!(migration.is_idempotent(), "{:?}", migration.dialect);
            assert!(
                migration.up_sql.contains("invoicekit_outbox"),
                "{:?}",
                migration.dialect
            );
            assert!(
                migration.up_sql.contains("invoicekit_outbox_dead_letter"),
                "{:?}",
                migration.dialect
            );
            assert!(
                migration.up_sql.contains("idempotency_key"),
                "{:?}",
                migration.dialect
            );
            assert!(
                migration.up_sql.contains("UNIQUE"),
                "{:?}",
                migration.dialect
            );
            assert!(
                migration.down_sql.contains("invoicekit_outbox_dead_letter"),
                "{:?}",
                migration.dialect
            );
        }
    }

    #[test]
    fn retry_policy_validates_unsafe_parameters() {
        assert!(matches!(
            RetryPolicy::new(0, 30, 60, 25),
            Err(ReconcileError::InvalidRetryPolicy {
                field: "max_attempts",
                ..
            })
        ));
        assert!(matches!(
            RetryPolicy::new(8, 0, 60, 25),
            Err(ReconcileError::InvalidRetryPolicy {
                field: "base_delay_seconds",
                ..
            })
        ));
        assert!(matches!(
            RetryPolicy::new(8, 60, 30, 25),
            Err(ReconcileError::InvalidRetryPolicy {
                field: "max_delay_seconds",
                ..
            })
        ));
        assert!(matches!(
            RetryPolicy::new(8, 30, 60, 101),
            Err(ReconcileError::InvalidRetryPolicy {
                field: "jitter_percent",
                ..
            })
        ));
    }

    #[test]
    fn retry_policy_applies_exponential_backoff_with_deterministic_jitter() {
        let key = IdempotencyKey::new("idem_retry_123").unwrap();
        let policy = RetryPolicy::new(8, 30, 600, 25).unwrap();

        let first = policy.delay_for_attempt(1, &key).unwrap();
        let second = policy.delay_for_attempt(2, &key).unwrap();
        let third = policy.delay_for_attempt(3, &key).unwrap();

        assert!((30..=37).contains(&first));
        assert!((60..=75).contains(&second));
        assert!((120..=150).contains(&third));
        assert_eq!(first, policy.delay_for_attempt(1, &key).unwrap());
    }

    #[test]
    fn retry_policy_rejects_attempts_outside_budget() {
        let key = IdempotencyKey::new("idem_retry_123").unwrap();
        let policy = RetryPolicy::new(2, 30, 60, 0).unwrap();

        assert!(matches!(
            policy.delay_for_attempt(0, &key),
            Err(ReconcileError::InvalidRetryAttempt { attempt: 0, .. })
        ));
        assert!(matches!(
            policy.delay_for_attempt(2, &key),
            Err(ReconcileError::InvalidRetryAttempt { attempt: 2, .. })
        ));
        assert!(matches!(
            RetryPolicy::new(1, 30, 60, 0)
                .unwrap()
                .delay_for_attempt(1, &key),
            Err(ReconcileError::InvalidRetryAttempt { attempt: 1, .. })
        ));
    }

    #[test]
    fn outbox_envelope_rejects_invalid_text() {
        assert!(OutboxEnvelope::new(
            " outbox_001 ",
            context("idem_invoice_123"),
            blake3::hash(b"invoice"),
            "{}",
            RetryPolicy::default(),
        )
        .is_err());

        assert!(OutboxEnvelope::new(
            "outbox_001",
            context("idem_invoice_123"),
            blake3::hash(b"invoice"),
            "",
            RetryPolicy::default(),
        )
        .is_err());
    }

    #[test]
    fn outbox_envelope_moves_to_dead_letter_after_retry_budget() {
        let mut envelope = OutboxEnvelope::new(
            "outbox_001",
            context("idem_invoice_123"),
            blake3::hash(b"invoice"),
            "{}",
            RetryPolicy::new(2, 30, 60, 0).unwrap(),
        )
        .unwrap();

        assert_eq!(
            envelope.record_failed_attempt().unwrap(),
            RetryDecision::RetryAfterSeconds(30)
        );
        assert_eq!(
            envelope.record_failed_attempt().unwrap(),
            RetryDecision::MoveToDeadLetter
        );
        assert_eq!(envelope.state, OutboxState::DeadLetter);
    }

    #[test]
    fn dead_letter_record_preserves_outbox_context() {
        let mut envelope = OutboxEnvelope::new(
            "outbox_001",
            context("idem_invoice_123"),
            blake3::hash(b"invoice"),
            "{}",
            RetryPolicy::new(1, 30, 60, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            envelope.record_failed_attempt().unwrap(),
            RetryDecision::MoveToDeadLetter
        );
        let error = GatewayError::new(
            GatewayErrorKind::GatewayMaintenance,
            GatewayOperation::Submit,
            "maintenance window exceeded retry budget",
            "inspect gateway status and replay later",
        );

        let dead = envelope.to_dead_letter("dead_001", &error).unwrap();

        assert_eq!(dead.dead_letter_id, "dead_001");
        assert_eq!(dead.outbox_id, "outbox_001");
        assert_eq!(dead.context.tenant_id.as_str(), "tenant_acme");
        assert_eq!(dead.failure_code, "gateway_maintenance");
        assert_eq!(dead.final_state, OutboxState::DeadLetter);
    }

    proptest! {
        #[test]
        fn retry_delay_is_deterministic_and_bounded(
            attempt in 1_u16..8,
            key_suffix in "[a-z0-9]{1,16}",
        ) {
            let key = IdempotencyKey::new(format!("idem_{key_suffix}")).unwrap();
            let policy = RetryPolicy::new(8, 10, 300, 25).unwrap();

            let first = policy.delay_for_attempt(attempt, &key).unwrap();
            let second = policy.delay_for_attempt(attempt, &key).unwrap();

            prop_assert_eq!(first, second);
            prop_assert!((10..=300).contains(&first));
        }
    }
}
