# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 reference Django demo: pure-ctypes bridge to libinvoicekit_ffi.
#
# Mirrors the FastAPI demo's bridge so the only difference between
# the two reference apps is the web framework, not the engine
# integration. CI builds the cdylib with `cargo build -p
# invoicekit-ffi`, exports INVOICEKIT_FFI_LIB, and runs Django's
# test suite.

from __future__ import annotations

import ctypes
import json
import os
import pathlib
from typing import Any


_DEFAULT_CANDIDATE_PATHS = (
    "/tmp/cargo-target/debug/libinvoicekit_ffi.so",
    "target/debug/libinvoicekit_ffi.so",
    "target/debug/libinvoicekit_ffi.dylib",
    "target/debug/invoicekit_ffi.dll",
)


def _shared_library_path() -> pathlib.Path:
    override = os.environ.get("INVOICEKIT_FFI_LIB")
    if override:
        return pathlib.Path(override)

    repo_root = pathlib.Path(__file__).resolve().parents[2]
    for relative in _DEFAULT_CANDIDATE_PATHS:
        candidate = pathlib.Path(relative)
        if not candidate.is_absolute():
            candidate = repo_root / candidate
        if candidate.exists():
            return candidate
    raise RuntimeError(
        "invoicekit_bridge: could not find libinvoicekit_ffi shared library. "
        "Set INVOICEKIT_FFI_LIB to the absolute path, or build the cdylib "
        "with `cargo build -p invoicekit-ffi`."
    )


class _EngineLibrary:
    """ctypes wrapper around libinvoicekit_ffi for the Engine ABI."""

    def __init__(self) -> None:
        lib = ctypes.CDLL(str(_shared_library_path()))
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
        self._lib = lib

    def process_engine_abi_json(self, request_bytes: bytes) -> tuple[int, bytes]:
        buffer = (ctypes.c_ubyte * len(request_bytes)).from_buffer_copy(request_bytes)
        result = self._lib.invoicekit_engine_process_json(buffer, len(request_bytes))
        if not result:
            raise RuntimeError("invoicekit_engine_process_json returned null")
        try:
            status = self._lib.invoicekit_engine_result_status(result)
            length = self._lib.invoicekit_engine_result_len(result)
            ptr = self._lib.invoicekit_engine_result_bytes(result)
            if not ptr:
                raise RuntimeError("invoicekit_engine_result_bytes returned null")
            response = bytes(ctypes.cast(ptr, ctypes.POINTER(ctypes.c_ubyte * length))[0])
            return int(status), response
        finally:
            self._lib.invoicekit_engine_result_free(result)


_ENGINE: _EngineLibrary | None = None


def _engine() -> _EngineLibrary:
    global _ENGINE
    if _ENGINE is None:
        _ENGINE = _EngineLibrary()
    return _ENGINE


def canonicalize(document: dict[str, Any]) -> dict[str, Any]:
    """Run the canonicalize Engine ABI op against a CommercialDocument dict."""
    request = json.dumps(
        {"abi_version": 1, "operation": "commercial_document.canonicalize", "payload": document},
        separators=(",", ":"),
    ).encode("utf-8")
    status, response_bytes = _engine().process_engine_abi_json(request)
    parsed: dict[str, Any] = json.loads(response_bytes.decode("utf-8"))
    parsed["_engine_status"] = status
    return parsed
