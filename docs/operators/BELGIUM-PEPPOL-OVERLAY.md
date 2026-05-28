# Belgium Peppol-overlay reporting

`crates/report-be-peppol` ships the typed Provider trait +
envelope + Mock implementation for Belgium's B2G (since 2019)
+ B2B (ramping from 2026) Peppol-based e-invoicing mandate.
Belgium uses Peppol BIS Billing 3 as the wire format. The
federal portal **Mercurius** receives B2G invoices and the
**Hermes** access point routes B2B invoices through Peppol to
the receiver's chosen access point.

## Why a runbook and not a fully-wired live impl

- Hermes / Mercurius API credentials (signed agreement with
  the Service Public Fédéral Stratégie et Appui — BOSA).
- A qualified electronic certificate from a Belgian QTSP
  (Cybertrust, QuoVadis, etc.) for the signing leg — see
  [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md).
- A Peppol AP relationship for the B2B path — partner AP
  (T-091) or self-hosted phase4 (T-092).

The Mock in `report_be_peppol::MockBePeppolProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-be-peppol/src/lib.rs`:

| Item | Role |
|---|---|
| `BePeppolEnvironment` | `Sandbox` (mercurius-test.fedict.be / Peppol test) vs `Production`. |
| `BePeppolMandate` | `B2g` (Mercurius), `B2b` (Hermes / Peppol), `B2cReporting` (forthcoming RD/AR). |
| `BePeppolReceiver` | `Kbo(10 digits)` / `VatId(BE + 10 digits)` / `PeppolParticipant(scheme:value)`. |
| `BePeppolVatCategory` | Belgian BTW/TVA overlay: `Standard` (21%), `Reduced12`, `Reduced6`, `Zero`, `Exempt`, `ReverseCharge`. |
| `BePeppolDeliverRequest` / `BePeppolDeliverEnvelope` / `BePeppolStatus` (Submitted/Delivered/Accepted/Rejected/ValidationFailed) / `BePeppolError`. |
| `BePeppolProvider` (trait) | `deliver(request)` + `poll_status(env, submission_id)`. |
| `MockBePeppolProvider` | 13 unit tests; submission ids encode routing prefix (MERC-SBX / MERC-PROD / HERMES-SBX / HERMES-PROD / B2CREP-*). |
| `validate_receiver(...)` | Shape-checks all three receiver variants. |
| `validate_vat_categories(...)` | Enforces non-empty + rejects mixing Exempt with Standard on the same invoice (Mercurius rule). |

## What the operator does

### 1. Pick mandate + transport

- **B2G** invoices go to Mercurius. Sandbox tests run
  against `mercurius-test.fedict.be`; production is
  `mercurius.fedict.be`.
- **B2B** invoices flow through Peppol. Hermes is one access
  point; any Peppol-conformant AP is acceptable. The engine
  delegates the actual AS4 delivery to
  `crates/transmit-peppol` — this crate adds the Belgian
  overlay (BTW categorisation, KBO lookup, mandate routing).

### 2. Wire `BePeppolProvider` in the engine

```rust
use invoicekit_report_be_peppol::{BePeppolProvider, MockBePeppolProvider};

let provider: Box<dyn BePeppolProvider> = Box::new(MockBePeppolProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_be_peppol_http::HttpBePeppolProvider::new(
//     /* mercurius_creds */ ...,
//     /* peppol_ap */ partner_ap_handle,
// ));
```

### 3. Delivery flow at runtime

1. Build the canonical Peppol BIS Billing 3 UBL invoice (use
   `crates/format-ubl` + `crates/profile-peppol-bis`).
2. Resolve the receiver lookup key:
   - `Kbo(...)` when the buyer's Belgian enterprise number
     is known.
   - `VatId("BE...")` when only the VAT id is known.
   - `PeppolParticipant("scheme:value")` when the receiver
     pre-declared a Peppol participant identifier.
3. Project per-line BTW categories into
   `BePeppolVatCategory` so the engine can pre-flight
   against Mercurius's stricter rules before the wire.
4. `provider.deliver(&request)` →
   `BePeppolDeliverEnvelope`. Persist the `submission_id`
   for reconciliation.
5. Poll via `provider.poll_status(...)` from the engine's
   reconciliation loop.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/peppol-bis-billing3.xml` | The signed Peppol UBL invoice. |
| `be-peppol/envelope.json` | Serialised `BePeppolDeliverEnvelope`. |
| `be-peppol/receiver.json` | The receiver lookup key the engine resolved. |
| `be-peppol/mlr.xml` | Peppol Message Level Response (when received). |

## BTW categorisation rules

Mercurius validates BTW categorisation more strictly than
plain Peppol BIS. Two cross-line rules the engine MUST
enforce locally before the wire:

1. **Empty categories** → reject. Every line declares a
   category.
2. **`Exempt` + `Standard` mix** on the same invoice →
   reject. The Belgian framework treats this as evidence of
   misclassification.

`validate_vat_categories(...)` captures both rules. The
provider's `deliver(...)` invokes it automatically.

## Validating

```bash
cargo test -p invoicekit-report-be-peppol
```

13 unit tests cover deliver happy paths for B2G/B2B, serial
id increment, empty-payload rejection, bad-KBO / bad-VAT /
bad-participant rejection, well-formed participant
acceptance, empty-categories rejection, Exempt+Standard
mix rejection, poll_status happy + empty-id rejection, and
the serde round-trip on a ValidationFailed envelope.

## Status today

- Substrate: shipped on main (commit `90f871d`).
- Live Mercurius/Hermes HTTP impl: open follow-up in
  `crates/report-be-peppol-http`.
- Bead T-802 (Belgium Peppol overlay archetype) tracks the
  higher-level archetype trait + state machine + cassette
  set + reconciliation that this substrate plugs into.

## References

- Belgian federal e-invoicing portal:
  <https://efacture.belgium.be/>
- Mercurius developer portal: <https://mercurius-test.fedict.be>
  (sandbox).
- Hermes Peppol AP: <https://hermes.belgium.be>.
- Shipped Mock + tests: `crates/report-be-peppol/src/lib.rs`.
- Related runbook: [`PARTNER-PEPPOL-AP.md`](./PARTNER-PEPPOL-AP.md)
  for the Peppol transport layer.
