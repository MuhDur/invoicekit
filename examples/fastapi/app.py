# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1406 reference FastAPI demo: a single-endpoint app that
# canonicalises three German XRechnung fixtures through the
# InvoiceKit Rust engine (loaded via ctypes from libinvoicekit_ffi).

from __future__ import annotations

from fastapi import FastAPI, HTTPException

from fixtures import FIXTURES
from invoicekit_bridge import canonicalize

app = FastAPI(
    title="InvoiceKit FastAPI Demo",
    description="Canonicalise three German XRechnung fixtures in under 5 minutes.",
    version="0.0.0",
)


@app.get("/")
def index() -> dict[str, object]:
    return {
        "title": "InvoiceKit FastAPI demo",
        "fixtures": sorted(FIXTURES.keys()),
        "usage": "POST /canonicalize/{fixture_name}",
    }


@app.get("/healthz")
def healthz() -> dict[str, str]:
    return {"status": "ok"}


@app.post("/canonicalize/{fixture_name}")
def canonicalize_endpoint(fixture_name: str) -> dict[str, object]:
    document = FIXTURES.get(fixture_name)
    if document is None:
        raise HTTPException(
            status_code=404,
            detail={
                "code": "UNKNOWN_FIXTURE",
                "available": sorted(FIXTURES.keys()),
            },
        )
    return canonicalize(document)
