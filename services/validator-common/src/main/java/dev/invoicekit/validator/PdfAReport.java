// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Method;

/**
 * T-052 thin wrapper around the veraPDF library so the wire shape
 * stays stable across library upgrades and so {@code ValidatorSidecar}
 * compiles cleanly on Maven profiles where {@code verapdf-library}
 * is not on the classpath.
 *
 * <p>This class uses reflection to invoke {@code
 * org.verapdf.pdfa.Foundries#defaultInstance()} so the build doesn't
 * fail to compile on the {@code kosit}, {@code phive}, and {@code
 * saxon} profiles. At runtime, the {@code verapdf} profile is the
 * only one where this code path is reached.
 *
 * <p>The returned JSON shape is documented in
 * {@code crates/render-verify/src/verapdf.rs} ({@code PdfAReport}
 * type). Wire-format owner = the Rust adapter; this class only
 * produces it.
 */
final class PdfAReport {
    private PdfAReport() {
    }

    static ObjectNode run(byte[] pdfBytes, String flavour, String traceId, ObjectMapper mapper) {
        ObjectNode report = mapper.createObjectNode();
        report.put("flavour", flavour);
        report.put("trace_id", traceId);

        Object foundry;
        try {
            Class<?> foundries = Class.forName("org.verapdf.pdfa.Foundries");
            Method defaultInstance = foundries.getMethod("defaultInstance");
            foundry = defaultInstance.invoke(null);
        } catch (Throwable ex) {
            report.put("conformant", false);
            report.put("error_class", ex.getClass().getName());
            report.put("error_message", "veraPDF Foundries.defaultInstance() unavailable: "
                + (ex.getMessage() == null ? "" : ex.getMessage()));
            ArrayNode failures = report.putArray("failures");
            ObjectNode finding = failures.addObject();
            finding.put("rule_id", "VERAPDF-FOUNDRY-INIT");
            finding.put("severity", "fatal");
            finding.put("message",
                "veraPDF library is not on the classpath; build the validator-verapdf "
                + "image to enable this backend");
            return report;
        }

        try (ByteArrayInputStream pdfStream = new ByteArrayInputStream(pdfBytes)) {
            // The verapdf-library validator chain is:
            //   PDFAParser parser = foundry.createParser(stream, flavour);
            //   PDFAValidator validator = foundry.createValidator(...);
            //   ValidationResult result = validator.validate(parser);
            //
            // We call this via reflection because the compile-time
            // classpath on non-verapdf profiles doesn't carry these
            // types. The string names below are the load-bearing
            // contract with the verapdf-library version pinned in
            // pom.xml (1.27.1); a major-version bump that renames
            // the types breaks this class loudly with a typed
            // failure, not a silent miscount.
            Class<?> flavourClass = Class.forName("org.verapdf.pdfa.flavours.PDFAFlavour");
            Object flavourEnum = flavourClass
                .getMethod("byFlavourId", String.class)
                .invoke(null, flavour.toLowerCase());

            Method createParser = foundry.getClass().getMethod("createParser", java.io.InputStream.class, flavourClass);
            Object parser = createParser.invoke(foundry, pdfStream, flavourEnum);

            Method createValidator = foundry.getClass().getMethod("createValidator", flavourClass, boolean.class);
            Object validator = createValidator.invoke(foundry, flavourEnum, false);

            Class<?> parserClass = parser.getClass();
            Method validate = validator.getClass().getMethod("validate", parserClass);
            Object validationResult = validate.invoke(validator, parser);

            boolean isCompliant = (boolean) validationResult.getClass()
                .getMethod("isCompliant").invoke(validationResult);
            report.put("conformant", isCompliant);

            ArrayNode failures = report.putArray("failures");
            Iterable<?> testAssertions = (Iterable<?>) validationResult.getClass()
                .getMethod("getTestAssertions").invoke(validationResult);
            for (Object assertion : testAssertions) {
                Class<?> aclass = assertion.getClass();
                Object status = aclass.getMethod("getStatus").invoke(assertion);
                if ("PASSED".equals(String.valueOf(status))) {
                    continue;
                }
                ObjectNode finding = failures.addObject();
                Object ruleId = aclass.getMethod("getRuleId").invoke(assertion);
                finding.put("rule_id", String.valueOf(ruleId));
                finding.put("severity", "violation");
                Object message = aclass.getMethod("getMessage").invoke(assertion);
                finding.put("message", message == null ? "" : String.valueOf(message));
                Object location = aclass.getMethod("getLocation").invoke(assertion);
                if (location != null) {
                    finding.put("location", String.valueOf(location));
                }
            }
            return report;
        } catch (Throwable ex) {
            report.put("conformant", false);
            report.put("error_class", ex.getClass().getName());
            report.put("error_message", ex.getMessage() == null ? "" : ex.getMessage());
            return report;
        }
    }
}
