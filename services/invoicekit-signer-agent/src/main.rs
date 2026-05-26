//! `invoicekit-signer-agent` — InvoiceKit workspace binary.
//!
//! See [`plans/PLAN.md`](../../../plans/PLAN.md) for the architectural role
//! of this binary. The entry point below is the workspace-identity
//! handshake every InvoiceKit binary exposes; downstream beads layer the
//! real subcommand dispatcher on top of it.

const CRATE_NAME: &str = "invoicekit-signer-agent";

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
        assert_eq!(CRATE_NAME, "invoicekit-signer-agent");
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
