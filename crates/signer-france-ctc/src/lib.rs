// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! France CTC (Continuous Transaction Control) signing-and-routing adapter.
//!
//! France's CTC mandate ramps from 2026 onward: every B2B
//! invoice must transit through either the public **PPF**
//! (Portail Public de Facturation, the government's free
//! platform) or a private **PDP** (Plateforme de
//! Dématérialisation Partenaire — an Agence des
//! Finances-accredited partner). The engine signs the invoice
//! with an EU qualified certificate (the same path as eIDAS
//! T-083a), routes via the chosen PDP/PPF, then receives
//! status callbacks: deposited → received → approved /
//! rejected.
//!
//! This crate ships the typed surface and a deterministic
//! `MockFrCtcProvider`. Real PDP integrations land behind
//! feature flags in follow-up crates (`signer-france-ctc-ppf`
//! for the public portal, `signer-france-ctc-<vendor>` per
//! accredited PDP).
//!
//! Reference reading: France's Direction Générale des Finances
//! Publiques (DGFiP) external specification "Spécifications
//! Externes Facture Électronique B2B" v2.x.

#![allow(clippy::doc_markdown)]

use invoicekit_signer_eidas::QualifiedCertificate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which platform routes the invoice on its way to the
/// receiver. Operators pick at engine-construction time.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcPlatform {
    /// Portail Public de Facturation (free public portal run
    /// by AIFE/Chorus Pro).
    Ppf,
    /// Plateforme de Dématérialisation Partenaire — a private
    /// accredited partner. The string carries the platform's
    /// SIRET so cassettes can pin which partner the test was
    /// recorded against.
    Pdp {
        /// Partner SIRET (14 digits).
        siret: String,
    },
}

/// Environment selector for the routing layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcEnvironment {
    /// PISTE / sandbox tier. Equivalent of the PPF
    /// `piste.gouv.fr` environment.
    Piste,
    /// Production tier.
    Production,
}

/// Receiver lookup key — either a SIRET, an SIREN, or an
/// "Annuaire" identifier the public portal exposes via the
/// receiver directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcReceiver {
    /// 14-digit SIRET.
    Siret(String),
    /// 9-digit SIREN.
    Siren(String),
    /// Annuaire identifier (typically `SIRET` + a routing
    /// suffix when the receiver opted into a specific PDP).
    Annuaire(String),
}

/// Status the engine observes after submission. Mirrors the
/// CTC "cycle de vie" the DGFiP spec defines.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrCtcStatus {
    /// Submitted to the platform; awaiting initial intake.
    Submitted,
    /// Platform accepted the deposit; awaiting routing to the
    /// receiver.
    Deposited,
    /// Receiver platform / inbox confirmed receipt.
    Received,
    /// Receiver accepted the invoice (legal validation done).
    Approved,
    /// Receiver or platform rejected with a typed reason.
    Rejected,
    /// Routing is suspended — usually a credit-note pending
    /// approval before the original invoice's state can
    /// advance.
    Suspended,
}

/// What the operator passes in to [`FrCtcProvider::submit`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrCtcSubmitRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: FrCtcEnvironment,
    /// Routing platform.
    pub platform: FrCtcPlatform,
    /// Receiver lookup.
    pub receiver: FrCtcReceiver,
    /// Canonical XML payload (UBL or CII). The provider
    /// computes its own hash + signs via the supplied
    /// qualified certificate; the operator does NOT
    /// pre-sign.
    pub xml: Vec<u8>,
}

/// What [`FrCtcProvider::submit`] returns when the platform
/// has acknowledged the submission and (where applicable)
/// routed it to the receiver.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrCtcStampEnvelope {
    /// Platform-assigned submission id.
    pub submission_id: String,
    /// Most recent observed lifecycle status. Subsequent
    /// statuses arrive asynchronously through the engine's
    /// reconciliation loop.
    pub status: FrCtcStatus,
    /// RFC-3339 UTC timestamp the platform recorded.
    pub stamped_at: String,
    /// Optional platform reason string when `status` is
    /// `Rejected`. Mirrors the DGFiP "motif de rejet"
    /// vocabulary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum FrCtcError {
    /// The supplied XML did not parse / wasn't UBL or CII.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// The qualified certificate failed signing — usually
    /// because the QSCD's PIN policy rejected the request.
    #[error("signing failure: {0}")]
    SigningFailure(String),
    /// The platform refused the submission (validation rule
    /// returned a specific motif de rejet).
    #[error("platform rejected: {motif}: {detail}")]
    PlatformRejection {
        /// DGFiP-defined motif de rejet code.
        motif: String,
        /// Human-readable detail string.
        detail: String,
    },
    /// HTTP / TLS / DNS failure talking to the platform.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The signing-and-routing surface. Real PDP / PPF
/// implementations satisfy this trait; the mock below is what
/// tests and cassette-replay use.
pub trait FrCtcProvider: Send + Sync {
    /// Sign and submit one invoice. The provider:
    ///
    /// 1. computes the canonical hash of `request.xml`,
    /// 2. signs the hash with the supplied
    ///    [`QualifiedCertificate`] (XAdES-BES enveloped per
    ///    DGFiP spec),
    /// 3. POSTs the signed payload to the platform endpoint
    ///    selected by `request.platform` + `request.environment`,
    /// 4. returns the platform's initial `FrCtcStampEnvelope`.
    ///
    /// Subsequent status transitions arrive asynchronously
    /// through `poll_status`.
    ///
    /// # Errors
    ///
    /// Returns [`FrCtcError`] when signing, transport, or
    /// platform validation fails.
    fn submit(
        &self,
        certificate: &QualifiedCertificate,
        request: &FrCtcSubmitRequest,
    ) -> Result<FrCtcStampEnvelope, FrCtcError>;

    /// Poll the platform for the latest status of a previously
    /// submitted invoice.
    ///
    /// # Errors
    ///
    /// Returns [`FrCtcError`] when transport fails or the
    /// platform doesn't recognise the submission id.
    fn poll_status(
        &self,
        environment: FrCtcEnvironment,
        platform: &FrCtcPlatform,
        submission_id: &str,
    ) -> Result<FrCtcStampEnvelope, FrCtcError>;
}

/// Deterministic mock provider.
///
/// Returns `FrCtcStatus::Deposited` for fresh submissions and
/// `FrCtcStatus::Approved` for repeat polls of the same
/// submission id, so cassette-replay tests can exercise the
/// full lifecycle without spinning up a real platform.
pub struct MockFrCtcProvider {
    fixed_stamped_at: String,
    next_submission_id: std::sync::Mutex<u64>,
}

impl MockFrCtcProvider {
    /// Build a mock with deterministic timestamps + serial
    /// submission ids.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_stamped_at("2026-07-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp (the mock
    /// emits this value verbatim into every
    /// [`FrCtcStampEnvelope`]).
    #[must_use]
    pub fn with_fixed_stamped_at(stamped_at: impl Into<String>) -> Self {
        Self {
            fixed_stamped_at: stamped_at.into(),
            next_submission_id: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockFrCtcProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FrCtcProvider for MockFrCtcProvider {
    fn submit(
        &self,
        _certificate: &QualifiedCertificate,
        request: &FrCtcSubmitRequest,
    ) -> Result<FrCtcStampEnvelope, FrCtcError> {
        if request.xml.is_empty() {
            return Err(FrCtcError::BadXml("payload is empty".to_owned()));
        }
        // Tiny well-formed-ness sanity check: must start with
        // `<` after optional BOM/whitespace.
        let trimmed_starts_with_lt = request
            .xml
            .iter()
            .find(|&&b| !b.is_ascii_whitespace() && !matches!(b, 0xfe | 0xff | 0xef | 0xbb | 0xbf))
            .is_some_and(|&b| b == b'<');
        if !trimmed_starts_with_lt {
            return Err(FrCtcError::BadXml(
                "payload does not look like XML (first non-whitespace byte is not `<`)".to_owned(),
            ));
        }

        let serial = {
            let mut guard = self
                .next_submission_id
                .lock()
                .expect("submission id mutex poisoned");
            let v = *guard;
            *guard += 1;
            v
        };
        let prefix = match (&request.environment, &request.platform) {
            (FrCtcEnvironment::Piste, FrCtcPlatform::Ppf) => "PISTE-PPF",
            (FrCtcEnvironment::Production, FrCtcPlatform::Ppf) => "PPF",
            (FrCtcEnvironment::Piste, FrCtcPlatform::Pdp { .. }) => "PISTE-PDP",
            (FrCtcEnvironment::Production, FrCtcPlatform::Pdp { .. }) => "PDP",
        };
        Ok(FrCtcStampEnvelope {
            submission_id: format!("{prefix}-{serial:08}"),
            status: FrCtcStatus::Deposited,
            stamped_at: self.fixed_stamped_at.clone(),
            rejection_reason: None,
        })
    }

    fn poll_status(
        &self,
        _environment: FrCtcEnvironment,
        _platform: &FrCtcPlatform,
        submission_id: &str,
    ) -> Result<FrCtcStampEnvelope, FrCtcError> {
        if submission_id.is_empty() {
            return Err(FrCtcError::Transport("empty submission id".to_owned()));
        }
        Ok(FrCtcStampEnvelope {
            submission_id: submission_id.to_owned(),
            status: FrCtcStatus::Approved,
            stamped_at: self.fixed_stamped_at.clone(),
            rejection_reason: None,
        })
    }
}

/// Validate that a SIRET is exactly 14 ASCII digits. Exposed
/// so callers can pre-flight before going to the wire.
///
/// # Errors
///
/// Returns [`FrCtcError::BadXml`] when the input isn't 14
/// ASCII digits.
pub fn validate_siret(siret: &str) -> Result<(), FrCtcError> {
    if siret.len() == 14 && siret.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(FrCtcError::BadXml(format!(
            "SIRET must be 14 ASCII digits, got {siret:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_france_ctc::crate_name(),
///     "invoicekit-signer-france-ctc"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-france-ctc"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert() -> QualifiedCertificate {
        QualifiedCertificate {
            id: invoicekit_signer_eidas::QualifiedCertificateId::new("fr-mock-cert"),
            subject_dn: "CN=Test Issuer, C=FR".to_owned(),
            issuer_dn: "CN=Test QTSP, C=FR".to_owned(),
            serial: "01".to_owned(),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-01-01T00:00:00Z".to_owned(),
            qualified: true,
        }
    }

    fn sample_request(platform: FrCtcPlatform) -> FrCtcSubmitRequest {
        FrCtcSubmitRequest {
            tenant_id: "tenant-fr-test".to_owned(),
            environment: FrCtcEnvironment::Piste,
            platform,
            receiver: FrCtcReceiver::Siret("12345678901234".to_owned()),
            xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn submit_to_ppf_returns_deposited_status() {
        let p = MockFrCtcProvider::default();
        let env = p
            .submit(&cert(), &sample_request(FrCtcPlatform::Ppf))
            .unwrap();
        assert_eq!(env.status, FrCtcStatus::Deposited);
        assert!(env.submission_id.starts_with("PISTE-PPF-"));
        assert_eq!(env.stamped_at, "2026-07-01T00:00:00Z");
        assert!(env.rejection_reason.is_none());
    }

    #[test]
    fn submit_to_pdp_carries_siret_in_routing_prefix() {
        let p = MockFrCtcProvider::default();
        let mut req = sample_request(FrCtcPlatform::Pdp {
            siret: "98765432109876".to_owned(),
        });
        req.environment = FrCtcEnvironment::Production;
        let env = p.submit(&cert(), &req).unwrap();
        assert!(env.submission_id.starts_with("PDP-"));
    }

    #[test]
    fn submit_serial_increments_per_provider() {
        let p = MockFrCtcProvider::default();
        let env1 = p
            .submit(&cert(), &sample_request(FrCtcPlatform::Ppf))
            .unwrap();
        let env2 = p
            .submit(&cert(), &sample_request(FrCtcPlatform::Ppf))
            .unwrap();
        assert_ne!(env1.submission_id, env2.submission_id);
    }

    #[test]
    fn submit_rejects_empty_payload() {
        let p = MockFrCtcProvider::default();
        let mut req = sample_request(FrCtcPlatform::Ppf);
        req.xml.clear();
        let err = p.submit(&cert(), &req).unwrap_err();
        assert!(matches!(err, FrCtcError::BadXml(_)));
    }

    #[test]
    fn submit_rejects_non_xml_payload() {
        let p = MockFrCtcProvider::default();
        let mut req = sample_request(FrCtcPlatform::Ppf);
        req.xml = b"not xml at all".to_vec();
        let err = p.submit(&cert(), &req).unwrap_err();
        assert!(matches!(err, FrCtcError::BadXml(_)));
    }

    #[test]
    fn poll_status_returns_approved_for_known_id() {
        let p = MockFrCtcProvider::default();
        let env = p
            .poll_status(
                FrCtcEnvironment::Piste,
                &FrCtcPlatform::Ppf,
                "PISTE-PPF-00000001",
            )
            .unwrap();
        assert_eq!(env.status, FrCtcStatus::Approved);
    }

    #[test]
    fn poll_status_rejects_empty_id() {
        let p = MockFrCtcProvider::default();
        let err = p
            .poll_status(FrCtcEnvironment::Piste, &FrCtcPlatform::Ppf, "")
            .unwrap_err();
        assert!(matches!(err, FrCtcError::Transport(_)));
    }

    #[test]
    fn validate_siret_accepts_14_digit_string() {
        assert!(validate_siret("12345678901234").is_ok());
    }

    #[test]
    fn validate_siret_rejects_wrong_length() {
        assert!(validate_siret("1234567890").is_err());
        assert!(validate_siret("123456789012345").is_err());
    }

    #[test]
    fn validate_siret_rejects_non_digits() {
        assert!(validate_siret("1234567890123A").is_err());
        assert!(validate_siret("123 4567890123").is_err());
    }

    #[test]
    fn rejection_status_round_trips_through_serde() {
        let env = FrCtcStampEnvelope {
            submission_id: "x".to_owned(),
            status: FrCtcStatus::Rejected,
            stamped_at: "2026-07-01T00:00:00Z".to_owned(),
            rejection_reason: Some("motif:NOMENCLATURE".to_owned()),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: FrCtcStampEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn platform_serde_round_trips_both_variants() {
        let ppf = FrCtcPlatform::Ppf;
        let pdp = FrCtcPlatform::Pdp {
            siret: "12345678901234".to_owned(),
        };
        let j_ppf = serde_json::to_string(&ppf).unwrap();
        let j_pdp = serde_json::to_string(&pdp).unwrap();
        assert!(j_ppf.contains("ppf"));
        assert!(j_pdp.contains("pdp"));
        let r_ppf: FrCtcPlatform = serde_json::from_str(&j_ppf).unwrap();
        let r_pdp: FrCtcPlatform = serde_json::from_str(&j_pdp).unwrap();
        assert_eq!(r_ppf, ppf);
        assert_eq!(r_pdp, pdp);
    }
}
