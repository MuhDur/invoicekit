<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-transmit-peppol-phase4

Transmits invoices over Peppol AS4 by delegating to the `validator-phase4` JSON-RPC sidecar. This crate is the Rust adapter; it does not implement AS4 itself.

`Phase4Adapter` implements `invoicekit_reconcile::GatewayAdapter` by translating each gateway operation into a JSON-RPC call on a local phase4 sidecar. The sidecar is the thing that speaks AS4 to the Peppol network; this crate builds the request bodies, classifies the responses, and maps them back into the reconcile state machine. It exists primarily as InvoiceKit's AS4 reference and differential-test oracle.

## Capabilities

- Implements `GatewayAdapter` against a phase4 sidecar over the four-method JSON-RPC contract `transmit` / `receive` / `status` / `health`.
- `submit` builds a `transmit` call (recipient, doc-type profile, process-id, base64 payload) and parses the returned `message_id` into a `Pending` receipt.
- `poll` builds a `status` call by `message_id` and maps the sidecar's `state` field: `delivered -> Accepted`, `queued -> Pending`, `rejected -> Rejected`. Unknown states are rejected as `UnexpectedResponse`.
- Maps transport and HTTP failures into typed `GatewayError` kinds: `401/403 -> AuthFailure`, `429 -> RateLimited`, `5xx -> GatewayMaintenance`, other non-2xx `-> PartnerError`, malformed body `-> UnexpectedResponse`, transport `-> NetworkFailure`.
- `SmlMode` (`acceptance` / `production`) records which Service Metadata Locator environment the sidecar targets, parsed from `PEPPOL_AP_SML_MODE`.
- `Phase4Config::from_env` reads `INVOICEKIT_PHASE4_URL` (default `http://127.0.0.1:8090`) and `PEPPOL_AP_SML_MODE`.
- `byok::phase4_config_from_byok` builds a `Phase4Config` from a customer-supplied `PeppolCredentials` bundle, mapping its `endpoint_url` to the sidecar URL and rejecting any transport other than `Phase4`.

## Mode

Bring-your-own-credentials, and currently mock-only at the transport boundary in this crate.

- The JSON-RPC transport is abstracted behind the `RpcClient` trait. The only implementation that ships here is `MockRpcClient`, which records calls and pops queued responses for tests. No live HTTP client is compiled in.
- A `reqwest`-backed `RpcClient` is the intended runtime path and is described in the source as living behind a `reqwest` feature flag, but that feature and implementation are not present in this crate yet.
- The live path needs: a running phase4 sidecar process, and a customer-held Peppol Access Point certificate plus endpoint (the source notes a 4-8 week OpenPeppol onboarding for the certificate). InvoiceKit does not hold the certificate; the customer supplies it via the BYOK credentials bundle, which sets the sidecar `endpoint_url`.
- No AS4, no TLS, and no real Peppol delivery happen inside this crate. All cryptographic and on-the-wire work lives in the external sidecar.

## Residuals

Documented in the module and function doc-comments:

- No live transport ships here. The `reqwest`-backed `RpcClient` is a follow-up gated on the Access Point certificate clearing OpenPeppol onboarding.
- `submit` routing is scaffold-level: it passes `route`/`profile` through directly and relies on the sidecar's own SMP cache to resolve the participant. Proper participant resolution waits on T-093 inbound and the `peppol-smp-sml` resolver crate.
- Receipt timestamps are emitted as a fixed `1970-01-01T00:00:00Z` placeholder rather than a real wall-clock time.
- `cancel` is unsupported: the phase4 sidecar exposes no cancel surface and AS4 is fire-and-forget; issue a corrective document via `submit` instead.
- `correct` is unsupported as a distinct operation: corrections are re-submitted as a fresh document through `submit` with a new idempotency key.
- The `receive` method is part of the contract but has no adapter surface in this crate.

## References

- `docs/operators/PHASE4-REFERENCE-ADAPTER.md` — the JSON-RPC sidecar contract this adapter targets.
- RFC 4648 §4 — base64 standard alphabet, used by the inlined payload encoder.

## License

Apache-2.0
