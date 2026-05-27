# examples/go-chi

Reference Go (chi) demo for InvoiceKit. Canonicalises three German XRechnung
fixtures through the InvoiceKit Rust engine via the `github.com/MuhDur/invoicekit/bindings/go`
SDK from T-107 (cgo path, with REST fallback if compiled `CGO_ENABLED=0`).

## 5-minute setup

```bash
# 1. Clone the repo
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit

# 2. Build the InvoiceKit cdylib (~30s)
cargo build -p invoicekit-ffi --locked

# 3. Run the demo
cd examples/go-chi
CGO_ENABLED=1 \
CGO_LDFLAGS="-L$(realpath ../../target/debug) -linvoicekit_ffi" \
LD_LIBRARY_PATH="$(realpath ../../target/debug)" \
INVOICEKIT_FFI_LIB="$(realpath ../../target/debug/libinvoicekit_ffi.so)" \
  go run .
# → http://localhost:8080
```

## Endpoints

| Method | Path                                              | Purpose                                            |
|--------|---------------------------------------------------|----------------------------------------------------|
| GET    | `/`                                               | List available fixtures + transport mode.          |
| GET    | `/healthz`                                        | Liveness probe.                                    |
| POST   | `/canonicalize/{basic\|with-allowance\|reverse-charge}` | Canonicalise the named XRechnung fixture.   |

## Smoke test

```bash
CGO_ENABLED=1 \
CGO_LDFLAGS="-L$(realpath ../../target/debug) -linvoicekit_ffi" \
LD_LIBRARY_PATH="$(realpath ../../target/debug)" \
INVOICEKIT_FFI_LIB="$(realpath ../../target/debug/libinvoicekit_ffi.so)" \
  go test -v ./...
```

Uses `net/http/httptest` so the gate stays fast.

## Files

| Path             | Purpose                                                       |
|------------------|---------------------------------------------------------------|
| `go.mod`         | Pins chi + a `replace` directive for the workspace Go SDK.    |
| `main.go`        | chi router + handlers + entrypoint.                           |
| `fixtures.go`    | Three German XRechnung CommercialDocument maps.               |
| `main_test.go`   | 6 smoke assertions via `httptest`.                            |

## Architecture

The demo uses `github.com/MuhDur/invoicekit/bindings/go.Process` directly.
Build with `CGO_ENABLED=1` (default) and the cgo path calls
`libinvoicekit_ffi` in-process; build with `CGO_ENABLED=0` and the SDK
auto-switches to its REST fallback (set `INVOICEKIT_REST_URL`).

## License

Apache-2.0.

Implemented by `invoices-t-1407-demo-go-chi-1h1d`.
