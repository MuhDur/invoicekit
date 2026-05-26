# services/validator-phive

JVM sidecar service - not a Rust workspace member.

`validator-phive` runs the shared InvoiceKit validator sidecar host with the
`jvm:phive` backend and the Helger phive Peppol rule dependency boundary:

- Java runtime: Eclipse Temurin 21.
- Oracle dependency: `com.helger.phive.rules:phive-rules-peppol:3.2.2`.
- Startup class check: `com.helger.phive.peppol.PeppolValidation`.
- Contract: [../validator-rpc.md](../validator-rpc.md).

Build from the repository root:

```bash
DOCKER_BUILDKIT=1 docker build -f services/validator-phive/Dockerfile -t invoicekit/validator-phive:ci .
```
