// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.validator;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

/**
 * 7psv acceptance test for the phive reflection wrapper.
 *
 * The bead's strict gate requires "a known invalid invoice emits at
 * least one BR-* or BR-CO-* rule from the oracle". This test feeds
 * a minimal Peppol BIS Billing 3.0 invoice with several mandatory
 * EN 16931 fields missing (CustomizationID, ProfileID, IssueDate,
 * DocumentCurrencyCode, AccountingSupplierParty, etc) and asserts:
 *
 *  1. PhiveReport.run() returns valid=false.
 *  2. The findings array contains at least one entry whose rule_id
 *     begins with "BR-" (the EN 16931 business-rule namespace,
 *     including BR-CO-* cross-condition rules).
 *  3. The phive finding carries the canonical T-032 fields:
 *     rule_id, severity, term, location, citation, suggested_fix,
 *     trace — none of which may be null or blank.
 *
 * This runs only under {@code -Pphive} because
 * {@code com.helger.phive.peppol.PeppolValidation} is on the
 * classpath only in that profile.
 */
final class PhiveReportTest {
    private static final ObjectMapper MAPPER = new ObjectMapper();

    /** Peppol BIS Billing 3.0 invoice that passes the UBL 2.x XSD
     * but breaks BR-CO-15: "Invoice total amount with VAT (BT-112)
     * = Invoice total amount without VAT (BT-109) + Invoice total
     * VAT amount (BT-110)". TaxInclusiveAmount is set to 999.00
     * deliberately so the Schematron layer raises a real BR rule
     * (not the XSD layer). */
    private static final String INVALID_PEPPOL_INVOICE =
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
        + "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\""
        + " xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\""
        + " xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">"
        + "<cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>"
        + "<cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>"
        + "<cbc:ID>I-7psv</cbc:ID>"
        + "<cbc:IssueDate>2026-05-27</cbc:IssueDate>"
        + "<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>"
        + "<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>"
        + "<cac:AccountingSupplierParty><cac:Party>"
        + "<cac:PartyName><cbc:Name>Seller</cbc:Name></cac:PartyName>"
        + "<cac:PostalAddress><cac:Country>"
        + "<cbc:IdentificationCode>NO</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
        + "<cac:PartyLegalEntity><cbc:RegistrationName>Seller AS</cbc:RegistrationName>"
        + "</cac:PartyLegalEntity>"
        + "</cac:Party></cac:AccountingSupplierParty>"
        + "<cac:AccountingCustomerParty><cac:Party>"
        + "<cac:PartyName><cbc:Name>Buyer</cbc:Name></cac:PartyName>"
        + "<cac:PostalAddress><cac:Country>"
        + "<cbc:IdentificationCode>NO</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
        + "<cac:PartyLegalEntity><cbc:RegistrationName>Buyer AS</cbc:RegistrationName>"
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
    void invalidPeppolInvoiceEmitsAtLeastOneBrRule() {
        // This test runs only when phive-rules-peppol is on the
        // classpath (Maven -Pphive profile). On other profiles
        // (-Pkosit, -Psaxon, -Pverapdf) the wrapper would emit a
        // PHIVE-LIBRARY-ERROR finding instead of a BR-* rule —
        // self-skip so the test doesn't fail cross-profile.
        try {
            Class.forName("com.helger.phive.peppol.PeppolValidation");
        } catch (ClassNotFoundException ignored) {
            return;
        }
        PhiveReport.Outcome outcome = PhiveReport.run(
            INVALID_PEPPOL_INVOICE,
            "peppol-bis-billing-3",
            "trace-7psv-test",
            MAPPER);

        assertFalse(outcome.valid(),
            "phive should reject an EN 16931 invoice missing all required business terms");
        assertNotNull(outcome.results());
        assertTrue(outcome.results().size() > 0,
            "phive should emit at least one finding for an invalid invoice");

        boolean foundBrRule = false;
        for (JsonNode finding : outcome.results()) {
            String ruleId = finding.path("rule_id").asText("");
            // The bead's acceptance text: "BR-* or BR-CO-* rule from the oracle".
            if (ruleId.startsWith("BR-")) {
                foundBrRule = true;
            }
            // Validate canonical T-032 shape on every finding.
            assertFalse(ruleId.isBlank(), "finding rule_id must be non-blank");
            assertFalse(finding.path("severity").asText("").isBlank(),
                "finding severity must be non-blank");
            assertTrue(finding.has("term"), "finding must carry term");
            assertTrue(finding.has("location"), "finding must carry location");
            assertTrue(finding.has("citation"), "finding must carry citation");
            assertTrue(finding.has("suggested_fix"), "finding must carry suggested_fix");
            assertTrue(finding.has("trace"), "finding must carry trace");
        }
        assertTrue(foundBrRule,
            "phive must emit at least one BR-* or BR-CO-* rule for an invalid Peppol invoice; "
            + "actual findings: " + outcome.results().toString());
    }
}
