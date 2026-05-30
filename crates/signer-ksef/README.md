<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-signer-ksef — Poland KSeF (Krajowy System e-Faktur) certificate-flow adapter

Layers the Polish Ministry of Finance KSeF clearance contract on top of `invoicekit-signer`: a `KsefProvider` trait, a typed session token, and a KSeF stamp envelope. KSeF is a portal-clearance flow — the taxpayer authenticates a session, submits the FA(3) XML, and the portal returns a KSeF reference number (`Numer KSeF`) that closes the invoice.

## Capabilities

- **`KsefProvider` trait.** `init_session(nip, auth_mode)` opens an authenticated session; `submit(request, target_environment)` submits an FA(3) invoice under that session. Both report `provider_name` and `environment`.
- **Typed session model.** `SessionToken` carries the opaque token, the KSeF `referenceNumber`, RFC 3339 `notBefore` / `notAfter` validity bounds, the `AuthMode`, and the target `KsefEnvironment` (`Demo` sandbox vs `Production`).
- **Two declared auth modes.** `AuthMode::QualifiedSignature` (qualified electronic signature, XAdES, over the InitSession payload) and `AuthMode::AuthorisationToken` (a pre-issued KSeF authorisation token). The enum records which was used; this crate does not itself perform the qualified signing.
- **Typed stamp envelope.** `KsefStampEnvelope` bundles the underlying `invoicekit_signer::Signature` receipt, the `Numer KSeF` reference number, the `UPO` (Urzędowe Poświadczenie Odbioru) acknowledgement reference, the `KsefAcceptance` status (`Accepted` / `Pending` / `Rejected`), and the session-token lineage.
- **`MockKsefProvider`.** A deterministic test provider that composes any `invoicekit_signer::Signer`. It validates a non-empty NIP, opens monotonically-numbered sessions, rejects an environment mismatch or an empty session token, signs the FA(3) bytes through the injected signer, and synthesizes a `Numer KSeF` / `UPO` reference. `with_forced_acceptance` drives the `Pending` / `Rejected` outcomes for tests; `sessions()` and `submissions()` expose recorded calls.

## Mode

**Mock / offline only.** The single shipped provider is `MockKsefProvider`: its session tokens, `Numer KSeF`, and `UPO` references are deterministically synthesized, and the cryptographic signature comes from whatever `invoicekit_signer::Signer` the caller injects (the test suite uses `SoftwareSigner`). No HTTPS, no real KSeF clearance, no qualified-signature production happens here.

The real provider is **bring-your-own-credentials** and lands behind a future `ksef-http` feature flag (not present in this crate today). Per the module doc-comment, the live path needs:

- HTTPS to the KSeF REST API (`ksef-test.mf.gov.pl` for demo, `ksef.mf.gov.pl` for production).
- XAdES signing of the `InitSession` payload.
- A NIP-bound qualified certificate, or KSeF tokenized credentials.

## Residuals

- Real KSeF integration is not implemented; only the deterministic mock substrate ships. The live REST/XAdES provider is deferred to a future `ksef-http` feature flag.
- This crate does not produce the XAdES qualified signature itself — it carries the `AuthMode` and stores an `invoicekit_signer::Signature` receipt from an injected signer. The mock signs the FA(3) bytes keyed by the session token, which is not a real KSeF clearance signature.
- FA(3) XML is accepted as opaque bytes (`KsefSubmitRequest.fa_xml`); this crate neither serializes nor schema-validates it. Native FA(3) serialization and NIP checksum validation live in `invoicekit-report-pl-ksef`.

## References

Only names present in the source are listed.

- KSeF — Krajowy System e-Faktur, Polish Ministry of Finance.
- KSeF environment endpoints: `ksef-test.mf.gov.pl` (demo / sandbox) and `ksef.mf.gov.pl` (production), named for the deferred `ksef-http` transport.
- XAdES — the qualified-signature standard named for the `InitSession` auth payload.
- UPO — Urzędowe Poświadczenie Odbioru (official acknowledgement of receipt).

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
