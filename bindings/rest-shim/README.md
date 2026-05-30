# invoicekit-binding-rest-shim

An Axum-based REST sidecar that bridges the InvoiceKit engine Application Binary Interface (ABI) to plain HTTP, for runtimes that cannot or do not want to load the native bindings (for example the Go no-cgo fallback path).

## Capabilities

The crate builds an Axum `Router` (`build_router`, `build_router_with_state`) and an async listener (`serve`). It exposes:

- `POST /v1/engine/process_json` — passes the request through `invoicekit_engine::process_abi_json` and returns the canonical engine response, base64-encoded, with a C-ABI-compatible status (0 ok, 1 error). The public function `process_engine_abi_json` is the same passthrough. A test asserts this matches the Go no-cgo fallback contract against a golden fixture.
- `POST /v1/invoices` — accepts either a raw Engine ABI envelope or a bare `CommercialDocument` JSON object (wrapped into a canonicalize operation), runs it through the engine, revalidates the returned document via `invoicekit_ir::CommercialDocument`, packs an evidence bundle, and stores the result. An optional `Idempotency-Key` header feeds the derived identifier.
- `POST /v1/invoices/{id}/validate` — re-runs the stored Engine ABI request through the engine.
- `POST /v1/invoices/{id}/render` — renders a PDF via `invoicekit_render_pdf::render_commercial_document_invoice`, returned as `application/pdf`.
- `POST /v1/invoices/{id}/transmit` and `GET /v1/transmissions/{id}` — create and read a transmission tracking record.
- `POST /v1/reconcile` — reports, per supplied invoice identifier, whether it is currently present in the store.
- `GET /v1/bundles/{id}` — returns the packed evidence bundle bytes (`application/vnd.invoicekit.bundle`).
- `POST /v1/bundles/verify` — verifies an uploaded bundle via `invoicekit_verify::verify_packed` and returns a per-check report.
- `GET /v1/capabilities` — serves a capability matrix compiled into the binary (`crates/cli/data/capabilities/matrix.json`), optionally filtered by `from`, `to`, `date`, `scenario`.
- `GET /healthz`, `GET /v1/healthz` — liveness payload (status, service name, crate version).
- `GET /openapi.json`, `GET /v1/openapi.json` — a deterministic OpenAPI 3.1 document generated from the Rust data-transfer-object schemas with `schemars`, plus an `x-invoicekit-openapi-sha256` response header pinning the exact body. `openapi_document`, `openapi_document_bytes`, and `openapi_sha256_hex` expose the same document and hash for release tooling.

Identifiers for invoices, bundles, and transmissions are derived from a BLAKE3 hash of the relevant inputs.

## Mode / Residuals

This is a thin, self-contained shim, not a production managed API. Real versus stub:

- **Storage is in-memory only.** `RestShimState` holds invoices and transmissions in `Arc<RwLock<BTreeMap<...>>>`. There is no database; all state is lost on restart. The doc-comment notes a production API "can replace this with persistent stores without changing the route contracts" — that persistence layer is not implemented here.
- **Transmission is a mock.** `POST /v1/invoices/{id}/transmit` always records `gateway: "mock"` and returns `state: "accepted"`. There is no Peppol, AS4, or any access-point delivery — no network call leaves the process. The state machine is a fixed single state.
- **Reconcile is a presence check only.** It reports whether each identifier exists in the in-memory store; it does not reconcile against any external system or status.
- **Bundle verification runs content-only checks.** `verify_bundle` calls `verify_packed` with `VerifyOptions::content_only()`, so per-artefact re-hashing and manifest reconciliation run, while the signature, DSSE manifest-envelope, and RFC 3161 timestamp checks are reported as skipped (not failed). The response type can carry pass/fail for those checks, but this endpoint does not request them.
- **Bundle `created_at` is a fixed constant** (`2026-01-01T00:00:00Z`), not a wall-clock timestamp.
- **No deployment glue in the library.** No TLS, authentication, rate limiting, or persistence is provided by the crate. A `Dockerfile` (referenced by tests; binds `0.0.0.0:8081`, exposes 8081, self-healthcheck) lives alongside the crate but the binary it builds is not in this library source.
- **The generated OpenAPI document lists `https://api.invoicekit.org` as a "Hosted API" server.** This crate does not host or run that endpoint; the URL is a static entry in the generated spec only.

## References

- `plans/PLAN.md` section 5.5 — the REST surface this shim implements (referenced in the module doc-comment).
- OpenAPI 3.1.0 — the generated specification format.
- RFC 3161 — named as the timestamp-check kind in the verification report type (the timestamp check itself is delegated to `invoicekit-verify` and skipped by this endpoint).

## License

Apache-2.0.
