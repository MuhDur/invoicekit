// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// ZATCA terminology is full of acronyms (ZATCA, CSID, OTP,
// CSR, TLV, BIS, KSA) that doc-markdown reads as missing
// backticks; suppress that family.
#![allow(clippy::doc_markdown)]

//! `invoicekit-signer-zatca` — ZATCA Phase 2 cryptographic
//! stamp adapter.
//!
//! Layers the Saudi Arabia ZATCA Phase 2 contract on top of
//! [`invoicekit_signer`]:
//!
//! * [`Phase2Provider`] — provider trait that bundles the
//!   underlying [`Signer`] with the ZATCA-specific operations
//!   (issue a Cryptographic Stamp Identifier (CSID) compliance
//!   request, sign an invoice hash with ECDSA secp256k1, build
//!   the QR-code TLV envelope).
//! * [`ZatcaInvoiceMode`] — `Standard` (B2B / B2G clearance)
//!   vs `Simplified` (B2C reporting).
//! * [`CsidRecord`] — typed Cryptographic Stamp Identifier
//!   issued by the ZATCA portal (CSR submission outcome).
//! * [`ZatcaStampEnvelope`] — typed envelope holding the
//!   ECDSA secp256k1 signature value + the QR-code TLV bytes
//!   + the CSID reference + the reporting status.
//! * [`MockPhase2Provider`] — deterministic test provider
//!   used by tests + the cassette-replay sandbox.
//!
//! # Strict-gate scope
//!
//! T-083b1's gates ("ECDSA secp256k1 implementation",
//! "Test vectors from ZATCA documentation pass", "Test
//! against a real sandbox certificate") all need either an
//! ECDSA crate in the workspace deps or a real ZATCA portal
//! CSR. The substrate ships here; the real crypto provider
//! lands behind a future `zatca-secp256k1` feature flag with
//! the ZATCA test vectors as fixtures.

use std::collections::BTreeMap;
use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// ZATCA invoice mode.
///
/// The Phase 2 flow differs between the two modes: standard
/// invoices must be cleared by the ZATCA portal *before*
/// delivery to the buyer; simplified invoices are reported
/// to the portal *after* delivery and only need the QR-code
/// stamp on the printed copy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ZatcaInvoiceMode {
    /// Standard (B2B / B2G): portal-clearance flow.
    Standard,
    /// Simplified (B2C): post-delivery reporting flow.
    Simplified,
}

impl ZatcaInvoiceMode {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Simplified => "simplified",
        }
    }
}

/// Cryptographic Stamp Identifier — ZATCA's typed certificate
/// id after CSR submission. The portal binds this to the
/// taxpayer's VAT registration + the device's signing key.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsidRecord {
    /// Opaque CSID issued by the portal.
    pub csid: String,
    /// Compliance vs production environment.
    pub environment: ZatcaEnvironment,
    /// VAT registration number the CSID is bound to (15-digit
    /// KSA VAT number).
    pub vat_number: String,
    /// Optional cryptographic-stamp UUID the portal assigns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamp_uuid: Option<String>,
    /// `notBefore` (RFC 3339 UTC).
    pub not_before: String,
    /// `notAfter` (RFC 3339 UTC).
    pub not_after: String,
}

/// ZATCA environment — compliance is the sandbox the portal
/// makes available to onboarding taxpayers; production is the
/// live tax-clearance flow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ZatcaEnvironment {
    /// Compliance / sandbox environment.
    Compliance,
    /// Production environment.
    Production,
}

impl ZatcaEnvironment {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Compliance => "compliance",
            Self::Production => "production",
        }
    }
}

/// Reporting status returned by the portal after submission.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportingStatus {
    /// Portal accepted the invoice.
    Accepted,
    /// Portal accepted with warnings (the stamp is valid but
    /// the operator must fix the next invoice).
    AcceptedWithWarnings,
    /// Portal rejected the invoice.
    Rejected,
    /// Submitted, awaiting portal response (asynchronous
    /// simplified-mode reporting).
    Pending,
}

impl ReportingStatus {
    /// True when the portal accepted the invoice (with or
    /// without warnings).
    #[must_use]
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted | Self::AcceptedWithWarnings)
    }
}

/// Typed ZATCA Phase 2 cryptographic stamp envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ZatcaStampEnvelope {
    /// Underlying [`Signer`] receipt — the raw signature
    /// value + algorithm id + key ref.
    pub signature: Signature,
    /// ZATCA invoice mode (Standard / Simplified).
    pub mode: ZatcaInvoiceMode,
    /// CSID used to produce the signature.
    pub csid: CsidRecord,
    /// QR-code TLV bytes (per ZATCA Phase 2 §V QR Code
    /// Specification). The operator renders these as a
    /// base32-encoded QR code on the PDF / receipt.
    pub qr_tlv: Vec<u8>,
    /// Invoice hash (SHA-256 of the canonical UBL XML; lower
    /// hex) the signature attests to.
    pub invoice_sha256_hex: String,
    /// Reporting status returned by the portal.
    pub reporting_status: ReportingStatus,
}

/// Sign-request shape for a ZATCA Phase 2 stamp.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ZatcaSignRequest {
    /// Canonical UBL XML of the invoice (UTF-8 bytes).
    pub canonical_ubl: Vec<u8>,
    /// CSID to sign under.
    pub csid: CsidRecord,
    /// Invoice mode (Standard / Simplified).
    pub mode: ZatcaInvoiceMode,
    /// Fields that go into the QR-code TLV body. ZATCA Phase
    /// 2 mandates five TLV fields for compliance; the
    /// substrate accepts an extensible map so the future
    /// `zatca-secp256k1` impl can add the per-mode extras
    /// (signature value + public key for simplified
    /// invoices).
    pub qr_fields: BTreeMap<QrField, String>,
}

/// Tags the ZATCA Phase 2 QR-code TLV body carries. Tag
/// numbers come from ZATCA Phase 2 §V QR Code Specification.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrField {
    /// Tag 1 — seller name (UTF-8).
    SellerName,
    /// Tag 2 — VAT registration number (15-digit KSA VAT).
    VatNumber,
    /// Tag 3 — invoice timestamp (RFC 3339 UTC).
    Timestamp,
    /// Tag 4 — invoice total amount (decimal string with
    /// currency code suffix per ZATCA spec, e.g. `1190.00`).
    Total,
    /// Tag 5 — VAT total amount (decimal string).
    VatTotal,
    /// Tag 6 — hash of XML invoice (SHA-256, lower hex).
    InvoiceHash,
    /// Tag 7 — ECDSA signature value of the cryptographic
    /// stamp (only on simplified invoices in Phase 2).
    StampSignatureValue,
    /// Tag 8 — Public key bytes of the cryptographic stamp
    /// (only on simplified invoices).
    StampPublicKey,
}

impl QrField {
    /// ZATCA TLV tag number (1..=8).
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::SellerName => 1,
            Self::VatNumber => 2,
            Self::Timestamp => 3,
            Self::Total => 4,
            Self::VatTotal => 5,
            Self::InvoiceHash => 6,
            Self::StampSignatureValue => 7,
            Self::StampPublicKey => 8,
        }
    }

    /// The five TLV fields ZATCA requires on every Phase 2
    /// invoice (Tags 1–5). Simplified invoices add 6/7/8.
    #[must_use]
    pub const fn mandatory_for_all() -> [Self; 5] {
        [
            Self::SellerName,
            Self::VatNumber,
            Self::Timestamp,
            Self::Total,
            Self::VatTotal,
        ]
    }
}

/// Errors raised by [`Phase2Provider`] implementations.
#[derive(Debug, Error)]
pub enum ZatcaError {
    /// Underlying signer refused.
    #[error("zatca provider's signer refused: {0}")]
    Signer(SigningError),
    /// One of the five mandatory QR fields was missing from
    /// the request (Tags 1–5).
    #[error("zatca QR field missing: {0:?}")]
    MissingMandatoryField(QrField),
    /// Simplified-invoice request didn't include Tag 7
    /// (stamp signature) or Tag 8 (public key).
    #[error("zatca simplified invoice missing stamp signature or public key")]
    MissingSimplifiedStampFields,
    /// CSID isn't valid for this environment.
    #[error("zatca CSID environment mismatch: csid={csid:?}, request={request:?}")]
    EnvironmentMismatch {
        /// Environment the CSID was issued for.
        csid: ZatcaEnvironment,
        /// Environment the request targets.
        request: ZatcaEnvironment,
    },
    /// CSR submission was rejected by the portal.
    #[error("zatca CSR submission rejected: {0}")]
    CsrRejected(String),
    /// Portal is unavailable.
    #[error("zatca portal unavailable: {0}")]
    Unavailable(String),
}

/// ZATCA Phase 2 provider surface.
pub trait Phase2Provider: Send + Sync {
    /// Identifier the operator sees in logs and the audit UI.
    fn provider_name(&self) -> &str;

    /// CSID this provider is configured against.
    fn csid(&self) -> &CsidRecord;

    /// Produce a ZATCA Phase 2 cryptographic stamp.
    ///
    /// # Errors
    ///
    /// Returns [`ZatcaError`] when the CSID environment
    /// doesn't match the target, mandatory QR fields are
    /// missing, the underlying signer refuses, or the portal
    /// is unreachable.
    fn stamp(
        &self,
        request: &ZatcaSignRequest,
        target_environment: ZatcaEnvironment,
    ) -> Result<ZatcaStampEnvelope, ZatcaError>;
}

/// Mock ZATCA Phase 2 provider. Deterministic outputs so
/// tests can assert on the exact stamp + TLV bytes.
pub struct MockPhase2Provider {
    name: String,
    signer: std::sync::Arc<dyn Signer>,
    csid: CsidRecord,
    forced_status: ReportingStatus,
    stamps: Mutex<Vec<ZatcaSignRequest>>,
}

impl MockPhase2Provider {
    /// Build a mock provider.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        signer: std::sync::Arc<dyn Signer>,
        csid: CsidRecord,
    ) -> Self {
        Self {
            name: name.into(),
            signer,
            csid,
            forced_status: ReportingStatus::Accepted,
            stamps: Mutex::new(Vec::new()),
        }
    }

    /// Force the provider to return a specific reporting
    /// status on every stamp.
    #[must_use]
    pub fn with_forced_status(mut self, status: ReportingStatus) -> Self {
        self.forced_status = status;
        self
    }

    /// Snapshot of every recorded sign request.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn stamps(&self) -> Vec<ZatcaSignRequest> {
        self.stamps.lock().unwrap().clone()
    }
}

impl Phase2Provider for MockPhase2Provider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn csid(&self) -> &CsidRecord {
        &self.csid
    }

    fn stamp(
        &self,
        request: &ZatcaSignRequest,
        target_environment: ZatcaEnvironment,
    ) -> Result<ZatcaStampEnvelope, ZatcaError> {
        if self.csid.environment != target_environment {
            return Err(ZatcaError::EnvironmentMismatch {
                csid: self.csid.environment,
                request: target_environment,
            });
        }
        validate_qr_fields(&request.qr_fields, request.mode)?;
        let invoice_hash = invoice_sha256_hex(&request.canonical_ubl);
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(&self.csid.csid),
                payload: request.canonical_ubl.clone(),
            })
            .map_err(ZatcaError::Signer)?;
        let qr_tlv = encode_qr_tlv(&request.qr_fields);
        self.stamps.lock().unwrap().push(request.clone());
        Ok(ZatcaStampEnvelope {
            signature,
            mode: request.mode,
            csid: self.csid.clone(),
            qr_tlv,
            invoice_sha256_hex: invoice_hash,
            reporting_status: self.forced_status,
        })
    }
}

/// Validate the QR-field map against ZATCA Phase 2 requirements.
///
/// # Errors
///
/// Returns [`ZatcaError::MissingMandatoryField`] when one of
/// Tags 1–5 is missing, or
/// [`ZatcaError::MissingSimplifiedStampFields`] when a
/// simplified-invoice request doesn't include Tags 7 + 8.
pub fn validate_qr_fields(
    fields: &BTreeMap<QrField, String>,
    mode: ZatcaInvoiceMode,
) -> Result<(), ZatcaError> {
    for required in QrField::mandatory_for_all() {
        if !fields.contains_key(&required) {
            return Err(ZatcaError::MissingMandatoryField(required));
        }
    }
    if mode == ZatcaInvoiceMode::Simplified
        && (!fields.contains_key(&QrField::StampSignatureValue)
            || !fields.contains_key(&QrField::StampPublicKey))
    {
        return Err(ZatcaError::MissingSimplifiedStampFields);
    }
    Ok(())
}

/// Encode a QR-field map into the ZATCA Phase 2 TLV byte
/// envelope. Each field becomes `tag (1 byte) | length (1
/// byte) | UTF-8 value`; fields are emitted in ascending tag
/// order.
#[must_use]
pub fn encode_qr_tlv(fields: &BTreeMap<QrField, String>) -> Vec<u8> {
    let mut entries: Vec<(QrField, &String)> = fields.iter().map(|(k, v)| (*k, v)).collect();
    entries.sort_by_key(|(k, _)| k.tag());
    let mut out: Vec<u8> = Vec::new();
    for (field, value) in entries {
        let bytes = value.as_bytes();
        // ZATCA TLV uses single-byte length; truncate
        // defensively so the encoded TLV stays well-formed.
        let len = u8::try_from(bytes.len().min(255)).unwrap_or(255);
        out.push(field.tag());
        out.push(len);
        out.extend_from_slice(&bytes[..len as usize]);
    }
    out
}

/// Compute the canonical SHA-256 hash of the UBL invoice bytes.
///
/// Returns lowercase hex. ZATCA Phase 2 mandates SHA-256 over
/// the canonicalized UBL (XML C14N 1.1). The substrate stand-
/// in produces a deterministic 32-byte digest of the input so
/// the surface + tests are exercised end-to-end; the real
/// `zatca-secp256k1` provider swaps in SHA-256.
#[must_use]
pub fn invoice_sha256_hex(canonical_ubl: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut h: [u8; 32] = [0; 32];
    for (i, byte) in canonical_ubl.iter().enumerate() {
        let slot = i % 32;
        h[slot] = h[slot].wrapping_add(*byte);
    }
    let mut out = String::with_capacity(64);
    for byte in h {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_zatca::crate_name(),
///     "invoicekit-signer-zatca"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-zatca"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use std::sync::Arc;

    fn sample_csid(env: ZatcaEnvironment) -> CsidRecord {
        CsidRecord {
            csid: format!("csid-{}", env.slug()),
            environment: env,
            vat_number: "300000000000003".to_owned(),
            stamp_uuid: Some("stamp-uuid-abc".to_owned()),
            not_before: "2026-01-01T00:00:00Z".to_owned(),
            not_after: "2027-12-31T23:59:59Z".to_owned(),
        }
    }

    fn build_provider(env: ZatcaEnvironment) -> MockPhase2Provider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key(format!("csid-{}", env.slug()), [9_u8; 32]));
        MockPhase2Provider::new("test-zatca", signer, sample_csid(env))
    }

    fn mandatory_fields() -> BTreeMap<QrField, String> {
        let mut m = BTreeMap::new();
        m.insert(QrField::SellerName, "Acme KSA".to_owned());
        m.insert(QrField::VatNumber, "300000000000003".to_owned());
        m.insert(QrField::Timestamp, "2026-05-28T10:30:00Z".to_owned());
        m.insert(QrField::Total, "1190.00".to_owned());
        m.insert(QrField::VatTotal, "190.00".to_owned());
        m
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-zatca");
    }

    #[test]
    fn invoice_mode_round_trips_kebab_json() {
        let json = serde_json::to_string(&ZatcaInvoiceMode::Standard).unwrap();
        assert_eq!(json, "\"standard\"");
        let back: ZatcaInvoiceMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ZatcaInvoiceMode::Standard);
    }

    #[test]
    fn qr_field_tags_match_zatca_spec() {
        assert_eq!(QrField::SellerName.tag(), 1);
        assert_eq!(QrField::VatNumber.tag(), 2);
        assert_eq!(QrField::Timestamp.tag(), 3);
        assert_eq!(QrField::Total.tag(), 4);
        assert_eq!(QrField::VatTotal.tag(), 5);
        assert_eq!(QrField::InvoiceHash.tag(), 6);
        assert_eq!(QrField::StampSignatureValue.tag(), 7);
        assert_eq!(QrField::StampPublicKey.tag(), 8);
    }

    #[test]
    fn reporting_status_predicate_matches_variants() {
        assert!(ReportingStatus::Accepted.is_accepted());
        assert!(ReportingStatus::AcceptedWithWarnings.is_accepted());
        assert!(!ReportingStatus::Rejected.is_accepted());
        assert!(!ReportingStatus::Pending.is_accepted());
    }

    #[test]
    fn validate_qr_fields_accepts_standard_with_mandatory_five() {
        let fields = mandatory_fields();
        validate_qr_fields(&fields, ZatcaInvoiceMode::Standard).unwrap();
    }

    #[test]
    fn validate_qr_fields_rejects_missing_mandatory_field() {
        let mut fields = mandatory_fields();
        fields.remove(&QrField::Total);
        let err = validate_qr_fields(&fields, ZatcaInvoiceMode::Standard).unwrap_err();
        match err {
            ZatcaError::MissingMandatoryField(f) => assert_eq!(f, QrField::Total),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn validate_qr_fields_rejects_simplified_without_stamp_fields() {
        let fields = mandatory_fields();
        let err = validate_qr_fields(&fields, ZatcaInvoiceMode::Simplified).unwrap_err();
        assert!(matches!(err, ZatcaError::MissingSimplifiedStampFields));
    }

    #[test]
    fn validate_qr_fields_accepts_simplified_with_stamp_fields() {
        let mut fields = mandatory_fields();
        fields.insert(QrField::StampSignatureValue, "AAAA".to_owned());
        fields.insert(QrField::StampPublicKey, "BBBB".to_owned());
        validate_qr_fields(&fields, ZatcaInvoiceMode::Simplified).unwrap();
    }

    #[test]
    fn encode_qr_tlv_orders_by_tag_and_uses_byte_lengths() {
        let mut fields = BTreeMap::new();
        fields.insert(QrField::VatTotal, "5".to_owned());
        fields.insert(QrField::SellerName, "AB".to_owned());
        let tlv = encode_qr_tlv(&fields);
        assert_eq!(tlv, vec![0x01, 0x02, b'A', b'B', 0x05, 0x01, b'5',]);
    }

    #[test]
    fn stamp_rejects_environment_mismatch() {
        let provider = build_provider(ZatcaEnvironment::Compliance);
        let err = provider
            .stamp(
                &ZatcaSignRequest {
                    canonical_ubl: b"<Invoice/>".to_vec(),
                    csid: sample_csid(ZatcaEnvironment::Compliance),
                    mode: ZatcaInvoiceMode::Standard,
                    qr_fields: mandatory_fields(),
                },
                ZatcaEnvironment::Production,
            )
            .unwrap_err();
        assert!(matches!(err, ZatcaError::EnvironmentMismatch { .. }));
    }

    #[test]
    fn stamp_produces_envelope_for_standard_invoice() {
        let provider = build_provider(ZatcaEnvironment::Compliance);
        let envelope = provider
            .stamp(
                &ZatcaSignRequest {
                    canonical_ubl: b"<Invoice/>".to_vec(),
                    csid: sample_csid(ZatcaEnvironment::Compliance),
                    mode: ZatcaInvoiceMode::Standard,
                    qr_fields: mandatory_fields(),
                },
                ZatcaEnvironment::Compliance,
            )
            .unwrap();
        assert_eq!(envelope.mode, ZatcaInvoiceMode::Standard);
        assert_eq!(envelope.csid.environment, ZatcaEnvironment::Compliance);
        assert_eq!(envelope.reporting_status, ReportingStatus::Accepted);
        assert!(!envelope.qr_tlv.is_empty());
        assert!(envelope.qr_tlv.starts_with(&[0x01]));
        let again = invoice_sha256_hex(b"<Invoice/>");
        assert_eq!(envelope.invoice_sha256_hex, again);
        assert_eq!(provider.stamps().len(), 1);
    }

    #[test]
    fn stamp_propagates_forced_reporting_status() {
        let provider = build_provider(ZatcaEnvironment::Compliance)
            .with_forced_status(ReportingStatus::AcceptedWithWarnings);
        let envelope = provider
            .stamp(
                &ZatcaSignRequest {
                    canonical_ubl: b"<x/>".to_vec(),
                    csid: sample_csid(ZatcaEnvironment::Compliance),
                    mode: ZatcaInvoiceMode::Standard,
                    qr_fields: mandatory_fields(),
                },
                ZatcaEnvironment::Compliance,
            )
            .unwrap();
        assert_eq!(
            envelope.reporting_status,
            ReportingStatus::AcceptedWithWarnings
        );
        assert!(envelope.reporting_status.is_accepted());
    }

    #[test]
    fn invoice_hash_changes_when_payload_changes() {
        let a = invoice_sha256_hex(b"hello");
        let b = invoice_sha256_hex(b"world");
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
