// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.phase4;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.junit.jupiter.api.Test;

final class Phase4ServerTest {

    private static final ObjectMapper JSON = new ObjectMapper();

    private final Phase4Server.RpcHandler handler = new Phase4Server.RpcHandler("acceptance");

    @Test
    void healthReturnsVersionAndSmlMode() {
        ObjectNode result = handler.dispatch("health", JSON.createObjectNode());
        assertEquals("0.1.0", result.get("version").asText());
        assertEquals("acceptance", result.get("sml").asText());
    }

    @Test
    void transmitReturnsMessageIdAndEmptyReceipt() {
        ObjectNode params = JSON.createObjectNode();
        params.put("to", "iso6523-actorid-upis::0192:991825827");
        params.put("doc_type", "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice##urn:cen.eu:en16931:2017");
        params.put("process_id", "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0");
        params.put("payload_b64", "PGludm9pY2UvPg==");
        ObjectNode result = handler.dispatch("transmit", params);
        assertNotNull(result.get("message_id"));
        assertTrue(result.get("message_id").asText().length() > 0);
        assertEquals("", result.get("receipt_b64").asText());
    }

    @Test
    void receiveReturnsEmptyMessagesArray() {
        ObjectNode result = handler.dispatch("receive", JSON.createObjectNode());
        assertEquals(0, result.get("messages").size());
    }

    @Test
    void statusReturnsQueuedForAnyMessageId() {
        ObjectNode params = JSON.createObjectNode();
        params.put("message_id", "msg-123");
        ObjectNode result = handler.dispatch("status", params);
        assertEquals("queued", result.get("state").asText());
        assertNotNull(result.get("detail"));
    }

    @Test
    void transmitRejectsMissingFields() {
        ObjectNode params = JSON.createObjectNode();
        params.put("to", "iso6523-actorid-upis::0192:991825827");
        assertThrows(IllegalArgumentException.class,
            () -> handler.dispatch("transmit", params));
    }

    @Test
    void dispatchRejectsUnknownMethod() {
        assertThrows(IllegalArgumentException.class,
            () -> handler.dispatch("evict-cache", JSON.createObjectNode()));
    }

    @Test
    void smlModeFromConstructorIsEchoed() {
        Phase4Server.RpcHandler prod = new Phase4Server.RpcHandler("production");
        JsonNode result = prod.dispatch("health", JSON.createObjectNode());
        assertEquals("production", result.get("sml").asText());
    }
}
