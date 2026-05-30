<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-signer

The signing substrate: the `Signer` trait the engine calls when it needs a payload signed, plus the backends that fulfil it.

This crate is the **abstraction boundary**, not a cryptographic signature implementation. It defines a stable `sign` surface (and a stable JSON request/response shape) so the engine and language bindings can depend on one interface while the actual signature backend is swapped underneath.

## Capabilities

- `Signer` trait — synchronous `sign(&SignRequest) -> Result<Signature, SigningError>` plus `list_keys() -> Vec<KeyRef>`.
- `KeyRef` — opaque, operator-facing handle into the signer's keyring. The backend resolves it to underlying material (file path, hardware-security-module slot, key-management-service key id); this crate never holds the secret directly.
- `Signature` — detached signature: the producing `KeyRef`, an algorithm identifier string, and base64-encoded signature bytes (RFC 4648 §4).
- `SoftwareSigner` — in-process backend keyed by 32-byte material per `KeyRef`. It computes a **keyed BLAKE3 message-authentication code**, not a digital signature. Algorithm id `blake3-keyed-256`.
- `MockSigner` — records every call and returns a deterministic value derived from the BLAKE3 hash of the payload (algorithm id `mock-blake3-256`); for tests and the cassette-replay sandbox.
- `UnixSocketSigner` (Unix only) — engine-side client that forwards `sign` / `list_keys` to the on-host `services/invoicekit-signer-agent` daemon over a local Unix socket, so customer key material stays in the daemon process. Adds `try_list_keys()` for pre-flight checks that must distinguish an empty keyring from an unreachable daemon.
- `SigningError` — `UnknownKey`, `Unavailable`, `Refused`.

## Mode

**Mock / placeholder. No real cryptographic signer ships in this crate.**

Neither in-process backend produces a verifiable digital signature:

- `SoftwareSigner` is a keyed BLAKE3 MAC. It exists so the trait, the daemon protocol, and the engine call sites are exercised end to end. It is labelled in the source as a placeholder substrate for non-regulated cases.
- `MockSigner` is a deterministic test double over the payload hash.

The doc-comment names the follow-up work for real cryptography: software RSA / ECDSA under bead T-083a, and a hardware-security-module / PKCS#11 backend under T-083b. Until those land, a real signing path needs that backend behind the same `Signer` trait — and, for the keys-never-leave-the-host model, the `invoicekit-signer-agent` daemon (reached via `UnixSocketSigner`) holding the actual key material.

This crate does not implement XAdES, CAdES, PAdES, JWS, eIDAS, or any national signature scheme. Those are the concern of the dedicated signer crates in the workspace (for example `signer-eidas`, `signer-sdi`, `signer-zatca`), not this substrate.

## Residuals

From the module doc-comment and source:

- `SoftwareSigner` is explicitly a placeholder MAC, not a signature; real RSA/ECDSA (T-083a) and HSM/PKCS#11 (T-083b) are deferred.
- `UnixSocketSigner` and the signer-agent daemon are Unix-only (`#[cfg(unix)]`).
- `Signer::list_keys` cannot surface errors; `UnixSocketSigner::list_keys` maps any failure to an empty list. Use `try_list_keys()` when the distinction matters.
- The daemon protocol is a minimal line-delimited JSON RPC (`{ "method", "params" }` → one JSON response line); error mapping recognizes only the `unknown key reference:` prefix and folds everything else into `Refused`.

## References

- RFC 4648 — base64 encoding (signature byte encoding). https://www.rfc-editor.org/rfc/rfc4648

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
