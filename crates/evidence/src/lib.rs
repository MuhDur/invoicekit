// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-evidence` — signed evidence bundle format
//! (`.invoicekit` directory tree / `.ikb` tar.zst portable archive).
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
//! held constant. The `.ikb` form is a zstd-compressed tar
//! archive whose entries carry normalized uid/gid/mtime/mode
//! metadata and are written in lexicographic order by path.
//! This is the bit-stability the `plans/PLAN.md` §2.10 and
//! §4.7 contracts require for hash-stable audit replay.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema version embedded in every manifest. Bumped only on
/// breaking changes to the container or the manifest shape.
pub const SCHEMA_VERSION: &str = "1.0";

/// Reserved artefact id for the bundle manifest. The manifest
/// is itself an artefact so the signing substrate (T-083) can
/// hash + sign the whole bundle by hashing this single entry.
pub const MANIFEST_ARTEFACT_ID: &str = "manifest.json";

/// Hard cap on the number of decompressed bytes [`unpack`] will
/// admit from a `.ikb` container, in bytes (512 MiB).
///
/// `unpack` runs zstd decompression on attacker-supplied bytes.
/// Without a cap, a small but pathologically-compressible input
/// (a "zip bomb" / decompression bomb) could expand to an
/// unbounded buffer and exhaust process memory. Decompression is
/// therefore performed through a size-limited reader and aborted
/// with [`BundleError::DecompressionTooLarge`] the moment the
/// decompressed stream exceeds this limit.
///
/// The cap is deliberately generous: a real evidence bundle
/// carries the canonical invoice, format artefacts, a PDF/A-3,
/// intake source bytes, the validation trace, and gateway
/// receipts, all comfortably under half a gigabyte. Callers that
/// genuinely need larger bundles can decompress with their own
/// bounds before re-deriving the typed [`EvidenceBundle`].
pub const MAX_DECOMPRESSED_SIZE: u64 = 512 * 1024 * 1024;

/// Reserved artefact id for the manifest DSSE sidecar.
///
/// This artefact is deliberately outside [`Manifest::artefacts`]:
/// the envelope signs `manifest.json`, so including the
/// envelope's own hash inside that same manifest would make the
/// signed payload self-referential. Higher-level verification
/// checks validate this sidecar explicitly.
pub const MANIFEST_SIGNATURE_ARTEFACT_ID: &str = "signatures/manifest.dsse";

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
    /// The `manifest.json` advertises a schema version this
    /// build does not understand.
    #[error("bundle schema version {0} unsupported by this build")]
    UnsupportedSchema(String),
    /// A tar entry path was absolute, contained `..`, was empty,
    /// used a Windows separator, or was not valid UTF-8.
    #[error("bundle artefact path is invalid: {0}")]
    InvalidArtefactPath(String),
    /// The `.ikb` tar contained an entry type this reader does
    /// not support.
    #[error("bundle artefact {id} has unsupported tar entry type {entry_type}")]
    UnsupportedTarEntry {
        /// Artefact path.
        id: String,
        /// Tar entry type byte rendered for diagnostics.
        entry_type: String,
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
    /// IO, tar, or zstd error during pack/unpack.
    #[error("bundle io error: {0}")]
    Io(String),
    /// The container decompressed to more bytes than
    /// [`MAX_DECOMPRESSED_SIZE`] allows. Guards against
    /// decompression-bomb / unbounded-memory denial of service.
    #[error(
        "bundle decompressed size exceeds the {limit}-byte cap; refusing to buffer a possible decompression bomb"
    )]
    DecompressionTooLarge {
        /// The cap that was exceeded, in bytes.
        limit: u64,
    },
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

/// Pack a bundle into bit-stable `.ikb` bytes.
///
/// The portable form is a zstd-compressed tar archive. Every
/// entry is a regular file; entries are sorted by artefact id
/// and carry normalized tar metadata:
///
/// ```text
/// uid=0, gid=0, mtime=0, mode=0644
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

    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        for (id, data) in entries {
            append_tar_entry(&mut builder, &id, data)?;
        }
        builder
            .finish()
            .map_err(|e| BundleError::Io(e.to_string()))?;
    }
    zstd::stream::encode_all(Cursor::new(tar_bytes), 0).map_err(|e| BundleError::Io(e.to_string()))
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
    let decoded = decode_all_capped(bytes, MAX_DECOMPRESSED_SIZE)?;
    let mut archive = tar::Archive::new(Cursor::new(decoded));
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for entry in archive
        .entries()
        .map_err(|e| BundleError::Io(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| BundleError::Io(e.to_string()))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        let path = entry.path().map_err(|e| BundleError::Io(e.to_string()))?;
        let id = artefact_id_from_path(&path)?;
        if !entry_type.is_file() {
            return Err(BundleError::UnsupportedTarEntry {
                id,
                entry_type: format!("{entry_type:?}"),
            });
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| BundleError::Io(e.to_string()))?;
        if artefacts.insert(id.clone(), data).is_some() {
            return Err(BundleError::Io(format!("duplicate artefact id: {id}")));
        }
    }

    let manifest_bytes = artefacts
        .remove(MANIFEST_ARTEFACT_ID)
        .ok_or(BundleError::MissingManifest)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| BundleError::BadManifestJson(e.to_string()))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(BundleError::UnsupportedSchema(manifest.schema_version));
    }
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
    if bundle.manifest.schema_version != SCHEMA_VERSION {
        return Err(BundleError::UnsupportedSchema(
            bundle.manifest.schema_version.clone(),
        ));
    }

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
            if id.eq(MANIFEST_SIGNATURE_ARTEFACT_ID) {
                continue;
            }
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

/// Decompress a zstd stream while refusing to buffer more than
/// `limit` bytes.
///
/// The decoder is read through a [`Read::take`] reader bounded at
/// `limit + 1`, so reading stops the instant the cap is crossed
/// instead of materialising the full (potentially enormous)
/// output. If the bounded read yields more than `limit` bytes,
/// the input is treated as a decompression bomb and rejected.
fn decode_all_capped(bytes: &[u8], limit: u64) -> Result<Vec<u8>, BundleError> {
    let decoder =
        zstd::stream::read::Decoder::new(Cursor::new(bytes)).map_err(|e| BundleError::Io(e.to_string()))?;
    // Read one byte past the cap so an output that lands exactly
    // on `limit` is accepted while anything larger is detected.
    let read_budget = limit.saturating_add(1);
    let mut limited = decoder.take(read_budget);
    let mut out = Vec::new();
    limited
        .read_to_end(&mut out)
        .map_err(|e| BundleError::Io(e.to_string()))?;
    if out.len() as u64 > limit {
        return Err(BundleError::DecompressionTooLarge { limit });
    }
    Ok(out)
}

fn append_tar_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    id: &str,
    data: &[u8],
) -> Result<(), BundleError> {
    validate_artefact_id(id)?;
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_cksum();
    builder
        .append_data(&mut header, id, Cursor::new(data))
        .map_err(|e| BundleError::Io(e.to_string()))
}

fn validate_artefact_id(id: &str) -> Result<(), BundleError> {
    if id.is_empty() || id.contains('\\') {
        return Err(BundleError::InvalidArtefactPath(id.to_owned()));
    }
    let normalized = artefact_id_from_path(Path::new(id))?;
    if normalized == id {
        Ok(())
    } else {
        Err(BundleError::InvalidArtefactPath(id.to_owned()))
    }
}

fn artefact_id_from_path(path: &Path) -> Result<String, BundleError> {
    let invalid = || BundleError::InvalidArtefactPath(path.to_string_lossy().into_owned());
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let Some(part) = part.to_str() else {
                    return Err(invalid());
                };
                if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
                    return Err(invalid());
                }
                parts.push(part.to_owned());
            }
            Component::CurDir => {}
            _ => {
                return Err(invalid());
            }
        }
    }
    if parts.is_empty() {
        return Err(invalid());
    }
    Ok(parts.join("/"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>, u64, u64, u64, u32)> {
        let decoded = zstd::stream::decode_all(Cursor::new(bytes)).unwrap();
        let mut archive = tar::Archive::new(Cursor::new(decoded));
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let path = entry.path().unwrap().to_string_lossy().into_owned();
                let uid = entry.header().uid().unwrap();
                let gid = entry.header().gid().unwrap();
                let mtime = entry.header().mtime().unwrap();
                let mode = entry.header().mode().unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (path, data, uid, gid, mtime, mode)
            })
            .collect()
    }

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
        let entries = tar_entries(&a);
        let ids: Vec<&str> = entries.iter().map(|(id, ..)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "canonical.json",
                "formats/cii.xml",
                "formats/ubl.xml",
                "manifest.json",
                "receipts/peppol.json"
            ]
        );
    }

    #[test]
    fn pack_normalizes_tar_metadata() {
        let bundle = sample_bundle("2026-05-27T00:00:00Z");
        for (_, _, uid, gid, mtime, mode) in tar_entries(&pack(&bundle).unwrap()) {
            assert_eq!(uid, 0);
            assert_eq!(gid, 0);
            assert_eq!(mtime, 0);
            assert_eq!(mode, 0o644);
        }
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
    fn unpack_rejects_bad_zstd() {
        let mut bytes = pack(&sample_bundle("2026-05-27T00:00:00Z")).unwrap();
        bytes[0] = b'X';
        let err = unpack(&bytes).unwrap_err();
        assert!(matches!(err, BundleError::Io(_)));
    }

    #[test]
    fn unpack_rejects_unsupported_schema() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle.manifest.schema_version = "9.9".to_owned();
        let bytes = pack(&bundle).unwrap();
        let err = unpack(&bytes).unwrap_err();
        assert!(matches!(
            err,
            BundleError::UnsupportedSchema(ref v) if v == "9.9"
        ));
    }

    #[test]
    fn verify_rejects_unsupported_schema() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle.manifest.schema_version = "9.9".to_owned();
        let err = verify(&bundle).unwrap_err();
        assert!(matches!(
            err,
            BundleError::UnsupportedSchema(ref v) if v == "9.9"
        ));
    }

    #[test]
    fn verify_catches_tampered_artefact() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        // Flip one byte of the canonical JSON without updating
        // the manifest hash.
        let payload = bundle.artefacts.get_mut("canonical.json").unwrap();
        payload[1] = b'!';
        let err = verify(&bundle).unwrap_err();
        assert!(matches!(
            err,
            BundleError::HashDrift { ref id, .. } if id == "canonical.json"
        ));
    }

    #[test]
    fn verify_catches_payload_absent_from_bundle() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle.artefacts.remove("formats/cii.xml");
        let err = verify(&bundle).unwrap_err();
        assert!(matches!(
            err,
            BundleError::MissingArtefact(ref id) if id == "formats/cii.xml"
        ));
    }

    #[test]
    fn verify_catches_payload_not_listed_in_manifest() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle
            .artefacts
            .insert("formats/unlisted.xml".to_owned(), b"<x/>".to_vec());
        let err = verify(&bundle).unwrap_err();
        assert!(matches!(
            err,
            BundleError::UnknownArtefact(ref id) if id == "formats/unlisted.xml"
        ));
    }

    #[test]
    fn verify_allows_unlisted_manifest_signature_sidecar() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle.artefacts.insert(
            MANIFEST_SIGNATURE_ARTEFACT_ID.to_owned(),
            br#"{"payload":"e30=","payloadType":"application/vnd.invoicekit.manifest+json","signatures":[]}"#.to_vec(),
        );
        verify(&bundle).unwrap();

        let packed = pack(&bundle).unwrap();
        let unpacked = unpack(&packed).unwrap();
        assert_eq!(unpacked, bundle);
    }

    #[test]
    fn unpack_rejects_truncated_container() {
        let bytes = pack(&sample_bundle("2026-05-27T00:00:00Z")).unwrap();
        let truncated = &bytes[..bytes.len() - 4];
        let err = unpack(truncated).unwrap_err();
        assert!(matches!(err, BundleError::Io(_)));
    }

    #[test]
    fn pack_rejects_unsafe_paths() {
        let mut bundle = sample_bundle("2026-05-27T00:00:00Z");
        bundle
            .artefacts
            .insert("../escape".to_owned(), b"x".to_vec());
        let err = pack(&bundle).unwrap_err();
        assert!(matches!(err, BundleError::InvalidArtefactPath(_)));
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
    fn decode_all_capped_rejects_decompression_bomb() {
        // A few kilobytes of zstd that inflate to ~8 MiB: a
        // miniature decompression bomb. Decoded against a 1 MiB
        // cap, it must be rejected without buffering the whole
        // 8 MiB output.
        let inflated = vec![0_u8; 8 * 1024 * 1024];
        let compressed = zstd::stream::encode_all(Cursor::new(inflated), 0)
            .expect("compress zero payload");
        assert!(
            compressed.len() < 64 * 1024,
            "bomb should compress tiny, got {} bytes",
            compressed.len()
        );
        let err = decode_all_capped(&compressed, 1024 * 1024)
            .expect_err("oversize payload must be rejected");
        assert!(matches!(
            err,
            BundleError::DecompressionTooLarge { limit } if limit == 1024 * 1024
        ));
    }

    #[test]
    fn decode_all_capped_accepts_payload_at_or_under_cap() {
        // Exactly the cap and just under both decode cleanly.
        let payload = vec![7_u8; 4096];
        let compressed = zstd::stream::encode_all(Cursor::new(&payload), 0)
            .expect("compress payload");
        let under = decode_all_capped(&compressed, 4096).expect("payload at cap decodes");
        assert_eq!(under, payload);
        let also = decode_all_capped(&compressed, 8192).expect("payload under cap decodes");
        assert_eq!(also, payload);
    }

    #[test]
    fn decode_all_capped_rejects_one_byte_over_cap() {
        // Boundary: a payload one byte larger than the cap is the
        // smallest case the limit must reject.
        let payload = vec![3_u8; 4097];
        let compressed = zstd::stream::encode_all(Cursor::new(&payload), 0)
            .expect("compress payload");
        let err = decode_all_capped(&compressed, 4096)
            .expect_err("payload one byte over the cap must be rejected");
        assert!(matches!(
            err,
            BundleError::DecompressionTooLarge { limit } if limit == 4096
        ));
    }

    #[test]
    fn unpack_rejects_oversize_via_default_cap() {
        // End-to-end: a hand-rolled zstd frame that inflates past
        // MAX_DECOMPRESSED_SIZE must be refused by unpack before
        // tar parsing, surfacing DecompressionTooLarge rather than
        // exhausting memory.
        let cap = usize::try_from(MAX_DECOMPRESSED_SIZE).expect("cap fits in usize");
        let inflated = vec![0_u8; cap + 1];
        let compressed =
            zstd::stream::encode_all(Cursor::new(inflated), 0).expect("compress bomb");
        let err = unpack(&compressed).expect_err("oversize container must be rejected");
        assert!(matches!(
            err,
            BundleError::DecompressionTooLarge { limit } if limit == MAX_DECOMPRESSED_SIZE
        ));
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
