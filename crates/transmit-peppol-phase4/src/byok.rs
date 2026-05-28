// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Bring-your-own-key bridge: build a [`Phase4Config`] from a
//! customer-supplied [`PeppolCredentials`] bundle.
//!
//! The credentials' `endpoint_url` becomes the phase4 sidecar
//! URL. BYOK `SmlMode::Test` and `SmlMode::Acceptance` both map
//! to phase4 `Acceptance` (phase4 has no separate Test mode);
//! `Production` maps through unchanged.

use invoicekit_transmit_peppol_byok::{PeppolCredentials, SmlMode as ByokSml, TransportKind};
use thiserror::Error;

use crate::{Phase4Config, SmlMode};

/// Errors raised translating BYOK credentials into a
/// [`Phase4Config`].
#[derive(Debug, Error)]
pub enum ByokBridgeError {
    /// Credentials carry a `transport` other than `phase4`.
    #[error("byok credentials have transport `{0:?}`, phase4 adapter requires Phase4")]
    WrongTransport(TransportKind),
}

/// Build a [`Phase4Config`] from BYOK credentials.
///
/// # Errors
///
/// Returns [`ByokBridgeError::WrongTransport`] when the
/// credentials' transport is not `Phase4`.
pub fn phase4_config_from_byok(creds: &PeppolCredentials) -> Result<Phase4Config, ByokBridgeError> {
    if creds.transport != TransportKind::Phase4 {
        return Err(ByokBridgeError::WrongTransport(creds.transport));
    }
    let sml_mode = match creds.sml_mode {
        ByokSml::Test | ByokSml::Acceptance => SmlMode::Acceptance,
        ByokSml::Production => SmlMode::Production,
    };
    Ok(Phase4Config {
        sidecar_url: creds.endpoint_url.clone(),
        sml_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_transmit_peppol_byok::ParticipantId;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn byok(transport: TransportKind, sml: ByokSml) -> PeppolCredentials {
        PeppolCredentials {
            participant_id: ParticipantId {
                scheme: "iso6523-actorid-upis".to_owned(),
                value: "0192:991825827".to_owned(),
            },
            cert_pem_path: PathBuf::from("/etc/peppol/cert.pem"),
            key_pem_path: PathBuf::from("/etc/peppol/key.pem"),
            key_passphrase_env: None,
            endpoint_url: "https://phase4.example.com".to_owned(),
            sml_mode: sml,
            transport,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn test_sml_maps_to_acceptance() {
        let cfg = phase4_config_from_byok(&byok(TransportKind::Phase4, ByokSml::Test)).unwrap();
        assert_eq!(cfg.sidecar_url, "https://phase4.example.com");
        assert_eq!(cfg.sml_mode, SmlMode::Acceptance);
    }

    #[test]
    fn acceptance_sml_maps_to_acceptance() {
        let cfg =
            phase4_config_from_byok(&byok(TransportKind::Phase4, ByokSml::Acceptance)).unwrap();
        assert_eq!(cfg.sml_mode, SmlMode::Acceptance);
    }

    #[test]
    fn production_sml_maps_through() {
        let cfg =
            phase4_config_from_byok(&byok(TransportKind::Phase4, ByokSml::Production)).unwrap();
        assert_eq!(cfg.sml_mode, SmlMode::Production);
    }

    #[test]
    fn wrong_transport_is_rejected() {
        let err =
            phase4_config_from_byok(&byok(TransportKind::Partner, ByokSml::Test)).unwrap_err();
        assert!(matches!(err, ByokBridgeError::WrongTransport(_)));
    }
}
