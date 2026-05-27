# bindings/java

Maven package for the InvoiceKit Java SDK.

The baseline API compiles on Java 17 and exposes the stable Engine ABI byte
contract. Java 22 runtimes use a multi-release FFM provider to call
`invoicekit-ffi` directly; Java 17 and Java 21 applications can fall back to a
REST sidecar endpoint.

```bash
mvn -B verify
```

Native golden tests run when `INVOICEKIT_FFI_LIB` points at a built
`invoicekit-ffi` shared library and the test JVM is Java 22 or newer.
