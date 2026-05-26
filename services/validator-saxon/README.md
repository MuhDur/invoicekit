# services/validator-saxon

JVM sidecar service - not a Rust workspace member.

`validator-saxon` runs the shared InvoiceKit validator sidecar host with the
`jvm:saxon` backend and the Saxon-HE dependency boundary:

- Java runtime: Eclipse Temurin 21.
- Oracle dependency: `net.sf.saxon:Saxon-HE:12.9`.
- Startup class check: `net.sf.saxon.s9api.Processor`.
- Contract: [../validator-rpc.md](../validator-rpc.md).

Build from the repository root:

```bash
DOCKER_BUILDKIT=1 docker build -f services/validator-saxon/Dockerfile -t invoicekit/validator-saxon:ci .
```
