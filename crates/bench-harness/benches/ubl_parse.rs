// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operation: `ubl-parse`.
//!
//! Measures `invoicekit-format-ubl` parsing on a synthetic UBL 2.1 Invoice of
//! at least 1 MiB. T-040's target is sub-100 ms p95 on the baseline runner.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_format_ubl::from_xml;

const TARGET_XML_BYTES: usize = 1_048_576;
const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CAC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
const UBL_CBC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";

fn synthetic_ubl_invoice_xml() -> String {
    let mut xml = String::with_capacity(TARGET_XML_BYTES + 8_192);
    xml.push_str(r#"<Invoice xmlns=""#);
    xml.push_str(UBL_INVOICE_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:cac=""#);
    xml.push_str(UBL_CAC_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:cbc=""#);
    xml.push_str(UBL_CBC_NAMESPACE_URI);
    xml.push_str(r#"">"#);
    xml.push_str("<cbc:ID>INV-BENCH-0001</cbc:ID>");
    xml.push_str("<cbc:UUID>doc-bench-0001</cbc:UUID>");
    xml.push_str("<cbc:IssueDate>2026-05-26</cbc:IssueDate>");
    xml.push_str("<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>");
    xml.push_str("<cbc:BuyerReference>tenant-bench</cbc:BuyerReference>");
    xml.push_str("<cbc:AccountingCost>trace-bench</cbc:AccountingCost>");
    write_party(
        &mut xml,
        "AccountingSupplierParty",
        "Supplier GmbH",
        "DE123456789",
    );
    write_party(
        &mut xml,
        "AccountingCustomerParty",
        "Customer BV",
        "NL123456789B01",
    );
    xml.push_str("<cac:LegalMonetaryTotal>");
    xml.push_str("<cbc:LineExtensionAmount currencyID=\"EUR\">100.00</cbc:LineExtensionAmount>");
    xml.push_str("<cbc:TaxExclusiveAmount currencyID=\"EUR\">100.00</cbc:TaxExclusiveAmount>");
    xml.push_str("<cbc:TaxInclusiveAmount currencyID=\"EUR\">119.00</cbc:TaxInclusiveAmount>");
    xml.push_str("<cbc:PayableAmount currencyID=\"EUR\">119.00</cbc:PayableAmount>");
    xml.push_str("</cac:LegalMonetaryTotal>");

    let mut line = 0_u32;
    while xml.len() < TARGET_XML_BYTES {
        write_invoice_line(&mut xml, line);
        line += 1;
    }

    xml.push_str("</Invoice>");
    xml
}

fn write_party(xml: &mut String, role: &str, name: &str, vat: &str) {
    xml.push_str("<cac:");
    xml.push_str(role);
    xml.push_str("><cac:Party><cac:PartyName><cbc:Name>");
    xml.push_str(name);
    xml.push_str("</cbc:Name></cac:PartyName><cac:PartyTaxScheme><cbc:CompanyID>");
    xml.push_str(vat);
    xml.push_str("</cbc:CompanyID><cac:TaxScheme><cbc:ID>vat</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme><cac:PostalAddress><cbc:StreetName>Main Street 1</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress></cac:Party></cac:");
    xml.push_str(role);
    xml.push('>');
}

fn write_invoice_line(xml: &mut String, line: u32) {
    xml.push_str("<cac:InvoiceLine>");
    xml.push_str("<cbc:ID>");
    xml.push_str(&line.to_string());
    xml.push_str("</cbc:ID>");
    xml.push_str("<cbc:InvoicedQuantity unitCode=\"EA\">1</cbc:InvoicedQuantity>");
    xml.push_str("<cbc:LineExtensionAmount currencyID=\"EUR\">100.00</cbc:LineExtensionAmount>");
    xml.push_str("<cac:Item><cbc:Name>Benchmark service line item ");
    xml.push_str(&line.to_string());
    xml.push_str("</cbc:Name></cac:Item>");
    xml.push_str(
        "<cac:Price><cbc:PriceAmount currencyID=\"EUR\">100.00</cbc:PriceAmount></cac:Price>",
    );
    xml.push_str("</cac:InvoiceLine>");
}

fn bench_ubl_parse(c: &mut Criterion) {
    let input = synthetic_ubl_invoice_xml();
    assert!(input.len() >= TARGET_XML_BYTES);
    assert!(from_xml(&input).is_ok());

    c.bench_function("ubl-parse", |b| {
        b.iter(|| {
            let (parsed, ledger) = from_xml(black_box(&input)).unwrap();
            black_box((parsed, ledger));
        });
    });
}

criterion_group!(benches, bench_ubl_parse);
criterion_main!(benches);
