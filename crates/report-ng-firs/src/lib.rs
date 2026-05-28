// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Nigeria **FIRS** e-Invoicing adapter.
//!
//! The Federal Inland Revenue Service (FIRS) operates
//! Nigeria's e-Invoicing clearance regime. Issuers submit
//! typed JSON envelopes to the FIRS portal; FIRS returns an
//! IRN (Invoice Reference Number) and acceptance status.
//!
//! Ships typed surface + [`MockFirsProvider`]; the live FIRS
//! REST integration lands in a follow-up
//! `report-ng-firs-http` crate.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the FIRS transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FirsEnvironment {
    /// FIRS sandbox.
    Sandbox,
    /// Production.
    Production,
}

/// What the operator passes in to
/// [`FirsProvider::submit_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FirsSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: FirsEnvironment,
    /// Issuer FIRS TIN (12 ASCII digits with optional `-`
    /// after the 8th).
    pub issuer_tin: String,
    /// Canonical signed JSON payload.
    pub payload: Vec<u8>,
}

/// FIRS per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FirsStatus {
    /// Accepted by FIRS.
    Accepted,
    /// Rejected by FIRS.
    Rejected,
}

/// What [`FirsProvider::submit_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FirsSubmitEnvelope {
    /// IRN (Invoice Reference Number).
    pub irn: String,
    /// Latest observed status.
    pub status: FirsStatus,
    /// RFC-3339 UTC timestamp FIRS recorded.
    pub recorded_at: String,
    /// Reason text when status is `Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum FirsError {
    /// Payload failed shape validation before the wire.
    #[error("payload rejected: {0}")]
    BadPayload(String),
    /// TIN didn't match the 12-digit shape.
    #[error("invalid TIN: {0}")]
    BadTin(String),
    /// HTTP / TLS / DNS failure talking to FIRS.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The FIRS integration surface.
pub trait FirsProvider: Send + Sync {
    /// Submit one invoice to FIRS.
    ///
    /// # Errors
    ///
    /// Returns [`FirsError`] when local validation fails
    /// before the wire or transport fails on the wire. The
    /// FIRS-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `FirsStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    fn submit_invoice(&self, request: &FirsSubmitRequest) -> Result<FirsSubmitEnvelope, FirsError>;
}

/// Deterministic mock provider.
pub struct MockFirsProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockFirsProvider {
    /// Build a mock with deterministic timestamps + serial
    /// IRNs.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_recorded_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_recorded_at(recorded_at: impl Into<String>) -> Self {
        Self {
            fixed_recorded_at: recorded_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockFirsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FirsProvider for MockFirsProvider {
    fn submit_invoice(&self, request: &FirsSubmitRequest) -> Result<FirsSubmitEnvelope, FirsError> {
        validate_tin(&request.issuer_tin)?;
        if request.payload.is_empty() {
            return Err(FirsError::BadPayload("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(FirsSubmitEnvelope {
            irn: format!("NG-{serial:0>16}"),
            status: FirsStatus::Accepted,
            recorded_at: self.fixed_recorded_at.clone(),
            reason: None,
        })
    }
}

/// Validate a Nigerian TIN — 12 ASCII digits (hyphens
/// stripped).
///
/// # Errors
///
/// Returns [`FirsError::BadTin`] on shape failure.
pub fn validate_tin(tin: &str) -> Result<(), FirsError> {
    let collapsed: String = tin.chars().filter(|c| *c != '-').collect();
    if collapsed.len() == 12 && collapsed.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(FirsError::BadTin(format!(
            "TIN must be 12 ASCII digits (optionally hyphenated), got {tin:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_ng_firs::crate_name(),
///     "invoicekit-report-ng-firs"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-ng-firs"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> FirsSubmitRequest {
        FirsSubmitRequest {
            tenant_id: "tenant-ng-test".to_owned(),
            environment: FirsEnvironment::Sandbox,
            issuer_tin: "12345678-9012".to_owned(),
            payload: br#"{"invoice":"v1"}"#.to_vec(),
        }
    }

    #[test]
    fn submit_invoice_returns_accepted() {
        let p = MockFirsProvider::default();
        let env = p.submit_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, FirsStatus::Accepted);
        assert!(env.irn.starts_with("NG-"));
    }

    #[test]
    fn submit_invoice_serial_increments() {
        let p = MockFirsProvider::default();
        let env1 = p.submit_invoice(&sample_request()).unwrap();
        let env2 = p.submit_invoice(&sample_request()).unwrap();
        assert_ne!(env1.irn, env2.irn);
    }

    #[test]
    fn submit_invoice_rejects_empty_payload() {
        let p = MockFirsProvider::default();
        let mut req = sample_request();
        req.payload.clear();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, FirsError::BadPayload(_)));
    }

    #[test]
    fn submit_invoice_rejects_bad_tin() {
        let p = MockFirsProvider::default();
        let mut req = sample_request();
        req.issuer_tin = "BAD".to_owned();
        let err = p.submit_invoice(&req).unwrap_err();
        assert!(matches!(err, FirsError::BadTin(_)));
    }

    #[test]
    fn validate_tin_round_trip() {
        assert!(validate_tin("123456789012").is_ok());
        assert!(validate_tin("12345678-9012").is_ok());
        assert!(validate_tin("12345").is_err());
        assert!(validate_tin("123456789012A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = FirsSubmitEnvelope {
            irn: "NG-0000000000000007".to_owned(),
            status: FirsStatus::Rejected,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            reason: Some("TIN not registered".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: FirsSubmitEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
