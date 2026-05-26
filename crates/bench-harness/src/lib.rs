//! `invoicekit-bench-harness` — cross-crate criterion benchmark harness.
//!
//! This crate is the central home for the InvoiceKit performance regression
//! budget (T-007). Each `[[bench]]` target tracks one named operation that the
//! CI bench workflow compares against the rolling baseline; regressions beyond
//! the per-operation threshold in `tools/perf-budget/budget.toml` fail the
//! pull-request build.
//!
//! The crate is intentionally a thin shell: the benches under `benches/` are
//! the real workload, and the only public item here is the workspace-identity
//! helper every InvoiceKit crate carries.

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_bench_harness::crate_name(), "invoicekit-bench-harness");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-bench-harness"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-bench-harness");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }
}
