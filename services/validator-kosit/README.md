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
