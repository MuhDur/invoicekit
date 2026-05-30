<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-archive

Pluggable storage backend for packed `.ikb` evidence bundles. Defines one `Archive` trait and four implementations.

## What it does

After the evidence stage packs an invoice operation into a `.ikb` bundle, something has to store it and hand back a stable identifier the gateway receipt can record for replay. This crate is that something. It defines the `Archive` trait — `store`, `retrieve`, `exists` — so a tenant can swap storage backends without touching call sites, and ships four implementations.

Every backend packs the bundle through `invoicekit-evidence::pack`, content-addresses it by the bundle's BLAKE3 hash, and verifies on the way back out by running `invoicekit-evidence::unpack`. The crate stores and retrieves bytes; it does not sign, encrypt, or timestamp them. The hash that makes a bundle tamper-evident is computed by `invoicekit-evidence`, not here.

## Capabilities

The `Archive` trait:

- `store(&EvidenceBundle) -> Result<ArchiveId, ArchiveError>` — pack and persist; returns the backend's id for the bundle.
- `retrieve(&ArchiveId) -> Result<EvidenceBundle, ArchiveError>` — read the bytes back and `unpack` them. Bytes that fail verification surface as `ArchiveError::Drift`.
- `exists(&ArchiveId) -> bool` — cheap probe before pulling a whole bundle.

`ArchiveId` is an opaque, stable string newtype. Each backend picks its own scheme.

Backends:

- `LocalFsArchive` — content-addressable directory on disk. The id is the bundle's BLAKE3 hash (64 lowercase hex chars). Files are sharded by the first two hex chars (`<root>/<ab>/<abcdef…>.ikb`). Writes are atomic: bytes are staged in a sibling `.ikb.tmp` file, `sync_all`'d, then renamed into place, so a reader never sees a half-written file. Identical bundle bytes deduplicate to the same id. The parent directory is created on first `store`.
- `S3ObjectLockArchive<S>` — stores `<prefix>/<blake3>.ikb` objects through an `ObjectLockStore` adapter and requests an Object Lock retention policy. Ids take the form `s3://<bucket>/<key>#versionId=<version>` when the adapter returns a version id, `s3://<bucket>/<key>` otherwise.
- `AzureWormArchive<S>` — same contract against an Azure WORM blob store; ids use the `azure://<container>/<blob>#versionId=<version>` form.
- `MockArchive` — in-memory `BTreeMap` store. Records every `store` call and exposes `entries()` so tests and cassette-replay can assert on the captured bundles.

Retention types for the object-lock backends:

- `RetentionPolicy` — `mode`, `retain_until` (RFC 3339 string), `legal_hold` flag. Constructors `compliance(...)`, `governance(...)`, and `with_legal_hold()`.
- `RetentionMode` — `Governance` / `Compliance`; `as_cloud_value()` returns the `"GOVERNANCE"` / `"COMPLIANCE"` spelling S3 Object Lock expects.
- `ObjectLockBackend` — `S3ObjectLock` / `AzureWorm`.
- `ObjectLockStore` — the adapter trait (`put_locked_object`, `get_locked_object`, `locked_object_exists`) the two cloud backends drive. `ObjectLockPut` carries the body, its BLAKE3 content hash, and the requested `RetentionPolicy`; `ObjectLockReceipt` carries an optional version id.

`crate_name()` returns `"invoicekit-archive"`.

### Input validation

- `LocalFsArchive` validates the id is exactly 64 lowercase hex chars before any path join. Traversal ids (`../../../etc/passwd`), uppercase, non-hex, wrong-length, and multibyte ids are rejected with `ArchiveError::InvalidId` on every read path — no filesystem touch, no panic on the shard slice.
- The object-lock backends reject ids whose scheme or namespace they do not own (`ArchiveError::InvalidId`).

## Mode / Residuals

- This crate carries no cloud SDK. `S3ObjectLockArchive` and `AzureWormArchive` are generic over an `ObjectLockStore` the operator supplies. The crate ships no production AWS or Azure adapter — only the trait. A real deployment must implement `ObjectLockStore` over the AWS SDK, Azure SDK, a LocalStack endpoint, or a sidecar API. The test module contains an in-memory adapter and a hand-rolled LocalStack S3 adapter used only under `cfg(test)`; neither is part of the public surface.
- The LocalStack smoke test is opt-in: it runs only when `INVOICEKIT_ARCHIVE_LOCALSTACK_S3_URL` is set, and is a no-op otherwise.
- Retention is request-side only. The backends pass `mode`, `retain_until`, and `legal_hold` to the `ObjectLockStore`; this crate does not itself enforce immutability or verify that the backing service honored the policy. The guarantee lives in the configured S3 / Azure service, not in this code.
- Tamper detection covers post-store byte drift, caught by `unpack` re-verification on `retrieve`. It does not provide signatures or authentication; an attacker who can rewrite both the bytes and recompute the BLAKE3 hash is not detected by this crate. Authenticated integrity belongs to the signer / verify crates downstream.
- GCS-retention and IPFS-CID backends named in the module doc-comment are not implemented here; they are noted as follow-up work that will share this trait surface.

## References

No external specifications are cited in the source. The crate names the S3 Object Lock retention header contract (`x-amz-object-lock-mode`, `x-amz-object-lock-retain-until-date`, `x-amz-object-lock-legal-hold`) and the Azure Blob immutability / legal-hold model only via the LocalStack test adapter and field documentation; it depends on `invoicekit-evidence` for the `.ikb` bundle format, `pack`/`unpack`, and `blake3_hex`.

## License

Apache-2.0.
