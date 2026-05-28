// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HashSet;
import java.util.Locale;
import java.util.Set;
import java.util.UUID;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NamedNodeMap;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;
import org.xml.sax.InputSource;

/**
 * 7psv reflection wrapper around the KoSIT validator
 * ({@code org.kosit:validator:1.6.2}). Calls into
 * {@code de.kosit.validationtool.api.{Check,Configuration,
 * Input,InputFactory,Result}} to validate UBL / XRechnung /
 * CII XML against an EN 16931 scenarios bundle and map the
 * per-rule findings into the T-032 {@code ValidationResult} shape
 * the rust side reads.
 *
 * <p>The scenarios bundle (XSDs + Schematron compiled to XSLT
 * plus the {@code scenarios.xml} file) is NOT bundled in the
 * validator-common JAR; it lives outside the classpath because
 * KoSIT publishes it as a separate ~10 MB ZIP per ruleset
 * (e.g. {@code validator-configuration-xrechnung-2024-1}).
 * Runtime deployments expose the selected scenarios.xml path via the
 * {@code INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS} environment
 * variable. Without that env var KoSIT cannot run domain rule
 * checks — this wrapper returns a typed configuration-missing
 * finding rather than silently passing.
 *
 * <p>Reflection is used so {@code ValidatorSidecar} continues to
 * compile cleanly on Maven profiles where
 * {@code org.kosit:validator} is not on the classpath. At
 * runtime, the {@code kosit} profile is the only one where this
 * code path fires.
 */
final class KositReport {
    static final String SCENARIOS_ENV = "INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS";
    private static final Pattern BR_RULE_ID =
        Pattern.compile("\\bBR(?:-[A-Z]{2,})?-\\d+[A-Z]?\\b");

    private KositReport() {
    }

    static Outcome run(String xml, String profile, String traceId, ObjectMapper mapper) {
        String scenariosPath = System.getenv(SCENARIOS_ENV);
        if (scenariosPath == null || scenariosPath.isBlank()) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(configurationMissingFinding(mapper, profile, traceId));
            return new Outcome(false, null, findings);
        }
        Path scenarios = Paths.get(scenariosPath);
        if (!Files.exists(scenarios)) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(configurationMissingFinding(
                mapper, profile, traceId,
                "scenarios file does not exist: " + scenariosPath));
            return new Outcome(false, null, findings);
        }
        try {
            return runReflective(xml, scenarios, profile, traceId, mapper);
        } catch (Throwable ex) {
            ArrayNode findings = mapper.createArrayNode();
            findings.add(libraryErrorFinding(mapper, profile, traceId, ex));
            return new Outcome(false, null, findings);
        }
    }

    private static Outcome runReflective(
        String xml,
        Path scenarios,
        String profile,
        String traceId,
        ObjectMapper mapper
    ) throws Throwable {
        // Configuration cfg = Configuration.load(scenarios.toURI()).build(processor);
        Class<?> configurationClass = Class.forName("de.kosit.validationtool.api.Configuration");
        Object loaderBuilder = configurationClass
            .getMethod("load", java.net.URI.class)
            .invoke(null, scenarios.toUri());
        Class<?> processorClass = Class.forName("net.sf.saxon.s9api.Processor");
        Object processor = processorClass.getConstructor(boolean.class).newInstance(false);
        Object config = loaderBuilder.getClass()
            .getMethod("build", processorClass)
            .invoke(loaderBuilder, processor);

        // Check check = new DefaultCheck(cfg);
        Class<?> defaultCheckClass = Class.forName("de.kosit.validationtool.impl.DefaultCheck");
        Object configArray = java.lang.reflect.Array.newInstance(configurationClass, 1);
        java.lang.reflect.Array.set(configArray, 0, config);
        Object check = defaultCheckClass
            .getConstructor(processorClass, configArray.getClass())
            .newInstance(processor, configArray);

        // Input input = InputFactory.read(bytes, name);
        Class<?> inputFactoryClass = Class.forName("de.kosit.validationtool.api.InputFactory");
        Class<?> inputClass = Class.forName("de.kosit.validationtool.api.Input");
        Object input = inputFactoryClass
            .getMethod("read", byte[].class, String.class)
            .invoke(null,
                xml.getBytes(StandardCharsets.UTF_8),
                "input-" + (traceId == null ? UUID.randomUUID().toString() : traceId));

        // Result result = check.checkInput(input);
        Object result = defaultCheckClass
            .getMethod("checkInput", inputClass)
            .invoke(check, input);

        ArrayNode findings = mapper.createArrayNode();
        Object accepted = result.getClass().getMethod("isAcceptable").invoke(result);
        boolean valid = accepted instanceof Boolean ? (Boolean) accepted : true;
        Set<String> seen = new HashSet<>();
        Object customFailedAsserts = result.getClass()
            .getMethod("getCustomFailedAsserts")
            .invoke(result);
        appendCustomFailedAsserts(customFailedAsserts, findings, profile, traceId, mapper, seen);
        Object failedAsserts = result.getClass().getMethod("getFailedAsserts").invoke(result);
        appendFailedAsserts(failedAsserts, findings, profile, traceId, mapper, seen);
        Object reportDocument = result.getClass().getMethod("getReportDocument").invoke(result);
        walkReportDocument(reportDocument, findings, profile, traceId, mapper, !valid);
        String rootElement = parseRoot(xml);
        // Don't shadow the reflection-only outcome with a parse
        // failure — the parse here is best-effort metadata.
        return new Outcome(valid, rootElement, findings);
    }

    private static void walkReportDocument(
        Object resultDoc,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper,
        boolean summaryFallback
    ) throws Throwable {
        if (resultDoc == null) {
            if (summaryFallback) {
                findings.add(reportSummaryFinding(mapper, profile, traceId,
                    "kosit-report-document-unavailable"));
            }
            return;
        }
        try {
            Document document = coerceReportDocument(resultDoc);
            int before = findings.size();
            if (document != null && document.getDocumentElement() != null) {
                appendRuleFindings(document.getDocumentElement(), findings,
                    profile, traceId, mapper, new HashSet<>());
            }
            if (summaryFallback && findings.size() == before) {
                findings.add(reportSummaryFinding(mapper, profile, traceId,
                    document == null || document.getDocumentElement() == null
                        ? "kosit-report-document-unavailable"
                        : reportSummary(document.getDocumentElement())));
            }
        } catch (Throwable ex) {
            if (summaryFallback) {
                findings.add(reportSummaryFinding(mapper, profile, traceId,
                    "kosit-report-document-unavailable"));
            }
        }
    }

    static ArrayNode reportFindingsFromXmlForTest(
        String reportXml,
        String profile,
        String traceId,
        ObjectMapper mapper
    ) {
        ArrayNode findings = mapper.createArrayNode();
        Document document = parseDocument(reportXml);
        if (document != null && document.getDocumentElement() != null) {
            appendRuleFindings(document.getDocumentElement(), findings,
                profile, traceId, mapper, new HashSet<>());
        }
        return findings;
    }

    private static void appendFailedAsserts(
        Object failedAsserts,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper,
        Set<String> seen
    ) throws Throwable {
        if (!(failedAsserts instanceof Iterable<?> iterable)) return;
        for (Object failedAssert : iterable) {
            appendFailedAssert(failedAssert, findings, profile, traceId, mapper, seen, null);
        }
    }

    private static void appendCustomFailedAsserts(
        Object customFailedAsserts,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper,
        Set<String> seen
    ) throws Throwable {
        if (!(customFailedAsserts instanceof Iterable<?> iterable)) return;
        for (Object customFailedAssert : iterable) {
            if (customFailedAssert == null) continue;
            Object failedAssert = customFailedAssert.getClass()
                .getMethod("getFailedAssert")
                .invoke(customFailedAssert);
            String severity = normalizeSeverity(
                invokeString(customFailedAssert, "getCustomLevelFlag"));
            appendFailedAssert(failedAssert, findings, profile, traceId, mapper, seen, severity);
        }
    }

    private static void appendFailedAssert(
        Object failedAssert,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper,
        Set<String> seen,
        String severityOverride
    ) throws Throwable {
        if (failedAssert == null) return;
        String message = failedAssertText(failedAssert);
        String ruleId = firstRuleId(invokeString(failedAssert, "getId"));
        if (ruleId == null) {
            ruleId = firstRuleId(invokeString(failedAssert, "getTest"));
        }
        if (ruleId == null) {
            ruleId = firstRuleId(message);
        }
        if (ruleId == null) return;
        String location = invokeString(failedAssert, "getLocation");
        String severity = severityOverride == null
            ? normalizeSeverity(invokeString(failedAssert, "getFlag"))
            : severityOverride;
        String dedupe = ruleId + "\u0000" + nullToEmpty(location) + "\u0000" + message;
        if (seen.add(dedupe)) {
            findings.add(ruleFinding(mapper, profile, traceId, ruleId,
                location, message, severity, "failed-assert"));
        }
    }

    private static String failedAssertText(Object failedAssert) throws Throwable {
        Object text = failedAssert.getClass().getMethod("getText").invoke(failedAssert);
        if (text == null) return "";
        Object content = text.getClass().getMethod("getContent").invoke(text);
        if (!(content instanceof Iterable<?> iterable)) return normalizeWhitespace(text.toString());
        StringBuilder message = new StringBuilder();
        for (Object entry : iterable) {
            if (entry == null) continue;
            if (message.length() > 0) message.append(' ');
            message.append(entry);
        }
        return normalizeWhitespace(message.toString());
    }

    private static String invokeString(Object target, String method) throws Throwable {
        Object value = target.getClass().getMethod(method).invoke(target);
        return value == null ? null : value.toString();
    }

    private static Document coerceReportDocument(Object resultDoc) throws Exception {
        if (resultDoc instanceof Document document) return document;
        if (resultDoc instanceof Element element) return element.getOwnerDocument();
        Object root = resultDoc.getClass().getMethod("getDocumentElement").invoke(resultDoc);
        if (root instanceof Document document) return document;
        if (root instanceof Element element) return element.getOwnerDocument();
        return null;
    }

    private static void appendRuleFindings(
        Element element,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper,
        Set<String> seen
    ) {
        if (isRuleFindingElement(element)) {
            String ruleId = findRuleId(element);
            if (ruleId != null) {
                String location = firstAttribute(element, "location", "path", "xpath");
                String message = normalizeWhitespace(element.getTextContent());
                String dedupe = ruleId + "\u0000" + nullToEmpty(location) + "\u0000" + message;
                if (seen.add(dedupe)) {
                    findings.add(ruleFinding(mapper, profile, traceId, element,
                        ruleId, location, message));
                }
            }
        }
        NodeList children = element.getChildNodes();
        for (int index = 0; index < children.getLength(); index++) {
            Node child = children.item(index);
            if (child instanceof Element childElement) {
                appendRuleFindings(childElement, findings, profile, traceId, mapper, seen);
            }
        }
    }

    private static boolean isRuleFindingElement(Element element) {
        String name = elementName(element).toLowerCase(Locale.ROOT);
        if (name.contains("failed-assert")
            || name.contains("successful-report")
            || name.contains("error")
            || name.contains("warning")
            || name.contains("violation")) {
            return true;
        }
        String ruleAttribute = firstAttribute(element, "id", "ruleId", "ruleID", "rule-id");
        if (name.contains("assertion") && firstRuleId(ruleAttribute) != null) return true;
        return false;
    }

    private static ObjectNode ruleFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        Element element,
        String ruleId,
        String location,
        String message
    ) {
        return ruleFinding(
            mapper,
            profile,
            traceId,
            ruleId,
            location,
            message,
            normalizeSeverity(firstAttribute(element, "flag", "severity", "level")),
            elementName(element));
    }

    private static ObjectNode ruleFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        String ruleId,
        String location,
        String message,
        String severity,
        String reportElement
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", ruleId);
        finding.put("severity", severity);
        if (message != null && !message.isBlank()) {
            finding.put("message", message);
        }
        ObjectNode term = finding.putObject("term");
        if (ruleId.startsWith("BR-CO-")) {
            term.put("kind", "business_term");
        } else {
            term.put("kind", "business_rule");
        }
        term.put("code", ruleId);
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", location == null || location.isBlank() ? "/" : location);
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "KoSIT validator 1.6.2");
        citation.put("section", ruleId);
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Adjust the invoice to satisfy " + ruleId + " in the KoSIT scenarios.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:kosit");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        details.put("report_element", reportElement);
        return finding;
    }

    private static String findRuleId(Element element) {
        NamedNodeMap attrs = element.getAttributes();
        for (int index = 0; index < attrs.getLength(); index++) {
            String value = attrs.item(index).getNodeValue();
            String ruleId = firstRuleId(value);
            if (ruleId != null) return ruleId;
        }
        return firstRuleId(element.getTextContent());
    }

    private static String firstRuleId(String value) {
        if (value == null) return null;
        Matcher matcher = BR_RULE_ID.matcher(value);
        return matcher.find() ? matcher.group() : null;
    }

    private static String firstAttribute(Element element, String... names) {
        for (String name : names) {
            String value = element.getAttribute(name);
            if (value != null && !value.isBlank()) return value;
            value = element.getAttributeNS(null, name);
            if (value != null && !value.isBlank()) return value;
        }
        return null;
    }

    private static String normalizeSeverity(String severity) {
        if (severity == null || severity.isBlank()) return "violation";
        String normalized = severity.toLowerCase(Locale.ROOT);
        if (normalized.contains("fatal")) return "fatal";
        if (normalized.contains("warn")) return "warning";
        if (normalized.contains("info")) return "info";
        return "violation";
    }

    private static String elementName(Element element) {
        String local = element.getLocalName();
        return local == null || local.isBlank() ? element.getNodeName() : local;
    }

    private static String normalizeWhitespace(String value) {
        if (value == null) return "";
        return value.replaceAll("\\s+", " ").trim();
    }

    private static String reportSummary(Element root) {
        StringBuilder summary = new StringBuilder(elementName(root));
        appendElementSummary(root, summary, 0, 12);
        return summary.toString();
    }

    private static int appendElementSummary(
        Element element,
        StringBuilder summary,
        int count,
        int limit
    ) {
        if (count >= limit) return count;
        NodeList children = element.getChildNodes();
        for (int index = 0; index < children.getLength() && count < limit; index++) {
            Node child = children.item(index);
            if (child instanceof Element childElement) {
                summary.append(" > ").append(elementName(childElement));
                count++;
                count = appendElementSummary(childElement, summary, count, limit);
            }
        }
        return count;
    }

    private static String nullToEmpty(String value) {
        return value == null ? "" : value;
    }

    private static String parseRoot(String xml) {
        Document document = parseDocument(xml);
        return document == null || document.getDocumentElement() == null
            ? null : document.getDocumentElement().getNodeName();
    }

    private static Document parseDocument(String xml) {
        try {
            javax.xml.parsers.DocumentBuilderFactory f =
                javax.xml.parsers.DocumentBuilderFactory.newInstance();
            f.setNamespaceAware(true);
            f.setXIncludeAware(false);
            f.setExpandEntityReferences(false);
            f.setFeature(javax.xml.XMLConstants.FEATURE_SECURE_PROCESSING, true);
            f.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
            f.setFeature("http://xml.org/sax/features/external-general-entities", false);
            f.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
            return f.newDocumentBuilder()
                .parse(new InputSource(new ByteArrayInputStream(
                    xml.getBytes(StandardCharsets.UTF_8))));
        } catch (Throwable ex) {
            return null;
        }
    }

    private static ObjectNode reportSummaryFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        String summary
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "KOSIT-REPORT-SUMMARY");
        finding.put("severity", "info");
        finding.put("message",
            "KoSIT scenarios report (summary): "
                + (summary == null ? "" : summary));
        ObjectNode term = finding.putObject("term");
        term.put("kind", "scenarios_report");
        term.put("code", "REPORT");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "KoSIT validator 1.6.2");
        citation.put("section", "scenarios report");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Inspect the KoSIT report XML for per-assertion BR-* findings.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:kosit");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        return finding;
    }

    private static ObjectNode configurationMissingFinding(
        ObjectMapper mapper,
        String profile,
        String traceId
    ) {
        return configurationMissingFinding(mapper, profile, traceId,
            "KoSIT scenarios bundle is not configured (env var "
                + SCENARIOS_ENV + " is not set).");
    }

    private static ObjectNode configurationMissingFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        String message
    ) {
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "KOSIT-SCENARIOS-MISSING");
        finding.put("severity", "fatal");
        finding.put("message", message);
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "KoSIT validator 1.6.2");
        citation.put("section", "scenarios bundle bootstrap");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Set " + SCENARIOS_ENV + " to the absolute path of the "
            + "scenarios.xml inside an unzipped validator-configuration-* bundle.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:kosit");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        return finding;
    }

    private static ObjectNode libraryErrorFinding(
        ObjectMapper mapper,
        String profile,
        String traceId,
        Throwable ex
    ) {
        Throwable root = rootCause(ex);
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "KOSIT-LIBRARY-ERROR");
        finding.put("severity", "fatal");
        finding.put("message",
            "KoSIT raised " + root.getClass().getName()
                + " before producing a result: "
                + (root.getMessage() == null ? "" : root.getMessage()));
        ObjectNode term = finding.putObject("term");
        term.put("kind", "business_group");
        term.put("code", "BG-1");
        ObjectNode loc = finding.putObject("location");
        loc.put("kind", "x_path");
        loc.put("expression", "/");
        ObjectNode citation = finding.putObject("citation");
        citation.put("source", "KoSIT validator 1.6.2");
        citation.put("section", "library invocation");
        ObjectNode fix = finding.putObject("suggested_fix");
        fix.put("summary",
            "Verify the scenarios bundle is intact and KoSIT 1.6.2 is on the classpath.");
        ObjectNode trace = finding.putObject("trace");
        trace.put("backend", "jvm:kosit");
        trace.put("trace_id", traceId == null ? UUID.randomUUID().toString() : traceId);
        ObjectNode details = trace.putObject("details");
        details.put("profile", profile);
        details.put("exception", root.getClass().getName());
        if (root != ex) {
            details.put("wrapper_exception", ex.getClass().getName());
        }
        return finding;
    }

    private static Throwable rootCause(Throwable ex) {
        Throwable current = ex;
        while (current instanceof java.lang.reflect.InvocationTargetException invocation
            && invocation.getCause() != null) {
            current = invocation.getCause();
        }
        return current;
    }

    /** Stable outcome record matching PhiveReport.Outcome for the
     * dispatcher in ValidatorSidecar. */
    record Outcome(boolean valid, String rootElement, ArrayNode results) {
    }
}
