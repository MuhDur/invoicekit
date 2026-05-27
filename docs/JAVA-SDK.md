<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 The InvoiceKit Authors
-->
# Java SDK

The Java SDK wraps the stable InvoiceKit Engine ABI. It targets Java 17 as the
baseline runtime, uses Java 22's Foreign Function and Memory API when the native
`invoicekit-ffi` library is available, and falls back to a REST sidecar when
native loading is unavailable.

## Install

```xml
<dependency>
  <groupId>dev.invoicekit</groupId>
  <artifactId>invoicekit-java</artifactId>
  <version>0.0.0</version>
</dependency>
```

Release publishing is wired for Maven Central through the Sonatype Central
Portal and runs only on release tags with Central Portal credentials.

## Native usage

Build or install `invoicekit-ffi`, then point the SDK at the shared library:

```bash
export INVOICEKIT_FFI_LIB=/path/to/libinvoicekit_ffi.so
```

Java 22 runtimes can call the C ABI directly:

```java
EngineClient client = InvoiceKit.nativeClient();
EngineResult result = client.process("""
    {"abi_version":1,"operation":"commercial_document.canonicalize","payload":{}}
    """);
```

The native client requires `--enable-native-access=ALL-UNNAMED` unless the
application runs from a module that explicitly enables native access.

## REST sidecar fallback

Java 17 and Java 21 applications should use the fallback factory when native
loading is optional:

```java
EngineClient client = InvoiceKit.nativeOrSidecar(
    URI.create("http://127.0.0.1:8080/engine/process")
);
String response = InvoiceKit.processEngineAbiJson(client, requestJson);
```

The sidecar endpoint accepts the same Engine ABI JSON bytes as the C ABI. When
the sidecar preserves the C ABI status code it sets the
`X-InvoiceKit-Status-Code` response header.
