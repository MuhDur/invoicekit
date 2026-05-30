<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-render-verify

Rust-side request/response adapters for the render-verification sidecars. Today it holds one adapter: the client half of the veraPDF PDF/A conformance contract.

This crate does **not** validate PDFs. It builds the JSON-RPC request the JVM-only veraPDF sidecar expects and parses the sidecar's JSON-RPC reply into typed Rust structs. The actual PDF/A grading happens in the isolated `validator-verapdf` sidecar (`services/validator-common`), which wraps the veraPDF library. Callers supply their own HTTP/transport between this adapter and that sidecar.

## Capabilities

The `verapdf` module is the Rust side of the `validator.validate_pdf` JSON-RPC contract:

- `build_request(rpc_id, pdf_bytes, flavour, trace_id) -> Vec<u8>` — serialize the JSON-RPC 2.0 request body. The PDF bytes are base64-encoded into `params.document.pdf_base64`; `flavour` and `trace_id` are passed through. The request is the bytes only; this crate does not send them.
- `parse_response(raw: &[u8]) -> Result<ValidatePdfResult, AdapterError>` — parse a sidecar response body. Checks the envelope is JSON-RPC 2.0, surfaces a JSON-RPC `error` object as `AdapterError::SidecarError { code, message }`, and deserializes `result` into `ValidatePdfResult`.
- `PdfAFlavour` — the three PDF/A flavours the caller can ask for: `Pdfa3A` (`pdfa-3a`), `Pdfa3B` (`pdfa-3b`, the Factur-X default), `Pdfa3U` (`pdfa-3u`). `as_str()` gives the wire spelling.
- `ValidatePdfResult` — the full typed `result` object: `backend`, `service`, `oracle_coordinate`, `oracle_class`, `flavour`, `trace_id`, `duration_ms`, `document: DocumentMeta`, `report: PdfAReport`.
- `PdfAReport` — `flavour`, `trace_id`, `conformant`, `failures: Vec<PdfAFinding>`, and optional `error_class` / `error_message` set when the veraPDF library itself raised an exception. Helpers: `is_clean()` (conformant and no library error) and `rule_ids_with_severity(severity)` (e.g. filter for `"fatal"`).
- `PdfAFinding` — one conformance check failure: `rule_id`, `severity` (`violation` for spec rule failures, `fatal` for library-level errors), `message`, optional `location`.
- `DocumentMeta` — the metadata the sidecar stamps on every response: `content_type`, `byte_length`, hex `sha256` of the body bytes (computed by the sidecar, not this crate).
- `AdapterError` — `BadEnvelope`, `SidecarError { code, message }`, `BadResult`.
- Constants: `RPC_METHOD` (`"validator.validate_pdf"`), `VERAPDF_ADAPTER_BEAD_ID`.
- `crate_name() -> &'static str` — returns `"invoicekit-render-verify"`.

The crate hand-rolls standard MIME base64 encoding internally so it does not pull in a `base64` dependency for one function; remainder handling (0/1/2 trailing bytes, `=` padding) is unit-tested against known vectors.

## Mode / Residuals

- **No PDF/A validation runs here.** All grading is delegated to the external veraPDF sidecar. This crate is purely the wire-contract client: serialize a request, parse a response.
- **No transport.** `build_request` returns bytes and `parse_response` consumes bytes; there is no HTTP client, no connection management, no retries. The doc-comments note that the T-029 RPC client will wrap these; that client is not part of this crate.
- **Single adapter today.** Only the veraPDF adapter exists. The module doc-comment lists future siblings ("PDF/A signature checks, font-embedding audits, etc.") — those are not implemented.
- **The conformance verdict is veraPDF's, not InvoiceKit's.** `conformant`, the rule ids, and severities all come from the sidecar's veraPDF run. This crate forwards them verbatim into typed Rust; it adds no rules of its own and validates none of the verdict's correctness.
- **Wire format is shared with Java.** The JSON shape parsed here mirrors `services/validator-common/.../PdfAReport.java`; any change must land in both places. Unit tests pin a representative passing, failing, RPC-error, and library-error response so an accidental field rename surfaces as a test failure.

## References

Drawn from the source and its sidecar contract:

- veraPDF library — `org.verapdf:verapdf-library` (Maven coordinate carried in `ValidatePdfResult::oracle_coordinate`).
- PDF/A-3 conformance flavours: `pdfa-3a`, `pdfa-3b`, `pdfa-3u`. `pdfa-3b` is the Factur-X default.
- JSON-RPC 2.0 envelope for the `validator.validate_pdf` method.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
