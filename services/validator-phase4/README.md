# services/validator-phase4

JVM sidecar service — **not** a Rust workspace member.

`validator-phase4` wraps Helger's `phase4` AS4 reference
implementation behind the InvoiceKit JSON-RPC contract documented
in [`docs/operators/PHASE4-REFERENCE-ADAPTER.md`](../../docs/operators/PHASE4-REFERENCE-ADAPTER.md).

- Java runtime: Eclipse Temurin 21.
- Upstream library: [`com.helger.phase4:phase4-peppol-client`](https://github.com/phax/phase4).
- SML lookup: `com.helger.peppol:peppol-smp-client`.
- Contract surface: `transmit` / `receive` / `status` / `health`,
  same four-method JSON-RPC shape the validator sidecars use.

## Scaffold vs production

The current `Phase4Server.java` is a **scaffold**: every method
returns the minimum JSON shape the Rust `Phase4Adapter` expects so
the contract tests can run end-to-end, but the real outbound AS4
push is not wired yet. Wiring requires:

1. An OpenPeppol AP certificate (P12 + passphrase).
2. SML registration of the AP's DNS endpoint.
3. The `phase4-peppol-client` runtime configured against those
   credentials.

All three land in follow-up beads
(`invoices-t-092-impl-{sidecar-runtime,adapter,oracle}`); the AP
certificate has a 4-8 week lead time.

## Build & run

```bash
DOCKER_BUILDKIT=1 docker build \
  -f services/validator-phase4/Dockerfile \
  -t invoicekit/phase4-server:scaffold .

docker run --rm -p 8090:8090 \
  -e PEPPOL_AP_SML_MODE=acceptance \
  invoicekit/phase4-server:scaffold
```

Local Maven test loop:

```bash
cd services/validator-phase4
mvn test
```

## Environment

| Variable | Purpose | Default |
| --- | --- | --- |
| `INVOICEKIT_PHASE4_PORT` | Listen port. | `8090` |
| `PEPPOL_AP_SML_MODE` | `acceptance` or `production`. | `acceptance` |
| `PEPPOL_AP_CERT_P12` | Path to AP certificate bundle. | *(unset; required in production)* |
| `PEPPOL_AP_CERT_PASS` | Bundle passphrase. | *(unset; required in production)* |

Scaffolded by bead **invoices-t-001-cargo-workspace-xos**;
populated by **invoices-t-092-impl-sidecar-3bol**.
