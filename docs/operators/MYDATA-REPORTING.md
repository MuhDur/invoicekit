# Greece myDATA reporting

`crates/report-gr-mydata` ships the typed Provider trait +
envelope + Mock implementation for Greece's **myDATA**
(Άυλο Διασύνδεσμο) continuous-reporting mandate. Issuers
transmit invoice summaries to the IAPR (Independent Authority
for Public Revenue, ΑΑΔΕ); the IAPR returns a **MARK**
(Μοναδικός Αριθμός Καταχώρησης) + **UID** that the issuer
must embed in the printed-invoice QR code.

## Why a runbook and not a fully-wired live impl

- A registered **myDATA account** at <https://www.aade.gr/mydata>
  with the issuer's ΑΦΜ (Α.Φ.Μ. — Greek tax registration
  number).
- API credentials (`User-Id` + `Subscription-Key`) for the
  IAPR REST endpoints at `mydata-dev.azure-api.net` (sandbox)
  or `mydatapi.aade.gr` (production).
- A pinned myDATA schema bundle for the `invoiceType` release
  the operator targets (1.0.10+ at time of writing).

The Mock in `report_gr_mydata::MockMyDataProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-gr-mydata/src/lib.rs`:

| Item | Role |
|---|---|
| `MyDataEnvironment` | `Sandbox` vs `Production`. |
| `MyDataInvoiceCategory` | Typed taxonomy: `SalesGoods{code}` (1.x), `Services{code}` (2.x), `SelfBilling{code}` (3.x), `CreditNote{code}` (5.x), `Statement{code}` (8.x), `Other{code}`. |
| `MyDataMark(String)` | Newtype for the IAPR-issued MARK. |
| `MyDataUid(String)` | Newtype for the IAPR-computed UID. |
| `MyDataStatus` | `Accepted` / `AcceptedWithWarnings` / `Rejected`. |
| `MyDataReportRequest` | Tenant, env, issuer ΑΦΜ, optional buyer ΑΦΜ, category, InvoicesDoc XML. |
| `MyDataReportEnvelope` | Status, MARK, UID, message, reported_at. |
| `MyDataError` | `BadXml` / `BadAfm` / `Transport`. |
| `MyDataProvider` (trait) | `report_invoice(request)`. |
| `MockMyDataProvider` | 12 unit tests. |
| `validate_afm(...)` | 9-digit ASCII check. |
| `qr_payload(base_url, envelope)` | Renders the printed-invoice QR per IAPR Annex 1. |

## What the operator does

### 1. Obtain credentials

Register at `https://www.aade.gr/mydata`, link the issuer's
ΑΦΜ, and generate API credentials. The IAPR's sandbox
(`mydata-dev.azure-api.net`) is open to any registered
account; production requires a separate activation.

### 2. Wire `MyDataProvider` in the engine

```rust
use invoicekit_report_gr_mydata::{MockMyDataProvider, MyDataProvider};

let provider: Box<dyn MyDataProvider> = Box::new(MockMyDataProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_gr_mydata_http::HttpMyDataProvider::new(
//     "https://mydatapi.aade.gr",
//     /* user_id */ user,
//     /* subscription_key */ key_secret,
// ));
```

### 3. Reporting flow at runtime

1. Build the canonical InvoicesDoc XML for the invoice (use
   the IAPR-published XSD bundle pinned per release).
2. Classify the invoice via the `MyDataInvoiceCategory` enum.
3. `provider.report_invoice(&request)` → `MyDataReportEnvelope`.
4. Embed the QR via `qr_payload("https://www.aade.gr/mydata", &envelope)?`
   into the printed PDF.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/mydata-invoicesdoc.xml` | The InvoicesDoc XML accepted by the IAPR. |
| `mydata/mark.txt` | The IAPR-assigned MARK. |
| `mydata/uid.txt` | The IAPR-computed UID. |
| `mydata/envelope.json` | Serialised `MyDataReportEnvelope`. |
| `mydata/qr-payload.txt` | The QR-code URL string. |

## Validating

```bash
cargo test -p invoicekit-report-gr-mydata
```

12 unit tests cover accepted happy path, serial MARK increment,
empty-XML and bad-AFM rejection, category code borrow,
AFM/SHA validation, QR rendering, and the serde round-trip
for `AcceptedWithWarnings`.

## Status today

- Substrate: shipped on main (commit `8237db0`).
- Live HTTP impl: open follow-up in
  `crates/report-gr-mydata-http`.
- Bead T-813 (Country crate: Greece myDATA) tracks the
  higher-level country adapter (state machine + cassette set
  + reconciliation API) on top of this reporting layer.

## References

- IAPR myDATA portal: <https://www.aade.gr/mydata>.
- Shipped Mock + tests: `crates/report-gr-mydata/src/lib.rs`.
