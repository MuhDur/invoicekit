<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-wasm

WebAssembly delivery wrapper around `invoicekit-engine`'s JSON Engine ABI, with feature-flagged country/format bundle advertising.

## What it does

This crate is the browser / edge entry point to the InvoiceKit engine. It exposes one request path — `process_engine_abi_json` — plus a small set of capability and diagnostic helpers. The JavaScript-callable surface (`wasm-bindgen`) is compiled only on `wasm32-*` targets; the same functions are callable natively, so the crate builds and tests as part of `cargo test --workspace`.

`process_engine_abi_json` does two things:

- For the two external-validator operations (`commercial_document.local_validate` and `commercial_document.reference_validate`), it returns a typed `requires_external_backend` error **before** the request reaches the engine. These validator paths cannot run inside a browser/edge WASM artifact, so the wrapper refuses them explicitly rather than silently downgrading. The error carries a `backend` identifier and a remediation string.
- For every other request, it forwards the bytes unchanged to `invoicekit_engine::process_abi_json` and returns its response. The byte contract is the engine's; this crate adds no transformation.

The `wasm-bindgen` export is a thin wrapper that converts to/from `js_sys::Uint8Array` (returned as `Box<[u8]>`).

## Capabilities

- `process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8>` — process one Engine ABI v1 JSON request. Returns the engine's response bytes, or a typed external-backend error JSON for the two validator operations. JS name: `processEngineAbiJson`.
- `compiled_country_bundles() -> Vec<&'static str>` / `compiled_format_bundles() -> Vec<&'static str>` — report which `country-*` / `format-*` cargo features were enabled at compile time, as sorted, deduplicated ISO-style codes (`"DE"`, `"FR"`, `"Peppol"`, ...). JS names: `compiledCountryBundles`, `compiledFormatBundles` (each returns a JSON array string). See the Mode / Residuals section for what these flags do and do not change.
- `RequiresExternalBackend` — typed diagnostic (`code`, `profile_id`, `capability`, `backend`, `remediation`) for validator paths that need a non-WASM backend. `to_json()` emits stable, key-ordered, escaped JSON. JS name: `requiresExternalBackendJson`. Default backend per known profile: `xrechnung*` → `jvm:kosit`, `peppol*` → `jvm:phive`, `factur-x*` → `verapdf`, `fatturapa*` → `partner:sdi`, otherwise `external-validator`.
- `require_external_backend(...)` — always returns `Err(RequiresExternalBackend)`; the building block for the refusal above.
- `crate_name() -> &'static str` — returns `"invoicekit-wasm"`.
- Bead-id constants (`WASM_ARTIFACT_BEAD_ID`, `CAPABILITY_MATRIX_BEAD_ID`) and operation-name constants for the two validator operations. `beadId` is reachable from JS for diagnostic correlation.

### Feature flags

`default` is empty: the leanest consumer gets the engine ABI surface and nothing extra. `country-be|br|de|es|fr|gr|hu|in|it|mx|pl|ro|sa|tr` and `format-cii|factur-x|fatturapa|peppol|ubl|xrechnung` are the individual bundle flags; `full` toggles all of them and is used by the artifact-size CI gate to assert the full bundle still fits the < 5 MB cap.

## Mode / Residuals

- **The bundle feature flags are advertising-only.** In `Cargo.toml` every `country-*` and `format-*` feature is defined as an empty list (`= []`) and the `invoicekit-engine` dependency is declared with no `features = [...]` wiring. Enabling a flag changes only what `compiled_country_bundles()` / `compiled_format_bundles()` return — it does **not** toggle any feature on the engine crate, and the compiled engine behavior (which countries and formats actually work) is identical regardless of which flags are set. The flags are a runtime capability-advertisement surface, not a code-stripping mechanism. Per-bundle code stripping (toggling the engine's `report-*` / `format-*` features) is a no-op today and lands in a follow-up once the engine feature wiring exists.
- **External validators are refused, not stubbed.** `commercial_document.local_validate` and `commercial_document.reference_validate` never produce a validation result here; they always return the typed `requires_external_backend` error. Reference validation requires a JVM sidecar (KoSIT, phive), veraPDF, or a partner service. This is intentional: the wrapper does not silently fall back to in-WASM validation.
- **All other operations delegate verbatim** to `invoicekit_engine::process_abi_json`. The capabilities, guarantees, and limitations of those operations live in `invoicekit-engine`, not here.
- The external-backend detection only triggers for valid UTF-8, `abi_version == 1` requests carrying a string `payload.profile_id`; anything else falls through to the engine, which returns its own error JSON for unknown/malformed requests.

## Build

```bash
cargo build \
  --release \
  --no-default-features \
  --features "country-de,country-fr,country-it,format-peppol" \
  --target wasm32-unknown-unknown
```

`wasm-pack` is the recommended publisher; `.github/workflows/wasm-artifact.yml` drives it and uploads the bundle as a release artifact. The crate is `publish = false`.

## Testing

Unit tests pin the typed-error JSON shape and escaping, the default-backend resolution per known profile, the empty-bundle behavior under default features, the sorted/deduplicated bundle invariants, and a full-meta-feature completeness check. `wasm_wrapper_matches_engine_abi_golden_fixture` asserts the wrapper reproduces the recorded engine response byte-for-byte against `conformance-corpus/golden/engine-abi-v1-commercial-document.json`.

## References

- Engine ABI v1 — `invoicekit_engine::process_abi_json` (the byte contract this crate wraps).
- External-validator backends named in the code: KoSIT (`jvm:kosit`), phive (`jvm:phive`), veraPDF (`verapdf`), Italy SdI partner (`partner:sdi`).

## License

Apache-2.0. Part of the InvoiceKit workspace; `publish = false`.
