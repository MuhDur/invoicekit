// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-archive` — pluggable archive backend.
//!
//! Exposes the [`Archive`] trait every InvoiceKit storage
//! backend implements. Shipped here:
//!
//! * [`LocalFsArchive`] — content-addressable directory under
//!   the operator's chosen root. The default backend the
//!   `services/invoicekit-archive-agent` (T-081 follow-up)
//!   wraps for single-host deployments.
//! * [`MockArchive`] — in-memory store; records every
//!   `store` call so tests + cassette-replay can assert on
//!   the captured bundles.
//!
//! S3 Object Lock, Azure WORM, GCS retention, and IPFS-hash
//! backends land behind feature flags in follow-up beads —
//! they share this trait surface so a tenant can switch
//! backends without touching call sites.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use invoicekit_evidence::{pack, unpack, BundleError, EvidenceBundle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Opaque, stable identifier for one archived bundle. Backends
/// pick the scheme ([`LocalFsArchive`] uses the bundle's BLAKE3 hash;
/// S3 returns the object key + version-id; IPFS returns the
/// CID).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArchiveId(pub String);

impl ArchiveId {
    /// Build a new archive id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Backend-side errors raised by [`Archive`] implementations.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// The bundle failed to serialise — typically because the
    /// caller built a malformed manifest.
    #[error("archive store failed: {0}")]
    Pack(#[from] BundleError),
    /// IO error talking to the backing store (FS unavailable,
    /// S3 rejected, etc.).
    #[error("archive backend io: {0}")]
    Io(String),
    /// The retrieve path could not find the archive id.
    #[error("archive id not found: {0}")]
    NotFound(String),
    /// The retrieved bytes failed bundle verification — the
    /// backend's store was tampered with after `store`.
    #[error("archive bundle drifted in storage: {0}")]
    Drift(String),
}

/// Pluggable archive trait.
///
/// All InvoiceKit storage operations happen through this surface
/// so a tenant can swap backends without touching call sites.
pub trait Archive: Send + Sync {
    /// Persist `bundle`. Returns an [`ArchiveId`] the caller
    /// records on the gateway receipt for replay.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when the bundle fails to
    /// serialise or the backend rejects the write.
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError>;

    /// Retrieve the bundle keyed by `id`. The returned bundle
    /// is verified via [`unpack`] — drift in the backing store
    /// surfaces as [`ArchiveError::Drift`].
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::NotFound`] when the id is
    /// unknown, or [`ArchiveError::Drift`] when the bytes
    /// fail verification.
    fn retrieve(&self, id: &ArchiveId) -> Result<EvidenceBundle, ArchiveError>;

    /// True if `id` is known to this archive. Cheap probe used
    /// by replay-from-bundle (T-085) before pulling the whole
    /// bundle over the wire.
    fn exists(&self, id: &ArchiveId) -> bool;
}

/// Content-addressable archive backed by a directory on disk.
///
/// Layout under `root`:
///
/// ```text
///   <root>/
///     <ab/cdef…>.ikb     // sharded by first two hex chars of the BLAKE3 hash
/// ```
///
/// The id is the bundle's BLAKE3 hash (lowercase hex). Two
/// bundles with identical bytes deduplicate naturally; two
/// bundles with the same logical content but different
/// manifests get distinct ids — by design, since the manifest
/// is part of the bundle.
pub struct LocalFsArchive {
    root: PathBuf,
}

impl LocalFsArchive {
    /// Build a new archive rooted at `root`. The directory is
    /// created on first `store` if it does not exist.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn entry_path(&self, id: &ArchiveId) -> PathBuf {
        let hex = id.as_str();
        let shard = if hex.len() >= 2 { &hex[..2] } else { "00" };
        self.root.join(shard).join(format!("{hex}.ikb"))
    }
}

impl Archive for LocalFsArchive {
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError> {
        let bytes = pack(bundle)?;
        let id = ArchiveId::new(invoicekit_evidence::blake3_hex(&bytes));
        let path = self.entry_path(&id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ArchiveError::Io(e.to_string()))?;
        }
        // Atomic write: stage in a sibling temp file, then
        // rename into place. Prevents readers from seeing a
        // half-written file under crash conditions.
        let temp_path = path.with_extension("ikb.tmp");
        {
            let mut file =
                fs::File::create(&temp_path).map_err(|e| ArchiveError::Io(e.to_string()))?;
            file.write_all(&bytes)
                .map_err(|e| ArchiveError::Io(e.to_string()))?;
            file.sync_all()
                .map_err(|e| ArchiveError::Io(e.to_string()))?;
        }
        fs::rename(&temp_path, &path).map_err(|e| ArchiveError::Io(e.to_string()))?;
        Ok(id)
    }

    fn retrieve(&self, id: &ArchiveId) -> Result<EvidenceBundle, ArchiveError> {
        let path = self.entry_path(id);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ArchiveError::NotFound(id.as_str().to_owned()));
            }
            Err(err) => return Err(ArchiveError::Io(err.to_string())),
        };
        unpack(&bytes).map_err(|err| ArchiveError::Drift(err.to_string()))
    }

    fn exists(&self, id: &ArchiveId) -> bool {
        self.entry_path(id).exists()
    }
}

/// In-memory archive.
///
/// Records every stored bundle so tests + the cassette-replay
/// sandbox can assert on the captured payloads.
pub struct MockArchive {
    store: Mutex<BTreeMap<ArchiveId, Vec<u8>>>,
}

impl MockArchive {
    /// Build a new empty mock archive.
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Mutex::new(BTreeMap::new()),
        }
    }

    /// Snapshot of every stored `(id, packed_bytes)` pair.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn entries(&self) -> Vec<(ArchiveId, Vec<u8>)> {
        self.store
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

impl Default for MockArchive {
    fn default() -> Self {
        Self::new()
    }
}

impl Archive for MockArchive {
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError> {
        let bytes = pack(bundle)?;
        let id = ArchiveId::new(invoicekit_evidence::blake3_hex(&bytes));
        self.store.lock().unwrap().insert(id.clone(), bytes);
        Ok(id)
    }

    fn retrieve(&self, id: &ArchiveId) -> Result<EvidenceBundle, ArchiveError> {
        let bytes = self
            .store
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| ArchiveError::NotFound(id.as_str().to_owned()))?;
        unpack(&bytes).map_err(|err| ArchiveError::Drift(err.to_string()))
    }

    fn exists(&self, id: &ArchiveId) -> bool {
        self.store.lock().unwrap().contains_key(id)
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_archive::crate_name(), "invoicekit-archive");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-archive"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::manifest_for;

    fn sample_bundle(created_at: &str) -> EvidenceBundle {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(&artefacts, "tenant-x", "trace-1", created_at);
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    fn alt_bundle(created_at: &str) -> EvidenceBundle {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-2"}"#.to_vec());
        let manifest = manifest_for(&artefacts, "tenant-x", "trace-2", created_at);
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-archive");
    }

    #[test]
    fn local_fs_archive_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let id = archive.store(&bundle).unwrap();
        assert!(archive.exists(&id));
        let recovered = archive.retrieve(&id).unwrap();
        assert_eq!(recovered, bundle);
    }

    #[test]
    fn local_fs_archive_dedups_identical_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let a = archive.store(&bundle).unwrap();
        let b = archive.store(&bundle).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn local_fs_archive_distinguishes_different_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let a = archive
            .store(&sample_bundle("2026-05-27T00:00:00Z"))
            .unwrap();
        let b = archive.store(&alt_bundle("2026-05-27T00:00:00Z")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn local_fs_archive_shards_by_hex_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let id = archive
            .store(&sample_bundle("2026-05-27T00:00:00Z"))
            .unwrap();
        let shard = &id.as_str()[..2];
        assert!(
            tmp.path().join(shard).is_dir(),
            "expected shard dir {shard}"
        );
    }

    #[test]
    fn local_fs_archive_retrieve_missing_id_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let err = archive.retrieve(&ArchiveId::new("deadbeef")).unwrap_err();
        assert!(matches!(err, ArchiveError::NotFound(_)));
    }

    #[test]
    fn local_fs_archive_detects_drift_in_storage() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        let id = archive
            .store(&sample_bundle("2026-05-27T00:00:00Z"))
            .unwrap();
        // Tamper with the file on disk.
        let entry = archive.entry_path(&id);
        let mut bytes = std::fs::read(&entry).unwrap();
        // Flip a byte well past the header so we hit a payload
        // hash check rather than the magic guard.
        let idx = bytes.len() - 5;
        bytes[idx] ^= 0xff;
        std::fs::write(&entry, bytes).unwrap();
        let err = archive.retrieve(&id).unwrap_err();
        assert!(matches!(err, ArchiveError::Drift(_)), "got {err:?}");
    }

    #[test]
    fn mock_archive_round_trip_and_entries() {
        let archive = MockArchive::new();
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let id = archive.store(&bundle).unwrap();
        assert!(archive.exists(&id));
        assert_eq!(archive.retrieve(&id).unwrap(), bundle);
        let entries = archive.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, id);
    }

    #[test]
    fn mock_archive_retrieve_missing_id_is_not_found() {
        let archive = MockArchive::new();
        let err = archive.retrieve(&ArchiveId::new("nope")).unwrap_err();
        assert!(matches!(err, ArchiveError::NotFound(_)));
    }

    #[test]
    fn archive_id_round_trips_through_json() {
        let id = ArchiveId::new("abc123");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"abc123\"");
        let back: ArchiveId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}
