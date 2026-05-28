// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Greece **myDATA** (Άυλο Διασύνδεσμο) e-books / e-invoicing reporting adapter.
//!
//! myDATA is Greece's mandatory continuous reporting of
//! invoices to the IAPR (Independent Authority for Public
//! Revenue, ΑΑΔΕ). Issuers transmit invoice summaries to the
//! IAPR REST endpoints; the authority returns a **MARK**
//! (Μοναδικός Αριθμός Καταχώρησης — Unique Registration
//! Number) plus a **UID** that the issuer must embed in the
//! printed invoice's QR code.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockMyDataProvider`]. The live REST integration lands in
//! a follow-up `crates/report-gr-mydata-http/` crate behind a
//! feature flag so operators who only need the substrate don't
//! pull in the HTTP stack.
//!
//! Reference reading: IAPR myDATA documentation portal at
//! <https://www.aade.gr/mydata>.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the IAPR transport. Operators
/// pick at engine-construction time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataEnvironment {
    /// `mydata-dev.azure-api.net` — the IAPR sandbox tier.
    Sandbox,
    /// `mydatapi.aade.gr` — production.
    Production,
}

/// myDATA invoice classification per IAPR taxonomy.
///
/// Codes mirror the official `invoiceType` field on the
/// myDATA REST API (`1.1` sales of goods, `1.2` ICA goods,
/// `2.1` services, `2.2` ICA services, etc.). The strings
/// stay opaque so the engine can target newer taxonomies
/// without bumping this enum.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataInvoiceCategory {
    /// `1.x` — sales of goods.
    SalesGoods {
        /// Sub-code, e.g. `"1.1"`, `"1.2"`, `"1.3"`.
        code: String,
    },
    /// `2.x` — provision of services.
    Services {
        /// Sub-code, e.g. `"2.1"`, `"2.2"`, `"2.3"`.
        code: String,
    },
    /// `3.x` — title of acquisition (self-billing).
    SelfBilling {
        /// Sub-code, e.g. `"3.1"`, `"3.2"`.
        code: String,
    },
    /// `5.x` — credit note.
    CreditNote {
        /// Sub-code, e.g. `"5.1"` (associated), `"5.2"`
        /// (non-associated).
        code: String,
    },
    /// `8.x` — payroll, deductions, statements.
    Statement {
        /// Sub-code, e.g. `"8.1"`, `"8.2"`.
        code: String,
    },
    /// Escape hatch for codes the engine hasn't yet enumerated.
    Other {
        /// Raw IAPR `invoiceType` code as published on the
        /// myDATA portal.
        code: String,
    },
}

impl MyDataInvoiceCategory {
    /// Borrow the IAPR sub-code as a string slice.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::SalesGoods { code }
            | Self::Services { code }
            | Self::SelfBilling { code }
            | Self::CreditNote { code }
            | Self::Statement { code }
            | Self::Other { code } => code.as_str(),
        }
    }
}

/// **MARK** — Μοναδικός Αριθμός Καταχώρησης / Unique Registration Number.
///
/// The IAPR assigns one per accepted invoice. Engine persists
/// this on the canonical document so every downstream artefact
/// carries it.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MyDataMark(pub String);

impl MyDataMark {
    /// Build a new MARK from any string-shaped value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Borrow the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// **UID** — the unique invoice identifier the IAPR computes
/// (SHA-1 over a canonical projection of the invoice fields).
/// Embedded in the printed-invoice QR code alongside the MARK.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MyDataUid(pub String);

impl MyDataUid {
    /// Build a new UID from any string-shaped value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Borrow the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the operator passes in to
/// [`MyDataProvider::report_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyDataReportRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: MyDataEnvironment,
    /// Issuer's Greek tax registration number (ΑΦΜ, Α.Φ.Μ.).
    pub issuer_afm: String,
    /// Optional buyer ΑΦΜ; some invoice types (e.g. retail)
    /// omit it.
    pub buyer_afm: Option<String>,
    /// myDATA category for this invoice.
    pub category: MyDataInvoiceCategory,
    /// Canonical InvoicesDoc XML payload the IAPR expects.
    pub invoices_doc_xml: Vec<u8>,
}

/// IAPR per-invoice verdict after a `report_invoice` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MyDataStatus {
    /// Successfully recorded; MARK + UID are returned.
    Accepted,
    /// Accepted with warnings (e.g. cross-checking against
    /// the buyer's classifications produced a low-severity
    /// flag). Engine should surface the warning text but the
    /// MARK is valid.
    AcceptedWithWarnings,
    /// IAPR refused the submission; no MARK is assigned. Fix
    /// + resubmit.
    Rejected,
}

/// What [`MyDataProvider::report_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MyDataReportEnvelope {
    /// IAPR verdict.
    pub status: MyDataStatus,
    /// MARK assigned by the IAPR when `status != Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<MyDataMark>,
    /// UID assigned by the IAPR when `status != Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<MyDataUid>,
    /// Raw error or warning text from the IAPR. Engines
    /// surface this verbatim in the audit log.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC-3339 UTC timestamp the IAPR recorded.
    pub reported_at: String,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum MyDataError {
    /// The invoices doc XML did not parse / wasn't InvoicesDoc.
    #[error("invoices doc xml rejected: {0}")]
    BadXml(String),
    /// The issuer ΑΦΜ wasn't 9 ASCII digits.
    #[error("invalid issuer AFM: {0}")]
    BadAfm(String),
    /// Transport-level failure talking to the IAPR.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The Greece myDATA reporting surface. Real IAPR HTTP
/// integrations satisfy this trait; the mock below is what
/// tests + cassette-replay use.
pub trait MyDataProvider: Send + Sync {
    /// Report one invoice to the IAPR.
    ///
    /// # Errors
    ///
    /// Returns [`MyDataError`] when validation fails before
    /// the wire or transport fails on the wire. The
    /// IAPR-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via [`MyDataStatus::Rejected`] inside
    /// [`MyDataReportEnvelope`] so the engine can persist the
    /// rejection alongside its audit trail.
    fn report_invoice(
        &self,
        request: &MyDataReportRequest,
    ) -> Result<MyDataReportEnvelope, MyDataError>;
}

/// Deterministic mock provider. Returns
/// [`MyDataStatus::Accepted`] with a synthesised MARK + UID
/// derived from the request, so cassette-replay tests are
/// byte-identical across runs.
pub struct MockMyDataProvider {
    fixed_reported_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockMyDataProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_reported_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp (the mock
    /// emits this value verbatim in every
    /// [`MyDataReportEnvelope`]).
    #[must_use]
    pub fn with_fixed_reported_at(reported_at: impl Into<String>) -> Self {
        Self {
            fixed_reported_at: reported_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockMyDataProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MyDataProvider for MockMyDataProvider {
    fn report_invoice(
        &self,
        request: &MyDataReportRequest,
    ) -> Result<MyDataReportEnvelope, MyDataError> {
        validate_afm(&request.issuer_afm)?;
        if request.invoices_doc_xml.is_empty() {
            return Err(MyDataError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut guard = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *guard;
            *guard += 1;
            v
        };
        let mark = MyDataMark::new(format!("4000{serial:012}"));
        let uid = MyDataUid::new(format!("MYDATA-MOCK-UID-{serial:08}"));
        Ok(MyDataReportEnvelope {
            status: MyDataStatus::Accepted,
            mark: Some(mark),
            uid: Some(uid),
            message: None,
            reported_at: self.fixed_reported_at.clone(),
        })
    }
}

/// Validate that an ΑΦΜ is exactly 9 ASCII digits. The Greek
/// AFM checksum is a separate concern; this helper only
/// catches obviously-wrong shapes before the wire.
///
/// # Errors
///
/// Returns [`MyDataError::BadAfm`] when the input isn't 9
/// ASCII digits.
pub fn validate_afm(afm: &str) -> Result<(), MyDataError> {
    if afm.len() == 9 && afm.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(MyDataError::BadAfm(format!(
            "AFM must be 9 ASCII digits, got {afm:?}"
        )))
    }
}

/// Build the QR-code payload string the IAPR's e-books portal
/// expects on a printed invoice. Format per IAPR Annex 1:
/// `{base_url}/?mark={MARK}&uid={UID}`.
///
/// # Errors
///
/// Returns [`MyDataError::BadXml`] when the supplied envelope
/// lacks a MARK or UID (i.e. status was `Rejected`).
pub fn qr_payload(base_url: &str, envelope: &MyDataReportEnvelope) -> Result<String, MyDataError> {
    let mark = envelope
        .mark
        .as_ref()
        .ok_or_else(|| MyDataError::BadXml("envelope carries no MARK".to_owned()))?;
    let uid = envelope
        .uid
        .as_ref()
        .ok_or_else(|| MyDataError::BadXml("envelope carries no UID".to_owned()))?;
    Ok(format!(
        "{base_url}/?mark={}&uid={}",
        mark.as_str(),
        uid.as_str()
    ))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_gr_mydata::crate_name(),
///     "invoicekit-report-gr-mydata"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-gr-mydata"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(category: MyDataInvoiceCategory) -> MyDataReportRequest {
        MyDataReportRequest {
            tenant_id: "tenant-gr-test".to_owned(),
            environment: MyDataEnvironment::Sandbox,
            issuer_afm: "123456789".to_owned(),
            buyer_afm: Some("987654321".to_owned()),
            category,
            invoices_doc_xml: b"<InvoicesDoc/>".to_vec(),
        }
    }

    #[test]
    fn report_invoice_returns_accepted_with_mark_and_uid() {
        let p = MockMyDataProvider::default();
        let env = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        assert_eq!(env.status, MyDataStatus::Accepted);
        assert!(env.mark.is_some());
        assert!(env.uid.is_some());
        assert_eq!(env.reported_at, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn report_invoice_serial_increments_per_provider() {
        let p = MockMyDataProvider::default();
        let env1 = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        let env2 = p
            .report_invoice(&sample_request(MyDataInvoiceCategory::SalesGoods {
                code: "1.1".to_owned(),
            }))
            .unwrap();
        assert_ne!(env1.mark.as_ref().unwrap().0, env2.mark.as_ref().unwrap().0);
    }

    #[test]
    fn report_invoice_rejects_empty_xml() {
        let p = MockMyDataProvider::default();
        let mut req = sample_request(MyDataInvoiceCategory::Services {
            code: "2.1".to_owned(),
        });
        req.invoices_doc_xml.clear();
        let err = p.report_invoice(&req).unwrap_err();
        assert!(matches!(err, MyDataError::BadXml(_)));
    }

    #[test]
    fn report_invoice_rejects_bad_afm() {
        let p = MockMyDataProvider::default();
        let mut req = sample_request(MyDataInvoiceCategory::CreditNote {
            code: "5.1".to_owned(),
        });
        req.issuer_afm = "12345".to_owned();
        let err = p.report_invoice(&req).unwrap_err();
        assert!(matches!(err, MyDataError::BadAfm(_)));
    }

    #[test]
    fn category_code_borrows_inner_string() {
        assert_eq!(
            MyDataInvoiceCategory::SalesGoods {
                code: "1.2".to_owned()
            }
            .code(),
            "1.2"
        );
        assert_eq!(
            MyDataInvoiceCategory::Other {
                code: "9.9".to_owned()
            }
            .code(),
            "9.9"
        );
    }

    #[test]
    fn validate_afm_accepts_9_digit_string() {
        assert!(validate_afm("123456789").is_ok());
    }

    #[test]
    fn validate_afm_rejects_wrong_length() {
        assert!(validate_afm("1234567890").is_err());
        assert!(validate_afm("12345678").is_err());
    }

    #[test]
    fn validate_afm_rejects_non_digits() {
        assert!(validate_afm("12345678A").is_err());
        assert!(validate_afm("123 56789").is_err());
    }

    #[test]
    fn qr_payload_renders_mark_and_uid_into_url() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::Accepted,
            mark: Some(MyDataMark::new("400000000000001")),
            uid: Some(MyDataUid::new("MYDATA-UID-1")),
            message: None,
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let qr = qr_payload("https://www.aade.gr/mydata", &envelope).unwrap();
        assert!(qr.contains("mark=400000000000001"));
        assert!(qr.contains("uid=MYDATA-UID-1"));
    }

    #[test]
    fn qr_payload_rejects_envelope_without_mark() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::Rejected,
            mark: None,
            uid: None,
            message: Some("schema validation failed".to_owned()),
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let err = qr_payload("https://www.aade.gr/mydata", &envelope).unwrap_err();
        assert!(matches!(err, MyDataError::BadXml(_)));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let envelope = MyDataReportEnvelope {
            status: MyDataStatus::AcceptedWithWarnings,
            mark: Some(MyDataMark::new("400000000000007")),
            uid: Some(MyDataUid::new("MYDATA-UID-7")),
            message: Some("buyer classification missing".to_owned()),
            reported_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: MyDataReportEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-gr-mydata");
    }
}
