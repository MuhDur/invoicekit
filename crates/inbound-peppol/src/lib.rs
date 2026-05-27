// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-093 impl: Peppol inbound receiver library.
//!
//! Per the T-093 runbook (`docs/operators/PEPPOL-INBOUND.md`), the
//! inbound pipeline runs:
//!
//! 1. **Format-detect** the payload via `invoicekit-format-detect`
//!    to confirm it's a UBL or CII document.
//! 2. **Parse** to [`CommercialDocument`] via the matching format
//!    crate (`invoicekit-format-ubl` or `invoicekit-format-cii`).
//! 3. **Validate** via the rule pack (T-031 — the JVM-validator
//!    bridge is a follow-up; today the scaffold trusts the IR's
//!    own envelope checks).
//! 4. **Archive** the evidence bundle (T-081 — the archive crate
//!    is a follow-up; today the scaffold writes the bundle via an
//!    injected [`Archive`] trait).
//!
//! This crate is the library half. The eventual
//! `services/inbound-peppol/` axum binary wraps it in an HTTP
//! handler that accepts either the partner-AP webhook payload
//! (T-091) or the unwrapped XML from the native AS4 receiver
//! (T-095). Both call sites end up at [`InboundPipeline::process`].

use invoicekit_format_detect::{detect_format, FormatId};
use invoicekit_ir::{CommercialDocument, LossinessLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Source of the inbound bytes. Logged on the evidence bundle so
/// the operator can tell whether a given invoice came through the
/// partner AP or the native AS4 receiver.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InboundSource {
    /// Delivered via the partner AP webhook (T-091).
    PartnerWebhook,
    /// Delivered via the native AS4 receiver (T-095).
    NativeAs4,
}

/// One inbound document the pipeline accepts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundDocument {
    /// Tenant on whose behalf the document is delivered. Surfaced
    /// in the audit log + archive bundle.
    pub tenant_id: String,
    /// Trace id propagated from the gateway adapter or the
    /// webhook handler.
    pub trace_id: String,
    /// Source of the bytes.
    pub source: InboundSource,
    /// Raw payload bytes (UBL or CII XML).
    pub payload: Vec<u8>,
}

/// Archive abstraction. The production archive crate (T-081) ships
/// an S3 Object Lock / Azure WORM-backed impl behind a `s3` or
/// `azure` cargo feature. Tests inject [`MockArchive`].
pub trait Archive: Send + Sync {
    /// Persist the evidence bundle.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::Persist`] when the backing store
    /// rejects the bundle.
    fn persist(&self, bundle: &EvidenceBundle) -> Result<EvidenceReceipt, ArchiveError>;
}

/// Errors raised by [`Archive`] implementations.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// The backing store rejected the bundle.
    #[error("evidence archive persist failed: {0}")]
    Persist(String),
}

/// Evidence bundle persisted on every accepted inbound document.
/// Shape locked here so the eventual T-081 archive crate can read
/// without re-deriving the schema.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    /// Tenant on whose behalf the document was delivered.
    pub tenant_id: String,
    /// Trace id.
    pub trace_id: String,
    /// Source of the bytes (partner-webhook | native-as4).
    pub source: InboundSource,
    /// Sniffed format identifier (`Ubl21` | `CiiD16B` | ...).
    pub detected_format: FormatId,
    /// Hex sha256 of the raw payload — content-addresses the
    /// archive entry.
    pub payload_sha256_hex: String,
    /// Raw payload bytes.
    pub payload: Vec<u8>,
    /// Parsed canonical document.
    pub document: CommercialDocument,
    /// Lossiness ledger emitted by the format crate's `from_xml`.
    /// Surfaces fields the source XML carried that the canonical
    /// IR does not yet represent — the operator's audit trail for
    /// inbound semantic drift.
    pub lossiness_ledger: LossinessLedger,
    /// Validator findings (empty until the T-031 bridge lands).
    pub validator_findings: Vec<ValidatorFinding>,
}

/// A single validator finding. Mirrors the shape the JVM-validator
/// JSON-RPC bridge returns so the schema is stable across the
/// scaffold and the eventual real validator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatorFinding {
    /// Rule identifier (e.g. `BR-CO-15`).
    pub rule: String,
    /// Severity (`error` | `warning` | `info`).
    pub severity: String,
    /// Human-readable message.
    pub message: String,
}

/// Per-archive-persist receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceReceipt {
    /// Stable archive id (typically the bundle's sha256 prefix).
    pub archive_id: String,
}

/// Errors raised by [`InboundPipeline::process`].
#[derive(Debug, Error)]
pub enum InboundError {
    /// `format-detect` did not recognise the payload as UBL or CII.
    #[error("inbound payload format not recognised: {0:?}")]
    UnrecognizedFormat(FormatId),
    /// The format crate's `from_xml` rejected the payload.
    #[error("inbound payload parse failed: {0}")]
    Parse(String),
    /// IR validation rejected the parsed document.
    #[error("inbound payload validation failed: {0}")]
    Validate(String),
    /// The archive trait rejected the evidence bundle.
    #[error("inbound archive persist failed: {0}")]
    Archive(#[from] ArchiveError),
}

/// Inbound pipeline. Holds the injected [`Archive`]; the rest of
/// the surface is pure logic over the InvoiceKit format / validate
/// crates.
pub struct InboundPipeline {
    archive: Box<dyn Archive>,
}

impl InboundPipeline {
    /// Build a new pipeline.
    #[must_use]
    pub fn new(archive: Box<dyn Archive>) -> Self {
        Self { archive }
    }

    /// Run the full pipeline on one inbound document.
    ///
    /// # Errors
    ///
    /// Returns [`InboundError`] when any pipeline stage rejects
    /// the payload.
    pub fn process(&self, inbound: InboundDocument) -> Result<EvidenceReceipt, InboundError> {
        let detected_format = detect_format(&inbound.payload);
        let (document, lossiness_ledger) = match detected_format {
            FormatId::Ubl21 => {
                let xml = std::str::from_utf8(&inbound.payload).map_err(|e| {
                    InboundError::Parse(format!("UBL payload is not valid UTF-8: {e}"))
                })?;
                invoicekit_format_ubl::from_xml(xml)
                    .map_err(|e| InboundError::Parse(format!("UBL: {e}")))?
            }
            FormatId::CiiD16B => {
                let xml = std::str::from_utf8(&inbound.payload).map_err(|e| {
                    InboundError::Parse(format!("CII payload is not valid UTF-8: {e}"))
                })?;
                invoicekit_format_cii::from_xml(xml)
                    .map_err(|e| InboundError::Parse(format!("CII: {e}")))?
            }
            other => return Err(InboundError::UnrecognizedFormat(other)),
        };
        document
            .validate()
            .map_err(|e| InboundError::Validate(format!("{e}")))?;

        // The T-031 JVM-validator bridge will populate the
        // findings vec; today we ship an empty list so the bundle
        // shape stays forward-compatible.
        let validator_findings: Vec<ValidatorFinding> = Vec::new();

        let bundle = EvidenceBundle {
            tenant_id: inbound.tenant_id,
            trace_id: inbound.trace_id,
            source: inbound.source,
            detected_format,
            payload_sha256_hex: sha256_hex(&inbound.payload),
            payload: inbound.payload,
            document,
            lossiness_ledger,
            validator_findings,
        };
        let receipt = self.archive.persist(&bundle)?;
        Ok(receipt)
    }
}

fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(out, "{byte:02x}").expect("writing to a String never fails");
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_inbound_peppol::crate_name(),
///     "invoicekit-inbound-peppol"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-inbound-peppol"
}

// ----- test scaffolding ------------------------------------------

/// Mock archive. Records every persisted bundle; returns a
/// receipt whose archive id is the bundle's payload sha256
/// prefix.
pub struct MockArchive {
    persisted: std::sync::Mutex<Vec<EvidenceBundle>>,
}

impl MockArchive {
    /// Build an empty mock archive.
    #[must_use]
    pub fn new() -> Self {
        Self {
            persisted: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of persisted bundles so far.
    ///
    /// # Panics
    ///
    /// Panics if a prior call panicked while holding the mutex.
    #[must_use]
    pub fn persisted(&self) -> Vec<EvidenceBundle> {
        self.persisted.lock().unwrap().clone()
    }
}

impl Default for MockArchive {
    fn default() -> Self {
        Self::new()
    }
}

impl Archive for MockArchive {
    fn persist(&self, bundle: &EvidenceBundle) -> Result<EvidenceReceipt, ArchiveError> {
        self.persisted.lock().unwrap().push(bundle.clone());
        Ok(EvidenceReceipt {
            archive_id: bundle
                .payload_sha256_hex
                .chars()
                .take(12)
                .collect::<String>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ubl_payload() -> Vec<u8> {
        // The same shape format-detect's positive UBL test uses.
        br#"<?xml version="1.0"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#
            .to_vec()
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-inbound-peppol");
    }

    #[test]
    fn process_rejects_unrecognised_format() {
        let pipeline = InboundPipeline::new(Box::new(MockArchive::new()));
        let err = pipeline
            .process(InboundDocument {
                tenant_id: "t".to_owned(),
                trace_id: "tr".to_owned(),
                source: InboundSource::PartnerWebhook,
                payload: b"not an invoice".to_vec(),
            })
            .unwrap_err();
        assert!(matches!(err, InboundError::UnrecognizedFormat(_)));
    }

    #[test]
    fn process_surfaces_parse_failure_on_malformed_ubl() {
        // format-detect recognises this as UBL by namespace match
        // even though the body is incomplete; from_xml rejects it.
        let pipeline = InboundPipeline::new(Box::new(MockArchive::new()));
        let err = pipeline
            .process(InboundDocument {
                tenant_id: "t".to_owned(),
                trace_id: "tr".to_owned(),
                source: InboundSource::NativeAs4,
                payload: ubl_payload(),
            })
            .unwrap_err();
        // The minimal UBL skeleton has no lines / totals etc., so
        // from_xml rejects via IR validation. Either Parse or
        // Validate is acceptable depending on where the format
        // crate enforces the envelope.
        assert!(
            matches!(err, InboundError::Parse(_) | InboundError::Validate(_)),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // sha256("abc") = ba7816bf...f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn inbound_source_round_trips_kebab_case_json() {
        let json = serde_json::to_string(&InboundSource::PartnerWebhook).unwrap();
        assert_eq!(json, "\"partner-webhook\"");
        let back: InboundSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, InboundSource::PartnerWebhook);
        let json = serde_json::to_string(&InboundSource::NativeAs4).unwrap();
        assert_eq!(json, "\"native-as4\"");
    }
}
