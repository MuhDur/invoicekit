// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.examples.springboot;

import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

/** T-1403 demo fixtures: three German XRechnung shapes. */
public final class Fixtures {
    private Fixtures() {}

    public static final Map<String, Map<String, Object>> ALL =
        buildAllFixtures();

    public static Set<String> names() {
        return ALL.keySet();
    }

    public static Map<String, Object> get(String name) {
        return ALL.get(name);
    }

    private static Map<String, Map<String, Object>> buildAllFixtures() {
        Map<String, Map<String, Object>> fixtures = new TreeMap<>();
        fixtures.put("basic", basic());
        fixtures.put("with-allowance", withAllowance());
        fixtures.put("reverse-charge", reverseCharge());
        return fixtures;
    }

    private static Map<String, Object> basic() {
        Map<String, Object> doc = baseDocument(
            "doc-de-spring-basic-2026-0001",
            "RE-SP-2026-0001",
            "basic"
        );
        doc.put(
            "lines",
            List.of(
                line("L1", "Software-Lizenz Q3/2026", "1", "1000.00", "1000.00", "S")
            )
        );
        doc.put(
            "tax_summary",
            List.of(taxSummary("S", "1000.00", "190.00", "19.00"))
        );
        doc.put(
            "monetary_total",
            monetaryTotal("1000.00", "1000.00", "1190.00", "1190.00")
        );
        return doc;
    }

    private static Map<String, Object> withAllowance() {
        Map<String, Object> doc = baseDocument(
            "doc-de-spring-allowance-2026-0002",
            "RE-SP-2026-0002",
            "with-allowance"
        );
        doc.put(
            "lines",
            List.of(
                line("L1", "Beratungsleistung März 2026", "10", "150.00", "1500.00", "S"),
                line("L2", "Mengenrabatt 10%", "-1", "150.00", "-150.00", "S")
            )
        );
        doc.put(
            "tax_summary",
            List.of(taxSummary("S", "1350.00", "256.50", "19.00"))
        );
        doc.put(
            "monetary_total",
            monetaryTotal("1350.00", "1350.00", "1606.50", "1606.50")
        );
        return doc;
    }

    private static Map<String, Object> reverseCharge() {
        Map<String, Object> doc = baseDocument(
            "doc-de-spring-rc-2026-0003",
            "RE-SP-2026-0003",
            "reverse-charge"
        );
        Map<String, Object> customer = new TreeMap<>(buyer());
        customer.put(
            "tax_ids",
            List.of(Map.of("scheme", "vat", "value", "ATU12345678"))
        );
        customer.put(
            "address",
            Map.of(
                "lines", List.of("Stephansplatz 1"),
                "city", "Wien",
                "postal_code", "1010",
                "country", "AT"
            )
        );
        doc.put("customer", customer);
        doc.put(
            "lines",
            List.of(
                line("L1", "Wartungsvertrag Q3/2026", "1", "5000.00", "5000.00", "AE")
            )
        );
        doc.put(
            "tax_summary",
            List.of(taxSummary("AE", "5000.00", "0.00", "0.00"))
        );
        doc.put(
            "monetary_total",
            monetaryTotal("5000.00", "5000.00", "5000.00", "5000.00")
        );
        return doc;
    }

    private static Map<String, Object> baseDocument(String id, String number, String name) {
        Map<String, Object> doc = new TreeMap<>();
        doc.put("schema_version", "1.0");
        doc.put("id", id);
        doc.put("document_type", "invoice");
        doc.put("issue_date", "2026-05-27");
        doc.put("due_date", "2026-06-26");
        doc.put("document_number", number);
        doc.put("currency", "EUR");
        doc.put("supplier", seller());
        doc.put("customer", buyer());
        doc.put("payment_instructions", List.of());
        doc.put("extensions", List.of());
        doc.put(
            "meta",
            Map.of("tenant_id", "tenant-demo-spring", "trace_id", "trace-spring-" + name)
        );
        return doc;
    }

    private static Map<String, Object> seller() {
        return Map.of(
            "name", "Acme GmbH",
            "tax_ids", List.of(Map.of("scheme", "vat", "value", "DE123456789")),
            "address", Map.of(
                "lines", List.of("Hauptstraße 42"),
                "city", "Berlin",
                "postal_code", "10115",
                "country", "DE"
            )
        );
    }

    private static Map<String, Object> buyer() {
        return Map.of(
            "name", "Beispielkunde AG",
            "tax_ids", List.of(Map.of("scheme", "vat", "value", "DE987654321")),
            "address", Map.of(
                "lines", List.of("Friedrichstraße 10"),
                "city", "München",
                "postal_code", "80331",
                "country", "DE"
            )
        );
    }

    private static Map<String, Object> line(
        String id, String description, String quantity,
        String unitPrice, String lineExtensionAmount, String taxCategory
    ) {
        Map<String, Object> line = new TreeMap<>();
        line.put("id", id);
        line.put("description", description);
        line.put("quantity", quantity);
        line.put("unit_price", unitPrice);
        line.put("line_extension_amount", lineExtensionAmount);
        line.put("tax_category", taxCategory);
        line.put("extensions", List.of());
        return line;
    }

    private static Map<String, Object> taxSummary(
        String categoryCode, String taxableAmount, String taxAmount, String taxRate
    ) {
        return Map.of(
            "category_code", categoryCode,
            "taxable_amount", taxableAmount,
            "tax_amount", taxAmount,
            "tax_rate", taxRate
        );
    }

    private static Map<String, Object> monetaryTotal(
        String lineExtension, String taxExclusive, String taxInclusive, String payable
    ) {
        return Map.of(
            "line_extension_amount", lineExtension,
            "tax_exclusive_amount", taxExclusive,
            "tax_inclusive_amount", taxInclusive,
            "payable_amount", payable
        );
    }
}
