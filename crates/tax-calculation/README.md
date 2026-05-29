# invoicekit-tax-calculation

Deterministic invoice arithmetic with a replayable trace.

## What it does

This crate computes the monetary facts an invoice has to add up to: line extension amounts, document-level allowances and charges, per-category tax subtotals, and the final payable total. It does not interpret tax law, look up rates, or decide which category applies — you hand it numbers and a rate, it does the arithmetic. The point is correctness and explainability, not tax logic.

Every public calculation returns both the result and a list of `TraceEntry` steps that record the inputs, the unrounded value, the rounding policy, and the rounded result for each operation. That trace serializes to canonical RFC 8785 JSON, so the same inputs always produce byte-identical output and a validator or evidence bundle can replay exactly how a total was reached.

All money uses `rust_decimal` through the `invoicekit-money` crate. No floats anywhere.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

This crate is foundation-layer arithmetic. It sits next to `invoicekit-money` and `invoicekit-ir`, downstream of the intermediate representation and upstream of validation and evidence. Validators use the trace to check that the totals declared in a document match a faithful recomputation; evidence bundles embed the canonical trace as the proof of how a total was derived. It does not parse or emit any invoice format.

## Public API

Four calculation functions, each returning `Calculation<T>` (the `result` plus a `Vec<TraceEntry>`):

- `calculate_line_extension(LineExtensionInput) -> Calculation<Money>` — `quantity * unit_price`, rounded to scale.
- `apply_allowance_charge(AllowanceChargeInput) -> Calculation<Money>` — subtracts an `Allowance` or adds a `Charge` to a base amount.
- `calculate_tax_subtotal(TaxSubtotalInput) -> Calculation<TaxCategorySubtotal>` — `taxable_amount * (tax_rate / 100)`, rounded to scale.
- `calculate_payable_amount(PayableAmountInput) -> Calculation<PayableBreakdown>` — combines line total, allowances, charges, tax subtotals, and prepaid amount into a full breakdown (tax-exclusive, tax total, tax-inclusive, payable).

Supporting types: `AllowanceChargeKind`, `TraceEntry`, `TraceMoney`, `TaxCategorySubtotal`, `PayableBreakdown`, and the error enum `TaxCalculationError`.

`trace_to_canonical_json(&[TraceEntry]) -> Result<String, _>` serializes a trace to canonical JSON for replay and storage.

`crate_name() -> &'static str` returns the Cargo package name, used by release and log-correlation tooling.

### Inputs are validated

Scales above 28 decimal places, blank identifiers (`line_id`, `category_code`), negative quantities, negative tax rates, and negative amounts where a non-negative one is required all surface as typed `TaxCalculationError` variants rather than producing a wrong number. Credit-note direction is expected to be encoded upstream in the document type or adjustment kind, not as a negative amount here. Currency mismatches propagate from `invoicekit-money` as `TaxCalculationError::Money`.

## Usage

```rust
use invoicekit_ir::{DecimalValue, Iso4217Code};
use invoicekit_money::{Money, Rounding};
use invoicekit_tax_calculation::{calculate_tax_subtotal, trace_to_canonical_json, TaxSubtotalInput};
use rust_decimal::Decimal;

let input = TaxSubtotalInput {
    category_code: "S".to_owned(),
    taxable_amount: Money::new(Decimal::new(10000, 2), Iso4217Code::new("EUR").unwrap()),
    tax_rate: DecimalValue::new(Decimal::new(1900, 2)), // 19.00 percent
    scale: 2,
    rounding: Rounding::HalfUp,
};

let calculation = calculate_tax_subtotal(input).unwrap();
assert_eq!(calculation.result.tax_amount.amount().to_string(), "19.00");

// The trace serializes to canonical JSON for replay.
let canonical = trace_to_canonical_json(&calculation.trace).unwrap();
```

Rounding policy comes from `invoicekit_money::Rounding`: `HalfUp`, `HalfEven` (banker's), or `HalfDown`.

## Status

Implemented and tested. The crate has unit tests for the happy paths and each validation error, byte-stability tests over the canonical trace, and property tests covering charge commutativity, tax-total associativity over category ordering, and run-to-run rounding consistency.

## License

Apache-2.0. Part of the InvoiceKit workspace.
