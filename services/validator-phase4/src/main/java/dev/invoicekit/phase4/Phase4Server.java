// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.phase4;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpHandler;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * JSON-RPC entry point for the phase4 sidecar.
 *
 * <p>The four methods documented in
 * {@code docs/operators/PHASE4-REFERENCE-ADAPTER.md} —
 * {@code transmit}, {@code receive}, {@code status}, and
 * {@code health} — are wired here.
 *
 * <p>The scaffold ships stubbed implementations that return the
 * minimum JSON shape the Rust {@code Phase4Adapter} expects. The
 * real transmit/receive paths require an OpenPeppol AP
 * certificate (4-8 week lead time) plus the {@code phase4-peppol-client}
 * runtime configured against the SML, both of which arrive in
 * the follow-up beads.
 */
public final class Phase4Server {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final String DEFAULT_PORT = "8090";

    private Phase4Server() {
        // utility entry point
    }

    public static void main(String[] args) throws IOException {
        int port = Integer.parseInt(System.getenv().getOrDefault("INVOICEKIT_PHASE4_PORT", DEFAULT_PORT));
        String smlMode = System.getenv().getOrDefault("PEPPOL_AP_SML_MODE", "acceptance");

        HttpServer server = HttpServer.create(new InetSocketAddress(port), 0);
        server.createContext("/", new RpcHandler(smlMode));
        server.setExecutor(null);
        server.start();
        System.out.println("phase4-server listening on :" + port + " (sml=" + smlMode + ")");
    }

    static final class RpcHandler implements HttpHandler {

        private final String smlMode;

        RpcHandler(String smlMode) {
            this.smlMode = smlMode;
        }

        @Override
        public void handle(HttpExchange exchange) throws IOException {
            try (exchange) {
                if (!"POST".equalsIgnoreCase(exchange.getRequestMethod())) {
                    writeError(exchange, 405, -32600, "POST required");
                    return;
                }
                JsonNode request = JSON.readTree(exchange.getRequestBody());
                String method = request.path("method").asText("");
                JsonNode params = request.has("params") ? request.get("params") : JSON.createObjectNode();
                ObjectNode result = dispatch(method, params);
                writeResult(exchange, request.path("id").asText("0"), result);
            } catch (RuntimeException ex) {
                writeError(exchange, 500, -32000, ex.getMessage() != null ? ex.getMessage() : ex.toString());
            }
        }

        ObjectNode dispatch(String method, JsonNode params) {
            return switch (method) {
                case "transmit" -> handleTransmit(params);
                case "receive" -> handleReceive();
                case "status" -> handleStatus(params);
                case "health" -> handleHealth();
                default -> throw new IllegalArgumentException("unknown method: " + method);
            };
        }

        private ObjectNode handleTransmit(JsonNode params) {
            requireString(params, "to");
            requireString(params, "doc_type");
            requireString(params, "process_id");
            requireString(params, "payload_b64");
            ObjectNode out = JSON.createObjectNode();
            out.put("message_id", UUID.randomUUID().toString());
            // Receipt MDN will be the real phase4 receipt once the
            // outbound path is wired; for now we return an empty
            // base64 placeholder so the Rust side can exercise the
            // happy-path JSON.
            out.put("receipt_b64", "");
            return out;
        }

        private ObjectNode handleReceive() {
            ObjectNode out = JSON.createObjectNode();
            out.set("messages", JSON.createArrayNode());
            return out;
        }

        private ObjectNode handleStatus(JsonNode params) {
            requireString(params, "message_id");
            ObjectNode out = JSON.createObjectNode();
            out.put("state", "queued");
            out.put("detail", "phase4 sidecar scaffold: real status will land after AP cert + SML registration");
            return out;
        }

        private ObjectNode handleHealth() {
            ObjectNode out = JSON.createObjectNode();
            out.put("version", "0.1.0");
            out.put("sml", smlMode);
            return out;
        }

        private void writeResult(HttpExchange exchange, String id, ObjectNode result) throws IOException {
            ObjectNode body = JSON.createObjectNode();
            body.put("jsonrpc", "2.0");
            body.put("id", id);
            body.set("result", result);
            byte[] bytes = JSON.writeValueAsBytes(body);
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, bytes.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(bytes);
            }
        }

        private void writeError(HttpExchange exchange, int httpStatus, int code, String message) throws IOException {
            ObjectNode body = JSON.createObjectNode();
            body.put("jsonrpc", "2.0");
            body.put("id", (String) null);
            ObjectNode error = JSON.createObjectNode();
            error.put("code", code);
            error.put("message", message);
            body.set("error", error);
            byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            exchange.sendResponseHeaders(httpStatus, bytes.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(bytes);
            }
        }

        private static void requireString(JsonNode params, String field) {
            if (!params.has(field) || params.get(field).isNull() || !params.get(field).isTextual()) {
                throw new IllegalArgumentException("missing required string field: " + field);
            }
        }
    }
}
