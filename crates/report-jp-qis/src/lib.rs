// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Japan **Qualified Invoice System** (QIS / 適格請求書発行事業者制度) adapter.
//!
//! Japan's National Tax Agency (NTA, 国税庁) launched QIS in
//! October 2023. To claim input JCT (Japanese Consumption Tax)
//! credit, a buyer must receive a **qualified invoice**
//! (適格請求書) carrying the issuer's NTA-issued registration
//! number — the letter `T` followed by 13 ASCII digits.
//!
//! Japan does NOT operate a clearance portal; the NTA only
//! runs a registration registry the buyer can look up to
//! confirm the issuer is registered. Wire delivery is via
//! Peppol-JP (Peppol BIS Billing 3 with the Japanese CIUS) —
//! the engine delegates the AS4 send to
//! `crates/transmit-peppol`.
//!
//! This crate ships the typed JP-specific overlay: issuer
//! registration validation, JCT rate enum, qualified vs
//! simplified invoice kinds, and the registry lookup trait
//! the engine pings before delivery.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the NTA registry lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NtaEnvironment {
    /// `kokuzei-test.nta.go.jp` / sandbox.
    Sandbox,
    /// `kokuzei.nta.go.jp` / production.
    Production,
}

/// Which kind of qualified invoice the issuer is producing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QisInvoiceKind {
    /// Full qualified invoice (適格請求書) — required for
    /// most B2B transactions.
    Qualified,
    /// Simplified qualified invoice (適格簡易請求書) — allowed
    /// for retail / restaurant / transport / parking
    /// industries where the buyer's name may be omitted.
    Simplified,
}

/// JCT category. The 10% standard rate covers most goods;
/// the 8% reduced rate covers food + newspapers; 0% is
/// exports + zero-rated supplies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JctCategory {
    /// 10% standard rate.
    Standard10,
    /// 8% reduced rate (food, newspapers).
    Reduced8,
    /// 0% (exports, zero-rated supplies).
    Zero,
    /// Exempt (medical, social welfare, etc.).
    Exempt,
}

/// What the engine knows about an issuer's NTA registration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QisIssuerRegistration {
    /// Registration number — `T` + 13 ASCII digits.
    pub registration_number: String,
    /// Registered legal name (for buyer-side reconciliation).
    pub legal_name: String,
    /// Effective registration date (RFC-3339).
    pub effective_from: String,
    /// `None` when the registration is currently active;
    /// `Some(date)` when revoked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

/// What the operator passes in to
/// [`QisRegistryProvider::lookup`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QisLookupRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: NtaEnvironment,
    /// Registration number to look up — `T` + 13 ASCII
    /// digits.
    pub registration_number: String,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum QisError {
    /// Registration number didn't match `T` + 13 ASCII
    /// digits.
    #[error("invalid registration number: {0}")]
    BadRegistrationNumber(String),
    /// NTA registry returned `not found`.
    #[error("registration not found in NTA registry: {0}")]
    NotFound(String),
    /// HTTP / TLS / DNS failure talking to the NTA.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// NTA registry lookup surface.
pub trait QisRegistryProvider: Send + Sync {
    /// Look up an issuer's registration with the NTA. The
    /// engine pings this before delivery so a buyer that
    /// receives the invoice can claim JCT input credit.
    ///
    /// # Errors
    ///
    /// Returns [`QisError`] when the registration number
    /// has a wrong shape, the registry doesn't recognise
    /// it, or transport fails.
    fn lookup(&self, request: &QisLookupRequest) -> Result<QisIssuerRegistration, QisError>;
}

/// Deterministic mock registry.
///
/// Resolves any well-formed registration number (`T` + 13
/// digits) to a synthetic registration record. Operator code
/// can populate `revoked_for` to flip specific numbers into
/// the revoked state for cassette-replay tests.
pub struct MockQisRegistryProvider {
    fixed_effective_from: String,
    revoked_for: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl MockQisRegistryProvider {
    /// Build a mock registry with deterministic effective
    /// dates.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_effective_from("2023-10-01T00:00:00Z")
    }

    /// Build a mock with a custom effective_from date.
    #[must_use]
    pub fn with_fixed_effective_from(effective_from: impl Into<String>) -> Self {
        Self {
            fixed_effective_from: effective_from.into(),
            revoked_for: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// Flip a registration number into the revoked state for
    /// subsequent `lookup` calls.
    ///
    /// # Panics
    ///
    /// Panics if another thread holds the revoke mutex and
    /// has poisoned it (test-only `Mutex`; the panic surfaces
    /// the corruption rather than masking it).
    pub fn revoke(&self, registration_number: impl Into<String>) {
        let mut g = self.revoked_for.lock().expect("revoke mutex poisoned");
        g.insert(registration_number.into());
    }
}

impl Default for MockQisRegistryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl QisRegistryProvider for MockQisRegistryProvider {
    fn lookup(&self, request: &QisLookupRequest) -> Result<QisIssuerRegistration, QisError> {
        validate_registration_number(&request.registration_number)?;
        let revoked_at = {
            let g = self.revoked_for.lock().expect("revoke mutex poisoned");
            g.contains(&request.registration_number)
                .then_some("2025-12-31T23:59:59Z".to_owned())
        };
        Ok(QisIssuerRegistration {
            registration_number: request.registration_number.clone(),
            legal_name: format!("Mock JP Issuer {}", &request.registration_number[1..5]),
            effective_from: self.fixed_effective_from.clone(),
            revoked_at,
        })
    }
}

/// Validate a Japanese QIS registration number — `T` prefix
/// + 13 ASCII digits.
///
/// # Errors
///
/// Returns [`QisError::BadRegistrationNumber`] on shape
/// failure.
pub fn validate_registration_number(value: &str) -> Result<(), QisError> {
    if value.len() == 14
        && value.starts_with('T')
        && value.bytes().skip(1).all(|b| b.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(QisError::BadRegistrationNumber(format!(
            "must be `T` + 13 ASCII digits, got {value:?}"
        )))
    }
}

/// JCT rate basis points for arithmetic. The standard rate
/// is 1000 bp (10%), reduced is 800 bp (8%), zero/exempt are
/// 0 bp.
#[must_use]
pub const fn jct_basis_points(category: JctCategory) -> u16 {
    match category {
        JctCategory::Standard10 => 1000,
        JctCategory::Reduced8 => 800,
        JctCategory::Zero | JctCategory::Exempt => 0,
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_jp_qis::crate_name(),
///     "invoicekit-report-jp-qis"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-jp-qis"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lookup() -> QisLookupRequest {
        QisLookupRequest {
            tenant_id: "tenant-jp-test".to_owned(),
            environment: NtaEnvironment::Sandbox,
            registration_number: "T1234567890123".to_owned(),
        }
    }

    #[test]
    fn lookup_returns_active_registration() {
        let p = MockQisRegistryProvider::default();
        let reg = p.lookup(&sample_lookup()).unwrap();
        assert_eq!(reg.registration_number, "T1234567890123");
        assert!(reg.revoked_at.is_none());
        assert_eq!(reg.effective_from, "2023-10-01T00:00:00Z");
    }

    #[test]
    fn lookup_returns_revoked_when_flipped() {
        let p = MockQisRegistryProvider::default();
        p.revoke("T1234567890123");
        let reg = p.lookup(&sample_lookup()).unwrap();
        assert_eq!(reg.revoked_at.as_deref(), Some("2025-12-31T23:59:59Z"));
    }

    #[test]
    fn lookup_rejects_bad_registration_number() {
        let p = MockQisRegistryProvider::default();
        let mut req = sample_lookup();
        req.registration_number = "X12345".to_owned();
        let err = p.lookup(&req).unwrap_err();
        assert!(matches!(err, QisError::BadRegistrationNumber(_)));
    }

    #[test]
    fn validate_registration_number_round_trip() {
        assert!(validate_registration_number("T1234567890123").is_ok());
        assert!(validate_registration_number("S1234567890123").is_err());
        assert!(validate_registration_number("T123456789012").is_err());
        assert!(validate_registration_number("T12345678901234").is_err());
        assert!(validate_registration_number("T1234567890ABC").is_err());
    }

    #[test]
    fn jct_basis_points_maps_each_category() {
        assert_eq!(jct_basis_points(JctCategory::Standard10), 1000);
        assert_eq!(jct_basis_points(JctCategory::Reduced8), 800);
        assert_eq!(jct_basis_points(JctCategory::Zero), 0);
        assert_eq!(jct_basis_points(JctCategory::Exempt), 0);
    }

    #[test]
    fn registration_round_trips_through_serde() {
        let reg = QisIssuerRegistration {
            registration_number: "T1234567890123".to_owned(),
            legal_name: "Mock JP Issuer 1234".to_owned(),
            effective_from: "2023-10-01T00:00:00Z".to_owned(),
            revoked_at: Some("2025-12-31T23:59:59Z".to_owned()),
        };
        let json = serde_json::to_string(&reg).unwrap();
        let parsed: QisIssuerRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reg);
    }

    #[test]
    fn invoice_kind_serde_round_trips_both_variants() {
        for kind in [QisInvoiceKind::Qualified, QisInvoiceKind::Simplified] {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: QisInvoiceKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn jct_category_serde_round_trips_all_variants() {
        for cat in [
            JctCategory::Standard10,
            JctCategory::Reduced8,
            JctCategory::Zero,
            JctCategory::Exempt,
        ] {
            let json = serde_json::to_string(&cat).unwrap();
            let parsed: JctCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, cat);
        }
    }

    #[test]
    fn revoked_at_is_skipped_on_serialise_when_active() {
        let reg = QisIssuerRegistration {
            registration_number: "T1234567890123".to_owned(),
            legal_name: "Mock JP Issuer 1234".to_owned(),
            effective_from: "2023-10-01T00:00:00Z".to_owned(),
            revoked_at: None,
        };
        let json = serde_json::to_string(&reg).unwrap();
        assert!(!json.contains("revoked_at"));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-jp-qis");
    }
}
