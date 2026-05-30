# invoicekit-transmit-peppol-native-as4

Native Rust AS4 sender for Peppol: a `GatewayAdapter` that assembles an ebMS3 / AS4 SOAP envelope, signs it with XMLDSig, and pushes it over HTTPS to the recipient access point.

This is a research-track **scaffold**. The cryptographic signer and the network transport are injectable traits; only deterministic mocks ship today. The Year-1 production path is the partner access-point adapter (`crates/transmit-peppol-partner/`, T-091), not this crate.

## Capabilities

- Implements `invoicekit_reconcile::GatewayAdapter` (`submit`, `poll`, `cancel`, `correct`) over the native AS4 send path.
- Builds a typed `As4Envelope` — callers never hand-build SOAP. The envelope wraps the canonical Universal Business Language XML emitted by `invoicekit-format-ubl` in an ebMS3 `UserMessage` SOAP envelope.
- Escapes the operator-controlled `MessageId` (derived from `tenant_id` + `gateway_attempt_id`) so a hostile identifier cannot inject markup into the envelope.
- Resolves the recipient access point through an injectable `SmpResolver` (SMP / SML lookup) abstraction.
- Maps recipient AP HTTP status codes (401/403, 404, 408, 409, 422, 429, 5xx) onto `GatewayErrorKind`, mirroring the partner adapter so failure handling is uniform across both send paths.
- `byok` module: `native_as4_config_from_byok` translates a customer-supplied `PeppolCredentials` bundle into a `NativeAs4Config`, reading the access-point certificate PEM and mapping the SML mode (BYOK `Test` / `Acceptance` both map to native `Acceptance`; `Production` maps through).

## Mode

**Mock / offline for both signing and transport. Bring-your-own-credentials for configuration.**

Signing — standard is **XMLDSig** (XML Digital Signature) on the AS4 / ebMS3 SOAP envelope. The only signer that ships is `MockSigner`, which records the call and appends a `<!--SIGNED-->` marker; it does no cryptography. The doc-comment states the real `xmlsec`-backed signer lands behind an `xmlsec` cargo feature (not yet present) and that it will read the sending AP's PEM-encoded certificate and its private key from `NativeAs4Config`. A live signer needs that Peppol-issued AP certificate plus access to its private key.

Transport — the wire path is an HTTPS push to the recipient AP. The only transport that ships is `MockTransport`, which records pushed envelopes and replays queued responses. The doc-comment states the real `reqwest`-backed transport lands behind a `reqwest` cargo feature (not yet present).

SMP / SML lookup — only `StaticSmpResolver` ships (returns a fixed target). The `invoicekit-peppol-smp-sml`-backed resolver is a documented follow-up.

Credentials are bring-your-own: the customer holds the AP certificate, private key, and endpoint; the `byok` bridge reads them in. This crate drives transmission but does not provision or store credentials. The `peppol-test-bed` cargo feature gates BYOK + Peppol Test Bed integration tests; it is off by default so a fork without a Test Bed certificate builds cleanly.

`publish = false` — internal workspace crate.

## Residuals

From the module doc-comments and inline scaffold notes:

- Native AS4 is a research track per AGENTS.md commitment #5/#7; the production Peppol send path is the partner AP adapter (T-091).
- `build_as4_envelope_bytes` emits a minimal SOAP body wrapper, not the full AS4 / ebMS3 SOAP headers. It is sufficient only to exercise the signer + transport end to end; the real header set is deferred to the follow-up.
- The adapter does not yet consult `NativeAs4Config`; the `ap_cert_pem` and `sml_mode` fields are read by the future `xmlsec` signer. The field is kept now to hold the constructor signature stable.
- Native AS4 is push-only on the sender side. `poll` is a local operation; the scaffold returns a `Pending` receipt and a follow-up bead wires the actual outbox lookup. Every locally minted receipt is `Pending` at the epoch sentinel timestamp because the sender never learns the final delivery status synchronously.
- `cancel` is unsupported — Peppol invoices are immutable post-submit; use `correct` to issue a credit note.
- The recipient participant is read from the customer's first tax id (the shape Storecove and ecosio accept); production routing from `country_iso` + the document is not yet wired.
- This crate is the in-tree reference the T-094 differential harness compares against the phase4 JVM sidecar (T-092); that comparison becomes meaningful once the `xmlsec` signer lands.
- See `docs/operators/NATIVE-RUST-AS4-SENDER.md` for the T-094 runbook.

## References

- AS4 / ebMS3 SOAP namespace: `http://docs.oasis-open.org/ebxml-msg/ebms/v3.0/ns/core/200704/`
- SOAP 1.2 envelope namespace: `http://www.w3.org/2003/05/soap-envelope`
- Peppol participant identifier scheme: `iso6523-actorid-upis`
- Peppol document type: `peppol-bis-billing-3`

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
