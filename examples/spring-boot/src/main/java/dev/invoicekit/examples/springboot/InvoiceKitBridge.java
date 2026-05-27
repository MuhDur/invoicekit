// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.examples.springboot;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.invoicekit.EngineClient;
import dev.invoicekit.InvoiceKit;
import dev.invoicekit.InvoiceKitException;
import java.util.Map;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Component;

/**
 * T-1403 thin wrapper around the InvoiceKit Java SDK's
 * {@link EngineClient}. Holds a single native client for the
 * lifetime of the Spring application and exposes a
 * {@link #canonicalize(Map)} method the controller calls.
 *
 * <p>The bridge prefers the Foreign Function and Memory API
 * native client (Java 22+) and falls back to a REST sidecar at
 * {@code http://127.0.0.1:8081/v1/engine/process_json} when the
 * native shared library cannot be loaded. CI provides
 * {@code INVOICEKIT_FFI_LIB} so the native path always wins.
 */
@Component
public class InvoiceKitBridge {
    private static final ObjectMapper MAPPER = new ObjectMapper();
    private static final String CANONICALIZE_TEMPLATE_PREFIX =
        "{\"abi_version\":1,\"operation\":\"commercial_document.canonicalize\",\"payload\":";

    private final EngineClient engine;

    @Autowired
    public InvoiceKitBridge() {
        this(createEngine());
    }

    private static EngineClient createEngine() {
        // Eagerly require the native client when the demo boots.
        // Falling back to a REST sidecar silently would hide a
        // misconfigured INVOICEKIT_FFI_LIB env var until a request
        // came in. Throwing here makes the misconfig visible at
        // startup time.
        try {
            return InvoiceKit.nativeClient();
        } catch (InvoiceKitException ex) {
            throw new IllegalStateException(
                "invoicekit native client unavailable: " + ex.getMessage()
                    + " (set INVOICEKIT_FFI_LIB to the absolute path of "
                    + "libinvoicekit_ffi.so/.dylib/.dll, or use Java 22+ for FFM)",
                ex);
        }
    }

    InvoiceKitBridge(EngineClient engine) {
        this.engine = engine;
    }

    /** Canonicalise a CommercialDocument map through the engine. */
    public Map<String, Object> canonicalize(Map<String, Object> document) {
        try {
            String payload = MAPPER.writeValueAsString(document);
            String request = CANONICALIZE_TEMPLATE_PREFIX + payload + "}";
            String response = InvoiceKit.processEngineAbiJson(engine, request);
            JsonNode parsed = MAPPER.readTree(response);
            if (parsed instanceof ObjectNode obj) {
                obj.put("_engine_status", 0);
            }
            return MAPPER.convertValue(parsed, Map.class);
        } catch (InvoiceKitException ex) {
            throw new RuntimeException(
                "invoicekit engine call failed: " + ex.getMessage(), ex);
        } catch (com.fasterxml.jackson.core.JsonProcessingException ex) {
            throw new RuntimeException(
                "invoicekit JSON serialisation failed: " + ex.getMessage(), ex);
        }
    }
}
