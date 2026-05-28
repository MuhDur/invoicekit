# Peppol BYOK — operator runbook (T-091 / T-092 / T-093 / T-094 / T-095)

InvoiceKit ships Peppol delivery in **bring-your-own-credentials**
mode. You — not InvoiceKit — own the Peppol identity. You provide
the X.509 certificate, the matching key, the Access Point endpoint
URL, and the SML mode; InvoiceKit drives transmission against
your credentials.

The `crates/transmit-peppol-byok` substrate is the shared
configuration surface. Each downstream transport (`partner`,
`phase4`, `native-as4`) accepts a `PeppolCredentials` bundle and
maps it to its native config.

## Why BYOK?

The Peppol network requires every Access Point to hold a signed
agreement with a Peppol Authority and a certificate issued by a
Peppol-recognised CA. InvoiceKit deliberately does not hold that
identity for you. Three reasons:

1. **Trust toolkit, not managed pipeline.** Direction A says we
   ship open code your evidence flows through, not a SaaS your
   evidence ends in.
2. **One-step audit.** When the auditor asks "who pressed send?",
   the answer is your identity, on your hardware, with your key.
3. **No data lock-in.** You can swap from a partner AP to phase4
   to native AS4 by editing one JSON file — no contract migration.

## Credentials file shape

```json
{
  "participant_id": {
    "scheme": "iso6523-actorid-upis",
    "value": "0192:991825827"
  },
  "cert_pem_path": "ap-cert.pem",
  "key_pem_path": "ap-key.pem",
  "key_passphrase_env": "PEPPOL_KEY_PASSPHRASE",
  "endpoint_url": "https://ap.example.com/as4",
  "sml_mode": "test",
  "transport": "partner",
  "labels": {
    "partner.vendor": "storecove"
  }
}
```

Fields:

| field | meaning |
| --- | --- |
| `participant_id.scheme` | Peppol participant id scheme (almost always `iso6523-actorid-upis`). |
| `participant_id.value` | `<icd>:<value>` (e.g. `0192:991825827` for a Norwegian organisation number). |
| `cert_pem_path` | X.509 certificate PEM. Relative paths resolve against the credentials JSON. |
| `key_pem_path` | Matching private key PEM. |
| `key_passphrase_env` | Name of an env var holding the passphrase. The credentials file never contains the passphrase itself — that lets you commit the credentials file safely. |
| `endpoint_url` | AS4 endpoint URL. Must be `https://`. Doctor rejects `http://`. |
| `sml_mode` | One of `test`, `acceptance`, `production`. |
| `transport` | One of `partner`, `phase4`, `native-as4`. Picks which crate drives delivery. |
| `labels` | Free-form string map. `partner.vendor` is required when `transport = "partner"`. |

## Doctor

```bash
invoicekit peppol doctor --credentials ./peppol-creds.json
```

Runs the [`PeppolDoctor`] checks against the file:

- `cert.exists` / `cert.readable` / `cert.pem-shaped`
- `key.exists` / `key.readable` / `key.pem-shaped`
- `endpoint.https` (rejects http://)
- `participant.well-formed` (re-parses the `scheme::value` wire form)
- `sml.mode-set`

Exit 0 on every check passing; exit 1 when at least one fails;
exit 2 on usage error or unreadable credentials file. Pass
`--json` for machine-readable output.

```bash
invoicekit peppol show --credentials ./peppol-creds.json
```

Pretty-prints the bundle for human review. Never echoes
passphrases — only the name of the env var.

## Three transports

### `partner` — hosted Access Point

Customer holds a contract with Storecove / ecosio / B2BRouter.
The partner vendor goes in `labels["partner.vendor"]`. SML mode
`test` and `acceptance` both map to the vendor's sandbox;
`production` flips sandbox off. Credentials only need to provide
the API endpoint + a key; the partner adapter resolves the
Peppol certificate on their side.

### `phase4` — self-hosted phase4 sidecar

Customer runs the JVM `phase4` sidecar with their own cert and
SML registration. `endpoint_url` is the sidecar's JSON-RPC URL.
`sml_mode` `test` / `acceptance` collapse to `acceptance` (phase4
has no separate Test mode); `production` maps through.

### `native-as4` — pure-Rust AS4 stack

Customer's cert + key drive InvoiceKit's native sender and
receiver. `endpoint_url` is the AS4 inbox we POST to (sender) or
bind on (receiver). SML mapping is identical to phase4.

## Peppol Test Bed — the unblock

The Peppol Test Bed is free, public, and requires no partner
contract. It uses the Acceptance SML. With Test Bed credentials
you can run the end-to-end conformance tests:

1. Apply for a Test Bed identity at
   [peppol.eu/contact](https://peppol.eu/contact). Free.
2. Receive your test certificate + participant id.
3. Write a `peppol-creds.json` per the shape above with
   `sml_mode = "test"`.
4. Run `invoicekit peppol doctor --credentials peppol-creds.json`
   to confirm parse + PEM shape.
5. Enable the `peppol-test-bed` cargo feature on the receive +
   send crates to compile the Test Bed integration tests:

   ```bash
   cargo test --features peppol-test-bed \
     -p invoicekit-transmit-peppol-native-as4 \
     -p invoicekit-transmit-peppol-native-as4-receive
   ```

6. Set `PEPPOL_TEST_BED_CREDENTIALS=/path/to/peppol-creds.json`
   in your env before running the tests.

The `.github/workflows/peppol-test-bed.yml` workflow runs these
on a schedule when the `PEPPOL_TEST_BED_CREDENTIALS_JSON` repo
secret is set.

## Migration from a managed-Peppol vendor

If you previously delegated Peppol identity to a SaaS:

1. Get your own Test Bed identity (free, no contract).
2. Write the credentials file.
3. Doctor it.
4. Flip `transport` between `partner` / `phase4` / `native-as4`
   freely — no other config changes.

Production rollout: once the Test Bed integration tests are
green, apply for a Peppol production certificate, change
`sml_mode` to `production`, and re-doctor. No code changes.

## What InvoiceKit never has

- Your private key.
- Your Peppol participant id (beyond what's in your credentials
  file on your hardware).
- The contractual relationship with the Peppol Authority.

If InvoiceKit's hosted infrastructure disappeared tomorrow, your
Peppol delivery continues uninterrupted: open-source binary +
your credentials = your AP.
