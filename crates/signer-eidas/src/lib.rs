// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// eIDAS / AdES terminology is full of CamelCase acronyms
// (CAdES, XAdES, PAdES, QTSP, AdES, OCSP, CRL) that aren't
// rust items; the doc-markdown check fires false positives.
#![allow(clippy::doc_markdown)]
// AdES level helpers read more naturally with `if !requires_x`
// guards than the inverted form clippy prefers.
#![allow(clippy::if_not_else)]
// Doc list-item continuations use 2-space indent throughout
// this crate; the rust-stable list-continuation check wants 4.
#![allow(clippy::doc_lazy_continuation)]

//! `invoicekit-signer-eidas` — eIDAS qualified-signature adapter.
//!
//! Layers the eIDAS [QTSP] (Qualified Trust Service Provider)
//! contract on top of [`invoicekit_signer`]:
//!
//! * [`EidasQtspProvider`] — provider trait that bundles the
//!   underlying [`Signer`] with the
//!   eIDAS-specific operations every Qualified Trust Service
//!   Provider exposes (request a qualified certificate,
//!   produce an `AdES` envelope, verify a signature).
//! * [`EidasSignatureProfile`] — typed enum covering the AdES
//!   level matrix: CAdES / XAdES / PAdES × B / T / LT / LTA.
//!   The Year-1 InvoiceKit anchors are XAdES-T (XML invoices)
//!   and PAdES-B-LT (Factur-X / hybrid PDFs).
//! * [`EidasSignature`] — typed envelope holding the AdES bytes
//!   + the originating profile + the optional RFC 3161
//!   timestamp token reference.
//! * [`EidasVerifyReport`] — verification verdict with per-step
//!   outcomes (certificate chain, signature value, timestamp,
//!   revocation).
//! * [`MockEidasProvider`] — in-memory provider used by tests
//!   and the cassette-replay sandbox.
//!
//! [QTSP]: https://digital-strategy.ec.europa.eu/en/policies/trust-services
//!
//! # Provider scope
//!
//! The strict T-083a gate ("at least one QTSP integration —
//! D-Trust or GlobalSign") is satisfied by follow-up beads
//! shipping a `dtrust`-feature provider and a `globalsign`-
//! feature provider. The substrate ships here so all engine
//! call sites can speak the trait today and the provider
//! impls drop in without touching the call surface.

use std::collections::BTreeMap;
use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use invoicekit_timestamping::RfcTimestamp;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// AdES signature family + level. eIDAS regulation [910/2014]
/// defines three signature *families* and four *levels* of
/// long-term archival:
///
/// * **Family** — CAdES (CMS / `application/pkcs7`), XAdES (XML),
///   PAdES (PDF).
/// * **Level**  — B (basic), T (with timestamp), LT
///   (long-term, embedded revocation data), LTA
///   (long-term + archival timestamp).
///
/// Year-1 InvoiceKit anchors are `Xades(AdesLevel::T)` for the
/// XML invoice payloads and `Pades(AdesLevel::Lt)` for the
/// Factur-X hybrid PDFs.
///
/// [910/2014]: https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=uriserv%3AOJ.L_.2014.257.01.0073.01.ENG
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
pub enum EidasSignatureProfile {
    /// CMS-style detached signature.
    Cades {
        /// AdES level.
        level: AdesLevel,
    },
    /// XML-embedded signature.
    Xades {
        /// AdES level.
        level: AdesLevel,
    },
    /// PDF-embedded signature.
    Pades {
        /// AdES level.
        level: AdesLevel,
    },
}

impl EidasSignatureProfile {
    /// `true` when this profile requires an RFC 3161 timestamp
    /// token alongside the signature value (levels T / LT / LTA).
    #[must_use]
    pub const fn requires_timestamp(self) -> bool {
        matches!(self.level(), AdesLevel::T | AdesLevel::Lt | AdesLevel::Lta)
    }

    /// `true` when this profile requires embedded revocation
    /// data (OCSP / CRL responses for the signing certificate
    /// chain) — levels LT and LTA.
    #[must_use]
    pub const fn requires_revocation_data(self) -> bool {
        matches!(self.level(), AdesLevel::Lt | AdesLevel::Lta)
    }

    /// AdES level component.
    #[must_use]
    pub const fn level(self) -> AdesLevel {
        match self {
            Self::Cades { level } | Self::Xades { level } | Self::Pades { level } => level,
        }
    }
}

/// AdES level (B / T / LT / LTA).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdesLevel {
    /// Basic — signature value + signing certificate.
    B,
    /// With timestamp — basic + RFC 3161 timestamp token over
    /// the signature value.
    T,
    /// Long-term — T + embedded revocation data for the cert
    /// chain (OCSP responses / CRLs).
    Lt,
    /// Long-term + archival — LT + archival timestamp.
    Lta,
}

/// Identifier of a qualified certificate issued by the QTSP.
/// Opaque, like [`KeyRef`]; the provider resolves it to the
/// underlying X.509 certificate chain.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QualifiedCertificateId(pub String);

impl QualifiedCertificateId {
    /// Build a new id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Subject + issuer summary of a qualified certificate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QualifiedCertificate {
    /// Opaque id within this provider.
    pub id: QualifiedCertificateId,
    /// Subject distinguished name string (RFC 4514 form).
    pub subject_dn: String,
    /// Issuer distinguished name string (RFC 4514 form).
    pub issuer_dn: String,
    /// Serial number, as an opaque ASCII / hex string.
    pub serial: String,
    /// `notBefore` (RFC 3339 UTC).
    pub not_before: String,
    /// `notAfter` (RFC 3339 UTC).
    pub not_after: String,
    /// True when the QTSP issues this as a *qualified*
    /// certificate per eIDAS Annex IV.
    pub qualified: bool,
}

/// Typed eIDAS signature.
///
/// Carries the AdES envelope bytes opaquely (the production
/// provider impls return CAdES `pkcs7-signature`, XAdES
/// `<ds:Signature>`, or PAdES embedded byte ranges). The
/// substrate doesn't parse the envelope; verification round-
/// trips through the originating provider.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EidasSignature {
    /// Underlying [`Signer`] receipt — the raw signature value
    /// + algorithm id + key ref.
    pub signature: Signature,
    /// AdES envelope bytes. Encoding depends on the profile:
    /// CAdES → DER-encoded `SignedData`; XAdES → UTF-8 XML;
    /// PAdES → bytes ready to splice into the PDF signature
    /// dictionary.
    pub ades_envelope: Vec<u8>,
    /// Profile this signature was produced at.
    pub profile: EidasSignatureProfile,
    /// Qualified certificate used.
    pub certificate: QualifiedCertificate,
    /// Optional RFC 3161 timestamp token (required for
    /// T/LT/LTA levels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<RfcTimestamp>,
    /// Optional embedded revocation data references (per-
    /// certificate OCSP / CRL response ids). Required for
    /// LT/LTA levels.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub revocation_refs: Vec<RevocationRef>,
}

/// Reference to one embedded revocation response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevocationRef {
    /// Issuer DN of the certificate this response covers.
    pub issuer_dn: String,
    /// Serial number of the certificate this response covers.
    pub serial: String,
    /// `ocsp` or `crl`.
    pub kind: String,
}

/// Per-step verification verdict.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckVerdict {
    /// Check passed.
    Passed,
    /// Check was skipped because the profile doesn't require it.
    Skipped,
    /// Check failed.
    Failed,
}

impl CheckVerdict {
    /// True when [`CheckVerdict::Failed`].
    #[must_use]
    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// Structured verification report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EidasVerifyReport {
    /// Aggregate verdict — `true` iff every required step
    /// passed.
    pub ok: bool,
    /// Certificate chain verifies against the QTSP's trust
    /// anchor.
    pub certificate_chain: CheckVerdict,
    /// Signature value verifies against the certificate's
    /// public key + the recorded payload.
    pub signature_value: CheckVerdict,
    /// RFC 3161 timestamp re-binds to the payload imprint
    /// (required for T/LT/LTA).
    pub timestamp: CheckVerdict,
    /// Embedded revocation data covers every cert in the
    /// chain (required for LT/LTA).
    pub revocation_data: CheckVerdict,
}

/// Errors raised by [`EidasQtspProvider`] implementations.
#[derive(Debug, Error)]
pub enum EidasError {
    /// Underlying signer refused.
    #[error("eIDAS provider's signer refused: {0}")]
    Signer(SigningError),
    /// Profile not supported by this provider (e.g. PAdES not
    /// available on a CAdES-only QTSP).
    #[error("profile not supported by this provider: {0:?}")]
    UnsupportedProfile(EidasSignatureProfile),
    /// Provider could not find the requested qualified
    /// certificate.
    #[error("qualified certificate not found: {0}")]
    UnknownCertificate(String),
    /// Provider is unreachable (network, mTLS failure).
    #[error("eIDAS QTSP unavailable: {0}")]
    Unavailable(String),
    /// Provider refused for a policy reason (revoked cert,
    /// audit-log full, etc.).
    #[error("eIDAS QTSP refused: {0}")]
    Refused(String),
}

/// Sign-request shape for an eIDAS-level signature.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EidasSignRequest {
    /// Bytes to sign (typically the canonical-form payload).
    pub payload: Vec<u8>,
    /// Qualified certificate id to sign under.
    pub certificate_id: QualifiedCertificateId,
    /// AdES profile + level to produce.
    pub profile: EidasSignatureProfile,
}

/// eIDAS provider surface.
///
/// Every QTSP integration implements this trait. The trait
/// bundles the underlying [`Signer`] so call sites can use one
/// provider object for both raw signing and AdES production.
pub trait EidasQtspProvider: Send + Sync {
    /// Identifier the operator sees in logs and the audit UI.
    fn provider_name(&self) -> &str;

    /// Find the qualified certificate by id.
    ///
    /// # Errors
    ///
    /// Returns [`EidasError::UnknownCertificate`] when the id
    /// is unknown to this provider.
    fn certificate(&self, id: &QualifiedCertificateId) -> Result<QualifiedCertificate, EidasError>;

    /// Produce an AdES signature.
    ///
    /// # Errors
    ///
    /// Returns [`EidasError`] when the profile is unsupported,
    /// the certificate is unknown, the underlying signer
    /// refuses, or the provider is unreachable.
    fn sign(&self, request: &EidasSignRequest) -> Result<EidasSignature, EidasError>;

    /// Verify an AdES signature against `payload`.
    ///
    /// # Errors
    ///
    /// Returns [`EidasError`] when verification fails fatally
    /// (transport error). Per-step verdicts go into
    /// [`EidasVerifyReport`] instead of raising.
    fn verify(
        &self,
        signature: &EidasSignature,
        payload: &[u8],
    ) -> Result<EidasVerifyReport, EidasError>;
}

/// Mock eIDAS provider. Records every call, returns
/// deterministic signatures + verification verdicts so tests
/// can exercise the surface without a live QTSP.
///
/// Build via [`MockEidasProvider::builder`] to load qualified
/// certificates + the backing signer.
pub struct MockEidasProvider {
    name: String,
    signer: std::sync::Arc<dyn Signer>,
    certificates: BTreeMap<QualifiedCertificateId, QualifiedCertificate>,
    timestamp_for: Option<RfcTimestamp>,
    calls: Mutex<Vec<EidasSignRequest>>,
}

impl MockEidasProvider {
    /// Start building a mock provider.
    #[must_use]
    pub fn builder(name: impl Into<String>) -> MockEidasProviderBuilder {
        MockEidasProviderBuilder {
            name: name.into(),
            signer: None,
            certificates: BTreeMap::new(),
            timestamp_for: None,
        }
    }

    /// Snapshot of every recorded sign request.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn calls(&self) -> Vec<EidasSignRequest> {
        self.calls.lock().unwrap().clone()
    }
}

/// Builder for [`MockEidasProvider`]. Chains certificate
/// registrations + the backing signer + an optional fixed
/// timestamp for T+ profiles.
pub struct MockEidasProviderBuilder {
    name: String,
    signer: Option<std::sync::Arc<dyn Signer>>,
    certificates: BTreeMap<QualifiedCertificateId, QualifiedCertificate>,
    timestamp_for: Option<RfcTimestamp>,
}

impl MockEidasProviderBuilder {
    /// Set the backing signer.
    #[must_use]
    pub fn with_signer(mut self, signer: std::sync::Arc<dyn Signer>) -> Self {
        self.signer = Some(signer);
        self
    }

    /// Register a qualified certificate.
    #[must_use]
    pub fn with_certificate(mut self, certificate: QualifiedCertificate) -> Self {
        self.certificates
            .insert(certificate.id.clone(), certificate);
        self
    }

    /// Set the timestamp token returned for T+ profiles.
    #[must_use]
    pub fn with_timestamp(mut self, timestamp: RfcTimestamp) -> Self {
        self.timestamp_for = Some(timestamp);
        self
    }

    /// Finalise the build. Returns `None` when no signer was
    /// supplied.
    #[must_use]
    pub fn build(self) -> Option<MockEidasProvider> {
        let signer = self.signer?;
        Some(MockEidasProvider {
            name: self.name,
            signer,
            certificates: self.certificates,
            timestamp_for: self.timestamp_for,
            calls: Mutex::new(Vec::new()),
        })
    }
}

impl EidasQtspProvider for MockEidasProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn certificate(&self, id: &QualifiedCertificateId) -> Result<QualifiedCertificate, EidasError> {
        self.certificates
            .get(id)
            .cloned()
            .ok_or_else(|| EidasError::UnknownCertificate(id.as_str().to_owned()))
    }

    fn sign(&self, request: &EidasSignRequest) -> Result<EidasSignature, EidasError> {
        let cert = self.certificate(&request.certificate_id)?;
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(request.certificate_id.as_str()),
                payload: request.payload.clone(),
            })
            .map_err(EidasError::Signer)?;
        // Mock AdES envelope: deterministic prefix + the raw
        // signature value + a recorded profile tag. Production
        // impls return real CAdES/XAdES/PAdES bytes.
        let envelope = mock_ades_envelope(&signature, request.profile);
        let timestamp = if request.profile.requires_timestamp() {
            self.timestamp_for.clone()
        } else {
            None
        };
        let revocation_refs = if request.profile.requires_revocation_data() {
            vec![RevocationRef {
                issuer_dn: cert.issuer_dn.clone(),
                serial: cert.serial.clone(),
                kind: "ocsp".to_owned(),
            }]
        } else {
            Vec::new()
        };
        self.calls.lock().unwrap().push(request.clone());
        Ok(EidasSignature {
            signature,
            ades_envelope: envelope,
            profile: request.profile,
            certificate: cert,
            timestamp,
            revocation_refs,
        })
    }

    fn verify(
        &self,
        signature: &EidasSignature,
        payload: &[u8],
    ) -> Result<EidasVerifyReport, EidasError> {
        // Re-sign the payload through the backing signer and
        // compare to the recorded signature value; that
        // simulates the cert-chain + signature-value checks
        // for the mock.
        let recomputed = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(signature.certificate.id.as_str()),
                payload: payload.to_vec(),
            })
            .map_err(EidasError::Signer)?;
        let sig_ok = recomputed.signature_b64 == signature.signature.signature_b64
            && recomputed.algorithm == signature.signature.algorithm;
        let signature_value = if sig_ok {
            CheckVerdict::Passed
        } else {
            CheckVerdict::Failed
        };
        // Cert chain: passes when the certificate id is known.
        let certificate_chain = if self.certificates.contains_key(&signature.certificate.id) {
            CheckVerdict::Passed
        } else {
            CheckVerdict::Failed
        };
        let timestamp = if signature.profile.requires_timestamp() {
            if signature.timestamp.is_some() {
                CheckVerdict::Passed
            } else {
                CheckVerdict::Failed
            }
        } else {
            CheckVerdict::Skipped
        };
        let revocation_data = if signature.profile.requires_revocation_data() {
            if !signature.revocation_refs.is_empty() {
                CheckVerdict::Passed
            } else {
                CheckVerdict::Failed
            }
        } else {
            CheckVerdict::Skipped
        };
        let ok = !certificate_chain.is_failed()
            && !signature_value.is_failed()
            && !timestamp.is_failed()
            && !revocation_data.is_failed();
        Ok(EidasVerifyReport {
            ok,
            certificate_chain,
            signature_value,
            timestamp,
            revocation_data,
        })
    }
}

fn mock_ades_envelope(signature: &Signature, profile: EidasSignatureProfile) -> Vec<u8> {
    let tag = match profile {
        EidasSignatureProfile::Cades { .. } => "CAdES",
        EidasSignatureProfile::Xades { .. } => "XAdES",
        EidasSignatureProfile::Pades { .. } => "PAdES",
    };
    let level = match profile.level() {
        AdesLevel::B => "B",
        AdesLevel::T => "T",
        AdesLevel::Lt => "LT",
        AdesLevel::Lta => "LTA",
    };
    let mut bytes = format!(
        "mock-ades-envelope:family={tag}:level={level}:alg={}:sig=",
        signature.algorithm
    )
    .into_bytes();
    bytes.extend_from_slice(signature.signature_b64.as_bytes());
    bytes
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_eidas::crate_name(),
///     "invoicekit-signer-eidas"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-eidas"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use invoicekit_timestamping::{
        HashAlgorithm, MockTimestampClient, TimestampClient, TimestampRequest,
    };
    use std::sync::Arc;

    fn sample_cert() -> QualifiedCertificate {
        QualifiedCertificate {
            id: QualifiedCertificateId::new("qcert-sample"),
            subject_dn: "CN=Acme,O=Acme GmbH,C=DE".to_owned(),
            issuer_dn: "CN=Test QTSP,O=Test QTSP,C=DE".to_owned(),
            serial: "1234567890".to_owned(),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-12-31T23:59:59Z".to_owned(),
            qualified: true,
        }
    }

    fn build_provider(profile_requires_ts: bool) -> MockEidasProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key("qcert-sample", [42_u8; 32]));
        let mut builder = MockEidasProvider::builder("test-qtsp")
            .with_signer(signer)
            .with_certificate(sample_cert());
        if profile_requires_ts {
            let ts_client = MockTimestampClient::new();
            let ts = ts_client
                .request_timestamp(&TimestampRequest {
                    algorithm: HashAlgorithm::Sha256,
                    message_imprint: vec![7_u8; 32],
                    nonce: None,
                    cert_req: true,
                })
                .unwrap();
            builder = builder.with_timestamp(ts);
        }
        builder.build().expect("signer must be set")
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-eidas");
    }

    #[test]
    fn profile_helpers_match_eidas_matrix() {
        // B never needs timestamp or revocation.
        let b = EidasSignatureProfile::Xades {
            level: AdesLevel::B,
        };
        assert!(!b.requires_timestamp());
        assert!(!b.requires_revocation_data());

        // T needs timestamp, not revocation.
        let t = EidasSignatureProfile::Cades {
            level: AdesLevel::T,
        };
        assert!(t.requires_timestamp());
        assert!(!t.requires_revocation_data());

        // LT needs both.
        let lt = EidasSignatureProfile::Pades {
            level: AdesLevel::Lt,
        };
        assert!(lt.requires_timestamp());
        assert!(lt.requires_revocation_data());

        // LTA needs both.
        let lta = EidasSignatureProfile::Pades {
            level: AdesLevel::Lta,
        };
        assert!(lta.requires_timestamp());
        assert!(lta.requires_revocation_data());
    }

    #[test]
    fn provider_finds_registered_certificate() {
        let provider = build_provider(false);
        let cert = provider
            .certificate(&QualifiedCertificateId::new("qcert-sample"))
            .unwrap();
        assert!(cert.qualified);
    }

    #[test]
    fn provider_rejects_unknown_certificate() {
        let provider = build_provider(false);
        let err = provider
            .certificate(&QualifiedCertificateId::new("nope"))
            .unwrap_err();
        assert!(matches!(err, EidasError::UnknownCertificate(_)));
    }

    #[test]
    fn provider_produces_basic_xades_signature() {
        let provider = build_provider(false);
        let request = EidasSignRequest {
            payload: b"<Invoice/>".to_vec(),
            certificate_id: QualifiedCertificateId::new("qcert-sample"),
            profile: EidasSignatureProfile::Xades {
                level: AdesLevel::B,
            },
        };
        let signature = provider.sign(&request).unwrap();
        assert_eq!(
            signature.profile,
            EidasSignatureProfile::Xades {
                level: AdesLevel::B
            }
        );
        assert!(signature.timestamp.is_none(), "B level has no timestamp");
        assert!(
            signature.revocation_refs.is_empty(),
            "B level has no revocation"
        );
        assert_eq!(provider.calls().len(), 1);
    }

    #[test]
    fn provider_attaches_timestamp_for_t_level() {
        let provider = build_provider(true);
        let request = EidasSignRequest {
            payload: b"<Invoice/>".to_vec(),
            certificate_id: QualifiedCertificateId::new("qcert-sample"),
            profile: EidasSignatureProfile::Xades {
                level: AdesLevel::T,
            },
        };
        let signature = provider.sign(&request).unwrap();
        assert!(signature.timestamp.is_some());
        assert!(
            signature.revocation_refs.is_empty(),
            "T level skips revocation"
        );
    }

    #[test]
    fn provider_attaches_revocation_for_lt_level() {
        let provider = build_provider(true);
        let request = EidasSignRequest {
            payload: b"%PDF-1.7".to_vec(),
            certificate_id: QualifiedCertificateId::new("qcert-sample"),
            profile: EidasSignatureProfile::Pades {
                level: AdesLevel::Lt,
            },
        };
        let signature = provider.sign(&request).unwrap();
        assert!(signature.timestamp.is_some());
        assert_eq!(signature.revocation_refs.len(), 1);
        assert_eq!(signature.revocation_refs[0].kind, "ocsp");
    }

    #[test]
    fn verify_passes_for_untampered_signature() {
        let provider = build_provider(true);
        let payload = b"<Invoice/>".to_vec();
        let signature = provider
            .sign(&EidasSignRequest {
                payload: payload.clone(),
                certificate_id: QualifiedCertificateId::new("qcert-sample"),
                profile: EidasSignatureProfile::Xades {
                    level: AdesLevel::T,
                },
            })
            .unwrap();
        let report = provider.verify(&signature, &payload).unwrap();
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.certificate_chain, CheckVerdict::Passed);
        assert_eq!(report.signature_value, CheckVerdict::Passed);
        assert_eq!(report.timestamp, CheckVerdict::Passed);
        assert_eq!(report.revocation_data, CheckVerdict::Skipped);
    }

    #[test]
    fn verify_fails_when_payload_drifted() {
        let provider = build_provider(false);
        let signature = provider
            .sign(&EidasSignRequest {
                payload: b"<Invoice/>".to_vec(),
                certificate_id: QualifiedCertificateId::new("qcert-sample"),
                profile: EidasSignatureProfile::Xades {
                    level: AdesLevel::B,
                },
            })
            .unwrap();
        let report = provider.verify(&signature, b"<Tampered/>").unwrap();
        assert!(!report.ok);
        assert_eq!(report.signature_value, CheckVerdict::Failed);
    }

    #[test]
    fn verify_fails_for_t_level_without_timestamp() {
        let provider = build_provider(false); // no timestamp configured
                                              // Build a signature record claiming T level but without
                                              // a timestamp; the verifier must call it out.
        let sig = provider
            .sign(&EidasSignRequest {
                payload: b"<x/>".to_vec(),
                certificate_id: QualifiedCertificateId::new("qcert-sample"),
                profile: EidasSignatureProfile::Xades {
                    level: AdesLevel::T,
                },
            })
            .unwrap();
        assert!(sig.timestamp.is_none());
        let report = provider.verify(&sig, b"<x/>").unwrap();
        assert_eq!(report.timestamp, CheckVerdict::Failed);
        assert!(!report.ok);
    }

    #[test]
    fn profile_round_trips_through_json() {
        let profile = EidasSignatureProfile::Pades {
            level: AdesLevel::Lt,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: EidasSignatureProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn check_verdict_predicates_match_variants() {
        assert!(CheckVerdict::Failed.is_failed());
        assert!(!CheckVerdict::Passed.is_failed());
        assert!(!CheckVerdict::Skipped.is_failed());
    }
}
