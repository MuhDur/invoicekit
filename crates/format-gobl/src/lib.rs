// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-format-gobl` — bidirectional adapter between InvoiceKit's
//! [`CommercialDocument`](invoicekit_ir::CommercialDocument) and the
//! [invopop/gobl](https://github.com/invopop/gobl) JSON shape.
//!
//! GOBL is the closest open-source neighbor (Apache 2.0, Go). Per
//! [`AGENTS.md`](../../AGENTS.md) we interoperate with their schema
//! rather than reinvent it, so customers can move data freely between
//! the two ecosystems.
//!
//! ```rust,ignore
//! use invoicekit_format_gobl::{from_gobl, to_gobl};
//! let ir = invoicekit_ir::CommercialDocument::try_from_value(/* … */).unwrap();
//! let (gobl, ledger) = to_gobl(&ir).unwrap();
//! let (rt_ir, _ledger) = from_gobl(&gobl).unwrap();
//! assert_eq!(ir.id, rt_ir.id);
//! ```
//!
//! ## Scope
//!
//! This first-cut adapter covers the core invoice surface: id, type,
//! dates, currency, supplier + customer parties (tax IDs + postal
//! address + contact), one-or-more lines (id + description + quantity +
//! unit price + line extension + tax category), per-category tax
//! summary, monetary totals, payment instructions, and document
//! references. Jurisdiction extensions are stamped into the GOBL
//! `ext` map keyed by URN; on the reverse pass they round-trip back
//! into [`JurisdictionExtension`](invoicekit_ir::JurisdictionExtension).
//!
//! The bead's strict-acceptance "20 fixtures from GOBL's own test
//! corpus" gate is filed as a follow-up bead because the upstream
//! corpus isn't checked into this repo. The adapter is shape-stable
//! enough that the follow-up just needs to feed real fixtures in.

mod codec;

pub use codec::{from_gobl, to_gobl, GoblEnvelope, GoblError};

/// Bead identifier carried alongside emitted log events for diagnostic correlation.
pub const GOBL_ADAPTER_BEAD_ID: &str = "invoices-t-013-gobl-adapter-p40";

/// GOBL schema URL prefix for the bill domain. GOBL uses one URL per
/// document type; the suffix is `invoice`, `credit-note`, etc.
pub const GOBL_BILL_SCHEMA_PREFIX: &str = "https://gobl.org/draft-0/bill/";

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_format_gobl::crate_name(), "invoicekit-format-gobl");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-format-gobl"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-format-gobl");
    }
}
