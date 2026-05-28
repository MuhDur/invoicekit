// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operations: `intake-factur-x-extract` and `format-detect`.
//!
//! Covers the "drop a PDF in" intake verb. A Factur-X / ZUGFeRD PDF carries its
//! invoice as an embedded CII XML attachment; intake must parse the PDF object
//! graph, locate the attachment via the `/AF` + `/Names` entries, and inflate
//! the embedded stream (`crates/intake-pdf/src/factur_x.rs:64`).
//!
//! - `intake-factur-x-extract` times `extract_factur_x_xml` over a realistic
//!   Factur-X PDF whose embedded CII attachment carries ~50 lines.
//! - `format-detect` times the cheap byte sniff every intake starts with, so we
//!   can tell sniff cost apart from extraction cost.
//!
//! The fixture PDF is built inline here (mirroring the PDF/A-3 attachment shape
//! that `crates/intake-pdf/src/factur_x.rs` tests construct) because that test
//! helper is private to the crate.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use std::fmt::Write as _;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_format_detect::{detect_format, FormatId};
use invoicekit_intake_pdf::extract_factur_x_xml;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Object, Stream};

const ATTACHMENT_NAME: &str = "factur-x.xml";
const ATTACHMENT_LINE_COUNT: usize = 50;

fn embedded_cii_xml(lines: usize) -> Vec<u8> {
    let mut s = String::with_capacity(512 + lines * 200);
    s.push_str(r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100" xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100">"#);
    s.push_str("<rsm:ExchangedDocument><ram:ID>FX-BENCH-0001</ram:ID><ram:TypeCode>380</ram:TypeCode></rsm:ExchangedDocument>");
    s.push_str("<rsm:SupplyChainTradeTransaction>");
    for i in 0..lines {
        let _ = write!(
            s,
            r"<ram:IncludedSupplyChainTradeLineItem><ram:AssociatedDocumentLineDocument><ram:LineID>{i}</ram:LineID></ram:AssociatedDocumentLineDocument><ram:SpecifiedTradeProduct><ram:Name>Line item {i}</ram:Name></ram:SpecifiedTradeProduct></ram:IncludedSupplyChainTradeLineItem>",
        );
    }
    s.push_str("</rsm:SupplyChainTradeTransaction></rsm:CrossIndustryInvoice>");
    s.into_bytes()
}

/// Build a minimal but valid PDF mirroring the PDF/A-3 Factur-X attachment
/// shape: a Catalog with `/AF` and `/Names/EmbeddedFiles`, a `Filespec`, and an
/// `EmbeddedFile` stream carrying the CII XML.
fn build_factur_x_pdf(attachment_name: &str, xml: &[u8]) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");

    let xml_stream_id = doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile", "Subtype" => Object::Name(b"text/xml".to_vec()) },
        xml.to_vec(),
    ));
    let filespec_id = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        "UF" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
        "AFRelationship" => Object::Name(b"Alternative".to_vec()),
        "EF" => dictionary! {
            "F" => xml_stream_id,
            "UF" => xml_stream_id,
        },
    });

    let content_id = doc.add_object(Stream::new(
        Dictionary::new(),
        Content {
            operations: vec![Operation::new("q", vec![]), Operation::new("Q", vec![])],
        }
        .encode()
        .unwrap(),
    ));
    let leaf_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference((0, 0)),
        "Contents" => content_id,
    });
    let parent_id = doc.add_object(dictionary! {
        "Type" => "Pages",
        "Count" => 1,
        "Kids" => vec![leaf_id.into()],
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(leaf_id) {
        d.set("Parent", parent_id);
    }

    let names_id = doc.add_object(dictionary! {
        "EmbeddedFiles" => dictionary! {
            "Names" => vec![
                Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
                filespec_id.into(),
            ],
        },
    });
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => parent_id,
        "Names" => names_id,
        "AF" => vec![filespec_id.into()],
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Size", Object::Integer(7));

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("serialize fixture pdf");
    bytes
}

fn bench_intake_pdf(c: &mut Criterion) {
    let xml = embedded_cii_xml(ATTACHMENT_LINE_COUNT);
    let pdf = build_factur_x_pdf(ATTACHMENT_NAME, &xml);

    // Sanity: the fixture sniffs as a Factur-X-carrying PDF and yields the
    // embedded attachment.
    assert_eq!(detect_format(&pdf), FormatId::PdfWithFacturX);
    let extracted = extract_factur_x_xml(&pdf).expect("extract must not error");
    assert!(
        extracted.is_some(),
        "fixture must carry a Factur-X attachment"
    );

    c.bench_function("intake-factur-x-extract", |b| {
        b.iter(|| {
            let found = extract_factur_x_xml(black_box(&pdf)).unwrap();
            black_box(found);
        });
    });

    c.bench_function("format-detect", |b| {
        b.iter(|| {
            let id = detect_format(black_box(&pdf));
            black_box(id);
        });
    });
}

criterion_group!(benches, bench_intake_pdf);
criterion_main!(benches);
