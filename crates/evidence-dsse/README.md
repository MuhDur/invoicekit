# invoicekit-evidence-dsse

A typed DSSE (Dead Simple Signing Envelope) wrapper for InvoiceKit evidence-bundle manifests, with the PAE (Pre-Authentication Encoding) helper, a `ManifestSigner` trait, and a deterministic test-only mock signer.

This crate defines the envelope shape and the bind/verify plumbing. It does **not** ship a real cryptographic signer. The only signer it provides is a non-cryptographic mock; production signing is expected from a sibling signer crate / HSM / KMS that implements the `ManifestSigner` trait.

## What it does

An evidence bundle (`invoicekit-evidence`) already carries a BLAKE3 hash ledger in `manifest.json`, but the unsigned core does not bind a signature to that manifest. This crate adds the envelope binding.

It computes the DSSE Pre-Authentication Encoding over `(payload_type, payload)`, hands those bytes to a `ManifestSigner` to sign, and packages the base64 payload, payload type, and resulting signature into a `DsseEnvelope`. `attach_manifest_dsse` writes the serialized envelope back onto a bundle as the reserved `signatures/manifest.dsse` sidecar artefact, leaving `manifest.json` itself unchanged (signing the manifest while listing the envelope inside it would make the signed payload self-referential). `verify_envelope` re-checks the binding.

The signature covers the PAE, not the raw payload bytes, so the payload type is bound into the signature and cannot be swapped after the fact.

## Capabilities

- `pae(payload_type, payload)` — DSSE v1.0 Pre-Authentication Encoding: `"DSSEv1" SP LEN(type) SP type SP LEN(payload) SP payload`, byte lengths in decimal, single-space separators, payload appended raw.
- `wrap(signer, payload_type, payload)` — sign the PAE with a `ManifestSigner` and produce a single-signature `DsseEnvelope`.
- `wrap_manifest(bundle, signer)` — wrap a bundle's canonical `manifest.json` bytes (the compact JSON serialization of `invoicekit_evidence::Manifest`) under `MANIFEST_PAYLOAD_TYPE`.
- `attach_manifest_dsse(bundle, signer)` — clone the bundle and insert/replace the `signatures/manifest.dsse` sidecar artefact carrying the JSON-encoded envelope. The manifest is left untouched.
- `verify_envelope(envelope, expected_payload_type, expected_payload, signer)` — verification in order: reject zero signatures (`NoSignatures`); reject payload-type mismatch (`PayloadTypeDrift`); constant-time byte-compare the envelope's decoded payload against the caller's freshly recomputed `expected_payload` (`PayloadDrift`); find a signature whose `keyid` matches the signer (`UnknownKey`); ask the signer to re-validate the signature over the recomputed PAE (`BadSignature`). The caller must supply the freshly recomputed payload; the function does not derive it from the envelope.
- `DsseEnvelope::decoded_payload()` — base64-decode the payload field back to raw bytes.
- `ManifestSigner` trait — `keyid()`, `sign_pae()`, `verify_pae()`. The crate-local definition keeps this crate independent of the wider signer stack. Any real signer (HSM/KMS slot, software key) is expected to implement it; the trait itself does not define an algorithm.
- Serde `Serialize`/`Deserialize` on `DsseEnvelope` and `DsseSignature`, with the `payloadType` field rename per the DSSE wire format.

Base64 is standard (not URL-safe) per the DSSE spec. Payload and signature byte comparisons use constant-time equality (`subtle`).

## Mode / Residuals

**The bundled signer is a mock, not cryptography.** `MockSigner` "signs" by emitting `b"mock-dsse:"` followed by a 32-byte deterministic digest of the PAE; verification recomputes the same value and compares. The digest (`mock_digest`) is an FNV-1a-derived stand-in — explicitly not BLAKE3 and not cryptographic. It exists only so the rest of the InvoiceKit pipeline can exercise the DSSE envelope shape end to end (including cassette replay) before real signer wiring lands. It provides no authenticity, integrity, or non-repudiation guarantee against an adversary.

**The real path needs a real signer.** To get genuine signatures, supply a `ManifestSigner` backed by an actual signing key — a software keypair with a real signature algorithm, or an HSM/KMS slot. This crate does not implement, select, or negotiate a signature algorithm; that is the signer's responsibility, and `keyid` is an opaque identifier the signer chooses (JWK thumbprint, X.509 SKI, HSM slot label).

**Verification is only as strong as the signer.** `verify_envelope` delegates the cryptographic check to `ManifestSigner::verify_pae`. Verified against `MockSigner`, a pass means only that the deterministic mock digest matched — not that any party with a key signed the manifest.

**Single-signature helpers.** `wrap` / `wrap_manifest` produce envelopes with exactly one signature. The `DsseEnvelope` type holds a `Vec<DsseSignature>` and the spec allows N-of-M, but threshold construction is not provided here; `verify_envelope` checks one signature matching the supplied signer's `keyid`.

## References

- DSSE specification v1.0 — <https://dsse.dev/spec/v1.0> (envelope shape, PAE construction, the worked `HelloWorld` example asserted in the crate tests).
- `MANIFEST_PAYLOAD_TYPE` — `application/vnd.invoicekit.manifest+json`, the payload type pinned for `manifest.json`.
- `MANIFEST_SIGNATURE_ARTEFACT_ID` — `signatures/manifest.dsse`, the reserved sidecar artefact id (re-exported from `invoicekit-evidence`).

## License

Apache-2.0. Part of the InvoiceKit workspace.
