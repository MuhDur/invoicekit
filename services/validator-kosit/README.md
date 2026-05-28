# services/validator-kosit

JVM sidecar service - not a Rust workspace member.

`validator-kosit` runs the shared InvoiceKit validator sidecar host with the
`jvm:kosit` backend and the KoSIT validator dependency boundary:

- Java runtime: Eclipse Temurin 21.
- Oracle dependency: `org.kosit:validator:1.6.2`.
- Startup class check: `de.kosit.validationtool.api.Check`.
- Contract: [../validator-rpc.md](../validator-rpc.md).

Build from the repository root:

```bash
DOCKER_BUILDKIT=1 docker build -f services/validator-kosit/Dockerfile -t invoicekit/validator-kosit:ci .
```

The image pins the official KoSIT XRechnung validator configuration release
`v2026-01-31`:

- Distribution:
  `xrechnung-3.0.2-validator-configuration-2026-01-31.zip`
- Source:
  `https://github.com/itplr-kosit/validator-configuration-xrechnung/releases/download/v2026-01-31/xrechnung-3.0.2-validator-configuration-2026-01-31.zip`
- SHA-256:
  `6a5a5911a421b25fbc423f62f93f894df7b236f5d73ca4f84bb222a945082704`
- Built-in scenarios path:
  `/app/kosit-xrechnung/scenarios.xml`

Override `INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS` only when testing a different
local configuration bundle.
