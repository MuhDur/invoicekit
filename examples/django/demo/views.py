# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 demo views — read-side index + healthz, plus a POST
# /canonicalize/<name> that runs the named XRechnung fixture
# through the InvoiceKit Rust engine via the ctypes bridge.

from __future__ import annotations

from django.http import HttpRequest, HttpResponse, JsonResponse
from django.views.decorators.csrf import csrf_exempt
from django.views.decorators.http import require_GET, require_POST

from fixtures import FIXTURES
from invoicekit_bridge import canonicalize


@require_GET
def index(_request: HttpRequest) -> JsonResponse:
    return JsonResponse(
        {
            "title": "InvoiceKit Django demo",
            "fixtures": sorted(FIXTURES.keys()),
            "usage": "POST /canonicalize/{fixture_name}",
        }
    )


@require_GET
def healthz(_request: HttpRequest) -> JsonResponse:
    return JsonResponse({"status": "ok"})


@csrf_exempt
@require_POST
def canonicalize_endpoint(_request: HttpRequest, fixture_name: str) -> HttpResponse:
    document = FIXTURES.get(fixture_name)
    if document is None:
        return JsonResponse(
            {
                "error": {
                    "code": "UNKNOWN_FIXTURE",
                    "available": sorted(FIXTURES.keys()),
                }
            },
            status=404,
        )
    return JsonResponse(canonicalize(document))
