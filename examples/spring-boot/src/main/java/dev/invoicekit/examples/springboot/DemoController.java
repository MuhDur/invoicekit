// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.examples.springboot;

import java.util.Map;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RestController;

/** T-1403 reference Spring Boot demo controller. */
@RestController
public class DemoController {
    private final InvoiceKitBridge bridge;

    public DemoController(InvoiceKitBridge bridge) {
        this.bridge = bridge;
    }

    @GetMapping("/")
    public Map<String, Object> index() {
        return Map.of(
            "title", "InvoiceKit Spring Boot demo",
            "fixtures", Fixtures.names(),
            "usage", "POST /canonicalize/{fixtureName}"
        );
    }

    @GetMapping("/healthz")
    public Map<String, String> healthz() {
        return Map.of("status", "ok");
    }

    @PostMapping("/canonicalize/{fixtureName}")
    public ResponseEntity<Map<String, Object>> canonicalize(
        @PathVariable String fixtureName
    ) {
        Map<String, Object> document = Fixtures.get(fixtureName);
        if (document == null) {
            return ResponseEntity.status(404).body(Map.of(
                "error", Map.of(
                    "code", "UNKNOWN_FIXTURE",
                    "available", Fixtures.names()
                )
            ));
        }
        return ResponseEntity.ok(bridge.canonicalize(document));
    }
}
