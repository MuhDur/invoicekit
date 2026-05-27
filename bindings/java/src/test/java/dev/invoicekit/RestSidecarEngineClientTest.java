// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

final class RestSidecarEngineClientTest {
    @Test
    void nativeOrSidecarFallsBackToRestWhenNativeIsUnavailable() throws Exception {
        byte[] response = "{\"status\":\"ok\"}".getBytes(StandardCharsets.UTF_8);
        AtomicReference<byte[]> capturedRequest = new AtomicReference<>();
        try (TestSidecar sidecar = TestSidecar.responding(response, EngineResult.STATUS_OK, capturedRequest)) {
            EngineClient client = InvoiceKit.nativeOrSidecar(sidecar.endpoint(), NativeLibraryConfig.disabledForTests());

            EngineResult result = client.process("{\"abi_version\":1,\"operation\":\"unknown\",\"payload\":{}}");

            assertEquals(EngineResult.STATUS_OK, result.statusCode());
            assertArrayEquals(response, result.responseBytes());
            assertEquals("{\"abi_version\":1,\"operation\":\"unknown\",\"payload\":{}}",
                    new String(capturedRequest.get(), StandardCharsets.UTF_8));
        }
    }

    @Test
    void restSidecarPreservesCanonicalErrorStatusHeader() throws Exception {
        byte[] response = "{\"status\":\"error\"}".getBytes(StandardCharsets.UTF_8);
        try (TestSidecar sidecar = TestSidecar.responding(response, EngineResult.STATUS_ERROR, new AtomicReference<>())) {
            EngineResult result = InvoiceKit.restSidecar(sidecar.endpoint()).process(new byte[0]);

            assertEquals(EngineResult.STATUS_ERROR, result.statusCode());
            assertEquals("{\"status\":\"error\"}", result.responseText());
        }
    }

    @Test
    void restSidecarTurnsHttpErrorsIntoTypedException() throws Exception {
        try (TestSidecar sidecar = TestSidecar.withHttpStatus(503)) {
            InvoiceKitException error = assertThrows(
                    InvoiceKitException.class,
                    () -> InvoiceKit.restSidecar(sidecar.endpoint()).process(new byte[0]));

            assertEquals("sidecar_http_error", error.code());
        }
    }

    private static final class TestSidecar implements AutoCloseable {
        private final HttpServer server;

        private TestSidecar(HttpServer server) {
            this.server = server;
        }

        static TestSidecar responding(byte[] response, int engineStatus, AtomicReference<byte[]> capturedRequest)
                throws IOException {
            HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
            server.createContext("/engine/process", exchange -> {
                capturedRequest.set(exchange.getRequestBody().readAllBytes());
                exchange.getResponseHeaders().add(RestSidecarEngineClient.STATUS_HEADER, Integer.toString(engineStatus));
                exchange.sendResponseHeaders(200, response.length);
                exchange.getResponseBody().write(response);
                exchange.close();
            });
            server.start();
            return new TestSidecar(server);
        }

        static TestSidecar withHttpStatus(int statusCode) throws IOException {
            HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
            server.createContext("/engine/process", exchange -> {
                byte[] response = "{}".getBytes(StandardCharsets.UTF_8);
                exchange.sendResponseHeaders(statusCode, response.length);
                exchange.getResponseBody().write(response);
                exchange.close();
            });
            server.start();
            return new TestSidecar(server);
        }

        URI endpoint() {
            return URI.create("http://127.0.0.1:" + server.getAddress().getPort() + "/engine/process");
        }

        @Override
        public void close() {
            server.stop(0);
        }
    }
}
