// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operation: `cii-parse`.
//!
//! Measures `invoicekit-format-cii` parsing on a synthetic UN/CEFACT CII D16B
//! invoice of at least 1 MiB. T-041's target is sub-100 ms p95 on the baseline
//! runner.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_format_cii::from_xml;

const TARGET_XML_BYTES: usize = 1_048_576;
const CII_RSM_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
const CII_RAM_NAMESPACE_URI: &str =
    "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
const CII_UDT_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100";

fn synthetic_cii_invoice_xml() -> String {
    let mut xml = String::with_capacity(TARGET_XML_BYTES + 8_192);
    xml.push_str(r#"<rsm:CrossIndustryInvoice xmlns:rsm=""#);
    xml.push_str(CII_RSM_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:ram=""#);
    xml.push_str(CII_RAM_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:udt=""#);
    xml.push_str(CII_UDT_NAMESPACE_URI);
    xml.push_str(r#"">"#);
    xml.push_str("<rsm:ExchangedDocumentContext>");
    xml.push_str(
        "<ram:BusinessProcessSpecifiedDocumentContextParameter><ram:ID>trace-bench</ram:ID></ram:BusinessProcessSpecifiedDocumentContextParameter>",
    );
    xml.push_str("</rsm:ExchangedDocumentContext>");
    xml.push_str("<rsm:ExchangedDocument>");
    xml.push_str("<ram:ID>CII-BENCH-0001</ram:ID>");
    xml.push_str("<ram:TypeCode>380</ram:TypeCode>");
    xml.push_str("<ram:IssueDateTime><udt:DateTimeString format=\"102\">20260526</udt:DateTimeString></ram:IssueDateTime>");
    xml.push_str("</rsm:ExchangedDocument>");
    xml.push_str("<rsm:SupplyChainTradeTransaction>");

    let mut line = 0_u32;
    while xml.len() < TARGET_XML_BYTES {
        write_line(&mut xml, line);
        line += 1;
    }

    xml.push_str("<ram:ApplicableHeaderTradeAgreement>");
    xml.push_str("<ram:BuyerReference>tenant-bench</ram:BuyerReference>");
    write_party(&mut xml, "SellerTradeParty", "Supplier GmbH", "DE123456789");
    write_party(&mut xml, "BuyerTradeParty", "Customer BV", "NL123456789B01");
    xml.push_str("</ram:ApplicableHeaderTradeAgreement>");
    xml.push_str("<ram:ApplicableHeaderTradeSettlement>");
    xml.push_str("<ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode>");
    xml.push_str("<ram:ApplicableTradeTax><ram:CalculatedAmount>19.00</ram:CalculatedAmount><ram:TypeCode>VAT</ram:TypeCode><ram:BasisAmount>100.00</ram:BasisAmount><ram:CategoryCode>S</ram:CategoryCode><ram:RateApplicablePercent>19.00</ram:RateApplicablePercent></ram:ApplicableTradeTax>");
    xml.push_str("<ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
    xml.push_str("<ram:LineTotalAmount>100.00</ram:LineTotalAmount>");
    xml.push_str("<ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount>");
    xml.push_str("<ram:GrandTotalAmount>119.00</ram:GrandTotalAmount>");
    xml.push_str("<ram:DuePayableAmount>119.00</ram:DuePayableAmount>");
    xml.push_str("</ram:SpecifiedTradeSettlementHeaderMonetarySummation>");
    xml.push_str("</ram:ApplicableHeaderTradeSettlement>");
    xml.push_str("</rsm:SupplyChainTradeTransaction>");
    xml.push_str("</rsm:CrossIndustryInvoice>");
    xml
}

fn write_party(xml: &mut String, role: &str, name: &str, vat: &str) {
    xml.push_str("<ram:");
    xml.push_str(role);
    xml.push_str("><ram:Name>");
    xml.push_str(name);
    xml.push_str("</ram:Name><ram:PostalTradeAddress><ram:PostcodeCode>10115</ram:PostcodeCode><ram:LineOne>Main Street 1</ram:LineOne><ram:CityName>Berlin</ram:CityName><ram:CountryID>DE</ram:CountryID></ram:PostalTradeAddress><ram:SpecifiedTaxRegistration><ram:ID schemeID=\"VA\">");
    xml.push_str(vat);
    xml.push_str("</ram:ID></ram:SpecifiedTaxRegistration></ram:");
    xml.push_str(role);
    xml.push('>');
}

fn write_line(xml: &mut String, line: u32) {
    xml.push_str("<ram:IncludedSupplyChainTradeLineItem>");
    xml.push_str("<ram:AssociatedDocumentLineDocument><ram:LineID>");
    xml.push_str(&line.to_string());
    xml.push_str("</ram:LineID></ram:AssociatedDocumentLineDocument>");
    xml.push_str("<ram:SpecifiedTradeProduct><ram:Name>Benchmark service line item ");
    xml.push_str(&line.to_string());
    xml.push_str("</ram:Name></ram:SpecifiedTradeProduct>");
    xml.push_str("<ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice><ram:ChargeAmount>100.00</ram:ChargeAmount></ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement>");
    xml.push_str("<ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode=\"C62\">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery>");
    xml.push_str("<ram:SpecifiedLineTradeSettlement><ram:ApplicableTradeTax><ram:TypeCode>VAT</ram:TypeCode><ram:CategoryCode>S</ram:CategoryCode></ram:ApplicableTradeTax><ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation></ram:SpecifiedLineTradeSettlement>");
    xml.push_str("</ram:IncludedSupplyChainTradeLineItem>");
}

fn bench_cii_parse(c: &mut Criterion) {
    let input = synthetic_cii_invoice_xml();
    assert!(input.len() >= TARGET_XML_BYTES);
    assert!(from_xml(&input).is_ok());

    c.bench_function("cii-parse", |b| {
        b.iter(|| {
            let (parsed, ledger) = from_xml(black_box(&input)).unwrap();
            black_box((parsed, ledger));
        });
    });
}

criterion_group!(benches, bench_cii_parse);
criterion_main!(benches);
