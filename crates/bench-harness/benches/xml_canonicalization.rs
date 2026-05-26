// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operation: `xml-canonicalization`.
//!
//! Measures InvoiceKit's XML canonicalization profile on a synthetic UBL-sized
//! invoice body of at least 1 MiB. The T-019 budget target is sub-50 ms p95 on
//! the baseline runner; the CI budget tooling also tracks regression against
//! the rolling `main` baseline.

// criterion_group! / criterion_main! expand to public items without rustdoc;
// the workspace's missing_docs warning would otherwise fail clippy here.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_canonical::canonicalize_xml;

const TARGET_XML_BYTES: usize = 1_048_576;
const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CAC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
const UBL_CBC_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";

fn synthetic_ubl_invoice_xml() -> String {
    let mut xml = String::with_capacity(TARGET_XML_BYTES + 4_096);
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push_str(r#"<n:Invoice xmlns:n=""#);
    xml.push_str(UBL_INVOICE_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:a=""#);
    xml.push_str(UBL_CAC_NAMESPACE_URI);
    xml.push_str(r#"" xmlns:b=""#);
    xml.push_str(UBL_CBC_NAMESPACE_URI);
    xml.push_str(r#"">"#);
    xml.push_str("<b:ID>INV-BENCH-0001</b:ID>");
    xml.push_str("<b:IssueDate>2026-05-26</b:IssueDate>");
    xml.push_str("<b:DocumentCurrencyCode>EUR</b:DocumentCurrencyCode>");

    let mut line = 0_u32;
    while xml.len() < TARGET_XML_BYTES {
        write_invoice_line(&mut xml, line);
        line += 1;
    }

    xml.push_str("</n:Invoice>");
    xml
}

fn write_invoice_line(xml: &mut String, line: u32) {
    xml.push_str("<a:InvoiceLine>");
    xml.push_str("<b:ID>");
    xml.push_str(&line.to_string());
    xml.push_str("</b:ID>");
    xml.push_str("<b:InvoicedQuantity unitCode=\"EA\">1</b:InvoicedQuantity>");
    xml.push_str("<b:LineExtensionAmount currencyID=\"EUR\">100.00</b:LineExtensionAmount>");
    xml.push_str("<a:Item><b:Name>Benchmark service line item ");
    xml.push_str(&line.to_string());
    xml.push_str("</b:Name></a:Item>");
    xml.push_str("<a:Price><b:PriceAmount currencyID=\"EUR\">100.00</b:PriceAmount></a:Price>");
    xml.push_str("</a:InvoiceLine>");
}

fn bench_xml_canonicalization(c: &mut Criterion) {
    let input = synthetic_ubl_invoice_xml();
    assert!(input.len() >= TARGET_XML_BYTES);
    let canonical = canonicalize_xml(&input).unwrap();
    assert_eq!(canonicalize_xml(&canonical).unwrap(), canonical);

    c.bench_function("xml-canonicalization", |b| {
        b.iter(|| {
            let canonical = canonicalize_xml(black_box(&input)).unwrap();
            black_box(canonical);
        });
    });
}

criterion_group!(benches, bench_xml_canonicalization);
criterion_main!(benches);
