// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.file.Path;
import java.util.Objects;

final class FfmEngineClient implements EngineClient {
    private final MethodHandle abiVersion;
    private final MethodHandle process;
    private final MethodHandle status;
    private final MethodHandle bytes;
    private final MethodHandle length;
    private final MethodHandle free;

    private FfmEngineClient(
            MethodHandle abiVersion,
            MethodHandle process,
            MethodHandle status,
            MethodHandle bytes,
            MethodHandle length,
            MethodHandle free) {
        this.abiVersion = abiVersion;
        this.process = process;
        this.status = status;
        this.bytes = bytes;
        this.length = length;
        this.free = free;
    }

    static EngineClient create(NativeLibraryConfig config) throws InvoiceKitException {
        try {
            Linker linker = Linker.nativeLinker();
            SymbolLookup lookup = lookup(config);
            return new FfmEngineClient(
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_abi_version"),
                            FunctionDescriptor.of(ValueLayout.JAVA_INT)),
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_process_json"),
                            FunctionDescriptor.of(
                                    ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG)),
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_result_status"),
                            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS)),
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_result_bytes"),
                            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)),
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_result_len"),
                            FunctionDescriptor.of(ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)),
                    linker.downcallHandle(
                            symbol(lookup, "invoicekit_engine_result_free"),
                            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)));
        } catch (IllegalArgumentException | IllegalStateException error) {
            throw nativeFailure("could not bind invoicekit-ffi symbols", error);
        }
    }

    @Override
    public int abiVersion() throws InvoiceKitException {
        try {
            return (int) abiVersion.invokeExact();
        } catch (Throwable error) {
            throw nativeFailure("invoicekit_engine_abi_version failed", error);
        }
    }

    @Override
    public EngineResult process(byte[] requestBytes) throws InvoiceKitException {
        Objects.requireNonNull(requestBytes, "requestBytes");
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment requestSegment = requestBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(requestBytes.length);
            if (requestBytes.length > 0) {
                MemorySegment.copy(MemorySegment.ofArray(requestBytes), 0, requestSegment, 0, requestBytes.length);
            }

            MemorySegment result = (MemorySegment) process.invokeExact(requestSegment, (long) requestBytes.length);
            if (result.equals(MemorySegment.NULL)) {
                throw new InvoiceKitException(
                        "native_null_result",
                        "invoicekit_engine_process_json returned null",
                        "Retry with the same request and report this deterministic native binding defect.");
            }
            try {
                int statusCode = (int) status.invokeExact(result);
                long responseLength = (long) length.invokeExact(result);
                if (responseLength < 0 || responseLength > Integer.MAX_VALUE) {
                    throw new InvoiceKitException(
                            "native_response_too_large",
                            "invoicekit-ffi response length is not representable in Java: " + responseLength,
                            "Retry with a smaller request or use the REST sidecar streaming endpoint.");
                }
                MemorySegment responseAddress = (MemorySegment) bytes.invokeExact(result);
                if (responseLength > 0 && responseAddress.equals(MemorySegment.NULL)) {
                    throw new InvoiceKitException(
                            "native_null_response_bytes",
                            "invoicekit_engine_result_bytes returned null for a non-empty response",
                            "Retry with the same request and report this deterministic native binding defect.");
                }
                byte[] responseBytes = responseLength == 0
                        ? new byte[0]
                        : responseAddress.reinterpret(responseLength).toArray(ValueLayout.JAVA_BYTE);
                return new EngineResult(statusCode, responseBytes);
            } finally {
                free.invokeExact(result);
            }
        } catch (InvoiceKitException error) {
            throw error;
        } catch (Throwable error) {
            throw nativeFailure("invoicekit_engine_process_json failed", error);
        }
    }

    private static SymbolLookup lookup(NativeLibraryConfig config) {
        return config.libraryPath()
                .map(FfmEngineClient::libraryLookup)
                .orElseGet(() -> SymbolLookup.libraryLookup(config.libraryName(), Arena.global()));
    }

    private static SymbolLookup libraryLookup(Path path) {
        return SymbolLookup.libraryLookup(path, Arena.global());
    }

    private static MemorySegment symbol(SymbolLookup lookup, String name) throws InvoiceKitException {
        return lookup.find(name).orElseThrow(() -> new InvoiceKitException(
                "native_missing_symbol",
                "invoicekit-ffi symbol is missing: " + name,
                "Use a matching invoicekit-ffi shared library built from the same InvoiceKit release."));
    }

    private static InvoiceKitException nativeFailure(String detail, Throwable cause) {
        return new InvoiceKitException(
                "native_failure",
                "InvoiceKit FFM native call failed: " + detail,
                "Set INVOICEKIT_FFI_LIB to a compatible invoicekit-ffi shared library or use the REST sidecar.",
                cause);
    }
}
