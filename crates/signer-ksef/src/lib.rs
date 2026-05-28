// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// KSeF / FA(3) / UPO / NIP / MF acronyms trip doc-markdown.
#![allow(clippy::doc_markdown)]

//! `invoicekit-signer-ksef` — Poland KSeF (Krajowy System
//! e-Faktur) certificate-flow adapter.
//!
//! Layers the Poland Ministry of Finance KSeF contract on top
//! of [`invoicekit_signer`]. KSeF is a portal-clearance flow:
//! the taxpayer authenticates against KSeF (via the qualified
//! signature on an `InitSessionRequest` or via a tokenized
//! API), receives a session token, submits the FA(3) XML, and
//! the portal returns a KSeF reference number (`Numer KSeF`)
//! that closes the invoice.
//!
//! Public surface:
//!
//! * [`KsefProvider`] — provider trait every KSeF integration
//!   implements.
//! * [`KsefEnvironment`] — `Demo` (sandbox) vs `Production`.
//! * [`SessionToken`] — typed session token returned by the
//!   KSeF `InitSession*` endpoints.
//! * [`KsefStampEnvelope`] — typed envelope: KSeF reference
//!   number + UPO acknowledgement reference + acceptance
//!   status + session-token lineage.
//! * [`MockKsefProvider`] — deterministic test provider.
//!
//! # Strict-gate scope
//!
//! Real KSeF integration needs (a) HTTPS to the KSeF REST API
//! (`ksef-test.mf.gov.pl` for demo, `ksef.mf.gov.pl` for prod),
//! (b) XAdES-signing of the InitSession payload, (c) NIP-bound
//! qualified certificate or KSeF tokenized credentials. The
//! substrate ships today; the real provider lands behind a
//! future `ksef-http` feature flag.

use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// KSeF environment — Demo is the sandbox the Ministry of
/// Finance maintains for onboarding; Production is the
/// post-2026 mandatory clearance environment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KsefEnvironment {
    /// Demo / sandbox environment (ksef-test.mf.gov.pl).
    Demo,
    /// Production environment (ksef.mf.gov.pl).
    Production,
}

impl KsefEnvironment {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Production => "production",
        }
    }
}

/// Authentication mode the KSeF session uses.
///
/// KSeF accepts either the taxpayer's qualified signature over
/// the InitSession payload (XAdES) or a pre-issued tokenized
/// credential the taxpayer generates from their KSeF account.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    /// Qualified electronic signature (XAdES).
    QualifiedSignature,
    /// Pre-issued KSeF authorisation token.
    AuthorisationToken,
}

/// Typed KSeF session token returned by `InitSession*`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct SessionToken {
    /// Opaque session token (UUID-shaped).
    pub session_token: String,
    /// `referenceNumber` KSeF returned with the session.
    pub reference_number: String,
    /// `notBefore` (RFC 3339 UTC).
    pub valid_from: String,
    /// `notAfter` (RFC 3339 UTC).
    pub valid_until: String,
    /// Auth mode used to mint the session.
    pub auth_mode: AuthMode,
    /// Environment the session targets.
    pub environment: KsefEnvironment,
}

/// KSeF acceptance status returned after invoice submission.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KsefAcceptance {
    /// Accepted — KSeF assigned a `Numer KSeF`.
    Accepted,
    /// Pending — invoice received, awaiting validation.
    Pending,
    /// Rejected — KSeF rejected the FA(3) schema or business
    /// rules.
    Rejected,
}

impl KsefAcceptance {
    /// True only when the invoice was accepted.
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Typed KSeF stamp envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KsefStampEnvelope {
    /// Underlying [`Signer`] receipt — the XAdES payload
    /// signature the taxpayer produced over the FA(3) XML.
    pub signature: Signature,
    /// `Numer KSeF` — the KSeF-assigned reference number that
    /// closes the invoice (25-character format
    /// `<NIP>-YYYYMMDD-<UUID>-XX`).
    pub numer_ksef: String,
    /// `UPO` (Urzędowe Poświadczenie Odbioru) reference id
    /// the portal returns once the invoice is processed.
    pub upo_reference: String,
    /// Acceptance status returned by KSeF.
    pub acceptance: KsefAcceptance,
    /// Session token used to submit the invoice.
    pub session: SessionToken,
}

/// Submission request shape for a KSeF FA(3) invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KsefSubmitRequest {
    /// FA(3) XML bytes (the canonical Polish e-invoice format).
    pub fa_xml: Vec<u8>,
    /// Active KSeF session token (from `init_session`).
    pub session: SessionToken,
    /// NIP (10-digit Polish tax id) the session is bound to.
    pub nip: String,
}

/// Errors raised by [`KsefProvider`] implementations.
#[derive(Debug, Error)]
pub enum KsefError {
    /// Underlying signer refused.
    #[error("ksef provider's signer refused: {0}")]
    Signer(SigningError),
    /// Session token mismatched the request environment.
    #[error("KSeF session environment mismatch: session={session:?}, request={request:?}")]
    EnvironmentMismatch {
        /// Environment the session token targets.
        session: KsefEnvironment,
        /// Environment the request targets.
        request: KsefEnvironment,
    },
    /// KSeF rejected the FA(3) schema or business rules.
    #[error("KSeF rejected the invoice: {0}")]
    InvoiceRejected(String),
    /// Session token expired before submission.
    #[error("KSeF session token expired: {0}")]
    SessionExpired(String),
    /// KSeF portal is unreachable.
    #[error("KSeF portal unavailable: {0}")]
    Unavailable(String),
}

/// KSeF provider surface.
pub trait KsefProvider: Send + Sync {
    /// Provider display name.
    fn provider_name(&self) -> &str;

    /// Environment this provider targets.
    fn environment(&self) -> KsefEnvironment;

    /// Open an authenticated session.
    ///
    /// # Errors
    ///
    /// Returns [`KsefError`] when the auth payload signing
    /// fails, the portal rejects the credentials, or the
    /// portal is unreachable.
    fn init_session(&self, nip: &str, auth_mode: AuthMode) -> Result<SessionToken, KsefError>;

    /// Submit an FA(3) invoice under the supplied session.
    ///
    /// # Errors
    ///
    /// Returns [`KsefError`] when the environment doesn't
    /// match, the session is expired, the portal rejects
    /// the invoice, or the underlying signer refuses.
    fn submit(
        &self,
        request: &KsefSubmitRequest,
        target_environment: KsefEnvironment,
    ) -> Result<KsefStampEnvelope, KsefError>;
}

/// Mock KSeF provider — deterministic test outputs.
pub struct MockKsefProvider {
    name: String,
    environment: KsefEnvironment,
    signer: std::sync::Arc<dyn Signer>,
    forced_acceptance: KsefAcceptance,
    sessions: Mutex<Vec<(String, AuthMode)>>,
    submissions: Mutex<Vec<KsefSubmitRequest>>,
    next_session_id: Mutex<u64>,
    next_invoice_id: Mutex<u64>,
}

impl MockKsefProvider {
    /// Build a mock provider.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        environment: KsefEnvironment,
        signer: std::sync::Arc<dyn Signer>,
    ) -> Self {
        Self {
            name: name.into(),
            environment,
            signer,
            forced_acceptance: KsefAcceptance::Accepted,
            sessions: Mutex::new(Vec::new()),
            submissions: Mutex::new(Vec::new()),
            next_session_id: Mutex::new(1),
            next_invoice_id: Mutex::new(1),
        }
    }

    /// Force the provider to return a specific acceptance
    /// outcome on every submit (Pending / Rejected for tests).
    #[must_use]
    pub fn with_forced_acceptance(mut self, acceptance: KsefAcceptance) -> Self {
        self.forced_acceptance = acceptance;
        self
    }

    /// Snapshot of every session ever opened.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn sessions(&self) -> Vec<(String, AuthMode)> {
        self.sessions.lock().unwrap().clone()
    }

    /// Snapshot of every submission ever made.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn submissions(&self) -> Vec<KsefSubmitRequest> {
        self.submissions.lock().unwrap().clone()
    }

    fn next_session_value(&self) -> u64 {
        let mut g = self.next_session_id.lock().expect("mutex poisoned");
        let n = *g;
        *g += 1;
        n
    }

    fn next_invoice_value(&self) -> u64 {
        let mut g = self.next_invoice_id.lock().expect("mutex poisoned");
        let n = *g;
        *g += 1;
        n
    }
}

impl KsefProvider for MockKsefProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn environment(&self) -> KsefEnvironment {
        self.environment
    }

    fn init_session(&self, nip: &str, auth_mode: AuthMode) -> Result<SessionToken, KsefError> {
        if nip.is_empty() {
            return Err(KsefError::InvoiceRejected("empty NIP".to_owned()));
        }
        let n = self.next_session_value();
        let token = SessionToken {
            session_token: format!("sess-{n:08}"),
            reference_number: format!("ref-{nip}-{n:06}"),
            valid_from: "2026-01-01T00:00:00Z".to_owned(),
            valid_until: "2026-12-31T23:59:59Z".to_owned(),
            auth_mode,
            environment: self.environment,
        };
        self.sessions
            .lock()
            .unwrap()
            .push((nip.to_owned(), auth_mode));
        Ok(token)
    }

    fn submit(
        &self,
        request: &KsefSubmitRequest,
        target_environment: KsefEnvironment,
    ) -> Result<KsefStampEnvelope, KsefError> {
        if request.session.environment != target_environment {
            return Err(KsefError::EnvironmentMismatch {
                session: request.session.environment,
                request: target_environment,
            });
        }
        if request.session.session_token.is_empty() {
            return Err(KsefError::SessionExpired(
                request.session.reference_number.clone(),
            ));
        }
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(&request.session.session_token),
                payload: request.fa_xml.clone(),
            })
            .map_err(KsefError::Signer)?;
        self.submissions.lock().unwrap().push(request.clone());
        let id = self.next_invoice_value();
        Ok(KsefStampEnvelope {
            signature,
            numer_ksef: format!("{nip}-20260528-{id:08}-AA", nip = request.nip),
            upo_reference: format!("upo-{id:08}"),
            acceptance: self.forced_acceptance,
            session: request.session.clone(),
        })
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_ksef::crate_name(),
///     "invoicekit-signer-ksef"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-ksef"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use std::sync::Arc;

    fn build_provider(env: KsefEnvironment) -> MockKsefProvider {
        let signer: Arc<dyn Signer> = Arc::new(
            SoftwareSigner::new()
                .with_key("sess-00000001", [5_u8; 32])
                .with_key("sess-00000002", [6_u8; 32]),
        );
        MockKsefProvider::new("test-ksef", env, signer)
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-ksef");
    }

    #[test]
    fn environment_round_trips_kebab_json() {
        assert_eq!(KsefEnvironment::Demo.slug(), "demo");
        assert_eq!(KsefEnvironment::Production.slug(), "production");
        let json = serde_json::to_string(&KsefEnvironment::Demo).unwrap();
        assert_eq!(json, "\"demo\"");
    }

    #[test]
    fn auth_mode_round_trips_kebab_json() {
        let json = serde_json::to_string(&AuthMode::QualifiedSignature).unwrap();
        assert_eq!(json, "\"qualified-signature\"");
        let back: AuthMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AuthMode::QualifiedSignature);
    }

    #[test]
    fn acceptance_predicate_matches_variants() {
        assert!(KsefAcceptance::Accepted.is_accepted());
        assert!(!KsefAcceptance::Pending.is_accepted());
        assert!(!KsefAcceptance::Rejected.is_accepted());
    }

    #[test]
    fn init_session_increments_session_id() {
        let provider = build_provider(KsefEnvironment::Demo);
        let a = provider
            .init_session("1234567890", AuthMode::QualifiedSignature)
            .unwrap();
        let b = provider
            .init_session("1234567890", AuthMode::AuthorisationToken)
            .unwrap();
        assert_ne!(a.session_token, b.session_token);
        assert_eq!(a.environment, KsefEnvironment::Demo);
        assert_eq!(provider.sessions().len(), 2);
    }

    #[test]
    fn init_session_rejects_empty_nip() {
        let provider = build_provider(KsefEnvironment::Demo);
        let err = provider
            .init_session("", AuthMode::QualifiedSignature)
            .unwrap_err();
        assert!(matches!(err, KsefError::InvoiceRejected(_)));
    }

    #[test]
    fn submit_rejects_environment_mismatch() {
        let provider = build_provider(KsefEnvironment::Demo);
        let session = provider
            .init_session("1234567890", AuthMode::QualifiedSignature)
            .unwrap();
        let err = provider
            .submit(
                &KsefSubmitRequest {
                    fa_xml: b"<FA/>".to_vec(),
                    session,
                    nip: "1234567890".to_owned(),
                },
                KsefEnvironment::Production,
            )
            .unwrap_err();
        assert!(matches!(err, KsefError::EnvironmentMismatch { .. }));
    }

    #[test]
    fn submit_produces_envelope_with_numer_ksef() {
        let provider = build_provider(KsefEnvironment::Demo);
        let session = provider
            .init_session("1234567890", AuthMode::QualifiedSignature)
            .unwrap();
        let envelope = provider
            .submit(
                &KsefSubmitRequest {
                    fa_xml: b"<FA/>".to_vec(),
                    session,
                    nip: "1234567890".to_owned(),
                },
                KsefEnvironment::Demo,
            )
            .unwrap();
        assert!(envelope.numer_ksef.starts_with("1234567890-20260528-"));
        assert!(envelope.upo_reference.starts_with("upo-"));
        assert_eq!(envelope.acceptance, KsefAcceptance::Accepted);
        assert_eq!(provider.submissions().len(), 1);
    }

    #[test]
    fn submit_propagates_forced_acceptance() {
        let provider =
            build_provider(KsefEnvironment::Demo).with_forced_acceptance(KsefAcceptance::Pending);
        let session = provider
            .init_session("1234567890", AuthMode::AuthorisationToken)
            .unwrap();
        let envelope = provider
            .submit(
                &KsefSubmitRequest {
                    fa_xml: b"<FA/>".to_vec(),
                    session,
                    nip: "1234567890".to_owned(),
                },
                KsefEnvironment::Demo,
            )
            .unwrap();
        assert_eq!(envelope.acceptance, KsefAcceptance::Pending);
    }

    #[test]
    fn submit_rejects_empty_session_token() {
        let provider = build_provider(KsefEnvironment::Demo);
        let mut session = provider
            .init_session("1234567890", AuthMode::QualifiedSignature)
            .unwrap();
        session.session_token = String::new();
        let err = provider
            .submit(
                &KsefSubmitRequest {
                    fa_xml: b"<FA/>".to_vec(),
                    session,
                    nip: "1234567890".to_owned(),
                },
                KsefEnvironment::Demo,
            )
            .unwrap_err();
        assert!(matches!(err, KsefError::SessionExpired(_)));
    }
}
