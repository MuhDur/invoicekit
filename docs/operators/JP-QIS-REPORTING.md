# Japan QIS (Qualified Invoice System) reporting

`crates/report-jp-qis` ships the typed JP-specific overlay
for Japan's Qualified Invoice System (適格請求書発行事業者制度)
— the NTA-issued registration regime in effect since October
2023. Japan does NOT operate a clearance portal; the NTA only
runs a registration registry the buyer pings to confirm an
issuer is registered. Wire delivery is via Peppol-JP (Peppol
BIS Billing 3 with the Japanese CIUS) — the engine delegates
the AS4 send to `crates/transmit-peppol`.

## Why a runbook and not a fully-wired live impl

- An NTA-issued registration number (`T` + 13 digits). Apply
  via the issuer's national tax office.
- API credentials for the NTA registry lookup at
  `kokuzei-test.nta.go.jp` (sandbox) or `kokuzei.nta.go.jp`
  (production).
- A Peppol AP relationship for the delivery leg — partner AP
  (T-091) or self-hosted phase4 (T-092).

The Mock in `report_jp_qis::MockQisRegistryProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-jp-qis/src/lib.rs`:

| Item | Role |
|---|---|
| `NtaEnvironment` | `Sandbox` vs `Production`. |
| `QisInvoiceKind` | `Qualified` (適格請求書, full) vs `Simplified` (適格簡易請求書, allowed for retail/restaurant/transport). |
| `JctCategory` | `Standard10` (1000 bp, 10%), `Reduced8` (800 bp, 8% food + newspapers), `Zero`, `Exempt`. |
| `QisIssuerRegistration` | registration_number, legal_name, effective_from, optional revoked_at. |
| `QisLookupRequest` | Tenant, env, registration_number. |
| `QisError` | `BadRegistrationNumber` / `NotFound` / `Transport`. |
| `QisRegistryProvider` (trait) | `lookup(request)`. |
| `MockQisRegistryProvider` | 10 unit tests. Operator code can `.revoke(num)` to flip specific numbers into the revoked state for cassette-replay tests. |
| `validate_registration_number(s)` | `T` + 13 ASCII digits. |
| `jct_basis_points(category)` | Typed bp lookup for arithmetic. |

## What the operator does

### 1. Acquire a registration number

Apply via the issuer's national tax office. Once issued, the
registration number is `T` followed by 13 ASCII digits. The
NTA registry exposes a public lookup endpoint the buyer pings
to confirm the issuer is registered.

### 2. Wire `QisRegistryProvider` in the engine

```rust
use invoicekit_report_jp_qis::{MockQisRegistryProvider, QisRegistryProvider};

let provider: Box<dyn QisRegistryProvider> =
    Box::new(MockQisRegistryProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_jp_qis_http::HttpQisRegistryProvider::new(
//     "https://kokuzei.nta.go.jp", api_creds,
// ));
```

### 3. Issuance flow at runtime

1. Stamp the invoice with the issuer's
   `registration_number`.
2. Project per-line JCT categories into `JctCategory` so the
   engine can compute totals against the correct basis
   points (`jct_basis_points(category)`).
3. Deliver via Peppol-JP (the engine calls
   `crates/transmit-peppol`).
4. Buyer-side: `provider.lookup(&request)` validates the
   issuer is currently registered (not revoked) before
   claiming JCT input credit.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/peppol-jp-billing.xml` | The Peppol BIS Billing 3 + JP CIUS UBL. |
| `qis/issuer-registration.json` | Serialised `QisIssuerRegistration` snapshot. |

## Validating

```bash
cargo test -p invoicekit-report-jp-qis
```

## Status today

- Substrate: shipped on main (commit `cd4619a`).
- Live NTA registry HTTP impl: open follow-up in
  `crates/report-jp-qis-http`.
- Bead T-827 (Country crate: Japan QIS) tracks the full
  country adapter on top, including the Peppol-JP CIUS
  overlay.

## References

- NTA QIS portal: <https://www.nta.go.jp/taxes/shiraberu/zeimokubetsu/shohi/keigenzeiritsu/invoice.htm>.
- Shipped Mock + tests: `crates/report-jp-qis/src/lib.rs`.
- Related runbook: [`PARTNER-PEPPOL-AP.md`](./PARTNER-PEPPOL-AP.md)
  for the Peppol transport layer.
