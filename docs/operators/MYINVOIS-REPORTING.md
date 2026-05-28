# Malaysia MyInvois (LHDNM) reporting

`crates/report-my-myinvois` ships the typed Provider trait +
envelope + Mock implementation for Malaysia's MyInvois
clearance portal operated by LHDNM (Lembaga Hasil Dalam
Negeri Malaysia / Inland Revenue Board). Every Malaysian B2B
issuer transmits invoices in near real time; LHDNM assigns
a UUID + 64-char content hash + Long ID for buyer-side
public validation.

## Why a runbook and not a fully-wired live impl

- A registered LHDNM MyInvois account with the issuer's TIN
  (`C` + 10 digits) + BRN (12 digits).
- API credentials for `preprod-api.myinvois.hasil.gov.my`
  (sandbox) or `api.myinvois.hasil.gov.my` (production).
- A pinned LHDNM UBL schema bundle for the release the
  operator targets.

The Mock in `report_my_myinvois::MockMyInvoisProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-my-myinvois/src/lib.rs`:

| Item | Role |
|---|---|
| `MyInvoisEnvironment` | `Sandbox` vs `Production`. |
| `MyInvoisDocumentKind` | 8 LHDNM eInvoiceTypeCode variants (Invoice 01, CreditNote 02, DebitNote 03, RefundNote 04, plus SelfBilled* 11-14). |
| `MyInvoisSubmitRequest` | Tenant, env, kind, issuer TIN, issuer BRN, optional buyer TIN, canonical UBL XML. |
| `MyInvoisStatus` | `Submitted` / `Valid` / `Cancelled` / `Rejected`. |
| `MyInvoisSubmitEnvelope` | UUID, content_hash_hex, long_id, status, submitted_at, optional rejection_reason. |
| `MyInvoisError` | `BadXml` / `BadTin` / `BadBrn` / `Transport`. |
| `MyInvoisProvider` (trait) | `submit_invoice(request)` + `cancel_invoice(env, uuid, reason)`. |
| `MockMyInvoisProvider` | 13 unit tests. |
| `validate_tin(s)` / `validate_brn(s)` | Shape checks. |

## What the operator does

### 1. Acquire credentials

Register at `https://myinvois.hasil.gov.my`. The portal issues
the API credentials tied to the issuer's TIN + BRN.

### 2. Wire `MyInvoisProvider` in the engine

```rust
use invoicekit_report_my_myinvois::{MockMyInvoisProvider, MyInvoisProvider};

let provider: Box<dyn MyInvoisProvider> = Box::new(MockMyInvoisProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_my_myinvois_http::HttpMyInvoisProvider::new(
//     "https://api.myinvois.hasil.gov.my", api_creds,
// ));
```

### 3. Submission flow at runtime

1. Build canonical UBL XML (LHDNM's PEPPOL-derived schema).
2. `provider.submit_invoice(&request)` → envelope with UUID +
   Long ID.
3. Persist UUID for reconciliation. Embed Long ID into the
   printed-invoice QR.
4. Within the 72-hour grace window, the buyer can request
   cancellation; the engine calls
   `provider.cancel_invoice(env, uuid, reason)`.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/myinvois.xml` | The canonical UBL submitted. |
| `myinvois/envelope.json` | Serialised `MyInvoisSubmitEnvelope`. |
| `myinvois/long-id.txt` | The buyer-validation Long ID. |
| `myinvois/uuid.txt` | The LHDNM-assigned UUID. |

## Validating

```bash
cargo test -p invoicekit-report-my-myinvois
```

## Status today

- Substrate: shipped on main (commit `4679843`).
- Live HTTP impl: open follow-up in
  `crates/report-my-myinvois-http`.
- Bead T-823 (Country crate: Malaysia MyInvois) tracks the
  full country adapter on top of this substrate.

## References

- LHDNM MyInvois portal: <https://myinvois.hasil.gov.my>.
- Shipped Mock + tests: `crates/report-my-myinvois/src/lib.rs`.
