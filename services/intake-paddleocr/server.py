# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""PaddleOCR sidecar HTTP server for InvoiceKit intake Layer 3.

POST /recognise with a `multipart/form-data` body containing
the `source` PDF/PNG/JPEG bytes. Returns a typed OcrResult
JSON shaped to match the `invoicekit-intake-ocr` Rust crate's
`OcrResult` so the `PaddleOcrProvider` impl can deserialise
directly into the engine.

The server pins to PaddleOCR 3.0.1 + FastAPI 0.115 inside
the official PaddlePaddle 3.1.0 image. Configuration via
env vars:

- `INVOICEKIT_PADDLEOCR_LANG` (default `ch`): PaddleOCR
  language pack — `ch`, `en`, `german`, `french`,
  `japanese`, etc.
- `INVOICEKIT_PADDLEOCR_USE_GPU` (default `false`): set
  `true` to enable GPU inference inside the container.
"""

from __future__ import annotations

import io
import logging
import os
import statistics
import tempfile
from typing import List, Optional

from fastapi import FastAPI, File, HTTPException, UploadFile
from fastapi.responses import JSONResponse
from pydantic import BaseModel

# PaddleOCR has a heavy import path; lazy-import the engine
# inside the bootstrap to keep the FastAPI module load fast
# enough for the Docker HEALTHCHECK.
_ocr_engine = None


def _get_engine():
    """Lazily build a `PaddleOCR` engine using the env config.

    Cached so subsequent requests reuse the same in-process
    inference graph.
    """
    global _ocr_engine
    if _ocr_engine is None:
        from paddleocr import PaddleOCR  # type: ignore

        lang = os.environ.get("INVOICEKIT_PADDLEOCR_LANG", "ch")
        use_gpu = (
            os.environ.get("INVOICEKIT_PADDLEOCR_USE_GPU", "false").lower()
            == "true"
        )
        logging.info("loading PaddleOCR lang=%s use_gpu=%s", lang, use_gpu)
        _ocr_engine = PaddleOCR(
            use_angle_cls=True,
            lang=lang,
            use_gpu=use_gpu,
            show_log=False,
        )
    return _ocr_engine


class BoundingBox(BaseModel):
    page: int
    x: float
    y: float
    width: float
    height: float


class OcrToken(BaseModel):
    text: str
    bbox: BoundingBox
    confidence: float


class OcrResult(BaseModel):
    layer: str
    tokens: List[OcrToken]
    page_count: int
    mean_confidence: float


class HealthResult(BaseModel):
    status: str
    paddleocr_lang: str
    paddleocr_use_gpu: bool


app = FastAPI(
    title="invoicekit-intake-paddleocr",
    summary="Layer 3 server-side PaddleOCR sidecar for InvoiceKit intake",
    version="0.1.0",
)


@app.get("/health", response_model=HealthResult)
def health() -> HealthResult:
    return HealthResult(
        status="ok",
        paddleocr_lang=os.environ.get("INVOICEKIT_PADDLEOCR_LANG", "ch"),
        paddleocr_use_gpu=(
            os.environ.get("INVOICEKIT_PADDLEOCR_USE_GPU", "false").lower()
            == "true"
        ),
    )


def _rasterise_pdf(pdf_bytes: bytes) -> List[bytes]:
    """Rasterise a PDF to PNG bytes per page via pdf2image.

    Returns a list of PNG bytes, one per page. Callers feed
    each PNG into PaddleOCR independently so per-token
    bounding boxes can record the correct page index.
    """
    from pdf2image import convert_from_bytes  # type: ignore

    images = convert_from_bytes(pdf_bytes, dpi=200, fmt="png")
    out: List[bytes] = []
    for img in images:
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        out.append(buf.getvalue())
    return out


def _recognise_one_page(page_idx: int, png_bytes: bytes) -> List[OcrToken]:
    engine = _get_engine()
    with tempfile.NamedTemporaryFile(suffix=".png", delete=True) as f:
        f.write(png_bytes)
        f.flush()
        result = engine.ocr(f.name, cls=True)
    tokens: List[OcrToken] = []
    if not result or not result[0]:
        return tokens
    for line in result[0]:
        box_coords, (text, score) = line
        xs = [p[0] for p in box_coords]
        ys = [p[1] for p in box_coords]
        x_min, y_min = min(xs), min(ys)
        x_max, y_max = max(xs), max(ys)
        tokens.append(
            OcrToken(
                text=text,
                bbox=BoundingBox(
                    page=page_idx,
                    x=float(x_min),
                    y=float(y_min),
                    width=float(x_max - x_min),
                    height=float(y_max - y_min),
                ),
                confidence=float(score),
            )
        )
    return tokens


@app.post("/recognise", response_model=OcrResult)
async def recognise(source: UploadFile = File(...)) -> OcrResult:
    body = await source.read()
    if not body:
        raise HTTPException(status_code=400, detail="empty source")

    # Decide rasterise-vs-direct from the MIME hint. Anything
    # that isn't a PDF goes straight to PaddleOCR as a
    # single-page image.
    mime = (source.content_type or "").lower()
    if mime == "application/pdf" or body[:4] == b"%PDF":
        pages = _rasterise_pdf(body)
    else:
        pages = [body]

    all_tokens: List[OcrToken] = []
    for page_idx, png in enumerate(pages):
        all_tokens.extend(_recognise_one_page(page_idx, png))

    mean_conf = statistics.fmean(t.confidence for t in all_tokens) if all_tokens else 0.0
    return OcrResult(
        layer="paddle-ocr",
        tokens=all_tokens,
        page_count=len(pages),
        mean_confidence=mean_conf,
    )


@app.exception_handler(Exception)
async def _unhandled(_request, exc: Exception) -> JSONResponse:
    logging.exception("unhandled error")
    return JSONResponse(
        status_code=500,
        content={"layer": "paddle-ocr", "error": str(exc)},
    )
