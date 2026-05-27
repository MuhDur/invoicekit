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

## Publishing to NuGet (d21q)

`.github/workflows/dotnet-sdk.yml` runs the NuGet publish on every `v*` tag.
The wiring uses `dotnet pack` + `dotnet nuget push` against
`https://api.nuget.org/v3/index.json` with `--skip-duplicate` so re-running a
tag is safe.

### One-time operator setup

1. **Claim the `InvoiceKit.Engine` package name on NuGet.org.** Sign in as the
   package owner. The first push from the workflow takes the reservation, but
   reserving the name beforehand prevents a third party from squatting it
   between when the tag is cut and when the workflow runs.
2. **Generate a scoped API key.** From the NuGet.org account → API Keys, create
   a key with these settings:
   - Glob pattern: `InvoiceKit.*` (so the same key can publish future sibling
     packages like `InvoiceKit.AbiGolden` without a re-key).
   - Scopes: `Push new packages and package versions`.
   - Expiration: 365 days (track the rotation in `docs/RELEASE-SIGNING.md`).
3. **Add the key as a GitHub Actions secret.** Settings → Secrets → Actions →
   New repository secret → `NUGET_API_KEY`. The workflow reads it from
   `${{ secrets.NUGET_API_KEY }}`.

### Per-release operator checklist

1. The workflow passes `-p:PackageVersion="${GITHUB_REF_NAME#v}"` to
   `dotnet pack` automatically, so no `.csproj` version edit is needed; the
   tag drives the version.
2. Cut a `v<MAJOR>.<MINOR>.<PATCH>` tag.
3. Watch the `.NET SDK` workflow's publish job. On success the package appears
   at `https://www.nuget.org/packages/InvoiceKit.Engine/<VERSION>` after the
   NuGet validation pipeline (usually under 10 minutes).
4. Record the NuGet version URL in the GitHub release notes.

### Why a long-lived API key and not OIDC

NuGet does not yet support GitHub Actions trusted publishing (as of this
writing). The fallback is a long-lived key. Rotate every 365 days and prefer
a glob-scoped key (`InvoiceKit.*`) over an unscoped one so a leak only affects
the InvoiceKit package family. When NuGet trusted publishing ships, replace
the `NUGET_API_KEY` secret reference with `id-token: write` permissions and
delete the secret — there is no other operator-visible change needed.
