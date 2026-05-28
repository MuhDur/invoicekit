// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operation: `codelist-lookup`.
//!
//! Measures `invoicekit-codelists` membership lookups. The validator consults
//! the code list registry for currency, country, unit, and VAT-category codes
//! on every relevant field of every invoice line, so lookup latency multiplies
//! across a bulk validation run.
//!
//! `Registry::lookup` currently does a linear `entries.iter().find` over each
//! list (`crates/codelists/src/lib.rs:414`); ISO 4217 carries ~180 entries and
//! ISO 3166 ~250, so a realistic field mix is `O(entries)` per call. This bench
//! drives a representative few-hundred-lookup mix against a seeded registry so
//! the rank-9 fix (per-list index keyed by code) has a baseline.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_codelists::{
    Registry, EN16931_VAT_CATEGORY, ISO_3166_1_ALPHA2, ISO_4217, UNECE_REC20_UNITS,
};

const ON_DATE: &str = "2026-05-27";

// A realistic per-invoice field mix: currency, country, unit, and VAT category
// codes, including a deliberate miss (`ZZZ`) to exercise the full-scan path
// that a linear lookup must walk to completion.
const LOOKUPS: &[(&str, &str)] = &[
    (ISO_4217, "EUR"),
    (ISO_4217, "USD"),
    (ISO_4217, "ZZZ"),
    (ISO_3166_1_ALPHA2, "DE"),
    (ISO_3166_1_ALPHA2, "NL"),
    (ISO_3166_1_ALPHA2, "FR"),
    (UNECE_REC20_UNITS, "C62"),
    (EN16931_VAT_CATEGORY, "S"),
    (EN16931_VAT_CATEGORY, "Z"),
];

const REPEATS: usize = 32;

fn bench_codelist_lookup(c: &mut Criterion) {
    let registry = Registry::seeded().expect("seed registry must load");

    // Sanity: at least one known code resolves.
    assert!(registry.lookup(ISO_4217, "EUR", ON_DATE).is_some());

    c.bench_function("codelist-lookup", |b| {
        b.iter(|| {
            for _ in 0..REPEATS {
                for (list, code) in LOOKUPS {
                    let hit = registry.lookup(black_box(list), black_box(code), ON_DATE);
                    black_box(hit);
                }
            }
        });
    });
}

criterion_group!(benches, bench_codelist_lookup);
criterion_main!(benches);
