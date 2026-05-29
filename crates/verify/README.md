# invoicekit-verify

Verify an InvoiceKit evidence bundle: re-hash its contents, re-check the optional signature, re-check the optional RFC 3161 timestamp.

## What it does

An evidence bundle (`.ikb`) is the signed record InvoiceKit produces at the end of the pipeline. This crate is the library that decides whether a bundle still tells the truth. It runs up to four independent checks, each one opt-in:

1. **Content-address** — re-hashes every artefact and reconciles it against the manifest. Always runs.
2. **Signature** — re-computes the detached signature over `manifest.json` and compares it, in constant time, to the signature record you hand in. Skipped unless you supply a signer and a signature.
3. **Manifest envelope** — verifies the DSSE sidecar at `signatures/manifest.dsse` against the canonical manifest bytes. Skipped unless you supply a DSSE verifier (or set `require_manifest_dsse` to fail closed when the sidecar is missing).
4. **Timestamp** — re-hashes the manifest into an imprint and asks the timestamp client to re-bind the RFC 3161 token to it. Skipped unless you supply a timestamp client and a timestamp record.

The result is a structured `VerifyReport`. Checks you did not request are reported as `Skipped`, not failures, so a content-only verification still comes back green. A bundle that cannot even be unpacked is the one hard error; drift in any individual check is reported as a `Failed` outcome, not raised.

This is the library half of the eventual `invoicekit verify` subcommand. The CLI wrapper lands separately (tracked under T-100). The same `VerifyReport` is meant to feed the CLI, the audit UI, and the replay-from-bundle command so they all render identical semantics.

## Where it sits

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence -> verify
```

`verify` is downstream of `evidence`. It reads what `evidence` packed and what `signer`, `evidence-dsse`, and `timestamping` sealed, and it re-derives all of those from the bundle to prove nothing drifted. It builds nothing new; it only checks.

## Public API

Entry points:

- `verify(bundle: &EvidenceBundle, options: &VerifyOptions) -> VerifyReport` — run the checks against an already-unpacked bundle.
- `verify_packed(bytes: &[u8], options: &VerifyOptions) -> Result<VerifyReport, VerifyError>` — unpack `.ikb` container bytes first, then verify. The only error path is a bundle that does not parse.

Configuration and result types:

- `VerifyOptions<'a>` — each check is gated by an `Option` field: `signer` + `signature`, `manifest_dsse_signer` (+ `require_manifest_dsse`), and `timestamp_client` + `timestamp` + `timestamp_algorithm`. `VerifyOptions::content_only()` builds the variant that runs nothing but the content-address check.
- `VerifyReport` — fields `ok`, `content_address`, `signature`, `manifest_envelope`, `timestamp`. `ok` is true only when no requested check failed. Serializes to and from JSON.
- `CheckOutcome` — `Passed`, `Skipped { reason }`, or `Failed { error }`, with `is_passed()` / `is_failed()` predicates.
- `VerifyError` — wraps a `BundleError` when the container cannot be unpacked.

Helpers for callers that also produce evidence:

- `canonical_manifest_bytes(bundle) -> Result<Vec<u8>, BundleError>` — the exact manifest bytes a signer or timestamp authority should sign.
- `sign_bundle(bundle, signer, key_ref) -> Result<Signature, SignBundleError>` — sign those bytes and get back a `Signature` record, instead of repeating the boilerplate.
- `recompute_imprint(algorithm, payload) -> Vec<u8>` — the imprint a timestamp's hash algorithm expects over a payload.
- `SIGNED_ARTEFACT_ID` — re-export of the reserved manifest artefact id, so you can build `Signature` records without depending on `invoicekit-evidence` directly.

A note on the imprint: `recompute_imprint` currently derives every algorithm variant from BLAKE3, padding to match the expected length for the SHA-384 and SHA-512 cases. The verification substrate only matches imprint length, so this is a placeholder until a SHA-2 crate lands in the workspace. The signature path uses whatever the supplied `Signer` implements.

## Usage

Content-only verification of an untampered bundle:

```rust
use invoicekit_verify::{verify, VerifyOptions};

let report = verify(&bundle, &VerifyOptions::content_only());
assert!(report.ok);
```

Sign a bundle, then verify the signature alongside the content-address check:

```rust
use invoicekit_verify::{sign_bundle, verify, VerifyOptions};
use invoicekit_signer::{KeyRef, SoftwareSigner};

let signer = SoftwareSigner::new().with_key("seal", [7u8; 32]);
let signature = sign_bundle(&bundle, &signer, KeyRef::new("seal"))?;

let report = verify(
    &bundle,
    &VerifyOptions {
        signer: Some(&signer),
        signature: Some(&signature),
        ..VerifyOptions::content_only()
    },
);
assert!(report.ok);
```

## License

Apache-2.0. See the workspace root for the full text.
