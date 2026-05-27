// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-managed-api` — InvoiceKit workspace member.
//!
//! This crate owns the deterministic tenant, authentication, authorization,
//! and audit-event model for the hosted managed layer. HTTP routing, token
//! exchange, database persistence, dashboards, and deployment artifacts are
//! intentionally left to later Track 11 and Track 13 crates. The types here
//! are the shared contract those layers use so tenant identity and audit
//! evidence do not drift across services.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod audit_log;
pub mod observability;
pub use observability::{
    redact_log_value, GatewayDashboardSnapshot, ManagedRequestObservation,
    ManagedRequestObservationInput, ObservedRequestSpan, OpenTelemetryIds, SloMetricEvent,
    SloOperation, TelemetryOutcome, LOG_REDACTION_PLACEHOLDER, OBSERVABILITY_BEAD_ID,
};

/// Google's `OpenID` Connect discovery document URL.
///
/// Google documents this URI as the hard-coded discovery entry point for
/// Google `OpenID` Connect clients.
pub const GOOGLE_DISCOVERY_DOCUMENT_URI: &str =
    "https://accounts.google.com/.well-known/openid-configuration";

/// Expected issuer for Google ID token claims.
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";

const GOOGLE_AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_SCOPE: &str = "openid email profile";

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_managed_api::crate_name(), "invoicekit-managed-api");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-managed-api"
}

/// Tenant identifier carried by every managed-layer request and resource.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(String);

impl TenantId {
    /// Build a tenant identifier from a stored or incoming value.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when the value is empty,
    /// contains surrounding whitespace, is longer than 128 bytes, or contains
    /// characters outside `A-Z`, `a-z`, `0-9`, `_`, `.`, `:`, and `-`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::TenantId;
    /// let tenant = TenantId::new("tenant_acme").unwrap();
    /// assert_eq!(tenant.as_str(), "tenant_acme");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self(validate_identifier("tenant_id", value.into())?))
    }

    /// Return the tenant identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::TenantId;
    /// let tenant = TenantId::new("tenant_acme").unwrap();
    /// assert_eq!(tenant.as_str(), "tenant_acme");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Principal identifier for a human or service identity inside a tenant.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrincipalId(String);

impl PrincipalId {
    /// Build a principal identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the value is not a
    /// valid managed-layer identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::PrincipalId;
    /// let principal = PrincipalId::new("user:google:123").unwrap();
    /// assert_eq!(principal.as_str(), "user:google:123");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self(validate_identifier("principal_id", value.into())?))
    }

    /// Return the principal identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::PrincipalId;
    /// let principal = PrincipalId::new("user_123").unwrap();
    /// assert_eq!(principal.as_str(), "user_123");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stored API-key identifier. This is not the API-key secret.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiKeyId(String);

impl ApiKeyId {
    /// Build an API-key identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the value is not a
    /// valid managed-layer identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::ApiKeyId;
    /// let key = ApiKeyId::new("key_live_123").unwrap();
    /// assert_eq!(key.as_str(), "key_live_123");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self(validate_identifier("api_key_id", value.into())?))
    }

    /// Return the API-key identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::ApiKeyId;
    /// let key = ApiKeyId::new("key_live_123").unwrap();
    /// assert_eq!(key.as_str(), "key_live_123");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ApiKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Trace identifier propagated from public API edge to gateway attempts.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceId(String);

impl TraceId {
    /// Build a trace identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the value is not a
    /// valid managed-layer identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::TraceId;
    /// let trace = TraceId::new("trace_123").unwrap();
    /// assert_eq!(trace.as_str(), "trace_123");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self(validate_identifier("trace_id", value.into())?))
    }

    /// Return the trace identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::TraceId;
    /// let trace = TraceId::new("trace_123").unwrap();
    /// assert_eq!(trace.as_str(), "trace_123");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Audit-event identifier.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuditEventId(String);

impl AuditEventId {
    /// Build an audit-event identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the value is not a
    /// valid managed-layer identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::AuditEventId;
    /// let event = AuditEventId::new("aud_123").unwrap();
    /// assert_eq!(event.as_str(), "aud_123");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self(validate_identifier("audit_event_id", value.into())?))
    }

    /// Return the audit-event identifier as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::AuditEventId;
    /// let event = AuditEventId::new("aud_123").unwrap();
    /// assert_eq!(event.as_str(), "aud_123");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuditEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Contract for values that are scoped to exactly one tenant.
pub trait TenantScoped {
    /// Return the tenant that owns this value.
    fn tenant_id(&self) -> &TenantId;
}

/// Actor attached to request context and audit events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// Request authenticated by a scoped API key.
    ApiKey {
        /// Stored API-key identifier.
        key_id: ApiKeyId,
    },
    /// Request authenticated by a human or service principal.
    Principal {
        /// Principal identifier inside the tenant.
        principal_id: PrincipalId,
    },
    /// Internal system actor.
    System {
        /// Stable system actor name.
        name: String,
    },
}

/// Per-request tenant context carried through managed service operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TenantRequestContext {
    /// Tenant selected by the credential and request route.
    pub tenant_id: TenantId,
    /// Trace identifier used for logs, audit events, and gateway attempts.
    pub trace_id: TraceId,
    /// Authenticated actor.
    pub actor: Actor,
}

impl TenantRequestContext {
    /// Build a tenant request context.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Actor, PrincipalId, TenantId, TenantRequestContext, TraceId};
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     Actor::Principal { principal_id: PrincipalId::new("user_123").unwrap() },
    /// );
    /// assert_eq!(ctx.tenant_id.as_str(), "tenant_acme");
    /// ```
    #[must_use]
    pub const fn new(tenant_id: TenantId, trace_id: TraceId, actor: Actor) -> Self {
        Self {
            tenant_id,
            trace_id,
            actor,
        }
    }

    /// Require that another tenant-scoped value belongs to this request.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::TenantMismatch`] when the scoped value belongs
    /// to a different tenant.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Actor, ApiKeyId, ApiKeyRecord, ApiKeySecretDigest, ApiScope, TenantId, TenantRequestContext, TraceId};
    /// # use std::collections::BTreeSet;
    /// let tenant = TenantId::new("tenant_acme").unwrap();
    /// let ctx = TenantRequestContext::new(
    ///     tenant.clone(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     Actor::System { name: "test".to_owned() },
    /// );
    /// let key = ApiKeyRecord::new(
    ///     tenant,
    ///     ApiKeyId::new("key_123").unwrap(),
    ///     "ci key",
    ///     ApiKeySecretDigest::new("sha256", "digest").unwrap(),
    ///     "1234",
    ///     BTreeSet::from([ApiScope::InvoiceRead]),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap();
    /// ctx.require_same_tenant(&key).unwrap();
    /// ```
    pub fn require_same_tenant<T: TenantScoped>(&self, value: &T) -> Result<(), ManagedApiError> {
        if self.tenant_id == *value.tenant_id() {
            Ok(())
        } else {
            Err(ManagedApiError::TenantMismatch {
                expected: self.tenant_id.clone(),
                actual: value.tenant_id().clone(),
            })
        }
    }
}

impl TenantScoped for TenantRequestContext {
    fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// Explicit API-key scopes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum ApiScope {
    /// Full tenant administration.
    #[serde(rename = "tenant:admin")]
    TenantAdmin,
    /// Read invoices and their validation/rendering status.
    #[serde(rename = "invoice:read")]
    InvoiceRead,
    /// Create or update invoices.
    #[serde(rename = "invoice:write")]
    InvoiceWrite,
    /// Validate invoices.
    #[serde(rename = "invoice:validate")]
    InvoiceValidate,
    /// Render invoices to deterministic PDF/HTML outputs.
    #[serde(rename = "invoice:render")]
    InvoiceRender,
    /// Send invoices through gateway adapters.
    #[serde(rename = "invoice:transmit")]
    InvoiceTransmit,
    /// Read archived evidence bundles and invoice artifacts.
    #[serde(rename = "archive:read")]
    ArchiveRead,
    /// Read customer-facing audit events.
    #[serde(rename = "audit:read")]
    AuditRead,
}

impl ApiScope {
    /// Return the wire-format scope string.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::ApiScope;
    /// assert_eq!(ApiScope::InvoiceRead.as_str(), "invoice:read");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TenantAdmin => "tenant:admin",
            Self::InvoiceRead => "invoice:read",
            Self::InvoiceWrite => "invoice:write",
            Self::InvoiceValidate => "invoice:validate",
            Self::InvoiceRender => "invoice:render",
            Self::InvoiceTransmit => "invoice:transmit",
            Self::ArchiveRead => "archive:read",
            Self::AuditRead => "audit:read",
        }
    }

    /// Return whether this scope grants a permission.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{ApiScope, Permission};
    /// assert!(ApiScope::InvoiceWrite.permits(Permission::WriteInvoices));
    /// assert!(!ApiScope::InvoiceRead.permits(Permission::WriteInvoices));
    /// ```
    #[must_use]
    pub const fn permits(self, permission: Permission) -> bool {
        matches!(
            (self, permission),
            (Self::TenantAdmin, _)
                | (Self::InvoiceRead, Permission::ReadInvoices)
                | (Self::InvoiceWrite, Permission::WriteInvoices)
                | (Self::InvoiceValidate, Permission::ValidateInvoices)
                | (Self::InvoiceRender, Permission::RenderInvoices)
                | (Self::InvoiceTransmit, Permission::TransmitInvoices)
                | (Self::ArchiveRead, Permission::ReadArchive)
                | (Self::AuditRead, Permission::ReadAudit)
        )
    }
}

impl fmt::Display for ApiScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stored digest for an API-key secret.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiKeySecretDigest {
    /// Digest algorithm name, such as `sha256` or `argon2id`.
    pub algorithm: String,
    /// Encoded digest. The raw API-key secret is never stored here.
    pub digest: String,
}

impl ApiKeySecretDigest {
    /// Build a stored API-key digest descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if the algorithm is empty
    /// or contains unsupported characters, or if the digest is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::ApiKeySecretDigest;
    /// let digest = ApiKeySecretDigest::new("sha256", "abc123").unwrap();
    /// assert_eq!(digest.algorithm, "sha256");
    /// ```
    pub fn new(
        algorithm: impl Into<String>,
        digest: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        let algorithm = validate_identifier("secret_digest.algorithm", algorithm.into())?;
        let digest = digest.into();
        if digest.trim().is_empty() {
            return Err(ManagedApiError::InvalidIdentifier {
                field: "secret_digest.digest",
                reason: "digest must not be empty",
            });
        }
        Ok(Self { algorithm, digest })
    }
}

/// Lifecycle state for a stored API key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyStatus {
    /// The key may authenticate requests.
    Active,
    /// The key is retained for audit history but may not authenticate.
    Revoked,
}

/// API key record with explicit tenant and scope ownership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    /// Tenant that owns the key.
    pub tenant_id: TenantId,
    /// Stored key identifier.
    pub key_id: ApiKeyId,
    /// Human-readable key name.
    pub name: String,
    /// Stored digest descriptor for the secret.
    pub secret_digest: ApiKeySecretDigest,
    /// Non-secret preview, usually the last four visible characters.
    pub secret_preview: String,
    /// Explicit scopes granted to the key.
    pub scopes: BTreeSet<ApiScope>,
    /// Key lifecycle state.
    pub status: ApiKeyStatus,
    /// RFC 3339 timestamp for key creation.
    pub created_at: String,
}

impl ApiKeyRecord {
    /// Build an active scoped API-key record.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when the name, preview,
    /// timestamp, or scope set is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{ApiKeyId, ApiKeyRecord, ApiKeySecretDigest, ApiScope, TenantId};
    /// # use std::collections::BTreeSet;
    /// let key = ApiKeyRecord::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     ApiKeyId::new("key_123").unwrap(),
    ///     "CI key",
    ///     ApiKeySecretDigest::new("sha256", "digest").unwrap(),
    ///     "1234",
    ///     BTreeSet::from([ApiScope::InvoiceRead]),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap();
    /// assert!(key.allows_scope(ApiScope::InvoiceRead));
    /// ```
    pub fn new(
        tenant_id: TenantId,
        key_id: ApiKeyId,
        name: impl Into<String>,
        secret_digest: ApiKeySecretDigest,
        secret_preview: impl Into<String>,
        scopes: BTreeSet<ApiScope>,
        created_at: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        if scopes.is_empty() {
            return Err(ManagedApiError::InvalidIdentifier {
                field: "scopes",
                reason: "at least one explicit scope is required",
            });
        }

        let name = require_non_empty("name", name.into())?;
        let secret_preview = require_non_empty("secret_preview", secret_preview.into())?;
        let created_at = require_non_empty("created_at", created_at.into())?;

        Ok(Self {
            tenant_id,
            key_id,
            name,
            secret_digest,
            secret_preview,
            scopes,
            status: ApiKeyStatus::Active,
            created_at,
        })
    }

    /// Return whether this active key carries a specific scope.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{ApiKeyId, ApiKeyRecord, ApiKeySecretDigest, ApiScope, TenantId};
    /// # use std::collections::BTreeSet;
    /// let key = ApiKeyRecord::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     ApiKeyId::new("key_123").unwrap(),
    ///     "CI key",
    ///     ApiKeySecretDigest::new("sha256", "digest").unwrap(),
    ///     "1234",
    ///     BTreeSet::from([ApiScope::TenantAdmin]),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap();
    /// assert!(key.allows_scope(ApiScope::AuditRead));
    /// ```
    #[must_use]
    pub fn allows_scope(&self, scope: ApiScope) -> bool {
        matches!(self.status, ApiKeyStatus::Active)
            && (self.scopes.contains(&ApiScope::TenantAdmin) || self.scopes.contains(&scope))
    }

    /// Require that this key carries a specific scope.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::MissingScope`] when the key is revoked or the
    /// requested scope is not explicitly granted by the key's scope set.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{ApiKeyId, ApiKeyRecord, ApiKeySecretDigest, ApiScope, TenantId};
    /// # use std::collections::BTreeSet;
    /// let key = ApiKeyRecord::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     ApiKeyId::new("key_123").unwrap(),
    ///     "CI key",
    ///     ApiKeySecretDigest::new("sha256", "digest").unwrap(),
    ///     "1234",
    ///     BTreeSet::from([ApiScope::InvoiceRead]),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap();
    /// key.require_scope(ApiScope::InvoiceRead).unwrap();
    /// ```
    pub fn require_scope(&self, scope: ApiScope) -> Result<(), ManagedApiError> {
        if self.allows_scope(scope) {
            Ok(())
        } else {
            Err(ManagedApiError::MissingScope { scope })
        }
    }
}

impl TenantScoped for ApiKeyRecord {
    fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// Managed-layer permission checked by RBAC roles and API scopes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Manage tenant settings.
    ManageTenant,
    /// Create, rotate, or revoke API keys.
    ManageApiKeys,
    /// Read invoices.
    ReadInvoices,
    /// Create or update invoices.
    WriteInvoices,
    /// Validate invoices.
    ValidateInvoices,
    /// Render invoices.
    RenderInvoices,
    /// Transmit invoices through gateways.
    TransmitInvoices,
    /// Read archived evidence bundles.
    ReadArchive,
    /// Read audit events.
    ReadAudit,
}

/// Built-in tenant roles.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Full tenant administrator.
    Admin,
    /// Operational user who can work with invoices but cannot administer access.
    Member,
    /// Read-only tenant user.
    Viewer,
}

impl Role {
    /// Return whether this role grants a permission.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Permission, Role};
    /// assert!(Role::Admin.allows(Permission::ManageApiKeys));
    /// assert!(!Role::Viewer.allows(Permission::WriteInvoices));
    /// ```
    #[must_use]
    pub const fn allows(self, permission: Permission) -> bool {
        match self {
            Self::Admin => true,
            Self::Member => matches!(
                permission,
                Permission::ReadInvoices
                    | Permission::WriteInvoices
                    | Permission::ValidateInvoices
                    | Permission::RenderInvoices
                    | Permission::TransmitInvoices
                    | Permission::ReadArchive
            ),
            Self::Viewer => matches!(
                permission,
                Permission::ReadInvoices
                    | Permission::ValidateInvoices
                    | Permission::RenderInvoices
            ),
        }
    }

    /// Require that this role grants a permission.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::PermissionDenied`] when this role does not
    /// grant the requested permission.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Permission, Role};
    /// Role::Admin.require_permission(Permission::ManageTenant).unwrap();
    /// ```
    pub fn require_permission(self, permission: Permission) -> Result<(), ManagedApiError> {
        if self.allows(permission) {
            Ok(())
        } else {
            Err(ManagedApiError::PermissionDenied {
                role: self,
                permission,
            })
        }
    }
}

/// Tenant membership connecting a principal to one RBAC role.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Membership {
    /// Tenant that owns the membership.
    pub tenant_id: TenantId,
    /// Principal receiving the role.
    pub principal_id: PrincipalId,
    /// Role granted inside the tenant.
    pub role: Role,
}

impl Membership {
    /// Build a tenant membership.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Membership, PrincipalId, Role, TenantId};
    /// let membership = Membership::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     PrincipalId::new("user_123").unwrap(),
    ///     Role::Admin,
    /// );
    /// assert_eq!(membership.role, Role::Admin);
    /// ```
    #[must_use]
    pub const fn new(tenant_id: TenantId, principal_id: PrincipalId, role: Role) -> Self {
        Self {
            tenant_id,
            principal_id,
            role,
        }
    }

    /// Require that the membership role grants a permission.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::PermissionDenied`] when the role does not
    /// grant the requested permission.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Membership, Permission, PrincipalId, Role, TenantId};
    /// let membership = Membership::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     PrincipalId::new("user_123").unwrap(),
    ///     Role::Member,
    /// );
    /// membership.require_permission(Permission::WriteInvoices).unwrap();
    /// ```
    pub fn require_permission(&self, permission: Permission) -> Result<(), ManagedApiError> {
        self.role.require_permission(permission)
    }
}

impl TenantScoped for Membership {
    fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// Supported `OpenID` Connect providers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OidcProvider {
    /// Google `OpenID` Connect.
    Google,
}

/// Typed Google OIDC client configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GoogleOidcConfig {
    /// OAuth client ID issued by Google.
    pub client_id: String,
    /// HTTPS redirect URI registered with Google.
    pub redirect_uri: String,
    /// Optional Google Workspace hosted-domain restriction.
    pub hosted_domain: Option<String>,
}

impl GoogleOidcConfig {
    /// Build Google OIDC configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::OidcConfigInvalid`] if the client ID is empty
    /// or the redirect URI is not HTTPS. Localhost exceptions belong to dev
    /// tooling, not this production contract.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::GoogleOidcConfig;
    /// let cfg = GoogleOidcConfig::new(
    ///     "client.apps.googleusercontent.com",
    ///     "https://app.invoicekit.example/oidc/callback",
    ///     None,
    /// ).unwrap();
    /// assert_eq!(GoogleOidcConfig::discovery_document_uri(), "https://accounts.google.com/.well-known/openid-configuration");
    /// ```
    pub fn new(
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        hosted_domain: Option<String>,
    ) -> Result<Self, ManagedApiError> {
        let client_id = require_non_empty("client_id", client_id.into())?;
        let redirect_uri = require_non_empty("redirect_uri", redirect_uri.into())?;
        if !redirect_uri.starts_with("https://") {
            return Err(ManagedApiError::OidcConfigInvalid {
                reason: "redirect_uri must use https",
            });
        }

        if hosted_domain
            .as_ref()
            .is_some_and(|domain| domain.trim().is_empty() || domain.contains(char::is_whitespace))
        {
            return Err(ManagedApiError::OidcConfigInvalid {
                reason: "hosted_domain must be non-empty and contain no whitespace",
            });
        }

        Ok(Self {
            client_id,
            redirect_uri,
            hosted_domain,
        })
    }

    /// Return Google's discovery document URI.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{GoogleOidcConfig, GOOGLE_DISCOVERY_DOCUMENT_URI};
    /// assert_eq!(GoogleOidcConfig::discovery_document_uri(), GOOGLE_DISCOVERY_DOCUMENT_URI);
    /// ```
    #[must_use]
    pub const fn discovery_document_uri() -> &'static str {
        GOOGLE_DISCOVERY_DOCUMENT_URI
    }

    /// Build a Google authorization URL using Authorization Code + PKCE.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::OidcConfigInvalid`] if `state`, `nonce`, or
    /// `pkce_code_challenge` is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::GoogleOidcConfig;
    /// let cfg = GoogleOidcConfig::new("client", "https://example.com/cb", None).unwrap();
    /// let url = cfg.authorization_url("state-1", "nonce-1", "challenge").unwrap();
    /// assert!(url.contains("response_type=code"));
    /// assert!(url.contains("code_challenge_method=S256"));
    /// ```
    pub fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        pkce_code_challenge: &str,
    ) -> Result<String, ManagedApiError> {
        let state = require_non_empty("state", state.to_owned())?;
        let nonce = require_non_empty("nonce", nonce.to_owned())?;
        let pkce_code_challenge =
            require_non_empty("pkce_code_challenge", pkce_code_challenge.to_owned())?;

        let mut query = vec![
            ("client_id", percent_encode_component(&self.client_id)),
            ("redirect_uri", percent_encode_component(&self.redirect_uri)),
            ("response_type", "code".to_owned()),
            ("scope", percent_encode_component(GOOGLE_SCOPE)),
            ("state", percent_encode_component(&state)),
            ("nonce", percent_encode_component(&nonce)),
            (
                "code_challenge",
                percent_encode_component(&pkce_code_challenge),
            ),
            ("code_challenge_method", "S256".to_owned()),
        ];

        if let Some(hosted_domain) = &self.hosted_domain {
            query.push(("hd", percent_encode_component(hosted_domain)));
        }

        let query = query
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&");

        Ok(format!("{GOOGLE_AUTHORIZATION_ENDPOINT}?{query}"))
    }

    /// Accept already signature-verified Google ID-token claims.
    ///
    /// This function validates tenant-model invariants after a gateway has
    /// verified the JWT signature against Google's `jwks_uri`: issuer,
    /// audience, expiry, issued-at skew, verified email, and optional hosted
    /// domain.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::OidcClaimRejected`] when any required Google
    /// OIDC claim invariant fails.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{GoogleIdTokenClaims, GoogleOidcConfig, GOOGLE_ISSUER};
    /// let cfg = GoogleOidcConfig::new("client", "https://example.com/cb", None).unwrap();
    /// let identity = cfg.accept_verified_claims(
    ///     &GoogleIdTokenClaims {
    ///         issuer: GOOGLE_ISSUER.to_owned(),
    ///         subject: "google-subject".to_owned(),
    ///         audience: vec!["client".to_owned()],
    ///         email: "user@example.com".to_owned(),
    ///         email_verified: true,
    ///         hosted_domain: None,
    ///         expires_at: 2_000,
    ///         issued_at: 1_000,
    ///     },
    ///     1_500,
    /// ).unwrap();
    /// assert_eq!(identity.email, "user@example.com");
    /// ```
    pub fn accept_verified_claims(
        &self,
        claims: &GoogleIdTokenClaims,
        now_unix_seconds: u64,
    ) -> Result<OidcIdentity, ManagedApiError> {
        if claims.issuer != GOOGLE_ISSUER {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "issuer must be https://accounts.google.com",
            });
        }
        if !claims.audience.iter().any(|aud| aud == &self.client_id) {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "audience does not include configured client_id",
            });
        }
        if claims.expires_at <= now_unix_seconds {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "token is expired",
            });
        }
        if claims.issued_at > now_unix_seconds + 300 {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "issued-at is too far in the future",
            });
        }
        if !claims.email_verified {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "email must be verified",
            });
        }
        if claims.subject.trim().is_empty() {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "subject must not be empty",
            });
        }
        if claims.email.trim().is_empty() || !claims.email.contains('@') {
            return Err(ManagedApiError::OidcClaimRejected {
                reason: "email must be present",
            });
        }
        if let Some(required_domain) = &self.hosted_domain {
            if claims.hosted_domain.as_deref() != Some(required_domain.as_str()) {
                return Err(ManagedApiError::OidcClaimRejected {
                    reason: "hosted domain does not match configuration",
                });
            }
        }

        Ok(OidcIdentity {
            provider: OidcProvider::Google,
            subject: claims.subject.clone(),
            email: claims.email.clone(),
            hosted_domain: claims.hosted_domain.clone(),
        })
    }
}

/// Google ID-token claims after JWT signature verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GoogleIdTokenClaims {
    /// ID-token issuer.
    pub issuer: String,
    /// Stable Google subject identifier.
    pub subject: String,
    /// ID-token audience values.
    pub audience: Vec<String>,
    /// User email claim.
    pub email: String,
    /// Whether Google says the email is verified.
    pub email_verified: bool,
    /// Optional Google Workspace hosted-domain claim.
    pub hosted_domain: Option<String>,
    /// Expiry time as Unix seconds.
    pub expires_at: u64,
    /// Issued-at time as Unix seconds.
    pub issued_at: u64,
}

/// Accepted `OpenID` Connect identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OidcIdentity {
    /// Provider that issued the identity.
    pub provider: OidcProvider,
    /// Provider-stable subject identifier.
    pub subject: String,
    /// Verified email address.
    pub email: String,
    /// Optional hosted-domain claim.
    pub hosted_domain: Option<String>,
}

/// Audit action names emitted by the managed layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// Tenant was created.
    TenantCreated,
    /// API key was created.
    ApiKeyCreated,
    /// API key was revoked.
    ApiKeyRevoked,
    /// OIDC login succeeded.
    OidcLoginSucceeded,
    /// OIDC login was rejected.
    OidcLoginRejected,
    /// RBAC role was assigned.
    RoleAssigned,
    /// RBAC role was removed.
    RoleRemoved,
    /// Invoice validation was requested.
    InvoiceValidated,
    /// Invoice was transmitted.
    InvoiceTransmitted,
}

/// Audit event outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// Operation succeeded.
    Succeeded,
    /// Operation was denied by authorization.
    Denied,
    /// Operation failed after authorization.
    Failed,
}

/// Audit target descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditTarget {
    /// Target resource kind, such as `invoice`, `api_key`, or `tenant`.
    pub kind: String,
    /// Target resource identifier.
    pub id: String,
}

impl AuditTarget {
    /// Build an audit target descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if `kind` or `id` is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::AuditTarget;
    /// let target = AuditTarget::new("invoice", "inv_123").unwrap();
    /// assert_eq!(target.kind, "invoice");
    /// ```
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Result<Self, ManagedApiError> {
        Ok(Self {
            kind: require_non_empty("target.kind", kind.into())?,
            id: require_non_empty("target.id", id.into())?,
        })
    }
}

/// Customer-facing audit event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Stable audit event identifier.
    pub event_id: AuditEventId,
    /// Tenant that owns the event.
    pub tenant_id: TenantId,
    /// Trace identifier linking this event to request logs and gateway attempts.
    pub trace_id: TraceId,
    /// Actor that attempted the operation.
    pub actor: Actor,
    /// Action being recorded.
    pub action: AuditAction,
    /// Operation outcome.
    pub outcome: AuditOutcome,
    /// Target resource.
    pub target: AuditTarget,
    /// RFC 3339 event timestamp.
    pub occurred_at: String,
    /// Small string metadata map. Secrets must not be stored here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl AuditEvent {
    /// Build an audit event.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] if `occurred_at` is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Actor, AuditAction, AuditEvent, AuditEventId, AuditOutcome, AuditTarget, TenantId, TenantRequestContext, TraceId};
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     Actor::System { name: "managed-api".to_owned() },
    /// );
    /// let event = AuditEvent::new(
    ///     AuditEventId::new("aud_123").unwrap(),
    ///     &ctx,
    ///     AuditAction::TenantCreated,
    ///     AuditOutcome::Succeeded,
    ///     AuditTarget::new("tenant", "tenant_acme").unwrap(),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap();
    /// assert_eq!(event.tenant_id.as_str(), "tenant_acme");
    /// ```
    pub fn new(
        event_id: AuditEventId,
        context: &TenantRequestContext,
        action: AuditAction,
        outcome: AuditOutcome,
        target: AuditTarget,
        occurred_at: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        Ok(Self {
            event_id,
            tenant_id: context.tenant_id.clone(),
            trace_id: context.trace_id.clone(),
            actor: context.actor.clone(),
            action,
            outcome,
            target,
            occurred_at: require_non_empty("occurred_at", occurred_at.into())?,
            metadata: BTreeMap::new(),
        })
    }

    /// Attach non-secret string metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ManagedApiError::InvalidIdentifier`] when the metadata key or
    /// value is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_managed_api::{Actor, AuditAction, AuditEvent, AuditEventId, AuditOutcome, AuditTarget, TenantId, TenantRequestContext, TraceId};
    /// let ctx = TenantRequestContext::new(
    ///     TenantId::new("tenant_acme").unwrap(),
    ///     TraceId::new("trace_123").unwrap(),
    ///     Actor::System { name: "managed-api".to_owned() },
    /// );
    /// let event = AuditEvent::new(
    ///     AuditEventId::new("aud_123").unwrap(),
    ///     &ctx,
    ///     AuditAction::TenantCreated,
    ///     AuditOutcome::Succeeded,
    ///     AuditTarget::new("tenant", "tenant_acme").unwrap(),
    ///     "2026-05-26T00:00:00Z",
    /// ).unwrap().with_metadata("source", "unit-test").unwrap();
    /// assert_eq!(event.metadata.get("source").unwrap(), "unit-test");
    /// ```
    pub fn with_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ManagedApiError> {
        let key = require_non_empty("metadata.key", key.into())?;
        let value = require_non_empty("metadata.value", value.into())?;
        self.metadata.insert(key, value);
        Ok(self)
    }
}

impl TenantScoped for AuditEvent {
    fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// Return the documented JSON Schema for customer-facing audit events.
///
/// The schema is intentionally hand-authored for now so the public audit event
/// contract is explicit and stable. T-142 can publish this behind
/// `/v1/audit/events` without changing the event shape.
///
/// # Examples
///
/// ```
/// # use invoicekit_managed_api::audit_event_json_schema;
/// let schema = audit_event_json_schema();
/// assert_eq!(schema["title"], "InvoiceKitAuditEvent");
/// ```
#[must_use]
pub fn audit_event_json_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "InvoiceKitAuditEvent",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "event_id",
            "tenant_id",
            "trace_id",
            "actor",
            "action",
            "outcome",
            "target",
            "occurred_at"
        ],
        "properties": {
            "event_id": { "type": "string", "minLength": 1 },
            "tenant_id": { "type": "string", "minLength": 1 },
            "trace_id": { "type": "string", "minLength": 1 },
            "actor": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": { "enum": ["api_key", "principal", "system"] },
                    "key_id": { "type": "string" },
                    "principal_id": { "type": "string" },
                    "name": { "type": "string" }
                }
            },
            "action": {
                "enum": [
                    "tenant_created",
                    "api_key_created",
                    "api_key_revoked",
                    "oidc_login_succeeded",
                    "oidc_login_rejected",
                    "role_assigned",
                    "role_removed",
                    "invoice_validated",
                    "invoice_transmitted"
                ]
            },
            "outcome": { "enum": ["succeeded", "denied", "failed"] },
            "target": {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "id"],
                "properties": {
                    "kind": { "type": "string", "minLength": 1 },
                    "id": { "type": "string", "minLength": 1 }
                }
            },
            "occurred_at": { "type": "string", "format": "date-time" },
            "metadata": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        }
    })
}

/// Managed API domain error.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ManagedApiError {
    /// Identifier validation failed.
    #[error("invalid {field}: {reason}; use 1-128 ASCII chars from A-Z, a-z, 0-9, '_', '.', ':', '-' with no surrounding whitespace")]
    InvalidIdentifier {
        /// Field name.
        field: &'static str,
        /// Remediation-oriented reason.
        reason: &'static str,
    },
    /// API key does not grant a requested scope.
    #[error(
        "API key is missing required scope {scope}; grant that explicit scope or use tenant:admin"
    )]
    MissingScope {
        /// Required scope.
        scope: ApiScope,
    },
    /// RBAC role does not grant a requested permission.
    #[error("role {role:?} does not grant permission {permission:?}; assign a stronger role or request a narrower operation")]
    PermissionDenied {
        /// Effective role.
        role: Role,
        /// Required permission.
        permission: Permission,
    },
    /// Tenant-scoped values crossed tenant boundaries.
    #[error("tenant mismatch: expected {expected}, got {actual}; route the operation to the tenant selected by the credential")]
    TenantMismatch {
        /// Expected tenant from request context.
        expected: TenantId,
        /// Tenant found on a scoped value.
        actual: TenantId,
    },
    /// OIDC configuration is invalid.
    #[error("OIDC configuration rejected: {reason}; configure a non-empty Google client ID and HTTPS redirect URI")]
    OidcConfigInvalid {
        /// Rejection reason.
        reason: &'static str,
    },
    /// OIDC claims failed local validation after signature verification.
    #[error("OIDC claim rejected: {reason}; verify Google discovery metadata, audience, hosted domain, and token freshness")]
    OidcClaimRejected {
        /// Rejection reason.
        reason: &'static str,
    },
}

fn validate_identifier(field: &'static str, value: String) -> Result<String, ManagedApiError> {
    if value.is_empty() {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "value must not be empty",
        });
    }
    if value.len() > 128 {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "value is longer than 128 bytes",
        });
    }
    if value.trim() != value {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "value must not contain surrounding whitespace",
        });
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        return Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "value contains unsupported characters",
        });
    }
    Ok(value)
}

fn require_non_empty(field: &'static str, value: String) -> Result<String, ManagedApiError> {
    if value.trim().is_empty() {
        Err(ManagedApiError::InvalidIdentifier {
            field,
            reason: "value must not be empty",
        })
    } else {
        Ok(value)
    }
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

const fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => '0',
    }
}

#[cfg(test)]
mod tests {
    use super::{
        audit_event_json_schema, percent_encode_component, Actor, ApiKeyId, ApiKeyRecord,
        ApiKeySecretDigest, ApiKeyStatus, ApiScope, AuditAction, AuditEvent, AuditEventId,
        AuditOutcome, AuditTarget, GoogleIdTokenClaims, GoogleOidcConfig, ManagedApiError,
        Membership, Permission, PrincipalId, Role, TenantId, TenantRequestContext, TenantScoped,
        TraceId, GOOGLE_DISCOVERY_DOCUMENT_URI, GOOGLE_ISSUER,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(super::crate_name(), "invoicekit-managed-api");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!super::crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = super::crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = super::crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn tenant_context_rejects_cross_tenant_resources() {
        let ctx = TenantRequestContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new("trace_123").unwrap(),
            Actor::System {
                name: "managed-api".to_owned(),
            },
        );
        let other_key = api_key_for("tenant_other", BTreeSet::from([ApiScope::InvoiceRead]));

        let err = ctx.require_same_tenant(&other_key).unwrap_err();
        assert_eq!(
            err,
            ManagedApiError::TenantMismatch {
                expected: TenantId::new("tenant_acme").unwrap(),
                actual: TenantId::new("tenant_other").unwrap(),
            }
        );
    }

    #[test]
    fn scoped_api_key_grants_only_explicit_scopes() {
        let key = api_key_for(
            "tenant_acme",
            BTreeSet::from([ApiScope::InvoiceRead, ApiScope::InvoiceValidate]),
        );

        assert!(key.allows_scope(ApiScope::InvoiceRead));
        assert!(key.require_scope(ApiScope::InvoiceValidate).is_ok());
        assert_eq!(
            key.require_scope(ApiScope::InvoiceTransmit).unwrap_err(),
            ManagedApiError::MissingScope {
                scope: ApiScope::InvoiceTransmit,
            }
        );
    }

    #[test]
    fn revoked_api_key_grants_no_scopes() {
        let mut key = api_key_for("tenant_acme", BTreeSet::from([ApiScope::TenantAdmin]));
        key.status = ApiKeyStatus::Revoked;

        assert!(!key.allows_scope(ApiScope::AuditRead));
    }

    #[test]
    fn api_key_requires_at_least_one_scope() {
        let err = ApiKeyRecord::new(
            TenantId::new("tenant_acme").unwrap(),
            ApiKeyId::new("key_123").unwrap(),
            "CI key",
            ApiKeySecretDigest::new("sha256", "digest").unwrap(),
            "1234",
            BTreeSet::new(),
            "2026-05-26T00:00:00Z",
        )
        .unwrap_err();

        assert_eq!(
            err,
            ManagedApiError::InvalidIdentifier {
                field: "scopes",
                reason: "at least one explicit scope is required",
            }
        );
    }

    #[test]
    fn rbac_roles_are_monotonic() {
        assert!(Role::Admin.allows(Permission::ManageApiKeys));
        assert!(Role::Member.allows(Permission::WriteInvoices));
        assert!(!Role::Member.allows(Permission::ManageApiKeys));
        assert!(Role::Viewer.allows(Permission::ReadInvoices));
        assert!(!Role::Viewer.allows(Permission::TransmitInvoices));
    }

    #[test]
    fn membership_carries_tenant_and_role() {
        let membership = Membership::new(
            TenantId::new("tenant_acme").unwrap(),
            PrincipalId::new("user_123").unwrap(),
            Role::Member,
        );

        assert_eq!(membership.tenant_id().as_str(), "tenant_acme");
        assert!(membership
            .require_permission(Permission::WriteInvoices)
            .is_ok());
        assert!(membership
            .require_permission(Permission::ManageTenant)
            .is_err());
    }

    #[test]
    fn google_oidc_config_builds_authorization_url() {
        let cfg = GoogleOidcConfig::new(
            "client.apps.googleusercontent.com",
            "https://app.invoicekit.example/oidc/callback",
            Some("example.com".to_owned()),
        )
        .unwrap();

        let url = cfg
            .authorization_url("state 1", "nonce/1", "challenge+1")
            .unwrap();

        assert_eq!(
            GoogleOidcConfig::discovery_document_uri(),
            GOOGLE_DISCOVERY_DOCUMENT_URI
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid%20email%20profile"));
        assert!(url.contains("state=state%201"));
        assert!(url.contains("nonce=nonce%2F1"));
        assert!(url.contains("code_challenge=challenge%2B1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("hd=example.com"));
    }

    #[test]
    fn google_oidc_config_requires_https_redirect_uri() {
        let err = GoogleOidcConfig::new("client", "http://example.com/cb", None).unwrap_err();

        assert_eq!(
            err,
            ManagedApiError::OidcConfigInvalid {
                reason: "redirect_uri must use https",
            }
        );
    }

    #[test]
    fn google_oidc_accepts_verified_claims() {
        let cfg = GoogleOidcConfig::new(
            "client",
            "https://example.com/cb",
            Some("example.com".into()),
        )
        .unwrap();
        let identity = cfg
            .accept_verified_claims(&valid_google_claims(), 1_500)
            .unwrap();

        assert_eq!(identity.email, "user@example.com");
        assert_eq!(identity.hosted_domain.as_deref(), Some("example.com"));
    }

    #[test]
    fn google_oidc_rejects_bad_issuer() {
        let cfg = GoogleOidcConfig::new("client", "https://example.com/cb", None).unwrap();
        let mut claims = valid_google_claims();
        claims.issuer = "https://issuer.example".to_owned();

        assert_eq!(
            cfg.accept_verified_claims(&claims, 1_500).unwrap_err(),
            ManagedApiError::OidcClaimRejected {
                reason: "issuer must be https://accounts.google.com",
            }
        );
    }

    #[test]
    fn google_oidc_rejects_audience_mismatch() {
        let cfg =
            GoogleOidcConfig::new("different-client", "https://example.com/cb", None).unwrap();

        assert_eq!(
            cfg.accept_verified_claims(&valid_google_claims(), 1_500)
                .unwrap_err(),
            ManagedApiError::OidcClaimRejected {
                reason: "audience does not include configured client_id",
            }
        );
    }

    #[test]
    fn google_oidc_rejects_unverified_email() {
        let cfg = GoogleOidcConfig::new("client", "https://example.com/cb", None).unwrap();
        let mut claims = valid_google_claims();
        claims.email_verified = false;

        assert_eq!(
            cfg.accept_verified_claims(&claims, 1_500).unwrap_err(),
            ManagedApiError::OidcClaimRejected {
                reason: "email must be verified",
            }
        );
    }

    #[test]
    fn google_oidc_rejects_expired_token() {
        let cfg = GoogleOidcConfig::new("client", "https://example.com/cb", None).unwrap();

        assert_eq!(
            cfg.accept_verified_claims(&valid_google_claims(), 2_001)
                .unwrap_err(),
            ManagedApiError::OidcClaimRejected {
                reason: "token is expired",
            }
        );
    }

    #[test]
    fn audit_event_serializes_with_required_tenant_trace_and_actor() {
        let ctx = TenantRequestContext::new(
            TenantId::new("tenant_acme").unwrap(),
            TraceId::new("trace_123").unwrap(),
            Actor::Principal {
                principal_id: PrincipalId::new("user_123").unwrap(),
            },
        );
        let event = AuditEvent::new(
            AuditEventId::new("aud_123").unwrap(),
            &ctx,
            AuditAction::InvoiceValidated,
            AuditOutcome::Succeeded,
            AuditTarget::new("invoice", "inv_123").unwrap(),
            "2026-05-26T00:00:00Z",
        )
        .unwrap()
        .with_metadata("rulepack", "en16931-2024")
        .unwrap();

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["tenant_id"], "tenant_acme");
        assert_eq!(value["trace_id"], "trace_123");
        assert_eq!(
            value["actor"],
            json!({"kind": "principal", "principal_id": "user_123"})
        );
        assert_eq!(value["metadata"]["rulepack"], "en16931-2024");
    }

    #[test]
    fn audit_event_schema_documents_required_fields() {
        let schema = audit_event_json_schema();
        assert_eq!(schema["title"], "InvoiceKitAuditEvent");
        assert_eq!(
            schema["required"],
            json!([
                "event_id",
                "tenant_id",
                "trace_id",
                "actor",
                "action",
                "outcome",
                "target",
                "occurred_at"
            ])
        );
        assert!(schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("oidc_login_succeeded")));
    }

    #[test]
    fn identifier_validation_rejects_whitespace() {
        assert_eq!(
            TenantId::new(" tenant").unwrap_err(),
            ManagedApiError::InvalidIdentifier {
                field: "tenant_id",
                reason: "value must not contain surrounding whitespace",
            }
        );
    }

    #[test]
    fn percent_encoding_uses_uppercase_hex() {
        assert_eq!(
            percent_encode_component("space/slash+plus"),
            "space%2Fslash%2Bplus"
        );
    }

    fn api_key_for(tenant: &str, scopes: BTreeSet<ApiScope>) -> ApiKeyRecord {
        ApiKeyRecord::new(
            TenantId::new(tenant).unwrap(),
            ApiKeyId::new("key_123").unwrap(),
            "CI key",
            ApiKeySecretDigest::new("sha256", "digest").unwrap(),
            "1234",
            scopes,
            "2026-05-26T00:00:00Z",
        )
        .unwrap()
    }

    fn valid_google_claims() -> GoogleIdTokenClaims {
        GoogleIdTokenClaims {
            issuer: GOOGLE_ISSUER.to_owned(),
            subject: "google-subject".to_owned(),
            audience: vec!["client".to_owned()],
            email: "user@example.com".to_owned(),
            email_verified: true,
            hosted_domain: Some("example.com".to_owned()),
            expires_at: 2_000,
            issued_at: 1_000,
        }
    }
}
