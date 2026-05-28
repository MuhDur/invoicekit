// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-evidence` — signed evidence bundle format
//! (`.invoicekit` directory tree / `.ikb` portable archive).
//!
//! The bundle is the deterministic record of every invoice
//! operation: the canonical IR JSON, the format artefacts
//! (UBL/CII/Factur-X), the PDF/A-3, intake source bytes, the
//! validation trace, the rule-pack manifest, gateway receipts,
//! and a `replay.json` the future `invoicekit verify` CLI
//! (T-084) re-executes. Every artefact is BLAKE3-hashed; the
//! hashes are recorded in the manifest, which is itself the
//! input to the T-083 signing substrate.
//!
//! This crate ships the **unsigned core** of the format.
//!
//! Public surface:
//! [`EvidenceBundle`], [`Manifest`], [`pack`], [`unpack`],
//! [`verify`].
//!
//! Signing (T-083), RFC 3161 timestamping (T-082), and the
//! `replay.json` execution surface (T-085) layer on top
//! without modifying this crate's public surface.
//!
//! # Determinism guarantee
//!
//! `pack(bundle) == pack(bundle)` on every run, on every
//! platform, given a bundle whose [`Manifest::created_at`] is
//! held constant. The container header carries no timestamps,
//! no host metadata, and writes artefacts in lexicographic
//! order by name. This is the bit-stability the
//! `plans/PLAN.md` §2.10 contract requires for hash-stable
//! audit replay.

use std::collections::BTreeMap;
use std::io::Read;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema version embedded in every manifest. Bumped only on
/// breaking changes to the container or the manifest shape.
pub const SCHEMA_VERSION: &str = "1.0";

/// Container magic bytes — `b"IKB1"` (InvoiceKit Bundle, v1).
pub const MAGIC: [u8; 4] = *b"IKB1";

/// Reserved artefact id for the bundle manifest. The manifest
/// is itself an artefact so the signing substrate (T-083) can
/// hash + sign the whole bundle by hashing this single entry.
pub const MANIFEST_ARTEFACT_ID: &str = "manifest.json";

/// One artefact entry in the bundle ledger.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtefactEntry {
    /// Lexicographically-sortable artefact id, e.g.
    /// `canonical.json`, `formats/ubl.xml`, `intake/source.pdf`.
    pub id: String,
    /// Artefact length in bytes.
    pub size: u64,
    /// BLAKE3 hash of the artefact bytes, lowercase hex.
    pub blake3_hex: String,
    /// Optional content-type hint for tooling; never affects
    /// the hash or the determinism of [`pack`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// BLAKE3 ledger that locks every artefact in the bundle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version.
    pub schema_version: String,
    /// RFC-3339 UTC timestamp the manifest was first emitted.
    /// The caller chooses this value (the signer typically
    /// pins it to the transmission attempt time) so that the
    /// same bundle re-packed at a later moment stays
    /// byte-identical.
    pub created_at: String,
    /// Tenant identifier, mirrored from the gateway context.
    pub tenant_id: String,
    /// Trace identifier, mirrored from the gateway context.
    pub trace_id: String,
    /// Artefact entries, ordered lexicographically by `id`.
    pub artefacts: Vec<ArtefactEntry>,
}

/// In-memory representation of one evidence bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceBundle {
    /// Bundle manifest. [`pack`] re-emits the manifest as an
    /// artefact named [`MANIFEST_ARTEFACT_ID`] so the signer
    /// hash covers it.
    pub manifest: Manifest,
    /// Artefact payloads keyed by id. The map is `BTreeMap`
    /// because lexicographic ordering is what makes [`pack`]
    /// deterministic.
    pub artefacts: BTreeMap<String, Vec<u8>>,
}

/// Errors returned by the evidence bundle codec.
#[derive(Debug, Error)]
pub enum BundleError {
    /// Container header magic did not match [`MAGIC`].
    #[error("bundle magic mismatch: got {0:?}")]
    BadMagic([u8; 4]),
    /// Container header advertises a schema version this build
    /// does not understand.
    #[error("bundle schema version {0} unsupported by this build")]
    UnsupportedSchema(String),
    /// Container ended before the declared payload length was
    /// consumed.
    #[error("bundle truncated: expected {expected} bytes, found {found}")]
    Truncated {
        /// Bytes the header promised.
        expected: u64,
        /// Bytes the reader actually saw.
        found: u64,
    },
    /// A bundle entry's recorded hash did not match the
    /// re-computed BLAKE3 hash of its bytes.
    #[error("bundle artefact {id} hash drift: manifest={manifest_hex}, observed={observed_hex}")]
    HashDrift {
        /// Artefact id that drifted.
        id: String,
        /// Hash recorded in the manifest.
        manifest_hex: String,
        /// Hash observed when re-hashing the bytes.
        observed_hex: String,
    },
    /// Artefact bytes for a manifest entry were absent.
    #[error("bundle artefact {0} listed in manifest but missing from container")]
    MissingArtefact(String),
    /// Artefact bytes were present but the manifest did not
    /// list them.
    #[error("bundle artefact {0} present but not listed in manifest")]
    UnknownArtefact(String),
    /// The manifest artefact itself was missing from the
    /// container.
    #[error("bundle missing manifest artefact")]
    MissingManifest,
    /// The manifest JSON did not parse.
    #[error("bundle manifest JSON parse failure: {0}")]
    BadManifestJson(String),
    /// IO error during pack/unpack.
    #[error("bundle io error: {0}")]
    Io(String),
}

/// Compute the BLAKE3 hash of the given bytes as lowercase hex.
#[must_use]
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Build a fresh manifest from a payload map. Hash + size are
/// computed from the payload; the manifest entries are sorted
/// lexicographically by id for determinism.
#[must_use]
pub fn manifest_for(
    payloads: &BTreeMap<String, Vec<u8>>,
    tenant_id: impl Into<String>,
    trace_id: impl Into<String>,
    created_at: impl Into<String>,
) -> Manifest {
    let artefacts = payloads
        .iter()
        .map(|(id, bytes)| ArtefactEntry {
            id: id.clone(),
            size: bytes.len() as u64,
            blake3_hex: blake3_hex(bytes),
            content_type: None,
        })
        .collect();
    Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        created_at: created_at.into(),
        tenant_id: tenant_id.into(),
        trace_id: trace_id.into(),
        artefacts,
    }
}

/// Pack a bundle into bit-stable container bytes.
///
/// Container layout:
///
/// ```text
///   [magic: 4 bytes]            // b"IKB1"
///   [schema_version_len: u8]
///   [schema_version: bytes]
///   [entry_count: u32 LE]
///   --- per entry, sorted by id ---
///     [id_len: u16 LE]
///     [id: bytes]
///     [data_len: u64 LE]
///     [data: bytes]
/// ```
///
/// The manifest is serialised as `manifest.json` and inserted
/// alongside the other artefacts; readers can pull it back via
/// [`unpack`] and re-verify every payload through [`verify`].
///
/// # Errors
///
/// Returns [`BundleError::BadManifestJson`] when the manifest
/// fails to serialise.
pub fn pack(bundle: &EvidenceBundle) -> Result<Vec<u8>, BundleError> {
    let manifest_bytes = serde_json::to_vec(&bundle.manifest)
        .map_err(|e| BundleError::BadManifestJson(e.to_string()))?;

    // Build a sorted view of (id, bytes) that includes the
    // manifest at the canonical entry id. We deliberately do
    // not allow callers to override the manifest entry in
    // `bundle.artefacts`.
    let mut entries: Vec<(String, &[u8])> = bundle
        .artefacts
        .iter()
        .filter(|(id, _)| id.as_str() != MANIFEST_ARTEFACT_ID)
        .map(|(id, bytes)| (id.clone(), bytes.as_slice()))
        .collect();
    entries.push((MANIFEST_ARTEFACT_ID.to_owned(), manifest_bytes.as_slice()));
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out: Vec<u8> = Vec::with_capacity(
        16 + entries
            .iter()
            .map(|(id, data)| id.len() + data.len() + 10)
            .sum::<usize>(),
    );
    out.extend_from_slice(&MAGIC);
    let schema_bytes = SCHEMA_VERSION.as_bytes();
    let schema_len = u8::try_from(schema_bytes.len())
        .map_err(|_| BundleError::Io("schema version too long".to_owned()))?;
    out.push(schema_len);
    out.extend_from_slice(schema_bytes);
    let entry_count = u32::try_from(entries.len())
        .map_err(|_| BundleError::Io("entry count overflows u32".to_owned()))?;
    out.extend_from_slice(&entry_count.to_le_bytes());
    for (id, data) in entries {
        let id_len = u16::try_from(id.len())
            .map_err(|_| BundleError::Io(format!("entry id `{id}` too long")))?;
        out.extend_from_slice(&id_len.to_le_bytes());
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    }
    Ok(out)
}

/// Unpack container bytes into an [`EvidenceBundle`].
///
/// The manifest is parsed back into the typed [`Manifest`];
/// every listed artefact is re-located in the container and
/// verified against the manifest's recorded hash via [`verify`].
///
/// # Errors
///
/// Returns [`BundleError`] when the container is malformed,
/// truncated, or carries hash drift.
pub fn unpack(bytes: &[u8]) -> Result<EvidenceBundle, BundleError> {
    let mut reader = Cursor::new(bytes);
    let mut magic = [0_u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|e| BundleError::Io(e.to_string()))?;
    if magic != MAGIC {
        return Err(BundleError::BadMagic(magic));
    }
    let mut schema_len = [0_u8; 1];
    reader
        .read_exact(&mut schema_len)
        .map_err(|e| BundleError::Io(e.to_string()))?;
    let mut schema_bytes = vec![0_u8; schema_len[0] as usize];
    reader
        .read_exact(&mut schema_bytes)
        .map_err(|e| BundleError::Io(e.to_string()))?;
    let schema = String::from_utf8(schema_bytes)
        .map_err(|e| BundleError::Io(format!("schema version is not UTF-8: {e}")))?;
    if schema != SCHEMA_VERSION {
        return Err(BundleError::UnsupportedSchema(schema));
    }
    let mut entry_count_bytes = [0_u8; 4];
    reader
        .read_exact(&mut entry_count_bytes)
        .map_err(|e| BundleError::Io(e.to_string()))?;
    let entry_count = u32::from_le_bytes(entry_count_bytes);

    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for _ in 0..entry_count {
        let mut id_len_bytes = [0_u8; 2];
        reader
            .read_exact(&mut id_len_bytes)
            .map_err(|e| BundleError::Io(e.to_string()))?;
        let id_len = u16::from_le_bytes(id_len_bytes);
        let mut id_bytes = vec![0_u8; id_len as usize];
        reader
            .read_exact(&mut id_bytes)
            .map_err(|e| BundleError::Io(e.to_string()))?;
        let id = String::from_utf8(id_bytes)
            .map_err(|e| BundleError::Io(format!("entry id is not UTF-8: {e}")))?;
        let mut data_len_bytes = [0_u8; 8];
        reader
            .read_exact(&mut data_len_bytes)
            .map_err(|e| BundleError::Io(e.to_string()))?;
        let data_len = u64::from_le_bytes(data_len_bytes);
        let data_len_usize = usize::try_from(data_len).map_err(|_| {
            BundleError::Io(format!("entry data length {data_len} overflows usize"))
        })?;
        let mut data = vec![0_u8; data_len_usize];
        reader
            .read_exact(&mut data)
            .map_err(|_| BundleError::Truncated {
                expected: data_len,
                found: 0,
            })?;
        if artefacts.insert(id.clone(), data).is_some() {
            return Err(BundleError::Io(format!("duplicate artefact id: {id}")));
        }
    }

    let manifest_bytes = artefacts
        .remove(MANIFEST_ARTEFACT_ID)
        .ok_or(BundleError::MissingManifest)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BundleError::BadManifestJson(e.to_string()))?;
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    verify(&bundle)?;
    Ok(bundle)
}

/// Re-hash every artefact and reject any drift.
///
/// Compares the manifest's recorded BLAKE3 hashes against the
/// observed payload hashes. Run automatically by [`unpack`];
/// callers should run it again after any in-place edit before
/// re-[`pack`]ing.
///
/// # Errors
///
/// Returns [`BundleError::HashDrift`] / [`BundleError::MissingArtefact`]
/// / [`BundleError::UnknownArtefact`] when the manifest and
/// payload disagree.
pub fn verify(bundle: &EvidenceBundle) -> Result<(), BundleError> {
    // Build a quick lookup of manifest entries by id.
    let mut manifest_map: BTreeMap<&str, &ArtefactEntry> = bundle
        .manifest
        .artefacts
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    // The manifest entry itself is not in `bundle.artefacts`;
    // its hash is implicit in the wire format (the manifest
    // bytes are re-derived from the typed Manifest on every
    // pack). We therefore do not require the manifest to list
    // itself.
    manifest_map.remove(MANIFEST_ARTEFACT_ID);
    for (id, bytes) in &bundle.artefacts {
        let Some(entry) = manifest_map.remove(id.as_str()) else {
            return Err(BundleError::UnknownArtefact(id.clone()));
        };
        if entry.size != bytes.len() as u64 {
            return Err(BundleError::HashDrift {
                id: id.clone(),
                manifest_hex: format!("size={}", entry.size),
                observed_hex: format!("size={}", bytes.len()),
            });
        }
        let observed = blake3_hex(bytes);
        if observed != entry.blake3_hex {
            return Err(BundleError::HashDrift {
                id: id.clone(),
                manifest_hex: entry.blake3_hex.clone(),
                observed_hex: observed,
            });
        }
    }
    if let Some(missing) = manifest_map.keys().next() {
        return Err(BundleError::MissingArtefact((*missing).to_owned()));
    }
    Ok(())
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_evidence::crate_name(), "invoicekit-evidence");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-evidence"
}

// Tiny `io::Cursor` wrapper used by `unpack`. The standard
// `std::io::Cursor` would work too, but this lets us keep
// the parser surface uniform with the bundle's bespoke layout
// (`Read::read_exact` is the only thing we need).
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.bytes.len().saturating_sub(self.pos);
        let n = remaining.min(buf.len());
        buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle(created_at: &str) -> EvidenceBundle {
        let mut artefacts = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        artefacts.insert(
            "formats/cii.xml".to_owned(),
            b"<CrossIndustryInvoice/>".to_vec(),
        );
        artefacts.insert(
            "receipts/peppol.json".to_owned(),
            br#"{"message_id":"msg-1"}"#.to_vec(),
        );
        let manifest = manifest_for(&artefacts, "tenant-a", "trace-xyz", created_at);
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-evidence");
    }

    #[test]
    fn pack_unpack_round_trip() {
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let packed = pack(&bundle).unwrap();
        let unpacked = unpack(&packed).unwrap();
        assert_eq!(unpacked, bundle);
    }

    #[test]
    fn pack_is_byte_stable_across_runs() {
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let a = pack(&bundle).unwrap();
        let b = pack(&bundle).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with(&MAGIC));
    }

    #[test]
    fn pack_is_byte_stable_regardless_of_insert_order() {
        let mut a = sample_bundle("2026-05-27T00:00:00Z");
        // Rebuild the bundle's payload map in reverse insertion
        // order; BTreeMap normalises, but ensure the packed
        // bytes stay equal.
        let mut reversed = BTreeMap::new();
        for (id, bytes) in a.artefacts.iter().rev() {
            reversed.insert(id.clone(), bytes.clone());
        }
        a.artefacts = reversed;
        a.manifest = manifest_for(
            &a.artefacts,
            &a.manifest.tenant_id,
            &a.manifest.trace_id,
            &a.manifest.created_at,
        );
        let b = sample_bundle("2026-05-27T00:00:00Z");
        assert_eq!(pack(&a).unwrap(), pack(&b).unwrap());
    }

    #[test]
    fn unpack_rejects_bad_magic() {
        let mut bytes = pack(&sample_bundle("2026-05-27T00:00:00Z")).unwrap();
        bytes[0] = b'X';
        let err = unpack(&bytes).unwrap_err();
        assert!(matches!(err, BundleError::BadMagic(_)));
    }

    #[test]
    fn unpack_rejects_unsupported_schema() {
        // Forge bytes that carry schema "9.9".
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(3);
        bytes.extend_from_slice(b"9.9");
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        let err = unpack(&bytes).unwrap_err();
        match err {
            BundleError::UnsupportedSchema(v) => assert_eq!(v, "9.9"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn verify_catches_tampered_artefact() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        // Flip one byte of the canonical JSON without updating
        // the manifest hash.
        let payload = bundle.artefacts.get_mut("canonical.json").unwrap();
        payload[1] = b'!';
        let err = verify(&bundle).unwrap_err();
        match err {
            BundleError::HashDrift { id, .. } => assert_eq!(id, "canonical.json"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn verify_catches_payload_absent_from_bundle() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle.artefacts.remove("formats/cii.xml");
        let err = verify(&bundle).unwrap_err();
        match err {
            BundleError::MissingArtefact(id) => assert_eq!(id, "formats/cii.xml"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn verify_catches_payload_not_listed_in_manifest() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle
            .artefacts
            .insert("formats/unlisted.xml".to_owned(), b"<x/>".to_vec());
        let err = verify(&bundle).unwrap_err();
        match err {
            BundleError::UnknownArtefact(id) => assert_eq!(id, "formats/unlisted.xml"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unpack_rejects_truncated_container() {
        let bytes = pack(&sample_bundle("2026-05-27T00:00:00Z")).unwrap();
        let truncated = &bytes[..bytes.len() - 4];
        let err = unpack(truncated).unwrap_err();
        // Truncating the final payload causes Io or Truncated.
        assert!(matches!(
            err,
            BundleError::Io(_) | BundleError::Truncated { .. }
        ));
    }

    #[test]
    fn manifest_artefacts_sort_lexicographically() {
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        let ids: Vec<&str> = bundle
            .manifest
            .artefacts
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn blake3_hex_known_vector() {
        // Known BLAKE3 hash of the empty input.
        assert_eq!(
            blake3_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
