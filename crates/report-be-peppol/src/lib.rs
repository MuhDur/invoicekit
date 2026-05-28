// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Belgium **Peppol-overlay** reporting adapter.
//!
//! Belgium's B2G + (from 2026) B2B e-invoicing mandate uses
//! Peppol BIS Billing 3 as the wire format. The Belgian
//! government's federal portal **Mercurius** receives B2G
//! invoices; the **Hermes** Peppol access point routes B2B
//! invoices through Peppol to the receiver's chosen access
//! point. The engine signs through `crates/signer-eidas` (or
//! a Belgian QTSP like Cybertrust / Quovadis), delivers
//! through `crates/transmit-peppol` (partner-AP or phase4),
//! and persists a typed Belgian envelope on top of the
//! standard Peppol receipt.
//!
//! This crate is a thin Belgium-specific overlay on the
//! shared Peppol BIS Billing 3 substrate. It captures:
//!
//! - the federal e-invoicing **mandate** state (B2G vs B2B,
//!   pre- vs post-2026 ramp),
//! - Belgian-specific **BTW / TVA categorisation** that the
//!   Mercurius / Hermes intake validates beyond the plain
//!   Peppol BIS rules,
//! - the typed **BePeppolReceiver** lookup keys (KBO/BCE
//!   enterprise number, VAT id, or Peppol participant id),
//! - per-invoice routing through Mercurius (B2G) vs Hermes
//!   (B2B Peppol delivery).
//!
//! Implements [`BePeppolProvider`]; a deterministic
//! [`MockBePeppolProvider`] ships for tests. The live impl
//! lands behind a feature flag in
//! `crates/report-be-peppol-http`.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the transport layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BePeppolEnvironment {
    /// `mercurius-test.fedict.be` / Peppol test network.
    Sandbox,
    /// `mercurius.fedict.be` / Peppol production network.
    Production,
}

/// Which Belgian mandate covers a given invoice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BePeppolMandate {
    /// B2G — invoices to Belgian public sector entities, in
    /// effect since 2019. Mercurius is the receiving portal.
    B2g,
    /// B2B — phased mandate ramping from 2026 onward. Hermes
    /// or any Peppol AP receives.
    B2b,
    /// B2C reporting tier (RD/AR forthcoming) — separate from
    /// classic Peppol Billing; included so the engine can
    /// pre-route without inventing a new enum later.
    B2cReporting,
}

/// Belgian receiver lookup key.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BePeppolReceiver {
    /// 10-digit Belgian enterprise number (Kruispuntbank van
    /// Ondernemingen / Banque-Carrefour des Entreprises).
    Kbo(String),
    /// VAT id including the `BE` prefix (so `BE0123456789`).
    VatId(String),
    /// Peppol participant id (`0208:0123456789` for KBO,
    /// `9925:BE0123456789` for VAT, etc.).
    PeppolParticipant(String),
}

/// Belgian BTW / TVA category. Overlays the plain Peppol BIS
/// UNCL5305 codes with the Belgian-specific verdicts the
/// Mercurius validator enforces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BePeppolVatCategory {
    /// Standard rate (21%).
    Standard,
    /// Reduced rate (12%).
    Reduced12,
    /// Reduced rate (6%).
    Reduced6,
    /// Zero-rated supplies (export, intra-EU).
    Zero,
    /// Exempt (medical, educational, etc.).
    Exempt,
    /// Reverse charge (cocontractant).
    ReverseCharge,
}

/// What the operator passes in to
/// [`BePeppolProvider::deliver`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BePeppolDeliverRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: BePeppolEnvironment,
    /// Which Belgian mandate this invoice falls under.
    pub mandate: BePeppolMandate,
    /// Receiver lookup.
    pub receiver: BePeppolReceiver,
    /// Per-line BTW categories (one per Peppol invoice line).
    /// Surfacing the typed categorisation up-front lets the
    /// engine pre-validate against Mercurius's stricter rules
    /// before the wire.
    pub vat_categories: Vec<BePeppolVatCategory>,
    /// Canonical Peppol BIS Billing 3 UBL payload.
    pub peppol_ubl_xml: Vec<u8>,
}

/// Lifecycle status after a `deliver` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BePeppolStatus {
    /// Submitted to Mercurius/Hermes; awaiting delivery.
    Submitted,
    /// Mercurius/Hermes confirmed receipt; awaiting receiver
    /// AP intake.
    Delivered,
    /// Receiver acknowledged (Peppol MLR Accepted).
    Accepted,
    /// Receiver rejected (Peppol MLR Rejected).
    Rejected,
    /// Mercurius/Hermes returned a typed business-rule
    /// validation failure before the invoice reached the
    /// receiver.
    ValidationFailed,
}

/// What [`BePeppolProvider::deliver`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BePeppolDeliverEnvelope {
    /// Mercurius/Hermes-assigned submission id.
    pub submission_id: String,
    /// Latest observed status.
    pub status: BePeppolStatus,
    /// Peppol Message Level Response (MLR) reason text when
    /// `status` is `Rejected` or `ValidationFailed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mlr_reason: Option<String>,
    /// RFC-3339 UTC timestamp Mercurius/Hermes recorded.
    pub delivered_at: String,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum BePeppolError {
    /// The supplied UBL XML did not parse / wasn't Peppol BIS
    /// Billing 3.
    #[error("peppol ubl xml rejected: {0}")]
    BadXml(String),
    /// The KBO / VAT id / Peppol participant id had a wrong
    /// shape.
    #[error("invalid receiver: {0}")]
    BadReceiver(String),
    /// Mercurius's stricter business rules flagged a typed
    /// category mismatch before transport.
    #[error("vat category mismatch: {0}")]
    BadVatCategorisation(String),
    /// HTTP / TLS / DNS failure talking to Mercurius / Hermes.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The Belgium Peppol-overlay delivery surface. Real
/// Mercurius/Hermes integrations satisfy this trait; the mock
/// below is what tests + cassette-replay use.
pub trait BePeppolProvider: Send + Sync {
    /// Deliver one invoice through Mercurius (B2G) or Hermes
    /// (B2B) and return the envelope. The provider:
    ///
    /// 1. validates `receiver` shape per its variant,
    /// 2. pre-checks VAT categorisation against the Belgian
    ///    rule set,
    /// 3. routes to the right transport endpoint per
    ///    `mandate` + `environment`,
    /// 4. returns the initial envelope; subsequent status
    ///    transitions arrive asynchronously through
    ///    [`poll_status`].
    ///
    /// [`poll_status`]: BePeppolProvider::poll_status
    ///
    /// # Errors
    ///
    /// Returns [`BePeppolError`] when the receiver shape is
    /// wrong, VAT pre-check fails, transport fails, or the
    /// payload isn't well-formed Peppol UBL.
    fn deliver(
        &self,
        request: &BePeppolDeliverRequest,
    ) -> Result<BePeppolDeliverEnvelope, BePeppolError>;

    /// Poll Mercurius/Hermes for the latest status of a
    /// previously delivered invoice.
    ///
    /// # Errors
    ///
    /// Returns [`BePeppolError`] when transport fails or the
    /// submission id is unknown.
    fn poll_status(
        &self,
        environment: BePeppolEnvironment,
        submission_id: &str,
    ) -> Result<BePeppolDeliverEnvelope, BePeppolError>;
}

/// Deterministic mock provider.
///
/// Emits a `Delivered` envelope per `deliver` call and
/// `Accepted` per subsequent `poll_status` so cassette-replay
/// tests can exercise the full lifecycle without spinning up
/// Mercurius / Hermes.
pub struct MockBePeppolProvider {
    fixed_delivered_at: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockBePeppolProvider {
    /// Build a mock with deterministic timestamps + serial
    /// submission ids.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_delivered_at("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_delivered_at(delivered_at: impl Into<String>) -> Self {
        Self {
            fixed_delivered_at: delivered_at.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockBePeppolProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl BePeppolProvider for MockBePeppolProvider {
    fn deliver(
        &self,
        request: &BePeppolDeliverRequest,
    ) -> Result<BePeppolDeliverEnvelope, BePeppolError> {
        validate_receiver(&request.receiver)?;
        validate_vat_categories(&request.vat_categories)?;
        if request.peppol_ubl_xml.is_empty() {
            return Err(BePeppolError::BadXml("payload is empty".to_owned()));
        }
        let serial = {
            let mut guard = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *guard;
            *guard += 1;
            v
        };
        let prefix = match (&request.environment, &request.mandate) {
            (BePeppolEnvironment::Sandbox, BePeppolMandate::B2g) => "MERC-SBX",
            (BePeppolEnvironment::Production, BePeppolMandate::B2g) => "MERC-PROD",
            (BePeppolEnvironment::Sandbox, BePeppolMandate::B2b) => "HERMES-SBX",
            (BePeppolEnvironment::Production, BePeppolMandate::B2b) => "HERMES-PROD",
            (BePeppolEnvironment::Sandbox, BePeppolMandate::B2cReporting) => "B2CREP-SBX",
            (BePeppolEnvironment::Production, BePeppolMandate::B2cReporting) => "B2CREP-PROD",
        };
        Ok(BePeppolDeliverEnvelope {
            submission_id: format!("{prefix}-{serial:08}"),
            status: BePeppolStatus::Delivered,
            mlr_reason: None,
            delivered_at: self.fixed_delivered_at.clone(),
        })
    }

    fn poll_status(
        &self,
        _environment: BePeppolEnvironment,
        submission_id: &str,
    ) -> Result<BePeppolDeliverEnvelope, BePeppolError> {
        if submission_id.is_empty() {
            return Err(BePeppolError::Transport("empty submission id".to_owned()));
        }
        Ok(BePeppolDeliverEnvelope {
            submission_id: submission_id.to_owned(),
            status: BePeppolStatus::Accepted,
            mlr_reason: None,
            delivered_at: self.fixed_delivered_at.clone(),
        })
    }
}

/// Validate a [`BePeppolReceiver`] shape.
///
/// - KBO: exactly 10 ASCII digits (optionally with a leading
///   `0` zero-pad).
/// - VatId: `BE` prefix + 10 ASCII digits.
/// - PeppolParticipant: non-empty, contains a `:` per the
///   Peppol scheme:value convention.
///
/// # Errors
///
/// Returns [`BePeppolError::BadReceiver`] on shape failure.
pub fn validate_receiver(receiver: &BePeppolReceiver) -> Result<(), BePeppolError> {
    match receiver {
        BePeppolReceiver::Kbo(s) => {
            if s.len() == 10 && s.bytes().all(|b| b.is_ascii_digit()) {
                Ok(())
            } else {
                Err(BePeppolError::BadReceiver(format!(
                    "KBO must be 10 ASCII digits, got {s:?}"
                )))
            }
        }
        BePeppolReceiver::VatId(s) => {
            if s.len() == 12 && s.starts_with("BE") && s.bytes().skip(2).all(|b| b.is_ascii_digit())
            {
                Ok(())
            } else {
                Err(BePeppolError::BadReceiver(format!(
                    "VAT id must be `BE` + 10 ASCII digits, got {s:?}"
                )))
            }
        }
        BePeppolReceiver::PeppolParticipant(s) => {
            if !s.is_empty() && s.contains(':') {
                Ok(())
            } else {
                Err(BePeppolError::BadReceiver(format!(
                    "Peppol participant id must contain `:`, got {s:?}"
                )))
            }
        }
    }
}

/// Validate VAT categorisation. The Belgian Mercurius
/// validator rejects empty category vectors (every line must
/// declare a category) and rejects mixing `Exempt` with
/// `Standard` on the same invoice.
///
/// # Errors
///
/// Returns [`BePeppolError::BadVatCategorisation`] on failure.
pub fn validate_vat_categories(categories: &[BePeppolVatCategory]) -> Result<(), BePeppolError> {
    if categories.is_empty() {
        return Err(BePeppolError::BadVatCategorisation(
            "at least one VAT category required".to_owned(),
        ));
    }
    let has_exempt = categories.contains(&BePeppolVatCategory::Exempt);
    let has_standard = categories.contains(&BePeppolVatCategory::Standard);
    if has_exempt && has_standard {
        return Err(BePeppolError::BadVatCategorisation(
            "Exempt and Standard cannot mix on the same invoice".to_owned(),
        ));
    }
    Ok(())
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_be_peppol::crate_name(),
///     "invoicekit-report-be-peppol"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-be-peppol"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> BePeppolDeliverRequest {
        BePeppolDeliverRequest {
            tenant_id: "tenant-be-test".to_owned(),
            environment: BePeppolEnvironment::Sandbox,
            mandate: BePeppolMandate::B2g,
            receiver: BePeppolReceiver::Kbo("0123456749".to_owned()),
            vat_categories: vec![BePeppolVatCategory::Standard],
            peppol_ubl_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn deliver_returns_delivered_envelope() {
        let p = MockBePeppolProvider::default();
        let env = p.deliver(&sample_request()).unwrap();
        assert_eq!(env.status, BePeppolStatus::Delivered);
        assert!(env.submission_id.starts_with("MERC-SBX-"));
        assert!(env.mlr_reason.is_none());
    }

    #[test]
    fn deliver_routes_b2b_via_hermes_in_production() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.environment = BePeppolEnvironment::Production;
        req.mandate = BePeppolMandate::B2b;
        let env = p.deliver(&req).unwrap();
        assert!(env.submission_id.starts_with("HERMES-PROD-"));
    }

    #[test]
    fn deliver_serial_increments_per_provider() {
        let p = MockBePeppolProvider::default();
        let env1 = p.deliver(&sample_request()).unwrap();
        let env2 = p.deliver(&sample_request()).unwrap();
        assert_ne!(env1.submission_id, env2.submission_id);
    }

    #[test]
    fn deliver_rejects_empty_payload() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.peppol_ubl_xml.clear();
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadXml(_)));
    }

    #[test]
    fn deliver_rejects_bad_kbo_shape() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.receiver = BePeppolReceiver::Kbo("123".to_owned());
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadReceiver(_)));
    }

    #[test]
    fn deliver_rejects_bad_vat_id_shape() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.receiver = BePeppolReceiver::VatId("FR0123456789".to_owned());
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadReceiver(_)));
    }

    #[test]
    fn deliver_accepts_well_formed_peppol_participant() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.receiver = BePeppolReceiver::PeppolParticipant("0208:0123456749".to_owned());
        let env = p.deliver(&req).unwrap();
        assert_eq!(env.status, BePeppolStatus::Delivered);
    }

    #[test]
    fn deliver_rejects_peppol_participant_without_colon() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.receiver = BePeppolReceiver::PeppolParticipant("notwell-formed".to_owned());
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadReceiver(_)));
    }

    #[test]
    fn deliver_rejects_empty_vat_categories() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.vat_categories.clear();
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadVatCategorisation(_)));
    }

    #[test]
    fn deliver_rejects_exempt_mixed_with_standard() {
        let p = MockBePeppolProvider::default();
        let mut req = sample_request();
        req.vat_categories = vec![BePeppolVatCategory::Standard, BePeppolVatCategory::Exempt];
        let err = p.deliver(&req).unwrap_err();
        assert!(matches!(err, BePeppolError::BadVatCategorisation(_)));
    }

    #[test]
    fn poll_status_returns_accepted_for_known_id() {
        let p = MockBePeppolProvider::default();
        let env = p
            .poll_status(BePeppolEnvironment::Sandbox, "MERC-SBX-00000001")
            .unwrap();
        assert_eq!(env.status, BePeppolStatus::Accepted);
    }

    #[test]
    fn poll_status_rejects_empty_id() {
        let p = MockBePeppolProvider::default();
        let err = p.poll_status(BePeppolEnvironment::Sandbox, "").unwrap_err();
        assert!(matches!(err, BePeppolError::Transport(_)));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = BePeppolDeliverEnvelope {
            submission_id: "MERC-PROD-00000007".to_owned(),
            status: BePeppolStatus::ValidationFailed,
            mlr_reason: Some("BR-CO-15 violation".to_owned()),
            delivered_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: BePeppolDeliverEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}
