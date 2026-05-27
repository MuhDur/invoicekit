// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.util.UUID;
import org.w3c.dom.Document;

/**
 * 7psv reflection wrapper around phive's Peppol validation
 * pipeline ({@code phive-rules-peppol:3.2.2}). Calls into
 * {@code com.helger.phive.peppol.PeppolValidation.initStandard(...)}
 * to register the latest Peppol BIS Billing 3.0 invoice rule set,
 * resolves the rule set's {@code DVRCoordinate} from one of the
 * dated {@code PeppolValidation20XX_XX.VID_OPENPEPPOL_INVOICE_UBL_V3}
 * constants, then invokes
 * {@code ValidationExecutionManager.executeValidation(...)} and
 * maps every per-rule finding into the T-032
 * {@code ValidationResult} shape that the rust side reads.
 *
 * <p>Reflection is used so {@code ValidatorSidecar} continues to
 * compile cleanly on Maven profiles where
 * {@code phive-rules-peppol} is not on the classpath (kosit,
 * saxon, verapdf). At runtime, the {@code phive} profile is the
 * only one where this code path fires.
 *
 * <p>The wrapper scans for the latest {@code PeppolValidation20XX_XX}
 * class on the classpath at runtime and prefers its
 * {@code VID_OPENPEPPOL_INVOICE_UBL_V3} constant. This avoids
 * hard-coding a version string that rolls every six months as
 * Peppol publishes new validation artefact sets.
 */
final class PhiveReport {
    private PhiveReport() {
    }

    /** Run validation. Returns a populated outcome on any path.
     * Never throws — every failure mode lands as a typed finding. */
    static Outcome run(String xml, String profile, String traceId, ObjectMapper mapper) {
        try {
            return runReflective(xml, profile, traceId, mapper);
        } catch (Throwable ex) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(libraryErrorFinding(mapper, profile, traceId, ex));
            return new Outcome(false, "Invoice", findings);
        }
    }

    private static Outcome runReflective(
        String xml,
        String profile,
        String traceId,
        ObjectMapper mapper
    ) throws Throwable {
        // Parse the input XML into a DOM Node — the phive
        // ValidationSourceXML.create() factory we use takes a
        // (String, Node) pair.
        Document inputDoc = parseDocument(xml);
        if (inputDoc == null) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(parseErrorFinding(mapper, profile, traceId));
            return new Outcome(false, "Invoice", findings);
        }

        Class<?> registryClass = Class.forName(
            "com.helger.phive.api.executorset.ValidationExecutorSetRegistry");
        Object registry = registryClass.getDeclaredConstructor().newInstance();

        // Register every standard Peppol rule set into the registry.
        Class<?> peppolValidation = Class.forName("com.helger.phive.peppol.PeppolValidation");
        Class<?> registryIface = Class.forName(
            "com.helger.phive.api.executorset.IValidationExecutorSetRegistry");
        peppolValidation
            .getMethod("initStandard", registryIface)
            .invoke(null, registry);

        // Resolve the VID for Peppol BIS Billing 3.0 UBL Invoice
        // from the most-recent PeppolValidation20XX_XX class on the
        // classpath. The classpath has 2023_05, 2023_11, 2024_05,
        // 2024_11 in phive-rules-peppol:3.2.2.
        Object vid = pickLatestInvoiceVid();
        if (vid == null) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(noMatchingSetFinding(mapper, profile, traceId));
            return new Outcome(false, "Invoice", findings);
        }

        Class<?> dvrCoordinate = Class.forName("com.helger.diver.api.coord.DVRCoordinate");
        Method getOfID = registryClass.getMethod("getOfID", dvrCoordinate);
        Object xset = getOfID.invoke(registry, vid);
        if (xset == null) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(noMatchingSetFinding(mapper, profile, traceId));
            return new Outcome(false, "Invoice", findings);
        }

        // ValidationSourceXML.create(String, org.w3c.dom.Node)
        Class<?> validationSourceXml = Class.forName(
            "com.helger.phive.xml.source.ValidationSourceXML");
        Object source = validationSourceXml
            .getMethod("create", String.class, org.w3c.dom.Node.class)
            .invoke(null, "input-" + (traceId == null ? "anon" : traceId), inputDoc);

        // ValidationExecutionManager.executeValidation(IValidationExecutorSet, ST)
        Class<?> executionManager = Class.forName(
            "com.helger.phive.api.execute.ValidationExecutionManager");
        Class<?> ivExecutorSet = Class.forName(
            "com.helger.phive.api.executorset.IValidationExecutorSet");
        Class<?> ivSource = Class.forName("com.helger.phive.api.source.IValidationSource");
        Object resultList = executionManager
            .getMethod("executeValidation", ivExecutorSet, ivSource)
            .invoke(null, xset, source);

        // ValidationResultList extends ArrayList<ValidationResult>;
        // getAllErrors() returns a flat ErrorList across all
        // results, which is what we want.
        ArrayNode findings = mapper.createArrayNode();
        Object errors = resultList.getClass().getMethod("getAllErrors").invoke(resultList);
        @SuppressWarnings("unchecked")
        Iterable<Object> errorIter = (Iterable<Object>) errors;
        for (Object error : errorIter) {
            ObjectNode finding = mapErrorToFinding(mapper, error, profile, traceId);
            if (finding != null) {
                findings.add(finding);
            }
        }
        return new Outcome(findings.size() == 0, "Invoice", findings);
    }

    /** Pick the latest PeppolValidation20XX_XX.VID_OPENPEPPOL_INVOICE_UBL_V3 on the classpath. */
    private static Object pickLatestInvoiceVid() {
        String[] candidates = new String[] {
            "com.helger.phive.peppol.PeppolValidation2024_11",
            "com.helger.phive.peppol.PeppolValidation2024_05",
            "com.helger.phive.peppol.PeppolValidation2023_11",
            "com.helger.phive.peppol.PeppolValidation2023_05",
        };
        for (String fqn : candidates) {
            try {
                Class<?> klass = Class.forName(fqn);
                Field f = klass.getField("VID_OPENPEPPOL_INVOICE_UBL_V3");
                Object value = f.get(null);
                if (value != null) return value;
            } catch (Throwable ex) {
                // try the next candidate
            }
        }
        return null;
    }

    private static Document parseDocument(String xml) {
        try {
            javax.xml.parsers.DocumentBuilderFactory factory =
                javax.xml.parsers.DocumentBuilderFactory.newInstance();
            factory.setNamespaceAware(true);
            factory.setXIncludeAware(false);
            factory.setExpandEntityReferences(false);
            factory.setFeature(javax.xml.XMLConstants.FEATURE_SECURE_PROCESSING, true);
            factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
            factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
            factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
            return factory.newDocumentBuilder()
                .parse(new org.xml.sax.InputSource(new ByteArrayInputStream(
                    xml.getBytes(StandardCharsets.UTF_8))));
        } catch (Throwable ex) {
            return null;
        }
    }

    private static ObjectNode mapErrorToFinding(
        ObjectMapper mapper,
        Object error,
        String profile,
        String traceId
    ) {
        Class<?> ec = error.getClass();
        String errorId = String.valueOf(invokeOrEmpty(ec, error, "getErrorID"));
        String message = readErrorText(error);
        String severity = readErrorLevel(error);
        String location = String.valueOf(invokeOrEmpty(ec, error, "getErrorFieldName"));

        if (errorId == null || errorId.isBlank() || "null".equals(errorId)) {
            errorId = "PHIVE-UNNAMED";
        }
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", errorId);
        finding.put("severity", normalizeSeverity(severity));

        ObjectNode term = finding.putObject("term");
        if (errorId.startsWith("BR-CO-")) {
            term.put("kind", "business_term");
            term.put("code", errorId);
        } else if (errorId.startsWith("BR-")) {
            term.put("kind", "business_rule");
            term.put("code", errorId);
        } else {
            term.put("kind", "phive_rule");
            term.put("code", errorId);
        }

        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", location == null || "null".equals(location) || location.isBlank()
            ? "/" : location);

        if (message != null && !message.isBlank() && !"null".equals(message)) {
            finding.put("message", message);
        }

        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "phive Peppol validation rules 3.2.2");
        citation.put("section", errorId);

        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Adjust the invoice to satisfy " + errorId + " — see Peppol BIS Billing 3.0.");

        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:phive");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        return finding;
    }

    private static Object invokeOrEmpty(Class<?> klass, Object instance, String method) {
        try {
            Method m = klass.getMethod(method);
            Object out = m.invoke(instance);
            return out == null ? "" : out;
        } catch (Throwable ex) {
            return "";
        }
    }

    /** IError.getErrorText(Locale) — pass ROOT so the message comes out
     * locale-independent. */
    private static String readErrorText(Object error) {
        try {
            Method m = error.getClass().getMethod("getErrorText", java.util.Locale.class);
            Object out = m.invoke(error, java.util.Locale.ROOT);
            return out == null ? "" : String.valueOf(out);
        } catch (Throwable ex) {
            return "";
        }
    }

    /** IError.getErrorLevel() → IErrorLevel; pull the ID/name and
     * normalize. */
    private static String readErrorLevel(Object error) {
        try {
            Object level = error.getClass().getMethod("getErrorLevel").invoke(error);
            if (level == null) return "";
            try {
                Object id = level.getClass().getMethod("getID").invoke(level);
                return id == null ? "" : String.valueOf(id);
            } catch (Throwable ignored) {
                return String.valueOf(level);
            }
        } catch (Throwable ex) {
            return "";
        }
    }

    private static String normalizeSeverity(String severity) {
        if (severity == null || severity.isBlank()) return "violation";
        String s = severity.toLowerCase();
        if (s.contains("fatal")) return "fatal";
        if (s.contains("error")) return "violation";
        if (s.contains("warn")) return "warning";
        if (s.contains("info")) return "info";
        return "violation";
    }

    private static ObjectNode libraryErrorFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        Throwable ex
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "PHIVE-LIBRARY-ERROR");
        finding.put("severity", "fatal");
        finding.put("message",
            "phive raised " + ex.getClass().getName()
            + " before producing a result: "
            + (ex.getMessage() == null ? "" : ex.getMessage()));
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "phive Peppol validation rules 3.2.2");
        citation.put("section", "library bootstrap");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Verify phive-rules-peppol is on the classpath of the validator-phive image.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:phive");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        details.put("exception", ex.getClass().getName());
        return finding;
    }

    private static ObjectNode noMatchingSetFinding(
        ObjectMapper mapper,
        String profile,
        String traceId
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "PHIVE-NO-MATCHING-RULESET");
        finding.put("severity", "fatal");
        finding.put("message",
            "No phive validation set matched profile " + profile
            + " (expected a Peppol BIS Billing 3.0 invoice set).");
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "phive Peppol validation rules 3.2.2");
        citation.put("section", "rule-set selection");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Request a Peppol BIS Billing 3.0 profile or update phive-rules-peppol.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:phive");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        return finding;
    }

    private static ObjectNode parseErrorFinding(
        ObjectMapper mapper,
        String profile,
        String traceId
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "PHIVE-XML-PARSE");
        finding.put("severity", "fatal");
        finding.put("message", "phive could not parse the input as XML.");
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "phive Peppol validation rules 3.2.2");
        citation.put("section", "input parsing");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary", "Provide well-formed UBL XML for phive validation.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:phive");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        return finding;
    }

    /** Stable outcome record so the dispatcher in ValidatorSidecar
     * can consume Kosit/Phive paths uniformly. */
    record Outcome(boolean valid, String rootElement, ArrayNode results) {
    }
}
