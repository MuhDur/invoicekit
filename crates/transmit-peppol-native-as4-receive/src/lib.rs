// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-095 impl: native Rust AS4 receiver scaffold.
//!
//! Mirror of T-094 (the sender) for the receive side. Per the
//! T-095 runbook (`docs/operators/NATIVE-RUST-AS4-RECEIVER.md`),
//! the receiver listens at `https://<host>:<port>/as4` with mTLS,
//! verifies the inbound `XMLDSig` signature against the sender AP's
//! certificate, optionally decrypts the payload, generates the
//! synchronous AS4 receipt MDN, and dispatches the unwrapped
//! canonical payload to the T-093 inbound pipeline.
//!
//! Public surface:
//!
//! * [`InboundEnvelope`] — typed inbound message (raw SOAP body
//!   plus sender / recipient participant ids plus message id).
//! * [`Verifier`] — trait abstraction over `XMLDSig` signature
//!   verification. `MockVerifier` shipped for tests; xmlsec-backed
//!   implementation lands behind the `xmlsec` cargo feature
//!   alongside the T-094 sender's xmlsec follow-up.
//! * [`Listener`] — trait abstraction over the mTLS HTTP listener.
//!   `MockListener` for tests; `axum`-backed listener lands behind
//!   the `axum` cargo feature (follow-up).
//! * [`Dispatcher`] — trait abstraction over the downstream T-093
//!   pipeline (format-detect → validate → archive). `MockDispatcher`
//!   for tests; production dispatcher posts the unwrapped payload
//!   to `INVOICEKIT_PEPPOL_RECEIVER_DISPATCH` per the runbook.
//! * [`NativeAs4Receiver`] — wires the three abstractions:
//!   parse → verify → dispatch → emit receipt MDN.

use std::pin::Pin;

use thiserror::Error;

pub use invoicekit_transmit_peppol_native_as4::{
    As4Envelope, NativeAs4Error, TransportError, TransportResponse,
};

/// Inbound AS4 envelope as received from the wire. Sender +
/// recipient participant ids are extracted from the SBDH header
/// during parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundEnvelope {
    /// Raw SOAP envelope bytes — fed back to the verifier
    /// without normalisation so the `XMLDSig` signature still
    /// covers the same bytes.
    pub raw_soap: Vec<u8>,
    /// Sender participant identifier (`iso6523-actorid-upis::…`).
    pub sender_participant: String,
    /// Recipient participant identifier.
    pub recipient_participant: String,
    /// Per-message ID surfaced as `RefToMessageId` in the
    /// receipt MDN.
    pub message_id: String,
}

/// `XMLDSig` verification abstraction.
pub trait Verifier: Send + Sync {
    /// Verify the inbound envelope's `XMLDSig` signature against
    /// the sender AP's certificate (looked up by participant id).
    ///
    /// # Errors
    ///
    /// Returns [`VerifierError::SignatureRejected`] when the
    /// signature does not validate or the sender's certificate is
    /// not trusted.
    fn verify(&self, envelope: &InboundEnvelope) -> Result<(), VerifierError>;
}

/// Errors raised by [`Verifier`] implementations.
#[derive(Debug, Error)]
pub enum VerifierError {
    /// The signature did not validate.
    #[error("AS4 signature rejected: {0}")]
    SignatureRejected(String),
    /// The sender's certificate is not trusted by the local
    /// Peppol PKI trust list.
    #[error("AS4 sender certificate untrusted: {0}")]
    CertificateUntrusted(String),
}

/// Downstream dispatcher abstraction — posts the unwrapped
/// payload to the T-093 inbound pipeline.
pub trait Dispatcher: Send + Sync {
    /// Hand the unwrapped XML payload to the T-093 inbound
    /// service. Returns the receipt MDN bytes the listener
    /// should write back synchronously.
    ///
    /// # Errors
    ///
    /// Returns [`DispatcherError::Downstream`] when the inbound
    /// pipeline rejects the payload (e.g. format-detect refuses
    /// the bytes).
    fn dispatch(
        &self,
        envelope: &InboundEnvelope,
        unwrapped_xml: &[u8],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, DispatcherError>> + Send + '_>>;
}

/// Errors raised by [`Dispatcher`] implementations.
#[derive(Debug, Error)]
pub enum DispatcherError {
    /// The downstream T-093 pipeline rejected the payload.
    #[error("inbound pipeline rejected payload: {0}")]
    Downstream(String),
}

/// Listener abstraction.
///
/// The production listener is an `axum` HTTP server with mTLS;
/// the mock listener pushes a queued inbound envelope on every
/// `accept()` call so the receiver integration tests stay
/// loop-free.
pub trait Listener: Send + Sync {
    /// Block until the next inbound envelope arrives.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerError::Closed`] when the listener has
    /// shut down (typical at process shutdown).
    fn accept(
        &self,
    ) -> Pin<
        Box<dyn std::future::Future<Output = Result<InboundEnvelope, ListenerError>> + Send + '_>,
    >;
}

/// Errors raised by [`Listener`] implementations.
#[derive(Debug, Error)]
pub enum ListenerError {
    /// The listener has shut down.
    #[error("AS4 listener closed: {0}")]
    Closed(String),
}

/// Outcome of processing one inbound envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiverOutcome {
    /// `RefToMessageId` value — the inbound message id.
    pub ref_to_message_id: String,
    /// Receipt MDN bytes the listener writes back synchronously.
    pub receipt_mdn: Vec<u8>,
}

/// Errors raised by [`NativeAs4Receiver::process_one`].
#[derive(Debug, Error)]
pub enum ReceiverError {
    /// The listener returned an error before an envelope arrived.
    #[error("listener error: {0}")]
    Listener(#[from] ListenerError),
    /// The verifier rejected the signature.
    #[error("verifier error: {0}")]
    Verifier(#[from] VerifierError),
    /// The dispatcher rejected the payload.
    #[error("dispatcher error: {0}")]
    Dispatcher(#[from] DispatcherError),
    /// The inbound envelope's SOAP body could not be parsed.
    #[error("inbound parse error: {0}")]
    Parse(String),
}

/// The receiver. Holds the listener + verifier + dispatcher; the
/// rest of the surface is pure logic.
pub struct NativeAs4Receiver {
    listener: Box<dyn Listener>,
    verifier: Box<dyn Verifier>,
    dispatcher: Box<dyn Dispatcher>,
}

impl NativeAs4Receiver {
    /// Build a new receiver.
    #[must_use]
    pub fn new(
        listener: Box<dyn Listener>,
        verifier: Box<dyn Verifier>,
        dispatcher: Box<dyn Dispatcher>,
    ) -> Self {
        Self {
            listener,
            verifier,
            dispatcher,
        }
    }

    /// Accept the next inbound envelope, verify its signature,
    /// dispatch the unwrapped payload, and return the receipt MDN.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiverError`] when any of the four steps
    /// (accept / verify / unwrap / dispatch) fails.
    pub async fn process_one(&self) -> Result<ReceiverOutcome, ReceiverError> {
        let envelope = self.listener.accept().await?;
        self.verifier.verify(&envelope)?;
        let unwrapped = unwrap_soap_body(&envelope.raw_soap)?;
        let receipt_mdn = self.dispatcher.dispatch(&envelope, &unwrapped).await?;
        Ok(ReceiverOutcome {
            ref_to_message_id: envelope.message_id.clone(),
            receipt_mdn,
        })
    }
}

/// Extract the bytes between `<soap:Body>` and `</soap:Body>`.
/// The full SBDH header parsing lives behind a follow-up bead;
/// this scaffold only needs the payload bytes so the dispatcher
/// has something to hand the T-093 pipeline.
fn unwrap_soap_body(raw: &[u8]) -> Result<Vec<u8>, ReceiverError> {
    let open = b"<soap:Body>";
    let close = b"</soap:Body>";
    let start = raw
        .windows(open.len())
        .position(|w| w == open)
        .ok_or_else(|| ReceiverError::Parse("missing <soap:Body> opening tag".to_owned()))?
        + open.len();
    let end = raw[start..]
        .windows(close.len())
        .position(|w| w == close)
        .ok_or_else(|| ReceiverError::Parse("missing </soap:Body> closing tag".to_owned()))?;
    Ok(raw[start..start + end].to_vec())
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_transmit_peppol_native_as4_receive::crate_name(),
///     "invoicekit-transmit-peppol-native-as4-receive"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-peppol-native-as4-receive"
}

// ----- test scaffolding ------------------------------------------

/// Mock listener that returns each queued envelope once.
pub struct MockListener {
    queued: std::sync::Mutex<Vec<Result<InboundEnvelope, ListenerError>>>,
}

impl MockListener {
    /// Build with a FIFO queue of inbound envelopes.
    #[must_use]
    pub fn new(envelopes: Vec<Result<InboundEnvelope, ListenerError>>) -> Self {
        Self {
            queued: std::sync::Mutex::new(envelopes.into_iter().rev().collect()),
        }
    }
}

impl Listener for MockListener {
    fn accept(
        &self,
    ) -> Pin<
        Box<dyn std::future::Future<Output = Result<InboundEnvelope, ListenerError>> + Send + '_>,
    > {
        let next = self
            .queued
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| Err(ListenerError::Closed("no queued envelope".to_owned())));
        Box::pin(async move { next })
    }
}

/// Mock verifier — accepts every envelope by default; configure
/// `accept_all = false` to reject every envelope.
#[derive(Clone, Debug)]
pub struct MockVerifier {
    /// When false, every `verify()` call returns
    /// [`VerifierError::SignatureRejected`].
    pub accept_all: bool,
}

impl MockVerifier {
    /// Build a verifier that accepts every envelope.
    #[must_use]
    pub const fn accepting() -> Self {
        Self { accept_all: true }
    }

    /// Build a verifier that rejects every envelope.
    #[must_use]
    pub const fn rejecting() -> Self {
        Self { accept_all: false }
    }
}

impl Verifier for MockVerifier {
    fn verify(&self, _envelope: &InboundEnvelope) -> Result<(), VerifierError> {
        if self.accept_all {
            Ok(())
        } else {
            Err(VerifierError::SignatureRejected(
                "mock verifier configured to reject".to_owned(),
            ))
        }
    }
}

/// Mock dispatcher — returns a fixed receipt MDN; records every
/// payload it received.
pub struct MockDispatcher {
    receipt: Vec<u8>,
    received: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl MockDispatcher {
    /// Build with the fixed receipt MDN to emit.
    #[must_use]
    pub fn new(receipt: Vec<u8>) -> Self {
        Self {
            receipt,
            received: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of payloads received so far.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    #[must_use]
    pub fn received(&self) -> Vec<Vec<u8>> {
        self.received.lock().unwrap().clone()
    }
}

impl Dispatcher for MockDispatcher {
    fn dispatch(
        &self,
        _envelope: &InboundEnvelope,
        unwrapped_xml: &[u8],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, DispatcherError>> + Send + '_>>
    {
        self.received.lock().unwrap().push(unwrapped_xml.to_vec());
        let response = self.receipt.clone();
        Box::pin(async move { Ok(response) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_envelope() -> InboundEnvelope {
        let raw = b"<soap:Envelope><soap:Header/><soap:Body><Invoice>x</Invoice></soap:Body></soap:Envelope>".to_vec();
        InboundEnvelope {
            raw_soap: raw,
            sender_participant: "iso6523-actorid-upis::DE0".to_owned(),
            recipient_participant: "iso6523-actorid-upis::DE1".to_owned(),
            message_id: "msg-42".to_owned(),
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(
            crate_name(),
            "invoicekit-transmit-peppol-native-as4-receive"
        );
    }

    #[test]
    fn unwrap_soap_body_extracts_payload() {
        let env = fixture_envelope();
        let body = unwrap_soap_body(&env.raw_soap).unwrap();
        assert_eq!(body, b"<Invoice>x</Invoice>");
    }

    #[test]
    fn unwrap_soap_body_rejects_missing_open_tag() {
        let err = unwrap_soap_body(b"<no-soap-body/>").unwrap_err();
        assert!(matches!(err, ReceiverError::Parse(_)));
    }

    #[test]
    fn unwrap_soap_body_rejects_missing_close_tag() {
        let err = unwrap_soap_body(b"<soap:Body>truncated").unwrap_err();
        assert!(matches!(err, ReceiverError::Parse(_)));
    }

    #[test]
    fn process_one_runs_full_pipeline() {
        let env = fixture_envelope();
        let listener = MockListener::new(vec![Ok(env)]);
        let verifier = MockVerifier::accepting();
        let dispatcher = MockDispatcher::new(b"<receipt/>".to_vec());
        let receiver = NativeAs4Receiver::new(
            Box::new(listener),
            Box::new(verifier),
            Box::new(MockDispatcherHolder(std::sync::Arc::new(dispatcher))),
        );
        let outcome = pollster::block_on(receiver.process_one()).unwrap();
        assert_eq!(outcome.ref_to_message_id, "msg-42");
        assert_eq!(outcome.receipt_mdn, b"<receipt/>");
    }

    #[test]
    fn process_one_surfaces_verifier_rejection() {
        let env = fixture_envelope();
        let receiver = NativeAs4Receiver::new(
            Box::new(MockListener::new(vec![Ok(env)])),
            Box::new(MockVerifier::rejecting()),
            Box::new(MockDispatcher::new(b"".to_vec())),
        );
        let err = pollster::block_on(receiver.process_one()).unwrap_err();
        assert!(matches!(err, ReceiverError::Verifier(_)));
    }

    #[test]
    fn process_one_surfaces_listener_closed() {
        let receiver = NativeAs4Receiver::new(
            Box::new(MockListener::new(vec![])),
            Box::new(MockVerifier::accepting()),
            Box::new(MockDispatcher::new(b"".to_vec())),
        );
        let err = pollster::block_on(receiver.process_one()).unwrap_err();
        assert!(matches!(err, ReceiverError::Listener(_)));
    }

    /// Tiny wrapper so the dispatcher can be exercised through
    /// `Arc` (the test needs to hold a copy to call `received()`
    /// after `process_one` consumes the dispatcher trait object).
    struct MockDispatcherHolder(std::sync::Arc<MockDispatcher>);

    impl Dispatcher for MockDispatcherHolder {
        fn dispatch(
            &self,
            envelope: &InboundEnvelope,
            unwrapped_xml: &[u8],
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, DispatcherError>> + Send + '_>>
        {
            self.0.dispatch(envelope, unwrapped_xml)
        }
    }
}
