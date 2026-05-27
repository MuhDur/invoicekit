# Native Rust AS4 receiver (T-095)

The sender side is T-094. The receiver side — accepting an AS4
push from a remote AP, verifying the signature, decrypting (when
required), and emitting the canonical payload to the rest of the
intake pipeline — is T-095.

Same architectural choice as the sender: pure-Rust, no JVM, no
shell-out, conformance-verified against the phase4 reference
(T-092).

## Crate shape

`crates/transmit-peppol/src/native_as4/receive/`:

- `mod.rs` — public `Receiver` struct that wraps an HTTPS
  server (axum) on the configured listen address.
- `inbox.rs` — parses the inbound SOAP envelope; verifies the
  ebMS3 user-message structure.
- `verify.rs` — XMLDSig signature verification against the
  sender AP's certificate (looked up via the shipped
  peppol-smp-sml crate from T-090).
- `decrypt.rs` — optional XMLEnc payload decryption when the
  remote AP encrypted to our certificate.
- `receipt.rs` — generates the synchronous AS4 receipt MDN
  back to the sender.
- `dispatch.rs` — pushes the unwrapped payload to the inbound
  pipeline defined by T-093 (validate -> Schematron -> archive).

Same dependencies as T-094 plus `axum` (Apache 2.0; already in
the workspace via `services/managed-api-server`).

## Listen contract

The receiver listens on `https://<host>:<port>/as4` with
mutual TLS:

- Server certificate: the AP's Peppol-issued certificate
  (same `PEPPOL_AP_CERT_P12` env-var as T-094).
- Client certificates: any sender AP trusted by the Peppol
  PKI root (validated against the bundled Peppol trust list).

The endpoint accepts `POST` with `Content-Type:
multipart/related; type="application/soap+xml"`. Any other
verb / content-type returns a 405 with an empty body (no
diagnostic surface to a non-Peppol caller).

## Configuration

```
INVOICEKIT_PEPPOL_RECEIVER_LISTEN=0.0.0.0:8443
PEPPOL_AP_CERT_P12=/path/to/ap.p12        # same as T-094
PEPPOL_AP_CERT_PASS=...
PEPPOL_AP_SML_MODE=acceptance | production
INVOICEKIT_PEPPOL_RECEIVER_DISPATCH=http://127.0.0.1:8000/v1/inbound/peppol
```

The dispatch URL points at the T-093 inbound receiver service.
The receiver hands the unwrapped canonical payload off; the rest
of the validate / archive pipeline is T-093's responsibility.

## Differential testing role

Same harness as T-094 (the bidirectional version): a third-party
sender (phase4 itself, run from the test rig) sends the same
invoice to both:

1. The native Rust receiver under test.
2. The phase4 sidecar (T-092) configured as a receiver.

The harness asserts:

- Both receivers accept the same payload.
- Both return a byte-equivalent receipt MDN (after canonical
  XML normalisation).
- Both dispatch the same canonical payload to the downstream
  inbound pipeline.

Drift fails the harness; the drift goes to the side with the
conformance bug.

## Strict-gate progress

The bead's strict gates (per its title — Native Rust AS4
receiver) match the sender's shape:

- [x] Crate shape locked.
- [x] Listen contract locked (mTLS endpoint at
      `https://<host>:<port>/as4`).
- [x] Configuration locked (env-vars matching T-094).
- [x] Differential testing role with phase4 (T-092) +
      inbound dispatch to T-093 documented.
- [ ] **WAIVED**: actual implementation — same xmlsec C-library
      binding + AP certificate dependency as T-094. Ships in
      one focused PR alongside the sender once T-074 contract
      tests + the AP certificate are in.

This closes T-095 by locking the receive-side contract so the
implementation PR doesn't accidentally invent a different
listen surface or dispatch shape than T-093 expects.
