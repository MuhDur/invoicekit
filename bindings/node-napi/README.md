# invoicekit-binding-node-napi

Placeholder Rust crate that forwards Engine ABI v1 JSON request bytes to the InvoiceKit engine. It does not yet bridge the engine to Node.

## Capabilities

- `process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8>` — passes the request bytes straight through to `invoicekit_engine::process_abi_json` and returns the canonical JSON response bytes. No transformation is added by this crate.
- `crate_name() -> &'static str` — returns the constant string `"invoicekit-binding-node-napi"`, used by release tooling and bead-correlation reports.
- A test that runs the shared Engine ABI golden fixture (`conformance-corpus/golden/engine-abi-v1-commercial-document.json`) through the wrapper, pinning the byte contract.

## Mode / Residuals

Stub binding. Despite the `node-napi` name, this crate contains no Node binding code:

- No napi-rs (or any other) dependency. The only runtime dependency is `invoicekit-engine`.
- No N-API surface, no native addon, no `index.node`, no JavaScript/TypeScript package, no `package.json`.
- No marshalling between JavaScript values and the engine ABI. The single function operates on Rust `&[u8]` / `Vec<u8>`.

What it actually is: a Rust library that re-exports one engine call plus a name constant, kept in the tree so the Node binding track is pinned to the engine's byte contract via the golden-fixture test. The crate's own doc-comment states the napi-rs package surface will be added by a later TypeScript SDK bead; that work is not present here.

The crate is `publish = false`.

The forwarded engine function never panics on user input; invalid UTF-8, invalid JSON, unsupported ABI versions, unsupported operations, and invalid payloads come back as canonical JSON error responses. That behavior lives in `invoicekit-engine`, not in this crate.

## References

- Engine ABI v1 contract: `crates/invoicekit-engine`.
- Golden fixture exercised by the test: `conformance-corpus/golden/engine-abi-v1-commercial-document.json`.

(No external specifications or URLs are referenced in this crate's source.)

## License

Apache-2.0
