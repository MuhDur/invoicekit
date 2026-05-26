//! `invoicekit` CLI entry point. Backed by the `invoicekit_cli` library
//! crate so the dispatcher can be exercised as a library from tests.

fn main() {
    // Workspace-identity handshake. Returning silently is the documented
    // contract for the no-arg invocation; downstream beads add subcommands.
    let _ = invoicekit_cli::crate_name();
}

#[cfg(test)]
mod tests {
    #[test]
    fn main_returns_without_panic() {
        super::main();
    }
}
