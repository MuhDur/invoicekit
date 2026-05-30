<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-intake-vlm

Typed surface and retry/cost wiring for the intake pipeline's Layer 5 vision-language-model step. Targets Qwen2.5-VL-7B.

## What it does

The intake pipeline routes a document through five layers. Layer 5 — the
vision-language model — is the most expensive, so the engine reaches it only when
Layers 1 through 4 (digital PDF, Factur-X, PaddleOCR, SmolDocling) all failed to
reach acceptable confidence. This crate is the typed seam for that step.

It does not run any model and does not parse any image or PDF. There is no
inference, no HTTP client, and no Qwen weights in this crate. What it ships is:

- The result and error types the rest of the pipeline depends on (`VlmResult`,
  `VlmField`, `VlmError`).
- An injectable `VlmTransport` trait. A follow-up `intake-vlm-http` crate is meant
  to provide the live `reqwest`-backed implementation that POSTs to the operator's
  endpoint; that crate does not exist yet.
- `Qwen25Vl7bProvider`, which wraps a `VlmTransport` with retry-on-rate-limit, a
  hard timeout passthrough, auth-failure surfacing, and cost telemetry — but the
  actual network call is whatever transport you hand it.
- `MockVlmProvider`, a deterministic stub that returns a fixed three-field result
  for tests and engine wiring.

So the crate is the plumbing and the contract. The thing that actually reads pixels
and talks to a model is out of scope here and not yet written.

## Capabilities

- `VlmProvider` trait — `model()` and `extract(source_bytes) -> Result<VlmResult, VlmError>`.
- `Qwen25Vl7bProvider` — production-shaped provider built over an injectable
  `VlmTransport`:
  - Retries on `TransportOutcome::RateLimited`, honouring the upstream
    `Retry-After` hint, bounded by `RetryPolicy` (`max_attempts` and
    `max_total_backoff_secs`). Gives up with `VlmError::RateLimited` when either
    bound is hit.
  - Surfaces `Timeout`, `Auth`, and `Provider` outcomes immediately as the matching
    `VlmError` variant (no retry).
  - Rejects empty source bytes (`BadSource`), empty `endpoint_url` (`Provider`), and
    empty `api_key_ref` (`Auth`) before calling the transport.
  - Records one `CostTelemetry` row per successful call (model, billed tokens, cost
    in micro-USD, attempt count) to a pluggable `CostTelemetrySink`.
- `RetryPolicy::production()` — 3 attempts, total backoff capped at 60 seconds.
  `RetryPolicy::no_retry()` — single attempt.
- Telemetry sinks: `TracingTelemetry` (emits a `tracing::info` event on
  `invoicekit_intake_vlm::cost`) and `InMemoryTelemetry` (collects rows for
  assertions via `snapshot()`).
- `MockVlmProvider` — deterministic stub. Returns three fields (`BT-1`, `BT-2`,
  `BT-5`) with `billed_tokens` and `cost_micro_usd` both zero. Rejects empty input.
- `crate_name() -> &'static str` — returns `"invoicekit-intake-vlm"`.

`VlmField` carries the EN 16931 BT/BG term id, the extracted value as a UTF-8 string
(numeric fields stay strings to preserve the issuer's formatting), and a
model-reported `confidence` in `[0.0, 1.0]`. `VlmResult` aggregates the fields,
their mean confidence, billed tokens, and per-call cost in micro-USD (integer,
`1_500` = $0.0015). `VlmResult`, `VlmField`, and `VlmModel` are `serde`-serializable.

## Mode / Residuals

This crate is a contract plus a backoff/telemetry wrapper. It is not a working
extractor on its own.

- **No live transport.** `Qwen25Vl7bProvider::extract` is only as real as the
  `VlmTransport` you pass it. The intended live implementation
  (`intake-vlm-http`, `reqwest`-backed) is named in the source but not yet built.
  With no real transport, this crate cannot extract anything from a real document.
- **Mock returns fixed data.** `MockVlmProvider` ignores the content of
  `source_bytes` beyond an emptiness check and always returns the same three fields.
  It exists for engine wiring tests, not extraction.
- **Confidence is whatever the upstream reports.** The crate does not compute, model,
  or validate confidence; `mean_confidence` is a plain arithmetic mean of the
  fields the transport returned.
- **No image/PDF decoding here.** `source_bytes` is passed through to the transport
  unparsed. The crate does not validate that bytes are a real PDF/PNG/JPEG; that is
  the transport's (or upstream model's) job. The `BadSource` variant is only raised
  for empty input in this crate.
- **No Unicode / right-to-left / Chinese-Japanese-Korean handling.** Whatever text
  extraction, script handling, or normalization the model performs happens upstream;
  this crate stores returned values as opaque UTF-8 strings and does nothing further
  with them.
- **`sleep_seconds` is a no-op under `cfg(test)`.** Tests verify the retry policy
  without real sleeping; production builds sleep for the upstream-suggested seconds.
- Extraction through any vision-language model is best-effort and unverified by this
  crate. Downstream confidence gating and validation are someone else's
  responsibility.

## References

- EN 16931 BT/BG term identifiers (e.g. `BT-1`, `BG-7`) — referenced as the field
  vocabulary for `VlmField.term`.
- Qwen2.5-VL-7B — named as the targeted Layer 5 model.

No external URLs or specification documents are embedded in the source.

## License

Apache-2.0.
