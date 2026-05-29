// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-094 impl: native Rust AS4 sender [`GatewayAdapter`] scaffold.
//!
//! Per the T-094 runbook (`docs/operators/NATIVE-RUST-AS4-SENDER.md`)
//! and AGENTS.md commitment #7, native AS4 is a research track. The
//! Year-1 production path is the partner AP adapter under
//! `crates/transmit-peppol-partner/` (T-091 impl). This crate is the
//! in-tree reference that the T-094 differential harness compares
//! against the phase4 JVM sidecar (T-092 impl).
//!
//! Public surface:
//!
//! * [`As4Envelope`] — typed AS4 SOAP envelope. Construction stays
//!   inside this crate; callers don't hand-build SOAP.
//! * [`Signer`] — trait abstraction over `XMLDSig` signing. The
//!   `xmlsec`-backed implementation lands behind the `xmlsec`
//!   cargo feature (follow-up bead). Tests use [`MockSigner`].
//! * [`Transport`] — abstraction over the HTTPS push to the
//!   recipient AP. The `reqwest`-backed transport lands behind the
//!   `reqwest` cargo feature. Tests use [`MockTransport`].
//! * [`SmpResolver`] — abstraction over the SMP / SML lookup. The
//!   `invoicekit-peppol-smp-sml`-backed implementation lands in a
//!   follow-up; tests inject [`StaticSmpResolver`].
//! * [`NativeAs4Adapter`] — implements
//!   [`invoicekit_reconcile::GatewayAdapter`] by assembling the
//!   envelope, signing it, and shipping it through the transport.

use std::pin::Pin;

pub mod byok;

use invoicekit_reconcile::{
    CancelRequest, CorrectRequest, GatewayAdapter, GatewayContext, GatewayError, GatewayErrorKind,
    GatewayFuture, GatewayOperation, GatewayReceipt, GatewayStatus, GatewaySubmissionId,
    PollRequest, SubmitRequest,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Recipient access point identity, resolved from the SMP / SML.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AccessPointTarget {
    /// HTTPS endpoint that accepts the AS4 push.
    pub endpoint_url: String,
    /// PEM-encoded recipient AP certificate. The signer uses this
    /// to encrypt the payload (when the recipient AP requires
    /// `XMLEnc`) and to validate the receipt `MDN`.
    pub recipient_cert_pem: String,
    /// AS4 transport profile slug. Fixed to AS4 v2.0 in 2026; the
    /// scaffold keeps the field so a future Peppol bump shows up
    /// as a typed migration, not a silent change.
    pub transport_profile: String,
}

/// One outbound AS4 envelope. Constructed by the adapter, signed
/// by the [`Signer`], and pushed by the [`Transport`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct As4Envelope {
    /// SOAP body bytes — the canonical UBL XML the sender hands
    /// us, wrapped in the AS4 user-message envelope.
    pub soap_body: Vec<u8>,
    /// Per-message ID generated locally; surfaces in the
    /// recipient's MDN as `RefToMessageId`.
    pub message_id: String,
    /// Recipient access point.
    pub recipient: AccessPointTarget,
}

/// `XMLDSig` signer abstraction.
pub trait Signer: Send + Sync {
    /// Sign `envelope` in place. Returns the signed bytes (the
    /// `XMLDSig` signature is embedded inside the SOAP envelope).
    ///
    /// # Errors
    ///
    /// Returns [`SignerError::SigningFailed`] when the signer
    /// rejects the payload (e.g. missing AP certificate).
    fn sign(&self, envelope: &mut As4Envelope) -> Result<(), SignerError>;
}

/// Errors raised by [`Signer`] implementations.
#[derive(Debug, Error)]
pub enum SignerError {
    /// The signer rejected the payload.
    #[error("`XMLDSig` signing failed: {0}")]
    SigningFailed(String),
}

/// Boxed future returned by [`Transport::push`].
pub type PushFuture<'a> =
    Pin<Box<dyn std::future::Future<Output = Result<TransportResponse, TransportError>> + Send + 'a>>;

/// HTTPS transport abstraction.
pub trait Transport: Send + Sync {
    /// Push the signed envelope to the recipient AP. Returns the
    /// receipt `MDN` bytes + HTTP status. The status mirrors what
    /// the partner adapter consumes so failure mapping is uniform
    /// across both T-091 and T-094.
    fn push(&self, envelope: &As4Envelope) -> PushFuture<'_>;
}

/// Response shape returned by [`Transport::push`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportResponse {
    /// HTTP status code (200 = receipt `MDN` attached, 5xx = retry).
    pub status: u16,
    /// Receipt MDN bytes (the XMLDSig-signed AS4 receipt).
    pub mdn_body: Vec<u8>,
}

/// Errors raised by [`Transport`] implementations.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The underlying network transport failed.
    #[error("AS4 transport error: {0}")]
    Network(String),
}

/// SMP / SML lookup abstraction.
pub trait SmpResolver: Send + Sync {
    /// Resolve the recipient participant + document type to an
    /// access point endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`SmpError::NotFound`] when the SML has no entry
    /// for the participant or the participant doesn't accept the
    /// requested document type.
    fn resolve(
        &self,
        participant: &str,
        document_type: &str,
    ) -> Result<AccessPointTarget, SmpError>;
}

/// Errors raised by [`SmpResolver`].
#[derive(Debug, Error)]
pub enum SmpError {
    /// The SML has no entry for the participant or document type.
    #[error("SMP lookup failed: {0}")]
    NotFound(String),
}

/// Adapter configuration. Lookup-driven so the test harness can
/// inject without touching the global env.
#[derive(Clone, Debug)]
pub struct NativeAs4Config {
    /// The sending AP's Peppol-issued certificate, PEM-encoded.
    /// Read by the signer for the `XMLDSig` key.
    pub ap_cert_pem: String,
    /// SML mode — `acceptance` or `production`. The reconcile
    /// state machine refuses sandbox + production-tagged invoices.
    pub sml_mode: SmlMode,
}

/// SML environment selector.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SmlMode {
    /// `edelivery.tech.ec.europa.eu` (test SML).
    Acceptance,
    /// `edelivery.tech.ec.europa.eu` production SML.
    Production,
}

/// Configuration / scaffold-level errors.
#[derive(Debug, Error)]
pub enum NativeAs4Error {
    /// SMP / SML lookup failed.
    #[error("SMP lookup failed: {0}")]
    Smp(#[from] SmpError),
    /// `XMLDSig` signing failed.
    #[error("signing failed: {0}")]
    Signer(#[from] SignerError),
    /// HTTPS transport failed.
    #[error("transport failed: {0}")]
    Transport(#[from] TransportError),
    /// UBL serialisation failed at the IR layer.
    #[error("UBL serialisation failed: {0}")]
    Ubl(String),
}

/// The adapter. Holds the config + the three injected trait
/// implementations; the rest of the surface is pure logic.
pub struct NativeAs4Adapter {
    // The scaffold does not consult `config` yet; the
    // `xmlsec`-backed signer follow-up will read `ap_cert_pem`
    // + `sml_mode` from it. Keeping the field on the struct now
    // keeps the constructor signature stable.
    #[allow(dead_code)]
    config: NativeAs4Config,
    smp: Box<dyn SmpResolver>,
    signer: Box<dyn Signer>,
    transport: Box<dyn Transport>,
}

impl NativeAs4Adapter {
    /// Build a new adapter.
    #[must_use]
    pub fn new(
        config: NativeAs4Config,
        smp: Box<dyn SmpResolver>,
        signer: Box<dyn Signer>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            config,
            smp,
            signer,
            transport,
        }
    }

    /// Recipient participant identifier extracted from the route.
    ///
    /// In production, the route's `country_iso` + the IR document
    /// resolve to a Peppol participant `iso6523-actorid-upis::…`.
    /// The scaffold reads the customer's first tax-id when set
    /// (Storecove + ecosio both accept this shape).
    fn recipient_participant_for(request: &SubmitRequest) -> String {
        request.document.customer.tax_ids.first().map_or_else(
            || {
                format!(
                    "iso6523-actorid-upis::DE{:09}",
                    request.document.customer.tax_ids.len()
                )
            },
            |t| format!("iso6523-actorid-upis::{}", t.value),
        )
    }

    /// Shared outbound pipeline: SMP lookup → envelope build → sign →
    /// push → decode. Both [`GatewayAdapter::submit`] and
    /// [`GatewayAdapter::correct`] run this with a per-operation
    /// participant + message id; the error mapping is identical bar
    /// the [`GatewayOperation`] tag.
    async fn dispatch(
        &self,
        context: &GatewayContext,
        operation: GatewayOperation,
        participant: &str,
        message_id: String,
        xml: &str,
    ) -> Result<GatewayReceipt, GatewayError> {
        let recipient = self
            .smp
            .resolve(participant, "peppol-bis-billing-3")
            .map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::NotFound,
                    operation,
                    format!("native-as4 adapter: SMP lookup failed: {e}"),
                    "verify the recipient participant identifier is registered on the Peppol SML",
                )
            })?;
        let mut envelope = As4Envelope {
            soap_body: build_as4_envelope_bytes(message_id.as_bytes(), xml.as_bytes()),
            message_id,
            recipient,
        };
        self.signer.sign(&mut envelope).map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::CertificateRejected,
                operation,
                format!("native-as4 adapter: `XMLDSig` signing failed: {e}"),
                "verify the AP certificate is current and the signer has access to its private key",
            )
        })?;
        let response = self.transport.push(&envelope).await.map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::NetworkFailure,
                operation,
                format!("native-as4 adapter: transport failed: {e}"),
                "check recipient AP endpoint reachability + your firewall rules",
            )
        })?;
        decode_response(context, &response, operation, &envelope.message_id)
    }
}

impl GatewayAdapter for NativeAs4Adapter {
    fn submit(&self, request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let participant = Self::recipient_participant_for(&request);
        Box::pin(async move {
            let xml = invoicekit_format_ubl::to_xml(&request.document).map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::InvalidRequest,
                    GatewayOperation::Submit,
                    format!("native-as4 adapter: UBL serialisation failed: {e}"),
                    "fix the IR document so format-ubl can serialise it",
                )
            })?;
            let message_id = format!(
                "ik:{}-{}",
                request.context.tenant_id.as_str(),
                request.context.gateway_attempt_id.as_str()
            );
            self.dispatch(
                &request.context,
                GatewayOperation::Submit,
                &participant,
                message_id,
                &xml,
            )
            .await
        })
    }

    fn poll(&self, request: PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
        // Native AS4 is push-only on the sender side; poll is a
        // local operation that just looks up the prior receipt
        // status via the outbox. The scaffold returns a Pending
        // receipt; a follow-up bead wires the actual outbox lookup.
        let submission_id = request.submission_id.clone();
        Box::pin(async move {
            GatewayReceipt::new(
                GatewayOperation::Poll,
                request.context.clone(),
                submission_id,
                GatewayStatus::Pending,
                "1970-01-01T00:00:00Z",
            )
            .map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::MalformedReceipt,
                    GatewayOperation::Poll,
                    format!("native-as4 adapter receipt envelope rejected: {e}"),
                    "report this as a native-as4 adapter bug",
                )
            })
        })
    }

    fn cancel(&self, _request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            Err(GatewayError::new(
                GatewayErrorKind::UnsupportedOperation,
                GatewayOperation::Cancel,
                "cancel is not supported by the native AS4 adapter",
                "Peppol invoices are immutable post-submit; use correct() to issue a credit note instead",
            ))
        })
    }

    fn correct(&self, request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
        // Correction is a fresh submit with a reference to the
        // prior submission. The submit path already handles the
        // full envelope + signing flow.
        let participant_owner = request.context.tenant_id.as_str().to_owned();
        Box::pin(async move {
            let xml = invoicekit_format_ubl::to_xml(&request.corrected_document).map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::InvalidRequest,
                    GatewayOperation::Correct,
                    format!("native-as4 adapter: UBL serialisation failed: {e}"),
                    "fix the IR document so format-ubl can serialise it",
                )
            })?;
            let recipient_participant = request
                .corrected_document
                .customer
                .tax_ids
                .first()
                .map_or_else(
                    || format!("iso6523-actorid-upis::{participant_owner}"),
                    |t| format!("iso6523-actorid-upis::{}", t.value),
                );
            let message_id = format!(
                "ik:{}-{}-correct",
                request.context.tenant_id.as_str(),
                request.context.gateway_attempt_id.as_str()
            );
            self.dispatch(
                &request.context,
                GatewayOperation::Correct,
                &recipient_participant,
                message_id,
                &xml,
            )
            .await
        })
    }
}

fn build_as4_envelope_bytes(message_id: &[u8], soap_body: &[u8]) -> Vec<u8> {
    // Minimal SOAP envelope shape — the real implementation
    // emits the AS4 / ebMS3 SOAP headers; this scaffold writes
    // the body wrapper around the canonical UBL bytes so the
    // signer + transport can be exercised end-to-end. Sufficient
    // for the differential harness (T-094 follow-up) once the
    // xmlsec signer lands.
    let mut out =
        b"<soap:Envelope xmlns:soap=\"http://www.w3.org/2003/05/soap-envelope\"><soap:Header><eb:Messaging xmlns:eb=\"http://docs.oasis-open.org/ebxml-msg/ebms/v3.0/ns/core/200704/\"><eb:UserMessage><eb:MessageInfo><eb:MessageId>".to_vec();
    out.extend_from_slice(message_id);
    out.extend_from_slice(b"</eb:MessageId></eb:MessageInfo></eb:UserMessage></eb:Messaging></soap:Header><soap:Body>");
    out.extend_from_slice(soap_body);
    out.extend_from_slice(b"</soap:Body></soap:Envelope>");
    out
}

fn decode_response(
    context: &GatewayContext,
    response: &TransportResponse,
    operation: GatewayOperation,
    message_id: &str,
) -> Result<GatewayReceipt, GatewayError> {
    if (200..300).contains(&response.status) {
        // The receipt `MDN` is signal that the recipient AP accepted
        // the push. Mark the receipt Pending; the actual delivery
        // status flows back via the recipient's downstream MDN.
        let submission_id = GatewaySubmissionId::new(message_id).map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::MalformedReceipt,
                operation,
                format!("native-as4 adapter: message_id rejected as submission id: {e}"),
                "report this as a native-as4 adapter bug",
            )
        })?;
        GatewayReceipt::new(
            operation,
            context.clone(),
            submission_id,
            GatewayStatus::Pending,
            "1970-01-01T00:00:00Z",
        )
        .map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::MalformedReceipt,
                operation,
                format!("native-as4 adapter receipt envelope rejected: {e}"),
                "report this as a native-as4 adapter bug",
            )
        })
    } else {
        Err(GatewayError::new(
            map_status(response.status),
            operation,
            format!(
                "native-as4 push returned HTTP {} (MDN bytes={})",
                response.status,
                response.mdn_body.len()
            ),
            "consult the recipient AP's status; re-try after the documented backoff",
        ))
    }
}

fn map_status(status: u16) -> GatewayErrorKind {
    match status {
        401 | 403 => GatewayErrorKind::AuthFailure,
        404 => GatewayErrorKind::NotFound,
        408 => GatewayErrorKind::Timeout,
        409 => GatewayErrorKind::DuplicateSubmission,
        422 => GatewayErrorKind::Rejected,
        429 => GatewayErrorKind::RateLimited,
        500..=599 => GatewayErrorKind::GatewayMaintenance,
        _ => GatewayErrorKind::PartnerError,
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_transmit_peppol_native_as4::crate_name(),
///     "invoicekit-transmit-peppol-native-as4"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-peppol-native-as4"
}

// ----- test scaffolding ------------------------------------------

/// Mock [`Signer`] that records calls and returns success.
#[derive(Clone, Debug, Default)]
pub struct MockSigner {
    /// Count of `sign()` invocations.
    pub sign_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Signer for MockSigner {
    fn sign(&self, envelope: &mut As4Envelope) -> Result<(), SignerError> {
        self.sign_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Append a stub signature marker so the test can assert
        // the signer ran without doing real XMLDSig work.
        envelope.soap_body.extend_from_slice(b"<!--SIGNED-->");
        Ok(())
    }
}

/// Mock [`Transport`] that records pushes + returns queued responses.
pub struct MockTransport {
    queued: std::sync::Mutex<Vec<Result<TransportResponse, TransportError>>>,
    pushed: std::sync::Mutex<Vec<As4Envelope>>,
}

impl MockTransport {
    /// Build with a FIFO queue of responses.
    #[must_use]
    pub fn new(responses: Vec<Result<TransportResponse, TransportError>>) -> Self {
        Self {
            queued: std::sync::Mutex::new(responses.into_iter().rev().collect()),
            pushed: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of envelopes pushed so far.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    #[must_use]
    pub fn pushed(&self) -> Vec<As4Envelope> {
        self.pushed.lock().unwrap().clone()
    }
}

impl Transport for MockTransport {
    fn push(&self, envelope: &As4Envelope) -> PushFuture<'_> {
        self.pushed.lock().unwrap().push(envelope.clone());
        let response = self
            .queued
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| Err(TransportError::Network("no queued response".to_owned())));
        Box::pin(async move { response })
    }
}

/// Static [`SmpResolver`] for tests.
#[derive(Clone, Debug)]
pub struct StaticSmpResolver {
    target: AccessPointTarget,
}

impl StaticSmpResolver {
    /// Build a resolver that returns the given target for every lookup.
    #[must_use]
    pub fn new(target: AccessPointTarget) -> Self {
        Self { target }
    }
}

impl SmpResolver for StaticSmpResolver {
    fn resolve(
        &self,
        _participant: &str,
        _document_type: &str,
    ) -> Result<AccessPointTarget, SmpError> {
        Ok(self.target.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-transmit-peppol-native-as4");
    }

    #[test]
    fn build_envelope_wraps_body_in_soap_envelope() {
        let env = build_as4_envelope_bytes(b"msg-1", b"<Invoice/>");
        let env_str = std::str::from_utf8(&env).unwrap();
        assert!(env_str.contains("<soap:Envelope"));
        assert!(env_str.contains("<eb:MessageId>msg-1</eb:MessageId>"));
        assert!(env_str.contains("<Invoice/>"));
        assert!(env_str.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn map_status_covers_documented_codes() {
        assert!(matches!(map_status(401), GatewayErrorKind::AuthFailure));
        assert!(matches!(map_status(403), GatewayErrorKind::AuthFailure));
        assert!(matches!(map_status(404), GatewayErrorKind::NotFound));
        assert!(matches!(map_status(408), GatewayErrorKind::Timeout));
        assert!(matches!(
            map_status(409),
            GatewayErrorKind::DuplicateSubmission
        ));
        assert!(matches!(map_status(422), GatewayErrorKind::Rejected));
        assert!(matches!(map_status(429), GatewayErrorKind::RateLimited));
        assert!(matches!(
            map_status(503),
            GatewayErrorKind::GatewayMaintenance
        ));
        assert!(matches!(map_status(418), GatewayErrorKind::PartnerError));
    }

    #[test]
    fn mock_signer_records_calls_and_appends_marker() {
        let signer = MockSigner::default();
        let mut env = As4Envelope {
            soap_body: b"<x/>".to_vec(),
            message_id: "m".to_owned(),
            recipient: AccessPointTarget {
                endpoint_url: "https://example.com/as4".to_owned(),
                recipient_cert_pem: "-----BEGIN CERTIFICATE-----\nxxx\n-----END CERTIFICATE-----"
                    .to_owned(),
                transport_profile: "as4-v2.0".to_owned(),
            },
        };
        signer.sign(&mut env).unwrap();
        assert_eq!(
            signer.sign_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(env
            .soap_body
            .windows(b"<!--SIGNED-->".len())
            .any(|w| w == b"<!--SIGNED-->"));
    }

    #[test]
    fn static_smp_resolver_returns_target_for_any_input() {
        let target = AccessPointTarget {
            endpoint_url: "https://ap.example.com/as4".to_owned(),
            recipient_cert_pem: "-----BEGIN CERTIFICATE-----\nyyy\n-----END CERTIFICATE-----"
                .to_owned(),
            transport_profile: "as4-v2.0".to_owned(),
        };
        let resolver = StaticSmpResolver::new(target.clone());
        let resolved_target = resolver
            .resolve("iso6523-actorid-upis::0192:12345", "peppol-bis-billing-3")
            .unwrap();
        assert_eq!(resolved_target, target);
    }

    #[test]
    fn mock_transport_records_pushes() {
        let target = AccessPointTarget {
            endpoint_url: "https://ap.example.com/as4".to_owned(),
            recipient_cert_pem: "-".to_owned(),
            transport_profile: "as4-v2.0".to_owned(),
        };
        let env = As4Envelope {
            soap_body: b"<x/>".to_vec(),
            message_id: "m".to_owned(),
            recipient: target,
        };
        let transport = MockTransport::new(vec![Ok(TransportResponse {
            status: 200,
            mdn_body: b"<mdn/>".to_vec(),
        })]);
        let response = pollster::block_on(transport.push(&env)).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(transport.pushed().len(), 1);
        assert_eq!(transport.pushed()[0].message_id, "m");
    }
}
