<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-envelope-encryption

Envelope encryption for tenant-scoped customer data: real AES-256-GCM on the payload, with the data-encryption-key wrapping delegated to a pluggable KMS adapter.

The pattern splits the data-encryption key (DEK) that encrypts the payload from the master key that protects the DEK. `seal` generates a fresh DEK per call, encrypts the payload under it with AES-256-GCM, then asks the adapter to wrap the DEK under the tenant's master key. The resulting `SealedPayload` is self-describing — ciphertext, nonce, wrapped DEK, residency tag, key version — and the cleartext DEK never leaves `seal`'s stack frame.

## Capabilities

Payload cryptography (real):

- `seal(kms, tenant, residency, plaintext) -> SealedPayload` — fresh 32-byte DEK from the platform RNG (`getrandom`), AES-256-GCM encryption (`aes-gcm` crate) under a fresh random 96-bit nonce per call.
- `unseal(kms, tenant, sealed) -> Vec<u8>` — unwraps the DEK via the adapter and AES-GCM-decrypts; AES-GCM authentication failure (tampered or corrupted ciphertext) surfaces as `KmsError::Aead`.
- `PlaintextDek` — 32-byte AES-256 DEK. `generate()` draws from `getrandom`; `Drop` zeroes the buffer via the `zeroize` crate so it does not linger in memory.

Adapter boundary:

- `KmsAdapter` trait — `served_regions()`, `wrap_data_key(tenant, dek) -> WrappedDek`, `unwrap_data_key(tenant, wrapped) -> PlaintextDek`. This is the integration point where a real key-management service plugs in.
- `WrappedDek` — opaque wrapped bytes plus the `KeyVersion` used. `SealedPayload` — tenant id, `Region` residency tag, nonce, ciphertext, wrapped DEK; serde-serializable with byte fields via `serde_bytes`.
- `Region` — `Eu`, `Us`, `Global`. `KeyVersion(u32)`. `TenantId` is a `String` alias.
- `KmsError` — `Rng`, `Aead`, `ResidencyViolation`, `WrongTenant`, `UnknownKeyVersion`, `AdapterNotBuilt`.

Policy guarantees enforced by the seal/unseal functions and pinned by the test suite:

- **Residency at seal time** — `seal` rejects a region the adapter does not serve with `ResidencyViolation` before any wrap. `Global` serves any region.
- **Cross-tenant unseal blocked** — `unseal` returns `WrongTenant` when the caller's tenant id differs from the payload's, before touching the adapter.
- **Key rotation** — `unwrap_data_key` is routed by the sealed payload's `key_version`, so a payload sealed before a rotation still unseals after one; an unknown version returns `UnknownKeyVersion`.

## Mode / Residuals

The payload-layer encryption is real AES-256-GCM. The **DEK wrapping** is where the production path is not yet built:

- `InMemoryKms` is a **deterministic test adapter, not production cryptography.** It derives a per-tenant "master key" from a `domain_secret` via BLAKE3, then wraps the DEK as `XOR(DEK, master)`. The source labels this wrap "not cryptographically meaningful, only deterministic for round-trip and rotation tests." It is suitable only for exercising the trait, residency, tenant-isolation, and rotation logic.
- `AwsKmsScaffold` is a **documentation scaffold with no implementation.** Every `wrap_data_key` / `unwrap_data_key` call returns `KmsError::AdapterNotBuilt { adapter: "AwsKmsScaffold", feature: "aws-kms" }`. It exists so the trait surface and operator docs match what production will see.

A real envelope-encryption deployment needs a `KmsAdapter` implementation backed by an actual key-management service or hardware security module (the doc-comment names AWS KMS, Azure KMS, GCP KMS as the production shape) wrapping the DEK under a real master key — not the XOR stand-in. The AES-GCM payload path is reusable as-is once that adapter lands.

Other limitations from the source:

- `seal` does not bind any associated data (no AES-GCM AAD); the tenant id, residency, and key version on `SealedPayload` are not authenticated by the AEAD tag. Tenant isolation rests on the explicit `WrongTenant` check in `unseal` and on the adapter producing a wrong (failing) DEK for the wrong tenant.
- `getrandom` failure (no kernel entropy source) returns `KmsError::Rng` rather than panicking.

## References

Specs and crates named in the source:

- AES-256-GCM via the `aes-gcm` crate.
- DEK generation and nonce via `getrandom`.
- In-memory test master-key derivation via the `blake3` crate.
- DEK zeroization via the `zeroize` crate.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
