// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-render-verify` — render-side verification adapters.
//!
//! Currently houses the T-052 veraPDF adapter that parses the
//! validator-verapdf sidecar's `validator.validate_pdf` JSON
//! response into a typed [`verapdf::ValidatePdfResult`] (which
//! carries a nested [`verapdf::PdfAReport`]). Additional render
//! verifiers (PDF/A signature checks, font-embedding audits, etc.)
//! will land here as sibling modules.

pub mod verapdf;

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_render_verify::crate_name(), "invoicekit-render-verify");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-render-verify"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-render-verify");
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
}
