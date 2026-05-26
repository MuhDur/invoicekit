# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Python ctypes golden test for the InvoiceKit Engine C ABI."""

from __future__ import annotations

import ctypes
import json
import os
import pathlib
import sys


def repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[3]


def shared_library_path() -> pathlib.Path:
    override = os.environ.get("INVOICEKIT_FFI_LIB")
    if override:
        return pathlib.Path(override)

    root = repo_root()
    candidates = [
        pathlib.Path("/tmp/cargo-target/debug/libinvoicekit_ffi.so"),
        root / "target/debug/libinvoicekit_ffi.so",
        root / "target/debug/libinvoicekit_ffi.dylib",
        root / "target/debug/invoicekit_ffi.dll",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise RuntimeError("set INVOICEKIT_FFI_LIB to the built invoicekit-ffi shared library")


def golden_fixture() -> dict[str, str]:
    fixture = repo_root() / "conformance-corpus/golden/engine-abi-v1-commercial-document.json"
    return json.loads(fixture.read_text(encoding="utf-8"))


def main() -> int:
    lib = ctypes.CDLL(str(shared_library_path()))
    lib.invoicekit_engine_process_json.argtypes = [
        ctypes.POINTER(ctypes.c_ubyte),
        ctypes.c_size_t,
    ]
    lib.invoicekit_engine_process_json.restype = ctypes.c_void_p
    lib.invoicekit_engine_result_status.argtypes = [ctypes.c_void_p]
    lib.invoicekit_engine_result_status.restype = ctypes.c_uint32
    lib.invoicekit_engine_result_bytes.argtypes = [ctypes.c_void_p]
    lib.invoicekit_engine_result_bytes.restype = ctypes.POINTER(ctypes.c_ubyte)
    lib.invoicekit_engine_result_len.argtypes = [ctypes.c_void_p]
    lib.invoicekit_engine_result_len.restype = ctypes.c_size_t
    lib.invoicekit_engine_result_free.argtypes = [ctypes.c_void_p]
    lib.invoicekit_engine_result_free.restype = None

    fixture = golden_fixture()
    request = fixture["request_bytes"].encode("utf-8")
    expected = fixture["expected_response_bytes"].encode("utf-8")
    request_buffer = (ctypes.c_ubyte * len(request)).from_buffer_copy(request)

    result = lib.invoicekit_engine_process_json(request_buffer, len(request))
    if not result:
        raise AssertionError("invoicekit_engine_process_json returned null")
    try:
        status = lib.invoicekit_engine_result_status(result)
        if status != 0:
            raise AssertionError(f"expected status 0, got {status}")
        response_len = lib.invoicekit_engine_result_len(result)
        response_ptr = lib.invoicekit_engine_result_bytes(result)
        actual = ctypes.string_at(response_ptr, response_len)
        if actual != expected:
            raise AssertionError("Python ctypes ABI response did not match golden bytes")
    finally:
        lib.invoicekit_engine_result_free(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
