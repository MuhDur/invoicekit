# invoicekit-transmit-peppol-byok

Bring-your-own-credentials substrate for Peppol AS4 transmission. The customer owns the Peppol identity (X.509 certificate, key, Access Point endpoint, SML mode); InvoiceKit drives transmission against it.

This crate does not transmit anything itself. It is the configuration seam plus a preflight doctor that feeds three downstream transport adapters — `partner`, `phase4`, and `native-as4`.

## Capabilities

- `PeppolCredentials` — typed BYOK credential bundle: participant id, paths to the X.509 PEM certificate and matching private-key PEM, optional `key_passphrase_env` (an env-var *name*, never the passphrase itself), Access Point endpoint URL, `SmlMode` (test / acceptance / production), the chosen `TransportKind`, and free-form routing `labels`.
- `PeppolCredentials::from_json_file` — load credentials from a JSON file; relative `cert_pem_path` / `key_pem_path` are resolved against the file's parent directory. Does not validate cert/key contents.
- `PeppolCredentials::resolve_passphrase` — resolve the key passphrase through a caller-supplied environment lookup; errors when a named env var is unset.
- `ParticipantId::parse` / `to_wire` — parse and emit the `<scheme>::<value>` Peppol participant wire string (e.g. `iso6523-actorid-upis::0192:991825827`).
- `TransportKind` — selects which downstream adapter the same credentials drive: `Partner` (hosted partner AP — Tickstar / Storecove / Pagero), `Phase4` (self-hosted phase4 sidecar over JSON-RPC), or `NativeAs4` (pure-Rust native AS4 stack).
- `PeppolDoctor::check` — offline preflight against a credentials bundle, returning a full `DoctorReport`: certificate and key files exist, are readable, and are PEM-shaped (`CERTIFICATE` block for the cert; `PRIVATE KEY` / `RSA PRIVATE KEY` / `EC PRIVATE KEY` for the key); the endpoint is a well-formed `https://` URL; the participant id parses; the SML mode is set. The report lists every row so an operator can fix all problems in one pass.
- `DoctorFs` trait with a `StdFs` implementation — the filesystem seam the doctor reads through.

## Mode

**Bring-your-own-credentials. No transmission and no live integration ships in this crate.** It carries the customer's Peppol identity as configuration and validates it before any adapter runs. The customer holds the certificate, the private key, the Access Point endpoint, and the SML mode; InvoiceKit never holds the Peppol identity.

The doctor is **offline and filesystem-only**. It checks file existence, readability, PEM block shape (string matching on the BEGIN/END markers), and that the endpoint URL starts with `https://`. It does **not** parse certificate or key cryptography, does not check chains or expiry, and makes no network calls.

The live transmission path requires the downstream adapters that consume `PeppolCredentials`:

- `invoicekit-transmit-peppol-partner` — a hosted partner Access Point contract and endpoint.
- `invoicekit-transmit-peppol-phase4` — a customer-hosted phase4 sidecar.
- `invoicekit-transmit-peppol-native-as4` — the pure-Rust native AS4 stack.

The Peppol Test Bed (`SmlMode::Test`) is the free, public on-ramp and needs no partner contract.

## Residuals

From the module documentation:

- Native AS4 is a research track. Year-1 live Peppol delivery uses a partner Access Point plus `phase4` as a reference adapter (architectural commitment in `AGENTS.md`).
- `from_json_file` does not validate certificate or key contents — `PeppolDoctor::check` is the separate, non-cryptographic validation step.
- The doctor's PEM check is a structural shape check (BEGIN/END markers), not cryptographic verification.

## References

- Peppol SML (Service Metadata Locator) modes: test, acceptance, production.
- Peppol participant identifier scheme `iso6523-actorid-upis`.
- AS4 (the Peppol transport profile the configured endpoint receives).

No specification URLs are present in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace.
