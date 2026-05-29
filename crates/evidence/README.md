# invoicekit-evidence

The `.invoicekit` / `.ikb` evidence bundle format: a deterministic zstd-compressed tar archive with a BLAKE3 manifest.

## What it does

An evidence bundle is the record of one invoice operation. It carries the canonical intermediate-representation JSON, the format artefacts (Universal Business Language, Cross Industry Invoice, Factur-X), the rendered PDF, intake source bytes, the validation trace, the rule-pack manifest, and gateway receipts. Every artefact is BLAKE3-hashed and the hashes are recorded in a `manifest.json` that ships inside the bundle. Re-pack the same bundle and you get byte-identical output, so the hash chain is stable enough to replay an audit and prove nothing changed.

This crate is the **unsigned core** of the format: pack, unpack, and verify. It does not sign anything. The signature, the RFC 3161 timestamp, and the `replay.json` execution surface layer on top in sibling crates without changing this crate's public surface.

Determinism is the whole point. `pack(bundle) == pack(bundle)` on every run and every platform, given a fixed `Manifest::created_at`. The tar entries are written in lexicographic order by path and carry normalized metadata (`uid=0, gid=0, mtime=0, mode=0644`), so neither insertion order nor the host filesystem leaks into the bytes.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

Evidence is the terminal stage. It collects the artefacts produced upstream and freezes them under a hash ledger. Downstream of this crate:

- `invoicekit-evidence-dsse` wraps `manifest.json` in a DSSE envelope, written back as the `signatures/manifest.dsse` sidecar artefact.
- `invoicekit-signer` mints that signature.
- `invoicekit-timestamping` binds an RFC 3161 timestamp.
- `invoicekit-verify` re-hashes, re-checks the signature, and re-validates the timestamp.
- `invoicekit-replay` re-runs the recorded pipeline from a bundle and reports byte-equality against the recorded outputs.

## Public API

Types:

- `EvidenceBundle` — a `Manifest` plus an artefact payload map (`BTreeMap<String, Vec<u8>>`, keyed by artefact id; the `BTreeMap` ordering is what makes packing deterministic).
- `Manifest` — `schema_version`, `created_at`, `tenant_id`, `trace_id`, and the lexicographically ordered `artefacts: Vec<ArtefactEntry>`.
- `ArtefactEntry` — one ledger row: `id`, `size`, `blake3_hex`, and an optional `content_type` hint that never affects the hash.
- `BundleError` — the error enum (`HashDrift`, `MissingArtefact`, `UnknownArtefact`, `InvalidArtefactPath`, `UnsupportedSchema`, and so on).

Functions:

- `pack(&EvidenceBundle) -> Result<Vec<u8>, BundleError>` — serialize to bit-stable `.ikb` bytes. Re-emits the manifest as the reserved `manifest.json` entry; callers cannot override it.
- `unpack(&[u8]) -> Result<EvidenceBundle, BundleError>` — decode container bytes back into a typed bundle. Runs `verify` before returning.
- `verify(&EvidenceBundle) -> Result<(), BundleError>` — re-hash every artefact and reject any drift, missing payload, or unlisted payload.
- `manifest_for(...) -> Manifest` — build a fresh manifest from a payload map, computing every size and hash.
- `blake3_hex(&[u8]) -> String` — BLAKE3 of the bytes, lowercase hex.

Constants:

- `SCHEMA_VERSION` (`"1.0"`) — embedded in every manifest, bumped only on breaking container changes.
- `MANIFEST_ARTEFACT_ID` (`"manifest.json"`) — the reserved manifest entry id.
- `MANIFEST_SIGNATURE_ARTEFACT_ID` (`"signatures/manifest.dsse"`) — the signature sidecar id. It is deliberately kept out of `Manifest::artefacts`, because the envelope signs `manifest.json` and including the envelope's own hash there would make the signed payload self-referential. `verify` tolerates it as an unlisted artefact; higher-level checks validate it explicitly.

Artefact ids are validated as relative, forward-slash paths. Absolute paths, `..`, empty segments, Windows separators, and non-UTF-8 bytes are rejected with `BundleError::InvalidArtefactPath`.

## Usage

```rust
use std::collections::BTreeMap;
use invoicekit_evidence::{manifest_for, pack, unpack, EvidenceBundle};

let mut artefacts = BTreeMap::new();
artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
artefacts.insert("receipts/peppol.json".to_owned(), br#"{"message_id":"msg-1"}"#.to_vec());

// created_at is caller-pinned so re-packing stays byte-identical.
let manifest = manifest_for(&artefacts, "tenant-a", "trace-xyz", "2026-05-27T00:00:00Z");
let bundle = EvidenceBundle { manifest, artefacts };

let packed = pack(&bundle).unwrap();          // bit-stable .ikb bytes
let restored = unpack(&packed).unwrap();      // verifies hashes on the way in
assert_eq!(restored, bundle);
```

## License

Apache-2.0. Part of the InvoiceKit workspace.
