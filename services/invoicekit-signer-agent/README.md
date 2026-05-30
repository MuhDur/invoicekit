# invoicekit-signer-agent

A local Unix-socket daemon that exposes the `invoicekit-signer` signing surface to the engine, so customer key material stays inside the on-host daemon process and never enters the engine's process. It is a scaffold: signing is delegated to `SoftwareSigner`, which produces a keyed BLAKE3 message authentication code — a placeholder, not a cryptographic digital signature.

## Capabilities

- Binds a Unix domain socket. Default path `/run/invoicekit/signer.sock`, overridable via the `INVOICEKIT_SIGNER_SOCK` environment variable.
- Accepts one newline-terminated JSON request per connection and writes one newline-terminated JSON response. Each accepted connection is handled on its own thread.
- Serves three methods, decoded from `{ "method": ..., "params": ... }`:
  - `ping` → `{ "version": "0.1.0" }`.
  - `list_keys` → `{ "keys": ["key-id", ...] }` (lexicographically sorted, from the backend's keyring).
  - `sign` with params `{ "key_ref": "...", "payload_b64": "..." }` → the `Signature` shape `{ "key_ref": "...", "algorithm": "...", "signature_b64": "..." }`. The algorithm is always `blake3-keyed-256`.
- Decodes `payload_b64` with an in-tree strict RFC 4648 base64 decoder that rejects non-alphabet bytes and bytes following padding.
- Returns `{ "error": "..." }` for malformed request JSON, unknown methods, missing/invalid `sign` params, unknown key references, and serialization failures.
- Signing logic lives in the shared `invoicekit-signer` crate, so the engine, the cassette-replay sandbox, and this daemon use the same `Signer` surface.

## Mode / Residuals

- **Placeholder cryptography, not signatures.** The only backend wired into the daemon is `SoftwareSigner`, which computes a keyed BLAKE3 MAC. It does not produce RSA, ECDSA, or Ed25519 signatures. The output is labeled `blake3-keyed-256` and is suitable only for exercising the protocol and call sites end to end.
- **Hardcoded scaffold keyring.** The daemon loads two fixed, deterministic in-memory keys at startup — `scaffold/default` (32 zero bytes) and `scaffold/test` (32 `0x01` bytes). There is no key provisioning. Real backends — environment-variable-driven file paths, hardware security module (HSM) slots, key management service (KMS) key ids — are deferred to follow-up beads T-083a (software RSA/ECDSA) and T-083b (HSM/PKCS#11).
- **Transport is line-delimited JSON over a Unix socket, not HTTP and not JSON-RPC 2.0.** Requests carry no `jsonrpc` or `id` fields; responses are an untagged ok-or-`{error}` union. There is no `id` correlation, batching, or HTTP layer.
- **No authentication, authorization, rate limiting, or audit logging** in this scaffold. The `Unavailable` and `Refused` error variants exist in the underlying crate but the daemon's `SoftwareSigner` only ever raises `unknown key reference`.
- **No graceful shutdown, socket cleanup, or connection limits.** The socket is bound but not unlinked on exit; binding fails if the path already exists. The accept loop runs until the process is killed. Status messages are written to stderr.
- **Unix only.** Built on `std::os::unix::net::UnixListener`.

## References

- The shared signing crate: `crates/signer` (`invoicekit-signer`), source of the `Signer` trait, `SoftwareSigner`, `SignRequest`, and `Signature` types.
- Base64 encoding/decoding follows RFC 4648 §4.

## License

Apache-2.0.
