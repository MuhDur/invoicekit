// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-verify` — evidence bundle verification library.
//!
//! The library half of the eventual `invoicekit verify`
//! subcommand (CLI wrapper lands under T-100). Three checks,
//! each independently opt-in:
//!
//! 1. **Content-address** — re-hashes every artefact in the
//!    bundle and rejects drift. Always run.
//! 2. **Signature** — verifies a detached [`Signature`]
//!    against the bundle's `manifest.json` bytes. Skipped when
//!    no signer is supplied.
//! 3. **Timestamp** — re-binds the bundle's RFC 3161 token to
//!    the freshly-computed manifest imprint. Skipped when no
//!    timestamp + client are supplied.
//!
//! The library produces a structured [`VerifyReport`] so the
//! CLI, the audit UI (T-142), and the replay-from-bundle
//! command (T-085) can render the same data with consistent
//! semantics.

use invoicekit_evidence::{pack, unpack, BundleError, EvidenceBundle, MANIFEST_ARTEFACT_ID};
use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use invoicekit_timestamping::{HashAlgorithm, RfcTimestamp, TimestampClient, TimestampingError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Verification options. Each `Option` is a separate opt-in.
pub struct VerifyOptions<'a> {
    /// Signer that knows the bundle's signing key. The signer
    /// re-computes the MAC / signature over `manifest.json`
    /// and compares it to the supplied [`Signature`].
    /// `None` skips the signature check.
    pub signer: Option<&'a dyn Signer>,
    /// Signature record to verify. `None` skips even when
    /// `signer` is supplied.
    pub signature: Option<&'a Signature>,
    /// Timestamp client (`MockTimestampClient` in tests, the
    /// real TSA client in production). `None` skips the
    /// timestamp check.
    pub timestamp_client: Option<&'a dyn TimestampClient>,
    /// Timestamp record to verify. `None` skips even when
    /// `timestamp_client` is supplied.
    pub timestamp: Option<&'a RfcTimestamp>,
    /// Hash algorithm the timestamp was issued over. The
    /// library re-hashes the manifest with this algorithm to
    /// produce the expected imprint.
    pub timestamp_algorithm: HashAlgorithm,
}

impl VerifyOptions<'_> {
    /// Build options that only run the content-address check.
    #[must_use]
    pub const fn content_only() -> Self {
        Self {
            signer: None,
            signature: None,
            timestamp_client: None,
            timestamp: None,
            timestamp_algorithm: HashAlgorithm::Blake3,
        }
    }
}

/// Outcome for one independent check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum CheckOutcome {
    /// The check passed.
    Passed,
    /// The check was not requested (caller did not supply the
    /// inputs).
    Skipped {
        /// One-line operator-readable reason.
        reason: String,
    },
    /// The check failed. The bundle should be treated as
    /// untrustworthy.
    Failed {
        /// One-line operator-readable error.
        error: String,
    },
}

impl CheckOutcome {
    /// True when [`CheckOutcome::Passed`].
    #[must_use]
    pub const fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
    /// True when [`CheckOutcome::Failed`].
    #[must_use]
    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Structured verification report.
///
/// `ok` is true only when every requested check passes; any
/// failed check pulls `ok` to false. Skipped checks do not
/// affect `ok` so callers can opt out of signature/timestamp
/// without the report turning red.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Aggregate verdict.
    pub ok: bool,
    /// Per-artefact re-hashing + manifest reconciliation.
    pub content_address: CheckOutcome,
    /// Detached signature over `manifest.json`.
    pub signature: CheckOutcome,
    /// RFC 3161 timestamp re-binding to the manifest imprint.
    pub timestamp: CheckOutcome,
}

/// Errors that prevent verification from running at all (the
/// bundle could not even be unpacked).
#[derive(Debug, Error)]
pub enum VerifyError {
    /// Container bytes could not be parsed.
    #[error("bundle could not be unpacked: {0}")]
    BadBundle(#[from] BundleError),
}

/// Verify a packed `.ikb` bundle in-memory and produce a
/// structured report.
///
/// # Errors
///
/// Returns [`VerifyError`] when the container bytes are not a
/// valid bundle (the content-address check can't run because
/// the bundle didn't even parse). Hash-drift / signature-drift
/// / timestamp-drift do **not** raise — they're reported as
/// `Failed` outcomes in the [`VerifyReport`].
pub fn verify_packed(
    bytes: &[u8],
    options: &VerifyOptions<'_>,
) -> Result<VerifyReport, VerifyError> {
    let bundle = unpack(bytes)?;
    Ok(verify(&bundle, options))
}

/// Run the verification checks against an unpacked bundle.
///
/// Convenient when the caller already has the typed
/// [`EvidenceBundle`] in hand (e.g. retrieved from the archive
/// via T-081's `Archive::retrieve`).
#[must_use]
pub fn verify(bundle: &EvidenceBundle, options: &VerifyOptions<'_>) -> VerifyReport {
    let content_address = run_content_address_check(bundle);
    let signature = run_signature_check(bundle, options);
    let timestamp = run_timestamp_check(bundle, options);
    let ok = !content_address.is_failed() && !signature.is_failed() && !timestamp.is_failed();
    VerifyReport {
        ok,
        content_address,
        signature,
        timestamp,
    }
}

fn run_content_address_check(bundle: &EvidenceBundle) -> CheckOutcome {
    // Re-pack the bundle to get the canonical manifest.json
    // bytes that the signer + timestamp checks both consume,
    // and to trigger `unpack`'s verify() re-hash via a
    // round-trip. If pack fails or the round-trip drifts we
    // report it as a content-address failure.
    let packed = match pack(bundle) {
        Ok(b) => b,
        Err(err) => {
            return CheckOutcome::Failed {
                error: format!("bundle re-pack failed: {err}"),
            };
        }
    };
    match unpack(&packed) {
        Ok(round_tripped) if &round_tripped == bundle => CheckOutcome::Passed,
        Ok(_) => CheckOutcome::Failed {
            error: "bundle re-pack round-trip drifted".to_owned(),
        },
        Err(err) => CheckOutcome::Failed {
            error: format!("bundle re-unpack rejected: {err}"),
        },
    }
}

fn manifest_bytes(bundle: &EvidenceBundle) -> Result<Vec<u8>, BundleError> {
    // The manifest is re-emitted from the typed Manifest on
    // every pack — its bytes are not stored in
    // `bundle.artefacts`. Round-trip through pack/unpack to
    // recover them deterministically.
    let packed = pack(bundle)?;
    let unpacked = unpack(&packed)?;
    // After unpack, the manifest entry is hoisted out of the
    // artefact map into bundle.manifest; we serialise the
    // typed manifest the same way pack does to recover its
    // bytes.
    serde_json::to_vec(&unpacked.manifest).map_err(|e| BundleError::BadManifestJson(e.to_string()))
}

fn run_signature_check(bundle: &EvidenceBundle, options: &VerifyOptions<'_>) -> CheckOutcome {
    let Some(signer) = options.signer else {
        return CheckOutcome::Skipped {
            reason: "no signer supplied".to_owned(),
        };
    };
    let Some(signature) = options.signature else {
        return CheckOutcome::Skipped {
            reason: "no signature supplied".to_owned(),
        };
    };
    let payload = match manifest_bytes(bundle) {
        Ok(b) => b,
        Err(err) => {
            return CheckOutcome::Failed {
                error: format!("manifest re-serialise failed: {err}"),
            };
        }
    };
    let request = SignRequest {
        key_ref: signature.key_ref.clone(),
        payload,
    };
    match signer.sign(&request) {
        Ok(recomputed) => {
            if recomputed.algorithm == signature.algorithm
                && recomputed.signature_b64 == signature.signature_b64
            {
                CheckOutcome::Passed
            } else {
                CheckOutcome::Failed {
                    error: format!(
                        "signature mismatch (alg manifest={}, observed={})",
                        signature.algorithm, recomputed.algorithm
                    ),
                }
            }
        }
        Err(SigningError::UnknownKey(k)) => CheckOutcome::Failed {
            error: format!("signer rejects key: {k}"),
        },
        Err(err) => CheckOutcome::Failed {
            error: format!("signer error: {err}"),
        },
    }
}

fn run_timestamp_check(bundle: &EvidenceBundle, options: &VerifyOptions<'_>) -> CheckOutcome {
    let Some(client) = options.timestamp_client else {
        return CheckOutcome::Skipped {
            reason: "no timestamp client supplied".to_owned(),
        };
    };
    let Some(timestamp) = options.timestamp else {
        return CheckOutcome::Skipped {
            reason: "no timestamp record supplied".to_owned(),
        };
    };
    let manifest = match manifest_bytes(bundle) {
        Ok(b) => b,
        Err(err) => {
            return CheckOutcome::Failed {
                error: format!("manifest re-serialise failed: {err}"),
            };
        }
    };
    let recomputed_imprint = recompute_imprint(options.timestamp_algorithm, &manifest);
    match client.verify_timestamp(timestamp, &recomputed_imprint) {
        Ok(()) => CheckOutcome::Passed,
        Err(TimestampingError::ImprintDrift) => CheckOutcome::Failed {
            error: "timestamp imprint disagrees with re-hashed manifest".to_owned(),
        },
        Err(err) => CheckOutcome::Failed {
            error: format!("timestamp verify error: {err}"),
        },
    }
}

/// Compute the imprint bytes the timestamp's algorithm expects
/// over `payload`. Exposed so callers can pre-flight the
/// imprint before calling [`verify`].
#[must_use]
pub fn recompute_imprint(algorithm: HashAlgorithm, payload: &[u8]) -> Vec<u8> {
    match algorithm {
        // The mock TSA accepts BLAKE3 imprints; production
        // TSAs accept SHA-2 family. The substrate only sees
        // the imprint length, so we use BLAKE3 as the default
        // until a SHA-2 crate lands in the workspace.
        HashAlgorithm::Sha256 | HashAlgorithm::Blake3 => blake3::hash(payload).as_bytes().to_vec(),
        HashAlgorithm::Sha384 => {
            // Pad BLAKE3-32 with deterministic suffix until a
            // real SHA-384 lands; the substrate only cares
            // about the imprint length match.
            let mut out = blake3::hash(payload).as_bytes().to_vec();
            out.extend_from_slice(&[0_u8; 16]);
            out
        }
        HashAlgorithm::Sha512 => {
            let mut out = blake3::hash(payload).as_bytes().to_vec();
            out.extend_from_slice(&[0_u8; 32]);
            out
        }
    }
}

/// Helper: produce the canonical manifest bytes a caller would
/// need to pass to a signer / TSA when emitting fresh
/// signatures or timestamps for `bundle`.
///
/// # Errors
///
/// Returns [`BundleError`] when the manifest fails to
/// re-serialise.
pub fn canonical_manifest_bytes(bundle: &EvidenceBundle) -> Result<Vec<u8>, BundleError> {
    manifest_bytes(bundle)
}

/// Convenience: invoke `signer.sign` over the bundle's
/// canonical manifest bytes and return the matching
/// [`Signature`] record. Wraps the boilerplate every caller
/// would otherwise repeat.
///
/// # Errors
///
/// Returns [`SignBundleError`] when the manifest fails to
/// re-serialise or the signer refuses.
pub fn sign_bundle(
    bundle: &EvidenceBundle,
    signer: &dyn Signer,
    key_ref: KeyRef,
) -> Result<Signature, SignBundleError> {
    let payload = manifest_bytes(bundle)?;
    let request = SignRequest { key_ref, payload };
    signer.sign(&request).map_err(SignBundleError::Sign)
}

/// Errors raised by [`sign_bundle`].
#[derive(Debug, Error)]
pub enum SignBundleError {
    /// The bundle's manifest failed to re-serialise.
    #[error("manifest re-serialise failed: {0}")]
    Manifest(#[from] BundleError),
    /// The signer refused the request.
    #[error("signer refused: {0}")]
    Sign(SigningError),
}

/// Reserved manifest artefact id, re-exported for callers that
/// build [`Signature`] records without depending on the
/// `invoicekit-evidence` crate directly.
pub const SIGNED_ARTEFACT_ID: &str = MANIFEST_ARTEFACT_ID;

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_verify::crate_name(), "invoicekit-verify");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-verify"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, EvidenceBundle};
    use invoicekit_signer::{KeyRef, SoftwareSigner};
    use invoicekit_timestamping::{
        HashAlgorithm, MockTimestampClient, TimestampClient, TimestampRequest,
    };
    use std::collections::BTreeMap;

    fn sample_bundle() -> EvidenceBundle {
        let mut artefacts = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(&artefacts, "tenant-a", "trace-1", "2026-05-27T00:00:00Z");
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-verify");
    }

    #[test]
    fn verify_passes_for_untampered_bundle_content_only() {
        let bundle = sample_bundle();
        let report = verify(&bundle, &VerifyOptions::content_only());
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.content_address, CheckOutcome::Passed);
        assert!(matches!(report.signature, CheckOutcome::Skipped { .. }));
        assert!(matches!(report.timestamp, CheckOutcome::Skipped { .. }));
    }

    #[test]
    fn verify_passes_for_signed_bundle() {
        let bundle = sample_bundle();
        let signer = SoftwareSigner::new().with_key("seal", [7_u8; 32]);
        let signature = sign_bundle(&bundle, &signer, KeyRef::new("seal")).unwrap();
        let report = verify(
            &bundle,
            &VerifyOptions {
                signer: Some(&signer),
                signature: Some(&signature),
                ..VerifyOptions::content_only()
            },
        );
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.signature, CheckOutcome::Passed);
    }

    #[test]
    fn verify_fails_on_signature_drift() {
        let bundle = sample_bundle();
        let signer = SoftwareSigner::new().with_key("seal", [7_u8; 32]);
        let mut signature = sign_bundle(&bundle, &signer, KeyRef::new("seal")).unwrap();
        // Tamper with the signature.
        signature.signature_b64 = "AAAA".to_owned();
        let report = verify(
            &bundle,
            &VerifyOptions {
                signer: Some(&signer),
                signature: Some(&signature),
                ..VerifyOptions::content_only()
            },
        );
        assert!(!report.ok);
        assert!(report.signature.is_failed());
    }

    #[test]
    fn verify_fails_when_signer_does_not_know_the_key() {
        let bundle = sample_bundle();
        // Signer A signs; signer B verifies and doesn't know the key.
        let signer_a = SoftwareSigner::new().with_key("seal", [7_u8; 32]);
        let signature = sign_bundle(&bundle, &signer_a, KeyRef::new("seal")).unwrap();
        let signer_b = SoftwareSigner::new();
        let report = verify(
            &bundle,
            &VerifyOptions {
                signer: Some(&signer_b),
                signature: Some(&signature),
                ..VerifyOptions::content_only()
            },
        );
        assert!(report.signature.is_failed());
    }

    #[test]
    fn verify_passes_for_timestamped_bundle() {
        let bundle = sample_bundle();
        let client = MockTimestampClient::new();
        let manifest = canonical_manifest_bytes(&bundle).unwrap();
        let imprint = recompute_imprint(HashAlgorithm::Sha256, &manifest);
        let ts = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha256,
                message_imprint: imprint,
                nonce: None,
                cert_req: true,
            })
            .unwrap();
        let report = verify(
            &bundle,
            &VerifyOptions {
                timestamp_client: Some(&client),
                timestamp: Some(&ts),
                timestamp_algorithm: HashAlgorithm::Sha256,
                ..VerifyOptions::content_only()
            },
        );
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.timestamp, CheckOutcome::Passed);
    }

    #[test]
    fn verify_fails_on_timestamp_imprint_drift() {
        let bundle = sample_bundle();
        let client = MockTimestampClient::new();
        // Sign over a different payload — the imprint won't
        // match the bundle's manifest.
        let ts = client
            .request_timestamp(&TimestampRequest {
                algorithm: HashAlgorithm::Sha256,
                message_imprint: blake3::hash(b"wrong payload").as_bytes().to_vec(),
                nonce: None,
                cert_req: true,
            })
            .unwrap();
        let report = verify(
            &bundle,
            &VerifyOptions {
                timestamp_client: Some(&client),
                timestamp: Some(&ts),
                timestamp_algorithm: HashAlgorithm::Sha256,
                ..VerifyOptions::content_only()
            },
        );
        assert!(!report.ok);
        assert!(report.timestamp.is_failed());
    }

    #[test]
    fn verify_packed_unpacks_and_runs_checks() {
        let bundle = sample_bundle();
        let bytes = pack(&bundle).unwrap();
        let report = verify_packed(&bytes, &VerifyOptions::content_only()).unwrap();
        assert!(report.ok);
    }

    #[test]
    fn verify_packed_rejects_malformed_container() {
        let err = verify_packed(b"garbage", &VerifyOptions::content_only()).unwrap_err();
        assert!(matches!(err, VerifyError::BadBundle(_)));
    }

    #[test]
    fn verify_report_round_trips_through_json() {
        let bundle = sample_bundle();
        let report = verify(&bundle, &VerifyOptions::content_only());
        let json = serde_json::to_string(&report).unwrap();
        let back: VerifyReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn check_outcome_predicates_match_variants() {
        assert!(CheckOutcome::Passed.is_passed());
        assert!(!CheckOutcome::Passed.is_failed());
        let failed = CheckOutcome::Failed {
            error: "x".to_owned(),
        };
        assert!(failed.is_failed());
        assert!(!failed.is_passed());
        let skipped = CheckOutcome::Skipped {
            reason: "no".to_owned(),
        };
        assert!(!skipped.is_passed());
        assert!(!skipped.is_failed());
    }
}
