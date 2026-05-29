// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Spain **VeriFactu** anti-fraud reporting adapter.
//!
//! VeriFactu is Spain's invoice-correctness regime under
//! Real Decreto 1007/2023. Issuers transmit each invoice
//! (or batches) to the AEAT (Agencia Estatal de
//! Administración Tributaria) along with a **hash chain**
//! pointing at the previous invoice's hash — so the AEAT can
//! re-check that no invoice was deleted or back-dated. The
//! printed/PDF invoice carries a QR code linking back to the
//! AEAT for buyer-side verification.
//!
//! Two operating modes:
//!
//! - **VeriFactu** (real-time / continuous reporting). Every
//!   invoice is reported to the AEAT immediately on issue.
//! - **No-VeriFactu** (also "Sistemas Informáticos de
//!   Facturación SIF"). The system records invoices locally
//!   with a hash chain and is subject to AEAT inspection on
//!   demand. The hash chain shape is the same; only the
//!   transport differs.
//!
//! This crate ships the typed surface and a deterministic
//! [`MockVeriFactuProvider`]. The live SOAP / REST AEAT
//! integration lands in a follow-up `report-es-verifactu-http`
//! crate behind a feature flag so operators who only need the
//! substrate don't pull in the HTTP stack.
//!
//! Reference reading: AEAT VeriFactu portal at
//! <https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu.html>.

#![allow(clippy::doc_markdown)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment selector for the AEAT transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VeriFactuEnvironment {
    /// AEAT preproducción / sandbox tier.
    Sandbox,
    /// Production.
    Production,
}

/// Operating mode per RD 1007/2023.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VeriFactuMode {
    /// Real-time reporting; every invoice is transmitted to
    /// the AEAT immediately on issue.
    VeriFactu,
    /// Local hash-chain mode (SIF). Records are inspected on
    /// demand; transport is the same shape.
    NoVeriFactu,
}

/// What the operator passes in to
/// [`VeriFactuProvider::register_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VeriFactuRegisterRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: VeriFactuEnvironment,
    /// Operating mode.
    pub mode: VeriFactuMode,
    /// Issuer's Spanish NIF / DNI / NIE (always 9 chars).
    pub issuer_nif: String,
    /// Series + invoice number ("F2026/0007" style — the
    /// AEAT enforces a single canonical string).
    pub invoice_number: String,
    /// RFC-3339 UTC issuance timestamp the AEAT pins for the
    /// hash chain.
    pub issued_at: String,
    /// SHA-256 hex (lowercase) of the previous invoice in the
    /// chain. `None` only for the first invoice the issuer
    /// reports (the AEAT records it as the chain root).
    pub previous_hash_hex: Option<String>,
    /// Canonical XML payload the AEAT expects. The provider
    /// computes its own SHA-256 over this for the chain.
    pub invoice_xml: Vec<u8>,
}

/// AEAT per-invoice verdict after `register_invoice`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VeriFactuStatus {
    /// Successfully recorded.
    Accepted,
    /// Recorded but a warning attached (e.g. customer NIF not
    /// found in the AEAT census).
    AcceptedWithWarnings,
    /// Refused; engine should not consider the invoice
    /// VeriFactu-valid until resubmit succeeds.
    Rejected,
}

/// What [`VeriFactuProvider::register_invoice`] returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VeriFactuRegisterEnvelope {
    /// AEAT verdict.
    pub status: VeriFactuStatus,
    /// SHA-256 hex (lowercase) the AEAT recorded for this
    /// invoice. Engines persist this as the
    /// `previous_hash_hex` for the next invoice.
    pub recorded_hash_hex: String,
    /// CSV (Código Seguro de Verificación) the AEAT assigns —
    /// embedded in the printed-invoice QR alongside the
    /// invoice number.
    pub csv: String,
    /// Optional warning / rejection text the AEAT returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC-3339 UTC timestamp the AEAT recorded.
    pub recorded_at: String,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum VeriFactuError {
    /// The supplied invoice XML didn't parse / wasn't the
    /// AEAT-expected envelope.
    #[error("invoice xml rejected: {0}")]
    BadXml(String),
    /// The issuer NIF / DNI / NIE wasn't 9 chars.
    #[error("invalid issuer NIF: {0}")]
    BadNif(String),
    /// The supplied previous-hash hex wasn't 64 lowercase hex
    /// chars (SHA-256).
    #[error("invalid previous hash: {0}")]
    BadPreviousHash(String),
    /// HTTP / TLS / DNS failure talking to the AEAT.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The Spain VeriFactu registration surface. Real AEAT
/// SOAP/REST integrations satisfy this trait; the mock below
/// is what tests + cassette-replay use.
pub trait VeriFactuProvider: Send + Sync {
    /// Register one invoice with the AEAT (or, in
    /// `NoVeriFactu` mode, against the local hash chain). The
    /// provider:
    ///
    /// 1. validates `issuer_nif` shape,
    /// 2. validates `previous_hash_hex` shape when supplied,
    /// 3. computes SHA-256 over the invoice payload,
    /// 4. transmits to the AEAT endpoint chosen by
    ///    `environment` (the no-veri mode short-circuits),
    /// 5. returns the AEAT-recorded hash + CSV.
    ///
    /// # Errors
    ///
    /// Returns [`VeriFactuError`] when validation fails
    /// before the wire or transport fails on the wire. The
    /// AEAT-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `VeriFactuStatus::Rejected` inside
    /// `VeriFactuRegisterEnvelope` so the engine persists the
    /// rejection alongside its audit trail.
    fn register_invoice(
        &self,
        request: &VeriFactuRegisterRequest,
    ) -> Result<VeriFactuRegisterEnvelope, VeriFactuError>;
}

/// Deterministic mock provider.
///
/// Returns [`VeriFactuStatus::Accepted`] with a synthesised
/// `recorded_hash_hex` (BLAKE3-derived but presented as
/// SHA-256-shaped hex) + `csv`, so cassette-replay tests stay
/// byte-identical across runs.
///
/// Use [`MockVeriFactuProvider::with_forced_status`] to drive
/// the genuine AEAT *authority verdicts* the trait contract
/// promises but the happy-path mock never reaches:
/// [`VeriFactuStatus::AcceptedWithWarnings`] (`AceptadoConErrores`)
/// and [`VeriFactuStatus::Rejected`] (`Incorrecto`). Those are
/// `Ok` envelopes, never `Err` — `Err` stays reserved for
/// pre-wire shape failures and transport faults.
pub struct MockVeriFactuProvider {
    fixed_recorded_at: String,
    next_serial: std::sync::Mutex<u64>,
    forced_status: VeriFactuStatus,
    forced_message: Option<String>,
}

impl MockVeriFactuProvider {
    /// Build a mock with deterministic timestamps + serial CSV.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_recorded_at("2026-07-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp.
    #[must_use]
    pub fn with_fixed_recorded_at(recorded_at: impl Into<String>) -> Self {
        Self {
            fixed_recorded_at: recorded_at.into(),
            next_serial: std::sync::Mutex::new(1),
            forced_status: VeriFactuStatus::Accepted,
            forced_message: None,
        }
    }

    /// Force every registration to return a specific AEAT
    /// verdict plus an optional `message`. This exercises the
    /// [`VeriFactuStatus::AcceptedWithWarnings`] and
    /// [`VeriFactuStatus::Rejected`] branches (the AEAT
    /// `AceptadoConErrores` / `Incorrecto` `EstadoRegistro`
    /// values) the always-`Accepted` happy path cannot reach.
    ///
    /// The shape validators (`issuer_nif`, `previous_hash_hex`,
    /// non-empty payload) still run first — a forced verdict
    /// never bypasses pre-wire `Err` refusal.
    #[must_use]
    pub fn with_forced_status(
        mut self,
        status: VeriFactuStatus,
        message: Option<String>,
    ) -> Self {
        self.forced_status = status;
        self.forced_message = message;
        self
    }
}

impl Default for MockVeriFactuProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl VeriFactuProvider for MockVeriFactuProvider {
    fn register_invoice(
        &self,
        request: &VeriFactuRegisterRequest,
    ) -> Result<VeriFactuRegisterEnvelope, VeriFactuError> {
        validate_nif(&request.issuer_nif)?;
        if let Some(prev) = &request.previous_hash_hex {
            validate_sha256_hex(prev)?;
        }
        if request.invoice_xml.is_empty() {
            return Err(VeriFactuError::BadXml("payload is empty".to_owned()));
        }
        // Mock "SHA-256" of the payload: deterministic
        // expansion of the byte length + a few bytes of
        // payload material, padded to 64 hex chars. Not a real
        // hash; the live impl computes a real SHA-256.
        let mut digest = String::with_capacity(64);
        let _ = write!(digest, "{:0>16x}", request.invoice_xml.len() as u64);
        for byte in request.invoice_xml.iter().take(24) {
            let _ = write!(digest, "{byte:02x}");
        }
        while digest.len() < 64 {
            digest.push('0');
        }
        digest.truncate(64);

        let serial = {
            let mut guard = self.next_serial.lock().expect("serial mutex poisoned");
            let v = *guard;
            *guard += 1;
            v
        };
        Ok(VeriFactuRegisterEnvelope {
            status: self.forced_status,
            recorded_hash_hex: digest,
            csv: format!("MOCK-CSV-{serial:08}"),
            message: self.forced_message.clone(),
            recorded_at: self.fixed_recorded_at.clone(),
        })
    }
}

/// Validate that a NIF / DNI / NIE is 9 ASCII alphanumeric chars.
///
/// Real shape: digits + final letter for NIF/NIE, or leading
/// letter for NIE/CIF. The Spanish modulo-23 checksum is a
/// separate concern; this helper only catches obviously-wrong
/// shapes before the wire.
///
/// # Errors
///
/// Returns [`VeriFactuError::BadNif`] when the input isn't 9
/// ASCII alphanumeric characters.
pub fn validate_nif(nif: &str) -> Result<(), VeriFactuError> {
    if nif.len() == 9 && nif.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(VeriFactuError::BadNif(format!(
            "NIF/DNI/NIE must be 9 ASCII alphanumeric chars, got {nif:?}"
        )))
    }
}

/// Validate that a hex string matches SHA-256 wire shape.
///
/// Accepts exactly 64 lowercase hex characters.
///
/// # Errors
///
/// Returns [`VeriFactuError::BadPreviousHash`] when the input
/// isn't 64 lowercase hex chars.
pub fn validate_sha256_hex(hex: &str) -> Result<(), VeriFactuError> {
    if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(VeriFactuError::BadPreviousHash(format!(
            "expected 64 lowercase hex chars, got {hex:?}"
        )))
    }
}

/// Build the AEAT VeriFactu QR-code payload string.
///
/// Per AEAT specification chapter 4 the QR encodes a
/// verification URL like
/// `https://prewww1.aeat.es/wlpl/TIKE-CONT/ValidarQR?nif={NIF}&numserie={INV}&fecha={ISO}&importe={GROSS}`.
///
/// The crate doesn't fetch the GROSS — that's an engine-side
/// concern. The function instead exposes a typed builder so
/// the engine glues the fields in without hand-formatting.
#[must_use]
pub fn qr_payload(
    portal_base: &str,
    issuer_nif: &str,
    invoice_number: &str,
    issued_at_yyyymmdd: &str,
    gross_total: &str,
) -> String {
    format!(
        "{portal_base}/ValidarQR?nif={issuer_nif}&numserie={invoice_number}&fecha={issued_at_yyyymmdd}&importe={gross_total}"
    )
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_es_verifactu::crate_name(),
///     "invoicekit-report-es-verifactu"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-es-verifactu"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> VeriFactuRegisterRequest {
        VeriFactuRegisterRequest {
            tenant_id: "tenant-es-test".to_owned(),
            environment: VeriFactuEnvironment::Sandbox,
            mode: VeriFactuMode::VeriFactu,
            issuer_nif: "A12345678".to_owned(),
            invoice_number: "F2026/0007".to_owned(),
            issued_at: "2026-07-01T10:00:00Z".to_owned(),
            previous_hash_hex: None,
            invoice_xml: b"<Invoice/>".to_vec(),
        }
    }

    #[test]
    fn register_invoice_returns_accepted_with_hash_and_csv() {
        let p = MockVeriFactuProvider::default();
        let env = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, VeriFactuStatus::Accepted);
        assert_eq!(env.recorded_hash_hex.len(), 64);
        assert!(env.csv.starts_with("MOCK-CSV-"));
        assert_eq!(env.recorded_at, "2026-07-01T00:00:00Z");
    }

    #[test]
    fn register_invoice_serial_increments_per_provider() {
        let p = MockVeriFactuProvider::default();
        let env1 = p.register_invoice(&sample_request()).unwrap();
        let env2 = p.register_invoice(&sample_request()).unwrap();
        assert_ne!(env1.csv, env2.csv);
    }

    #[test]
    fn register_invoice_accepts_chained_previous_hash() {
        let p = MockVeriFactuProvider::default();
        let mut req = sample_request();
        req.previous_hash_hex =
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned());
        let env = p.register_invoice(&req).unwrap();
        assert_eq!(env.status, VeriFactuStatus::Accepted);
    }

    #[test]
    fn register_invoice_rejects_bad_previous_hash_shape() {
        let p = MockVeriFactuProvider::default();
        let mut req = sample_request();
        req.previous_hash_hex = Some("too-short".to_owned());
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, VeriFactuError::BadPreviousHash(_)));
    }

    #[test]
    fn register_invoice_rejects_bad_nif() {
        let p = MockVeriFactuProvider::default();
        let mut req = sample_request();
        req.issuer_nif = "A123".to_owned();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, VeriFactuError::BadNif(_)));
    }

    #[test]
    fn register_invoice_rejects_empty_xml() {
        let p = MockVeriFactuProvider::default();
        let mut req = sample_request();
        req.invoice_xml.clear();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, VeriFactuError::BadXml(_)));
    }

    #[test]
    fn validate_nif_accepts_9_alphanumeric_chars() {
        assert!(validate_nif("12345678Z").is_ok());
        assert!(validate_nif("X1234567L").is_ok());
        assert!(validate_nif("A12345678").is_ok());
    }

    #[test]
    fn validate_nif_rejects_wrong_length() {
        assert!(validate_nif("123456789012").is_err());
        assert!(validate_nif("12345").is_err());
    }

    #[test]
    fn validate_sha256_hex_accepts_valid_digest() {
        assert!(validate_sha256_hex(
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        )
        .is_ok());
    }

    #[test]
    fn validate_sha256_hex_rejects_uppercase() {
        assert!(validate_sha256_hex(
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
        )
        .is_err());
    }

    #[test]
    fn qr_payload_glues_fields_into_aeat_url() {
        let qr = qr_payload(
            "https://prewww1.aeat.es/wlpl/TIKE-CONT",
            "A12345678",
            "F2026/0007",
            "2026-07-01",
            "121.00",
        );
        assert!(qr.contains("nif=A12345678"));
        assert!(qr.contains("numserie=F2026/0007"));
        assert!(qr.contains("fecha=2026-07-01"));
        assert!(qr.contains("importe=121.00"));
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = VeriFactuRegisterEnvelope {
            status: VeriFactuStatus::AcceptedWithWarnings,
            recorded_hash_hex: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                .to_owned(),
            csv: "MOCK-CSV-00000005".to_owned(),
            message: Some("buyer NIF not found in census".to_owned()),
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: VeriFactuRegisterEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }

    #[test]
    fn forced_status_surfaces_rejected_as_ok_envelope() {
        // AEAT `Incorrecto` (rejection) is a verdict, NOT an `Err` — the
        // engine persists it alongside its audit trail. Shape validation
        // still runs first (the request below is well-formed).
        let p = MockVeriFactuProvider::default().with_forced_status(
            VeriFactuStatus::Rejected,
            Some("1109 El NIF del destinatario no esta identificado en el censo".to_owned()),
        );
        let env = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, VeriFactuStatus::Rejected);
        assert!(env.message.as_deref().unwrap().starts_with("1109"));
    }

    #[test]
    fn forced_status_surfaces_accepted_with_warnings() {
        let p = MockVeriFactuProvider::default().with_forced_status(
            VeriFactuStatus::AcceptedWithWarnings,
            Some("AceptadoConErrores".to_owned()),
        );
        let env = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, VeriFactuStatus::AcceptedWithWarnings);
        // Even a warning verdict still records a chain link + CSV.
        assert_eq!(env.recorded_hash_hex.len(), 64);
        assert!(env.csv.starts_with("MOCK-CSV-"));
    }

    #[test]
    fn forced_status_still_refuses_bad_shapes_before_the_wire() {
        // A forced verdict must never bypass pre-wire shape `Err` refusal.
        let p = MockVeriFactuProvider::default()
            .with_forced_status(VeriFactuStatus::Rejected, None);
        let mut req = sample_request();
        req.issuer_nif = "BAD".to_owned();
        assert!(matches!(
            p.register_invoice(&req).unwrap_err(),
            VeriFactuError::BadNif(_)
        ));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-report-es-verifactu");
    }
}
