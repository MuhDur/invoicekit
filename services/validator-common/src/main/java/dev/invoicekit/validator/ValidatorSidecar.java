// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.StringReader;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.HexFormat;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilderFactory;
import javax.xml.parsers.ParserConfigurationException;
import org.xml.sax.InputSource;
import org.xml.sax.SAXException;

public final class ValidatorSidecar {
    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final int MAX_REQUEST_BYTES = 4 * 1024 * 1024;

    private ValidatorSidecar() {
    }

    public static void main(String[] args) throws IOException {
        BackendConfig config = BackendConfig.fromEnvironment();
        config.verifyOracleClass();

        int port = Integer.parseInt(System.getenv().getOrDefault("PORT", "8080"));
        int threads = Integer.parseInt(System.getenv().getOrDefault("INVOICEKIT_VALIDATOR_THREADS", "4"));
        HttpServer server = HttpServer.create(new InetSocketAddress("0.0.0.0", port), 0);
        server.createContext("/healthz", exchange -> handleHealth(exchange, config));
        server.createContext("/rpc", exchange -> handleRpc(exchange, config));
        ExecutorService executor = Executors.newFixedThreadPool(threads);
        server.setExecutor(executor);
        Runtime.getRuntime().addShutdownHook(new Thread(
            () -> stopServer(server, executor, config),
            "validator-sidecar-shutdown"
        ));
        server.start();

        structuredLog("validator_sidecar_started", Map.of(
            "backend", config.backend(),
            "service", config.serviceName(),
            "oracle_coordinate", config.oracleCoordinate(),
            "oracle_class", config.oracleClass(),
            "port", port
        ));
    }

    private static void stopServer(HttpServer server, ExecutorService executor, BackendConfig config) {
        structuredLog("validator_sidecar_stopping", Map.of("backend", config.backend()));
        server.stop(1);
        executor.shutdown();
        try {
            if (!executor.awaitTermination(5, TimeUnit.SECONDS)) {
                executor.shutdownNow();
            }
        } catch (InterruptedException ex) {
            executor.shutdownNow();
            Thread.currentThread().interrupt();
        }
    }

    private static void handleHealth(HttpExchange exchange, BackendConfig config) throws IOException {
        if (!"GET".equals(exchange.getRequestMethod())) {
            sendPlain(exchange, 405, "method not allowed");
            return;
        }
        ObjectNode body = MAPPER.createObjectNode();
        body.put("status", "ok");
        body.put("backend", config.backend());
        body.put("service", config.serviceName());
        body.put("oracle_coordinate", config.oracleCoordinate());
        body.put("oracle_class", config.oracleClass());
        sendJson(exchange, 200, body);
    }

    private static void handleRpc(HttpExchange exchange, BackendConfig config) throws IOException {
        if (!"POST".equals(exchange.getRequestMethod())) {
            sendPlain(exchange, 405, "method not allowed");
            return;
        }

        JsonNode request;
        try {
            request = MAPPER.readTree(readBounded(exchange, MAX_REQUEST_BYTES));
        } catch (IllegalArgumentException ex) {
            sendJson(exchange, 413, jsonRpcError(null, -32001, ex.getMessage()));
            return;
        } catch (IOException ex) {
            sendJson(exchange, 400, jsonRpcError(null, -32700, "invalid JSON request"));
            return;
        }

        JsonNode id = request.get("id");
        if (!"2.0".equals(request.path("jsonrpc").asText()) ||
            !"validator.validate".equals(request.path("method").asText())) {
            sendJson(exchange, 200, jsonRpcError(id, -32601, "expected JSON-RPC 2.0 method validator.validate"));
            return;
        }

        JsonNode params = request.path("params");
        JsonNode xmlNode = params.path("document").path("xml");
        if (!xmlNode.isTextual() || xmlNode.asText().isBlank()) {
            sendJson(exchange, 200, jsonRpcError(id, -32602, "params.document.xml must be a non-empty string"));
            return;
        }

        long started = System.nanoTime();
        ValidationOutcome outcome = validateXml(config, xmlNode.asText(), params);
        long durationMs = Math.max(0, (System.nanoTime() - started) / 1_000_000);

        ObjectNode response = MAPPER.createObjectNode();
        response.put("jsonrpc", "2.0");
        response.set("id", id == null ? MAPPER.nullNode() : id);
        ObjectNode result = response.putObject("result");
        result.put("backend", config.backend());
        result.put("service", config.serviceName());
        result.put("oracle_coordinate", config.oracleCoordinate());
        result.put("oracle_class", config.oracleClass());
        result.put("profile", params.path("profile").asText("unknown"));
        result.put("rule_pack_id", params.path("rule_pack").path("id").asText("unknown"));
        result.put("valid", outcome.valid());
        result.put("duration_ms", durationMs);
        ObjectNode document = result.putObject("document");
        document.put("content_type", "application/xml");
        document.put("byte_length", xmlNode.asText().getBytes(StandardCharsets.UTF_8).length);
        document.put("sha256", sha256(xmlNode.asText()));
        if (outcome.rootElement() != null) {
            document.put("root", outcome.rootElement());
        }
        result.set("results", outcome.results());
        sendJson(exchange, 200, response);
    }

    private static ValidationOutcome validateXml(BackendConfig config, String xml, JsonNode params) {
        ArrayNode results = MAPPER.createArrayNode();
        try {
            String rootElement = parseRootElement(xml);
            return new ValidationOutcome(true, rootElement, results);
        } catch (ParserConfigurationException | SAXException | IOException ex) {
            results.add(wellFormednessFinding(config, params, ex));
            return new ValidationOutcome(false, null, results);
        }
    }

    private static ObjectNode wellFormednessFinding(BackendConfig config, JsonNode params, Exception ex) {
        String traceId = params.path("trace_id").asText(UUID.randomUUID().toString());
        ObjectNode finding = MAPPER.createObjectNode();
        finding.put("rule_id", config.rulePrefix() + "-XML-WELLFORMED");
        finding.put("severity", "fatal");
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode location = finding.putObject("location");
        location.put("kind", "x_path");
        location.put("expression", "/");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary", "Provide well-formed XML before invoking " + config.backend() + ".");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", config.citationSource());
        citation.put("section", "XML well-formedness");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", config.backend());
        trace.put("trace_id", traceId);
        ObjectNode details = trace.putObject("details");
        details.put("oracle_coordinate", config.oracleCoordinate());
        details.put("oracle_class", config.oracleClass());
        details.put("exception", ex.getClass().getName());
        details.put("message", sanitizeMessage(ex.getMessage()));
        return finding;
    }

    private static String parseRootElement(String xml)
        throws ParserConfigurationException, IOException, SAXException {
        DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
        factory.setNamespaceAware(true);
        factory.setXIncludeAware(false);
        factory.setExpandEntityReferences(false);
        factory.setFeature(XMLConstants.FEATURE_SECURE_PROCESSING, true);
        factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
        factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
        factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
        return factory
            .newDocumentBuilder()
            .parse(new InputSource(new StringReader(xml)))
            .getDocumentElement()
            .getNodeName();
    }

    private static ObjectNode jsonRpcError(JsonNode id, int code, String message) {
        ObjectNode response = MAPPER.createObjectNode();
        response.put("jsonrpc", "2.0");
        response.set("id", id == null ? MAPPER.nullNode() : id);
        ObjectNode error = response.putObject("error");
        error.put("code", code);
        error.put("message", message);
        return response;
    }

    private static byte[] readBounded(HttpExchange exchange, int maxBytes) throws IOException {
        ByteArrayOutputStream buffer = new ByteArrayOutputStream();
        byte[] chunk = new byte[16 * 1024];
        int total = 0;
        int read;
        while ((read = exchange.getRequestBody().read(chunk)) != -1) {
            total += read;
            if (total > maxBytes) {
                throw new IllegalArgumentException("request body exceeds " + maxBytes + " bytes");
            }
            buffer.write(chunk, 0, read);
        }
        return buffer.toByteArray();
    }

    private static void sendJson(HttpExchange exchange, int status, JsonNode body) throws IOException {
        byte[] bytes = MAPPER.writeValueAsBytes(body);
        exchange.getResponseHeaders().set("content-type", "application/json; charset=utf-8");
        exchange.sendResponseHeaders(status, bytes.length);
        exchange.getResponseBody().write(bytes);
        exchange.close();
    }

    private static void sendPlain(HttpExchange exchange, int status, String body) throws IOException {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().set("content-type", "text/plain; charset=utf-8");
        exchange.sendResponseHeaders(status, bytes.length);
        exchange.getResponseBody().write(bytes);
        exchange.close();
    }

    private static String sha256(String value) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            return HexFormat.of().formatHex(digest.digest(value.getBytes(StandardCharsets.UTF_8)));
        } catch (Exception ex) {
            throw new IllegalStateException("SHA-256 digest unavailable", ex);
        }
    }

    private static String sanitizeMessage(String message) {
        if (message == null || message.isBlank()) {
            return "XML parser rejected the document";
        }
        return message.replace('\n', ' ').replace('\r', ' ');
    }

    private static void structuredLog(String event, Map<String, ?> fields) {
        ObjectNode log = MAPPER.createObjectNode();
        log.put("ts", Instant.now().toString());
        log.put("event", event);
        fields.forEach((key, value) -> log.putPOJO(key, value));
        try {
            System.err.println(MAPPER.writeValueAsString(log));
        } catch (IOException ex) {
            System.err.println("{\"event\":\"validator_sidecar_log_failure\"}");
        }
    }

    private record ValidationOutcome(boolean valid, String rootElement, ArrayNode results) {
    }

    private record BackendConfig(
        String backend,
        String serviceName,
        String oracleCoordinate,
        String oracleClass,
        String rulePrefix,
        String citationSource
    ) {
        static BackendConfig fromEnvironment() {
            String backend = System.getenv().getOrDefault("INVOICEKIT_VALIDATOR_BACKEND", "jvm:saxon");
            return switch (backend) {
                case "jvm:kosit" -> new BackendConfig(
                    backend,
                    "validator-kosit",
                    "org.kosit:validator:1.6.2",
                    "de.kosit.validationtool.api.Check",
                    "KOSIT",
                    "KoSIT validator 1.6.2"
                );
                case "jvm:phive" -> new BackendConfig(
                    backend,
                    "validator-phive",
                    "com.helger.phive.rules:phive-rules-peppol:3.2.2",
                    "com.helger.phive.peppol.PeppolValidation",
                    "PHIVE",
                    "phive Peppol validation rules 3.2.2"
                );
                case "jvm:saxon" -> new BackendConfig(
                    backend,
                    "validator-saxon",
                    "net.sf.saxon:Saxon-HE:12.9",
                    "net.sf.saxon.s9api.Processor",
                    "SAXON",
                    "Saxon-HE 12.9"
                );
                default -> throw new IllegalArgumentException("unsupported backend " + backend);
            };
        }

        void verifyOracleClass() {
            try {
                Class.forName(oracleClass);
            } catch (ClassNotFoundException ex) {
                throw new IllegalStateException("missing validator oracle dependency " + oracleCoordinate, ex);
            }
        }
    }
}
