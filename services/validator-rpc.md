# Validator JSON-RPC Contract

InvoiceKit reference validators run as domain-specific JVM sidecars. Each
sidecar listens on HTTP `POST /rpc`, accepts JSON-RPC 2.0, and returns validation
findings in the T-032 `ValidationResult` shape.

## Sidecars

| Service | Backend ID | Oracle dependency | Startup class check |
|---|---|---|---|
| `validator-kosit` | `jvm:kosit` | `org.kosit:validator:1.6.2` | `de.kosit.validationtool.api.Check` |
| `validator-phive` | `jvm:phive` | `com.helger.phive.rules:phive-rules-peppol:3.2.2` | `com.helger.phive.peppol.PeppolValidation` |
| `validator-saxon` | `jvm:saxon` | `net.sf.saxon:Saxon-HE:12.9` | `net.sf.saxon.s9api.Processor` |

The container fails at startup if its configured oracle class is not present on
the runtime classpath.

This T-030 contract implementation proves the container boundary, dependency
isolation, request/response schema, XML well-formedness handling, and latency
gate. Full domain rule invocation through KoSIT, phive, and Saxon is owned by the
downstream validator-parity beads and must not be inferred from this contract
smoke harness alone.

### Profile-driven dispatch (7psv)

The sidecar picks one of two paths based on `params.profile`:

- `params.profile = "contract-smoke"` (or absent) — XML
  well-formedness only. This preserves the T-030 contract surface
  and the `services/validator-smoke.py` p95 latency gate.
- Any other profile — the sidecar dispatches to its backend's
  real rule engine: `jvm:phive` calls
  `com.helger.phive.peppol.PeppolValidation.initStandard(...)`
  and runs the latest Peppol BIS Billing 3.0 EN 16931 Schematron
  pipeline; `jvm:kosit` calls
  `de.kosit.validationtool.api.DefaultCheck.checkInput(...)`
  against the scenarios bundle at the path in the
  `INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS` env var; `jvm:saxon`
  currently still falls back to well-formedness pending T-031
  generalisation.

Findings from either backend land in `result.results[]` shaped as
`{rule_id, severity, term, location, message, citation,
suggested_fix, trace}`. `rule_id` carries the oracle's native
identifier (`BR-CO-15`, `BR-04`, etc) so the rust side can map
each finding back to its EN 16931 / Peppol business term without
re-interpreting the Schematron output.

## Request

```json
{
  "jsonrpc": "2.0",
  "id": "req-001",
  "method": "validator.validate",
  "params": {
    "backend": "jvm:kosit",
    "profile": "xrechnung",
    "trace_id": "trace_123",
    "rule_pack": {
      "id": "xrechnung-kosit-2024",
      "version": "2024.1",
      "effective_date": "2026-05-26"
    },
    "document": {
      "content_type": "application/xml",
      "encoding": "utf-8",
      "xml": "<Invoice><ID>INV-001</ID></Invoice>"
    }
  }
}
```

Required fields:

- `jsonrpc`: must be `"2.0"`.
- `method`: must be `"validator.validate"`.
- `params.document.xml`: non-empty XML string.

Recommended fields:

- `params.trace_id`: propagated into each `ValidationResult.trace.trace_id`.
- `params.profile`: profile key such as `xrechnung`, `peppol-bis`, or `cii`.
- `params.rule_pack`: pinned rule-pack metadata selected by effective date.

## Response

```json
{
  "jsonrpc": "2.0",
  "id": "req-001",
  "result": {
    "backend": "jvm:kosit",
    "service": "validator-kosit",
    "oracle_coordinate": "org.kosit:validator:1.6.2",
    "oracle_class": "de.kosit.validationtool.api.Check",
    "profile": "xrechnung",
    "rule_pack_id": "xrechnung-kosit-2024",
    "valid": true,
    "duration_ms": 2,
    "document": {
      "content_type": "application/xml",
      "byte_length": 35,
      "sha256": "lowercase-hex",
      "root": "Invoice"
    },
    "results": []
  }
}
```

Malformed XML returns `valid: false` and at least one T-032 shaped result:

```json
{
  "rule_id": "KOSIT-XML-WELLFORMED",
  "severity": "fatal",
  "term": { "kind": "business_group", "code": "BG-1" },
  "location": { "kind": "x_path", "expression": "/" },
  "suggested_fix": {
    "summary": "Provide well-formed XML before invoking jvm:kosit."
  },
  "citation": {
    "source": "KoSIT validator 1.6.2",
    "section": "XML well-formedness"
  },
  "trace": {
    "backend": "jvm:kosit",
    "trace_id": "trace_123",
    "details": {
      "oracle_coordinate": "org.kosit:validator:1.6.2",
      "oracle_class": "de.kosit.validationtool.api.Check",
      "exception": "org.xml.sax.SAXParseException",
      "message": "parser diagnostic"
    }
  }
}
```

## Errors

JSON-RPC errors are returned with HTTP 200 when the request is syntactically
valid JSON-RPC but semantically invalid:

- `-32601`: unsupported method or JSON-RPC version.
- `-32602`: invalid params, including missing `params.document.xml`.

HTTP status errors are reserved for transport failures:

- `400`: request body is not JSON.
- `405`: method other than `POST` on `/rpc`.
- `413`: request body exceeds the sidecar limit.

## Latency Gate

The smoke harness sends a 1 MiB XML document to each sidecar after warmup and
requires p95 latency below 200 ms. The measurement is transport-inclusive over
localhost and excludes container image build and JVM startup.
