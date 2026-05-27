// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-money` — currency-aware monetary type with deterministic rounding.
//!
//! `Money` carries an amount as a fixed-scale `rust_decimal::Decimal` plus an
//! `Iso4217Code` currency. Arithmetic is checked for overflow and currency
//! mismatch; allocation uses the Stripe-style banker's-remainder algorithm so
//! the sum of the allocated parts equals the original to the last minor unit.
//!
//! ## Why a dedicated type
//!
//! InvoiceKit's [architectural commitment 2.3](../../plans/PLAN.md) bans
//! floating-point arithmetic for monetary values. `Money` is the boundary
//! type every crate uses; the underlying `Decimal` is the source of truth
//! and is exposed through `Money::amount`.
//!
//! ## Rounding
//!
//! Every operation that may produce a non-representable scale carries a
//! [`Rounding`] selector. The three modes mandated by the invoice spec are
//! [`Rounding::HalfUp`] (most common), [`Rounding::HalfEven`] (banker's, the
//! IEEE-754 default), and [`Rounding::HalfDown`] (rare; supported because
//! some national rule packs require it).

use invoicekit_ir::Iso4217Code;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Rounding policy used by [`Money::round`] and the rounding-aware arithmetic
/// helpers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
// Variants share the `Half` prefix because that mirrors the IEEE 754 / ISO 80000
// rounding-mode vocabulary every invoice rule pack uses; renaming would obscure
// the spec mapping for readers.
#[allow(clippy::enum_variant_names)]
pub enum Rounding {
    /// Half-up (sometimes called "commercial" rounding): 0.5 rounds away
    /// from zero. The most common policy in invoice arithmetic.
    #[default]
    HalfUp,
    /// Half-even (banker's rounding, IEEE-754 default): 0.5 rounds to the
    /// nearest even digit, eliminating positive bias across many roundings.
    HalfEven,
    /// Half-down: 0.5 rounds toward zero. Supported because a small set of
    /// national rule packs require it; never use this without checking the
    /// rule pack.
    HalfDown,
}

impl From<Rounding> for RoundingStrategy {
    fn from(value: Rounding) -> Self {
        match value {
            Rounding::HalfUp => Self::MidpointAwayFromZero,
            Rounding::HalfEven => Self::MidpointNearestEven,
            Rounding::HalfDown => Self::MidpointTowardZero,
        }
    }
}

/// Errors emitted by [`Money`] arithmetic.
#[derive(Debug, Error)]
pub enum MoneyError {
    /// The two operands carried different currencies.
    #[error(
        "currency mismatch: left=`{left}`, right=`{right}`; hint: convert through an explicit FX policy before mixing currencies"
    )]
    CurrencyMismatch {
        /// Currency of the left operand.
        left: String,
        /// Currency of the right operand.
        right: String,
    },
    /// The operation overflowed the supported `Decimal` range.
    #[error(
        "money arithmetic overflowed `Decimal`'s representable range during `{operation}`; hint: split the operation into chunks or convert to a higher-range type"
    )]
    Overflow {
        /// Operation that overflowed (`add`, `sub`, `mul`, `round`, ...).
        operation: &'static str,
    },
    /// Allocation was asked to split across zero ratios.
    #[error(
        "money allocate requires a non-empty ratio vector; hint: pass at least one positive ratio"
    )]
    AllocateRequiresRatios,
    /// Allocation was asked to use a zero-sum ratio vector.
    #[error("money allocate requires at least one positive ratio; hint: zeros must be a subset of positive entries")]
    AllocateZeroSumRatios,
}

/// Currency-tagged monetary amount.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Money {
    #[serde(with = "rust_decimal::serde::str")]
    amount: Decimal,
    currency: Iso4217Code,
}

impl Money {
    /// Build a money value from a decimal amount and a validated currency.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_money::Money;
    /// use invoicekit_ir::Iso4217Code;
    /// use rust_decimal::Decimal;
    ///
    /// let m = Money::new(Decimal::new(10000, 2), Iso4217Code::new("EUR").unwrap());
    /// assert_eq!(m.amount().to_string(), "100.00");
    /// assert_eq!(m.currency_code(), "EUR");
    /// ```
    #[must_use]
    pub fn new(amount: Decimal, currency: Iso4217Code) -> Self {
        Self { amount, currency }
    }

    /// Returns the underlying decimal amount.
    #[must_use]
    pub const fn amount(&self) -> Decimal {
        self.amount
    }

    /// Returns the currency.
    #[must_use]
    pub const fn currency(&self) -> &Iso4217Code {
        &self.currency
    }

    /// Returns the currency as the three-letter ISO 4217 string.
    ///
    /// Internally this round-trips through serde because `Iso4217Code`
    /// does not expose the inner string directly; if serde fails for any
    /// reason (which should never happen for a validated `Iso4217Code`)
    /// this returns an empty string.
    #[must_use]
    pub fn currency_code(&self) -> String {
        // serde-derived `Display`-equivalent: a transparent newtype around
        // a validated three-letter ASCII string serializes as itself.
        serde_json::to_value(&self.currency)
            .ok()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_default()
    }

    /// Returns zero in the same currency.
    #[must_use]
    pub fn zero_like(&self) -> Self {
        Self {
            amount: Decimal::ZERO,
            currency: self.currency.clone(),
        }
    }

    /// True when the amount is exactly zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }

    /// Add two money values of the same currency.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::CurrencyMismatch`] when the currencies differ
    /// or [`MoneyError::Overflow`] when the sum exceeds the `Decimal` range.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_money::Money;
    /// # use invoicekit_ir::Iso4217Code;
    /// # use rust_decimal::Decimal;
    /// let a = Money::new(Decimal::new(10000, 2), Iso4217Code::new("EUR").unwrap());
    /// let b = Money::new(Decimal::new(2599, 2), Iso4217Code::new("EUR").unwrap());
    /// assert_eq!(a.add(&b).unwrap().amount().to_string(), "125.99");
    /// ```
    pub fn add(&self, other: &Self) -> Result<Self, MoneyError> {
        self.require_same_currency(other)?;
        let amount = self
            .amount
            .checked_add(other.amount)
            .ok_or(MoneyError::Overflow { operation: "add" })?;
        Ok(Self {
            amount,
            currency: self.currency.clone(),
        })
    }

    /// Subtract another money value of the same currency.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::CurrencyMismatch`] or [`MoneyError::Overflow`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_money::Money;
    /// # use invoicekit_ir::Iso4217Code;
    /// # use rust_decimal::Decimal;
    /// let gross = Money::new(Decimal::new(12000, 2), Iso4217Code::new("EUR").unwrap());
    /// let tax = Money::new(Decimal::new(2000, 2), Iso4217Code::new("EUR").unwrap());
    /// assert_eq!(gross.sub(&tax).unwrap().amount().to_string(), "100.00");
    /// ```
    pub fn sub(&self, other: &Self) -> Result<Self, MoneyError> {
        self.require_same_currency(other)?;
        let amount = self
            .amount
            .checked_sub(other.amount)
            .ok_or(MoneyError::Overflow { operation: "sub" })?;
        Ok(Self {
            amount,
            currency: self.currency.clone(),
        })
    }

    /// Multiply by a scalar `Decimal` (typically a quantity or rate).
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] when the product exceeds the `Decimal`
    /// range.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_money::Money;
    /// # use invoicekit_ir::Iso4217Code;
    /// # use rust_decimal::Decimal;
    /// let unit_price = Money::new(Decimal::new(2500, 2), Iso4217Code::new("EUR").unwrap());
    /// let line_total = unit_price.mul_scalar(Decimal::new(3, 0)).unwrap();
    /// assert_eq!(line_total.amount().to_string(), "75.00");
    /// ```
    pub fn mul_scalar(&self, scalar: Decimal) -> Result<Self, MoneyError> {
        let amount = self
            .amount
            .checked_mul(scalar)
            .ok_or(MoneyError::Overflow { operation: "mul" })?;
        Ok(Self {
            amount,
            currency: self.currency.clone(),
        })
    }

    /// Round the amount to `dp` decimal places using `mode`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_money::{Money, Rounding};
    /// # use invoicekit_ir::Iso4217Code;
    /// # use rust_decimal::Decimal;
    /// let raw = Money::new(
    ///     Decimal::new(123456, 4), // 12.3456
    ///     Iso4217Code::new("EUR").unwrap(),
    /// );
    /// let rounded = raw.round(2, Rounding::HalfEven);
    /// assert_eq!(rounded.amount().to_string(), "12.35");
    /// ```
    #[must_use]
    pub fn round(&self, dp: u32, mode: Rounding) -> Self {
        let strategy = RoundingStrategy::from(mode);
        let amount = self.amount.round_dp_with_strategy(dp, strategy);
        Self {
            amount,
            currency: self.currency.clone(),
        }
    }

    /// Allocate the amount across `ratios` using the Stripe-style banker's
    /// remainder distribution: each share is `amount * ratio / sum(ratios)`
    /// rounded down to the nearest minor unit, and the remainder is
    /// distributed one minor unit at a time to the largest-fractional-part
    /// shares so that the sum of the allocated parts equals the original
    /// amount exactly.
    ///
    /// The `dp` argument controls the working scale (typically the
    /// currency's minor-unit count — 2 for EUR/USD, 0 for JPY, 3 for KWD).
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::AllocateRequiresRatios`] when `ratios` is empty,
    /// [`MoneyError::AllocateZeroSumRatios`] when every ratio is zero, or
    /// [`MoneyError::Overflow`] on internal `Decimal` overflow.
    ///
    /// # Examples
    ///
    /// ```
    /// # use invoicekit_money::Money;
    /// # use invoicekit_ir::Iso4217Code;
    /// # use rust_decimal::Decimal;
    /// // Split €1.00 three ways across equal ratios.
    /// let total = Money::new(Decimal::new(100, 2), Iso4217Code::new("EUR").unwrap());
    /// let parts = total.allocate(&[1, 1, 1], 2).unwrap();
    /// assert_eq!(parts.len(), 3);
    /// // 0.34 + 0.33 + 0.33 = 1.00 (no rounding error).
    /// assert_eq!(parts[0].amount().to_string(), "0.34");
    /// assert_eq!(parts[1].amount().to_string(), "0.33");
    /// assert_eq!(parts[2].amount().to_string(), "0.33");
    /// ```
    pub fn allocate(&self, ratios: &[u64], dp: u32) -> Result<Vec<Self>, MoneyError> {
        if ratios.is_empty() {
            return Err(MoneyError::AllocateRequiresRatios);
        }
        let sum: u128 = ratios.iter().map(|r| u128::from(*r)).sum();
        if sum == 0 {
            return Err(MoneyError::AllocateZeroSumRatios);
        }

        // Work in minor units to avoid repeated rounding error: scale the
        // amount up by 10^dp, round to the nearest integer with the chosen
        // dp, then distribute integer minor units across the ratios. Any
        // arithmetic that could silently lose precision must surface as a
        // typed Overflow error so the caller never sees a corrupted split.
        let scale_factor_u64 = 10u64.checked_pow(dp).ok_or(MoneyError::Overflow {
            operation: "allocate",
        })?;
        let scale_factor = Decimal::from(scale_factor_u64);
        let total_minor = self
            .amount
            .checked_mul(scale_factor)
            .ok_or(MoneyError::Overflow {
                operation: "allocate",
            })?
            .round();
        let sign = if total_minor.is_sign_negative() {
            Decimal::NEGATIVE_ONE
        } else {
            Decimal::ONE
        };
        let abs_minor = total_minor.abs();
        let total_u128: u128 = abs_minor
            .to_string()
            .parse()
            .map_err(|_| MoneyError::Overflow {
                operation: "allocate",
            })?;

        let mut shares = Vec::with_capacity(ratios.len());
        let mut remainders = Vec::with_capacity(ratios.len());
        let mut distributed: u128 = 0;
        for &r in ratios {
            // checked_mul propagates overflow as a typed error; the prior
            // saturating_mul could cap silently and corrupt the share table.
            let num = total_u128
                .checked_mul(u128::from(r))
                .ok_or(MoneyError::Overflow {
                    operation: "allocate",
                })?;
            let base = num / sum;
            let rem = num % sum;
            shares.push(base);
            remainders.push(rem);
            distributed = distributed.checked_add(base).ok_or(MoneyError::Overflow {
                operation: "allocate",
            })?;
        }
        let leftover = total_u128
            .checked_sub(distributed)
            .ok_or(MoneyError::Overflow {
                operation: "allocate",
            })?;

        // Distribute the leftover minor units to the largest remainders;
        // ties broken by lowest original index (so the result is stable).
        let mut order: Vec<usize> = (0..ratios.len()).collect();
        order.sort_by(|&a, &b| remainders[b].cmp(&remainders[a]).then_with(|| a.cmp(&b)));
        let take_n = usize::try_from(leftover).map_err(|_| MoneyError::Overflow {
            operation: "allocate",
        })?;
        for &idx in order.iter().take(take_n) {
            shares[idx] = shares[idx].checked_add(1).ok_or(MoneyError::Overflow {
                operation: "allocate",
            })?;
        }

        // Convert each minor-unit share back into a Money at the requested
        // scale, restoring the original sign. The divisor reuses the
        // already-checked scale_factor_u64 from above so any future change
        // can't reintroduce the silent-fallback bug.
        let divisor = Decimal::from(scale_factor_u64);
        let mut out = Vec::with_capacity(shares.len());
        for share in shares {
            let amt_minor = Decimal::from(share);
            let signed = amt_minor * sign;
            let amount = signed.checked_div(divisor).ok_or(MoneyError::Overflow {
                operation: "allocate",
            })?;
            out.push(Self {
                amount,
                currency: self.currency.clone(),
            });
        }
        Ok(out)
    }

    fn require_same_currency(&self, other: &Self) -> Result<(), MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch {
                left: self.currency_code(),
                right: other.currency_code(),
            });
        }
        Ok(())
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_money::crate_name(), "invoicekit-money");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-money"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn eur(amount: Decimal) -> Money {
        Money::new(amount, Iso4217Code::new("EUR").unwrap())
    }

    fn usd(amount: Decimal) -> Money {
        Money::new(amount, Iso4217Code::new("USD").unwrap())
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-money");
    }

    #[test]
    fn add_same_currency_works() {
        let a = eur(Decimal::new(10000, 2));
        let b = eur(Decimal::new(2599, 2));
        assert_eq!(a.add(&b).unwrap().amount().to_string(), "125.99");
    }

    #[test]
    fn add_different_currency_is_rejected() {
        let a = eur(Decimal::new(10000, 2));
        let b = usd(Decimal::new(10000, 2));
        let err = a.add(&b).unwrap_err();
        assert!(matches!(err, MoneyError::CurrencyMismatch { .. }));
    }

    #[test]
    fn sub_works_and_can_go_negative() {
        let a = eur(Decimal::new(2000, 2));
        let b = eur(Decimal::new(3000, 2));
        assert_eq!(a.sub(&b).unwrap().amount().to_string(), "-10.00");
    }

    #[test]
    fn mul_scalar_round_trips() {
        let a = eur(Decimal::new(10000, 2));
        assert_eq!(
            a.mul_scalar(Decimal::from(3)).unwrap().amount().to_string(),
            "300.00"
        );
    }

    #[test]
    fn round_half_up_vs_half_even_diverges_on_midpoint() {
        let value = eur(Decimal::new(125, 2)); // 1.25
        assert_eq!(value.round(1, Rounding::HalfUp).amount().to_string(), "1.3");
        assert_eq!(
            value.round(1, Rounding::HalfEven).amount().to_string(),
            "1.2"
        );
        let other = eur(Decimal::new(135, 2)); // 1.35
        assert_eq!(
            other.round(1, Rounding::HalfEven).amount().to_string(),
            "1.4"
        );
        assert_eq!(
            other.round(1, Rounding::HalfDown).amount().to_string(),
            "1.3"
        );
    }

    #[test]
    fn allocate_distributes_remainder_to_largest_fractions() {
        // €1.00 across [1, 1, 1] → 0.34 + 0.33 + 0.33
        let total = eur(Decimal::new(100, 2));
        let parts = total.allocate(&[1, 1, 1], 2).unwrap();
        assert_eq!(parts.len(), 3);
        let sum: Decimal = parts.iter().map(Money::amount).sum();
        assert_eq!(sum, total.amount());
        assert_eq!(parts[0].amount().to_string(), "0.34");
        assert_eq!(parts[1].amount().to_string(), "0.33");
        assert_eq!(parts[2].amount().to_string(), "0.33");
    }

    #[test]
    fn allocate_weighted_split_preserves_sum() {
        // €1.00 across [2, 1] → 0.67 + 0.33
        let total = eur(Decimal::new(100, 2));
        let parts = total.allocate(&[2, 1], 2).unwrap();
        let sum: Decimal = parts.iter().map(Money::amount).sum();
        assert_eq!(sum, total.amount());
        assert_eq!(parts[0].amount().to_string(), "0.67");
        assert_eq!(parts[1].amount().to_string(), "0.33");
    }

    #[test]
    fn allocate_empty_ratios_is_rejected() {
        let total = eur(Decimal::new(100, 2));
        let err = total.allocate(&[], 2).unwrap_err();
        assert!(matches!(err, MoneyError::AllocateRequiresRatios));
    }

    #[test]
    fn allocate_zero_sum_ratios_is_rejected() {
        let total = eur(Decimal::new(100, 2));
        let err = total.allocate(&[0, 0, 0], 2).unwrap_err();
        assert!(matches!(err, MoneyError::AllocateZeroSumRatios));
    }

    /// Regression for invoices-6jsl: an oversized `dp` argument used to fall
    /// back to scale factor 1 and silently corrupt the result; now it must
    /// surface as `MoneyError::Overflow`.
    #[test]
    fn allocate_oversized_dp_overflows() {
        let total = eur(Decimal::new(100, 2));
        // 10u64.pow(20) overflows u64, so dp >= 20 must be rejected.
        let err = total.allocate(&[1, 1], 20).unwrap_err();
        assert!(matches!(
            err,
            MoneyError::Overflow {
                operation: "allocate"
            }
        ));
    }

    /// Regression for invoices-6jsl: a huge `(total_minor, ratio)` product
    /// used to saturate at `u128::MAX` and corrupt the share table; now the
    /// internal `checked_mul` must surface the overflow.
    #[test]
    fn allocate_ratio_product_overflow_is_reported() {
        // Construct an amount whose minor-unit count is near u128::MAX so the
        // ratio multiplication overflows. Use dp = 0 so total_minor == amount.
        let huge_amount = Decimal::MAX;
        let total = Money::new(huge_amount, Iso4217Code::new("EUR").unwrap());
        let err = total.allocate(&[u64::MAX, u64::MAX], 0).unwrap_err();
        assert!(matches!(
            err,
            MoneyError::Overflow {
                operation: "allocate"
            }
        ));
    }

    #[test]
    fn money_round_trips_through_json() {
        let m = eur(Decimal::new(12345, 2));
        let json = serde_json::to_string(&m).unwrap();
        let back: Money = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        // Amount serializes as a fixed-scale string (rust_decimal::serde::str).
        assert!(json.contains("\"123.45\""));
    }

    proptest! {
        /// Add is commutative for same-currency operands.
        #[test]
        fn add_is_commutative(a in -10_000_000_i64..=10_000_000, b in -10_000_000_i64..=10_000_000) {
            let m1 = eur(Decimal::new(a, 2));
            let m2 = eur(Decimal::new(b, 2));
            prop_assert_eq!(m1.add(&m2).unwrap(), m2.add(&m1).unwrap());
        }

        /// Add is associative for same-currency operands.
        #[test]
        fn add_is_associative(
            a in -1_000_000_i64..=1_000_000,
            b in -1_000_000_i64..=1_000_000,
            c in -1_000_000_i64..=1_000_000,
        ) {
            let m1 = eur(Decimal::new(a, 2));
            let m2 = eur(Decimal::new(b, 2));
            let m3 = eur(Decimal::new(c, 2));
            let left = m1.add(&m2).unwrap().add(&m3).unwrap();
            let right = m1.add(&m2.add(&m3).unwrap()).unwrap();
            prop_assert_eq!(left, right);
        }

        /// Allocate's defining invariant: sum of parts equals the original.
        #[test]
        fn allocate_preserves_sum(
            amount in -1_000_000_i64..=1_000_000,
            r1 in 1_u64..=10,
            r2 in 1_u64..=10,
            r3 in 1_u64..=10,
        ) {
            let total = eur(Decimal::new(amount, 2));
            let parts = total.allocate(&[r1, r2, r3], 2).unwrap();
            let sum: Decimal = parts.iter().map(Money::amount).sum();
            prop_assert_eq!(sum, total.amount());
        }
    }
}
