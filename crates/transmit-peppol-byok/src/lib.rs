// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-transmit-peppol-byok` — bring-your-own-credentials
//! Peppol substrate.
//!
//! Year-1 live Peppol delivery uses a partner Access Point plus
//! `phase4` as a reference adapter (architectural commitment 7 in
//! `AGENTS.md`). This crate is the seam that lets the customer —
//! not InvoiceKit — own the Peppol identity. The customer
//! supplies the X.509 certificate, the matching key, the
//! Access Point endpoint URL, and the SML mode (test, acceptance,
//! production). InvoiceKit drives transmission against those
//! credentials.
//!
//! BYOK keeps InvoiceKit honest with the trust-toolkit direction:
//! we ship open code that reads the customer's evidence, never a
//! managed pipeline that requires customers to trust our identity.
//!
//! # Three transports
//!
//! The same [`PeppolCredentials`] feeds all three downstream
//! adapters:
//!
//! * [`TransportKind::Partner`] — drives the
//!   `invoicekit-transmit-peppol-partner` adapter against a
//!   hosted partner AP (Tickstar, Storecove, Pagero, …).
//! * [`TransportKind::Phase4`] — drives the
//!   `invoicekit-transmit-peppol-phase4` adapter against a
//!   customer-hosted phase4 sidecar.
//! * [`TransportKind::NativeAs4`] — drives the
//!   `invoicekit-transmit-peppol-native-as4` pure-Rust stack.
//!
//! # Doctor
//!
//! [`PeppolDoctor::check`] runs a sequence of safety checks
//! against a credentials file: each artefact exists, is
//! readable, parses as PEM, and the endpoint URL is a
//! well-formed `https://` URL. A complete check report
//! ([`DoctorReport`]) is returned so operators can fix
//! every problem in one pass instead of guessing.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

/// The Peppol transport the customer's credentials drive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    /// Hosted partner Access Point (Tickstar / Storecove / Pagero).
    Partner,
    /// Self-hosted phase4 sidecar speaking JSON-RPC.
    Phase4,
    /// Pure-Rust native AS4 stack.
    NativeAs4,
}

impl TransportKind {
    /// Stable kebab-case slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Partner => "partner",
            Self::Phase4 => "phase4",
            Self::NativeAs4 => "native-as4",
        }
    }
}

/// Peppol SML (Service Metadata Locator) mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SmlMode {
    /// Peppol Test Bed — free, public, requires no partner contract.
    Test,
    /// Peppol Acceptance (pre-production conformance).
    Acceptance,
    /// Production Peppol network.
    Production,
}

impl SmlMode {
    /// Stable kebab-case slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Acceptance => "acceptance",
            Self::Production => "production",
        }
    }
}

/// Typed Peppol participant identifier. Wire format is
/// `<scheme>::<value>` (e.g.
/// `iso6523-actorid-upis::0192:991825827`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParticipantId {
    /// Identifier scheme (e.g. `iso6523-actorid-upis`).
    pub scheme: String,
    /// Identifier value (e.g. `0192:991825827`).
    pub value: String,
}

impl ParticipantId {
    /// Parse a `<scheme>::<value>` wire string.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialsError::MalformedParticipantId`] when
    /// the separator is missing or either half is empty.
    pub fn parse(s: &str) -> Result<Self, CredentialsError> {
        let (scheme, value) = s.split_once("::").ok_or_else(|| {
            CredentialsError::MalformedParticipantId(format!("missing `::` separator in `{s}`"))
        })?;
        if scheme.is_empty() || value.is_empty() {
            return Err(CredentialsError::MalformedParticipantId(format!(
                "empty scheme or value in `{s}`"
            )));
        }
        Ok(Self {
            scheme: scheme.to_owned(),
            value: value.to_owned(),
        })
    }

    /// Canonical wire string `<scheme>::<value>`.
    #[must_use]
    pub fn to_wire(&self) -> String {
        format!("{}::{}", self.scheme, self.value)
    }
}

/// BYOK Peppol credentials supplied by the customer.
///
/// All paths are resolved relative to the credentials file (or
/// the process working directory when constructed in memory).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeppolCredentials {
    /// Customer's Peppol participant identifier.
    pub participant_id: ParticipantId,
    /// Path to the X.509 PEM certificate.
    pub cert_pem_path: PathBuf,
    /// Path to the matching private key PEM.
    pub key_pem_path: PathBuf,
    /// Name of an env var that holds the passphrase when the key
    /// is encrypted. Never the passphrase itself — the env-var
    /// indirection keeps the credentials file safe to commit.
    pub key_passphrase_env: Option<String>,
    /// Access Point endpoint URL (the AS4 inbox we POST to).
    pub endpoint_url: String,
    /// SML mode (test, acceptance, production).
    pub sml_mode: SmlMode,
    /// Which transport will drive these credentials.
    pub transport: TransportKind,
    /// Optional extra labels (e.g. `country = "NO"`) the
    /// downstream adapter can read for routing decisions.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl PeppolCredentials {
    /// Load credentials from a JSON file on disk.
    ///
    /// Relative `cert_pem_path` and `key_pem_path` are resolved
    /// against the parent of `path`.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialsError`] on I/O failure or malformed
    /// JSON. Does *not* validate cert / key contents — use
    /// [`PeppolDoctor::check`] for that.
    pub fn from_json_file(path: &Path) -> Result<Self, CredentialsError> {
        let text = fs::read_to_string(path)
            .map_err(|e| CredentialsError::Io(format!("read {}: {e}", path.display())))?;
        let mut creds: Self = serde_json::from_str(&text)
            .map_err(|e| CredentialsError::Malformed(format!("parse {}: {e}", path.display())))?;
        if let Some(parent) = path.parent() {
            if creds.cert_pem_path.is_relative() {
                creds.cert_pem_path = parent.join(&creds.cert_pem_path);
            }
            if creds.key_pem_path.is_relative() {
                creds.key_pem_path = parent.join(&creds.key_pem_path);
            }
        }
        Ok(creds)
    }

    /// Resolve the optional passphrase via the configured env var.
    ///
    /// Returns `None` when no env var was configured, returns the
    /// value when the variable is set, and returns an error when
    /// the variable was named but is missing from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialsError::MissingPassphraseEnv`] when the
    /// configured env var is unset.
    pub fn resolve_passphrase(
        &self,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Option<String>, CredentialsError> {
        let Some(var) = self.key_passphrase_env.as_deref() else {
            return Ok(None);
        };
        env(var)
            .map(Some)
            .ok_or_else(|| CredentialsError::MissingPassphraseEnv(var.to_owned()))
    }
}

/// Errors raised loading and validating credentials.
#[derive(Debug, Error)]
pub enum CredentialsError {
    /// Underlying filesystem read failed.
    #[error("credentials io: {0}")]
    Io(String),
    /// JSON was syntactically invalid.
    #[error("credentials malformed: {0}")]
    Malformed(String),
    /// Participant id wire string was malformed.
    #[error("participant id: {0}")]
    MalformedParticipantId(String),
    /// `key_passphrase_env` named a variable that wasn't set.
    #[error("passphrase env var `{0}` is not set")]
    MissingPassphraseEnv(String),
}

/// Result of one doctor check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckStatus {
    /// Check passed.
    Ok,
    /// Check failed; carries the operator-facing reason.
    Failed(String),
    /// Check was skipped (typically because an upstream check
    /// failed and re-running this one would be misleading).
    Skipped(String),
}

/// One row in a [`DoctorReport`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckRow {
    /// Stable check id (e.g. `cert.readable`).
    pub id: String,
    /// Outcome.
    pub status: CheckStatus,
}

/// Full doctor report. The `overall` field flips false if any
/// row is `Failed`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// True when every check passed (skipped rows do NOT fail
    /// the overall report).
    pub overall: bool,
    /// One row per check.
    pub rows: Vec<CheckRow>,
}

impl DoctorReport {
    /// True when every row is `Ok` or `Skipped`.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.overall
    }

    /// Rows whose status is `Failed`.
    #[must_use]
    pub fn failures(&self) -> Vec<&CheckRow> {
        self.rows
            .iter()
            .filter(|r| matches!(r.status, CheckStatus::Failed(_)))
            .collect()
    }
}

/// Filesystem seam — lets tests inject in-memory blobs without
/// touching `std::fs`.
pub trait DoctorFs: Send + Sync {
    /// Return file bytes.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`std::io::Error`] when the path
    /// cannot be read (missing, permission denied, etc.).
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>>;
    /// True when the path exists.
    fn exists(&self, path: &Path) -> bool;
}

/// Production [`DoctorFs`] wired to `std::fs`.
#[derive(Default)]
pub struct StdFs;

impl DoctorFs for StdFs {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        fs::read(path)
    }
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Validates a [`PeppolCredentials`] bundle end-to-end without
/// transmitting anything.
pub struct PeppolDoctor<'a> {
    fs: &'a dyn DoctorFs,
}

impl<'a> PeppolDoctor<'a> {
    /// Build a doctor against the given filesystem.
    #[must_use]
    pub fn new(fs: &'a dyn DoctorFs) -> Self {
        Self { fs }
    }

    /// Run every check and return a report.
    #[must_use]
    pub fn check(&self, creds: &PeppolCredentials) -> DoctorReport {
        let mut rows = Vec::new();
        self.check_artefact(&mut rows, "cert", &creds.cert_pem_path, ArtefactKind::Cert);
        self.check_artefact(&mut rows, "key", &creds.key_pem_path, ArtefactKind::Key);
        rows.push(CheckRow {
            id: "endpoint.https".to_owned(),
            status: validate_endpoint_url(&creds.endpoint_url),
        });
        rows.push(CheckRow {
            id: "participant.well-formed".to_owned(),
            status: match ParticipantId::parse(&creds.participant_id.to_wire()) {
                Ok(_) => CheckStatus::Ok,
                Err(e) => CheckStatus::Failed(e.to_string()),
            },
        });
        rows.push(CheckRow {
            id: "sml.mode-set".to_owned(),
            status: CheckStatus::Ok,
        });
        let overall = rows
            .iter()
            .all(|r| !matches!(r.status, CheckStatus::Failed(_)));
        debug!(overall, rows = rows.len(), "peppol doctor finished");
        DoctorReport { overall, rows }
    }

    fn check_artefact(
        &self,
        rows: &mut Vec<CheckRow>,
        prefix: &str,
        path: &Path,
        kind: ArtefactKind,
    ) {
        let exists = self.fs.exists(path);
        rows.push(CheckRow {
            id: format!("{prefix}.exists"),
            status: if exists {
                CheckStatus::Ok
            } else {
                CheckStatus::Failed(format!("{prefix} pem not found: {}", path.display()))
            },
        });
        let bytes = if exists {
            match self.fs.read(path) {
                Ok(b) => Some(b),
                Err(e) => {
                    rows.push(CheckRow {
                        id: format!("{prefix}.readable"),
                        status: CheckStatus::Failed(format!("read failed: {e}")),
                    });
                    None
                }
            }
        } else {
            rows.push(CheckRow {
                id: format!("{prefix}.readable"),
                status: CheckStatus::Skipped(format!("{prefix} missing")),
            });
            None
        };
        if let Some(bytes) = &bytes {
            rows.push(CheckRow {
                id: format!("{prefix}.readable"),
                status: CheckStatus::Ok,
            });
            rows.push(CheckRow {
                id: format!("{prefix}.pem-shaped"),
                status: kind.validate_pem(bytes),
            });
        } else {
            rows.push(CheckRow {
                id: format!("{prefix}.pem-shaped"),
                status: CheckStatus::Skipped(format!("{prefix} unreadable")),
            });
        }
    }
}

#[derive(Clone, Copy)]
enum ArtefactKind {
    Cert,
    Key,
}

impl ArtefactKind {
    fn validate_pem(self, bytes: &[u8]) -> CheckStatus {
        match self {
            Self::Cert if is_pem_block(bytes, "CERTIFICATE") => CheckStatus::Ok,
            Self::Cert => CheckStatus::Failed("cert is not a PEM CERTIFICATE block".to_owned()),
            Self::Key if is_any_key_pem(bytes) => CheckStatus::Ok,
            Self::Key => CheckStatus::Failed(
                "key is not a PEM PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY block".to_owned(),
            ),
        }
    }
}

fn is_pem_block(bytes: &[u8], label: &str) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    text.contains(&begin) && text.contains(&end)
}

fn is_any_key_pem(bytes: &[u8]) -> bool {
    for label in ["PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY"] {
        if is_pem_block(bytes, label) {
            return true;
        }
    }
    false
}

fn validate_endpoint_url(url: &str) -> CheckStatus {
    if url.starts_with("https://") && url.len() > "https://".len() {
        CheckStatus::Ok
    } else if url.starts_with("http://") {
        CheckStatus::Failed("endpoint must be https://, got http://".to_owned())
    } else {
        CheckStatus::Failed(format!("endpoint must start with https://, got `{url}`"))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_transmit_peppol_byok::crate_name(),
///     "invoicekit-transmit-peppol-byok"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-peppol-byok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct InMemoryFs {
        files: Mutex<HashMap<PathBuf, Vec<u8>>>,
    }

    impl InMemoryFs {
        fn new(entries: &[(&str, &[u8])]) -> Self {
            let mut map = HashMap::new();
            for (k, v) in entries {
                map.insert(PathBuf::from(k), (*v).to_vec());
            }
            Self {
                files: Mutex::new(map),
            }
        }
    }

    impl DoctorFs for InMemoryFs {
        fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.lock().unwrap().contains_key(path)
        }
    }

    fn happy_creds() -> PeppolCredentials {
        PeppolCredentials {
            participant_id: ParticipantId {
                scheme: "iso6523-actorid-upis".to_owned(),
                value: "0192:991825827".to_owned(),
            },
            cert_pem_path: PathBuf::from("/etc/peppol/cert.pem"),
            key_pem_path: PathBuf::from("/etc/peppol/key.pem"),
            key_passphrase_env: None,
            endpoint_url: "https://ap.example.com/as4".to_owned(),
            sml_mode: SmlMode::Test,
            transport: TransportKind::Partner,
            labels: BTreeMap::new(),
        }
    }

    const CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\nMIIBLOL\n-----END CERTIFICATE-----\n";
    const KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMIIBLOL\n-----END PRIVATE KEY-----\n";

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-transmit-peppol-byok");
    }

    #[test]
    fn participant_id_parses_well_formed_wire_string() {
        let pid = ParticipantId::parse("iso6523-actorid-upis::0192:991825827").unwrap();
        assert_eq!(pid.scheme, "iso6523-actorid-upis");
        assert_eq!(pid.value, "0192:991825827");
        assert_eq!(pid.to_wire(), "iso6523-actorid-upis::0192:991825827");
    }

    #[test]
    fn participant_id_rejects_missing_separator() {
        assert!(matches!(
            ParticipantId::parse("iso6523-actorid-upis-0192-991825827"),
            Err(CredentialsError::MalformedParticipantId(_))
        ));
    }

    #[test]
    fn participant_id_rejects_empty_scheme_or_value() {
        assert!(matches!(
            ParticipantId::parse("::0192:991825827"),
            Err(CredentialsError::MalformedParticipantId(_))
        ));
        assert!(matches!(
            ParticipantId::parse("iso6523-actorid-upis::"),
            Err(CredentialsError::MalformedParticipantId(_))
        ));
    }

    #[test]
    fn transport_kind_slugs_round_trip_serde() {
        for k in [
            TransportKind::Partner,
            TransportKind::Phase4,
            TransportKind::NativeAs4,
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let back: TransportKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, k);
            assert!(!k.slug().is_empty());
        }
    }

    #[test]
    fn sml_mode_slugs_round_trip_serde() {
        for m in [SmlMode::Test, SmlMode::Acceptance, SmlMode::Production] {
            let json = serde_json::to_string(&m).unwrap();
            let back: SmlMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, m);
            assert!(!m.slug().is_empty());
        }
    }

    #[test]
    fn resolve_passphrase_returns_none_when_not_configured() {
        let creds = happy_creds();
        let env = |_: &str| -> Option<String> { None };
        assert_eq!(creds.resolve_passphrase(&env).unwrap(), None);
    }

    #[test]
    fn resolve_passphrase_returns_value_when_env_set() {
        let mut creds = happy_creds();
        creds.key_passphrase_env = Some("PEPPOL_KEY_PASSPHRASE".to_owned());
        let env = |k: &str| -> Option<String> {
            if k == "PEPPOL_KEY_PASSPHRASE" {
                Some("hunter2".to_owned())
            } else {
                None
            }
        };
        assert_eq!(
            creds.resolve_passphrase(&env).unwrap(),
            Some("hunter2".to_owned())
        );
    }

    #[test]
    fn resolve_passphrase_errors_when_env_var_named_but_unset() {
        let mut creds = happy_creds();
        creds.key_passphrase_env = Some("MISSING_VAR".to_owned());
        let env = |_: &str| -> Option<String> { None };
        let err = creds.resolve_passphrase(&env).unwrap_err();
        assert!(matches!(err, CredentialsError::MissingPassphraseEnv(_)));
    }

    #[test]
    fn doctor_passes_on_well_formed_credentials() {
        let fs = InMemoryFs::new(&[
            ("/etc/peppol/cert.pem", CERT_PEM),
            ("/etc/peppol/key.pem", KEY_PEM),
        ]);
        let doctor = PeppolDoctor::new(&fs);
        let report = doctor.check(&happy_creds());
        assert!(report.passed(), "report: {report:?}");
        assert!(report.failures().is_empty());
    }

    #[test]
    fn doctor_fails_when_cert_missing() {
        let fs = InMemoryFs::new(&[("/etc/peppol/key.pem", KEY_PEM)]);
        let doctor = PeppolDoctor::new(&fs);
        let report = doctor.check(&happy_creds());
        assert!(!report.passed());
        assert!(report.failures().iter().any(|r| r.id == "cert.exists"));
    }

    #[test]
    fn doctor_fails_on_http_endpoint() {
        let fs = InMemoryFs::new(&[
            ("/etc/peppol/cert.pem", CERT_PEM),
            ("/etc/peppol/key.pem", KEY_PEM),
        ]);
        let doctor = PeppolDoctor::new(&fs);
        let mut creds = happy_creds();
        creds.endpoint_url = "http://ap.example.com/as4".to_owned();
        let report = doctor.check(&creds);
        assert!(!report.passed());
        assert!(report.failures().iter().any(|r| r.id == "endpoint.https"));
    }

    #[test]
    fn doctor_fails_when_cert_is_not_pem_shaped() {
        let fs = InMemoryFs::new(&[
            ("/etc/peppol/cert.pem", b"not a pem block"),
            ("/etc/peppol/key.pem", KEY_PEM),
        ]);
        let doctor = PeppolDoctor::new(&fs);
        let report = doctor.check(&happy_creds());
        assert!(!report.passed());
        assert!(report.failures().iter().any(|r| r.id == "cert.pem-shaped"));
    }

    #[test]
    fn doctor_accepts_rsa_and_ec_private_key_pem() {
        let rsa = b"-----BEGIN RSA PRIVATE KEY-----\nABC\n-----END RSA PRIVATE KEY-----\n";
        let fs = InMemoryFs::new(&[
            ("/etc/peppol/cert.pem", CERT_PEM),
            ("/etc/peppol/key.pem", rsa.as_slice()),
        ]);
        let doctor = PeppolDoctor::new(&fs);
        let report = doctor.check(&happy_creds());
        assert!(report.passed(), "report: {report:?}");
    }

    #[test]
    fn credentials_round_trip_through_json() {
        let creds = happy_creds();
        let json = serde_json::to_string(&creds).unwrap();
        let back: PeppolCredentials = serde_json::from_str(&json).unwrap();
        assert_eq!(back, creds);
    }

    #[test]
    fn from_json_file_resolves_relative_paths() {
        let tmp = tempdir();
        let cert = tmp.join("cert.pem");
        let key = tmp.join("key.pem");
        fs::write(&cert, CERT_PEM).unwrap();
        fs::write(&key, KEY_PEM).unwrap();
        let creds_path = tmp.join("creds.json");
        let body = r#"{
            "participant_id": {"scheme": "iso6523-actorid-upis", "value": "0192:991825827"},
            "cert_pem_path": "cert.pem",
            "key_pem_path": "key.pem",
            "endpoint_url": "https://ap.example.com/as4",
            "sml_mode": "test",
            "transport": "partner"
        }"#;
        fs::write(&creds_path, body).unwrap();
        let loaded = PeppolCredentials::from_json_file(&creds_path).unwrap();
        assert_eq!(loaded.cert_pem_path, cert);
        assert_eq!(loaded.key_pem_path, key);
        assert_eq!(loaded.sml_mode, SmlMode::Test);
        assert_eq!(loaded.transport, TransportKind::Partner);
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let n: u128 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!("ik-byok-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
