// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-timestamping` — RFC 3161 timestamping substrate.
//!
//! Exposes the [`TimestampClient`] trait every InvoiceKit
//! timestamping path calls into. Backends:
//!
//! * [`MockTimestampClient`] — deterministic timestamp tokens
//!   bound to a fixed clock; used by tests and by the
//!   cassette-replay sandbox.
//! * Real TSA-over-HTTP client + ASN.1 token codec land behind
//!   a future `reqwest` feature flag (T-082a follow-up).
//!
//! Tokens are kept opaque (raw bytes) so the substrate stays
//! independent of the eventual ASN.1 dependency choice;
//! [`RfcTimestamp::token`] is what the evidence bundle archives
//! and what the `invoicekit verify` CLI (T-084) re-parses.
//!
//! # Why a trait, not a concrete impl
//!
//! Production deployments choose their TSA:
//! `GlobalSign` / `Sectigo` / `DigiCert` / Apple's free TSA / a
//! self-hosted `openssl ts` server. They all speak the same
//! RFC 3161 wire shape but with different URLs, auth, and
//! retry budgets. The trait lets the engine swap TSAs per
//! tenant without touching the call sites.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hash algorithm identifier used in the timestamp request.
/// Mirrors RFC 3161 §2.4 `messageImprint.hashAlgorithm`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// SHA-256 (the InvoiceKit default; widely supported by TSAs).
    #[serde(rename = "sha-256")]
    Sha256,
    /// SHA-384.
    #[serde(rename = "sha-384")]
    Sha384,
    /// SHA-512.
    #[serde(rename = "sha-512")]
    Sha512,
    /// BLAKE3 (not on the RFC 3161 OID list; the mock backend
    /// accepts it for the cassette-replay sandbox so the same
    /// payload hash the `invoicekit-evidence` bundle records
    /// can be timestamped in tests without re-hashing).
    #[serde(rename = "blake3")]
    Blake3,
}

impl HashAlgorithm {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Sha256 => "sha-256",
            Self::Sha384 => "sha-384",
            Self::Sha512 => "sha-512",
            Self::Blake3 => "blake3",
        }
    }
}

/// Timestamp request input.
///
/// Mirrors the JSON-RPC body the future signer-agent
/// `request_timestamp` method accepts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimestampRequest {
    /// Hash algorithm used to produce `message_imprint`.
    pub algorithm: HashAlgorithm,
    /// Raw hash bytes (32 for SHA-256, 48 for SHA-384, ...).
    pub message_imprint: Vec<u8>,
    /// Optional nonce echoed back in the token. Operators
    /// usually leave this `None`; the TSA generates one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    /// Whether the client requires the TSA to return its
    /// signing certificate in the response. Mirrors RFC 3161
    /// `certReq`; default `true` for evidence bundles so
    /// verification works offline.
    #[serde(default)]
    pub cert_req: bool,
}

/// One RFC 3161 timestamp token.
///
/// Carries the opaque DER-encoded `TimeStampToken` bytes (or,
/// in the mock backend, a JSON envelope) plus the parsed
/// fields the engine needs without re-parsing the token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RfcTimestamp {
    /// Raw DER-encoded `TimeStampToken`. The mock backend
    /// stores a deterministic placeholder; production TSAs
    /// store the real bytes.
    pub token: Vec<u8>,
    /// Hash algorithm the token attests to.
    pub algorithm: HashAlgorithm,
    /// Hash bytes the token attests to (echo of the request).
    pub message_imprint: Vec<u8>,
    /// `genTime` from the token (RFC 3339 UTC timestamp).
    pub generated_at: String,
    /// TSA's distinguished name as a UTF-8 string. The mock
    /// backend uses `mock-tsa`; production tokens carry the
    /// real DN.
    pub tsa_name: String,
    /// Token serial number (RFC 3161 `serialNumber` field;
    /// always positive integer encoded as decimal string).
    pub serial: String,
}

/// Errors raised by [`TimestampClient`] implementations.
#[derive(Debug, Error)]
pub enum TimestampingError {
    /// The hash imprint length did not match the declared
    /// algorithm (e.g. SHA-256 expects 32 bytes).
    #[error("hash imprint length mismatch for {algorithm}: expected {expected}, got {got}")]
    BadImprintLength {
        /// Declared algorithm.
        algorithm: &'static str,
        /// Expected byte length.
        expected: usize,
        /// Observed byte length.
        got: usize,
    },
    /// The TSA returned a status other than `granted`.
    #[error("TSA refused: {0}")]
    Refused(String),
    /// Transport-level failure (timeout, DNS, TLS).
    #[error("TSA transport failure: {0}")]
    Transport(String),
    /// The TSA's response could not be parsed.
    #[error("TSA response was not a valid TimeStampToken: {0}")]
    Malformed(String),
    /// `verify_timestamp` re-hashed the imprint and the token
    /// did not match.
    #[error("timestamp message imprint drifted")]
    ImprintDrift,
}

/// Timestamping surface.
///
/// Synchronous — real backends are HTTP-over-TLS to a TSA at
/// 100-300 ms latency; the signer-agent runs the request on a
/// dedicated thread so the engine doesn't need an async
/// runtime for the call path.
pub trait TimestampClient: Send + Sync {
    /// Request a timestamp token for `request.message_imprint`.
    ///
    /// # Errors
    ///
    /// Returns [`TimestampingError`] when the imprint shape is
    /// wrong, the TSA refuses, or the transport fails.
    fn request_timestamp(
        &self,
        request: &TimestampRequest,
    ) -> Result<RfcTimestamp, TimestampingError>;

    /// Verify a token against a freshly-computed imprint.
    /// Used by the `invoicekit verify` CLI (T-084) to confirm
    /// the bundle hasn't drifted since the timestamp was
    /// issued.
    ///
    /// # Errors
    ///
    /// Returns [`TimestampingError::ImprintDrift`] when the
    /// recomputed imprint disagrees with the token.
    fn verify_timestamp(
        &self,
        timestamp: &RfcTimestamp,
        recomputed_imprint: &[u8],
    ) -> Result<(), TimestampingError>;
}

/// Mock TSA client.
///
/// Returns deterministic timestamp tokens bound to a fixed
/// `genTime` so cassette-replay tests produce byte-identical
/// evidence bundles across runs.
pub struct MockTimestampClient {
    fixed_time: String,
    tsa_name: String,
    next_serial: std::sync::Mutex<u64>,
}

impl MockTimestampClient {
    /// Build a mock client that pins `genTime` to
    /// `2026-01-01T00:00:00Z` and the TSA name to `mock-tsa`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_time("2026-01-01T00:00:00Z", "mock-tsa")
    }

    /// Build a mock with a custom fixed `genTime` + TSA name.
    #[must_use]
    pub fn with_fixed_time(fixed_time: impl Into<String>, tsa_name: impl Into<String>) -> Self {
        Self {
            fixed_time: fixed_time.into(),
            tsa_name: tsa_name.into(),
            next_serial: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockTimestampClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TimestampClient for MockTimestampClient {
    fn request_timestamp(
        &self,
        request: &TimestampRequest,
    ) -> Result<RfcTimestamp, TimestampingError> {
        validate_imprint_length(request.algorithm, request.message_imprint.len())?;
        // Increment serial deterministically per client.
        let mut serial_guard = self.next_serial.lock().expect("serial mutex poisoned");
        let serial = *serial_guard;
        *serial_guard += 1;
        drop(serial_guard);
        // The "token" is just a deterministic `serde_json`
        // envelope of the request fields (it embeds the imprint
        // bytes verbatim — it is not hashed or content-addressed);
        // the real ASN.1 codec lands in a follow-up.
        let envelope = serde_json::json!({
            "tsa_name": self.tsa_name,
            "generated_at": self.fixed_time,
            "algorithm": request.algorithm.slug(),
            "message_imprint": request.message_imprint,
            "serial": serial,
            "nonce": request.nonce,
        });
        let token = serde_json::to_vec(&envelope)
            .map_err(|e| TimestampingError::Malformed(e.to_string()))?;
        Ok(RfcTimestamp {
            token,
            algorithm: request.algorithm,
            message_imprint: request.message_imprint.clone(),
            generated_at: self.fixed_time.clone(),
            tsa_name: self.tsa_name.clone(),
            serial: serial.to_string(),
        })
    }

    fn verify_timestamp(
        &self,
        timestamp: &RfcTimestamp,
        recomputed_imprint: &[u8],
    ) -> Result<(), TimestampingError> {
        if timestamp.message_imprint != recomputed_imprint {
            return Err(TimestampingError::ImprintDrift);
        }
        Ok(())
    }
}

/// Validate the imprint byte length against the declared
/// algorithm. Exposed so callers can pre-flight the request
/// before going to the wire.
///
/// # Errors
///
/// Returns [`TimestampingError::BadImprintLength`] when the
/// observed length is wrong for the declared algorithm.
pub fn validate_imprint_length(
    algorithm: HashAlgorithm,
    observed: usize,
) -> Result<(), TimestampingError> {
    let expected = match algorithm {
        HashAlgorithm::Sha256 | HashAlgorithm::Blake3 => 32,
        HashAlgorithm::Sha384 => 48,
        HashAlgorithm::Sha512 => 64,
    };
    if observed == expected {
        Ok(())
    } else {
        Err(TimestampingError::BadImprintLength {
            algorithm: algorithm.slug(),
            expected,
            got: observed,
        })
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_timestamping::crate_name(),
///     "invoicekit-timestamping"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-timestamping"
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256_imprint(payload: &[u8]) -> Vec<u8> {
        use blake3::Hasher;
        // SHA-256 is the canonical hash; this test uses
        // BLAKE3 truncated to 32 bytes as the imprint to keep
        // the dep footprint small. The substrate doesn't care
        // which hash function produced the bytes — it only
        // cares about the length.
        let mut h = Hasher::new();
        h.update(payload);
        h.finalize().as_bytes().to_vec()
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-timestamping");
    }

    #[test]
    fn validate_imprint_length_accepts_correct_sizes() {
        assert!(validate_imprint_length(HashAlgorithm::Sha256, 32).is_ok());
        assert!(validate_imprint_length(HashAlgorithm::Sha384, 48).is_ok());
        assert!(validate_imprint_length(HashAlgorithm::Sha512, 64).is_ok());
        assert!(validate_imprint_length(HashAlgorithm::Blake3, 32).is_ok());
    }

    #[test]
    fn validate_imprint_length_rejects_wrong_size() {
        let err = validate_imprint_length(HashAlgorithm::Sha256, 16).unwrap_err();
        match err {
            TimestampingError::BadImprintLength { expected, got, .. } => {
                assert_eq!(expected, 32);
                assert_eq!(got, 16);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn hash_algorithm_round_trips_kebab_json() {
        let json = serde_json::to_string(&HashAlgorithm::Sha256).unwrap();
        assert_eq!(json, "\"sha-256\"");
        let back: HashAlgorithm = serde_json::from_str(&json).unwrap();
        assert_eq!(back, HashAlgorithm::Sha256);
    }

    #[test]
    fn mock_timestamp_request_round_trip() {
        let client = MockTimestampClient::new();
        let imprint = sha256_imprint(b"hello");
        let req = TimestampRequest {
            algorithm: HashAlgorithm::Sha256,
            message_imprint: imprint.clone(),
            nonce: Some(42),
            cert_req: true,
        };
        let ts = client.request_timestamp(&req).unwrap();
        assert_eq!(ts.algorithm, HashAlgorithm::Sha256);
        assert_eq!(ts.message_imprint, imprint);
        assert_eq!(ts.generated_at, "2026-01-01T00:00:00Z");
        assert_eq!(ts.tsa_name, "mock-tsa");
        assert_eq!(ts.serial, "1");
        // Verify the token round-trip.
        client.verify_timestamp(&ts, &imprint).unwrap();
    }

    #[test]
    fn mock_timestamp_verify_detects_drift() {
        let client = MockTimestampClient::new();
        let imprint = sha256_imprint(b"hello");
        let ts = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha256,
                message_imprint: imprint,
                nonce: None,
                cert_req: false,
            })
            .unwrap();
        let drifted = sha256_imprint(b"goodbye");
        let err = client.verify_timestamp(&ts, &drifted).unwrap_err();
        assert!(matches!(err, TimestampingError::ImprintDrift));
    }

    #[test]
    fn mock_timestamp_serial_increments_per_client() {
        let client = MockTimestampClient::new();
        let imprint = sha256_imprint(b"a");
        let a = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha256,
                message_imprint: imprint.clone(),
                nonce: None,
                cert_req: false,
            })
            .unwrap();
        let b = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha256,
                message_imprint: imprint,
                nonce: None,
                cert_req: false,
            })
            .unwrap();
        assert_eq!(a.serial, "1");
        assert_eq!(b.serial, "2");
    }

    #[test]
    fn mock_timestamp_rejects_wrong_imprint_length() {
        let client = MockTimestampClient::new();
        let err = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha512,
                message_imprint: vec![0_u8; 32],
                nonce: None,
                cert_req: false,
            })
            .unwrap_err();
        assert!(matches!(err, TimestampingError::BadImprintLength { .. }));
    }

    #[test]
    fn rfc_timestamp_round_trips_through_json() {
        let ts = RfcTimestamp {
            token: vec![1, 2, 3],
            algorithm: HashAlgorithm::Sha256,
            message_imprint: vec![9_u8; 32],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
            tsa_name: "test-tsa".to_owned(),
            serial: "42".to_owned(),
        };
        let json = serde_json::to_string(&ts).unwrap();
        let back: RfcTimestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ts);
    }

    #[test]
    fn hash_algorithm_display_uses_slug() {
        assert_eq!(format!("{}", HashAlgorithm::Sha384), "sha-384");
    }
}
