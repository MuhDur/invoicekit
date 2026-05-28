// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operations: `validate-ubl-small`, `validate-ubl`, and `validate-cii`.
//!
//! Exercises the EN 16931 BR/BR-CO rule engine in
//! `invoicekit-validate-ubl-cii` over realistic invoices. The validate verb is
//! the headline free-core operation and was previously unbenchmarked.
//!
//! - `validate-ubl-small` is a single-line UBL invoice. It captures the fixed,
//!   document-independent cost of the rule engine — the floor the optimization
//!   phase drives down.
//! - `validate-ubl` is a 200-line UBL invoice. The gap to the small case
//!   isolates the per-line `O(rules x lines)` scaling the four uncached helper
//!   functions contribute.
//! - `validate-cii` is a 200-line UN/CEFACT CII invoice. It reaches the same
//!   engine through the recursive CII parser, a separate hot path.
//!
//! The large cases intentionally repeat one line shape, so document-level
//! BR-CO total rules report findings. That does not weaken the measurement:
//! `validate_xml` runs the full rule set regardless of whether the invoice is
//! conformant, which is exactly the work being timed.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use std::fmt::Write as _;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_validate_ubl_cii::validate_xml;

const LARGE_LINE_COUNT: usize = 200;

const UBL_SUPPLIER_PARTY: &str = r"<cac:AccountingSupplierParty><cac:Party><cac:PartyIdentification><cbc:ID>SUPPLIER-1</cbc:ID></cac:PartyIdentification><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main Street 1</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyLegalEntity><cbc:RegistrationName>Supplier GmbH</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingSupplierParty>";

const UBL_CUSTOMER_PARTY: &str = r"<cac:AccountingCustomerParty><cac:Party><cac:PartyName><cbc:Name>Customer BV</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main Street 2</cbc:StreetName><cbc:CityName>Amsterdam</cbc:CityName><cbc:PostalZone>1000AA</cbc:PostalZone><cac:Country><cbc:IdentificationCode>NL</cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyLegalEntity><cbc:RegistrationName>Customer BV</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingCustomerParty>";

fn ubl_invoice(lines: usize) -> String {
    let mut s = String::with_capacity(2_048 + lines * 320);
    s.push_str(r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">"#);
    s.push_str("<cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>");
    s.push_str("<cbc:ID>INV-BENCH</cbc:ID>");
    s.push_str("<cbc:IssueDate>2026-05-27</cbc:IssueDate>");
    s.push_str("<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>");
    s.push_str("<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>");
    s.push_str(UBL_SUPPLIER_PARTY);
    s.push_str(UBL_CUSTOMER_PARTY);
    for i in 0..lines {
        let _ = write!(
            s,
            r#"<cac:InvoiceLine><cbc:ID>{i}</cbc:ID><cbc:InvoicedQuantity unitCode="C62">1</cbc:InvoicedQuantity><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cac:Item><cbc:Name>Implementation service</cbc:Name><cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:ClassifiedTaxCategory></cac:Item><cac:Price><cbc:PriceAmount>100.00</cbc:PriceAmount></cac:Price></cac:InvoiceLine>"#,
        );
    }
    s.push_str(r#"<cac:TaxTotal><cbc:TaxAmount currencyID="EUR">19.00</cbc:TaxAmount><cac:TaxSubtotal><cbc:TaxableAmount>100.00</cbc:TaxableAmount><cbc:TaxAmount>19.00</cbc:TaxAmount><cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:TaxSubtotal></cac:TaxTotal>"#);
    s.push_str(r"<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount>100.00</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount><cbc:PayableAmount>119.00</cbc:PayableAmount></cac:LegalMonetaryTotal>");
    s.push_str("</ubl:Invoice>");
    s
}

fn cii_invoice(lines: usize) -> String {
    let mut s = String::with_capacity(2_048 + lines * 360);
    s.push_str(r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100" xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100" xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100">"#);
    s.push_str("<rsm:ExchangedDocumentContext><ram:GuidelineSpecifiedDocumentContextParameter><ram:ID>urn:cen.eu:en16931:2017</ram:ID></ram:GuidelineSpecifiedDocumentContextParameter></rsm:ExchangedDocumentContext>");
    s.push_str(r#"<rsm:ExchangedDocument><ram:ID>CII-BENCH-0001</ram:ID><ram:TypeCode>380</ram:TypeCode><ram:IssueDateTime><udt:DateTimeString format="102">20260527</udt:DateTimeString></ram:IssueDateTime></rsm:ExchangedDocument>"#);
    s.push_str("<rsm:SupplyChainTradeTransaction>");
    for i in 0..lines {
        let _ = write!(
            s,
            r#"<ram:IncludedSupplyChainTradeLineItem><ram:AssociatedDocumentLineDocument><ram:LineID>{i}</ram:LineID></ram:AssociatedDocumentLineDocument><ram:SpecifiedTradeProduct><ram:Name>Implementation service</ram:Name></ram:SpecifiedTradeProduct><ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice><ram:ChargeAmount>100.00</ram:ChargeAmount></ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement><ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery><ram:SpecifiedLineTradeSettlement><ram:ApplicableTradeTax><ram:TypeCode>VAT</ram:TypeCode><ram:CategoryCode>S</ram:CategoryCode><ram:RateApplicablePercent>19.00</ram:RateApplicablePercent></ram:ApplicableTradeTax><ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation></ram:SpecifiedLineTradeSettlement></ram:IncludedSupplyChainTradeLineItem>"#,
        );
    }
    s.push_str(r#"<ram:ApplicableHeaderTradeAgreement><ram:BuyerReference>tenant-bench</ram:BuyerReference><ram:SellerTradeParty><ram:Name>Supplier GmbH</ram:Name><ram:PostalTradeAddress><ram:PostcodeCode>10115</ram:PostcodeCode><ram:LineOne>Main Street 1</ram:LineOne><ram:CityName>Berlin</ram:CityName><ram:CountryID>DE</ram:CountryID></ram:PostalTradeAddress><ram:SpecifiedTaxRegistration><ram:ID schemeID="VA">DE123456789</ram:ID></ram:SpecifiedTaxRegistration></ram:SellerTradeParty><ram:BuyerTradeParty><ram:Name>Customer BV</ram:Name><ram:PostalTradeAddress><ram:PostcodeCode>1000AA</ram:PostcodeCode><ram:LineOne>Main Street 2</ram:LineOne><ram:CityName>Amsterdam</ram:CityName><ram:CountryID>NL</ram:CountryID></ram:PostalTradeAddress></ram:BuyerTradeParty></ram:ApplicableHeaderTradeAgreement>"#);
    s.push_str(r"<ram:ApplicableHeaderTradeSettlement><ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode><ram:ApplicableTradeTax><ram:CalculatedAmount>19.00</ram:CalculatedAmount><ram:TypeCode>VAT</ram:TypeCode><ram:BasisAmount>100.00</ram:BasisAmount><ram:CategoryCode>S</ram:CategoryCode><ram:RateApplicablePercent>19.00</ram:RateApplicablePercent></ram:ApplicableTradeTax><ram:SpecifiedTradeSettlementHeaderMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount><ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount><ram:GrandTotalAmount>119.00</ram:GrandTotalAmount><ram:DuePayableAmount>119.00</ram:DuePayableAmount></ram:SpecifiedTradeSettlementHeaderMonetarySummation></ram:ApplicableHeaderTradeSettlement>");
    s.push_str("</rsm:SupplyChainTradeTransaction></rsm:CrossIndustryInvoice>");
    s
}

fn bench_validate(c: &mut Criterion) {
    let ubl_small = ubl_invoice(1);
    let ubl_large = ubl_invoice(LARGE_LINE_COUNT);
    let cii_large = cii_invoice(LARGE_LINE_COUNT);

    // Sanity: the validator must accept (parse) every input. It returns Ok with
    // findings for non-conformant invoices and only errors on unparseable XML.
    assert!(validate_xml(&ubl_small).is_ok());
    assert!(validate_xml(&ubl_large).is_ok());
    assert!(validate_xml(&cii_large).is_ok());

    c.bench_function("validate-ubl-small", |b| {
        b.iter(|| {
            let report = validate_xml(black_box(&ubl_small)).unwrap();
            black_box(report);
        });
    });
    c.bench_function("validate-ubl", |b| {
        b.iter(|| {
            let report = validate_xml(black_box(&ubl_large)).unwrap();
            black_box(report);
        });
    });
    c.bench_function("validate-cii", |b| {
        b.iter(|| {
            let report = validate_xml(black_box(&cii_large)).unwrap();
            black_box(report);
        });
    });
}

criterion_group!(benches, bench_validate);
criterion_main!(benches);
