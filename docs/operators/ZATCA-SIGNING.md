# Saudi Arabia ZATCA Phase 2 cryptographic stamp (T-083b1)

`crates/signer-zatca` ships the typed Provider trait + envelope
+ Mock implementation for **Zakat, Tax and Customs Authority**
(ZATCA) Phase 2 e-invoicing in Saudi Arabia. This runbook
captures everything an operator needs to swap the Mock for a
live ZATCA backend.

## Why a runbook and not a fully-wired live impl

Live ZATCA integration requires three operator-side assets:

- A ZATCA Fatoora portal account on
  `https://fatoora.zatca.gov.sa` (sandbox: `sandbox.zatca.gov.sa`).
- A **CSID** (Cryptographic Stamp Identifier) issued by ZATCA
  after submitting a Certificate Signing Request. Production
  CSIDs are tied to a Saudi commercial registration number.
- A pinned ZATCA UBL extension schema bundle for the format
  release the operator targets.

The Mock in `signer-zatca::MockPhase2Provider` covers everything
*except* those operator-side credentials.

## Crate shape (already shipped)

`crates/signer-zatca/src/lib.rs`:

| Item | Role |
|---|---|
| `ZatcaInvoiceMode` | `Standard` (B2B tax invoice) vs `Simplified` (B2C). Determines clearance vs reporting path. |
| `ZatcaEnvironment` | `Compliance` (CCS sandbox) vs `Production`. |
| `CsidRecord` | `{ csid, private_key, certificate_pem, environment }` — what the live backend persists per tenant. |
| `QrField` | Tag 1–8 TLV encoding for the QR field embedded in the printed invoice. |
| `ReportingStatus` | `Accepted` / `AcceptedWithWarnings` / `Rejected` / `Pending` — the typed verdict the engine surfaces. |
| `ZatcaStampEnvelope` | What `stamp` returns: invoice hash + signature + QR + status. |
| `ZatcaSignRequest` | What the operator passes in. |
| `ZatcaError` | Typed transport / validation / refusal errors. |
| `Phase2Provider` (trait) | `stamp(csid, request) -> ZatcaStampEnvelope`. |
| `MockPhase2Provider` | Deterministic in-memory backend (13 unit tests). |
| `validate_qr_fields` | Standalone helper; rejects missing required tags. |
| `encode_qr_tlv` | Standalone helper; encodes Tag/Length/Value bytes for the QR. |
| `invoice_sha256_hex` | Standalone helper; lowercase hex SHA-256 of the canonical XML. |

## What the operator does

### 1. Acquire a CSID

- **Compliance (sandbox)**: register at
  `https://sandbox.zatca.gov.sa`, submit a CSR, receive a
  sandbox CSID.
- **Production**: register at the Fatoora portal. Production
  CSIDs require an active commercial registration and ZATCA
  approval.

The CSID material is a secret of the same class as a TLS
private key. Store via the OS secret store or a sealed-secret
KMS; never check it into the repo.

### 2. Wire `Phase2Provider` in the engine

```rust
use invoicekit_signer_zatca::{CsidRecord, Phase2Provider, MockPhase2Provider, ZatcaEnvironment};

// Until the live HTTP impl PR lands:
let provider: Box<dyn Phase2Provider> = Box::new(MockPhase2Provider::default());

// After the live HTTP impl PR lands:
// let provider = Box::new(invoicekit_signer_zatca_http::HttpPhase2Provider::new(
//     ZatcaEnvironment::Production,
//     /* base_url */ "https://gw-apic-gov.gazt.gov.sa",
//     /* tls_ca_bundle */ trusted_roots,
// ));
```

### 3. Submission flow at runtime

1. Load the per-tenant `CsidRecord` from your secret store.
2. Build the canonical UBL XML (use `crates/format-ubl`
   downstream of the canonical model). Compute its SHA-256
   via `invoice_sha256_hex` — ZATCA needs SHA-256, not BLAKE3.
3. Construct `QrField` tags 1–8 (seller name, VAT registration
   number, invoice timestamp, invoice total with VAT, VAT
   total, XML hash, public key, ECDSA signature). Pass through
   `validate_qr_fields` before calling `stamp`.
4. `provider.stamp(&csid, &request)` → `ZatcaStampEnvelope`.
5. Embed the QR via `encode_qr_tlv` into the printed PDF and
   the cleared UBL.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/zatca-ubl.xml` | Cleared UBL XML, byte-identical to what ZATCA accepted. |
| `zatca/qr.tlv` | Encoded QR TLV (tags 1–8). |
| `zatca/envelope.json` | Serialised `ZatcaStampEnvelope`. |
| `zatca/csid-public.pem` | Public half of the CSID (the private key never enters the bundle). |

### 5. Standard vs Simplified handling

`ZatcaInvoiceMode::Simplified` (B2C) is the **reporting** path:
the invoice is reported to ZATCA within 24 hours after the
sale; the seller can deliver to the buyer before clearance.

`ZatcaInvoiceMode::Standard` (B2B) is the **clearance** path:
the invoice MUST be cleared by ZATCA before delivery to the
buyer. The engine code path must hold the printed/sent
invoice until `ReportingStatus::Accepted` (or
`AcceptedWithWarnings`).

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-zatca
```

13 unit tests cover both modes, both environments, QR tag
validation, hash mismatch handling, and the four reporting
statuses.

## Validating with the live backend

1. Set `ZATCA_BASE_URL`, `ZATCA_CSID_PATH`, `ZATCA_VAT_NUMBER`
   in the environment.
2. Run the new `tools/zatca-live-smoke/` binary (lands with
   the live impl PR) against the sandbox.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## Status today

- Mock impl: shipped on main (T-083b1, closed).
- Live HTTP impl: open follow-up. Belongs in a new
  `crates/signer-zatca-http/` crate.
- Bead T-801 (ZATCA cryptographic archetype) tracks the
  archetype lock-in on top of this signer.

## References

- ZATCA developer portal: <https://zatca.gov.sa/en/E-Invoicing/SystemComponents>
- The shipped Mock + tests: `crates/signer-zatca/src/lib.rs`.
