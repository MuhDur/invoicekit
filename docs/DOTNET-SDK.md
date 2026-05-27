<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 The InvoiceKit Authors
-->
# .NET SDK

The .NET SDK wraps the stable InvoiceKit Engine ABI. It targets .NET 8 LTS,
calls the `invoicekit-ffi` native library through the C ABI, and falls back to a
REST sidecar when native loading is unavailable.

## Install

```bash
dotnet add package InvoiceKit.Engine --version 0.0.0
```

Release publishing is wired for NuGet and runs only on release tags with a
`NUGET_API_KEY` secret.

## Native usage

Build or install `invoicekit-ffi`, then point the SDK at the shared library:

```bash
export INVOICEKIT_FFI_LIB=/path/to/libinvoicekit_ffi.so
```

Use the native client from a .NET 8 application:

```csharp
using InvoiceKit;

using var client = EngineClients.NativeClient();
EngineResult result = client.Process(
    "{\"abi_version\":1,\"operation\":\"commercial_document.canonicalize\",\"payload\":{}}");
```

The native client validates that the loaded library reports Engine ABI version
`1` before it processes requests.

## REST sidecar fallback

Use the fallback factory when native loading is optional:

```csharp
using InvoiceKit;

using IEngineClient client = EngineClients.NativeOrSidecar(
    new Uri("http://127.0.0.1:8080/engine/process"));
string response = EngineClients.ProcessEngineAbiJson(client, requestJson);
```

The sidecar endpoint accepts the same Engine ABI JSON bytes as the C ABI. A
sidecar can preserve the C ABI status code with the `X-InvoiceKit-Status-Code`
response header; when the header is absent, the SDK derives the status from the
canonical response body's top-level `status` field.
