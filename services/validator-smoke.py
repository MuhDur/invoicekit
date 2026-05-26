#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass


GOOD_XML = "<Invoice><ID>INV-001</ID><Amount currency=\"EUR\">1.00</Amount></Invoice>"
BAD_XML = "<Invoice><ID>INV-001</Invoice>"


@dataclass(frozen=True)
class ImageTarget:
    backend: str
    image: str


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke-test InvoiceKit validator sidecar images.")
    parser.add_argument(
        "--image",
        action="append",
        default=[],
        help="Backend/image pair, e.g. jvm:kosit=invoicekit/validator-kosit:ci",
    )
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--latency-threshold-ms", type=float, default=200.0)
    args = parser.parse_args()

    targets = parse_targets(args.image)
    for target in targets:
        smoke_target(target, args.warmup, args.iterations, args.latency_threshold_ms)
    return 0


def parse_targets(values: list[str]) -> list[ImageTarget]:
    if not values:
        values = [
            "jvm:kosit=invoicekit/validator-kosit:ci",
            "jvm:phive=invoicekit/validator-phive:ci",
            "jvm:saxon=invoicekit/validator-saxon:ci",
        ]
    targets: list[ImageTarget] = []
    for value in values:
        if "=" not in value:
            raise SystemExit(f"--image must be backend=image, got {value!r}")
        backend, image = value.split("=", 1)
        if backend not in {"jvm:kosit", "jvm:phive", "jvm:saxon"}:
            raise SystemExit(f"unsupported backend {backend!r}")
        targets.append(ImageTarget(backend=backend, image=image))
    return targets


def smoke_target(target: ImageTarget, warmup: int, iterations: int, threshold_ms: float) -> None:
    container_id = start_container(target.image)
    try:
        base_url = wait_for_health(container_id)
        good = rpc_validate(base_url, target.backend, GOOD_XML, "smoke-good")
        assert_result(good, target.backend, expected_valid=True)
        bad = rpc_validate(base_url, target.backend, BAD_XML, "smoke-bad")
        assert_result(bad, target.backend, expected_valid=False)

        one_mb_xml = "<Invoice><Payload>" + ("A" * (1024 * 1024)) + "</Payload></Invoice>"
        for index in range(warmup):
            assert_result(
                rpc_validate(base_url, target.backend, one_mb_xml, f"warmup-{index}"),
                target.backend,
                expected_valid=True,
            )

        latencies: list[float] = []
        for index in range(iterations):
            started = time.perf_counter()
            assert_result(
                rpc_validate(base_url, target.backend, one_mb_xml, f"latency-{index}"),
                target.backend,
                expected_valid=True,
            )
            latencies.append((time.perf_counter() - started) * 1000.0)

        p95 = percentile(latencies, 95)
        if p95 > threshold_ms:
            raise AssertionError(
                f"{target.backend} p95 latency {p95:.2f}ms exceeds {threshold_ms:.2f}ms"
            )
        print(
            json.dumps(
                {
                    "backend": target.backend,
                    "image": target.image,
                    "p95_ms": round(p95, 2),
                    "iterations": iterations,
                },
                sort_keys=True,
            )
        )
    finally:
        subprocess.run(["docker", "stop", container_id], check=False, stdout=subprocess.DEVNULL)


def start_container(image: str) -> str:
    result = subprocess.run(
        ["docker", "run", "-d", "-p", "127.0.0.1::8080", image],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    return result.stdout.strip()


def wait_for_health(container_id: str) -> str:
    port = ""
    deadline = time.monotonic() + 45.0
    while time.monotonic() < deadline:
        port_result = subprocess.run(
            ["docker", "port", container_id, "8080/tcp"],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        if port_result.returncode == 0 and port_result.stdout.strip():
            port = port_result.stdout.strip().rsplit(":", 1)[-1]
            url = f"http://127.0.0.1:{port}"
            try:
                with urllib.request.urlopen(f"{url}/healthz", timeout=2.0) as response:
                    if response.status == 200:
                        return url
            except (OSError, urllib.error.URLError, TimeoutError):
                pass
        time.sleep(0.5)
    logs = subprocess.run(
        ["docker", "logs", container_id],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    ).stdout[-4000:]
    raise TimeoutError(f"container {container_id} did not become healthy on port {port}: {logs}")


def rpc_validate(base_url: str, backend: str, xml: str, request_id: str) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "validator.validate",
        "params": {
            "backend": backend,
            "profile": "contract-smoke",
            "trace_id": f"trace-{request_id}",
            "rule_pack": {
                "id": "validator-sidecar-contract",
                "version": "2026.05",
                "effective_date": "2026-05-26",
            },
            "document": {
                "content_type": "application/xml",
                "encoding": "utf-8",
                "xml": xml,
            },
        },
    }
    data = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url}/rpc",
        data=data,
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=10.0) as response:
        body = response.read().decode("utf-8")
    try:
        return json.loads(body)
    except json.JSONDecodeError as ex:
        raise AssertionError(f"invalid JSON-RPC response from {backend}: {body[:500]!r}") from ex


def assert_result(response: dict, backend: str, expected_valid: bool) -> None:
    if "error" in response:
        raise AssertionError(f"JSON-RPC error from {backend}: {response['error']}")
    result = response["result"]
    if result["backend"] != backend:
        raise AssertionError(f"expected backend {backend}, got {result['backend']}")
    if result["valid"] is not expected_valid:
        raise AssertionError(f"expected valid={expected_valid} from {backend}, got {result['valid']}")
    if expected_valid:
        if result["results"] != []:
            raise AssertionError(f"expected no findings from {backend}, got {result['results']}")
    else:
        finding = result["results"][0]
        if finding["severity"] != "fatal":
            raise AssertionError(f"expected fatal finding from {backend}, got {finding}")
        if finding["location"]["kind"] != "x_path":
            raise AssertionError(f"expected XPath location from {backend}, got {finding}")
        if finding["trace"]["backend"] != backend:
            raise AssertionError(f"expected trace backend {backend}, got {finding['trace']}")


def percentile(values: list[float], pct: int) -> float:
    if not values:
        raise AssertionError("cannot compute percentile of empty latency list")
    ordered = sorted(values)
    index = max(0, math.ceil((pct / 100.0) * len(ordered)) - 1)
    return ordered[index]


if __name__ == "__main__":
    sys.exit(main())
