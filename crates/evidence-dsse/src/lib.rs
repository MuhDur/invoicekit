// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! DSSE (Dead Simple Signing Envelope) wrapper for InvoiceKit
//! evidence-bundle manifests.
//!
//! Closes the bead-8h6g gap noted in the T-080 review: the
//! packed `.ikb` bundle's content-address ledger is intact,
//! but the bundle has no formal envelope binding a detached
//! signature to `manifest.json`. DSSE — the standard
//! published at <https://dsse.dev/spec/v1.0> — is the format
//! the broader supply-chain ecosystem (in-toto, sigstore, slsa)
//! already settled on. Adopting it instead of inventing our
//! own envelope means InvoiceKit evidence bundles drop into
//! those ecosystems without translation.
//!
//! Wire format (canonical JSON, lexicographically-sorted keys):
//!
//! ```json
//! {
//!   "payload": "<base64(manifest.json bytes)>",
//!   "payloadType": "application/vnd.invoicekit.manifest+json",
//!   "signatures": [
//!     { "keyid": "<opaque key id>", "sig": "<base64(signature over PAE)>" }
//!   ]
//! }
//! ```
//!
//! The signature is computed over the **PAE** (Pre-Authentication
//! Encoding) of `(payload_type, payload)`, not over the raw
//! payload bytes. This binds the payload type into the
//! signature so attackers cannot swap the `payloadType` to
//! something the verifier interprets differently.
//!
//! Public surface:
//! [`DsseEnvelope`], [`DsseSignature`], [`pae`], [`wrap`],
//! [`wrap_manifest`], [`attach_manifest_dsse`],
//! [`verify_envelope`], [`ManifestSigner`], [`MockSigner`],
//! [`MANIFEST_PAYLOAD_TYPE`], [`MANIFEST_SIGNATURE_ARTEFACT_ID`].

use base64::Engine;
use invoicekit_evidence::EvidenceBundle;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Payload type registered for InvoiceKit manifests inside a DSSE envelope.
///
/// Operators MAY choose a different value if they re-use the
/// envelope for a different artefact, but the engine pins
/// this one for `manifest.json`.
pub const MANIFEST_PAYLOAD_TYPE: &str = "application/vnd.invoicekit.manifest+json";

/// Reserved artefact id for the DSSE envelope inside the
/// evidence bundle. `invoicekit verify` looks for this id when
/// the operator opts into the signature check.
pub use invoicekit_evidence::MANIFEST_SIGNATURE_ARTEFACT_ID;

/// One signature in a DSSE envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DsseSignature {
    /// Opaque key identifier. The signer chooses the format
    /// (a JWK thumbprint, an X.509 SKI, an HSM slot label).
    pub keyid: String,
    /// Signature bytes, base64-encoded per DSSE spec.
    pub sig: String,
}

/// DSSE envelope. Carries one payload and one or more
/// detached signatures over the PAE of `(payload_type,
/// payload)`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DsseEnvelope {
    /// Base64-encoded payload bytes (the raw `manifest.json`
    /// bytes when used for InvoiceKit bundles).
    pub payload: String,
    /// IANA-style media type describing the payload.
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    /// Signatures over the PAE. Threshold-style verifiers can
    /// require N-of-M.
    pub signatures: Vec<DsseSignature>,
}

impl DsseEnvelope {
    /// Decode the base64-encoded payload back to raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DsseError::BadBase64`] when the payload field
    /// isn't valid standard base64.
    pub fn decoded_payload(&self) -> Result<Vec<u8>, DsseError> {
        base64::engine::general_purpose::STANDARD
            .decode(self.payload.as_bytes())
            .map_err(|e| DsseError::BadBase64(e.to_string()))
    }
}

/// Errors raised by this crate.
#[derive(Debug, Error)]
pub enum DsseError {
    /// A base64 field could not be decoded.
    #[error("base64 decode failure: {0}")]
    BadBase64(String),
    /// The bundle manifest could not be serialised to
    /// canonical JSON bytes for signing.
    #[error("manifest serialise failure: {0}")]
    ManifestSerialize(String),
    /// The envelope carried zero signatures (the spec allows
    /// it but every verifier treats it as a failure).
    #[error("envelope carries no signatures")]
    NoSignatures,
    /// The signer / verifier emitted an internal error
    /// (transport, HSM, key derivation).
    #[error("signer/verifier error: {0}")]
    Signer(String),
    /// No signature in the envelope matched the expected key
    /// id.
    #[error("no signature found for keyid {0:?}")]
    UnknownKey(String),
    /// The signature bytes didn't validate against the PAE.
    #[error("signature did not verify for keyid {0:?}")]
    BadSignature(String),
    /// The envelope's `payload_type` field didn't match the
    /// expected value (e.g. the bundle's manifest envelope
    /// was re-purposed to carry a different artefact type).
    #[error("payload type drift: expected {expected:?}, got {got:?}")]
    PayloadTypeDrift {
        /// Expected media type.
        expected: String,
        /// Observed media type.
        got: String,
    },
    /// The envelope's decoded payload bytes didn't match the
    /// re-computed manifest bytes — i.e. the envelope was
    /// produced over a different manifest than the verifier
    /// re-hashed.
    #[error("payload drift: envelope payload differs from re-computed manifest")]
    PayloadDrift,
}

/// Surface a real or mock signer implements.
///
/// The same trait shape is what `crates/signer` exposes; this
/// crate-local definition keeps the DSSE crate
/// dependency-free from the wider signer stack so it can ship
/// before the signer-agent IPC contract is finalised.
pub trait ManifestSigner: Send + Sync {
    /// The signer's opaque key identifier (returned verbatim
    /// in the [`DsseSignature::keyid`] field).
    fn keyid(&self) -> &str;

    /// Sign the supplied PAE bytes and return the raw
    /// signature.
    ///
    /// # Errors
    ///
    /// Returns [`DsseError::Signer`] when the underlying
    /// signer rejects the request.
    fn sign_pae(&self, pae_bytes: &[u8]) -> Result<Vec<u8>, DsseError>;

    /// Verify that `sig_bytes` is a valid signature over
    /// `pae_bytes` for the key the signer holds.
    ///
    /// # Errors
    ///
    /// Returns [`DsseError::Signer`] when the verifier errors
    /// internally (transport, malformed key). Returns
    /// [`DsseError::BadSignature`] when the signature simply
    /// does not validate.
    fn verify_pae(&self, pae_bytes: &[u8], sig_bytes: &[u8]) -> Result<(), DsseError>;
}

/// Compute the DSSE Pre-Authentication Encoding for a payload
/// and payload type.
///
/// Per [dsse.dev/spec/v1.0](https://dsse.dev/spec/v1.0):
///
/// ```text
///   PAE = "DSSEv1" SP LEN(type) SP type SP LEN(payload) SP payload
/// ```
///
/// where SP is ASCII space and LEN(x) is the decimal byte
/// length of `x`.
#[must_use]
pub fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + payload_type.len() + payload.len());
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(payload_type.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload_type.as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload);
    out
}

/// Wrap a payload in a DSSE envelope using a single
/// [`ManifestSigner`].
///
/// `payload_type` should be [`MANIFEST_PAYLOAD_TYPE`] for the
/// InvoiceKit `manifest.json` case; the helper accepts an
/// override so the same crate is reusable for other
/// artefacts (e.g. provenance attestations).
///
/// # Errors
///
/// Returns [`DsseError`] when the signer fails to sign the
/// PAE bytes.
pub fn wrap(
    signer: &dyn ManifestSigner,
    payload_type: &str,
    payload: &[u8],
) -> Result<DsseEnvelope, DsseError> {
    let pae_bytes = pae(payload_type, payload);
    let sig_bytes = signer.sign_pae(&pae_bytes)?;
    Ok(DsseEnvelope {
        payload: base64::engine::general_purpose::STANDARD.encode(payload),
        payload_type: payload_type.to_owned(),
        signatures: vec![DsseSignature {
            keyid: signer.keyid().to_owned(),
            sig: base64::engine::general_purpose::STANDARD.encode(sig_bytes),
        }],
    })
}

/// Wrap a bundle's canonical `manifest.json` bytes in a DSSE
/// envelope.
///
/// The manifest bytes are exactly the bytes that
/// `invoicekit-evidence` writes at [`invoicekit_evidence::MANIFEST_ARTEFACT_ID`]:
/// compact JSON serialisation of [`invoicekit_evidence::Manifest`].
///
/// # Errors
///
/// Returns [`DsseError`] when the manifest fails to serialise
/// or the signer fails.
pub fn wrap_manifest(
    bundle: &EvidenceBundle,
    signer: &dyn ManifestSigner,
) -> Result<DsseEnvelope, DsseError> {
    let payload = manifest_payload_bytes(bundle)?;
    wrap(signer, MANIFEST_PAYLOAD_TYPE, &payload)
}

/// Add or replace the reserved `signatures/manifest.dsse`
/// sidecar artefact on a bundle.
///
/// The manifest is intentionally left unchanged. The DSSE
/// envelope signs the manifest, so listing the envelope inside
/// that manifest would make the signed payload self-referential.
/// The evidence codec allows this single reserved sidecar, and
/// `invoicekit-verify` validates it explicitly.
///
/// # Errors
///
/// Returns [`DsseError`] when the manifest fails to serialise,
/// the signer fails, or the envelope cannot be encoded as JSON.
pub fn attach_manifest_dsse(
    bundle: &EvidenceBundle,
    signer: &dyn ManifestSigner,
) -> Result<EvidenceBundle, DsseError> {
    let envelope = wrap_manifest(bundle, signer)?;
    let bytes =
        serde_json::to_vec(&envelope).map_err(|e| DsseError::ManifestSerialize(e.to_string()))?;
    let mut next = bundle.clone();
    next.artefacts
        .insert(MANIFEST_SIGNATURE_ARTEFACT_ID.to_owned(), bytes);
    Ok(next)
}

fn manifest_payload_bytes(bundle: &EvidenceBundle) -> Result<Vec<u8>, DsseError> {
    serde_json::to_vec(&bundle.manifest).map_err(|e| DsseError::ManifestSerialize(e.to_string()))
}

/// Verify a DSSE envelope against the expected payload bytes
/// and payload type.
///
/// The function:
///
/// 1. confirms the envelope carries at least one signature
///    matching the supplied signer's `keyid`,
/// 2. recomputes the PAE from `(payload_type, expected_payload)`,
/// 3. asks the signer to re-validate the signature.
///
/// `expected_payload` must be the same bytes the verifier
/// freshly computed — the function compares the envelope's
/// decoded payload byte-for-byte against it so a tampered
/// envelope is rejected before the cryptographic check runs.
///
/// # Errors
///
/// Returns [`DsseError`] with a specific variant explaining
/// which check failed:
/// [`DsseError::NoSignatures`], [`DsseError::UnknownKey`],
/// [`DsseError::PayloadTypeDrift`], [`DsseError::PayloadDrift`],
/// [`DsseError::BadSignature`], [`DsseError::BadBase64`],
/// [`DsseError::Signer`].
pub fn verify_envelope(
    envelope: &DsseEnvelope,
    expected_payload_type: &str,
    expected_payload: &[u8],
    signer: &dyn ManifestSigner,
) -> Result<(), DsseError> {
    if envelope.signatures.is_empty() {
        return Err(DsseError::NoSignatures);
    }
    if envelope.payload_type != expected_payload_type {
        return Err(DsseError::PayloadTypeDrift {
            expected: expected_payload_type.to_owned(),
            got: envelope.payload_type.clone(),
        });
    }
    let observed_payload = envelope.decoded_payload()?;
    if observed_payload != expected_payload {
        return Err(DsseError::PayloadDrift);
    }
    let keyid = signer.keyid();
    let signature = envelope
        .signatures
        .iter()
        .find(|s| s.keyid == keyid)
        .ok_or_else(|| DsseError::UnknownKey(keyid.to_owned()))?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature.sig.as_bytes())
        .map_err(|e| DsseError::BadBase64(e.to_string()))?;
    let pae_bytes = pae(expected_payload_type, expected_payload);
    signer.verify_pae(&pae_bytes, &sig_bytes)
}

/// Deterministic mock signer for tests / cassette replay.
///
/// "Signs" by emitting a deterministic digest of the PAE
/// prefixed with `b"mock-dsse:"`. Verification re-computes
/// the same value and compares. The mock is intentionally not
/// cryptographically meaningful — its only purpose is to let
/// the rest of the InvoiceKit pipeline exercise the DSSE
/// shape end to end before real signer-agent / HSM wiring
/// lands.
pub struct MockSigner {
    keyid: String,
}

impl MockSigner {
    /// Build a mock signer with the supplied opaque key id.
    #[must_use]
    pub fn new(keyid: impl Into<String>) -> Self {
        Self {
            keyid: keyid.into(),
        }
    }
}

impl Default for MockSigner {
    fn default() -> Self {
        Self::new("mock-dsse-key")
    }
}

impl ManifestSigner for MockSigner {
    fn keyid(&self) -> &str {
        &self.keyid
    }
    fn sign_pae(&self, pae_bytes: &[u8]) -> Result<Vec<u8>, DsseError> {
        let mut out = b"mock-dsse:".to_vec();
        let hash = mock_digest(pae_bytes);
        out.extend_from_slice(&hash);
        Ok(out)
    }
    fn verify_pae(&self, pae_bytes: &[u8], sig_bytes: &[u8]) -> Result<(), DsseError> {
        let mut expected = b"mock-dsse:".to_vec();
        expected.extend_from_slice(&mock_digest(pae_bytes));
        if expected == sig_bytes {
            Ok(())
        } else {
            Err(DsseError::BadSignature(self.keyid.clone()))
        }
    }
}

/// Tiny FNV-1a-derived 32-byte digest used only by
/// [`MockSigner`]. Not BLAKE3, not cryptographic — the mock
/// only needs deterministic output. Real signers compute
/// their own hashes inside their own crate so this path is
/// production-unreachable.
#[allow(clippy::cast_possible_truncation)]
fn mock_digest(bytes: &[u8]) -> [u8; 32] {
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut out = [0_u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        let rotation = (u32::try_from(i).unwrap_or(0)).wrapping_mul(7) & 63;
        let r = state.rotate_left(rotation);
        // Truncation is intentional — we want one byte per
        // output slot.
        *slot = (r ^ (r >> 8)) as u8;
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_evidence_dsse::crate_name(),
///     "invoicekit-evidence-dsse"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-evidence-dsse"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack, unpack};
    use std::collections::BTreeMap;

    fn sample_bundle() -> EvidenceBundle {
        let mut artefacts = BTreeMap::new();
        artefacts.insert(
            "canonical.json".to_owned(),
            br#"{"id":"INV-DSSE-1"}"#.to_vec(),
        );
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(
            &artefacts,
            "tenant-dsse",
            "trace-dsse",
            "2026-05-28T00:00:00Z",
        );
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    #[test]
    fn pae_matches_dsse_v1_spec_example() {
        // From dsse.dev/spec/v1.0 §2.1 worked example:
        //   payloadType = "http://example.com/HelloWorld"
        //   payload     = "hello world"
        // PAE should be:
        //   "DSSEv1 29 http://example.com/HelloWorld 11 hello world"
        let got = pae("http://example.com/HelloWorld", b"hello world");
        let want = b"DSSEv1 29 http://example.com/HelloWorld 11 hello world";
        assert_eq!(got, want);
    }

    #[test]
    fn pae_handles_empty_payload() {
        let got = pae("text/plain", b"");
        assert_eq!(got, b"DSSEv1 10 text/plain 0 ");
    }

    #[test]
    fn pae_handles_binary_payload() {
        let got = pae("application/octet-stream", &[0_u8, 1, 2, 0xff]);
        // Length prefix is 4 bytes; payload bytes follow raw.
        let mut want = b"DSSEv1 24 application/octet-stream 4 ".to_vec();
        want.extend_from_slice(&[0_u8, 1, 2, 0xff]);
        assert_eq!(got, want);
    }

    #[test]
    fn wrap_round_trips_with_default_mock_signer() {
        let signer = MockSigner::default();
        let payload = b"{\"manifest\":\"v1\"}";
        let env = wrap(&signer, MANIFEST_PAYLOAD_TYPE, payload).unwrap();
        assert_eq!(env.payload_type, MANIFEST_PAYLOAD_TYPE);
        assert_eq!(env.signatures.len(), 1);
        assert_eq!(env.signatures[0].keyid, "mock-dsse-key");
        verify_envelope(&env, MANIFEST_PAYLOAD_TYPE, payload, &signer).unwrap();
    }

    #[test]
    fn attach_manifest_dsse_adds_reserved_sidecar_without_changing_manifest() {
        let bundle = sample_bundle();
        let signer = MockSigner::default();
        let attached = attach_manifest_dsse(&bundle, &signer).unwrap();

        assert_eq!(attached.manifest, bundle.manifest);
        assert!(attached
            .artefacts
            .contains_key(MANIFEST_SIGNATURE_ARTEFACT_ID));
        assert!(!attached
            .manifest
            .artefacts
            .iter()
            .any(|a| a.id == MANIFEST_SIGNATURE_ARTEFACT_ID));

        let packed = pack(&attached).unwrap();
        let unpacked = unpack(&packed).unwrap();
        assert_eq!(unpacked, attached);
    }

    #[test]
    fn wrap_manifest_binds_to_manifest_json_bytes() {
        let bundle = sample_bundle();
        let signer = MockSigner::default();
        let envelope = wrap_manifest(&bundle, &signer).unwrap();
        let expected_payload = serde_json::to_vec(&bundle.manifest).unwrap();

        verify_envelope(&envelope, MANIFEST_PAYLOAD_TYPE, &expected_payload, &signer).unwrap();
    }

    #[test]
    fn verify_rejects_mutated_payload() {
        let signer = MockSigner::default();
        let env = wrap(&signer, MANIFEST_PAYLOAD_TYPE, b"original").unwrap();
        let err = verify_envelope(&env, MANIFEST_PAYLOAD_TYPE, b"different", &signer).unwrap_err();
        assert!(matches!(err, DsseError::PayloadDrift));
    }

    #[test]
    fn verify_rejects_mutated_payload_type() {
        let signer = MockSigner::default();
        let env = wrap(&signer, MANIFEST_PAYLOAD_TYPE, b"x").unwrap();
        let err = verify_envelope(&env, "wrong/type", b"x", &signer).unwrap_err();
        match err {
            DsseError::PayloadTypeDrift { expected, got } => {
                assert_eq!(expected, "wrong/type");
                assert_eq!(got, MANIFEST_PAYLOAD_TYPE);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn verify_rejects_envelope_with_no_signatures() {
        let env = DsseEnvelope {
            payload: base64::engine::general_purpose::STANDARD.encode(b"x"),
            payload_type: MANIFEST_PAYLOAD_TYPE.to_owned(),
            signatures: Vec::new(),
        };
        let err =
            verify_envelope(&env, MANIFEST_PAYLOAD_TYPE, b"x", &MockSigner::default()).unwrap_err();
        assert!(matches!(err, DsseError::NoSignatures));
    }

    #[test]
    fn verify_rejects_unknown_keyid() {
        let alice = MockSigner::new("alice");
        let bob = MockSigner::new("bob");
        let env = wrap(&alice, MANIFEST_PAYLOAD_TYPE, b"x").unwrap();
        let err = verify_envelope(&env, MANIFEST_PAYLOAD_TYPE, b"x", &bob).unwrap_err();
        match err {
            DsseError::UnknownKey(k) => assert_eq!(k, "bob"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let signer = MockSigner::default();
        let mut env = wrap(&signer, MANIFEST_PAYLOAD_TYPE, b"x").unwrap();
        // Flip a byte in the signature.
        let mut bad_sig = base64::engine::general_purpose::STANDARD
            .decode(env.signatures[0].sig.as_bytes())
            .unwrap();
        bad_sig[5] ^= 0xff;
        env.signatures[0].sig = base64::engine::general_purpose::STANDARD.encode(&bad_sig);
        let err = verify_envelope(&env, MANIFEST_PAYLOAD_TYPE, b"x", &signer).unwrap_err();
        assert!(matches!(err, DsseError::BadSignature(_)));
    }

    #[test]
    fn verify_rejects_bad_base64_payload() {
        let env = DsseEnvelope {
            payload: "not-base64!!".to_owned(),
            payload_type: MANIFEST_PAYLOAD_TYPE.to_owned(),
            signatures: vec![DsseSignature {
                keyid: "k".to_owned(),
                sig: "AA==".to_owned(),
            }],
        };
        let err = env.decoded_payload().unwrap_err();
        assert!(matches!(err, DsseError::BadBase64(_)));
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let signer = MockSigner::default();
        let env = wrap(&signer, MANIFEST_PAYLOAD_TYPE, b"hello").unwrap();
        let json = serde_json::to_string(&env).unwrap();
        // Sanity: JSON contains the spec-required keys.
        assert!(json.contains("\"payload\""));
        assert!(json.contains("\"payloadType\""));
        assert!(json.contains("\"signatures\""));
        let parsed: DsseEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn manifest_signature_artefact_id_is_under_signatures_prefix() {
        // Locks the canonical path so bundle writers + verify
        // both agree without round-tripping a constant rename.
        assert_eq!(MANIFEST_SIGNATURE_ARTEFACT_ID, "signatures/manifest.dsse");
    }
}
