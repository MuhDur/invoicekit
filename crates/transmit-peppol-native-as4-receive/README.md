# invoicekit-transmit-peppol-native-as4-receive

Native Rust AS4 *receiver* scaffold for Peppol: an mTLS listener that verifies the inbound XMLDSig signature, unwraps the canonical payload, dispatches it downstream, and returns the synchronous AS4 receipt MDN.

This is the receive-side mirror of the native AS4 sender (`invoicekit-transmit-peppol-native-as4`). It defines the receiver pipeline and its three pluggable abstractions; the real network, cryptographic, and pipeline implementations are feature-gated follow-ups and do **not** ship in this crate yet.

## Capabilities

- `NativeAs4Receiver::process_one` — the receive pipeline: accept an inbound envelope, verify its signature, unwrap the SOAP body, dispatch the payload, and return a `ReceiverOutcome` (the `RefToMessageId` plus the receipt MDN bytes).
- Three trait seams the pipeline is built from:
  - `Listener` — blocks until the next inbound envelope arrives. Documented production target is an `axum` HTTP server with mTLS.
  - `Verifier` — verifies the inbound envelope's XMLDSig signature against the sender access point's certificate, looked up by participant id. Documented production target is an `xmlsec`-backed implementation.
  - `Dispatcher` — hands the unwrapped XML to the downstream inbound pipeline and returns the receipt MDN bytes to write back synchronously.
- `InboundEnvelope` — typed inbound message: raw SOAP bytes (kept un-normalised so the signature still covers the same bytes), sender and recipient participant ids, and the per-message id.
- `unwrap_soap_body` — extracts the bytes between `<soap:Body>` and `</soap:Body>`. This is the only wire parsing the scaffold performs; full SBDH header parsing is a documented follow-up.
- BYOK bridge (`byok` module): `receiver_config_from_byok` turns a customer-supplied `PeppolCredentials` bundle into a `ReceiverConfig` (bind URL, AP certificate and key PEM paths, SML mode, wire-format participant id). It rejects any credentials whose transport is not `NativeAs4`.

## Mode

**Mock / scaffold + bring-your-own-credentials.** No real cryptography, no real network, no real downstream dispatch ships here.

- The only signature work is **verification** of an inbound XMLDSig signature — there is no signer in this crate. The shipped `MockVerifier` accepts (or, when constructed with `rejecting()`, rejects) every envelope without inspecting any signature. A real verifier needs the `xmlsec`-backed implementation noted in the source as a follow-up behind an `xmlsec` cargo feature, plus the Peppol PKI trust list to look up and trust the sender AP certificate.
- Transport is **native AS4** (a Rust mTLS AS4 endpoint), not a partner access point and not `phase4`. The shipped `MockListener` replays a queued list of envelopes and never opens a socket. The live path needs the `axum`-backed mTLS listener (noted as a follow-up behind an `axum` cargo feature).
- Downstream delivery uses `MockDispatcher`, which returns a fixed receipt MDN and records payloads. The documented production dispatcher posts the unwrapped payload to the inbound pipeline at `INVOICEKIT_PEPPOL_RECEIVER_DISPATCH`.
- This is bring-your-own-credentials: the customer hosts the receive endpoint and holds the AP certificate, private key, and bind URL. The `byok` bridge maps those credentials into a `ReceiverConfig`. A `peppol-test-bed` cargo feature gates Peppol Test Bed integration tests; it is off by default so a fork without a Test Bed certificate still builds.

Operator setup for the real path is documented in the runbook at `docs/operators/NATIVE-RUST-AS4-RECEIVER.md`.

## Residuals

From the module documentation:

- This crate is a **scaffold**. The `Verifier` (XMLDSig via `xmlsec`), `Listener` (mTLS via `axum`), and production `Dispatcher` are follow-ups; only mock implementations of all three ship today.
- `unwrap_soap_body` only locates the payload between the `<soap:Body>` tags. Full Standard Business Document Header (SBDH) parsing — including extracting sender and recipient participant ids and the message id from the header — is a follow-up bead; the scaffold's `InboundEnvelope` carries those fields but the wire-to-struct extraction is not implemented here.
- `publish = false`: this crate is workspace-internal and is not released to a registry.

## References

Standards named in the source (no external URLs are present in the source):

- AS4 (the Peppol message exchange profile) and its synchronous receipt MDN.
- XMLDSig — XML digital signature, the inbound signature this receiver verifies.

Repository document referenced by the source:

- `docs/operators/NATIVE-RUST-AS4-RECEIVER.md` — operator runbook for the native receiver.

## License

Apache-2.0
