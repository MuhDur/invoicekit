# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

from __future__ import annotations

import json
import pathlib

import invoicekit
import pytest


def repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[3]


def golden_fixture() -> dict[str, str]:
    fixture = repo_root() / "conformance-corpus/golden/engine-abi-v1-commercial-document.json"
    parsed = parse_json(fixture.read_text(encoding="utf-8"))
    assert isinstance(parsed["request_bytes"], str)
    assert isinstance(parsed["expected_response_bytes"], str)
    return {
        "request_bytes": parsed["request_bytes"],
        "expected_response_bytes": parsed["expected_response_bytes"],
    }


def parse_json(raw: str) -> dict[str, object]:
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        pytest.fail(f"expected valid canonical JSON: {exc}")  # pragma: no cover
    assert isinstance(parsed, dict)
    return parsed


def test_engine_abi_version() -> None:
    assert invoicekit.ENGINE_ABI_VERSION == 1
    assert invoicekit.engine_abi_version() == 1


def test_process_engine_abi_json_returns_golden_bytes() -> None:
    fixture = golden_fixture()

    actual = invoicekit.process_engine_abi_json(
        fixture["request_bytes"].encode("utf-8")
    )

    assert actual == fixture["expected_response_bytes"].encode("utf-8")


def test_result_handle_mirrors_c_abi_accessors() -> None:
    fixture = golden_fixture()

    result = invoicekit.engine_process_json(fixture["request_bytes"].encode("utf-8"))

    assert isinstance(result, invoicekit.EngineResult)
    assert invoicekit.engine_result_status(result) == 0
    assert invoicekit.engine_result_len(result) == len(
        fixture["expected_response_bytes"].encode("utf-8")
    )
    assert invoicekit.engine_result_bytes(result) == fixture[
        "expected_response_bytes"
    ].encode("utf-8")


def test_result_free_marks_handle_invalid() -> None:
    fixture = golden_fixture()
    result = invoicekit.engine_process_json(fixture["request_bytes"].encode("utf-8"))

    invoicekit.engine_result_free(result)

    assert invoicekit.engine_result_status(result) == 2
    assert invoicekit.engine_result_len(None) == 0
    assert invoicekit.engine_result_bytes(None) == b""


def test_null_result_accessors_match_c_abi_invalid_handle() -> None:
    invoicekit.engine_result_free(None)

    assert invoicekit.engine_result_status(None) == 2
    assert invoicekit.engine_result_len(None) == 0
    assert invoicekit.engine_result_bytes(None) == b""


def test_freed_result_rejects_byte_and_len_access() -> None:
    fixture = golden_fixture()
    result = invoicekit.engine_process_json(fixture["request_bytes"].encode("utf-8"))

    result.free()

    assert result.status == 2
    with pytest.raises(RuntimeError, match="freed"):
        _ = result.bytes
    with pytest.raises(RuntimeError, match="freed"):
        _ = result.len
    with pytest.raises(RuntimeError, match="freed"):
        len(result)


def test_invalid_request_returns_canonical_error_response() -> None:
    response = invoicekit.engine_process_json(b"not json")

    assert invoicekit.engine_result_status(response) == 1
    payload = parse_json(invoicekit.engine_result_bytes(response).decode("utf-8"))
    error = payload["error"]
    assert isinstance(error, dict)
    assert payload["abi_version"] == 1
    assert payload["status"] == "error"
    assert error["code"] == "invalid_request_json"
