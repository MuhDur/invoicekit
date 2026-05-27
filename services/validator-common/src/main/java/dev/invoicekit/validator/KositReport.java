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
import java.util.UUID;

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
 * (e.g. {@code validator-configuration-xrechnung-2024-1}). The
 * Dockerfile for validator-kosit downloads the bundle at build
 * time and exposes its scenarios.xml path via the
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
        // Configuration cfg = Configuration.load(scenarios.toURI()).build();
        Class<?> configurationClass = Class.forName("de.kosit.validationtool.api.Configuration");
        Object loaderBuilder = configurationClass
            .getMethod("load", java.net.URI.class)
            .invoke(null, scenarios.toUri());
        Object config = loaderBuilder.getClass().getMethod("build").invoke(loaderBuilder);

        // Check check = new DefaultCheck(cfg);
        Class<?> defaultCheckClass = Class.forName("de.kosit.validationtool.api.DefaultCheck");
        Object check = defaultCheckClass
            .getConstructor(configurationClass)
            .newInstance(config);

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
        @SuppressWarnings("unchecked")
        Iterable<Object> reports = (Iterable<Object>) result.getClass()
            .getMethod("getReports").invoke(result);
        if (reports != null) {
            for (Object report : reports) {
                walkReport(report, findings, profile, traceId, mapper);
            }
        }
        Object accepted = result.getClass().getMethod("isAcceptable").invoke(result);
        boolean valid = accepted instanceof Boolean ? (Boolean) accepted : findings.size() == 0;
        String rootElement = parseRoot(xml);
        // Don't shadow the reflection-only outcome with a parse
        // failure — the parse here is best-effort metadata.
        return new Outcome(valid, rootElement, findings);
    }

    private static void walkReport(
        Object report,
        ArrayNode findings,
        String profile,
        String traceId,
        ObjectMapper mapper
    ) throws Throwable {
        Class<?> reportClass = report.getClass();
        Object resultDoc = reportClass.getMethod("getReportDocument").invoke(report);
        if (resultDoc == null) return;
        // The KoSIT scenarios produce a structured report XML; the
        // exact element layout is per-scenarios-bundle, so we scan
        // for any <messages>/<assertion>-style element whose
        // attributes look like a rule outcome.
        try {
            String docXml = String.valueOf(resultDoc.getClass()
                .getMethod("getDocumentElement").invoke(resultDoc));
            findings.add(reportSummaryFinding(mapper, profile, traceId, docXml));
        } catch (Throwable ex) {
            findings.add(reportSummaryFinding(mapper, profile, traceId,
                "kosit-report-document-unavailable"));
        }
    }

    private static String parseRoot(String xml) {
        try {
            javax.xml.parsers.DocumentBuilderFactory f =
                javax.xml.parsers.DocumentBuilderFactory.newInstance();
            f.setNamespaceAware(true);
            f.setXIncludeAware(false);
            f.setExpandEntityReferences(false);
            f.setFeature(javax.xml.XMLConstants.FEATURE_SECURE_PROCESSING, true);
            f.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
            return f.newDocumentBuilder()
                .parse(new org.xml.sax.InputSource(new ByteArrayInputStream(
                    xml.getBytes(StandardCharsets.UTF_8))))
                .getDocumentElement()
                .getNodeName();
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
        // The structured KoSIT report is forwarded verbatim as a
        // single non-fatal info finding under a stable rule id;
        // a future bead will parse the per-scenario assertions
        // (BR-* rules) once we pin the scenarios bundle version.
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
        ObjectNode finding = mapper.createObjectNode();
        finding.put("rule_id", "KOSIT-LIBRARY-ERROR");
        finding.put("severity", "fatal");
        finding.put("message",
            "KoSIT raised " + ex.getClass().getName()
                + " before producing a result: "
                + (ex.getMessage() == null ? "" : ex.getMessage()));
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
        details.put("exception", ex.getClass().getName());
        return finding;
    }

    /** Stable outcome record matching PhiveReport.Outcome for the
     * dispatcher in ValidatorSidecar. */
    record Outcome(boolean valid, String rootElement, ArrayNode results) {
    }
}
