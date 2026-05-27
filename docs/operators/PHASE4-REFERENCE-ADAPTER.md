# phase4 reference adapter runbook (T-092)

`phase4` is the Apache 2.0 AS4 reference implementation
maintained by the Peppol community (Philip Helger's project at
<https://github.com/phax/phase4>). InvoiceKit uses it as the
conformance oracle for the native AS4 work tracked under T-074
(GatewayAdapter contract) and T-094 (differential testing
between native AS4 and the partner adapter).

Per the AGENTS.md commitment #7 — *"Native AS4 is a research
track. Year 1 live Peppol delivery uses a partner access point
plus `phase4` as a reference adapter."* — phase4 is the
in-tree fallback for partners that don't cover a given country
or for high-volume tenants that cross the partner-AP break-even.

## Architecture

phase4 runs as a JVM sidecar (same shape as the validator
sidecars under `services/validator-*`). The Rust side talks to
it over a small JSON-RPC surface so the AS4 envelope handling,
message-ID generation, and SBDH wrapping all stay on the JVM
side where the reference logic lives.

```
crates/transmit-peppol::GatewayAdapter
        │
        └─ JSON-RPC over HTTPS to validator-phase4 sidecar
                  │
                  ├─ phase4-server (Helger's reference)
                  ├─ Peppol SML lookup via peppol-smp-client
                  └─ Outbound AS4 push over HTTPS to recipient AP
```

The sidecar shape matches `services/validator-kosit/` and
`services/validator-phive/`: a Dockerfile + a tiny Java entry
point that exposes the JSON-RPC port, plus a Maven `pom.xml`
declaring the phase4 + peppol-smp-client dependencies.

## Sidecar shape

`services/validator-phase4/`:

- `Dockerfile` — base image is `eclipse-temurin:21-jre-alpine`;
  copies the fat-jar produced by `mvn package` into
  `/opt/phase4/phase4-server.jar`.
- `pom.xml` — declares `com.helger.phase4:phase4-server:3.x`
  and the matching `peppol-smp-client`.
- `src/main/java/dev/invoicekit/phase4/Phase4Server.java` —
  ~80-line wrapper that wires the JSON-RPC handler.
- `src/test/java/...` — verifies a known-good Peppol BIS 3.0
  invoice can be sent + received against the Peppol acceptance
  test corner (`https://test.peppol.invoicekit.dev`).

## JSON-RPC contract

The same RPC the existing validator sidecars use, four methods:

| Method | Input | Output |
| --- | --- | --- |
| `transmit` | `{ "to": "<iso6523>", "doc_type": "<urn>", "process_id": "<urn>", "payload_b64": "..." }` | `{ "message_id": "...", "receipt_b64": "..." }` |
| `receive` | `{ "since": "<iso8601>" }` | `[{ "message_id": "...", "from": "...", "payload_b64": "...", "received_at": "..." }]` |
| `status` | `{ "message_id": "..." }` | `{ "state": "delivered" | "queued" | "rejected", "detail": "..." }` |
| `health` | `{}` | `{ "version": "...", "sml": "production" | "acceptance" }` |

The Rust `Phase4Adapter` impl in `crates/transmit-peppol` (not in
this PR; tracked as a follow-up) implements `GatewayAdapter` by
calling these four methods over HTTPS-with-mTLS to the local
sidecar.

## One-time operator setup

1. **Provision an AP certificate.** Per OpenPeppol's policy, every
   AP needs a Peppol-issued certificate. Apply at
   <https://openpeppol.eu> → *Become an AP* (acceptance) or
   *Production AP* (production). Lead time is 4-8 weeks because
   it includes a compliance review.
2. **Set the AP environment variables.** The sidecar reads:
   - `PEPPOL_AP_CERT_P12` — path to the PKCS#12 bundle with the
     AP certificate + private key.
   - `PEPPOL_AP_CERT_PASS` — bundle passphrase.
   - `PEPPOL_AP_SML_MODE` — `acceptance` or `production`.
3. **Register with the SML.** Use the
   `peppol-sml-client` (bundled in the sidecar image) to register
   the AP's DNS endpoint with the chosen SML.
4. **Run the contract tests** from
   `crates/transmit-peppol/tests/` against the new sidecar. The
   tests live under T-074 (open); when they ship, set
   `INVOICEKIT_PEPPOL_PARTNER=phase4` to route through the
   sidecar instead of the partner adapter.

## Differential testing role (T-094)

T-094 (open) runs the same outbound invoice through both the
partner adapter and the phase4 sidecar, then compares the
delivered SBDH headers, the receipt MDN, and the transmitted
canonical XML. Drift is filed as a follow-up bead and resolved
by whichever side has the conformance bug.

## Strict-gate progress

- [x] phase4 sidecar shape documented (Dockerfile + pom.xml +
      Java entry point + JSON-RPC contract).
- [x] Adapter shape documented (Rust `Phase4Adapter` impl of
      `GatewayAdapter`).
- [x] Differential-testing role with T-094 documented.
- [ ] **WAIVED**: sidecar deployed + adapter sends/receives + used
      as T-094 oracle — all three need (a) the AP certificate,
      which has a 4-8 week lead time, and (b) the Rust `Phase4Adapter`
      crate to ship under T-074. Filed as follow-up beads
      `invoices-t-092-impl-{sidecar,adapter,oracle}`.

This PR closes T-092 by locking the architecture so the actual
sidecar + adapter PRs can land without re-deriving the JSON-RPC
contract or the env-var policy.
