// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-045 BR-DE-* Schematron rule coverage matrix.
//
// XRechnung 3.x adds ~30 German-specific business rules on top
// of EN 16931's BR-* and BR-CO-* layer. KoSIT's scenarios bundle
// owns the runtime enforcement; this module documents the
// coverage state per rule so the rulepack registry can publish
// it alongside the EN16931 BR coverage matrix shipped in PR #73.

/// One row of the BR-DE-* coverage matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrDeRow {
    /// Rule identifier (e.g. `BR-DE-15`).
    pub rule_id: &'static str,
    /// Short human-readable summary of what the rule constrains.
    pub summary: &'static str,
    /// True when the rust-native projection already enforces this
    /// rule structurally (e.g. by forcing the field to be present).
    pub rust_enforced: bool,
    /// True when the rule is validated by KoSIT at runtime against
    /// the projected XML.
    pub kosit_enforced: bool,
}

/// BR-DE-* coverage rows. Hand-maintained against KoSIT
/// XRechnung 3.0.2 spec; updates land alongside scenarios
/// bundle bumps.
pub const BR_DE_COVERAGE: &[BrDeRow] = &[
    BrDeRow {
        rule_id: "BR-DE-1",
        summary: "Invoice must carry the buyer reference (BT-10) for B2G recipients.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-2",
        summary: "Contact point group (BG-6) is mandatory for the supplier.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-3",
        summary: "Supplier contact name (BT-41) is mandatory.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-4",
        summary: "Supplier contact telephone (BT-42) is mandatory.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-5",
        summary: "Supplier contact email (BT-43) is mandatory.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-6",
        summary: "Supplier postal city (BT-37) is mandatory.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-7",
        summary: "Supplier postal code (BT-38) is mandatory.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-8",
        summary: "Buyer postal city (BT-52) is mandatory.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-9",
        summary: "Buyer postal code (BT-53) is mandatory.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-10",
        summary: "Country code (BT-40 / BT-55) must be a known ISO 3166-1 alpha-2.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-13",
        summary: "Payment terms (BT-20) must use the supported XRechnung syntax when present.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-14",
        summary:
            "Document-level allowance/charge categories must be coded with the EN 16931 codelist.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-15",
        summary: "Leitweg-ID is mandatory in BT-10 for B2G recipients.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-16",
        summary: "Payee party (BG-10) is mandatory when the payee differs from the supplier.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-17",
        summary: "Invoice type code (BT-3) must be one of the XRechnung-allowed UNCL1001 values.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-18",
        summary: "Tax point date (BT-7) format is YYYY-MM-DD when present.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-19",
        summary: "Document-level reference (BT-12) text must be no longer than 100 characters.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-21",
        summary: "Specification identifier (BT-24) must be the XRechnung 3.x CustomizationID URN.",
        rust_enforced: true,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-23",
        summary: "Bank assigned creditor identifier (BT-90) is mandatory for SEPA direct debit.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-26",
        summary: "Tax accounting currency (BT-6) must match the document currency unless allowed.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-27",
        summary: "Telephone numbers (BT-42 / BT-57) must be at least 3 characters long.",
        rust_enforced: false,
        kosit_enforced: true,
    },
    BrDeRow {
        rule_id: "BR-DE-28",
        summary: "Electronic addresses (BT-43 / BT-58) must contain an `@`.",
        rust_enforced: false,
        kosit_enforced: true,
    },
];

/// Number of BR-DE-* rows shipped in this matrix.
#[must_use]
pub const fn br_de_row_count() -> usize {
    BR_DE_COVERAGE.len()
}

/// Number of rules currently enforced by the rust-native projection.
#[must_use]
pub fn br_de_rust_enforced_count() -> usize {
    BR_DE_COVERAGE
        .iter()
        .filter(|row| row.rust_enforced)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_is_non_empty() {
        assert!(br_de_row_count() >= 22);
    }

    #[test]
    fn rust_enforcement_coverage_meets_minimum() {
        // We currently enforce ~10 rules natively (address, type,
        // customization, leitweg). The rest rely on KoSIT.
        assert!(br_de_rust_enforced_count() >= 9);
    }

    #[test]
    fn rule_ids_are_unique() {
        let mut ids: Vec<&str> = BR_DE_COVERAGE.iter().map(|r| r.rule_id).collect();
        ids.sort_unstable();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate BR-DE rule id");
    }
}
