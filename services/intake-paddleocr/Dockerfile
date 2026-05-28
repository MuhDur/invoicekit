# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

# T-062: Layer 3 intake — server-side PaddleOCR sidecar.
# Wraps PaddleOCR's `PPOCR` pipeline behind a small FastAPI
# HTTP server so the invoicekit-intake-ocr Rust crate's
# PaddleOcrProvider can POST PDF/image bytes and receive a
# typed OcrResult JSON.
#
# Image is pinned to PaddlePaddle's official slim base so
# the GPU/CPU runtime is deterministic across reruns.

FROM paddlepaddle/paddle:3.1.0 AS runtime
WORKDIR /app

RUN python3 -m pip install --no-cache-dir \
        "paddleocr==3.0.1" \
        "fastapi==0.115.0" \
        "uvicorn[standard]==0.31.0" \
        "pdf2image==1.17.0" \
    && apt-get update \
    && apt-get install -y --no-install-recommends poppler-utils \
    && rm -rf /var/lib/apt/lists/*

COPY services/intake-paddleocr/server.py /app/server.py

ENV INVOICEKIT_PADDLEOCR_LANG=ch
ENV INVOICEKIT_PADDLEOCR_USE_GPU=false
ENV PORT=7001
EXPOSE 7001

HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:${PORT}/health || exit 1

ENTRYPOINT ["uvicorn", "server:app", "--host", "0.0.0.0", "--port", "7001"]
