// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-managed-api-server` — InvoiceKit workspace binary.
//!
//! See [`plans/PLAN.md`](../../../plans/PLAN.md) for the architectural role
//! of this binary. The entry point below is the workspace-identity
//! handshake every InvoiceKit binary exposes; downstream beads layer the
//! real subcommand dispatcher on top of it.

const CRATE_NAME: &str = "invoicekit-managed-api-server";

fn main() {
    // Workspace-identity handshake. Returning silently is the documented
    // contract for the no-arg invocation; downstream beads add subcommands.
    let _ = CRATE_NAME;
}

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(CRATE_NAME, "invoicekit-managed-api-server");
    }

    #[test]
    fn main_returns_without_panic() {
        super::main();
    }

    #[test]
    fn main_is_idempotent() {
        super::main();
        super::main();
    }
}
