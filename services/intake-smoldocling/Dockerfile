# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
# T-063: Layer 4 intake — SmolDocling-256M ONNX sidecar.

FROM python:3.12-slim AS runtime

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends poppler-utils curl \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m pip install --no-cache-dir \
        "onnxruntime==1.20.0" \
        "fastapi==0.115.0" \
        "uvicorn[standard]==0.31.0" \
        "pdf2image==1.17.0" \
        "Pillow==11.0.0" \
        "numpy==2.1.3" \
        "transformers==4.46.0"

COPY services/intake-smoldocling/server.py /app/server.py

ENV INVOICEKIT_SMOLDOCLING_MODEL_PATH=/var/lib/invoicekit/smoldocling.onnx
ENV INVOICEKIT_SMOLDOCLING_DEVICE=cpu
ENV PORT=7002

EXPOSE 7002

HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:${PORT}/health || exit 1

ENTRYPOINT ["uvicorn", "server:app", "--host", "0.0.0.0", "--port", "7002"]
