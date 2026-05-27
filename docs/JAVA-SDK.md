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

## Publishing to Maven Central (2ggj)

`.github/workflows/java-sdk.yml` runs the Central Portal publish on every `v*`
tag. The wiring uses the central-publishing-maven-plugin via the
`central-release` Maven profile.

### One-time operator setup

1. **Register the namespace `dev.invoicekit` on the Sonatype Central Portal.**
   Sign in to <https://central.sonatype.com>, request the namespace, and
   complete the DNS verification step Sonatype emails out (TXT record on the
   `invoicekit.dev` zone).
2. **Generate Central Portal credentials.** From the Central Portal UI, create
   a user token. Add the two halves to GitHub Actions secrets:
   - `CENTRAL_USERNAME`
   - `CENTRAL_PASSWORD`
3. **Provision a signing key.** Generate a GPG key pair dedicated to release
   artefact signing (`gpg --quick-generate-key 'InvoiceKit Release Signing <release@invoicekit.dev>' rsa4096 default 2y`).
   Export the private key (`gpg --export-secret-key -a <fingerprint>`) and
   the passphrase to GitHub secrets:
   - `MAVEN_GPG_PRIVATE_KEY` (ASCII-armoured private key)
   - `MAVEN_GPG_PASSPHRASE`
   Publish the public key to `keys.openpgp.org` so Maven Central can verify
   the signature.

### Per-release operator checklist

1. Bump the workspace version that the Maven `revision` property reads from
   `${GITHUB_REF_NAME#v}`. The workflow already passes the tag minus the
   leading `v`; no further version edits needed.
2. Cut a `v<MAJOR>.<MINOR>.<PATCH>` tag.
3. Watch the `Maven Central publish` job in the workflow run. On success it
   stages the bundle to the Central Portal and waits for the validation
   pipeline; the URL `https://central.sonatype.com/artifact/dev.invoicekit/invoicekit-java`
   will surface the new version within roughly 30 minutes.
4. Record the published version URL in the GitHub release notes.

### Why no key-based signing fallback for the GPG key

Sonatype Central explicitly requires a GPG signature on every artefact; the
sigstore-style keyless approach we use for binary releases is not accepted
upstream. The InvoiceKit GPG key is documented to rotate every two years and
the new public key must be re-published before the old one expires, otherwise
the next release will be rejected at the validation stage.
