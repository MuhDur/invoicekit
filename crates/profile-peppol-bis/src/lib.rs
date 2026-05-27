// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-042 projection from the core [`CommercialDocument`] IR to
//! Peppol BIS Billing 3.0 — the `OpenPeppol` CIUS / EN 16931
//! UBL profile that backs cross-border B2B/B2G `e-invoicing`
//! through the four-corner Peppol network.
//!
//! Peppol BIS Billing 3.0 layers two things on top of EN 16931 /
//! UBL Invoice:
//!
//!   1. A fixed [`PEPPOL_BIS_3_0_CUSTOMIZATION_ID`] URN that the
//!      phive validator dispatches its `BR-*` / `PEPPOL-*`
//!      Schematron rules from (T-7psv).
//!   2. A fixed [`PEPPOL_BIS_3_0_PROFILE_ID`] URN for the
//!      billing transaction.
//!
//! Unlike XRechnung, Peppol BIS does not mandate a `BuyerReference`
//! Leitweg-ID. The projection therefore touches only
//! `cbc:CustomizationID` + `cbc:ProfileID` and leaves the rest of
//! the upstream document-fields extension intact.
//!
//! Live validation runs through the T-7psv `PhiveReport`
//! reflection wrapper (`services/validator-phive`) which calls
//! `com.helger.phive.peppol.PeppolValidation.initStandard` and
//! exercises the latest Peppol BIS Billing 3.0 invoice rule set
//! (`VID_OPENPEPPOL_INVOICE_UBL_V3`).

use invoicekit_format_ubl::{to_xml, UblError, UBL_DOCUMENT_FIELDS_EXTENSION_URN};
use invoicekit_ir::{CommercialDocument, IrError, JurisdictionExtension};
use serde_json::{json, Value};
use thiserror::Error;

/// `CustomizationID` URN published by `OpenPeppol` for BIS
/// Billing 3.0.
pub const PEPPOL_BIS_3_0_CUSTOMIZATION_ID: &str =
    "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0";

/// `ProfileID` URN for the Peppol BIS Billing 3.0 transaction.
pub const PEPPOL_BIS_3_0_PROFILE_ID: &str = "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0";

/// Peppol BIS Billing 3.0 projection error.
#[derive(Debug, Error)]
pub enum PeppolBisError {
    /// Underlying UBL serializer rejected the projection.
    #[error("UBL serialization error: {0}")]
    Ubl(#[from] UblError),
    /// IR refused to build the projected document.
    #[error("IR error: {0}")]
    Ir(#[from] IrError),
}

/// Project a [`CommercialDocument`] to Peppol BIS Billing 3.0 UBL
/// XML.
///
/// # Errors
///
/// Returns a [`PeppolBisError`] when the underlying UBL serializer
/// fails or the IR rejects the projected extension payload.
pub fn to_peppol_bis_3_0_xml(document: &CommercialDocument) -> Result<String, PeppolBisError> {
    let projected = project_for_peppol_bis(document)?;
    let xml = to_xml(&projected)?;
    Ok(xml)
}

fn project_for_peppol_bis(
    document: &CommercialDocument,
) -> Result<CommercialDocument, PeppolBisError> {
    let mut top_level = collect_existing_top_level(document);
    upsert_top_level(
        &mut top_level,
        "cbc:CustomizationID",
        &format!("<cbc:CustomizationID>{PEPPOL_BIS_3_0_CUSTOMIZATION_ID}</cbc:CustomizationID>"),
    );
    upsert_top_level(
        &mut top_level,
        "cbc:ProfileID",
        &format!("<cbc:ProfileID>{PEPPOL_BIS_3_0_PROFILE_ID}</cbc:ProfileID>"),
    );

    let mut payload = collect_existing_document_fields_payload(document);
    payload.insert("top_level".to_owned(), Value::Array(top_level));

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

/// Crate name advertised in operator logs.
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-profile-peppol-bis"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customization_id_matches_openpeppol_billing_3_0() {
        assert_eq!(
            PEPPOL_BIS_3_0_CUSTOMIZATION_ID,
            "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0"
        );
    }

    #[test]
    fn profile_id_matches_billing_transaction() {
        assert_eq!(
            PEPPOL_BIS_3_0_PROFILE_ID,
            "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0"
        );
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-profile-peppol-bis");
    }
}
