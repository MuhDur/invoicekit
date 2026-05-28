# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""Pure-Python tests for the InvoiceKit sidecar client.

These don't require an Odoo runtime — they exercise the sidecar
HTTP client directly so the load-bearing transport + receipt
parsing stays covered without spinning up an Odoo container.
"""
from __future__ import annotations

import json
import unittest
import urllib.error
import urllib.request

from addons.invoicekit_einvoice.models.sidecar_client import (
    InvoiceKitSidecar,
    InvoiceKitSidecarError,
    ResponseLike,
)


class _RecordingOpener:
    """Test double for urllib's opener function."""

    def __init__(self, responses: list[ResponseLike | Exception]) -> None:
        self.responses = responses
        self.calls: list[urllib.request.Request] = []

    def __call__(
        self,
        request: urllib.request.Request,
        timeout: float | None,
    ) -> ResponseLike:
        self.calls.append(request)
        item = self.responses.pop(0)
        if isinstance(item, Exception):
            raise item
        return item


class SidecarClientTests(unittest.TestCase):

    def test_transmit_posts_json_and_parses_receipt(self) -> None:
        body = json.dumps(
            {
                "submission_id": "sub-42",
                "state": "accepted",
                "evidence_bundle_url": "https://archive/example/sub-42",
            }
        ).encode("utf-8")
        opener = _RecordingOpener([ResponseLike(200, body)])
        client = InvoiceKitSidecar("http://sidecar:8088/", opener=opener)
        receipt = client.transmit({"tenant_id": "t-1", "document": {}})
        self.assertEqual(receipt.submission_id, "sub-42")
        self.assertEqual(receipt.state, "accepted")
        self.assertEqual(
            receipt.evidence_bundle_url, "https://archive/example/sub-42"
        )
        self.assertEqual(len(opener.calls), 1)
        sent = opener.calls[0]
        self.assertEqual(sent.full_url, "http://sidecar:8088/v1/transmit")
        self.assertEqual(sent.get_method(), "POST")
        self.assertEqual(json.loads(sent.data), {"tenant_id": "t-1", "document": {}})
        self.assertEqual(sent.get_header("Content-type"), "application/json")

    def test_transmit_with_api_key_sets_bearer_header(self) -> None:
        body = json.dumps({"submission_id": "s", "state": "queued"}).encode("utf-8")
        opener = _RecordingOpener([ResponseLike(202, body)])
        client = InvoiceKitSidecar(
            "http://sidecar/", api_key="topsecret", opener=opener
        )
        client.transmit({"document": {}})
        self.assertEqual(
            opener.calls[0].get_header("Authorization"), "Bearer topsecret"
        )

    def test_transmit_handles_missing_evidence_bundle_url(self) -> None:
        body = json.dumps({"submission_id": "s", "state": "queued"}).encode("utf-8")
        opener = _RecordingOpener([ResponseLike(200, body)])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        receipt = client.transmit({"document": {}})
        self.assertIsNone(receipt.evidence_bundle_url)

    def test_transmit_raises_when_response_misses_fields(self) -> None:
        body = json.dumps({"state": "queued"}).encode("utf-8")
        opener = _RecordingOpener([ResponseLike(200, body)])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        with self.assertRaisesRegex(InvoiceKitSidecarError, "submission_id"):
            client.transmit({})

    def test_transmit_raises_on_non_2xx_status(self) -> None:
        opener = _RecordingOpener([ResponseLike(500, b'{"error":"boom"}')])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        with self.assertRaisesRegex(InvoiceKitSidecarError, "HTTP 500"):
            client.transmit({})

    def test_transmit_raises_on_malformed_json(self) -> None:
        opener = _RecordingOpener([ResponseLike(200, b"not json")])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        with self.assertRaisesRegex(InvoiceKitSidecarError, "not JSON"):
            client.transmit({})

    def test_transmit_wraps_http_error(self) -> None:
        err = urllib.error.HTTPError(
            url="http://sidecar/v1/transmit",
            code=401,
            msg="Unauthorized",
            hdrs=None,  # type: ignore[arg-type]
            fp=None,
        )
        opener = _RecordingOpener([err])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        with self.assertRaisesRegex(InvoiceKitSidecarError, "HTTP 401"):
            client.transmit({})

    def test_transmit_wraps_url_error(self) -> None:
        opener = _RecordingOpener([urllib.error.URLError("nodns")])
        client = InvoiceKitSidecar("http://sidecar/", opener=opener)
        with self.assertRaisesRegex(InvoiceKitSidecarError, "unreachable"):
            client.transmit({})


if __name__ == "__main__":
    unittest.main()
