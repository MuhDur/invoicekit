// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-signer` — signing substrate.
//!
//! Exposes the [`Signer`] trait the engine calls into when it
//! needs a payload signed. Backends:
//!
//! * [`SoftwareSigner`] — keyed BLAKE3 MAC for non-regulated
//!   cases (it's a placeholder substrate so the daemon RPC and
//!   the engine call sites can land + ship; real crypto
//!   providers land under T-083a for software RSA/ECDSA and
//!   T-083b for HSM/PKCS#11).
//! * [`MockSigner`] — records every call; used by tests and by
//!   the cassette-replay sandbox.
//!
//! The on-host `services/invoicekit-signer-agent` daemon
//! exposes the same [`Signer`] surface over a local Unix
//! socket so customer keys never leave the host process.
//!
//! Sign request / response shapes are stable JSON so language
//! bindings can speak the daemon protocol without linking the
//! Rust crate.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(unix)]
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

/// Opaque, operator-facing reference into the signer's keyring.
/// The signer-agent resolves this to the underlying key
/// material (file path, HSM slot, KMS key id, ...).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyRef(pub String);

impl KeyRef {
    /// Build a new key ref.
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

/// Canonical detached signature, base64-encoded payload bytes,
/// plus the [`KeyRef`] and algorithm id that produced it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct Signature {
    /// Key that signed this payload.
    pub key_ref: KeyRef,
    /// Algorithm identifier (e.g. `blake3-keyed-256`,
    /// `ed25519`, `rsassa-pss-sha256`).
    pub algorithm: String,
    /// Detached signature bytes, base64-encoded (no line breaks,
    /// no `=` padding stripping — RFC 4648 §4).
    pub signature_b64: String,
}

/// Sign-request input. Mirrors the JSON-RPC body the daemon
/// accepts at `POST /sign`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignRequest {
    /// Bytes to sign.
    pub payload: Vec<u8>,
    /// Key reference.
    pub key_ref: KeyRef,
}

/// Errors raised by [`Signer`] implementations.
#[derive(Debug, Error)]
pub enum SigningError {
    /// The key reference is not registered with this signer.
    #[error("unknown key reference: {0}")]
    UnknownKey(String),
    /// The signer's backing keystore is unavailable (file
    /// missing, HSM disconnected, daemon down).
    #[error("signer backend unavailable: {0}")]
    Unavailable(String),
    /// The signer's runtime refused the request (rate-limited,
    /// policy violation, audit-log full).
    #[error("signer refused: {0}")]
    Refused(String),
}

/// Signing surface.
///
/// Synchronous because real backends are either in-process
/// software (which is synchronous anyway) or the local
/// signer-agent daemon (which speaks over Unix sockets at
/// sub-millisecond latency).
pub trait Signer: Send + Sync {
    /// Sign `request.payload` with `request.key_ref`.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] when the key is unknown, the
    /// backend is unavailable, or the runtime refuses.
    fn sign(&self, request: &SignRequest) -> Result<Signature, SigningError>;

    /// List the key refs this signer can serve. Used by the
    /// engine's pre-flight check + the signer-agent's `list`
    /// RPC.
    fn list_keys(&self) -> Vec<KeyRef>;
}

/// In-process software signer.
///
/// Uses BLAKE3's keyed-hash mode as a placeholder MAC so the
/// surface, the daemon, and the engine call sites are all
/// exercised end-to-end while the real RSA/ECDSA software
/// signer (T-083a) and the HSM/PKCS#11 signer (T-083b) ship in
/// follow-up beads.
pub struct SoftwareSigner {
    /// `key_ref` -> 32-byte BLAKE3 key material.
    keys: BTreeMap<KeyRef, [u8; 32]>,
}

impl SoftwareSigner {
    /// Build an empty signer; load keys via [`SoftwareSigner::with_key`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            keys: BTreeMap::new(),
        }
    }

    /// Add a key. The key bytes are 32-byte BLAKE3 keying material.
    #[must_use]
    pub fn with_key(mut self, key_ref: impl Into<KeyRef>, key_bytes: [u8; 32]) -> Self {
        self.keys.insert(key_ref.into(), key_bytes);
        self
    }
}

impl Default for SoftwareSigner {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for KeyRef {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for KeyRef {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Signer for SoftwareSigner {
    fn sign(&self, request: &SignRequest) -> Result<Signature, SigningError> {
        let key = self
            .keys
            .get(&request.key_ref)
            .ok_or_else(|| SigningError::UnknownKey(request.key_ref.as_str().to_owned()))?;
        let mac = blake3::keyed_hash(key, &request.payload);
        Ok(Signature {
            key_ref: request.key_ref.clone(),
            algorithm: "blake3-keyed-256".to_owned(),
            signature_b64: base64_encode(mac.as_bytes()),
        })
    }

    fn list_keys(&self) -> Vec<KeyRef> {
        self.keys.keys().cloned().collect()
    }
}

/// Mock signer for tests + cassette-replay. Records every call;
/// returns a deterministic signature derived from the payload
/// hash so tests can assert on the exact bytes.
pub struct MockSigner {
    calls: Mutex<Vec<SignRequest>>,
    known_keys: Vec<KeyRef>,
}

impl MockSigner {
    /// Build a mock signer that knows the listed keys.
    #[must_use]
    pub fn new(known_keys: Vec<KeyRef>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            known_keys,
        }
    }

    /// Snapshot of every recorded `sign` request.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn calls(&self) -> Vec<SignRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl Signer for MockSigner {
    fn sign(&self, request: &SignRequest) -> Result<Signature, SigningError> {
        if !self.known_keys.contains(&request.key_ref) {
            return Err(SigningError::UnknownKey(
                request.key_ref.as_str().to_owned(),
            ));
        }
        self.calls.lock().unwrap().push(request.clone());
        let digest = blake3::hash(&request.payload);
        Ok(Signature {
            key_ref: request.key_ref.clone(),
            algorithm: "mock-blake3-256".to_owned(),
            signature_b64: base64_encode(digest.as_bytes()),
        })
    }

    fn list_keys(&self) -> Vec<KeyRef> {
        self.known_keys.clone()
    }
}

/// Signer implementation that calls `invoicekit-signer-agent`
/// over a local Unix socket.
///
/// This is the engine-side half of the signer-agent boundary:
/// production code can depend on the [`Signer`] trait and swap
/// between in-process [`SoftwareSigner`] and this local proxy
/// without changing call sites.
#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnixSocketSigner {
    socket_path: PathBuf,
}

#[cfg(unix)]
impl UnixSocketSigner {
    /// Build a signer-agent client for `socket_path`.
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Borrow the configured signer-agent socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Fallible key listing for engine pre-flight checks.
    ///
    /// [`Signer::list_keys`] cannot return errors, so it maps
    /// failures to an empty list. Engine setup code should use
    /// this method when it needs to distinguish "empty
    /// keyring" from "daemon unavailable".
    ///
    /// # Errors
    ///
    /// Returns [`SigningError`] when the socket is unavailable,
    /// the daemon refuses the request, or the response shape is
    /// not the expected `{ "keys": [...] }`.
    pub fn try_list_keys(&self) -> Result<Vec<KeyRef>, SigningError> {
        Self::parse_list_keys_response(&self.rpc("list_keys", &serde_json::json!({}))?)
    }

    fn rpc(
        &self,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, SigningError> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|err| {
            SigningError::Unavailable(format!("{}: {err}", self.socket_path.display()))
        })?;
        let body = serde_json::json!({ "method": method, "params": params });
        let body =
            serde_json::to_string(&body).map_err(|err| SigningError::Refused(err.to_string()))?;
        writeln!(stream, "{body}").map_err(|err| SigningError::Unavailable(err.to_string()))?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .map_err(|err| SigningError::Unavailable(err.to_string()))?;
        let value: serde_json::Value = serde_json::from_str(response.trim())
            .map_err(|err| SigningError::Refused(format!("malformed daemon response: {err}")))?;
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            return Err(Self::map_daemon_error(error));
        }
        Ok(value)
    }

    fn sign_params(request: &SignRequest) -> serde_json::Value {
        serde_json::json!({
            "key_ref": request.key_ref.as_str(),
            "payload_b64": base64_encode(&request.payload),
        })
    }

    fn parse_list_keys_response(value: &serde_json::Value) -> Result<Vec<KeyRef>, SigningError> {
        let keys = value
            .get("keys")
            .and_then(|keys| keys.as_array())
            .ok_or_else(|| SigningError::Refused("list_keys response missing keys".to_owned()))?;
        keys.iter()
            .map(|key| {
                key.as_str().map(KeyRef::new).ok_or_else(|| {
                    SigningError::Refused("list_keys key is not a string".to_owned())
                })
            })
            .collect()
    }

    fn map_daemon_error(error: &str) -> SigningError {
        error.strip_prefix("unknown key reference: ").map_or_else(
            || SigningError::Refused(error.to_owned()),
            |key| SigningError::UnknownKey(key.to_owned()),
        )
    }
}

#[cfg(unix)]
impl Signer for UnixSocketSigner {
    fn sign(&self, request: &SignRequest) -> Result<Signature, SigningError> {
        let params = Self::sign_params(request);
        let value = self.rpc("sign", &params)?;
        serde_json::from_value(value)
            .map_err(|err| SigningError::Refused(format!("malformed signature response: {err}")))
    }

    fn list_keys(&self) -> Vec<KeyRef> {
        self.try_list_keys().unwrap_or_default()
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_signer::crate_name(), "invoicekit-signer");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer"
}

/// RFC 4648 §4 base64 encoder. Inlined to avoid pulling a new
/// dependency for a one-shot encode.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char);
        out.push(ALPHABET[(b2 & 0b11_1111) as usize] as char);
        i += 3;
    }
    let remaining = bytes.len() - i;
    if remaining == 1 {
        let b0 = bytes[i];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[((b0 & 0b11) << 4) as usize] as char);
        out.push('=');
        out.push('=');
    } else if remaining == 2 {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(ALPHABET[((b1 & 0b1111) << 2) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer");
    }

    #[test]
    fn software_signer_signs_known_key() {
        let signer = SoftwareSigner::new().with_key("tenant-a/seal", [7_u8; 32]);
        let req = SignRequest {
            payload: b"hello".to_vec(),
            key_ref: KeyRef::new("tenant-a/seal"),
        };
        let sig = signer.sign(&req).unwrap();
        assert_eq!(sig.algorithm, "blake3-keyed-256");
        assert_eq!(sig.key_ref.as_str(), "tenant-a/seal");
        assert!(!sig.signature_b64.is_empty());
    }

    #[test]
    fn software_signer_is_deterministic() {
        let signer = SoftwareSigner::new().with_key("k", [3_u8; 32]);
        let req = SignRequest {
            payload: b"deterministic".to_vec(),
            key_ref: KeyRef::new("k"),
        };
        let a = signer.sign(&req).unwrap();
        let b = signer.sign(&req).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn software_signer_rejects_unknown_key() {
        let signer = SoftwareSigner::new();
        let req = SignRequest {
            payload: b"".to_vec(),
            key_ref: KeyRef::new("missing"),
        };
        let err = signer.sign(&req).unwrap_err();
        assert!(matches!(err, SigningError::UnknownKey(ref k) if k == "missing"));
    }

    #[test]
    fn software_signer_list_keys_is_lexicographic() {
        let signer = SoftwareSigner::new()
            .with_key("zeta", [0; 32])
            .with_key("alpha", [0; 32])
            .with_key("mu", [0; 32]);
        let keys: Vec<String> = signer.list_keys().into_iter().map(|k| k.0).collect();
        assert_eq!(keys, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn mock_signer_records_calls_and_rejects_unknown() {
        let signer = MockSigner::new(vec![KeyRef::new("test")]);
        let ok = signer
            .sign(&SignRequest {
                payload: b"x".to_vec(),
                key_ref: KeyRef::new("test"),
            })
            .unwrap();
        assert_eq!(ok.algorithm, "mock-blake3-256");
        assert_eq!(signer.calls().len(), 1);

        let err = signer
            .sign(&SignRequest {
                payload: b"y".to_vec(),
                key_ref: KeyRef::new("other"),
            })
            .unwrap_err();
        assert!(matches!(err, SigningError::UnknownKey(_)));
    }

    #[test]
    fn signature_round_trips_through_json() {
        let sig = Signature {
            key_ref: KeyRef::new("test"),
            algorithm: "blake3-keyed-256".to_owned(),
            signature_b64: "AAA=".to_owned(),
        };
        let json = serde_json::to_string(&sig).unwrap();
        let back: Signature = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sig);
    }

    #[test]
    fn base64_encode_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_signer_builds_agent_sign_request() {
        let request = SignRequest {
            payload: b"hello".to_vec(),
            key_ref: KeyRef::new("tenant-a/seal"),
        };
        let params = UnixSocketSigner::sign_params(&request);
        assert_eq!(
            params.get("key_ref").and_then(|v| v.as_str()),
            Some("tenant-a/seal")
        );
        assert_eq!(
            params.get("payload_b64").and_then(|v| v.as_str()),
            Some("aGVsbG8=")
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_signer_parses_list_keys_response() {
        let response = serde_json::json!({
            "keys": ["alpha", "omega"]
        });
        let keys = UnixSocketSigner::parse_list_keys_response(&response).unwrap();
        assert_eq!(keys, vec![KeyRef::new("alpha"), KeyRef::new("omega")]);
    }
}
