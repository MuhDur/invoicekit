// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-045 projection from the core [`CommercialDocument`] IR to
//! German XRechnung 3.x — a Peppol BIS Billing 3.0 / EN 16931
//! Core Invoice Usage Specification (CIUS) for the German B2G
//! channel.
//!
//! XRechnung 3.x layers three things on top of EN 16931 / UBL
//! Invoice:
//!
//!   1. A fixed [`XRECHNUNG_3_CUSTOMIZATION_ID`] `CustomizationID`
//!      URN that `KoSIT`'s validator dispatches its CIUS-DE
//!      scenarios from.
//!   2. A fixed [`XRECHNUNG_PROFILE_ID`] `ProfileID` URN.
//!   3. A mandatory `cbc:BuyerReference` carrying the
//!      `Leitweg-ID` for invoices destined for German federal,
//!      state, or municipal authorities (`BR-DE-15`).
//!
//! This crate is intentionally tiny: the UBL serializer in
//! `invoicekit-format-ubl` already emits `EN 16931`-compliant
//! XML; the projection just injects three `cbc:*` overrides via
//! the `urn:invoicekit:ubl:2.1:document-fields` extension's
//! `top_level` payload key before delegating to
//! [`invoicekit_format_ubl::to_xml`].
//!
//! Validating the projected XML against `KoSIT` requires the
//! `KoSIT` `validator-configuration-xrechnung` scenarios bundle
//! on disk (env var `INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS`, see
//! `services/validator-kosit` and the T-7psv `KositReport`
//! reflection wrapper). Tests under `tests/kosit_parity.rs`
//! self-skip when the bundle is absent, so the crate's unit
//! gates always run; only the integration parity gate needs
//! the bundle.

use invoicekit_format_ubl::{to_xml, UblError, UBL_DOCUMENT_FIELDS_EXTENSION_URN};
use invoicekit_ir::{CommercialDocument, IrError, JurisdictionExtension};
use serde_json::{json, Value};
use thiserror::Error;

/// BR-DE-* Schematron rule coverage matrix shipped alongside this
/// projection. See [`coverage::BR_DE_COVERAGE`] for the row data.
pub mod coverage;

/// CIUS-DE `CustomizationID` published by `KoSIT` for XRechnung 3.x.
pub const XRECHNUNG_3_CUSTOMIZATION_ID: &str =
    "urn:cen.eu:en16931:2017#compliant#urn:xoev-de:kosit:standard:xrechnung_3.0";

/// Peppol BIS Billing 3.0 transaction `ProfileID` that XRechnung
/// reuses (BT-23).
pub const XRECHNUNG_PROFILE_ID: &str = "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0";

/// XRechnung 3.x projection error.
#[derive(Debug, Error)]
pub enum XRechnungError {
    /// Underlying UBL serializer rejected the projection.
    #[error("UBL serialization error: {0}")]
    Ubl(#[from] UblError),
    /// IR refused to build the projected document.
    #[error("IR error: {0}")]
    Ir(#[from] IrError),
    /// Mandatory Leitweg-ID was missing for a B2G projection.
    #[error("Leitweg-ID required for B2G XRechnung but none provided")]
    MissingLeitwegId,
    /// Leitweg-ID failed the BR-DE-15 cross-reference shape check.
    #[error("invalid Leitweg-ID `{0}`: {1}")]
    InvalidLeitwegId(String, &'static str),
}

/// Options for the XRechnung 3.x projection.
#[derive(Clone, Debug, Default)]
pub struct XRechnungOptions {
    /// Leitweg-ID (BT-10 / `cbc:BuyerReference`) for B2G recipients.
    ///
    /// When `None` the projection assumes a B2B XRechnung and the
    /// Buyer Reference falls through to whatever the upstream
    /// document already carries.
    pub leitweg_id: Option<String>,
}

/// Project a [`CommercialDocument`] to XRechnung 3.x UBL XML.
///
/// # Errors
///
/// Returns an [`XRechnungError`] when the underlying UBL
/// serializer fails, when the IR rejects the projected extension
/// payload, or when an invalid Leitweg-ID is supplied.
pub fn to_xrechnung_3_x_xml(
    document: &CommercialDocument,
    options: &XRechnungOptions,
) -> Result<String, XRechnungError> {
    let leitweg_id = options
        .leitweg_id
        .as_deref()
        .map(validate_leitweg_id)
        .transpose()?;
    let projected = project_for_xrechnung_3_x(document, leitweg_id)?;
    let xml = to_xml(&projected)?;
    Ok(xml)
}

/// Project the document by injecting top-level `CustomizationID` /
/// `ProfileID` / `BuyerReference` overrides into the UBL
/// document-fields extension. The original input document is not
/// mutated.
fn project_for_xrechnung_3_x(
    document: &CommercialDocument,
    leitweg_id: Option<&str>,
) -> Result<CommercialDocument, XRechnungError> {
    let mut top_level = collect_existing_top_level(document);
    upsert_top_level(
        &mut top_level,
        "cbc:CustomizationID",
        &format!("<cbc:CustomizationID>{XRECHNUNG_3_CUSTOMIZATION_ID}</cbc:CustomizationID>"),
    );
    upsert_top_level(
        &mut top_level,
        "cbc:ProfileID",
        &format!("<cbc:ProfileID>{XRECHNUNG_PROFILE_ID}</cbc:ProfileID>"),
    );
    // Take the rest of the existing document-fields payload as is.
    let mut payload = collect_existing_document_fields_payload(document);
    payload.insert("top_level".to_owned(), Value::Array(top_level));
    if let Some(id) = leitweg_id {
        // BuyerReference is written from the `buyer_reference`
        // document_fields key, not from the `top_level` overrides
        // path — so we update it in-place.
        payload.insert("buyer_reference".to_owned(), Value::String(id.to_owned()));
    }

    let new_extension = JurisdictionExtension::new(
        UBL_DOCUMENT_FIELDS_EXTENSION_URN,
        Value::Object(payload.into_iter().collect()),
    )?;

    let mut projected = document.clone();
    projected
        .extensions
        .retain(|ext| ext.urn != UBL_DOCUMENT_FIELDS_EXTENSION_URN);
    projected.extensions.push(new_extension);
    Ok(projected)
}

fn collect_existing_top_level(document: &CommercialDocument) -> Vec<Value> {
    document
        .extensions
        .iter()
        .find(|ext| ext.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|ext| ext.payload.get("top_level"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn collect_existing_document_fields_payload(
    document: &CommercialDocument,
) -> std::collections::BTreeMap<String, Value> {
    document
        .extensions
        .iter()
        .find(|ext| ext.urn == UBL_DOCUMENT_FIELDS_EXTENSION_URN)
        .and_then(|ext| ext.payload.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn upsert_top_level(top_level: &mut Vec<Value>, element: &str, xml: &str) {
    let payload = json!({"element": element, "xml": xml});
    if let Some(existing) = top_level
        .iter_mut()
        .find(|entry| entry.get("element").and_then(Value::as_str) == Some(element))
    {
        *existing = payload;
    } else {
        top_level.push(payload);
    }
}

/// Validate a Leitweg-ID shape per the FRA-DE rules (BR-DE-15).
///
/// The Leitweg-ID format is `<grobadressierung>-<feinadressierung>-<pruefziffer>`
/// where `grobadressierung` is 2-12 characters, `feinadressierung`
/// is up to 30 characters, and `pruefziffer` is 2 numeric digits.
/// This implementation enforces a non-empty string with at least
/// one segment and ASCII characters only; deeper validation
/// (checksum digit) is left to the KoSIT validator.
fn validate_leitweg_id(value: &str) -> Result<&str, XRechnungError> {
    if value.is_empty() {
        return Err(XRechnungError::MissingLeitwegId);
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(XRechnungError::InvalidLeitwegId(
            value.to_owned(),
            "must be ASCII alphanumeric with `-` separators",
        ));
    }
    Ok(value)
}

/// Crate name advertised in operator logs.
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-profile-xrechnung"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customization_id_constants_match_kosit() {
        // Frozen against the upstream XRechnung 3.x KoSIT
        // configuration so a drift here is caught here, not
        // surprises us in the parity harness.
        assert_eq!(
            XRECHNUNG_3_CUSTOMIZATION_ID,
            "urn:cen.eu:en16931:2017#compliant#urn:xoev-de:kosit:standard:xrechnung_3.0"
        );
        assert_eq!(
            XRECHNUNG_PROFILE_ID,
            "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0"
        );
    }

    #[test]
    fn validate_leitweg_id_accepts_canonical_examples() {
        // Examples taken from the official KoSIT documentation.
        assert!(validate_leitweg_id("04011000-1234512345-06").is_ok());
        assert!(validate_leitweg_id("99661-DEMO-31").is_ok());
    }

    #[test]
    fn validate_leitweg_id_rejects_empty() {
        assert!(matches!(
            validate_leitweg_id(""),
            Err(XRechnungError::MissingLeitwegId)
        ));
    }

    #[test]
    fn validate_leitweg_id_rejects_non_ascii() {
        assert!(matches!(
            validate_leitweg_id("Leitwég-1"),
            Err(XRechnungError::InvalidLeitwegId(_, _))
        ));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-profile-xrechnung");
    }
}
