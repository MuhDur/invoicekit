// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import org.junit.jupiter.api.Test;

/**
 * 7psv unit tests for the KoSIT reflection wrapper's
 * configuration-missing path. The full real-validation path runs
 * only when the {@code INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS} env
 * var is set to a downloaded validator-configuration-xrechnung
 * bundle's scenarios.xml.
 *
 * <p>The cheap path (no env var → typed
 * KOSIT-SCENARIOS-MISSING finding) is what we lock down with a
 * pure JUnit test here so the wrapper's wire shape stays stable
 * even on developer machines without the bundle present.
 */
final class KositReportTest {
    private static final ObjectMapper MAPPER = new ObjectMapper();

    /** XRechnung-flavoured UBL invoice that passes basic XML
     * parsing but breaks BR-CO-15 by setting BT-112
     * (TaxInclusiveAmount) to a value that is not BT-109 + BT-110.
     * The integration test below feeds this to the real KoSIT
     * scenarios bundle when it is available locally. */
    private static final String INVALID_XRECHNUNG_INVOICE =
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
        + "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\""
        + " xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\""
        + " xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">"
        + "<cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#"
        + "urn:xoev-de:kosit:standard:xrechnung_3.0</cbc:CustomizationID>"
        + "<cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>"
        + "<cbc:ID>I-kosit-br</cbc:ID>"
        + "<cbc:IssueDate>2026-05-27</cbc:IssueDate>"
        + "<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>"
        + "<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>"
        + "<cac:AccountingSupplierParty><cac:Party>"
        + "<cac:PartyName><cbc:Name>Seller</cbc:Name></cac:PartyName>"
        + "<cac:PostalAddress><cac:Country>"
        + "<cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
        + "<cac:PartyLegalEntity><cbc:RegistrationName>Seller GmbH</cbc:RegistrationName>"
        + "</cac:PartyLegalEntity>"
        + "</cac:Party></cac:AccountingSupplierParty>"
        + "<cac:AccountingCustomerParty><cac:Party>"
        + "<cac:PartyName><cbc:Name>Buyer</cbc:Name></cac:PartyName>"
        + "<cac:PostalAddress><cac:Country>"
        + "<cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
        + "<cac:PartyLegalEntity><cbc:RegistrationName>Buyer GmbH</cbc:RegistrationName>"
        + "</cac:PartyLegalEntity>"
        + "</cac:Party></cac:AccountingCustomerParty>"
        + "<cac:TaxTotal><cbc:TaxAmount currencyID=\"EUR\">0.00</cbc:TaxAmount></cac:TaxTotal>"
        + "<cac:LegalMonetaryTotal>"
        + "<cbc:LineExtensionAmount currencyID=\"EUR\">100.00</cbc:LineExtensionAmount>"
        + "<cbc:TaxExclusiveAmount currencyID=\"EUR\">100.00</cbc:TaxExclusiveAmount>"
        + "<cbc:TaxInclusiveAmount currencyID=\"EUR\">999.00</cbc:TaxInclusiveAmount>"
        + "<cbc:PayableAmount currencyID=\"EUR\">999.00</cbc:PayableAmount>"
        + "</cac:LegalMonetaryTotal>"
        + "<cac:InvoiceLine>"
        + "<cbc:ID>1</cbc:ID>"
        + "<cbc:InvoicedQuantity unitCode=\"EA\">1</cbc:InvoicedQuantity>"
        + "<cbc:LineExtensionAmount currencyID=\"EUR\">100.00</cbc:LineExtensionAmount>"
        + "<cac:Item><cbc:Name>Widget</cbc:Name>"
        + "<cac:ClassifiedTaxCategory>"
        + "<cbc:ID>S</cbc:ID><cbc:Percent>0</cbc:Percent>"
        + "<cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>"
        + "</cac:ClassifiedTaxCategory>"
        + "</cac:Item>"
        + "<cac:Price><cbc:PriceAmount currencyID=\"EUR\">100.00</cbc:PriceAmount></cac:Price>"
        + "</cac:InvoiceLine>"
        + "</Invoice>";

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

    @Test
    void reportXmlEmitsNativeBrFinding() {
        String reportXml = """
            <rep:report xmlns:rep="urn:example:kosit-report"
              xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
              <svrl:failed-assert id="BR-CO-15" flag="fatal"
                location="/ubl:Invoice/cac:LegalMonetaryTotal">
                <svrl:text>[BR-CO-15] Invoice total amount with VAT shall equal
                  invoice total amount without VAT plus VAT total amount.</svrl:text>
              </svrl:failed-assert>
            </rep:report>
            """;

        var findings = KositReport.reportFindingsFromXmlForTest(
            reportXml, "xrechnung", "trace-kosit-rule", MAPPER);

        assertEquals(1, findings.size(),
            "one KoSIT failed assertion must produce one ValidationResult");
        var finding = findings.get(0);
        assertEquals("BR-CO-15", finding.path("rule_id").asText());
        assertEquals("fatal", finding.path("severity").asText());
        assertEquals("business_term", finding.path("term").path("kind").asText());
        assertEquals("/ubl:Invoice/cac:LegalMonetaryTotal",
            finding.path("location").path("expression").asText());
        assertEquals("KoSIT validator 1.6.2",
            finding.path("citation").path("source").asText());
        assertEquals("BR-CO-15", finding.path("citation").path("section").asText());
        assertEquals("jvm:kosit", finding.path("trace").path("backend").asText());
        assertEquals("trace-kosit-rule", finding.path("trace").path("trace_id").asText());
    }

    @Test
    void invalidXrechnungEmitsNativeBrFindingWhenScenariosAvailable() {
        if (System.getenv(KositReport.SCENARIOS_ENV) == null) {
            return;
        }
        try {
            Class.forName("de.kosit.validationtool.api.Check");
        } catch (ClassNotFoundException ignored) {
            return;
        }

        KositReport.Outcome outcome = KositReport.run(
            INVALID_XRECHNUNG_INVOICE,
            "xrechnung",
            "trace-kosit-invalid-xrechnung",
            MAPPER);

        assertFalse(outcome.valid(),
            "KoSIT should reject the invalid XRechnung fixture");
        assertNativeBrFinding(outcome.results());
    }

    private static void assertNativeBrFinding(ArrayNode findings) {
        assertTrue(findings.size() > 0,
            "KoSIT should emit at least one finding for an invalid XRechnung fixture");
        boolean foundNativeBrRule = false;
        for (JsonNode finding : findings) {
            String ruleId = finding.path("rule_id").asText("");
            if (ruleId.startsWith("BR-")) {
                foundNativeBrRule = true;
            }
            assertFalse(ruleId.isBlank(), "finding rule_id must be non-blank");
            assertFalse(finding.path("severity").asText("").isBlank(),
                "finding severity must be non-blank");
            assertTrue(finding.has("term"), "finding must carry term");
            assertTrue(finding.has("location"), "finding must carry location");
            assertTrue(finding.has("citation"), "finding must carry citation");
            assertTrue(finding.has("suggested_fix"), "finding must carry suggested_fix");
            assertTrue(finding.has("trace"), "finding must carry trace");
        }
        assertTrue(foundNativeBrRule,
            "KoSIT must emit at least one native BR-* rule; actual findings: "
            + findings.toString());
    }
}
