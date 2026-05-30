# invoicekit-timestamping

The RFC 3161 timestamping substrate: a `TimestampClient` trait, the request/token wire-shape types, and a deterministic mock backend. It does **not** ship a real Time Stamping Authority client or an ASN.1 token codec yet.

## What it does

InvoiceKit binds an RFC 3161 timestamp to an evidence bundle's manifest hash. This crate is the seam every timestamping path calls into: one trait, `TimestampClient`, with two methods — `request_timestamp` and `verify_timestamp` — plus the value types that cross it. Tokens are kept opaque (raw bytes) so the substrate stays independent of the eventual ASN.1 dependency choice; `RfcTimestamp::token` is what a downstream evidence bundle would archive.

The trait exists so the engine can swap the Time Stamping Authority per tenant (GlobalSign, Sectigo, DigiCert, Apple's free TSA, or a self-hosted `openssl ts` server all speak the same RFC 3161 wire shape with different URLs, auth, and retry budgets) without touching call sites. The call surface is synchronous: a real backend is HTTP-over-TLS at roughly 100-300 ms, intended to run on a dedicated thread rather than forcing an async runtime onto the engine.

## Mode / Residuals

This crate today is **trait + types + a mock**. There is no real timestamping.

- **`MockTimestampClient` is not an RFC 3161 client.** Its `token` is a `serde_json` envelope of the request fields (TSA name, fixed `genTime`, algorithm slug, imprint bytes, serial, nonce) — **not** a DER-encoded `TimeStampToken`. `genTime` is pinned to a fixed clock (`2026-01-01T00:00:00Z` by default) and the serial increments deterministically per client, so cassette-replay tests produce byte-identical output across runs. The TSA name is `mock-tsa`.
- **`verify_timestamp` is a byte-equality check on the imprint, not a cryptographic verification.** The mock returns `ImprintDrift` only when the supplied `recomputed_imprint` differs from the bytes stored on the token. It does not parse the token, check any signature, or validate a TSA certificate chain. There is no signature anywhere in this crate.
- **`validate_imprint_length` checks length, not the hash.** It confirms the imprint byte count matches the declared algorithm (32 for SHA-256/BLAKE3, 48 for SHA-384, 64 for SHA-512). It does not hash anything and does not know which function produced the bytes.
- **`cert_req` is carried but unused.** The field exists on `TimestampRequest` to mirror RFC 3161 `certReq`; the mock backend does not act on it.
- **`HashAlgorithm::Blake3` is not an RFC 3161 algorithm.** It is not on the RFC 3161 OID list. It is accepted only so the cassette-replay sandbox can timestamp the same BLAKE3 payload hash an evidence bundle records, without re-hashing. A real TSA would reject it.

What the real path needs: an HTTP-over-TLS client to a TSA endpoint and an ASN.1 codec to build the `TimeStampReq` and parse the returned `TimeStampToken` (signature, TSA certificate, `genTime`, `serialNumber`). The crate's own doc-comment tracks these as a follow-up behind a future `reqwest` feature flag.

## Public API

Types:

- `TimestampClient` — the trait. `request_timestamp(&TimestampRequest) -> Result<RfcTimestamp, TimestampingError>` and `verify_timestamp(&RfcTimestamp, recomputed_imprint: &[u8]) -> Result<(), TimestampingError>`. `Send + Sync`.
- `TimestampRequest` — `algorithm`, `message_imprint` (raw hash bytes), optional `nonce`, and `cert_req`. Mirrors the JSON-RPC body a future signer-agent `request_timestamp` method would accept.
- `RfcTimestamp` — `token` (opaque bytes; the mock stores a JSON envelope), plus the parsed fields the engine needs without re-parsing: `algorithm`, `message_imprint`, `generated_at` (RFC 3339 UTC), `tsa_name`, `serial` (decimal string).
- `HashAlgorithm` — `Sha256` (default), `Sha384`, `Sha512`, `Blake3`. Serializes to the kebab slugs `sha-256` / `sha-384` / `sha-512` / `blake3`; `slug()` and `Display` return the same.
- `TimestampingError` — `BadImprintLength`, `Refused`, `Transport`, `Malformed`, `ImprintDrift`. (`Refused`, `Transport`, and `Malformed` exist for a real backend; the mock raises only `BadImprintLength`, `ImprintDrift`, and — on a `serde_json` failure — `Malformed`.)

Backends and helpers:

- `MockTimestampClient` — `new()` (fixed time `2026-01-01T00:00:00Z`, TSA name `mock-tsa`) and `with_fixed_time(time, tsa_name)`.
- `validate_imprint_length(algorithm, observed) -> Result<(), TimestampingError>` — pre-flight the imprint byte count before going to the wire.
- `crate_name() -> &'static str` — returns `"invoicekit-timestamping"`.

## References

- RFC 3161 — Internet X.509 Public Key Infrastructure Time-Stamp Protocol. The wire shape this crate models: `messageImprint.hashAlgorithm` (§2.4), `certReq`, `genTime`, `serialNumber`, and the `granted` status.

## License

Apache-2.0.
