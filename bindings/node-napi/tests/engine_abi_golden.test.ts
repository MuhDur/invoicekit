// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import { describe, expect, test } from "bun:test";
import { dlopen, FFIType, ptr, suffix, toArrayBuffer } from "bun:ffi";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

type GoldenFixture = {
  request_bytes: string;
  expected_response_bytes: string;
};

function repoRoot(): string {
  return resolve(import.meta.dir, "..", "..", "..");
}

function sharedLibraryPath(): string {
  if (process.env.INVOICEKIT_FFI_LIB) {
    return process.env.INVOICEKIT_FFI_LIB;
  }
  const root = repoRoot();
  const candidates = [
    `/tmp/cargo-target/debug/libinvoicekit_ffi.${suffix}`,
    join(root, "target", "debug", `libinvoicekit_ffi.${suffix}`),
    join(root, "target", "debug", `invoicekit_ffi.${suffix}`),
  ];
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error("set INVOICEKIT_FFI_LIB to the built invoicekit-ffi shared library");
  }
  return found;
}

function goldenFixture(): GoldenFixture {
  const fixturePath = join(
    repoRoot(),
    "conformance-corpus",
    "golden",
    "engine-abi-v1-commercial-document.json",
  );
  try {
    return JSON.parse(readFileSync(fixturePath, "utf8")) as GoldenFixture;
  } catch (error) {
    throw new Error(`failed to parse golden fixture at ${fixturePath}`, { cause: error });
  }
}

describe("Engine ABI golden fixture", () => {
  test("Bun FFI matches the frozen response bytes", () => {
    const fixture = goldenFixture();
    const request = new TextEncoder().encode(fixture.request_bytes);
    const library = dlopen(sharedLibraryPath(), {
      invoicekit_engine_process_json: {
        args: [FFIType.ptr, FFIType.usize],
        returns: FFIType.ptr,
      },
      invoicekit_engine_result_status: {
        args: [FFIType.ptr],
        returns: FFIType.u32,
      },
      invoicekit_engine_result_bytes: {
        args: [FFIType.ptr],
        returns: FFIType.ptr,
      },
      invoicekit_engine_result_len: {
        args: [FFIType.ptr],
        returns: FFIType.usize,
      },
      invoicekit_engine_result_free: {
        args: [FFIType.ptr],
        returns: FFIType.void,
      },
    });

    const result = library.symbols.invoicekit_engine_process_json(ptr(request), request.byteLength);
    expect(result).not.toBe(0);
    try {
      expect(library.symbols.invoicekit_engine_result_status(result)).toBe(0);
      const responseLength = Number(library.symbols.invoicekit_engine_result_len(result));
      const responsePtr = library.symbols.invoicekit_engine_result_bytes(result);
      expect(responsePtr).not.toBe(0);
      const actual = Buffer.from(toArrayBuffer(responsePtr, 0, responseLength)).toString("utf8");
      expect(actual).toBe(fixture.expected_response_bytes);
    } finally {
      library.symbols.invoicekit_engine_result_free(result);
      library.close();
    }
  });
});
