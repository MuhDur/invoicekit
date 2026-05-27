# examples/asp-net

Reference ASP.NET Core 8 minimal-API demo for InvoiceKit. Canonicalises three
German XRechnung fixtures through the InvoiceKit Rust engine using the
`InvoiceKit` .NET SDK's native client (P/Invoke over the C ABI).

## 5-minute setup

```bash
# 1. Clone the repo
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit

# 2. Build the InvoiceKit cdylib (~30s)
cargo build -p invoicekit-ffi --locked

# 3. Run the demo
cd examples/asp-net
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  dotnet run
# → http://localhost:5000
```

## Endpoints

| Method | Path                                              | Purpose                                            |
|--------|---------------------------------------------------|----------------------------------------------------|
| GET    | `/`                                               | List available fixtures.                           |
| GET    | `/healthz`                                        | Liveness probe.                                    |
| POST   | `/canonicalize/{basic\|with-allowance\|reverse-charge}` | Canonicalise the named XRechnung fixture.   |

## Smoke test

```bash
cd tests
INVOICEKIT_FFI_LIB=$(realpath ../../../target/debug/libinvoicekit_ffi.so) \
  dotnet test
```

Uses `Microsoft.AspNetCore.Mvc.Testing.WebApplicationFactory<Program>` so the
gate stays fast.

## Files

| Path                                  | Purpose                                          |
|---------------------------------------|--------------------------------------------------|
| `InvoiceKitDemo.csproj`               | net8.0 Web SDK project; refs InvoiceKit SDK.     |
| `Program.cs`                          | Minimal-API entrypoint + route definitions.      |
| `InvoiceKitBridge.cs`                 | Wraps the SDK's `NativeEngineClient`.            |
| `Fixtures.cs`                         | Three German XRechnung CommercialDocs.           |
| `tests/InvoiceKitDemo.Tests.csproj`   | xunit + AspNetCore Mvc Testing.                  |
| `tests/SmokeTests.cs`                 | 6 smoke assertions via WebApplicationFactory.    |

## Architecture

The bridge uses `EngineClients.NativeClient()` so calls go straight from the
CLR to `libinvoicekit_ffi` via P/Invoke. Boot fails fast with a clear message
if the native library is unavailable — preferable to a silent fall-through to
a REST sidecar that is not running.

## License

Apache-2.0.

Implemented by `invoices-t-1404-demo-asp-net-oy35`.
