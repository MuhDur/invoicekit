# services/intake-paddleocr — Layer 3 OCR sidecar (T-062)

PaddleOCR HTTP server that the `invoicekit-intake-ocr` Rust
crate's `PaddleOcrProvider` calls. Runs as a side-by-side
container under `deploy/docker-compose.yml`; bound to port
`7001` per the convention `docs/operators/`-level documents
record for the OCR layer.

## Build

```bash
docker build -t invoicekit/intake-paddleocr:scaffold \
  -f services/intake-paddleocr/Dockerfile .
```

## Run

```bash
docker run --rm -p 7001:7001 \
  -e INVOICEKIT_PADDLEOCR_LANG=ch \
  invoicekit/intake-paddleocr:scaffold
```

## Endpoints

### `GET /health`

Returns `{ "status": "ok", "paddleocr_lang": ..., "paddleocr_use_gpu": ... }`.
Engine bootstraps lazily on the first `/recognise` call so
the `HEALTHCHECK` stays fast.

### `POST /recognise`

`multipart/form-data` with a `source` file (PDF / PNG /
JPEG). Returns a typed `OcrResult` JSON shaped to match the
`invoicekit-intake-ocr::OcrResult` Rust struct so the
Provider impl can deserialise directly into the engine.

PDFs are rasterised page-by-page at 200 dpi via
`pdf2image`; per-token `bbox.page` records the source page
index.

## Config (env vars)

| var | default | meaning |
|---|---|---|
| `INVOICEKIT_PADDLEOCR_LANG` | `ch` | PaddleOCR language pack — `ch`, `en`, `german`, `french`, `japanese`, etc. |
| `INVOICEKIT_PADDLEOCR_USE_GPU` | `false` | `true` enables GPU inference inside the container. |

## Layer

Layer 3 in the intake stack per PLAN.md §3.5 (digital PDF →
Factur-X → **PaddleOCR** → SmolDocling-256M ONNX → Qwen2.5-VL).

## Closes

`invoices-t-062-l3-paddleocr-integration-unp`.
