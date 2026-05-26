# services/validator-zatca

JVM sidecar service — not a Rust workspace member.

This directory is reserved for the validator-zatca Java service per
`plans/PLAN.md` §4.1 / §2.6. It will be implemented in a separate bead
(see the InvoiceKit beads graph) and built as a containerized service
called from the engine over JSON-RPC.

Scaffolded by bead **invoices-t-001-cargo-workspace-xos**.
