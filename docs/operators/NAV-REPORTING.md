# Hungary NAV Online Számla reporting

`crates/report-hu-nav` ships the typed Provider trait +
envelope + Mock implementation for Hungary's NAV Online
Számla v3.0 reporting mandate. Issuers submit
`manageInvoiceRequest` XML via a token-exchange flow; NAV
assigns a transaction id and processes asynchronously.

## Why a runbook and not a fully-wired live impl

- A registered NAV Online Számla account with the issuer's
  Hungarian adóazonosító / adószám.
- API credentials for `api-test.onlineszamla.nav.gov.hu`
  (test) or `api.onlineszamla.nav.gov.hu` (production).
- A pinned NAV XSD bundle for the schema release the
  operator targets.

The Mock in `report_hu_nav::MockNavProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-hu-nav/src/lib.rs`:

| Item | Role |
|---|---|
| `NavEnvironment` | `Test` vs `Production`. |
| `NavOperation` | `Create` / `Modify` / `Storno` / `Annul`. |
| `NavManageRequest` | Tenant, env, operation, issuer tax id, manageInvoiceRequest XML. |
| `NavStatus` | `Received` / `InProgress` / `Done` / `Aborted`. |
| `NavManageEnvelope` | transaction_id, status, recorded_at, optional validation_result. |
| `NavError` | `BadXml` / `BadTaxId` / `Transport`. |
| `NavProvider` (trait) | `manage_invoice(request)` + `query_transaction(env, txid)`. |
| `MockNavProvider` | 11 unit tests. |
| `validate_tax_id(s)` | 8/9/11 digits (optionally hyphenated as 8-1-2). |

## What the operator does

### 1. Acquire credentials

Register at `https://onlineszamla.nav.gov.hu`. The portal
issues XML signing certificate + API password tied to the
issuer's adóazonosító.

### 2. Wire `NavProvider` in the engine

```rust
use invoicekit_report_hu_nav::{MockNavProvider, NavProvider};

let provider: Box<dyn NavProvider> = Box::new(MockNavProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_hu_nav_http::HttpNavProvider::new(
//     "https://api.onlineszamla.nav.gov.hu", api_creds, cert,
// ));
```

### 3. Submission flow at runtime

1. Build the canonical `manageInvoiceRequest` XML wrapping
   the invoice payload.
2. `provider.manage_invoice(&request)` → envelope with
   transaction id (NAV exchanges a one-shot token under the
   hood).
3. Persist transaction_id for reconciliation. Poll via
   `provider.query_transaction(env, txid)` until `Done` /
   `Aborted`.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/nav-manage-invoice.xml` | The submitted `manageInvoiceRequest` XML. |
| `nav/envelope.json` | Serialised `NavManageEnvelope`. |
| `nav/transaction-id.txt` | NAV-assigned transaction id. |

## Validating

```bash
cargo test -p invoicekit-report-hu-nav
```

## Status today

- Substrate: shipped on main (commit `ef984d5`).
- Live HTTP impl: open follow-up in
  `crates/report-hu-nav-http`.
- Bead T-826 (Country crate: Hungary NAV) tracks the full
  country adapter on top.

## References

- NAV Online Számla portal: <https://onlineszamla.nav.gov.hu>.
- Shipped Mock + tests: `crates/report-hu-nav/src/lib.rs`.
