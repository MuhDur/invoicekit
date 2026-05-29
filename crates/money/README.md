# invoicekit-money

Currency-aware monetary type for InvoiceKit. Decimal-backed, no floats, with deterministic rounding and exact allocation.

## What it does

`Money` pairs a fixed-scale `rust_decimal::Decimal` amount with an `Iso4217Code` currency. Arithmetic is checked: adding or subtracting two values of different currencies returns an error rather than silently coercing, and every operation that could exceed `Decimal`'s range returns a typed `Overflow` instead of panicking or wrapping. This is the type InvoiceKit crates pass around whenever a value is money. Architectural commitment 2.3 bans floating point for monetary values; this crate is how that ban is enforced at the boundary.

There is no foreign-exchange conversion here. Mixing currencies is a caller decision that must go through an explicit policy upstream, so `add` and `sub` reject mismatched currencies on purpose.

## Where it sits

In the pipeline `engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence`, this crate is a foundation primitive. It depends only on `invoicekit-ir` (for `Iso4217Code`) and `rust_decimal`. The tax-calculation and invoice-model layers build on it; nothing in this crate reaches back up the stack.

## Public API

Types:

- `Money` — currency-tagged amount. Constructed with `Money::new(amount, currency)`.
- `Rounding` — rounding policy enum: `HalfUp` (default; "commercial" rounding, away from zero on 0.5), `HalfEven` (banker's), `HalfDown` (toward zero; only when a national rule pack demands it).
- `MoneyError` — `CurrencyMismatch`, `Overflow { operation }`, `AllocateRequiresRatios`, `AllocateZeroSumRatios`.

`Money` methods:

- `amount() -> Decimal` — the underlying decimal, the source of truth.
- `currency() -> &Iso4217Code` and `currency_code() -> String` — the currency, and its three-letter ISO 4217 string.
- `zero_like() -> Money` / `is_zero() -> bool`.
- `add(&other)`, `sub(&other)` — same-currency arithmetic; error on mismatch or overflow.
- `mul_scalar(scalar)` — multiply by a `Decimal` quantity or rate; error on overflow.
- `round(dp, mode)` — round to `dp` decimal places under a `Rounding` mode.
- `allocate(ratios, dp)` — split an amount across integer ratios so the parts sum back to the original exactly (see below).

Plus `crate_name()`, which returns `"invoicekit-money"`.

## Allocation

`allocate` uses the Stripe-style banker's-remainder distribution. It works in minor units at the given scale `dp` (2 for EUR/USD, 0 for JPY, 3 for KWD): each share gets `amount * ratio / sum(ratios)` rounded down, then the leftover minor units are handed out one at a time to the shares with the largest fractional remainders. Ties break toward the lowest original index, so the result is stable. The defining property — the allocated parts sum to the original amount, with no rounding drift — is checked by a property test in the suite.

## Usage

```rust
use invoicekit_money::{Money, Rounding};
use invoicekit_ir::Iso4217Code;
use rust_decimal::Decimal;

let eur = Iso4217Code::new("EUR").unwrap();

// Decimal::new(value, scale): 10000 at scale 2 == 100.00.
let price = Money::new(Decimal::new(10000, 2), eur.clone());
let tax = Money::new(Decimal::new(2599, 2), eur.clone());

let gross = price.add(&tax).unwrap();
assert_eq!(gross.amount().to_string(), "125.99");

// Split €1.00 three equal ways with no rounding error.
let total = Money::new(Decimal::new(100, 2), eur);
let parts = total.allocate(&[1, 1, 1], 2).unwrap();
assert_eq!(parts[0].amount().to_string(), "0.34");
assert_eq!(parts[1].amount().to_string(), "0.33");
assert_eq!(parts[2].amount().to_string(), "0.33");
```

`Money` serializes through serde with the amount rendered as a fixed-scale string (`rust_decimal::serde::str`), so JSON round-trips preserve scale exactly.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
