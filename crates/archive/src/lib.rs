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
//! * [`S3ObjectLockArchive`] — retention-aware S3 Object Lock
//!   archive over a small object-store adapter.
//! * [`AzureWormArchive`] — retention-aware Azure WORM blob
//!   archive over the same object-store adapter contract.
//! * [`MockArchive`] — in-memory store; records every
//!   `store` call so tests + cassette-replay can assert on
//!   the captured bundles.
//!
//! GCS retention and IPFS-hash backends land in follow-up
//! beads; they share this trait surface so a tenant can
//! switch backends without touching call sites.

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
    /// The archive id used a scheme or namespace that this
    /// backend does not own.
    #[error("invalid archive id for backend: {0}")]
    InvalidId(String),
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

/// Backend family used when sending immutable object writes to
/// an [`ObjectLockStore`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectLockBackend {
    /// Amazon S3-compatible Object Lock.
    S3ObjectLock,
    /// Azure Blob immutability policy / legal hold.
    AzureWorm,
}

/// Regulatory retention mode attached to a stored object.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionMode {
    /// Object can be shortened only by privileged operators.
    Governance,
    /// Object cannot be shortened by ordinary operators.
    Compliance,
}

impl RetentionMode {
    /// Return the retention mode spelling used by S3 Object Lock
    /// and compatible object-store APIs.
    #[must_use]
    pub fn as_cloud_value(self) -> &'static str {
        match self {
            Self::Governance => "GOVERNANCE",
            Self::Compliance => "COMPLIANCE",
        }
    }
}

/// Immutable retention policy applied to an archived bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Retention mode requested from the backing service.
    pub mode: RetentionMode,
    /// RFC 3339 instant until which the object must be retained.
    pub retain_until: String,
    /// Whether legal hold should be enabled for the object.
    pub legal_hold: bool,
}

impl RetentionPolicy {
    /// Build a compliance-mode retention policy.
    #[must_use]
    pub fn compliance(retain_until: impl Into<String>) -> Self {
        Self {
            mode: RetentionMode::Compliance,
            retain_until: retain_until.into(),
            legal_hold: false,
        }
    }

    /// Build a governance-mode retention policy.
    #[must_use]
    pub fn governance(retain_until: impl Into<String>) -> Self {
        Self {
            mode: RetentionMode::Governance,
            retain_until: retain_until.into(),
            legal_hold: false,
        }
    }

    /// Enable legal hold on the stored object.
    #[must_use]
    pub fn with_legal_hold(mut self) -> Self {
        self.legal_hold = true;
        self
    }
}

/// Object write request produced by WORM archive backends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectLockPut {
    /// Backend family that produced this request.
    pub backend: ObjectLockBackend,
    /// Bucket, container, or equivalent object namespace.
    pub namespace: String,
    /// Object key / blob name.
    pub key: String,
    /// Packed `.ikb` bytes.
    pub body: Vec<u8>,
    /// BLAKE3 hash of `body`, used as the content address.
    pub content_hash: String,
    /// Retention policy requested for this object.
    pub retention: RetentionPolicy,
}

/// Object write receipt returned by an [`ObjectLockStore`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectLockReceipt {
    /// Version id or equivalent immutable generation marker.
    pub version_id: Option<String>,
}

/// Minimal object-store adapter contract for retention-capable
/// archives.
///
/// The archive crate stays cloud-SDK-free; production agents can
/// adapt AWS SDK, Azure SDK, `LocalStack`, or a sidecar API behind
/// this trait without changing [`Archive`] call sites.
pub trait ObjectLockStore: Send + Sync {
    /// Persist immutable object bytes with the requested retention
    /// policy.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when the backing service rejects
    /// the write or cannot apply retention.
    fn put_locked_object(&self, request: ObjectLockPut) -> Result<ObjectLockReceipt, ArchiveError>;

    /// Retrieve previously stored object bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError::NotFound`] when the key is unknown.
    fn get_locked_object(&self, namespace: &str, key: &str) -> Result<Vec<u8>, ArchiveError>;

    /// True if the object is known to the backing service.
    fn locked_object_exists(&self, namespace: &str, key: &str) -> bool;
}

/// S3 Object Lock archive.
///
/// Stores packed `.ikb` evidence bundles under
/// `<prefix>/<blake3>.ikb` and requests retention from the
/// provided [`ObjectLockStore`]. Archive ids use the form
/// `s3://<bucket>/<key>#versionId=<version>` when the backend
/// returns a version id.
pub struct S3ObjectLockArchive<S> {
    bucket: String,
    key_prefix: String,
    retention: RetentionPolicy,
    store: S,
}

impl<S> S3ObjectLockArchive<S> {
    /// Build an S3 Object Lock archive over a store adapter.
    #[must_use]
    pub fn new(
        bucket: impl Into<String>,
        key_prefix: impl Into<String>,
        store: S,
        retention: RetentionPolicy,
    ) -> Self {
        let key_prefix = key_prefix.into();
        Self {
            bucket: bucket.into(),
            key_prefix: normalize_prefix(&key_prefix),
            retention,
            store,
        }
    }
}

impl<S: ObjectLockStore> Archive for S3ObjectLockArchive<S> {
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError> {
        let bytes = pack(bundle)?;
        let hash = invoicekit_evidence::blake3_hex(&bytes);
        let key = object_key(&self.key_prefix, &hash);
        let receipt = self.store.put_locked_object(ObjectLockPut {
            backend: ObjectLockBackend::S3ObjectLock,
            namespace: self.bucket.clone(),
            key: key.clone(),
            body: bytes,
            content_hash: hash,
            retention: self.retention.clone(),
        })?;
        Ok(scheme_archive_id(
            "s3",
            &self.bucket,
            &key,
            receipt.version_id.as_deref(),
        ))
    }

    fn retrieve(&self, id: &ArchiveId) -> Result<EvidenceBundle, ArchiveError> {
        let key = object_from_scheme_id("s3", &self.bucket, id)?;
        let bytes = self.store.get_locked_object(&self.bucket, &key)?;
        unpack(&bytes).map_err(|err| ArchiveError::Drift(err.to_string()))
    }

    fn exists(&self, id: &ArchiveId) -> bool {
        object_from_scheme_id("s3", &self.bucket, id)
            .is_ok_and(|key| self.store.locked_object_exists(&self.bucket, &key))
    }
}

/// Azure Blob WORM archive.
///
/// Stores packed `.ikb` evidence bundles under
/// `<prefix>/<blake3>.ikb` and requests an immutable blob policy
/// from the provided [`ObjectLockStore`]. Archive ids use the form
/// `azure://<container>/<blob>#versionId=<version>`.
pub struct AzureWormArchive<S> {
    container: String,
    blob_prefix: String,
    retention: RetentionPolicy,
    store: S,
}

impl<S> AzureWormArchive<S> {
    /// Build an Azure WORM archive over a store adapter.
    #[must_use]
    pub fn new(
        container: impl Into<String>,
        blob_prefix: impl Into<String>,
        store: S,
        retention: RetentionPolicy,
    ) -> Self {
        let blob_prefix = blob_prefix.into();
        Self {
            container: container.into(),
            blob_prefix: normalize_prefix(&blob_prefix),
            retention,
            store,
        }
    }
}

impl<S: ObjectLockStore> Archive for AzureWormArchive<S> {
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError> {
        let bytes = pack(bundle)?;
        let hash = invoicekit_evidence::blake3_hex(&bytes);
        let blob = object_key(&self.blob_prefix, &hash);
        let receipt = self.store.put_locked_object(ObjectLockPut {
            backend: ObjectLockBackend::AzureWorm,
            namespace: self.container.clone(),
            key: blob.clone(),
            body: bytes,
            content_hash: hash,
            retention: self.retention.clone(),
        })?;
        Ok(scheme_archive_id(
            "azure",
            &self.container,
            &blob,
            receipt.version_id.as_deref(),
        ))
    }

    fn retrieve(&self, id: &ArchiveId) -> Result<EvidenceBundle, ArchiveError> {
        let blob = object_from_scheme_id("azure", &self.container, id)?;
        let bytes = self.store.get_locked_object(&self.container, &blob)?;
        unpack(&bytes).map_err(|err| ArchiveError::Drift(err.to_string()))
    }

    fn exists(&self, id: &ArchiveId) -> bool {
        object_from_scheme_id("azure", &self.container, id)
            .is_ok_and(|blob| self.store.locked_object_exists(&self.container, &blob))
    }
}

fn normalize_prefix(prefix: &str) -> String {
    prefix.trim_matches('/').to_owned()
}

fn object_key(prefix: &str, hash: &str) -> String {
    if prefix.is_empty() {
        format!("{hash}.ikb")
    } else {
        format!("{prefix}/{hash}.ikb")
    }
}

/// Build a `<scheme>://<namespace>/<object>` archive id, appending
/// `#versionId=<version>` when the backend returned a version id.
fn scheme_archive_id(
    scheme: &str,
    namespace: &str,
    object: &str,
    version_id: Option<&str>,
) -> ArchiveId {
    version_id.map_or_else(
        || ArchiveId::new(format!("{scheme}://{namespace}/{object}")),
        |version_id| {
            ArchiveId::new(format!("{scheme}://{namespace}/{object}#versionId={version_id}"))
        },
    )
}

/// Parse the object key/blob name out of a `<scheme>://<namespace>/…`
/// archive id, rejecting ids that belong to another namespace.
fn object_from_scheme_id(
    scheme: &str,
    namespace: &str,
    id: &ArchiveId,
) -> Result<String, ArchiveError> {
    let prefix = format!("{scheme}://{namespace}/");
    let rest = id
        .as_str()
        .strip_prefix(&prefix)
        .ok_or_else(|| ArchiveError::InvalidId(id.as_str().to_owned()))?;
    let object = rest
        .split_once("#versionId=")
        .map_or(rest, |(object, _)| object);
    if object.is_empty() {
        return Err(ArchiveError::InvalidId(id.as_str().to_owned()));
    }
    Ok(object.to_owned())
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

    fn entry_path(&self, id: &ArchiveId) -> Result<PathBuf, ArchiveError> {
        let hex = id.as_str();
        // The id for this backend is the bundle's BLAKE3 hash:
        // exactly 64 lowercase hex chars. Validating it here
        // before any path join blocks traversal ids such as
        // `../../../etc/passwd` from escaping the archive root,
        // and guarantees the `&hex[..2]` shard slice lands on a
        // char boundary (it is pure ASCII). Mirrors the
        // `object_from_scheme_id` guard the S3/Azure backends use.
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(ArchiveError::InvalidId(hex.to_owned()));
        }
        let shard = &hex[..2];
        Ok(self.root.join(shard).join(format!("{hex}.ikb")))
    }
}

impl Archive for LocalFsArchive {
    fn store(&self, bundle: &EvidenceBundle) -> Result<ArchiveId, ArchiveError> {
        let bytes = pack(bundle)?;
        let id = ArchiveId::new(invoicekit_evidence::blake3_hex(&bytes));
        let path = self.entry_path(&id)?;
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
        let path = self.entry_path(id)?;
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
        // An id this backend would never have minted cannot exist
        // here; reject it without touching the filesystem.
        self.entry_path(id).is_ok_and(|path| path.exists())
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
    use std::io::Read as _;
    use std::net::{Shutdown, TcpStream, ToSocketAddrs};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct StoredLockedObject {
        backend: ObjectLockBackend,
        body: Vec<u8>,
        content_hash: String,
        retention: RetentionPolicy,
        version_id: String,
    }

    #[derive(Clone, Default)]
    struct InMemoryObjectLockStore {
        objects: Arc<Mutex<BTreeMap<(String, String), StoredLockedObject>>>,
    }

    impl InMemoryObjectLockStore {
        fn object(&self, namespace: &str, key: &str) -> Option<StoredLockedObject> {
            self.objects
                .lock()
                .unwrap()
                .get(&(namespace.to_owned(), key.to_owned()))
                .cloned()
        }
    }

    impl ObjectLockStore for InMemoryObjectLockStore {
        fn put_locked_object(
            &self,
            request: ObjectLockPut,
        ) -> Result<ObjectLockReceipt, ArchiveError> {
            let mut objects = self.objects.lock().unwrap();
            let version_id = format!("v{}", objects.len() + 1);
            objects.insert(
                (request.namespace, request.key),
                StoredLockedObject {
                    backend: request.backend,
                    body: request.body,
                    content_hash: request.content_hash,
                    retention: request.retention,
                    version_id: version_id.clone(),
                },
            );
            drop(objects);
            Ok(ObjectLockReceipt {
                version_id: Some(version_id),
            })
        }

        fn get_locked_object(&self, namespace: &str, key: &str) -> Result<Vec<u8>, ArchiveError> {
            self.objects
                .lock()
                .unwrap()
                .get(&(namespace.to_owned(), key.to_owned()))
                .map(|object| object.body.clone())
                .ok_or_else(|| ArchiveError::NotFound(format!("{namespace}/{key}")))
        }

        fn locked_object_exists(&self, namespace: &str, key: &str) -> bool {
            self.objects
                .lock()
                .unwrap()
                .contains_key(&(namespace.to_owned(), key.to_owned()))
        }
    }

    struct LocalStackS3Store {
        host: String,
        port: u16,
    }

    impl LocalStackS3Store {
        fn new(endpoint: &str) -> Result<Self, ArchiveError> {
            let without_scheme = endpoint
                .strip_prefix("http://")
                .ok_or_else(|| ArchiveError::Io("LocalStack endpoint must use http://".into()))?;
            let authority = without_scheme
                .split_once('/')
                .map_or(without_scheme, |(authority, _)| authority);
            let (host, port) = authority
                .rsplit_once(':')
                .ok_or_else(|| ArchiveError::Io("LocalStack endpoint must include port".into()))?;
            let port = port
                .parse()
                .map_err(|_| ArchiveError::Io("LocalStack endpoint port is invalid".into()))?;
            Ok(Self {
                host: host.to_owned(),
                port,
            })
        }

        fn create_object_lock_bucket(&self, bucket: &str) -> Result<(), ArchiveError> {
            let (status, _) = self.request(
                "PUT",
                &format!("/{bucket}"),
                &[("x-amz-bucket-object-lock-enabled", "true")],
                &[],
            )?;
            if matches!(status, 200 | 409) {
                Ok(())
            } else {
                Err(ArchiveError::Io(format!(
                    "LocalStack bucket create returned HTTP {status}"
                )))
            }
        }

        fn request(
            &self,
            method: &str,
            path: &str,
            headers: &[(&str, &str)],
            body: &[u8],
        ) -> Result<(u16, Vec<u8>), ArchiveError> {
            let mut addrs = format!("{}:{}", self.host, self.port)
                .to_socket_addrs()
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            let addr = addrs
                .next()
                .ok_or_else(|| ArchiveError::Io("LocalStack host resolved to no addrs".into()))?;
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(|err| ArchiveError::Io(err.to_string()))?;

            write!(
                stream,
                "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nContent-Length: {}\r\nConnection: close\r\n",
                self.host,
                self.port,
                body.len()
            )
            .map_err(|err| ArchiveError::Io(err.to_string()))?;
            for (name, value) in headers {
                write!(stream, "{name}: {value}\r\n")
                    .map_err(|err| ArchiveError::Io(err.to_string()))?;
            }
            stream
                .write_all(b"\r\n")
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            stream
                .write_all(body)
                .map_err(|err| ArchiveError::Io(err.to_string()))?;

            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            let _ = stream.shutdown(Shutdown::Both);
            let header_end = response
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or_else(|| ArchiveError::Io("LocalStack response missing headers".into()))?;
            let headers = std::str::from_utf8(&response[..header_end])
                .map_err(|err| ArchiveError::Io(err.to_string()))?;
            let status = headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|status| status.parse::<u16>().ok())
                .ok_or_else(|| ArchiveError::Io("LocalStack response missing status".into()))?;
            Ok((status, response[(header_end + 4)..].to_vec()))
        }
    }

    impl ObjectLockStore for LocalStackS3Store {
        fn put_locked_object(
            &self,
            request: ObjectLockPut,
        ) -> Result<ObjectLockReceipt, ArchiveError> {
            let legal_hold = if request.retention.legal_hold {
                "ON"
            } else {
                "OFF"
            };
            let (status, _) = self.request(
                "PUT",
                &format!("/{}/{}", request.namespace, request.key),
                &[
                    (
                        "x-amz-object-lock-mode",
                        request.retention.mode.as_cloud_value(),
                    ),
                    (
                        "x-amz-object-lock-retain-until-date",
                        &request.retention.retain_until,
                    ),
                    ("x-amz-object-lock-legal-hold", legal_hold),
                    ("x-amz-meta-invoicekit-blake3", &request.content_hash),
                ],
                &request.body,
            )?;
            if matches!(status, 200 | 201) {
                Ok(ObjectLockReceipt { version_id: None })
            } else {
                Err(ArchiveError::Io(format!(
                    "LocalStack put object returned HTTP {status}"
                )))
            }
        }

        fn get_locked_object(&self, namespace: &str, key: &str) -> Result<Vec<u8>, ArchiveError> {
            let (status, body) = self.request("GET", &format!("/{namespace}/{key}"), &[], &[])?;
            match status {
                200 => Ok(body),
                404 => Err(ArchiveError::NotFound(format!("{namespace}/{key}"))),
                status => Err(ArchiveError::Io(format!(
                    "LocalStack get object returned HTTP {status}"
                ))),
            }
        }

        fn locked_object_exists(&self, namespace: &str, key: &str) -> bool {
            self.request("HEAD", &format!("/{namespace}/{key}"), &[], &[])
                .is_ok_and(|(status, _)| status == 200)
        }
    }

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
        // A well-formed id (64 lowercase hex chars) that was never
        // stored must surface as NotFound, not InvalidId.
        let missing = ArchiveId::new("0".repeat(64));
        let err = archive.retrieve(&missing).unwrap_err();
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
        let entry = archive.entry_path(&id).expect("stored id is well-formed");
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
    fn local_fs_archive_rejects_path_traversal_id() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        // An id that tries to climb out of the archive root must be
        // refused before any path join, on every read path.
        let escape = ArchiveId::new("../../../../../../etc/passwd");
        let err = archive.retrieve(&escape).unwrap_err();
        assert!(matches!(err, ArchiveError::InvalidId(_)), "got {err:?}");
        assert!(!archive.exists(&escape));
    }

    #[test]
    fn local_fs_archive_rejects_multibyte_id_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        // A multibyte id once panicked on the `&hex[..2]` shard
        // slice (byte index 2 lands mid-char). It must now be a
        // clean InvalidId error.
        let multibyte = ArchiveId::new("a\u{e9}foobar");
        let err = archive.retrieve(&multibyte).unwrap_err();
        assert!(matches!(err, ArchiveError::InvalidId(_)), "got {err:?}");
        assert!(!archive.exists(&multibyte));
    }

    #[test]
    fn local_fs_archive_rejects_malformed_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        // Too short, too long, uppercase, and non-hex are all
        // rejected; only 64 lowercase hex chars are valid.
        for bad in [
            "deadbeef",                                                            // too short
            &"a".repeat(63),                                                       // 63 chars
            &"a".repeat(65),                                                       // 65 chars
            "DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF",     // uppercase
            "zz00000000000000000000000000000000000000000000000000000000000000",    // non-hex
        ] {
            let id = ArchiveId::new(bad);
            let err = archive.retrieve(&id).unwrap_err();
            assert!(matches!(err, ArchiveError::InvalidId(_)), "id {bad:?} got {err:?}");
            assert!(!archive.exists(&id), "id {bad:?} should not exist");
        }
    }

    #[test]
    fn local_fs_archive_accepts_well_formed_id() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = LocalFsArchive::new(tmp.path());
        // A round-trip through store proves valid ids still reach
        // the filesystem exactly as before.
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let id = archive.store(&bundle).unwrap();
        assert_eq!(id.as_str().len(), 64);
        assert!(archive.entry_path(&id).is_ok());
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
    fn s3_object_lock_archive_round_trip_records_retention() {
        let store = InMemoryObjectLockStore::default();
        let archive = S3ObjectLockArchive::new(
            "invoicekit-archive",
            "tenant-a",
            store.clone(),
            RetentionPolicy::compliance("2036-05-27T00:00:00Z").with_legal_hold(),
        );
        let bundle = sample_bundle("2026-05-27T00:00:00Z");

        let id = archive.store(&bundle).unwrap();
        assert!(id.as_str().starts_with("s3://invoicekit-archive/tenant-a/"));
        assert!(id.as_str().contains("#versionId=v1"));
        assert!(archive.exists(&id));
        assert_eq!(archive.retrieve(&id).unwrap(), bundle);

        let key = object_from_scheme_id("s3", "invoicekit-archive", &id).unwrap();
        let object = store.object("invoicekit-archive", &key).unwrap();
        assert_eq!(object.backend, ObjectLockBackend::S3ObjectLock);
        assert_eq!(object.retention.mode, RetentionMode::Compliance);
        assert_eq!(object.retention.mode.as_cloud_value(), "COMPLIANCE");
        assert_eq!(object.retention.retain_until, "2036-05-27T00:00:00Z");
        assert!(object.retention.legal_hold);
        assert_eq!(
            object.content_hash,
            invoicekit_evidence::blake3_hex(&object.body)
        );
    }

    #[test]
    fn s3_object_lock_archive_rejects_foreign_ids() {
        let archive = S3ObjectLockArchive::new(
            "invoicekit-archive",
            "tenant-a",
            InMemoryObjectLockStore::default(),
            RetentionPolicy::governance("2030-01-01T00:00:00Z"),
        );

        let err = archive
            .retrieve(&ArchiveId::new("s3://other-bucket/tenant-a/deadbeef.ikb"))
            .unwrap_err();
        assert!(matches!(err, ArchiveError::InvalidId(_)));
        assert!(!archive.exists(&ArchiveId::new("s3://other-bucket/tenant-a/deadbeef.ikb")));
    }

    #[test]
    fn s3_object_lock_archive_localstack_smoke_when_configured() {
        let Ok(endpoint) = std::env::var("INVOICEKIT_ARCHIVE_LOCALSTACK_S3_URL") else {
            return;
        };
        let store = LocalStackS3Store::new(&endpoint).unwrap();
        let bucket = format!("invoicekit-archive-{}", std::process::id());
        store.create_object_lock_bucket(&bucket).unwrap();
        let archive = S3ObjectLockArchive::new(
            &bucket,
            "tenant-a",
            store,
            RetentionPolicy::governance("2030-05-27T00:00:00Z"),
        );
        let bundle = sample_bundle("2026-05-27T00:00:00Z");

        let id = archive.store(&bundle).unwrap();
        assert!(archive.exists(&id));
        assert_eq!(archive.retrieve(&id).unwrap(), bundle);
    }

    #[test]
    fn azure_worm_archive_round_trip_records_retention() {
        let store = InMemoryObjectLockStore::default();
        let archive = AzureWormArchive::new(
            "invoicekit-container",
            "tenant-a",
            store.clone(),
            RetentionPolicy::governance("2031-05-27T00:00:00Z"),
        );
        let bundle = sample_bundle("2026-05-27T00:00:00Z");

        let id = archive.store(&bundle).unwrap();
        assert!(id
            .as_str()
            .starts_with("azure://invoicekit-container/tenant-a/"));
        assert!(id.as_str().contains("#versionId=v1"));
        assert!(archive.exists(&id));
        assert_eq!(archive.retrieve(&id).unwrap(), bundle);

        let blob = object_from_scheme_id("azure", "invoicekit-container", &id).unwrap();
        let object = store.object("invoicekit-container", &blob).unwrap();
        assert_eq!(object.backend, ObjectLockBackend::AzureWorm);
        assert_eq!(object.retention.mode, RetentionMode::Governance);
        assert_eq!(object.retention.mode.as_cloud_value(), "GOVERNANCE");
        assert_eq!(object.retention.retain_until, "2031-05-27T00:00:00Z");
        assert!(!object.retention.legal_hold);
        assert_eq!(object.version_id, "v1");
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
