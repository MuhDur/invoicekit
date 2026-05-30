<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-transmit-email

Reserved workspace slot for email-based invoice transmission. No transport ships yet.

This crate is currently a scaffold. Its only public surface is the workspace-identity helper `crate_name()` that every InvoiceKit crate carries; the email delivery logic is not implemented. The module doc-comment states the surface is the stable identity helper and that "downstream beads layer their domain logic on top of it without touching this surface."

## Capabilities

- Returns the canonical Cargo package name, `"invoicekit-transmit-email"`, via `crate_name()`. Release tooling and the bead-correlation reports use this to map runtime log records back to the originating crate without parsing `Cargo.toml` at runtime.

That is the entire public API. There is no message assembly, no Simple Mail Transfer Protocol (SMTP) client, no attachment handling, no credential handling, and no transport code in this crate today.

## Mode

Neither real nor mock — unimplemented. No email is sent and no connection is opened. There is no transport (real or simulated), and no credential model has been declared in the source.

When email transmission is built, the live path will need, at minimum, a mail submission endpoint and the sender's credentials. None of that exists here yet; do not treat this crate as a working email transport.

## Residuals

- The module doc-comment defers all domain logic to "downstream beads," which have not landed. The crate exposes only the identity helper.
- No transport standard, credential source, or delivery semantics are named in the source.

## References

- [`plans/PLAN.md`](../../plans/PLAN.md) — the architectural role of this crate, as cited in the module doc-comment.

(No specification or standard URLs are cited in the crate source.)

## License

Apache-2.0.
