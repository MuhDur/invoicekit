// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operations: `evidence-pack`, `evidence-unpack`, and
//! `evidence-verify`.
//!
//! Every InvoiceKit operation emits a signed `.ikb` evidence bundle, so the
//! pack/unpack/verify path runs once per invoice on the trust-toolkit hot path.
//!
//! - `evidence-pack` zstd-compresses a deterministic tar of the artefacts.
//! - `evidence-unpack` inflates and re-parses one back into memory.
//! - `evidence-verify` re-hashes every artefact with BLAKE3.
//!
//! Splitting the three isolates whether zstd (pack) or BLAKE3 (verify)
//! dominates, which the rank-8 optimization needs to know before touching
//! either. The fixture mirrors a real bundle: a canonical XML document
//! (~64 KiB), an embedded PDF-shaped blob (~256 KiB), and a validation report
//! JSON.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_evidence::{manifest_for, pack, unpack, verify, EvidenceBundle};

const CANONICAL_XML_BYTES: usize = 64 * 1024;
const PDF_BYTES: usize = 256 * 1024;

fn synthetic_bundle() -> EvidenceBundle {
    let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    // Canonical XML: compressible, repeated invoice-line shape.
    let mut xml = String::with_capacity(CANONICAL_XML_BYTES + 256);
    xml.push_str("<Invoice>");
    let mut line = 0_u32;
    while xml.len() < CANONICAL_XML_BYTES {
        xml.push_str("<Line><Id>");
        xml.push_str(&line.to_string());
        xml.push_str("</Id><Amount>100.00</Amount></Line>");
        line += 1;
    }
    xml.push_str("</Invoice>");
    artefacts.insert("formats/ubl.xml".to_owned(), xml.into_bytes());

    // PDF-shaped blob: less compressible (pseudo-random byte spread) so the
    // zstd cost is realistic rather than trivially compressible.
    let mut pdf = Vec::with_capacity(PDF_BYTES);
    pdf.extend_from_slice(b"%PDF-1.7\n");
    let mut state: u32 = 0x9E37_79B9;
    while pdf.len() < PDF_BYTES {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        pdf.push((state >> 24) as u8);
    }
    artefacts.insert("render/invoice.pdf".to_owned(), pdf);

    // Validation report JSON.
    artefacts.insert(
        "validation/report.json".to_owned(),
        br#"{"syntax":"ubl","findings":[],"coverage":{"implemented":86,"total":86}}"#.to_vec(),
    );

    let manifest = manifest_for(
        &artefacts,
        "tenant_bench",
        "trace_bench",
        "2026-05-27T00:00:00Z",
    );
    EvidenceBundle {
        manifest,
        artefacts,
    }
}

fn bench_evidence(c: &mut Criterion) {
    let bundle = synthetic_bundle();
    let packed = pack(&bundle).expect("pack must succeed");
    assert!(unpack(&packed).is_ok());
    assert!(verify(&bundle).is_ok());

    c.bench_function("evidence-pack", |b| {
        b.iter(|| {
            let bytes = pack(black_box(&bundle)).unwrap();
            black_box(bytes);
        });
    });

    c.bench_function("evidence-unpack", |b| {
        b.iter(|| {
            let restored = unpack(black_box(&packed)).unwrap();
            black_box(restored);
        });
    });

    c.bench_function("evidence-verify", |b| {
        b.iter(|| {
            verify(black_box(&bundle)).unwrap();
        });
    });
}

criterion_group!(benches, bench_evidence);
criterion_main!(benches);
