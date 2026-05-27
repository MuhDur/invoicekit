# @invoicekit/go (`github.com/MuhDur/invoicekit/bindings/go`)

InvoiceKit Go SDK. Two transports, same surface:

- **cgo** (default): links against `libinvoicekit_ffi` for in-process speed.
  Requires `CGO_ENABLED=1` and `crates/invoicekit-ffi` built on the host's
  library search path.
- **REST fallback** (`CGO_ENABLED=0`): pure-Go HTTP client that POSTs Engine
  ABI JSON to a rest-shim sidecar configured via `INVOICEKIT_REST_URL`.

The build tag is selected automatically; consumers don't write build tags
themselves.

## Quick start

```go
package main

import (
    "fmt"

    invoicekit "github.com/MuhDur/invoicekit/bindings/go"
)

func main() {
    fmt.Println("transport:", invoicekit.TransportMode())
    resp, status, err := invoicekit.Process([]byte(`
        {"abi_version":1,"operation":"engine.info","payload":{}}
    `))
    if err != nil {
        panic(err)
    }
    fmt.Println("status:", status)
    fmt.Println("response:", string(resp))
}
```

## REST fallback configuration

| Variable                 | Default                  | Purpose                            |
|--------------------------|--------------------------|------------------------------------|
| `INVOICEKIT_REST_URL`    | `http://127.0.0.1:8081`  | Base URL of the rest-shim sidecar. |
| `INVOICEKIT_REST_BEARER` | _(unset)_                | Optional Bearer token for auth.    |

The sidecar must implement `POST /v1/engine/process_json` with the body being
the raw Engine ABI JSON, returning:

```json
{
  "status": 0,
  "response_base64": "<base64 encoded engine response bytes>"
}
```

## Publishing

Go modules are "published" by tagging the repository with `bindings/go/vX.Y.Z`.
pkg.go.dev fetches the module via the standard proxy on first download. The
`go-sdk.yml` CI workflow gates every tag push: it validates the tag shape and
runs the full matrix.

## License

Apache-2.0.

Scaffolded by `invoices-t-001-cargo-workspace-xos`; implemented by
`invoices-t-107-go-sdk-30e9`.
