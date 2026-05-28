# services/intake-smoldocling — Layer 4 SmolDocling sidecar (T-063)

ONNX-runtime HTTP server for the SmolDocling-256M small-VLM
checkpoint. Same I/O contract as the
`intake-paddleocr` sidecar so the Rust
`SmolDoclingProvider` can deserialise the response straight
into the engine.

## Build

```bash
docker build -t invoicekit/intake-smoldocling:scaffold \
  -f services/intake-smoldocling/Dockerfile .
```

## Run

The container expects the SmolDocling ONNX checkpoint to be
mounted as a read-only volume. The default in-container
path is `/var/lib/invoicekit/smoldocling.onnx`.

```bash
docker run --rm -p 7002:7002 \
  -v /host/path/to/smoldocling.onnx:/var/lib/invoicekit/smoldocling.onnx:ro \
  invoicekit/intake-smoldocling:scaffold
```

## Endpoints

### `GET /health`

Returns `{ "status": "ok", "model_path": ..., "device": ..., "model_loaded": ... }`.
The ONNX session loads lazily on the first `/recognise` so
the `HEALTHCHECK` stays fast — `model_loaded` only flips
true after the first inference call.

### `POST /recognise`

`multipart/form-data` with a `source` file (PDF / PNG /
JPEG). Returns a typed `OcrResult` JSON shaped to match
`invoicekit-intake-ocr::OcrResult`.

The current scaffold confirms the model can be loaded and
emits a page-level smoke token; the full token-level decoder
lands with the follow-up T-066 bounding-box citation
taxonomy bead.

## Config (env vars)

| var | default | meaning |
|---|---|---|
| `INVOICEKIT_SMOLDOCLING_MODEL_PATH` | `/var/lib/invoicekit/smoldocling.onnx` | container path to mounted ONNX file |
| `INVOICEKIT_SMOLDOCLING_DEVICE` | `cpu` | `cuda` flips to `CUDAExecutionProvider` |

## Layer

Layer 4 in the intake stack per PLAN.md §3.5
(`digital PDF → Factur-X → PaddleOCR → **SmolDocling-256M ONNX** → Qwen2.5-VL`).

## Closes

`invoices-t-063-l4-smoldocling-onnx-s0w`.
