// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Bring-your-own-key bridge: build a [`NativeAs4Config`] from a
//! customer-supplied [`PeppolCredentials`] bundle.
//!
//! The customer's PEM-encoded certificate is read at translation
//! time. BYOK `SmlMode::Test` / `SmlMode::Acceptance` both map to
//! native-as4 `Acceptance` (the Peppol Test Bed runs under the
//! Acceptance SML); `Production` maps through unchanged.

use std::fs;

use invoicekit_transmit_peppol_byok::{PeppolCredentials, SmlMode as ByokSml, TransportKind};
use thiserror::Error;

use crate::{NativeAs4Config, SmlMode};

/// Errors raised translating BYOK credentials into a
/// [`NativeAs4Config`].
#[derive(Debug, Error)]
pub enum ByokBridgeError {
    /// Credentials carry a `transport` other than `native-as4`.
    #[error("byok credentials have transport `{0:?}`, native-as4 adapter requires NativeAs4")]
    WrongTransport(TransportKind),
    /// Cert PEM read failed.
    #[error("read cert pem `{path}`: {source}")]
    CertRead {
        /// Path the read was attempted from.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Cert PEM was not UTF-8.
    #[error("cert pem is not UTF-8")]
    CertNotUtf8,
}

/// Build a [`NativeAs4Config`] from BYOK credentials.
///
/// # Errors
///
/// Returns [`ByokBridgeError::WrongTransport`] when the
/// credentials' transport is not `NativeAs4`, or
/// [`ByokBridgeError::CertRead`] / [`ByokBridgeError::CertNotUtf8`]
/// when the certificate file can't be read.
pub fn native_as4_config_from_byok(
    creds: &PeppolCredentials,
) -> Result<NativeAs4Config, ByokBridgeError> {
    if creds.transport != TransportKind::NativeAs4 {
        return Err(ByokBridgeError::WrongTransport(creds.transport));
    }
    let cert_bytes =
        fs::read(&creds.cert_pem_path).map_err(|source| ByokBridgeError::CertRead {
            path: creds.cert_pem_path.display().to_string(),
            source,
        })?;
    let ap_cert_pem = String::from_utf8(cert_bytes).map_err(|_| ByokBridgeError::CertNotUtf8)?;
    let sml_mode = match creds.sml_mode {
        ByokSml::Test | ByokSml::Acceptance => SmlMode::Acceptance,
        ByokSml::Production => SmlMode::Production,
    };
    Ok(NativeAs4Config {
        ap_cert_pem,
        sml_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_transmit_peppol_byok::ParticipantId;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\nMIIBLOL\n-----END CERTIFICATE-----\n";

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let n: u128 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!("ik-byok-nas4-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn byok(transport: TransportKind, sml: ByokSml, cert_path: PathBuf) -> PeppolCredentials {
        PeppolCredentials {
            participant_id: ParticipantId {
                scheme: "iso6523-actorid-upis".to_owned(),
                value: "0192:991825827".to_owned(),
            },
            cert_pem_path: cert_path,
            key_pem_path: PathBuf::from("/etc/peppol/key.pem"),
            key_passphrase_env: None,
            endpoint_url: "https://ap.example.com/as4".to_owned(),
            sml_mode: sml,
            transport,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn happy_path_reads_cert_pem_and_maps_test_to_acceptance() {
        let tmp = tempdir();
        let cert = tmp.join("cert.pem");
        std::fs::write(&cert, CERT_PEM).unwrap();
        let cfg = native_as4_config_from_byok(&byok(TransportKind::NativeAs4, ByokSml::Test, cert))
            .unwrap();
        assert!(cfg.ap_cert_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(cfg.sml_mode, SmlMode::Acceptance);
    }

    #[test]
    fn production_sml_maps_through() {
        let tmp = tempdir();
        let cert = tmp.join("cert.pem");
        std::fs::write(&cert, CERT_PEM).unwrap();
        let cfg =
            native_as4_config_from_byok(&byok(TransportKind::NativeAs4, ByokSml::Production, cert))
                .unwrap();
        assert_eq!(cfg.sml_mode, SmlMode::Production);
    }

    #[test]
    fn wrong_transport_is_rejected() {
        let tmp = tempdir();
        let cert = tmp.join("cert.pem");
        std::fs::write(&cert, CERT_PEM).unwrap();
        let err = native_as4_config_from_byok(&byok(TransportKind::Partner, ByokSml::Test, cert))
            .unwrap_err();
        assert!(matches!(err, ByokBridgeError::WrongTransport(_)));
    }

    #[test]
    fn missing_cert_file_is_surfaced() {
        let creds = byok(
            TransportKind::NativeAs4,
            ByokSml::Test,
            PathBuf::from("/this/path/does/not/exist.pem"),
        );
        let err = native_as4_config_from_byok(&creds).unwrap_err();
        assert!(matches!(err, ByokBridgeError::CertRead { .. }));
    }
}
