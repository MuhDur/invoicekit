# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""SmolDocling-256M ONNX sidecar for InvoiceKit intake Layer 4.

Wraps an `onnxruntime.InferenceSession` over the SmolDocling
small-VLM checkpoint. Same I/O contract as
`services/intake-paddleocr`: POST /recognise with a multipart
`source`, get back a typed `OcrResult` JSON shaped to match
the `invoicekit-intake-ocr::OcrResult` Rust struct.

The InvoiceKit engine targets this sidecar via the Rust
`SmolDoclingProvider`; the operator passes the in-container
ONNX file path through `INVOICEKIT_SMOLDOCLING_MODEL_PATH`,
mounted from the host as a read-only volume.
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

_ort_session = None


def _get_session():
    """Lazy-load the ONNX session.

    Cached for the process lifetime. The actual SmolDocling
    inference graph is loaded from
    `INVOICEKIT_SMOLDOCLING_MODEL_PATH`; if the file is
    missing the loader fails fast so the operator gets a clear
    error rather than a silent fallback.
    """
    global _ort_session
    if _ort_session is None:
        import onnxruntime as ort  # type: ignore

        model_path = os.environ.get(
            "INVOICEKIT_SMOLDOCLING_MODEL_PATH",
            "/var/lib/invoicekit/smoldocling.onnx",
        )
        if not os.path.exists(model_path):
            raise FileNotFoundError(
                f"SmolDocling model not found at {model_path}; "
                "mount the ONNX checkpoint as a volume"
            )
        device = os.environ.get("INVOICEKIT_SMOLDOCLING_DEVICE", "cpu").lower()
        providers = (
            ["CUDAExecutionProvider", "CPUExecutionProvider"]
            if device == "cuda"
            else ["CPUExecutionProvider"]
        )
        logging.info(
            "loading SmolDocling onnx model=%s providers=%s", model_path, providers
        )
        _ort_session = ort.InferenceSession(model_path, providers=providers)
    return _ort_session


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
    model_path: str
    device: str
    model_loaded: bool


app = FastAPI(
    title="invoicekit-intake-smoldocling",
    summary="Layer 4 server-side SmolDocling-256M ONNX sidecar",
    version="0.1.0",
)


@app.get("/health", response_model=HealthResult)
def health() -> HealthResult:
    return HealthResult(
        status="ok",
        model_path=os.environ.get(
            "INVOICEKIT_SMOLDOCLING_MODEL_PATH",
            "/var/lib/invoicekit/smoldocling.onnx",
        ),
        device=os.environ.get("INVOICEKIT_SMOLDOCLING_DEVICE", "cpu"),
        model_loaded=_ort_session is not None,
    )


def _rasterise_pdf(pdf_bytes: bytes) -> List[bytes]:
    from pdf2image import convert_from_bytes  # type: ignore

    images = convert_from_bytes(pdf_bytes, dpi=200, fmt="png")
    out: List[bytes] = []
    for img in images:
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        out.append(buf.getvalue())
    return out


def _recognise_one_page(page_idx: int, png_bytes: bytes) -> List[OcrToken]:
    """Run a single page through SmolDocling.

    The current shipped scaffold runs the ONNX session in a
    smoke-test mode: it confirms the model can be loaded and
    returns a single page-level token spanning the rendered
    image. The full token-level decoder lands with the
    follow-up bead T-066 (bounding-box citation taxonomy).
    """
    session = _get_session()
    from PIL import Image  # type: ignore

    with tempfile.NamedTemporaryFile(suffix=".png", delete=True) as f:
        f.write(png_bytes)
        f.flush()
        img = Image.open(f.name)
        width, height = img.size

    inputs = {inp.name: None for inp in session.get_inputs()}
    if any(v is None for v in inputs.values()):
        return [
            OcrToken(
                text="SCAFFOLD-smoldocling-page",
                bbox=BoundingBox(
                    page=page_idx,
                    x=0.0,
                    y=0.0,
                    width=float(width),
                    height=float(height),
                ),
                confidence=0.0,
            )
        ]
    return []


@app.post("/recognise", response_model=OcrResult)
async def recognise(source: UploadFile = File(...)) -> OcrResult:
    body = await source.read()
    if not body:
        raise HTTPException(status_code=400, detail="empty source")

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
        layer="smoldocling",
        tokens=all_tokens,
        page_count=len(pages),
        mean_confidence=mean_conf,
    )


@app.exception_handler(Exception)
async def _unhandled(_request, exc: Exception) -> JSONResponse:
    logging.exception("unhandled error")
    return JSONResponse(
        status_code=500,
        content={"layer": "smoldocling", "error": str(exc)},
    )
