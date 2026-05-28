# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
"""Pure-Python InvoiceKit sidecar HTTP client.

Isolated from Odoo so it can be unit-tested against any
HTTP mock; the Odoo `account.move` model imports this and
calls it from the `action_send_via_invoicekit` server action.
"""
from __future__ import annotations

import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any, Callable, Mapping


# Transport surface that lets tests inject a stub instead of
# touching the real network. Mirrors `urllib.request.urlopen`'s
# return shape closely enough for the cases the client needs.
OpenerLike = Callable[[urllib.request.Request, float | None], "ResponseLike"]


class ResponseLike:
    """Minimal subset of `http.client.HTTPResponse` we depend on."""

    def __init__(self, status: int, body: bytes) -> None:
        self.status = status
        self._body = body

    def read(self) -> bytes:
        return self._body

    def __enter__(self) -> "ResponseLike":
        return self

    def __exit__(self, *exc: object) -> None:
        return None


class InvoiceKitSidecarError(Exception):
    """Raised when the sidecar returns a non-2xx response or fails."""


@dataclass(frozen=True)
class TransmitReceipt:
    """Parsed `/v1/transmit` response from the sidecar."""

    submission_id: str
    state: str
    evidence_bundle_url: str | None


class InvoiceKitSidecar:
    """Thin wrapper over the sidecar's REST API.

    The sidecar implements the same Engine ABI all language SDKs
    use, just exposed over HTTP for connectors that can't easily
    link a native library.
    """

    def __init__(
        self,
        base_url: str,
        api_key: str | None = None,
        timeout_seconds: float = 30.0,
        opener: OpenerLike | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout_seconds = timeout_seconds
        self._opener: OpenerLike = opener or _default_opener

    def transmit(self, invoice: Mapping[str, Any]) -> TransmitReceipt:
        """POST `invoice` to `<base>/v1/transmit` and parse the receipt."""
        body = self._post("/v1/transmit", invoice)
        try:
            submission_id = str(body["submission_id"])
            state = str(body["state"])
        except KeyError as exc:
            raise InvoiceKitSidecarError(
                f"sidecar response missing required field: {exc}"
            ) from exc
        evidence_url = body.get("evidence_bundle_url")
        return TransmitReceipt(
            submission_id=submission_id,
            state=state,
            evidence_bundle_url=str(evidence_url) if evidence_url else None,
        )

    def _post(self, path: str, payload: Mapping[str, Any]) -> Mapping[str, Any]:
        url = f"{self.base_url}{path}"
        data = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            url=url,
            data=data,
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Accept": "application/json",
                **({"Authorization": f"Bearer {self.api_key}"} if self.api_key else {}),
            },
        )
        try:
            with self._opener(request, self.timeout_seconds) as response:
                status = getattr(response, "status", None) or getattr(response, "code", 200)
                raw = response.read()
        except urllib.error.HTTPError as exc:
            raise InvoiceKitSidecarError(
                f"sidecar HTTP {exc.code}: {exc.reason}"
            ) from exc
        except urllib.error.URLError as exc:
            raise InvoiceKitSidecarError(
                f"sidecar unreachable at {url}: {exc.reason}"
            ) from exc
        if not 200 <= int(status) < 300:
            raise InvoiceKitSidecarError(
                f"sidecar refused: HTTP {status}: {raw[:200]!r}"
            )
        try:
            return json.loads(raw)
        except json.JSONDecodeError as exc:
            raise InvoiceKitSidecarError(
                f"sidecar response was not JSON: {exc}"
            ) from exc


def _default_opener(request: urllib.request.Request, timeout: float | None) -> ResponseLike:
    response = urllib.request.urlopen(request, timeout=timeout)
    body = response.read()
    wrapped = ResponseLike(getattr(response, "status", 200), body)
    response.close()
    return wrapped
