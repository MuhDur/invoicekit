# services/validator-verapdf

JVM sidecar service — not a Rust workspace member.

`validator-verapdf` runs the shared InvoiceKit validator sidecar host with the
`jvm:verapdf` backend and the [veraPDF](https://verapdf.org/) reference
PDF/A conformance verifier as the validation oracle:

- Java runtime: Eclipse Temurin 21.
- Oracle dependency: `org.verapdf:verapdf-library:1.27.x`.
- Startup class check: `org.verapdf.pdfa.Foundries`.
- Contract: [`../validator-rpc.md`](../validator-rpc.md).
- Rust adapter: [`crates/render-verify/src/verapdf.rs`](../../crates/render-verify/src/verapdf.rs).

The sidecar accepts a single JSON-RPC method, `validator.validate_pdf`,
that takes a base64-encoded PDF body plus an optional conformance flavour
hint (`pdfa-3a`, `pdfa-3b`, `pdfa-3u`) and returns a typed `PdfAReport`:

```json
{
  "jsonrpc": "2.0",
  "id": "trace-001",
  "method": "validator.validate_pdf",
  "params": {
    "document": {"pdf_base64": "JVBERi0xLjQK..."},
    "flavour": "pdfa-3b",
    "trace_id": "trace-001"
  }
}
```

Build from the repository root:

```bash
DOCKER_BUILDKIT=1 docker build -f services/validator-verapdf/Dockerfile -t invoicekit/validator-verapdf:ci .
```

## Why a sidecar?

Per the InvoiceKit architectural commitments (AGENTS.md item 6) we run reference
validators "as an isolated JVM worker service called over JSON-RPC." veraPDF is
the canonical PDF/A reference verifier and is JVM-only; embedding it in
WebAssembly is explicitly out of scope. The sidecar contract keeps the
PDF/A check fungible across deployment targets and lets the Rust core stay
deterministic.
