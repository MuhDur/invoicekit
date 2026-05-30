# invoicekit-replay

Re-runs the pipeline recorded in an evidence bundle and reports whether the freshly-produced artefacts are byte-equal to the recorded ones, or emits a structured diff.

## What it does

Given an unpacked `invoicekit_evidence::EvidenceBundle`, `replay` walks the recorded artefacts, asks an injected `PipelineReplayer` to re-produce each one, and reconciles the re-emitted bytes against the recorded bytes. The result is a `ReplayReport`: a stable, lexicographically ordered map of per-artefact verdicts plus an aggregate `ok` flag. A byte-for-byte match means the engine reproduced the recorded output; any divergence means the re-emitted bytes no longer match the recorded bytes and is an operator signal worth alerting on. `replay` does not attribute the cause of a divergence (see Mode / Residuals).

Comparison is by BLAKE3 content hash, taken from `invoicekit_evidence::blake3_hex` (a real BLAKE3 over the bytes, lowercase hex). The hash here is used only for content-addressing the diff — this crate performs no signing, encryption, or signature verification, and treats the hash purely as an equality key.

## Capabilities

- `replay(bundle, replayer, options) -> Result<ReplayReport, ReplayError>` — iterate the bundle's recorded artefacts (filtered by `ReplayOptions`), call the replayer once per selected id, and reconcile.
- `PipelineReplayer` trait — the injection point. `replay_artefact` returns `Ok(Some(bytes))` to diff, `Ok(None)` to signal the engine failed to reproduce a selected output, and `Err` for transport/engine errors that fail the whole run. `extra_artefacts` (default empty) lets a replayer surface artefacts the bundle never recorded.
- `ReplayOptions` — `only` (allow-list of artefact ids; empty means all) and `ignore` (skip-list applied after `only`). `all()`, `only(ids)`, and chainable `ignoring(id)` builders.
- `ArtefactDelta` — per-artefact verdict: `ByteEqual` (hash recorded), `Drifted` (expected/observed hash and size), `NotReplayed` (replayer returned `None` for a selected artefact), `Unexpected` (replayer emitted an id the bundle does not record). `is_byte_equal()` and `is_diff()` classify a delta.
- `ReplayReport` — `ok` is true only when every selected artefact is byte-equal; `Drifted`, `Unexpected`, and `NotReplayed` all pull it false. `drifted_ids()` iterates the diverging ids. Serializes to stable JSON via serde.
- `IdentityReplayer` — re-emits each recorded artefact unchanged.
- `MutatingReplayer` — drifts named artefact ids (appends a fixed suffix) and can emit `extra` artefacts; used to exercise the drift/unexpected paths.
- `crate_name() -> &'static str` — returns `"invoicekit-replay"`.

The filter runs before the replayer is consulted, so operator-ignored artefacts never produce a `NotReplayed` verdict; any `NotReplayed` in a report means a selected artefact the engine failed to reproduce.

## Mode / Residuals

- This crate ships the replay machinery and the diff/report types, not the real engine. The doc-comment and the public surface state that the eventual `invoicekit replay` subcommand wires a real engine crate behind the `PipelineReplayer` trait. That wiring does not live here.
- The only concrete `PipelineReplayer` implementations shipped — `IdentityReplayer` and `MutatingReplayer` — are deterministic stubs for exercising the byte-equal, drift, and unexpected-artefact paths without dragging the engine into the test target. They are not the production replayer. `IdentityReplayer` will always report byte-equal because it returns the recorded bytes unchanged.
- No cryptography or tamper-proofing is implemented in this crate. The BLAKE3 hash (supplied by `invoicekit-evidence`) is a content-equality key, not a signature or message authentication code. Detecting that "the bundle was tampered with" relies on the bundle's own integrity guarantees and on the upstream engine producing the same bytes — `replay` only observes that re-emitted bytes differ from recorded bytes; it does not attribute the cause.
- `replay` returns `Err(ReplayError)` only for replayer-raised transport/engine errors; drift, `None`, and unexpected artefacts are reported as deltas, not errors.

## References

No external specifications or URLs are referenced in the source. The crate depends on `invoicekit-evidence` (the `.invoicekit`/`.ikb` bundle format and its BLAKE3 manifest).

## License

Apache-2.0. Part of the InvoiceKit workspace.
