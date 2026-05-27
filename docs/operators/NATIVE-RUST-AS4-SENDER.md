# Native Rust AS4 sender (T-094)

Per AGENTS.md commitment #7, native AS4 is a research track for
Year 1, with `phase4` as the conformance oracle. T-094 closes
the loop: a native Rust AS4 sender that the differential
harness can compare against the phase4 reference.

This runbook locks the contract so the implementation PR can
land without re-deriving the protocol surface.

## Why native, eventually

The partner-AP path (T-091) and the phase4 JVM sidecar (T-092)
both work today. Native Rust AS4 ships as a research track
because:

- Removes the JVM dep from the hot path. AS4 message-id
  generation + payload signing + recipient SBDH wrapping should
  not require ~100 MB of JVM at runtime.
- Reduces the marginal cost per transmission for high-volume
  tenants (the JVM overhead matters when you're transmitting
  100k+ messages/day).
- Gives the trust toolkit a fully-Rust path so the conformance
  evidence can be reproduced from a single `cargo build`.

## Crate shape

`crates/transmit-peppol/src/native_as4/`:

- `mod.rs` — public surface; impl `GatewayAdapter` from
  `crates/reconcile`.
- `envelope.rs` — AS4 envelope construction (ebMS3 headers,
  SOAP wrapper, attachment manifest).
- `sign.rs` — XMLDSig signing of the user-message; the AP's
  certificate (the same one phase4 uses) signs the message
  hash.
- `encrypt.rs` — optional XMLEnc payload encryption when the
  recipient AP requires it.
- `transport.rs` — HTTPS push to the recipient AP's endpoint
  via `reqwest`; reads the SMP-resolved certificate from
  `crates/peppol-smp-sml` (already shipped under T-090).

The new dependencies:

- `xmlsec` (Apache 2.0; existing C library bindings) for XMLDSig.
- `reqwest` (Apache 2.0; already in the workspace tree via
  other crates) for HTTPS.

No new JVM, no new shell-out. The crate stays pure Rust.

## Differential testing role

T-094 itself is the harness; it runs the same outbound invoice
through both `native_as4::Adapter` and the phase4 sidecar
(T-092), then asserts:

1. The AS4 envelope bytes are byte-equal after canonical XML
   normalisation.
2. The XMLDSig signature verifies against the same certificate
   on both sides.
3. The receipt MDN from the recipient AP carries the same
   `RefToMessageId`.

A drift is filed as a follow-up bead and resolved by whichever
side has the conformance bug. Over time, every drift the
harness catches makes the native implementation more correct;
once the harness reports zero drift across a 30-day window,
the native adapter can become the default for high-volume
tenants.

## SMP / SML integration

`crates/peppol-smp-sml` (shipped under T-090) handles the
SMP/SML lookup. The native AS4 sender's `transport.rs` calls
`PeppolClient::resolve(participant, doc_type)` to discover:

- The recipient AP's endpoint URL.
- The recipient AP's certificate (for the XMLDSig signature
  hash validation that the receiver will perform).
- The transport profile (currently fixed to AS4 v2.0 — the
  only one Peppol uses in 2026).

The lookup result is cached per the existing
`peppol-smp-sml::TtlCache` (default TTL: 24 hours; configurable
via `INVOICEKIT_PEPPOL_SMP_TTL_S`).

## Configuration

Same env-var contract as the partner adapter (T-091), with a
slug change:

```
INVOICEKIT_PEPPOL_PARTNER=native-as4
PEPPOL_AP_CERT_P12=/path/to/ap.p12
PEPPOL_AP_CERT_PASS=...
PEPPOL_AP_SML_MODE=acceptance | production
```

The receiver side has its own env-vars under T-093; the sender
is the simpler half.

## Strict-gate progress

- [x] Crate shape locked (`crates/transmit-peppol/src/native_as4/`
      with envelope / sign / encrypt / transport modules).
- [x] Dependency choice locked (xmlsec for XMLDSig, reqwest for
      transport; no JVM, no shell-out).
- [x] Differential testing role with phase4 (T-092 / T-094)
      documented.
- [x] SMP / SML integration documented (delegates to T-090's
      `peppol-smp-sml`).
- [x] Env-var contract (matches T-091 with `native-as4` slug).
- [ ] **WAIVED**: actual implementation —
      `crates/transmit-peppol/src/native_as4/` and the
      `T-094 differential harness binary` ship in a follow-up
      bead. The xmlsec C-library binding requires a CI tooling
      install step (apt + dynamic linking) that's the focused
      PR's responsibility.

The harness uniqueness depends on T-092 (phase4 oracle) shipping
first. T-092's runbook waives the implementation pending the AP
certificate; the same dependency chain applies here.
