# Turkey GİB e-Fatura / e-Arşiv reporting

`crates/report-tr-efatura` ships the typed Provider trait +
envelope + Mock implementation for Turkey's e-Fatura
(registered B2B) + e-Arşiv (non-registered receivers)
mandates operated by GİB (Gelir İdaresi Başkanlığı, the
Revenue Administration). Both flow through GİB and assign a
16-char alphanumeric ETTN (Evrensel Tekil Tanımlama
Numarası) the issuer prints on the invoice.

## Why a runbook and not a fully-wired live impl

- A registered GİB e-Fatura account (issuer must be on the
  GİB mukellef registry for the e-Fatura mandate; e-Arşiv is
  available to any issuer above the turnover threshold).
- API credentials for `efaturatest.izibiz.com.tr` (sandbox)
  or `efatura.gib.gov.tr` (production).
- A pinned UBL-TR schema bundle for the release the operator
  targets.

The Mock in `report_tr_efatura::MockEFaturaProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-tr-efatura/src/lib.rs`:

| Item | Role |
|---|---|
| `EFaturaEnvironment` | `Sandbox` vs `Production`. |
| `EFaturaMandate` | `EFatura` (B2B between registered parties) vs `EArsiv` (non-registered receivers). |
| `EFaturaSubmitRequest` | Tenant, env, mandate, issuer VKN (10 digits), optional buyer tax id (VKN 10 / TCKN 11), canonical UBL-TR XML. |
| `EFaturaStatus` | `Submitted` / `Cleared` / `Rejected` (Red Yanıtı) / `Cancelled` (İptal). |
| `EFaturaSubmitEnvelope` | ETTN, status, submitted_at, optional message. |
| `EFaturaError` | `BadXml` / `BadTaxId` / `Transport`. |
| `EFaturaProvider` (trait) | `submit_invoice(request)` + `cancel_invoice(env, ettn, reason)`. |
| `MockEFaturaProvider` | 12 unit tests. |
| `validate_vkn(s)` / `validate_tax_id(s)` | Shape checks. |

## What the operator does

### 1. Decide which mandate

- **e-Fatura** is mandatory for issuers/receivers on the GİB
  mukellef registry — invoices flow exclusively through GİB.
- **e-Arşiv** is mandatory for issuers above the turnover
  threshold whose receivers are NOT on the e-Fatura registry
  — invoices flow to GİB as a reporting copy alongside the
  paper / email delivery to the buyer.

### 2. Wire `EFaturaProvider` in the engine

```rust
use invoicekit_report_tr_efatura::{EFaturaProvider, MockEFaturaProvider};

let provider: Box<dyn EFaturaProvider> = Box::new(MockEFaturaProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_tr_efatura_http::HttpEFaturaProvider::new(
//     "https://efatura.gib.gov.tr", api_creds,
// ));
```

### 3. Submission flow at runtime

1. Build canonical UBL-TR XML.
2. `provider.submit_invoice(&request)` → envelope with
   16-char ETTN.
3. Persist ETTN for reconciliation. Within the legal
   cancellation window, the engine can call
   `provider.cancel_invoice(env, ettn, reason)`.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/ubl-tr.xml` | The canonical UBL-TR submitted. |
| `efatura/envelope.json` | Serialised `EFaturaSubmitEnvelope`. |
| `efatura/ettn.txt` | The 16-char ETTN. |

## Validating

```bash
cargo test -p invoicekit-report-tr-efatura
```

## Status today

- Substrate: shipped on main (commit `0c7d511`).
- Live SOAP impl: open follow-up in
  `crates/report-tr-efatura-http`.
- Bead T-824 (Country crate: Turkey e-Fatura) tracks the
  full country adapter on top.

## References

- GİB e-Fatura portal: <https://efatura.gib.gov.tr>.
- Shipped Mock + tests: `crates/report-tr-efatura/src/lib.rs`.
