<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-engine

Deterministic entry point for the InvoiceKit Engine ABI v1: it takes request bytes, validates and canonicalizes a commercial document, and returns canonical JSON response bytes.

## What it is

This crate defines the byte-level request/response contract that the InvoiceKit native bindings, WebAssembly build, and service shims call into. It is a thin dispatcher: it parses a JSON request envelope, routes the single implemented operation through `invoicekit-ir` (validation) and `invoicekit-canonical` (canonicalization), and hands back deterministic bytes. The same input always produces byte-identical output across runs.

## Capabilities

- `process_abi_json(request_bytes: &[u8]) -> Vec<u8>` — the one public processing function. It:
  - decodes the request as UTF-8, then canonicalizes the request JSON (RFC 8785 via `invoicekit-canonical`) before parsing, so duplicate object members and unsafe I-JSON numbers are rejected up front;
  - parses the envelope `{ "abi_version", "operation", "payload" }` with unknown fields rejected;
  - for the `commercial_document.canonicalize` operation, builds a `CommercialDocument` from the payload (`invoicekit-ir`), re-serializes it, and canonicalizes the result;
  - returns a success object containing both `canonical_document_json` (a canonical string) and the parsed `document` value.
- Never panics on user input. Invalid UTF-8, invalid/duplicate-member JSON, unsupported ABI versions, unsupported operations, invalid IR payloads, and internal serialization failures are all returned as canonical JSON error objects with a stable `code`, a `message`, and a `remediation` string. Error `code` values: `invalid_utf8`, `invalid_request_json`, `invalid_request_envelope`, `unsupported_abi_version`, `unsupported_operation`, `invalid_commercial_document`, `serialize_commercial_document`, `canonicalize_response`, plus an `internal_response_serialization` fallback used if even the error object fails to canonicalize.
- Public constants and helpers: `ENGINE_ABI_VERSION` (`1`), `COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION` (`"commercial_document.canonicalize"`), `crate_name()` (`"invoicekit-engine"`), and the `EngineAbiError` enum (public so wrappers can assert on it in conformance tests).
- Emits a `tracing` debug record per processed request carrying the operation name, `tenant_id`, and `trace_id` from the document metadata.

## Mode / Residuals

- Exactly one operation is implemented: `commercial_document.canonicalize`. Any other operation string returns `unsupported_operation`. Any `abi_version` other than `1` returns `unsupported_abi_version`.
- This crate is dispatch only. It does no signing, hashing, encryption, transport, or rendering. All validation logic lives in `invoicekit-ir`; all byte-stability guarantees come from `invoicekit-canonical`. It contains no cryptography and no placeholder crypto.
- "Validation" here means whatever `CommercialDocument::try_from_value` enforces (required non-empty metadata such as `tenant_id`/`trace_id`, and the IR structural checks). It is not format-profile conformance (Universal Business Language, Cross Industry Invoice, EN 16931, Peppol, and so on); those run in the format, profile, and validate crates further down the pipeline.
- Determinism is asserted by tests: a golden fixture (`conformance-corpus/golden/engine-abi-v1-commercial-document.json`) must match byte-for-byte, and a property test confirms `process_abi_json` returns identical bytes across two runs for arbitrary byte inputs. The crate is `publish = false`.

## References

- RFC 8785 JSON Canonicalization Scheme — applied transitively through `invoicekit-canonical`; this crate cites it in error remediation text ("Send RFC 8259 JSON without duplicate object members or unsafe I-JSON numbers").

## License

Apache-2.0.
