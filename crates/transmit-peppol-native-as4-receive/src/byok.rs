// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Bring-your-own-key bridge: build a [`ReceiverConfig`] from a
//! customer-supplied [`PeppolCredentials`] bundle.
//!
//! The receiver's mTLS listener needs the AP certificate + key,
//! the bind URL (taken from `endpoint_url`), the SML mode, and
//! the participant id that incoming messages must address.

use std::path::PathBuf;

use invoicekit_transmit_peppol_byok::{PeppolCredentials, SmlMode as ByokSml, TransportKind};
use thiserror::Error;

use invoicekit_transmit_peppol_native_as4::SmlMode;

/// Receiver configuration derived from a [`PeppolCredentials`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiverConfig {
    /// URL the mTLS listener binds to (e.g. `https://0.0.0.0:4443/as4`).
    pub bind_url: String,
    /// AP certificate PEM path.
    pub cert_pem_path: PathBuf,
    /// AP private key PEM path.
    pub key_pem_path: PathBuf,
    /// SML mode (`Acceptance` or `Production`).
    pub sml_mode: SmlMode,
    /// Participant id incoming messages must address (wire format).
    pub participant_id_wire: String,
}

/// Errors raised translating BYOK credentials into a
/// [`ReceiverConfig`].
#[derive(Debug, Error)]
pub enum ByokBridgeError {
    /// Credentials carry a `transport` other than `native-as4`.
    #[error(
        "byok credentials have transport `{0:?}`, receiver requires NativeAs4 (the customer-hosted receive endpoint)"
    )]
    WrongTransport(TransportKind),
}

/// Build a [`ReceiverConfig`] from BYOK credentials.
///
/// # Errors
///
/// Returns [`ByokBridgeError::WrongTransport`] when the
/// credentials' transport is not `NativeAs4`.
pub fn receiver_config_from_byok(
    creds: &PeppolCredentials,
) -> Result<ReceiverConfig, ByokBridgeError> {
    if creds.transport != TransportKind::NativeAs4 {
        return Err(ByokBridgeError::WrongTransport(creds.transport));
    }
    let sml_mode = match creds.sml_mode {
        ByokSml::Test | ByokSml::Acceptance => SmlMode::Acceptance,
        ByokSml::Production => SmlMode::Production,
    };
    Ok(ReceiverConfig {
        bind_url: creds.endpoint_url.clone(),
        cert_pem_path: creds.cert_pem_path.clone(),
        key_pem_path: creds.key_pem_path.clone(),
        sml_mode,
        participant_id_wire: creds.participant_id.to_wire(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_transmit_peppol_byok::ParticipantId;
    use std::collections::BTreeMap;

    fn byok(transport: TransportKind, sml: ByokSml) -> PeppolCredentials {
        PeppolCredentials {
            participant_id: ParticipantId {
                scheme: "iso6523-actorid-upis".to_owned(),
                value: "0192:991825827".to_owned(),
            },
            cert_pem_path: PathBuf::from("/etc/peppol/cert.pem"),
            key_pem_path: PathBuf::from("/etc/peppol/key.pem"),
            key_passphrase_env: None,
            endpoint_url: "https://0.0.0.0:4443/as4".to_owned(),
            sml_mode: sml,
            transport,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn happy_path_test_sml_maps_to_acceptance() {
        let cfg =
            receiver_config_from_byok(&byok(TransportKind::NativeAs4, ByokSml::Test)).unwrap();
        assert_eq!(cfg.bind_url, "https://0.0.0.0:4443/as4");
        assert_eq!(cfg.cert_pem_path, PathBuf::from("/etc/peppol/cert.pem"));
        assert_eq!(cfg.sml_mode, SmlMode::Acceptance);
        assert_eq!(
            cfg.participant_id_wire,
            "iso6523-actorid-upis::0192:991825827"
        );
    }

    #[test]
    fn production_sml_maps_through() {
        let cfg = receiver_config_from_byok(&byok(TransportKind::NativeAs4, ByokSml::Production))
            .unwrap();
        assert_eq!(cfg.sml_mode, SmlMode::Production);
    }

    #[test]
    fn wrong_transport_is_rejected() {
        let err =
            receiver_config_from_byok(&byok(TransportKind::Partner, ByokSml::Test)).unwrap_err();
        assert!(matches!(err, ByokBridgeError::WrongTransport(_)));
    }
}
