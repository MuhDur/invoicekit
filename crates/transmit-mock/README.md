<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-transmit-mock

Deterministic mock transmission gateway. Replays recorded `.vcr` cassettes instead of contacting any real gateway.

This crate implements the `GatewayAdapter` interface from `invoicekit-reconcile` by matching gateway operations against committed cassettes and normalizing the recorded responses into `GatewayReceipt` or `GatewayError` values. It transmits nothing over a network. It is the fixture and contract-test backbone for the transmit and country-report crates.

## Capabilities

- `MockGatewayAdapter` — a `GatewayAdapter` implementation backed by in-memory cassettes. It serves `submit`, `poll`, `cancel`, and `correct` by turning each operation into a deterministic internal request, matching it against the loaded cassettes, and normalizing the recorded response.
- Cassette format. `Cassette`, `CassetteInteraction`, `RecordedRequest`, `RecordedResponse`, and `ScenarioMetadata` define a JSON `.vcr` document plus its sidecar `scenario.json`. `to_vcr_bytes` / `from_vcr_bytes` are byte-stable. `scenario_metadata_schema()` returns the embedded JSON Schema (Draft 2020-12) for the metadata sidecar.
- Recording. `CassetteRecorder` appends interactions and finishes a cassette.
- Matching. `CassetteMatcher` keys responses by method, request path, and a BLAKE3 fingerprint of the request body (`body_fingerprint`). The fingerprint is recomputed from the body at match time, so a stale stored fingerprint can never route a request to the wrong response. Two interactions with the same key are rejected as a collision.
- Scrubbing. `Scrubber` and `ScrubRule` apply country-scoped literal replacements to request paths, bodies, and headers (`ScrubScope`) before a cassette is committed.
- Personal-data scan. `count_unscrubbed_pii_patterns` and `assert_no_unscrubbed_pii_patterns` flag tokens that look like country-prefixed tax identifiers, IBAN-like account numbers, or email addresses, for use as a commit gate.
- Reusable contract suite. `gateway_contract_scenarios()` and `gateway_contract_cassettes()` build a fixed set of `GatewayAdapter` contract scenarios — `GATEWAY_CONTRACT_SCENARIO_IDS` (idempotent replay, duplicate submission, timeout, malformed receipt, auth failure, certificate rejection, rate limit, delayed async receipt, unknown response field, gateway maintenance page, partner error translation) — that any adapter implementation can be verified against via `verify_submit_result` / `verify_poll_result`.

## Mode

Mock and offline. There is no live transport, no certificate, and no endpoint here. The adapter never opens a connection; it only replays committed `.vcr` cassettes. The recorded HTTP-like status codes, the `content-type` header, and a small set of body markers are the only inputs it inspects to decide between a normalized receipt and a normalized error (including an HTML maintenance-page heuristic for 5xx responses).

There is no real gateway path in this crate. Real Peppol transmission and bring-your-own-credentials delivery live in the separate `transmit-peppol-*` crates; this crate exists so those adapters — and the country-report crates — have deterministic, regulator-free fixtures to test against. Cassettes can be sourced from an official regulator sandbox, a partner sandbox, or be fully synthetic, as declared by `ScenarioSource` in the metadata; the source label is documentary and does not change replay behavior.

## Residuals

From the module documentation and public API:

- Cassettes are JSON `.vcr` documents. Request matching is keyed solely by method, path, and the BLAKE3 body fingerprint; nothing else about a request is considered when selecting a response.
- The personal-data scan is intentionally biased toward false positives for continuous-integration use. It is a heuristic over tax-id-, IBAN-, and email-shaped tokens, not a complete personal-data detector.
- Scrub rules are exact literal find/replace within a country scope (`*` matches all countries); they are not regular expressions or structural redactions.

## References

- JSON Schema 2020-12 — https://json-schema.org/draft/2020-12/schema (the `$schema` of the embedded scenario-metadata schema)
- ISO 3166-1 alpha-2 — country codes validated in scenario metadata and scrub rules

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
