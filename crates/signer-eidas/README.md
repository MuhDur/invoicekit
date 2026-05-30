# invoicekit-signer-eidas

eIDAS qualified-signature adapter. Layers the AdES signature matrix — CAdES / XAdES / PAdES at levels B / T / LT / LTA — on top of `invoicekit-signer`'s `Signer` trait.

This crate defines the provider surface every eIDAS Qualified Trust Service Provider (QTSP) integration implements, plus the typed signature, profile, and verification-report types. It does not ship a real QTSP integration. The only provider that ships here is a deterministic in-memory mock.

## Capabilities

- `EidasSignatureProfile` — the AdES family × level matrix: `Cades` / `Xades` / `Pades`, each carrying an `AdesLevel` of `B` (basic), `T` (timestamp), `Lt` (long-term, embedded revocation data), or `Lta` (long-term + archival timestamp). Helpers `requires_timestamp()` and `requires_revocation_data()` encode which levels need an RFC 3161 token (T/LT/LTA) and embedded OCSP/CRL data (LT/LTA). The doc-comments name XAdES-T and PAdES-LT as the Year-1 anchors for XML invoices and Factur-X hybrid PDFs.
- `EidasQtspProvider` — the provider trait: `certificate()` resolves a `QualifiedCertificateId` to its `QualifiedCertificate`, `sign()` produces an `EidasSignature`, and `verify()` returns a structured `EidasVerifyReport`. It bundles the underlying `Signer` so one provider object does both raw signing and AdES production.
- `EidasSignature` — typed envelope carrying the underlying `Signature` receipt, opaque AdES envelope bytes, the originating profile, the `QualifiedCertificate` used, an optional `RfcTimestamp`, and optional `RevocationRef` entries. The crate does not parse the envelope; verification round-trips through the originating provider.
- `EidasVerifyReport` — per-step verdicts (`Passed` / `Skipped` / `Failed`) for certificate chain, signature value, timestamp, and revocation data, plus an aggregate `ok`.
- `MockEidasProvider` — an in-memory provider built via `MockEidasProvider::builder`, loading registered qualified certificates, a backing `Signer`, and an optional fixed timestamp. It records every sign request (`calls()`) and returns deterministic signatures and verdicts.

## Mode

Mock only. The single provider in this crate, `MockEidasProvider`, produces a deterministic placeholder AdES envelope (a `mock-ades-envelope:family=…:level=…:alg=…:sig=…` byte string), attaches the builder's fixed timestamp for T+ profiles, and synthesizes one OCSP `RevocationRef` for LT/LTA. Its `verify()` re-signs the payload through the backing `Signer` and compares the recorded signature value; the certificate-chain check passes when the certificate id is registered. No real CAdES/XAdES/PAdES bytes are emitted and no real X.509 chain, timestamp token, or revocation response is validated.

The backing `Signer` it is wired to in tests is `invoicekit-signer`'s `SoftwareSigner`, itself a keyed-BLAKE3-MAC placeholder, not a real cryptographic signer.

The live path needs a real QTSP integration: a provider implementing `EidasQtspProvider` that holds (or remotely accesses via the QTSP) a qualified certificate and key, returns real AdES envelope bytes, binds RFC 3161 timestamps, embeds OCSP/CRL revocation data, and verifies against the QTSP trust anchor. Per the module doc-comment and `Cargo.toml`, those real providers (D-Trust / GlobalSign / Sectigo, behind feature flags) are a T-083a follow-up and do not ship here.

## Residuals

From the module doc-comment and type docs:

- The substrate ships so engine call sites can speak the trait today; the real QTSP provider implementations are deferred. The strict T-083a gate ("at least one QTSP integration — D-Trust or GlobalSign") is satisfied by follow-up beads shipping a `dtrust`-feature provider and a `globalsign`-feature provider.
- The crate does not parse AdES envelopes; verification round-trips through the originating provider.
- `MockEidasProviderBuilder::build()` returns `None` when no backing signer was supplied.
- `EidasError::UnsupportedProfile` exists so providers can refuse profiles they do not support (e.g. PAdES on a CAdES-only QTSP); the mock supports all profiles.

## References

Named in the source:

- eIDAS Regulation (EU) 910/2014 — https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=uriserv%3AOJ.L_.2014.257.01.0073.01.ENG
- EU Trust Services (QTSP) — https://digital-strategy.ec.europa.eu/en/policies/trust-services
- AdES families (CAdES / XAdES / PAdES) and levels (B / T / LT / LTA); RFC 3161 timestamp tokens; OCSP / CRL revocation data; eIDAS Annex IV qualified certificates; RFC 4514 distinguished-name strings; RFC 3339 timestamps (referenced by name, no URL in source).

## License

Apache-2.0. Part of the InvoiceKit workspace.
