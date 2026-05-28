// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Tracked operations: `tax-line-extensions`, `tax-payable-amount`, and
//! `tax-trace-canonical-json`.
//!
//! Exercises `invoicekit-tax-calculation` over a realistic multi-line invoice.
//! Tax arithmetic runs on every create/validate of an invoice and produces a
//! replayable trace that evidence bundles serialize, so all three phases are on
//! the hot path:
//!
//! - `tax-line-extensions` computes the per-line extension amount for every
//!   line (the `O(lines)` Decimal-arithmetic core).
//! - `tax-payable-amount` rolls a tax subtotal and the document totals up once.
//! - `tax-trace-canonical-json` serializes the accumulated arithmetic trace
//!   through the canonical JSON writer the evidence bundle depends on.

// criterion_group! / criterion_main! expand to public items without rustdoc.
#![allow(missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use invoicekit_ir::{DecimalValue, Iso4217Code};
use invoicekit_money::{Money, Rounding};
use invoicekit_tax_calculation::{
    calculate_line_extension, calculate_payable_amount, calculate_tax_subtotal,
    trace_to_canonical_json, LineExtensionInput, PayableAmountInput, TaxSubtotalInput, TraceEntry,
};
use rust_decimal::Decimal;

const LINE_COUNT: usize = 200;

fn eur(minor_units: i64) -> Money {
    Money::new(
        Decimal::new(minor_units, 2),
        Iso4217Code::new("EUR").expect("EUR is a valid ISO 4217 code"),
    )
}

fn line_inputs() -> Vec<LineExtensionInput> {
    (0..LINE_COUNT)
        .map(|i| LineExtensionInput {
            line_id: i.to_string(),
            quantity: DecimalValue::new(Decimal::new(250, 2)),
            unit_price: eur(4000),
            scale: 2,
            rounding: Rounding::HalfUp,
        })
        .collect()
}

fn bench_tax_calc(c: &mut Criterion) {
    let inputs = line_inputs();

    // Sanity: the arithmetic core must succeed on the fixture.
    assert!(calculate_line_extension(inputs[0].clone()).is_ok());

    c.bench_function("tax-line-extensions", |b| {
        b.iter(|| {
            for input in &inputs {
                let calc = calculate_line_extension(black_box(input.clone())).unwrap();
                black_box(calc.result);
            }
        });
    });

    c.bench_function("tax-payable-amount", |b| {
        b.iter(|| {
            let subtotal = calculate_tax_subtotal(TaxSubtotalInput {
                category_code: "S".to_owned(),
                taxable_amount: eur(2_000_000),
                tax_rate: DecimalValue::new(Decimal::new(1900, 2)),
                scale: 2,
                rounding: Rounding::HalfUp,
            })
            .unwrap();
            let payable = calculate_payable_amount(PayableAmountInput {
                line_extension_total: eur(2_000_000),
                allowance_total: None,
                charge_total: None,
                tax_subtotals: vec![subtotal.result],
                prepaid_amount: None,
            })
            .unwrap();
            black_box(payable.result);
        });
    });

    // Build a representative trace once (line extensions + a subtotal + the
    // payable rollup) and time only its canonical JSON serialization.
    let mut trace: Vec<TraceEntry> = Vec::new();
    for input in &inputs {
        trace.extend(calculate_line_extension(input.clone()).unwrap().trace);
    }
    let subtotal = calculate_tax_subtotal(TaxSubtotalInput {
        category_code: "S".to_owned(),
        taxable_amount: eur(2_000_000),
        tax_rate: DecimalValue::new(Decimal::new(1900, 2)),
        scale: 2,
        rounding: Rounding::HalfUp,
    })
    .unwrap();
    trace.extend(subtotal.trace);
    let payable = calculate_payable_amount(PayableAmountInput {
        line_extension_total: eur(2_000_000),
        allowance_total: None,
        charge_total: None,
        tax_subtotals: vec![subtotal.result],
        prepaid_amount: None,
    })
    .unwrap();
    trace.extend(payable.trace);
    assert!(trace_to_canonical_json(&trace).is_ok());

    c.bench_function("tax-trace-canonical-json", |b| {
        b.iter(|| {
            let json = trace_to_canonical_json(black_box(&trace)).unwrap();
            black_box(json);
        });
    });
}

criterion_group!(benches, bench_tax_calc);
criterion_main!(benches);
