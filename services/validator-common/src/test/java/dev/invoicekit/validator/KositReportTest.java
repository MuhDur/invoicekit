// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

/**
 * 7psv unit tests for the KoSIT reflection wrapper's
 * configuration-missing path. The full real-validation path runs
 * only when the {@code INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS} env
 * var is set to a downloaded validator-configuration-xrechnung
 * bundle's scenarios.xml — that integration test is covered by
 * the validator-smoke harness in CI's docker stage where the
 * Dockerfile downloads the bundle.
 *
 * <p>The cheap path (no env var → typed
 * KOSIT-SCENARIOS-MISSING finding) is what we lock down with a
 * pure JUnit test here so the wrapper's wire shape stays stable
 * even on developer machines without the bundle present.
 */
final class KositReportTest {
    private static final ObjectMapper MAPPER = new ObjectMapper();

    @Test
    void missingScenariosBundleEmitsTypedFinding() {
        // The KOSIT_SCENARIOS env var is intentionally not set in
        // the test JVM, so the wrapper takes the configuration-
        // missing fallback path. Even if a developer has the env
        // var pointed at a real bundle, the rule_id we assert on
        // ("KOSIT-SCENARIOS-MISSING") only appears in the
        // missing-config path — this test self-skips quietly.
        if (System.getenv(KositReport.SCENARIOS_ENV) != null) {
            return;
        }
        KositReport.Outcome outcome = KositReport.run(
            "<Invoice><ID>I-7psv</ID></Invoice>",
            "xrechnung",
            "trace-kosit-test",
            MAPPER);
        assertFalse(outcome.valid(),
            "kosit without a scenarios bundle must report invalid");
        assertEquals(1, outcome.results().size(),
            "missing-bundle path must produce exactly one finding");
        String ruleId = outcome.results().get(0).path("rule_id").asText();
        assertEquals("KOSIT-SCENARIOS-MISSING", ruleId,
            "missing-bundle finding rule_id must be KOSIT-SCENARIOS-MISSING");
        assertTrue(outcome.results().get(0).path("suggested_fix").has("summary"),
            "missing-bundle finding must carry a suggested_fix.summary");
        assertEquals("KoSIT validator 1.6.2",
            outcome.results().get(0).path("citation").path("source").asText(),
            "missing-bundle finding citation.source must be the canonical KoSIT label");
    }
}
