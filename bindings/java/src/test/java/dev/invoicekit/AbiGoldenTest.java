// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

final class AbiGoldenTest {
    @Test
    void nativeClientMatchesEngineAbiGoldenFixtureWhenFfmIsAvailable() throws Exception {
        assumeTrue(Runtime.version().feature() >= 22, "FFM provider requires Java 22 or newer");
        assumeTrue(System.getenv("INVOICEKIT_FFI_LIB") != null, "native golden requires INVOICEKIT_FFI_LIB");

        EngineClient client = InvoiceKit.nativeClient();
        GoldenFixture fixture = GoldenFixture.read(repoRoot()
                .resolve("conformance-corpus/golden/engine-abi-v1-commercial-document.json"));

        assertEquals(InvoiceKit.ENGINE_ABI_VERSION, client.abiVersion());
        EngineResult result = client.process(fixture.requestBytes());

        assertEquals(EngineResult.STATUS_OK, result.statusCode());
        assertEquals(fixture.expectedResponseBytes(), result.responseText());
    }

    @Test
    void nativeClientReportsUnavailableWhenNativeLoadingIsDisabled() {
        InvoiceKitException error = assertThrows(
                InvoiceKitException.class,
                () -> InvoiceKit.nativeClient(NativeLibraryConfig.disabledForTests()));

        assertEquals("native_unavailable", error.code());
    }

    private static Path repoRoot() {
        Path current = Path.of("").toAbsolutePath().normalize();
        while (current != null) {
            if (Files.exists(current.resolve("Cargo.toml"))
                    && Files.isDirectory(current.resolve("conformance-corpus"))) {
                return current;
            }
            current = current.getParent();
        }
        throw new IllegalStateException("could not locate repository root");
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
