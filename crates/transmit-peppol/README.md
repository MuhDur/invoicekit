# invoicekit-transmit-peppol

Peppol delivery slot in the InvoiceKit transmit family. This crate is a scaffold: it carries no transmission code yet.

## Capabilities

This crate exposes one public item — `crate_name() -> &'static str`, the workspace-identity helper every InvoiceKit crate carries. It returns `"invoicekit-transmit-peppol"` for release tooling and bead-correlation reports.

It does not transmit anything. There is no transport, no AS4 client, no access-point integration, no credential handling, and no `GatewayAdapter` implementation in this crate. The module doc-comment names the architectural role only; the working code lives in sibling crates.

## Mode

Neither real nor mock — there is no transmission path here at all. The actual Peppol delivery work is implemented elsewhere in the workspace:

- `invoicekit-transmit-peppol-byok` — bring-your-own-credentials seam. The customer supplies the X.509 certificate, the matching key, the access-point endpoint URL, and the Service Metadata Locator (SML) mode (test / acceptance / production). InvoiceKit drives transmission against those credentials; it never holds a managed Peppol identity.
- `invoicekit-transmit-peppol-partner` — adapter for a hosted partner access point. Vendor REST mapping is largely stubbed; unit tests prove URL and body construction against a mock HTTP client, and the live `reqwest`-backed client is a follow-up.
- `invoicekit-transmit-peppol-phase4` — adapter over a `validator-phase4` JSON-RPC sidecar; the live client lands once an access-point certificate clears OpenPeppol onboarding.
- `invoicekit-transmit-peppol-native-as4` — the pure-Rust AS4 research track.

To do real Peppol delivery today, depend on `-byok` plus one of the transport adapters above, not on this crate.

## Residuals

The module doc-comment states only that the crate ships the stable workspace-identity helper and that "downstream beads layer their domain logic on top of it without touching this surface." No transmission capability is documented or implemented here.

## References

- `plans/PLAN.md` — the architectural role of this crate (referenced from the module doc-comment).

No Peppol, AS4, or SML specification is cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace.
