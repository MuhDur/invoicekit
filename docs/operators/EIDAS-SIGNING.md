# eIDAS qualified signature adapter (T-083a)

`crates/signer-eidas` ships the typed Provider trait + envelope
+ Mock implementation for **eIDAS** qualified electronic
signatures — the EU-wide regulation that makes a qualified
electronic signature legally equivalent to a handwritten one
in every member state. This runbook captures what an operator
needs to swap the Mock for a live qualified trust service
provider (QTSP).

## Why a runbook and not a fully-wired live impl

Live eIDAS integration requires:

- A relationship with an **EU/EEA-listed qualified trust
  service provider** (the EU trusted list at
  `https://esignature.ec.europa.eu/efda/tl-browser/` enumerates
  every approved QTSP per member state).
- A **qualified certificate** (QC) issued by that QTSP, tied
  either to a natural person (QES) or a legal entity (eSeal).
- Operator-side custody of the QC private key — typically
  HSM-backed for production deployments, on-disk
  passphrase-encrypted for development.

The Mock in `signer-eidas::MockEidasProvider` covers everything
except those QTSP-side credentials.

## Crate shape (already shipped)

`crates/signer-eidas/src/lib.rs`:

| Item | Role |
|---|---|
| `AdesFamily` | `Cades` (binary payload signatures), `Xades` (XML payload signatures), `Pades` (PDF payload signatures). Picks the wire format. |
| `AdesLevel` | `B` (baseline) → `T` (with timestamp) → `LT` (long-term, with revocation data) → `LTA` (long-term archival, with archive timestamps). Picks the long-term validation tier. |
| `QualifiedCertificate` | `{ subject_dn, issuer_dn, certificate_pem, private_key_pem, key_passphrase }` — the QTSP-issued material. |
| `EidasSignEnvelope` | What `sign` returns: signature + signed-data hash + AdES family/level metadata. |
| `EidasSignRequest` | What the operator passes in: `{ tenant_id, family, level, payload }`. |
| `EidasError` | Typed transport / validation / refusal errors. |
| `EidasProvider` (trait) | `sign(certificate, request) -> EidasSignEnvelope`. |
| `MockEidasProvider` | Deterministic in-memory backend. |

## What the operator does

### 1. Acquire a qualified certificate

- Pick a QTSP from the EU trusted list above. Examples per
  member state: D-Trust (DE), Buypass (NO), Sectigo (multi-EU
  reseller), Notario CertiCámara (ES), AC Camerfirma (ES),
  Aruba PEC (IT — see also the dedicated SDI runbook),
  Certigna (FR), Cybertrust (BE).
- Complete identity verification (in person at a registration
  authority, or via the QTSP's video-id flow). Some QTSPs
  also issue via SPID (IT), itsme (BE), or EUDI wallets
  (forthcoming under the eIDAS 2.0 regulation).
- The QTSP ships the QC as a `.p12`/`.pfx` (software-stored),
  on a USB token (qualified signature creation device — QSCD),
  or provisioned into an HSM. PEM-convert as needed before
  loading into `QualifiedCertificate`.

For production deployments the private key SHOULD live in a
QSCD or HSM. The trait surface is identical — the live
implementation will accept a `KeyRef` that opaquely refers to
the on-HSM key handle.

### 2. Pick AdES family + level

| Need | Family | Level |
|---|---|---|
| Sign a JSON canonical doc binary payload | `Cades` | `B` for short-lived; `T` if you also timestamp; `LT`/`LTA` for long-term archive evidence. |
| Sign a UBL / Cross Industry Invoice XML | `Xades` | Pair with the country's clearance expectations; SDI requires XAdES-BES which maps to `Xades` + `B`. |
| Sign a printed PDF/A-3 | `Pades` | Always `B` minimum; `LTA` for the archive bundle. |

`AdesLevel::Lta` (long-term archival) is what the trust toolkit
narrative actually wants for every bundle: the signature carries
embedded timestamps and revocation data so an auditor can
verify it offline ten years later without needing live CA
endpoints. The live implementation will couple
`AdesLevel::Lta` to a configured timestamping authority
(see T-082 + `invoicekit timestamp`).

### 3. Wire `EidasProvider` in the engine

```rust
use invoicekit_signer_eidas::{EidasProvider, MockEidasProvider, AdesFamily, AdesLevel};

// Today, until the live PR lands:
let provider: Box<dyn EidasProvider> = Box::new(MockEidasProvider::default());

// After the live impl PR lands (likely several crates — one
// per QTSP HTTP API + one HSM-backed crate for PKCS#11 keys):
// let provider = Box::new(invoicekit_signer_eidas_pkcs11::Pkcs11Provider::new(
//     /* token_label */ "invoicekit-prod",
//     /* key_label */ "invoicekit-signing-key",
//     /* pin_secret_ref */ secret,
// ));
```

### 4. Submission flow at runtime

1. Build the canonical payload (canonical JSON, UBL XML,
   PAdES-staged PDF, etc.).
2. Construct `EidasSignRequest` with the right family + level
   for the payload type.
3. `provider.sign(&certificate, &request)` →
   `EidasSignEnvelope`. The provider:
   - re-hashes the payload,
   - signs the hash with the QSCD/HSM,
   - assembles the AdES container at the requested level
     (timestamping + revocation-data fetch happen here for
     `T`/`LT`/`LTA`),
   - returns the wire-format signature.
4. Embed the signature into the artefact:
   - `Cades` → store the detached signature alongside the
     payload bytes.
   - `Xades` → splice into the XML's `ds:Signature` element.
   - `Pades` → append the byte-range signature to the PDF.

### 5. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/payload.{ext}` | The signed payload (XML/PDF/JSON). |
| `eidas/envelope.json` | Serialised `EidasSignEnvelope` (family, level, signed hash, signature blob). |
| `eidas/cert-chain.pem` | Issuer chain up to the QTSP root, suitable for offline verification. |
| `eidas/tsa-token.bin` | RFC 3161 timestamp token (only when `level >= T`). |
| `eidas/revocation/*.crl` and `eidas/revocation/*.ocsp` | Revocation data for offline verify (only when `level >= LT`). |

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-eidas
```

The Mock covers every (family, level) combination plus
malformed-payload and bad-certificate error paths.

## Validating with the live backend

1. Set `EIDAS_QTSP=<qtsp-id>` plus the QTSP-specific
   credentials (PKCS#11 token labels for HSM, file paths for
   software-stored).
2. Run the new `tools/eidas-live-smoke/` binary (lands with
   the live impl PR) against the QTSP's test environment.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## eIDAS 1 vs eIDAS 2.0

The current trait surface targets the **eIDAS 1** regulation
(2014 baseline). **eIDAS 2.0** (2024) adds the EU Digital
Identity Wallet (EUDI Wallet) as a qualified signature path
for natural persons. When EUDI Wallet QTSPs become
production-listed, they'll plug in behind the same trait —
no engine code changes expected.

## Status today

- Mock impl: shipped on main (T-083a, closed).
- Live impl: open follow-up. Best path is probably two
  crates: `crates/signer-eidas-pkcs11/` for HSM-backed keys
  and `crates/signer-eidas-pem/` for software-stored keys
  during development.
- Country signers that build on top of eIDAS — Italy SDI
  (T-083b4), and any future EU clearance path — already
  reference this trait surface.

## References

- EU Trusted Lists browser: <https://esignature.ec.europa.eu/efda/tl-browser/>
- ETSI EN 319 122 (CAdES baseline), EN 319 132 (XAdES baseline),
  EN 319 142 (PAdES baseline) — the wire-format specs.
- The shipped Mock + tests: `crates/signer-eidas/src/lib.rs`.
