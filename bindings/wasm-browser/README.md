# invoicekit-binding-wasm-browser

A thin Rust wrapper that forwards the InvoiceKit Engine ABI to the browser binding track. The browser-facing surface (wasm-bindgen, JavaScript package) is not yet present in this crate.

## Capabilities

- `process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8>` — passes the request bytes straight to `invoicekit_engine::process_abi_json` and returns the engine's canonical response bytes. The wrapper adds no logic of its own.
- `crate_name() -> &'static str` — returns the constant `"invoicekit-binding-wasm-browser"` for release tooling and bead-correlation reports.
- A test pins the wrapper to the shared Engine ABI golden fixture (`conformance-corpus/golden/engine-abi-v1-commercial-document.json`), asserting byte-for-byte equality with the engine's output.

## Mode / Residuals

This is a stub binding, not a complete browser binding.

- It bridges nothing host-specific yet. There is no `wasm-bindgen` surface, no `#[wasm_bindgen]` exports, no JavaScript or npm package, and no WebAssembly target configuration in `Cargo.toml`. The crate compiles as a plain Rust library that re-exports one engine function.
- `Cargo.toml` sets `publish = false`. The crate is internal and not published.
- All request validation, canonicalization, error handling, and operation dispatch live in `invoicekit-engine`. Per its own doc-comment, an unknown operation produces a `{"status":"error",...}` response; this crate inherits that behavior unchanged.
- The crate doc-comment states that "the browser bundle bead will add the JavaScript package and wasm-bindgen surface" — confirming the browser binding is future work. The crate exists now to pin the browser track to the engine's byte contract via the golden-fixture test.

## References

- Engine ABI golden fixture: `conformance-corpus/golden/engine-abi-v1-commercial-document.json` (referenced by the crate's test via `include_str!`).

No external specifications or URLs are referenced in the source.

## License

Apache-2.0
