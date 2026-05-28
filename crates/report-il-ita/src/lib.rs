// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Israel **ITA** (Israel Tax Authority) e-Invoicing adapter.
//!
//! Israel's Tax Authority (רשות המסים) operates the
//! e-Invoicing clearance regime via the Israel Invoicing
//! gateway. Above the legal turnover threshold, issuers
//! request an **Allocation Number** for each B2B invoice and
//! print it on the document; buyers cannot claim VAT input
//! credit without it.
//!
//! Ships typed surface + [`MockItaProvider`]; the live ITA
//! REST integration lands in a follow-up
//! `report-il-ita-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the ITA transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItaEnvironment {
    /// ITA sandbox tier.
    Sandbox,
    /// Production.
    Production,
}

/// What the operator passes in to
/// [`ItaProvider::request_allocation`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItaAllocationRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: ItaEnvironment,
    /// Issuer Tax Authority id (9 ASCII digits).
    pub issuer_id: String,
    /// Buyer Tax Authority id (9 ASCII digits).
    pub buyer_id: String,
    /// Gross invoice amount in basis points (i.e. 1 NIS =
    /// 10_000 bp) — typed so currency / decimal handling
    /// happens upstream.
    pub gross_basis_points: u64,
    /// Canonical invoice payload (UBL or ITA-defined JSON).
    pub payload: Vec<u8>,
}

/// ITA per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItaStatus {
    /// Allocation granted.
    Allocated,
    /// Allocation refused.
    Rejected,
}

/// What [`ItaProvider::request_allocation`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItaAllocationEnvelope {
    /// ITA-issued allocation number (9-digit numeric the
    /// issuer prints on the invoice).
    pub allocation_number: String,
    /// Latest observed status.
    pub status: ItaStatus,
    /// RFC-3339 UTC timestamp ITA recorded.
    pub issued_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum ItaError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// Tax authority id didn't match the 9-digit shape.
    #[error("invalid tax id: {0}")]
    BadId(String),
    /// HTTP / TLS / DNS failure talking to ITA.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The ITA integration surface.
pub trait ItaProvider: Send + Sync {
    /// Request an Allocation Number from ITA.
    ///
    /// # Errors
    ///
    /// Returns [`ItaError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// ITA-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `ItaStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn request_allocation(
        &self,
        request: &ItaAllocationRequest,
    ) -> Result<ItaAllocationEnvelope, ItaError>;
}

/// Deterministic mock provider.
pub struct MockItaProvider {
    fixed_issued_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockItaProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_issued_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_issued_at(issued_at: impl Into<String>) -> Self {
        Self {
            fixed_issued_at: issued_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockItaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ItaProvider for MockItaProvider {
    fn request_allocation(
        &self,
        request: &ItaAllocationRequest,
    ) -> Result<ItaAllocationEnvelope, ItaError> {
        validate_id(&request.issuer_id)?;
        validate_id(&request.buyer_id)?;
        if request.payload.is_empty() {
            return Err(ItaError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(ItaAllocationEnvelope {
            allocation_number: format!("{serial:0>9}"),
            status: ItaStatus::Allocated,
            issued_at: self.fixed_issued_at.clone(),
            reason: None,
        })
    }
}

/// Validate an Israeli tax authority id — 9 ASCII digits.
///
/// # Errors
///
/// Returns [`ItaError::BadId`] on shape failure.
pub fn validate_id(value: &str) -> Result<(), ItaError> {
    if value.len() == 9 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(ItaError::BadId(format!(
            "tax id must be 9 ASCII digits, got {value:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_il_ita::crate_name(),
///     "invoicekit-report-il-ita"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-il-ita"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> ItaAllocationRequest {
        ItaAllocationRequest {
            tenant_id: "tenant-il-test".to_owned(),
            environment: ItaEnvironment::Sandbox,
            issuer_id: "123456789".to_owned(),
            buyer_id: "987654321".to_owned(),
            gross_basis_points: 1_000_000,
            payload: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn request_allocation_returns_allocated() {
        let p = MockItaProvider::default();
        let env = p.request_allocation(&sample_request()).unwrap();
        assert_eq!(env.status, ItaStatus::Allocated);
        assert_eq!(env.allocation_number.len(), 9);
    }

    #[test]
    fn request_allocation_serial_increments() {
        let p = MockItaProvider::default();
        let env1 = p.request_allocation(&sample_request()).unwrap();
        let env2 = p.request_allocation(&sample_request()).unwrap();
        assert_ne!(env1.allocation_number, env2.allocation_number);
    }

    #[test]
    fn request_allocation_rejects_empty_payload() {
        let p = MockItaProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.request_allocation(&req).unwrap_err();
        assert!(matches!(err, ItaError::BadPayload(_)));
    }

    #[test]
    fn request_allocation_rejects_bad_issuer_id() {
        let p = MockItaProvider::default();
        let mut req = sample_request();
        req.issuer_id = "BAD".to_owned();
        let err = p.request_allocation(&req).unwrap_err();
        assert!(matches!(err, ItaError::BadId(_)));
    }

    #[test]
    fn request_allocation_rejects_bad_buyer_id() {
        let p = MockItaProvider::default();
        let mut req = sample_request();
        req.buyer_id = "ALSO-BAD".to_owned();
        let err = p.request_allocation(&req).unwrap_err();
        assert!(matches!(err, ItaError::BadId(_)));
    }

    #[test]
    fn validate_id_round_trip() {
        assert!(validate_id("123456789").is_ok());
        assert!(validate_id("12345678").is_err());
        assert!(validate_id("1234567890").is_err());
        assert!(validate_id("12345678A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = ItaAllocationEnvelope {
            allocation_number: "000000007".to_owned(),
            status: ItaStatus::Rejected,
            issued_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("issuer below threshold".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: ItaAllocationEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
