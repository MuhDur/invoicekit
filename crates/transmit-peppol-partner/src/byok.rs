// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Bring-your-own-key bridge: build a [`PartnerConfig`] from a
//! customer-supplied [`PeppolCredentials`] bundle.
//!
//! The vendor slug travels in the credentials' `labels`
//! (`partner.vendor = "storecove"`); the participant id's value
//! becomes the legal-entity id. SML mode `Test` and `Acceptance`
//! both map to `sandbox = true`; `Production` maps to
//! `sandbox = false`.

use invoicekit_transmit_peppol_byok::{PeppolCredentials, SmlMode, TransportKind};
use thiserror::Error;

use crate::{PartnerConfig, PartnerVendor};

/// Errors raised translating BYOK credentials into a
/// [`PartnerConfig`].
#[derive(Debug, Error)]
pub enum ByokBridgeError {
    /// Credentials carry a `transport` other than `partner`.
    #[error("byok credentials have transport `{0:?}`, partner adapter requires Partner")]
    WrongTransport(TransportKind),
    /// `labels["partner.vendor"]` was missing.
    #[error("byok credentials must set labels[`partner.vendor`] (one of: storecove, ecosio, b2b-router)")]
    MissingVendorLabel,
    /// `labels["partner.vendor"]` was an unknown vendor slug.
    #[error("byok credentials labels[`partner.vendor`] = `{0}` is not a known vendor")]
    UnknownVendor(String),
}

/// Build a [`PartnerConfig`] from BYOK credentials.
///
/// # Errors
///
/// Returns [`ByokBridgeError`] when the transport is wrong, the
/// vendor label is missing, or the vendor slug is unknown.
pub fn partner_config_from_byok(
    creds: &PeppolCredentials,
) -> Result<PartnerConfig, ByokBridgeError> {
    if creds.transport != TransportKind::Partner {
        return Err(ByokBridgeError::WrongTransport(creds.transport));
    }
    let vendor_slug = creds
        .labels
        .get("partner.vendor")
        .ok_or(ByokBridgeError::MissingVendorLabel)?;
    let vendor = PartnerVendor::from_slug(vendor_slug)
        .map_err(|_| ByokBridgeError::UnknownVendor(vendor_slug.clone()))?;
    Ok(PartnerConfig {
        vendor,
        api_base: creds.endpoint_url.clone(),
        legal_entity_id: creds.participant_id.value.clone(),
        sandbox: !matches!(creds.sml_mode, SmlMode::Production),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_transmit_peppol_byok::ParticipantId;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn byok(transport: TransportKind, sml: SmlMode, vendor: Option<&str>) -> PeppolCredentials {
        let mut labels = BTreeMap::new();
        if let Some(v) = vendor {
            labels.insert("partner.vendor".to_owned(), v.to_owned());
        }
        PeppolCredentials {
            participant_id: ParticipantId {
                scheme: "iso6523-actorid-upis".to_owned(),
                value: "0192:991825827".to_owned(),
            },
            cert_pem_path: PathBuf::from("/etc/peppol/cert.pem"),
            key_pem_path: PathBuf::from("/etc/peppol/key.pem"),
            key_passphrase_env: None,
            endpoint_url: "https://api.storecove.com".to_owned(),
            sml_mode: sml,
            transport,
            labels,
        }
    }

    #[test]
    fn happy_path_storecove_test_sml_maps_to_sandbox() {
        let creds = byok(TransportKind::Partner, SmlMode::Test, Some("storecove"));
        let cfg = partner_config_from_byok(&creds).unwrap();
        assert_eq!(cfg.vendor, PartnerVendor::Storecove);
        assert_eq!(cfg.api_base, "https://api.storecove.com");
        assert_eq!(cfg.legal_entity_id, "0192:991825827");
        assert!(cfg.sandbox);
    }

    #[test]
    fn production_sml_maps_to_sandbox_false() {
        let creds = byok(TransportKind::Partner, SmlMode::Production, Some("ecosio"));
        let cfg = partner_config_from_byok(&creds).unwrap();
        assert!(!cfg.sandbox);
    }

    #[test]
    fn wrong_transport_is_rejected() {
        let creds = byok(TransportKind::Phase4, SmlMode::Test, Some("storecove"));
        let err = partner_config_from_byok(&creds).unwrap_err();
        assert!(matches!(err, ByokBridgeError::WrongTransport(_)));
    }

    #[test]
    fn missing_vendor_label_is_rejected() {
        let creds = byok(TransportKind::Partner, SmlMode::Test, None);
        let err = partner_config_from_byok(&creds).unwrap_err();
        assert!(matches!(err, ByokBridgeError::MissingVendorLabel));
    }

    #[test]
    fn unknown_vendor_slug_is_rejected() {
        let creds = byok(TransportKind::Partner, SmlMode::Test, Some("not-a-vendor"));
        let err = partner_config_from_byok(&creds).unwrap_err();
        assert!(matches!(err, ByokBridgeError::UnknownVendor(_)));
    }
}
