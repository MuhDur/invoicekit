// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import java.io.IOException;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodType;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import jdk.incubator.foreign.CLinker;
import jdk.incubator.foreign.FunctionDescriptor;
import jdk.incubator.foreign.MemoryAddress;
import jdk.incubator.foreign.MemorySegment;
import jdk.incubator.foreign.ResourceScope;
import jdk.incubator.foreign.SymbolLookup;

public final class AbiGoldenTest {
    private AbiGoldenTest() {}

    public static void main(String[] args) throws Throwable {
        String libraryPath = System.getenv("INVOICEKIT_FFI_LIB");
        if (libraryPath == null || libraryPath.isBlank()) {
            throw new IllegalStateException("set INVOICEKIT_FFI_LIB to the built invoicekit-ffi shared library");
        }
        System.load(libraryPath);

        Path root = Path.of("").toAbsolutePath().normalize();
        GoldenFixture fixture = GoldenFixture.read(
                root.resolve("conformance-corpus/golden/engine-abi-v1-commercial-document.json"));

        CLinker linker = CLinker.getInstance();
        SymbolLookup lookup = SymbolLookup.loaderLookup();
        MethodHandle process = linker.downcallHandle(
                lookup.lookup("invoicekit_engine_process_json").orElseThrow(),
                MethodType.methodType(MemoryAddress.class, MemoryAddress.class, long.class),
                FunctionDescriptor.of(CLinker.C_POINTER, CLinker.C_POINTER, CLinker.C_LONG));
        MethodHandle status = linker.downcallHandle(
                lookup.lookup("invoicekit_engine_result_status").orElseThrow(),
                MethodType.methodType(int.class, MemoryAddress.class),
                FunctionDescriptor.of(CLinker.C_INT, CLinker.C_POINTER));
        MethodHandle bytes = linker.downcallHandle(
                lookup.lookup("invoicekit_engine_result_bytes").orElseThrow(),
                MethodType.methodType(MemoryAddress.class, MemoryAddress.class),
                FunctionDescriptor.of(CLinker.C_POINTER, CLinker.C_POINTER));
        MethodHandle length = linker.downcallHandle(
                lookup.lookup("invoicekit_engine_result_len").orElseThrow(),
                MethodType.methodType(long.class, MemoryAddress.class),
                FunctionDescriptor.of(CLinker.C_LONG, CLinker.C_POINTER));
        MethodHandle free = linker.downcallHandle(
                lookup.lookup("invoicekit_engine_result_free").orElseThrow(),
                MethodType.methodType(void.class, MemoryAddress.class),
                FunctionDescriptor.ofVoid(CLinker.C_POINTER));

        byte[] request = fixture.requestBytes.getBytes(StandardCharsets.UTF_8);
        try (ResourceScope scope = ResourceScope.newConfinedScope()) {
            MemorySegment requestSegment = MemorySegment.allocateNative(request.length, scope);
            requestSegment.asByteBuffer().put(request);
            MemoryAddress result = (MemoryAddress) process.invokeExact(requestSegment.address(), (long) request.length);
            if (result.equals(MemoryAddress.NULL)) {
                throw new AssertionError("invoicekit_engine_process_json returned null");
            }
            try {
                int statusCode = (int) status.invokeExact(result);
                if (statusCode != 0) {
                    throw new AssertionError("expected status 0, got " + statusCode);
                }
                long responseLength = (long) length.invokeExact(result);
                if (responseLength > Integer.MAX_VALUE) {
                    throw new AssertionError("response too large for Java test: " + responseLength);
                }
                MemoryAddress responseAddress = (MemoryAddress) bytes.invokeExact(result);
                byte[] actual = new byte[(int) responseLength];
                responseAddress.asSegment(responseLength, scope).asByteBuffer().get(actual);
                String actualText = new String(actual, StandardCharsets.UTF_8);
                if (!actualText.equals(fixture.expectedResponseBytes)) {
                    throw new AssertionError("Java FFM ABI response did not match golden bytes");
                }
            } finally {
                free.invokeExact(result);
            }
        }
    }

    private record GoldenFixture(String requestBytes, String expectedResponseBytes) {
        static GoldenFixture read(Path path) throws IOException {
            String json = Files.readString(path, StandardCharsets.UTF_8);
            return new GoldenFixture(
                    readJsonStringField(json, "request_bytes"),
                    readJsonStringField(json, "expected_response_bytes"));
        }
    }

    private static String readJsonStringField(String json, String fieldName) {
        String needle = "\"" + fieldName + "\"";
        int nameIndex = json.indexOf(needle);
        if (nameIndex < 0) {
            throw new IllegalArgumentException("missing field " + fieldName);
        }
        int colonIndex = json.indexOf(':', nameIndex + needle.length());
        int quoteIndex = json.indexOf('"', colonIndex + 1);
        StringBuilder out = new StringBuilder();
        for (int index = quoteIndex + 1; index < json.length(); index++) {
            char ch = json.charAt(index);
            if (ch == '"') {
                return out.toString();
            }
            if (ch != '\\') {
                out.append(ch);
                continue;
            }
            if (++index >= json.length()) {
                throw new IllegalArgumentException("unterminated escape in " + fieldName);
            }
            char escaped = json.charAt(index);
            switch (escaped) {
                case '"', '\\', '/' -> out.append(escaped);
                case 'b' -> out.append('\b');
                case 'f' -> out.append('\f');
                case 'n' -> out.append('\n');
                case 'r' -> out.append('\r');
                case 't' -> out.append('\t');
                case 'u' -> {
                    String hex = json.substring(index + 1, index + 5);
                    out.append((char) Integer.parseInt(hex, 16));
                    index += 4;
                }
                default -> throw new IllegalArgumentException("bad escape in " + fieldName + ": " + escaped);
            }
        }
        throw new IllegalArgumentException("unterminated string field " + fieldName);
    }
}
