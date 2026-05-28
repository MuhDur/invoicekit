// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Hungary **NAV Online Számla** reporting adapter.
//!
//! Hungary's Nemzeti Adó- és Vámhivatal (NAV, the National
//! Tax and Customs Administration) runs the Online Számla
//! v3.0 reporting endpoints at `api.onlineszamla.nav.gov.hu`.
//! Every Hungarian B2B issuer submits invoices via a typed
//! XML wrapper (`manageInvoiceRequest`); NAV runs a
//! token-exchange + transaction-id flow and returns
//! per-invoice processing status the engine reconciles
//! against.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockNavProvider`]. The live REST integration lands in a
//! follow-up `report-hu-nav-http` crate behind a feature
//! flag.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the NAV transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavEnvironment {
    /// `api-test.onlineszamla.nav.gov.hu` — NAV test tier.
    Test,
    /// `api.onlineszamla.nav.gov.hu` — production.
    Production,
}

/// Which operation the engine is asking the NAV to perform.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavOperation {
    /// Create — first-time submission of a new invoice.
    Create,
    /// Modify — issue a follow-up that corrects a
    /// previously-submitted invoice.
    Modify,
    /// Storno — annul a previously-submitted invoice.
    Storno,
    /// Annul (NAV-side technical annulment for accidentally
    /// duplicated submissions).
    Annul,
}

/// What the operator passes in to
/// [`NavProvider::manage_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavManageRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: NavEnvironment,
    /// Operation to perform.
    pub operation: NavOperation,
    /// Issuer's Hungarian adóazonosító (8 digits + check
    /// digit) or adószám (8 + 1 + 2 digits, hyphenated).
    pub issuer_tax_id: String,
    /// Canonical NAV `manageInvoiceRequest` XML payload.
    pub manage_invoice_xml: Vec<u8>,
}

/// NAV per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavStatus {
    /// Accepted; transaction id assigned and queued for
    /// async processing.
    Received,
    /// Processing in progress (Online Számla processes in
    /// batches).
    InProgress,
    /// Done — invoice is final and visible on the NAV
    /// portal.
    Done,
    /// Aborted — a typed validation rule rejected the
    /// payload.
    Aborted,
}

/// What [`NavProvider::manage_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavManageEnvelope {
    /// NAV-assigned transaction id.
    pub transaction_id: String,
    /// Latest observed status.
    pub status: NavStatus,
    /// RFC-3339 UTC timestamp NAV recorded.
    pub recorded_at: String,
    /// Free-form `validationResult` text when `status ==
    /// Aborted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_result: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum NavError {
    /// `manageInvoiceRequest` XML failed shape validation
    /// before the wire.
    #[error("manage invoice xml rejected: {0}")]
    BadXml(String),
    /// Issuer tax id didn't match the NAV pattern.
    #[error("invalid tax id: {0}")]
    BadTaxId(String),
    /// HTTP / TLS / DNS failure talking to NAV.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The NAV integration surface.
pub trait NavProvider: Send + Sync {
    /// Submit a `manageInvoice` request to NAV. The provider:
    ///
    /// 1. validates `issuer_tax_id` shape,
    /// 2. exchanges the engine's API credentials for a NAV
    ///    one-shot token,
    /// 3. POSTs the `manageInvoiceRequest` XML and returns
    ///    the NAV-issued envelope.
    ///
    /// # Errors
    ///
    /// Returns [`NavError`] when validation fails before the
    /// wire or transport fails on the wire. The
    /// NAV-returned `Aborted` verdict is NOT an `Err` — it's
    /// surfaced via `NavStatus::Aborted` inside the envelope
    /// so the engine persists the rejection alongside its
    /// audit trail.
    fn manage_invoice(&self, request: &NavManageRequest) -> Result<NavManageEnvelope, NavError>;

    /// Poll NAV for the latest status of a previously
    /// submitted transaction.
    ///
    /// # Errors
    ///
    /// Returns [`NavError::Transport`] when the
    /// transaction_id is unknown.
    fn query_transaction(
        &self,
        environment: NavEnvironment,
        transaction_id: &str,
    ) -> Result<NavManageEnvelope, NavError>;
}

/// Deterministic mock provider.
///
/// Emits a `Received` envelope per `manage_invoice` call and
/// `Done` per subsequent `query_transaction` so
/// cassette-replay tests can exercise the full lifecycle
/// without spinning up the NAV test tier.
pub struct MockNavProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockNavProvider {
    /// Build a mock with deterministic timestamps + serial
    /// transaction ids.
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

impl Default for MockNavProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl NavProvider for MockNavProvider {
    fn manage_invoice(&self, request: &NavManageRequest) -> Result<NavManageEnvelope, NavError> {
        validate_tax_id(&request.issuer_tax_id)?;
        if request.manage_invoice_xml.is_empty() {
            return Err(NavError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut g = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        Ok(NavManageEnvelope {
            transaction_id: format!("NAV-{serial:016}"),
            status: NavStatus::Received,
            recorded_at: self.fixed_recorded_at.clone(),
            validation_result: None,
        })
    }

    fn query_transaction(
        &self,
        _environment: NavEnvironment,
        transaction_id: &str,
    ) -> Result<NavManageEnvelope, NavError> {
        if transaction_id.is_empty() {
            return Err(NavError::Transport("empty transaction id".to_owned()));
        }
        Ok(NavManageEnvelope {
            transaction_id: transaction_id.to_owned(),
            status: NavStatus::Done,
            recorded_at: self.fixed_recorded_at.clone(),
            validation_result: None,
        })
    }
}

/// Validate a Hungarian tax id — either an 8-digit
/// adóazonosító (plus optional 1-digit check + 2-digit
/// area), allowing both `12345678` and `12345678-1-23`
/// shapes.
///
/// # Errors
///
/// Returns [`NavError::BadTaxId`] on shape failure.
pub fn validate_tax_id(tax_id: &str) -> Result<(), NavError> {
    let collapsed: String = tax_id.chars().filter(|c| *c != '-').collect();
    let len_ok = matches!(collapsed.len(), 8 | 9 | 11);
    let digits_ok = collapsed.bytes().all(|b| b.is_ascii_digit());
    if len_ok && digits_ok {
        Ok(())
    } else {
        Err(NavError::BadTaxId(format!(
            "tax id must be 8/9/11 digits (optionally hyphenated as 8-1-2), got {tax_id:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_hu_nav::crate_name(),
///     "invoicekit-report-hu-nav"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-hu-nav"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> NavManageRequest {
        NavManageRequest {
            tenant_id: "tenant-hu-test".to_owned(),
            environment: NavEnvironment::Test,
            operation: NavOperation::Create,
            issuer_tax_id: "12345678-1-23".to_owned(),
            manage_invoice_xml: b"<manageInvoiceRequest/>".to_vec(),
        }
    }

    #[test]
    fn manage_invoice_returns_received_with_transaction_id() {
        let p = MockNavProvider::default();
        let env = p.manage_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, NavStatus::Received);
        assert!(env.transaction_id.starts_with("NAV-"));
        assert_eq!(env.recorded_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn manage_invoice_serial_increments_per_provider() {
        let p = MockNavProvider::default();
        let env1 = p.manage_invoice(&sample_request()).unwrap();
        let env2 = p.manage_invoice(&sample_request()).unwrap();
        assert_ne!(env1.transaction_id, env2.transaction_id);
    }

    #[test]
    fn manage_invoice_rejects_empty_payload() {
        let p = MockNavProvider::default();
        let mut req = sample_request();
        req.manage_invoice_xml.clear();
        let err = p.manage_invoice(&req).unwrap_err();
        assert!(matches!(err, NavError::BadXml(_)));
    }

    #[test]
    fn manage_invoice_rejects_bad_tax_id() {
        let p = MockNavProvider::default();
        let mut req = sample_request();
        req.issuer_tax_id = "BAD".to_owned();
        let err = p.manage_invoice(&req).unwrap_err();
        assert!(matches!(err, NavError::BadTaxId(_)));
    }

    #[test]
    fn query_transaction_returns_done() {
        let p = MockNavProvider::default();
        let env = p
            .query_transaction(NavEnvironment::Test, "NAV-0000000000000001")
            .unwrap();
        assert_eq!(env.status, NavStatus::Done);
    }

    #[test]
    fn query_transaction_rejects_empty_id() {
        let p = MockNavProvider::default();
        let err = p.query_transaction(NavEnvironment::Test, "").unwrap_err();
        assert!(matches!(err, NavError::Transport(_)));
    }

    #[test]
    fn validate_tax_id_accepts_8_9_or_11_digit_shapes() {
        assert!(validate_tax_id("12345678").is_ok());
        assert!(validate_tax_id("123456789").is_ok());
        assert!(validate_tax_id("12345678123").is_ok());
        assert!(validate_tax_id("12345678-1-23").is_ok());
    }

    #[test]
    fn validate_tax_id_rejects_wrong_lengths() {
        assert!(validate_tax_id("1234567").is_err());
        assert!(validate_tax_id("1234567890").is_err());
    }

    #[test]
    fn validate_tax_id_rejects_non_digits() {
        assert!(validate_tax_id("1234567A").is_err());
        assert!(validate_tax_id("12345678-1-2A").is_err());
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = NavManageEnvelope {
            transaction_id: "NAV-0000000000000007".to_owned(),
            status: NavStatus::Aborted,
            recorded_at: "2026-01-01T00:00:00Z".to_owned(),
            validation_result: Some("INVOICE_NUMBER required".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: NavManageEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn operation_serde_round_trips_all_four_variants() {
        for op in [
            NavOperation::Create,
            NavOperation::Modify,
            NavOperation::Storno,
            NavOperation::Annul,
        ] {
            let json = serde_json::to_string(&op).unwrap();
            let parsed: NavOperation = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, op);
        }
    }
}
