# DSSE manifest envelope (bead 8h6g)

`crates/evidence-dsse` ships the
**Dead Simple Signing Envelope** (DSSE, dsse.dev/spec/v1.0)
wrapper InvoiceKit evidence bundles use to bind a detached
signature to the bundle's `manifest.json`. Same envelope
format the broader supply-chain ecosystem (in-toto, sigstore,
SLSA) settled on — so InvoiceKit evidence drops into those
tools without translation.

This runbook explains the wire format, the canonical artefact
location inside the bundle, and the swap-in point for real
signer wiring.

## Why DSSE specifically

Three constraints pointed at one choice:

- **Don't invent a new envelope.** The supply-chain ecosystem
  agreed on DSSE in 2021. Anything new would just be an
  InvoiceKit dialect.
- **Bind the payload type into the signature.** DSSE's PAE
  (Pre-Authentication Encoding) prevents attackers from
  swapping `payloadType` to fool a verifier — the signature
  is computed over `DSSEv1 LEN(type) SP type SP LEN(payload)
  SP payload`, not over the raw payload.
- **Stay codec-agnostic.** DSSE doesn't pick a signature
  algorithm. The engine wires in HMAC-SHA256 for development
  and an HSM-backed ECDSA (or eIDAS qualified RSA) for
  production — both look the same from the envelope's
  perspective.

JWS (RFC 7515) was the alternative. We picked DSSE because
the supply-chain tooling around it is more mature for the
attestation use case (cosign-verify, in-toto-verify,
slsa-verifier all consume DSSE directly).

## Wire format

```json
{
  "payload": "<base64(manifest.json bytes)>",
  "payloadType": "application/vnd.invoicekit.manifest+json",
  "signatures": [
    {
      "keyid": "<opaque key id chosen by the signer>",
      "sig": "<base64(signature over PAE)>"
    }
  ]
}
```

The signature is computed over the PAE of `(payloadType,
payload)` — see `evidence_dsse::pae` for the exact byte
sequence. Verification must re-compute the PAE from the
freshly-recomputed manifest, not from the envelope's own
payload field (the verifier compares the two byte-for-byte
first to catch tampering before the cryptographic check
runs).

## Canonical artefact location

The DSSE envelope lives at the reserved artefact id
**`signatures/manifest.dsse`** inside the `.ikb` bundle.
Use the const `evidence_dsse::MANIFEST_SIGNATURE_ARTEFACT_ID`
rather than hand-writing the path; the crate's verify helper
keys on that same constant.

`invoicekit verify` (once the wiring lands — separate
follow-up) will:

1. Read `signatures/manifest.dsse` if present (absence is
   fine when no signer was wired; only the content-address
   check runs).
2. Re-serialise the bundle's manifest into canonical JSON.
3. Decode the envelope's payload + payload type.
4. Reject on `PayloadTypeDrift` / `PayloadDrift` /
   `NoSignatures` / `UnknownKey` *before* asking the signer
   to verify the signature itself — the typed error surface
   in `DsseError` makes "missing signature" distinct from
   "tampered signature" in CI output.

## What the operator does

### 1. Pick a signer

For dev, use `evidence_dsse::MockSigner` — deterministic,
zero-config. The "signature" is a deterministic digest of the
PAE prefixed with `b"mock-dsse:"`; verification re-computes
the same value. **Never** use the mock for production
bundles.

For production, supply any `ManifestSigner` implementation.
Recommended paths:

| Path | When |
|---|---|
| HSM-backed ECDSA via PKCS#11 | The default for tenants who already operate an HSM. The signer-agent (T-083) will ship a PKCS#11 backend in `crates/signer-agent-pkcs11`. |
| eIDAS qualified signature | When the bundle is also a legal artefact (Italy SDI, France CTC, Spain VeriFactu). Wire through `crates/signer-eidas` — see [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md). |
| Sigstore keyless | When OIDC-based ephemeral signing is appropriate (CI bundles, conformance corpus releases). |

### 2. Sign at bundle assembly

```rust
use invoicekit_evidence_dsse::{
    wrap, MANIFEST_PAYLOAD_TYPE, MANIFEST_SIGNATURE_ARTEFACT_ID,
};

// `manifest_bytes` is what `invoicekit_evidence::pack` will
// emit for the manifest artefact. Compute it once, sign it,
// then hand both back to pack() for assembly.
let envelope = wrap(&signer, MANIFEST_PAYLOAD_TYPE, &manifest_bytes)?;
let envelope_bytes = serde_json::to_vec(&envelope)?;
bundle.artefacts.insert(
    MANIFEST_SIGNATURE_ARTEFACT_ID.to_owned(),
    envelope_bytes,
);
```

### 3. Verify at audit time

```rust
use invoicekit_evidence_dsse::{
    verify_envelope, DsseEnvelope, MANIFEST_PAYLOAD_TYPE,
    MANIFEST_SIGNATURE_ARTEFACT_ID,
};

let envelope_bytes = unpacked.artefacts
    .get(MANIFEST_SIGNATURE_ARTEFACT_ID)
    .ok_or(VerifyError::MissingSignature)?;
let envelope: DsseEnvelope = serde_json::from_slice(envelope_bytes)?;

verify_envelope(
    &envelope,
    MANIFEST_PAYLOAD_TYPE,
    &freshly_serialised_manifest_bytes,
    &signer,
)?;
```

The error surface is typed (see `DsseError`); pattern-match on
it so audit UIs can render distinct "no signature found" vs
"signature mismatch" vs "wrong key" verdicts.

### 4. Multi-signature bundles

The envelope's `signatures` field is a `Vec`, not a single
value. Threshold verifiers (N-of-M) can require multiple
signers — for example, "two engineers sign every production
release bundle" or "compliance + treasury co-sign every
manual override". `wrap` only adds one signature today;
multi-signer assembly is a thin loop over `wrap` results
that the principal can land when threshold policies become
relevant.

## Validating today

```bash
cargo test -p invoicekit-evidence-dsse
```

12 unit tests cover: PAE spec example, empty payload, binary
payload, round-trip with `MockSigner`, payload mutation
rejection, payload-type mutation rejection, no-signatures
rejection, unknown-keyid rejection, tampered-signature
rejection, bad base64 rejection, JSON round-trip, canonical
artefact id constant.

## Status today

- Substrate: shipped on main as `crates/evidence-dsse`
  (commit `f2b6e4c`).
- Wiring into `EvidenceBundle.pack` write path: open
  follow-up; touches `crates/evidence` which is the tar.zst
  rewrite lane (see bead 8h6g).
- Wiring into `invoicekit-verify::run_signature_check`: open
  follow-up; touches `crates/verify`.
- Real HSM signer: lands with the signer-agent T-083
  follow-up.

## References

- DSSE v1 spec: <https://dsse.dev/spec/v1.0>
- The shipped substrate + tests:
  `crates/evidence-dsse/src/lib.rs`.
- Related runbook for the qualified-signature path:
  [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md).
