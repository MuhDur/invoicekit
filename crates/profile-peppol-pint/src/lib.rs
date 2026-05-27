// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-043 projection from the core [`CommercialDocument`] IR to
//! Peppol PINT — the `OpenPeppol` international UBL profile
//! family with one `CustomizationID` URN per country authority.
//!
//! PINT profiles share the EN 16931 / UBL Invoice backbone and
//! differ by:
//!
//!   * `cbc:CustomizationID` — country authority's PINT URN.
//!   * `cbc:ProfileID` — country authority's transaction URN.
//!
//! The projection takes a [`PintCountry`] enum that selects the
//! URN pair and delegates serialization to `format-ubl`.

use invoicekit_format_ubl::{to_xml, UblError, UBL_DOCUMENT_FIELDS_EXTENSION_URN};
use invoicekit_ir::{CommercialDocument, IrError, JurisdictionExtension};
use serde_json::{json, Value};
use thiserror::Error;

/// Peppol PINT country authority for which the projection
/// targets a specific `CustomizationID` / `ProfileID` pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PintCountry {
    /// Australia + New Zealand joint PINT (`aunz`).
    AustraliaNewZealand,
    /// Singapore PINT.
    Singapore,
    /// Japan PINT (Digital Agency authority).
    Japan,
    /// United Arab Emirates PINT (Federal Tax Authority).
    UnitedArabEmirates,
    /// Malaysia PINT (`MyInvois`).
    Malaysia,
}

impl PintCountry {
    /// Lookup by ISO 3166-1 alpha-2 country code. Returns `None`
    /// when the country is not yet a PINT authority.
    #[must_use]
    pub fn from_alpha2(code: &str) -> Option<Self> {
        match code {
            "AU" | "NZ" => Some(Self::AustraliaNewZealand),
            "SG" => Some(Self::Singapore),
            "JP" => Some(Self::Japan),
            "AE" => Some(Self::UnitedArabEmirates),
            "MY" => Some(Self::Malaysia),
            _ => None,
        }
    }

    /// `CustomizationID` URN published by the PINT authority for
    /// this country.
    #[must_use]
    pub const fn customization_id(self) -> &'static str {
        match self {
            Self::AustraliaNewZealand => "urn:peppol:pint:billing-1@aunz-1",
            Self::Singapore => "urn:peppol:pint:billing-1@sg-1",
            Self::Japan => "urn:peppol:pint:billing-1@jp-1",
            Self::UnitedArabEmirates => "urn:peppol:pint:billing-1@ae-1",
            Self::Malaysia => "urn:peppol:pint:billing-1@my-1",
        }
    }

    /// `ProfileID` URN for the billing transaction. PINT uses a
    /// single transaction profile across all member authorities
    /// today, so this is `self`-independent — kept as a method
    /// instead of a constant so the API shape parallels
    /// [`Self::customization_id`].
    #[allow(clippy::unused_self)]
    #[must_use]
    pub const fn profile_id(self) -> &'static str {
        "urn:peppol:bis:billing"
    }
}

/// Peppol PINT projection error.
#[derive(Debug, Error)]
pub enum PintError {
    /// Underlying UBL serializer rejected the projection.
    #[error("UBL serialization error: {0}")]
    Ubl(#[from] UblError),
    /// IR refused to build the projected document.
    #[error("IR error: {0}")]
    Ir(#[from] IrError),
}

/// Project a [`CommercialDocument`] to Peppol PINT UBL XML for
/// the given [`PintCountry`].
///
/// # Errors
///
/// Returns a [`PintError`] when the underlying UBL serializer
/// fails or the IR rejects the projected extension payload.
pub fn to_peppol_pint_xml(
    document: &CommercialDocument,
    country: PintCountry,
) -> Result<String, PintError> {
    let projected = project_for_pint(document, country)?;
    let xml = to_xml(&projected)?;
    Ok(xml)
}

fn project_for_pint(
    document: &CommercialDocument,
    country: PintCountry,
) -> Result<CommercialDocument, PintError> {
    let customization = country.customization_id();
    let profile = country.profile_id();

    let mut top_level = collect_existing_top_level(document);
    upsert_top_level(
        &mut top_level,
        "cbc:CustomizationID",
        &format!("<cbc:CustomizationID>{customization}</cbc:CustomizationID>"),
    );
    upsert_top_level(
        &mut top_level,
        "cbc:ProfileID",
        &format!("<cbc:ProfileID>{profile}</cbc:ProfileID>"),
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
    "invoicekit-profile-peppol-pint"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_alpha2_covers_strict_gate_countries() {
        // The bead's strict gate names AU, NZ, SG, JP, AE.
        assert!(PintCountry::from_alpha2("AU").is_some());
        assert!(PintCountry::from_alpha2("NZ").is_some());
        assert!(PintCountry::from_alpha2("SG").is_some());
        assert!(PintCountry::from_alpha2("JP").is_some());
        assert!(PintCountry::from_alpha2("AE").is_some());
    }

    #[test]
    fn from_alpha2_returns_none_for_unknown_country() {
        assert!(PintCountry::from_alpha2("US").is_none());
        assert!(PintCountry::from_alpha2("ZZ").is_none());
    }

    #[test]
    fn customization_id_format_is_peppol_pint() {
        for country in [
            PintCountry::AustraliaNewZealand,
            PintCountry::Singapore,
            PintCountry::Japan,
            PintCountry::UnitedArabEmirates,
            PintCountry::Malaysia,
        ] {
            let id = country.customization_id();
            assert!(
                id.starts_with("urn:peppol:pint:billing-1@"),
                "{country:?} customization_id does not match PINT pattern: {id}"
            );
        }
    }

    #[test]
    fn profile_id_is_peppol_bis_billing() {
        for country in [
            PintCountry::AustraliaNewZealand,
            PintCountry::Singapore,
            PintCountry::Japan,
            PintCountry::UnitedArabEmirates,
            PintCountry::Malaysia,
        ] {
            assert_eq!(country.profile_id(), "urn:peppol:bis:billing");
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-profile-peppol-pint");
    }
}
