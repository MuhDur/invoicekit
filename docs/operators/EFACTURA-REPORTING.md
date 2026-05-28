# Romania RO e-Factura (ANAF) reporting

`crates/report-ro-efactura` ships the typed Provider trait +
envelope + Mock implementation for Romania's RO e-Factura
clearance portal operated by ANAF (Agenția Națională de
Administrare Fiscală). Every Romanian B2B issuer transmits
UBL 2.1 + RO CIUS invoices; ANAF assigns an indice de
încărcare (upload index) and within minutes follows up with a
signed mesaj XML carrying the cleared invoice + ANAF's
countersignature.

## Why a runbook and not a fully-wired live impl

- An ANAF SPV (Spațiul Privat Virtual) account with the
  issuer's CUI (2–10 ASCII digits, optionally `RO`-prefixed).
- API credentials for `api.anaf.ro/test` (sandbox) or
  `api.anaf.ro/prod` (production).
- A pinned RO CIUS XSD bundle for the release the operator
  targets.

The Mock in `report_ro_efactura::MockEFacturaProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-ro-efactura/src/lib.rs`:

| Item | Role |
|---|---|
| `EFacturaEnvironment` | `Sandbox` vs `Production`. |
| `EFacturaDocumentKind` | `Invoice` / `CreditNote` / `SelfBilling` (autofactura). |
| `EFacturaUploadRequest` | Tenant, env, kind, issuer CUI, optional buyer CUI, canonical UBL XML. |
| `EFacturaStatus` | `Uploaded` / `InProgress` / `Cleared` / `Rejected`. |
| `EFacturaUploadEnvelope` | indice_incarcare, status, uploaded_at, optional motivare. |
| `EFacturaError` | `BadXml` / `BadCui` / `Transport`. |
| `EFacturaProvider` (trait) | `upload(request)` + `poll_status(env, indice)`. |
| `MockEFacturaProvider` | 11 unit tests. |
| `validate_cui(s)` | 2–10 ASCII digits (optionally `RO`-prefixed). |

## What the operator does

### 1. Acquire credentials

Register in ANAF's SPV at `https://anaf.ro`. The portal issues
the API credentials tied to the issuer's CUI.

### 2. Wire `EFacturaProvider` in the engine

```rust
use invoicekit_report_ro_efactura::{EFacturaProvider, MockEFacturaProvider};

let provider: Box<dyn EFacturaProvider> = Box::new(MockEFacturaProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_ro_efactura_http::HttpEFacturaProvider::new(
//     "https://api.anaf.ro/prod", api_creds,
// ));
```

### 3. Submission flow at runtime

1. Build canonical UBL 2.1 + RO CIUS XML.
2. `provider.upload(&request)` → envelope with indice de
   încărcare.
3. Persist indice for reconciliation. Poll via
   `provider.poll_status(env, indice)` until `Cleared` /
   `Rejected`.
4. On `Cleared`, fetch the signed mesaj XML for the evidence
   bundle.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/efactura.xml` | The canonical UBL submitted. |
| `efactura/envelope.json` | Serialised `EFacturaUploadEnvelope`. |
| `efactura/mesaj.xml` | The ANAF-signed mesaj after `Cleared`. |
| `efactura/indice-incarcare.txt` | The numeric upload index. |

## Validating

```bash
cargo test -p invoicekit-report-ro-efactura
```

## Status today

- Substrate: shipped on main (commit `e0530c5`).
- Live HTTP impl: open follow-up in
  `crates/report-ro-efactura-http`.
- Bead T-825 (Country crate: Romania RO e-Factura) tracks
  the full country adapter on top.

## References

- ANAF e-Factura portal: <https://anaf.ro>.
- Shipped Mock + tests: `crates/report-ro-efactura/src/lib.rs`.
