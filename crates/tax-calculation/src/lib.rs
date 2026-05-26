// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Deterministic invoice arithmetic with replayable trace output.
//!
//! This crate computes the arithmetic facts that validators and evidence
//! bundles need to explain invoice totals: line extension amounts,
//! allowances and charges, tax category subtotals, and payable totals. Every
//! public calculation returns both the result and a structured [`TraceEntry`]
//! sequence that can be serialized through [`trace_to_canonical_json`] for
//! byte-stable replay.

use invoicekit_canonical::{canonicalize_value, CanonicalizeError};
use invoicekit_ir::DecimalValue;
use invoicekit_money::{Money, MoneyError, Rounding};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_DECIMAL_SCALE: u32 = 28;

/// Result wrapper returned by every tax calculation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Calculation<T> {
    /// Computed business result.
    pub result: T,
    /// Replayable arithmetic trace that produced `result`.
    pub trace: Vec<TraceEntry>,
}

/// Currency-tagged amount recorded in a trace entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TraceMoney {
    /// Decimal amount serialized as a fixed-scale string.
    pub amount: DecimalValue,
    /// ISO 4217 currency code.
    pub currency: String,
}

impl TraceMoney {
    fn from_money(value: &Money) -> Self {
        Self {
            amount: DecimalValue::new(value.amount()),
            currency: value.currency().as_str().to_owned(),
        }
    }
}

/// Kind of invoice-level adjustment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowanceChargeKind {
    /// Subtract the adjustment amount from the base amount.
    Allowance,
    /// Add the adjustment amount to the base amount.
    Charge,
}

/// Input for [`calculate_line_extension`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LineExtensionInput {
    /// Stable line identifier copied into the trace.
    pub line_id: String,
    /// Invoiced quantity.
    pub quantity: DecimalValue,
    /// Unit price amount.
    pub unit_price: Money,
    /// Decimal places to round the line extension to.
    pub scale: u32,
    /// Rounding policy for the final line extension.
    pub rounding: Rounding,
}

/// Input for [`apply_allowance_charge`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AllowanceChargeInput {
    /// Amount before applying the adjustment.
    pub base_amount: Money,
    /// Whether the adjustment is an allowance or a charge.
    pub kind: AllowanceChargeKind,
    /// Positive adjustment amount.
    pub adjustment_amount: Money,
}

/// Input for [`calculate_tax_subtotal`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaxSubtotalInput {
    /// Tax category code, such as EN 16931 `S` or `Z`.
    pub category_code: String,
    /// Taxable amount for this category.
    pub taxable_amount: Money,
    /// Tax rate percentage, for example `19.00` for 19 percent.
    pub tax_rate: DecimalValue,
    /// Decimal places to round the tax amount to.
    pub scale: u32,
    /// Rounding policy for the final tax amount.
    pub rounding: Rounding,
}

/// Calculated tax summary for one tax category.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaxCategorySubtotal {
    /// Tax category code.
    pub category_code: String,
    /// Taxable amount for this category.
    pub taxable_amount: Money,
    /// Tax rate percentage.
    pub tax_rate: DecimalValue,
    /// Rounded tax amount.
    pub tax_amount: Money,
}

/// Tax subtotal projection stored inside payable-amount traces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaxSubtotalTracePart {
    /// Tax category code.
    pub category_code: String,
    /// Taxable amount for this category.
    pub taxable_amount: TraceMoney,
    /// Rounded tax amount for this category.
    pub tax_amount: TraceMoney,
    /// Tax rate percentage.
    pub tax_rate: DecimalValue,
}

/// Input for [`calculate_payable_amount`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PayableAmountInput {
    /// Sum of line extension amounts.
    pub line_extension_total: Money,
    /// Optional sum of document-level allowances.
    pub allowance_total: Option<Money>,
    /// Optional sum of document-level charges.
    pub charge_total: Option<Money>,
    /// Tax category subtotals that contribute to tax-inclusive total.
    pub tax_subtotals: Vec<TaxCategorySubtotal>,
    /// Optional prepaid amount to subtract from the payable amount.
    pub prepaid_amount: Option<Money>,
}

/// Calculated invoice monetary totals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PayableBreakdown {
    /// Sum of line extension amounts.
    pub line_extension_total: Money,
    /// Sum of document-level allowances.
    pub allowance_total: Money,
    /// Sum of document-level charges.
    pub charge_total: Money,
    /// Tax-exclusive total.
    pub tax_exclusive_amount: Money,
    /// Sum of tax category tax amounts.
    pub tax_total: Money,
    /// Tax-inclusive total.
    pub tax_inclusive_amount: Money,
    /// Prepaid amount subtracted from the payable amount.
    pub prepaid_amount: Money,
    /// Final payable amount.
    pub payable_amount: Money,
}

/// Replayable arithmetic step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TraceEntry {
    /// Line extension calculation: `quantity * unit_price`, rounded to scale.
    LineExtension {
        /// Stable line identifier.
        line_id: String,
        /// Invoiced quantity.
        quantity: DecimalValue,
        /// Unit price amount.
        unit_price: TraceMoney,
        /// Rounding policy used for the final amount.
        rounding: Rounding,
        /// Decimal places used for the final amount.
        scale: u32,
        /// Raw unrounded product.
        unrounded_amount: TraceMoney,
        /// Rounded line extension amount.
        result: TraceMoney,
    },
    /// Document-level allowance or charge application.
    AllowanceCharge {
        /// Adjustment kind.
        kind: AllowanceChargeKind,
        /// Amount before applying the adjustment.
        base_amount: TraceMoney,
        /// Adjustment amount.
        adjustment_amount: TraceMoney,
        /// Amount after applying the adjustment.
        result: TraceMoney,
    },
    /// Tax category subtotal calculation.
    TaxSubtotal {
        /// Tax category code.
        category_code: String,
        /// Taxable amount for the category.
        taxable_amount: TraceMoney,
        /// Tax rate percentage.
        tax_rate: DecimalValue,
        /// Tax rate as a decimal factor.
        rate_factor: DecimalValue,
        /// Rounding policy used for the tax amount.
        rounding: Rounding,
        /// Decimal places used for the tax amount.
        scale: u32,
        /// Raw unrounded tax amount.
        unrounded_tax_amount: TraceMoney,
        /// Rounded tax amount.
        tax_amount: TraceMoney,
    },
    /// Payable amount calculation across line, adjustment, tax, and prepaid totals.
    PayableAmount {
        /// Sum of line extension amounts.
        line_extension_total: TraceMoney,
        /// Sum of document-level allowances.
        allowance_total: TraceMoney,
        /// Sum of document-level charges.
        charge_total: TraceMoney,
        /// Tax-exclusive total.
        tax_exclusive_amount: TraceMoney,
        /// Tax category inputs that produced the tax total.
        tax_subtotals: Vec<TaxSubtotalTracePart>,
        /// Sum of tax category tax amounts.
        tax_total: TraceMoney,
        /// Tax-inclusive total.
        tax_inclusive_amount: TraceMoney,
        /// Prepaid amount subtracted from the payable amount.
        prepaid_amount: TraceMoney,
        /// Final payable amount.
        payable_amount: TraceMoney,
    },
}

/// Errors emitted by invoice tax arithmetic.
#[derive(Debug, Error)]
pub enum TaxCalculationError {
    /// Decimal scale exceeded `rust_decimal`'s supported precision.
    #[error("invalid decimal scale `{scale}`; hint: use a scale between 0 and 28 decimal places")]
    InvalidScale {
        /// Requested scale.
        scale: u32,
    },
    /// Required text input was blank.
    #[error("missing required field `{field}`; hint: provide a non-empty value")]
    MissingRequiredField {
        /// Field name.
        field: &'static str,
    },
    /// A value that must be non-negative was negative.
    #[error("`{field}` must be non-negative; hint: encode credit-note direction in the document type or adjustment kind")]
    NegativeAmount {
        /// Field name.
        field: &'static str,
    },
    /// Money arithmetic failed.
    #[error("money arithmetic failed during `{operation}`: {source}; hint: verify currencies and decimal magnitude before retrying")]
    Money {
        /// Operation being performed.
        operation: &'static str,
        /// Underlying money error.
        #[source]
        source: MoneyError,
    },
    /// Trace conversion to JSON failed.
    #[error("trace serialization failed: {source}; hint: report this as an InvoiceKit bug because traces only contain serializable primitives")]
    TraceSerialization {
        /// Underlying JSON serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// Trace canonicalization failed.
    #[error("trace canonicalization failed: {source}; hint: report this as an InvoiceKit bug because traces use I-JSON-safe values")]
    TraceCanonicalization {
        /// Underlying canonicalization error.
        #[source]
        source: CanonicalizeError,
    },
}

/// Calculate one line extension amount.
///
/// # Errors
///
/// Returns [`TaxCalculationError::InvalidScale`] for unsupported decimal
/// scales, [`TaxCalculationError::MissingRequiredField`] for a blank line id,
/// [`TaxCalculationError::NegativeAmount`] for a negative quantity, or
/// [`TaxCalculationError::Money`] for decimal overflow.
///
/// # Examples
///
/// ```
/// use invoicekit_ir::{DecimalValue, Iso4217Code};
/// use invoicekit_money::{Money, Rounding};
/// use invoicekit_tax_calculation::{calculate_line_extension, LineExtensionInput};
/// use rust_decimal::Decimal;
///
/// let input = LineExtensionInput {
///     line_id: "1".to_owned(),
///     quantity: DecimalValue::new(Decimal::new(250, 2)),
///     unit_price: Money::new(Decimal::new(4000, 2), Iso4217Code::new("EUR").unwrap()),
///     scale: 2,
///     rounding: Rounding::HalfUp,
/// };
///
/// let calculation = calculate_line_extension(input).unwrap();
/// assert_eq!(calculation.result.amount().to_string(), "100.00");
/// assert_eq!(calculation.trace.len(), 1);
/// ```
pub fn calculate_line_extension(
    input: LineExtensionInput,
) -> Result<Calculation<Money>, TaxCalculationError> {
    validate_scale(input.scale)?;
    validate_non_empty(&input.line_id, "line_id")?;
    if input.quantity.inner().is_sign_negative() {
        return Err(TaxCalculationError::NegativeAmount { field: "quantity" });
    }

    let unrounded = input
        .unit_price
        .mul_scalar(input.quantity.inner())
        .map_err(|source| money_error("line_extension.mul", source))?;
    let result = unrounded.round(input.scale, input.rounding);
    let trace = vec![TraceEntry::LineExtension {
        line_id: input.line_id,
        quantity: input.quantity,
        unit_price: TraceMoney::from_money(&input.unit_price),
        rounding: input.rounding,
        scale: input.scale,
        unrounded_amount: TraceMoney::from_money(&unrounded),
        result: TraceMoney::from_money(&result),
    }];

    Ok(Calculation { result, trace })
}

/// Apply one document-level allowance or charge to a base amount.
///
/// # Errors
///
/// Returns [`TaxCalculationError::NegativeAmount`] for a negative adjustment
/// amount or [`TaxCalculationError::Money`] for currency mismatch or overflow.
///
/// # Examples
///
/// ```
/// use invoicekit_ir::Iso4217Code;
/// use invoicekit_money::Money;
/// use invoicekit_tax_calculation::{
///     apply_allowance_charge, AllowanceChargeInput, AllowanceChargeKind,
/// };
/// use rust_decimal::Decimal;
///
/// let input = AllowanceChargeInput {
///     base_amount: Money::new(Decimal::new(10000, 2), Iso4217Code::new("EUR").unwrap()),
///     kind: AllowanceChargeKind::Allowance,
///     adjustment_amount: Money::new(Decimal::new(500, 2), Iso4217Code::new("EUR").unwrap()),
/// };
///
/// let calculation = apply_allowance_charge(input).unwrap();
/// assert_eq!(calculation.result.amount().to_string(), "95.00");
/// ```
pub fn apply_allowance_charge(
    input: AllowanceChargeInput,
) -> Result<Calculation<Money>, TaxCalculationError> {
    let AllowanceChargeInput {
        base_amount,
        kind,
        adjustment_amount,
    } = input;
    ensure_non_negative_money("adjustment_amount", &adjustment_amount)?;

    let result = match kind {
        AllowanceChargeKind::Allowance => base_amount
            .sub(&adjustment_amount)
            .map_err(|source| money_error("allowance.sub", source))?,
        AllowanceChargeKind::Charge => base_amount
            .add(&adjustment_amount)
            .map_err(|source| money_error("charge.add", source))?,
    };
    let trace = vec![TraceEntry::AllowanceCharge {
        kind,
        base_amount: TraceMoney::from_money(&base_amount),
        adjustment_amount: TraceMoney::from_money(&adjustment_amount),
        result: TraceMoney::from_money(&result),
    }];

    Ok(Calculation { result, trace })
}

/// Calculate a tax category subtotal.
///
/// The tax amount is `taxable_amount * (tax_rate / 100)`, rounded with the
/// supplied scale and rounding policy.
///
/// # Errors
///
/// Returns [`TaxCalculationError::MissingRequiredField`] for a blank category,
/// [`TaxCalculationError::NegativeAmount`] for a negative tax rate,
/// [`TaxCalculationError::InvalidScale`] for unsupported decimal scales, or
/// [`TaxCalculationError::Money`] for decimal overflow.
///
/// # Examples
///
/// ```
/// use invoicekit_ir::{DecimalValue, Iso4217Code};
/// use invoicekit_money::{Money, Rounding};
/// use invoicekit_tax_calculation::{calculate_tax_subtotal, TaxSubtotalInput};
/// use rust_decimal::Decimal;
///
/// let input = TaxSubtotalInput {
///     category_code: "S".to_owned(),
///     taxable_amount: Money::new(Decimal::new(10000, 2), Iso4217Code::new("EUR").unwrap()),
///     tax_rate: DecimalValue::new(Decimal::new(1900, 2)),
///     scale: 2,
///     rounding: Rounding::HalfUp,
/// };
///
/// let calculation = calculate_tax_subtotal(input).unwrap();
/// assert_eq!(calculation.result.tax_amount.amount().to_string(), "19.00");
/// ```
pub fn calculate_tax_subtotal(
    input: TaxSubtotalInput,
) -> Result<Calculation<TaxCategorySubtotal>, TaxCalculationError> {
    validate_non_empty(&input.category_code, "category_code")?;
    validate_scale(input.scale)?;
    if input.tax_rate.inner().is_sign_negative() {
        return Err(TaxCalculationError::NegativeAmount { field: "tax_rate" });
    }

    let rate_factor = input
        .tax_rate
        .inner()
        .checked_div(Decimal::from(100_u8))
        .ok_or(TaxCalculationError::Money {
            operation: "tax_subtotal.rate_factor",
            source: MoneyError::Overflow {
                operation: "tax_subtotal.rate_factor",
            },
        })?;
    let unrounded_tax_amount = input
        .taxable_amount
        .mul_scalar(rate_factor)
        .map_err(|source| money_error("tax_subtotal.mul", source))?;
    let tax_amount = unrounded_tax_amount.round(input.scale, input.rounding);
    let result = TaxCategorySubtotal {
        category_code: input.category_code,
        taxable_amount: input.taxable_amount,
        tax_rate: input.tax_rate,
        tax_amount,
    };
    let trace = vec![TraceEntry::TaxSubtotal {
        category_code: result.category_code.clone(),
        taxable_amount: TraceMoney::from_money(&result.taxable_amount),
        tax_rate: result.tax_rate.clone(),
        rate_factor: DecimalValue::new(rate_factor),
        rounding: input.rounding,
        scale: input.scale,
        unrounded_tax_amount: TraceMoney::from_money(&unrounded_tax_amount),
        tax_amount: TraceMoney::from_money(&result.tax_amount),
    }];

    Ok(Calculation { result, trace })
}

/// Calculate invoice payable totals.
///
/// # Errors
///
/// Returns [`TaxCalculationError::NegativeAmount`] for a negative prepaid
/// amount or [`TaxCalculationError::Money`] for currency mismatch or overflow.
///
/// # Examples
///
/// ```
/// use invoicekit_ir::{DecimalValue, Iso4217Code};
/// use invoicekit_money::Money;
/// use invoicekit_tax_calculation::{
///     calculate_payable_amount, PayableAmountInput, TaxCategorySubtotal,
/// };
/// use rust_decimal::Decimal;
///
/// let currency = Iso4217Code::new("EUR").unwrap();
/// let input = PayableAmountInput {
///     line_extension_total: Money::new(Decimal::new(10000, 2), currency.clone()),
///     allowance_total: Some(Money::new(Decimal::new(500, 2), currency.clone())),
///     charge_total: Some(Money::new(Decimal::new(250, 2), currency.clone())),
///     tax_subtotals: vec![TaxCategorySubtotal {
///         category_code: "S".to_owned(),
///         taxable_amount: Money::new(Decimal::new(9750, 2), currency.clone()),
///         tax_rate: DecimalValue::new(Decimal::new(1900, 2)),
///         tax_amount: Money::new(Decimal::new(1853, 2), currency.clone()),
///     }],
///     prepaid_amount: None,
/// };
///
/// let calculation = calculate_payable_amount(input).unwrap();
/// assert_eq!(calculation.result.payable_amount.amount().to_string(), "116.03");
/// ```
pub fn calculate_payable_amount(
    input: PayableAmountInput,
) -> Result<Calculation<PayableBreakdown>, TaxCalculationError> {
    if let Some(prepaid) = &input.prepaid_amount {
        ensure_non_negative_money("prepaid_amount", prepaid)?;
    }

    let zero = input.line_extension_total.zero_like();
    let allowance_total = input.allowance_total.unwrap_or_else(|| zero.clone());
    let charge_total = input.charge_total.unwrap_or_else(|| zero.clone());
    let prepaid_amount = input.prepaid_amount.unwrap_or_else(|| zero.clone());

    ensure_non_negative_money("allowance_total", &allowance_total)?;
    ensure_non_negative_money("charge_total", &charge_total)?;

    let after_allowance = input
        .line_extension_total
        .sub(&allowance_total)
        .map_err(|source| money_error("payable.sub_allowance", source))?;
    let tax_exclusive_amount = after_allowance
        .add(&charge_total)
        .map_err(|source| money_error("payable.add_charge", source))?;

    let mut tax_total = zero;
    for subtotal in &input.tax_subtotals {
        tax_total = tax_total
            .add(&subtotal.tax_amount)
            .map_err(|source| money_error("payable.add_tax", source))?;
    }

    let tax_inclusive_amount = tax_exclusive_amount
        .add(&tax_total)
        .map_err(|source| money_error("payable.add_tax_total", source))?;
    let payable_amount = tax_inclusive_amount
        .sub(&prepaid_amount)
        .map_err(|source| money_error("payable.sub_prepaid", source))?;

    let tax_subtotals = input
        .tax_subtotals
        .iter()
        .map(|subtotal| TaxSubtotalTracePart {
            category_code: subtotal.category_code.clone(),
            taxable_amount: TraceMoney::from_money(&subtotal.taxable_amount),
            tax_amount: TraceMoney::from_money(&subtotal.tax_amount),
            tax_rate: subtotal.tax_rate.clone(),
        })
        .collect();
    let result = PayableBreakdown {
        line_extension_total: input.line_extension_total,
        allowance_total,
        charge_total,
        tax_exclusive_amount,
        tax_total,
        tax_inclusive_amount,
        prepaid_amount,
        payable_amount,
    };
    let trace = vec![TraceEntry::PayableAmount {
        line_extension_total: TraceMoney::from_money(&result.line_extension_total),
        allowance_total: TraceMoney::from_money(&result.allowance_total),
        charge_total: TraceMoney::from_money(&result.charge_total),
        tax_exclusive_amount: TraceMoney::from_money(&result.tax_exclusive_amount),
        tax_subtotals,
        tax_total: TraceMoney::from_money(&result.tax_total),
        tax_inclusive_amount: TraceMoney::from_money(&result.tax_inclusive_amount),
        prepaid_amount: TraceMoney::from_money(&result.prepaid_amount),
        payable_amount: TraceMoney::from_money(&result.payable_amount),
    }];

    Ok(Calculation { result, trace })
}

/// Serialize trace entries as canonical RFC 8785 JSON.
///
/// # Errors
///
/// Returns [`TaxCalculationError::TraceSerialization`] when serde cannot
/// represent a trace as JSON, or
/// [`TaxCalculationError::TraceCanonicalization`] when canonicalization fails.
///
/// # Examples
///
/// ```
/// use invoicekit_tax_calculation::trace_to_canonical_json;
///
/// let canonical = trace_to_canonical_json(&[]).unwrap();
/// assert_eq!(canonical, "[]");
/// ```
pub fn trace_to_canonical_json(trace: &[TraceEntry]) -> Result<String, TaxCalculationError> {
    let value = serde_json::to_value(trace)
        .map_err(|source| TaxCalculationError::TraceSerialization { source })?;
    canonicalize_value(&value)
        .map_err(|source| TaxCalculationError::TraceCanonicalization { source })
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_tax_calculation::crate_name(), "invoicekit-tax-calculation");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-tax-calculation"
}

fn validate_scale(scale: u32) -> Result<(), TaxCalculationError> {
    if scale > MAX_DECIMAL_SCALE {
        return Err(TaxCalculationError::InvalidScale { scale });
    }
    Ok(())
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), TaxCalculationError> {
    if value.trim().is_empty() {
        return Err(TaxCalculationError::MissingRequiredField { field });
    }
    Ok(())
}

fn ensure_non_negative_money(
    field: &'static str,
    value: &Money,
) -> Result<(), TaxCalculationError> {
    if value.amount().is_sign_negative() {
        return Err(TaxCalculationError::NegativeAmount { field });
    }
    Ok(())
}

fn money_error(operation: &'static str, source: MoneyError) -> TaxCalculationError {
    TaxCalculationError::Money { operation, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::Iso4217Code;
    use proptest::prelude::*;

    fn eur(amount: Decimal) -> Money {
        Money::new(amount, Iso4217Code::new("EUR").unwrap())
    }

    fn usd(amount: Decimal) -> Money {
        Money::new(amount, Iso4217Code::new("USD").unwrap())
    }

    fn decimal(units: i64, scale: u32) -> DecimalValue {
        DecimalValue::new(Decimal::new(units, scale))
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-tax-calculation");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn line_extension_happy_path_records_replayable_trace() {
        let calculation = calculate_line_extension(LineExtensionInput {
            line_id: "1".to_owned(),
            quantity: decimal(250, 2),
            unit_price: eur(Decimal::new(4000, 2)),
            scale: 2,
            rounding: Rounding::HalfUp,
        })
        .unwrap();

        assert_eq!(calculation.result.amount().to_string(), "100.00");
        assert_eq!(calculation.trace.len(), 1);
        let canonical = trace_to_canonical_json(&calculation.trace).unwrap();
        assert_eq!(
            canonical,
            r#"[{"line_id":"1","operation":"line_extension","quantity":"2.50","result":{"amount":"100.00","currency":"EUR"},"rounding":"half_up","scale":2,"unit_price":{"amount":"40.00","currency":"EUR"},"unrounded_amount":{"amount":"100.0000","currency":"EUR"}}]"#
        );
    }

    #[test]
    fn allowance_subtracts_and_charge_adds() {
        let allowance = apply_allowance_charge(AllowanceChargeInput {
            base_amount: eur(Decimal::new(10000, 2)),
            kind: AllowanceChargeKind::Allowance,
            adjustment_amount: eur(Decimal::new(500, 2)),
        })
        .unwrap();
        let charge = apply_allowance_charge(AllowanceChargeInput {
            base_amount: allowance.result,
            kind: AllowanceChargeKind::Charge,
            adjustment_amount: eur(Decimal::new(250, 2)),
        })
        .unwrap();

        assert_eq!(charge.result.amount().to_string(), "97.50");
    }

    #[test]
    fn tax_subtotal_calculates_percent_with_rounding() {
        let calculation = calculate_tax_subtotal(TaxSubtotalInput {
            category_code: "S".to_owned(),
            taxable_amount: eur(Decimal::new(9999, 2)),
            tax_rate: decimal(1900, 2),
            scale: 2,
            rounding: Rounding::HalfUp,
        })
        .unwrap();

        assert_eq!(calculation.result.tax_amount.amount().to_string(), "19.00");
        assert_eq!(calculation.trace.len(), 1);
    }

    #[test]
    fn payable_amount_combines_all_components() {
        let calculation = calculate_payable_amount(PayableAmountInput {
            line_extension_total: eur(Decimal::new(10000, 2)),
            allowance_total: Some(eur(Decimal::new(500, 2))),
            charge_total: Some(eur(Decimal::new(250, 2))),
            tax_subtotals: vec![TaxCategorySubtotal {
                category_code: "S".to_owned(),
                taxable_amount: eur(Decimal::new(9750, 2)),
                tax_rate: decimal(1900, 2),
                tax_amount: eur(Decimal::new(1853, 2)),
            }],
            prepaid_amount: Some(eur(Decimal::new(1000, 2))),
        })
        .unwrap();

        assert_eq!(
            calculation.result.tax_exclusive_amount.amount().to_string(),
            "97.50"
        );
        assert_eq!(calculation.result.tax_total.amount().to_string(), "18.53");
        assert_eq!(
            calculation.result.payable_amount.amount().to_string(),
            "106.03"
        );
    }

    #[test]
    fn invalid_scale_is_rejected() {
        let err = calculate_line_extension(LineExtensionInput {
            line_id: "1".to_owned(),
            quantity: decimal(1, 0),
            unit_price: eur(Decimal::new(100, 2)),
            scale: 29,
            rounding: Rounding::HalfEven,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            TaxCalculationError::InvalidScale { scale: 29 }
        ));
    }

    #[test]
    fn blank_tax_category_is_rejected() {
        let err = calculate_tax_subtotal(TaxSubtotalInput {
            category_code: " ".to_owned(),
            taxable_amount: eur(Decimal::new(10000, 2)),
            tax_rate: decimal(1900, 2),
            scale: 2,
            rounding: Rounding::HalfUp,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            TaxCalculationError::MissingRequiredField {
                field: "category_code"
            }
        ));
    }

    #[test]
    fn negative_adjustment_is_rejected() {
        let err = apply_allowance_charge(AllowanceChargeInput {
            base_amount: eur(Decimal::new(10000, 2)),
            kind: AllowanceChargeKind::Allowance,
            adjustment_amount: eur(Decimal::new(-500, 2)),
        })
        .unwrap_err();

        assert!(matches!(
            err,
            TaxCalculationError::NegativeAmount {
                field: "adjustment_amount"
            }
        ));
    }

    #[test]
    fn currency_mismatch_surfaces_as_typed_error() {
        let err = calculate_payable_amount(PayableAmountInput {
            line_extension_total: eur(Decimal::new(10000, 2)),
            allowance_total: Some(usd(Decimal::new(500, 2))),
            charge_total: None,
            tax_subtotals: Vec::new(),
            prepaid_amount: None,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            TaxCalculationError::Money {
                operation: "payable.sub_allowance",
                ..
            }
        ));
    }

    #[test]
    fn trace_serialization_is_byte_stable_across_runs() {
        let input = TaxSubtotalInput {
            category_code: "S".to_owned(),
            taxable_amount: eur(Decimal::new(10000, 2)),
            tax_rate: decimal(1900, 2),
            scale: 2,
            rounding: Rounding::HalfUp,
        };

        let first = calculate_tax_subtotal(input.clone()).unwrap();
        let second = calculate_tax_subtotal(input).unwrap();

        assert_eq!(
            trace_to_canonical_json(&first.trace).unwrap(),
            trace_to_canonical_json(&second.trace).unwrap()
        );
    }

    proptest! {
        #[test]
        fn charges_are_commutative(
            base in 0_i64..=1_000_000,
            c1 in 0_i64..=100_000,
            c2 in 0_i64..=100_000,
        ) {
            let base = eur(Decimal::new(base, 2));
            let first = apply_allowance_charge(AllowanceChargeInput {
                base_amount: base.clone(),
                kind: AllowanceChargeKind::Charge,
                adjustment_amount: eur(Decimal::new(c1, 2)),
            }).unwrap();
            let first = apply_allowance_charge(AllowanceChargeInput {
                base_amount: first.result,
                kind: AllowanceChargeKind::Charge,
                adjustment_amount: eur(Decimal::new(c2, 2)),
            }).unwrap();

            let second = apply_allowance_charge(AllowanceChargeInput {
                base_amount: base,
                kind: AllowanceChargeKind::Charge,
                adjustment_amount: eur(Decimal::new(c2, 2)),
            }).unwrap();
            let second = apply_allowance_charge(AllowanceChargeInput {
                base_amount: second.result,
                kind: AllowanceChargeKind::Charge,
                adjustment_amount: eur(Decimal::new(c1, 2)),
            }).unwrap();

            prop_assert_eq!(first.result, second.result);
        }

        #[test]
        fn tax_totals_are_associative_over_category_grouping(
            amount_a in 0_i64..=1_000_000,
            amount_b in 0_i64..=1_000_000,
            amount_c in 0_i64..=1_000_000,
        ) {
            let tax_a = TaxCategorySubtotal {
                category_code: "A".to_owned(),
                taxable_amount: eur(Decimal::new(amount_a, 2)),
                tax_rate: decimal(1900, 2),
                tax_amount: eur(Decimal::new(amount_a, 2)),
            };
            let tax_b = TaxCategorySubtotal {
                category_code: "B".to_owned(),
                taxable_amount: eur(Decimal::new(amount_b, 2)),
                tax_rate: decimal(700, 2),
                tax_amount: eur(Decimal::new(amount_b, 2)),
            };
            let tax_c = TaxCategorySubtotal {
                category_code: "C".to_owned(),
                taxable_amount: eur(Decimal::new(amount_c, 2)),
                tax_rate: decimal(0, 0),
                tax_amount: eur(Decimal::new(amount_c, 2)),
            };

            let left = calculate_payable_amount(PayableAmountInput {
                line_extension_total: eur(Decimal::ZERO),
                allowance_total: None,
                charge_total: None,
                tax_subtotals: vec![tax_a.clone(), tax_b.clone(), tax_c.clone()],
                prepaid_amount: None,
            }).unwrap();
            let right = calculate_payable_amount(PayableAmountInput {
                line_extension_total: eur(Decimal::ZERO),
                allowance_total: None,
                charge_total: None,
                tax_subtotals: vec![tax_c, tax_b, tax_a],
                prepaid_amount: None,
            }).unwrap();

            prop_assert_eq!(left.result.tax_total, right.result.tax_total);
            prop_assert_eq!(left.result.payable_amount, right.result.payable_amount);
        }

        #[test]
        fn line_rounding_is_consistent_across_runs(
            quantity_minor in 0_i64..=1_000_000,
            unit_price_minor in 0_i64..=1_000_000,
        ) {
            let input = LineExtensionInput {
                line_id: "prop".to_owned(),
                quantity: decimal(quantity_minor, 2),
                unit_price: eur(Decimal::new(unit_price_minor, 2)),
                scale: 2,
                rounding: Rounding::HalfEven,
            };

            let first = calculate_line_extension(input.clone()).unwrap();
            let second = calculate_line_extension(input).unwrap();

            prop_assert_eq!(first.result, second.result);
            prop_assert_eq!(
                trace_to_canonical_json(&first.trace).unwrap(),
                trace_to_canonical_json(&second.trace).unwrap()
            );
        }
    }
}
