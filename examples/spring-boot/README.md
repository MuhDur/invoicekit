# examples/spring-boot

Reference Spring Boot 3.4 demo for InvoiceKit. Canonicalises three German
XRechnung fixtures through the InvoiceKit Rust engine via the
`invoicekit-java` SDK's FFM-based native client (Java 22+).

## 5-minute setup

```bash
# 1. Clone the repo
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit

# 2. Build the InvoiceKit cdylib (~30s)
cargo build -p invoicekit-ffi --locked

# 3. Install the Java SDK to your local Maven cache
mvn -B -ntp install -DskipTests -f bindings/java/pom.xml

# 4. Run the demo
cd examples/spring-boot
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  mvn spring-boot:run
# → http://localhost:8080
```

## Endpoints

| Method | Path                                              | Purpose                                            |
|--------|---------------------------------------------------|----------------------------------------------------|
| GET    | `/`                                               | List available fixtures.                           |
| GET    | `/healthz`                                        | Liveness probe.                                    |
| POST   | `/canonicalize/{basic\|with-allowance\|reverse-charge}` | Canonicalise the named XRechnung fixture.   |

## Smoke test

```bash
INVOICEKIT_FFI_LIB=$(realpath ../../target/debug/libinvoicekit_ffi.so) \
  mvn -B -ntp test
```

Uses Spring's `MockMvc` so the gate stays fast.

## Files

| Path                                                                        | Purpose                                  |
|-----------------------------------------------------------------------------|------------------------------------------|
| `pom.xml`                                                                   | Spring Boot 3.4 + invoicekit-java SDK.   |
| `src/main/java/.../DemoApplication.java`                                    | Boot entrypoint.                         |
| `src/main/java/.../DemoController.java`                                     | REST controller with 3 endpoints.        |
| `src/main/java/.../InvoiceKitBridge.java`                                   | Wraps the SDK's `EngineClient`.          |
| `src/main/java/.../Fixtures.java`                                           | Three German XRechnung CommercialDocs.   |
| `src/test/java/.../SmokeIT.java`                                            | 6 smoke assertions via MockMvc.          |

## Architecture

The bridge uses `InvoiceKit.nativeClient()` (the FFM-backed
`FfmEngineClient` on Java 22+) so calls go straight from the JVM to
`libinvoicekit_ffi` without an HTTP hop. The bridge fails fast at boot if
the native library is unavailable — falling back to a REST sidecar would
hide a missing `INVOICEKIT_FFI_LIB` until the first request.

## License

Apache-2.0.

Implemented by `invoices-t-1403-demo-spring-boot-4kb1`.
