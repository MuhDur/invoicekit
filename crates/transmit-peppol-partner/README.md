<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-transmit-peppol-partner

Peppol transmission through a third-party (partner) access point. Implements the `invoicekit-reconcile` `GatewayAdapter` against the REST shapes of three named vendors: Storecove, ecosio, and B2BRouter.

## What it does

This crate does not run AS4 or hold a Peppol certificate. It hands the canonical Universal Business Language XML to a partner access point over the partner's own HTTP API, and that partner does the AS4 delivery into the Peppol network. The adapter translates the reconcile state machine's `SubmitRequest` / `PollRequest` / `CancelRequest` / `CorrectRequest` into the chosen vendor's URL and JSON body, posts it through an injected `HttpClient`, and maps the response back into a `GatewayReceipt` or a typed `GatewayError`.

`submit` serializes the in-memory document to UBL via `invoicekit-format-ubl`, base64-wraps it, and builds the vendor-specific body. `correct` is a fresh submit. `cancel` is unsupported on purpose (`GatewayErrorKind::UnsupportedOperation`): a Peppol invoice is immutable once submitted, so the documented path is to issue a credit note via `correct`. `poll` percent-encodes the partner-supplied submission id before interpolating it into the status URL, so a hostile id cannot escape its path segment.

## Capabilities

- `PartnerVendor` — closed enum of the three supported vendors (`Storecove`, `Ecosio`, `B2brouter`), with slug parsing (`from_slug`) and a default production base URL per vendor (`default_api_base`).
- `PartnerConfig` — env-var-driven configuration (`from_env`) or direct construction. Carries vendor, API base, vendor-assigned legal-entity id, and a `sandbox` flag.
- `SecretResolver` trait with two shipped implementations: `EnvSecretResolver` (reads `INVOICEKIT_PEPPOL_API_KEY` / `_API_SECRET`) and `StaticSecretResolver`. Credentials are resolved through this trait so the API key never lives in `PartnerConfig`.
- `HttpClient` trait — the injectable HTTP transport. `MockHttpClient` is the shipped implementation: it records every request (method, URL, bearer, body) and replays queued responses.
- `PartnerPeppolAdapter` — the `GatewayAdapter`. Builds per-vendor submit and poll URLs and per-vendor JSON submit bodies, mapping HTTP status codes to `GatewayErrorKind` (401/403 to `AuthFailure`, 409 to `DuplicateSubmission`, 422 to `Rejected`, 429 to `RateLimited`, 5xx to `GatewayMaintenance`, and so on).
- `byok::partner_config_from_byok` — builds a `PartnerConfig` from a customer-supplied `invoicekit-transmit-peppol-byok` `PeppolCredentials` bundle. The vendor travels in `labels["partner.vendor"]`, the participant id becomes the legal-entity id, and SML `Test` / `Acceptance` map to `sandbox = true`.

The submit body is assembled with `serde_json` so an operator-controlled `legal_entity_id` containing JSON metacharacters is escaped rather than able to inject sibling keys.

## Mode

**Mock / bring-your-own-credentials. No live transmission ships in this crate.**

This is a scaffold. What is real and exercised by tests: per-vendor URL construction, per-vendor JSON body construction, HTTP-status-to-error mapping, submission-id percent-encoding, the BYOK credentials bridge, and the `MockHttpClient`. What is **not** here:

- **No real HTTP transport.** Only `MockHttpClient` ships. The module doc-comment notes a `reqwest`-backed client is gated behind a follow-up `reqwest` feature flag that does not yet exist in `Cargo.toml`. Without it, nothing leaves the process.
- **Bring-your-own-credentials.** The customer holds the partner account, the API key/secret, and the legal-entity id; this crate drives the partner's API on their behalf. It mints no credentials.
- **REST-shaped bodies for SOAP vendors.** ecosio and B2BRouter are documented as SOAP APIs, but the adapter currently emits JSON bodies (`/peppol/submit`, `/invoices`) for them. The real SOAP mapping is not implemented.
- **Substring submission-id parser.** On a 2xx submit, the id is extracted by scanning for the first `"id":"..."` substring in the response body, not a per-vendor structured parser. The receipt timestamp is a hard-coded `1970-01-01T00:00:00Z` placeholder, and poll status is `Accepted` only if the body contains the literal bytes `delivered`, else `Pending`.

The live path needs: the `reqwest`-backed `HttpClient`, the per-vendor structured response parsers (real submission-id field and timestamp), the real SOAP request mapping for ecosio and B2BRouter, and a partner account with valid credentials supplied through a `SecretResolver`.

## Residuals

Documented in the module doc-comment and source:

- The vendor REST mapping is "mostly stubbed": each method constructs the URL and body shape and delegates to the `HttpClient`; the unit tests prove only that construction, not a real round-trip.
- The `reqwest`-backed client is a follow-up bead; `MockHttpClient` is the only `HttpClient` that ships.
- `SecretResolver` ships Env + Static backends only; the Vault and SOPS backends the T-091 runbook calls for are future beads.
- `StaticSecretResolver` is the type the Stdin interactive bootstrap flow will build; the readline prompt itself is a future bead.
- `extract_submission_id` is the substring-fallback parser; per-vendor structured parsers are a follow-up.

## References

Vendor API base URLs present in the source (production defaults, overridable via `INVOICEKIT_PEPPOL_API_BASE`):

- Storecove — `https://api.storecove.com/api/v2` (submit path `/document_submissions`)
- ecosio — `https://api.ecosio.com`
- B2BRouter — `https://app.b2brouter.net/projects/-/api`

Standards named in the source:

- RFC 3986 (URL path-segment unreserved set) — used by the percent-encoder.
- RFC 4648 §4 (base64 standard alphabet) — used to wrap the UBL payload.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
