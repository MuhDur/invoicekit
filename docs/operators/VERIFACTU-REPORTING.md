# Spain VeriFactu reporting

`crates/report-es-verifactu` ships the typed Provider trait +
envelope + Mock implementation for Spain's **VeriFactu**
anti-fraud regime under Real Decreto 1007/2023. Each invoice
carries a SHA-256 hash chain pointing at the previous
invoice's hash so the AEAT can detect deletions or
back-dating; printed/PDF invoices carry a QR code linking to
the AEAT `ValidarQR` portal.

## Why a runbook and not a fully-wired live impl

- An **AEAT certificate** (digital certificate issued by the
  Fábrica Nacional de Moneda y Timbre, FNMT, or any Spanish
  QTSP) for mTLS to the AEAT endpoints.
- The issuer's NIF / DNI / NIE registered with the AEAT.
- A pinned VeriFactu schema bundle for the operating tier
  (sandbox `preprod.agenciatributaria.gob.es` vs production
  `aeat.es`).

The Mock in `report_es_verifactu::MockVeriFactuProvider`
covers everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-es-verifactu/src/lib.rs`:

| Item | Role |
|---|---|
| `VeriFactuEnvironment` | `Sandbox` (preproducción) vs `Production`. |
| `VeriFactuMode` | `VeriFactu` (real-time reporting) vs `NoVeriFactu` (SIF local hash-chain mode). |
| `VeriFactuRegisterRequest` | Tenant, env, mode, issuer NIF, invoice number, issued_at, optional previous-hash, canonical XML. |
| `VeriFactuStatus` | `Accepted` / `AcceptedWithWarnings` / `Rejected`. |
| `VeriFactuRegisterEnvelope` | Status, AEAT-recorded SHA-256 hex (engines persist for the next chain step), CSV, message, recorded_at. |
| `VeriFactuError` | `BadXml` / `BadNif` / `BadPreviousHash` / `Transport`. |
| `VeriFactuProvider` (trait) | `register_invoice(request)`. |
| `MockVeriFactuProvider` | 13 unit tests. |
| `validate_nif(...)` | 9-char ASCII alphanumeric shape check. |
| `validate_sha256_hex(...)` | 64 lowercase hex chars check. |
| `qr_payload(...)` | Renders the AEAT `ValidarQR` URL. |

## What the operator does

### 1. Obtain credentials

Acquire an AEAT-recognized digital certificate (FNMT, AC
Camerfirma, etc.). The certificate is the same secret class
as a TLS private key — store via OS secret store or HSM.

### 2. Wire `VeriFactuProvider` in the engine

```rust
use invoicekit_report_es_verifactu::{MockVeriFactuProvider, VeriFactuProvider};

let provider: Box<dyn VeriFactuProvider> = Box::new(MockVeriFactuProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_es_verifactu_http::HttpVeriFactuProvider::new(
//     /* mtls_cert */ ..., /* mtls_key */ ...,
//     /* base_url */ "https://www2.agenciatributaria.gob.es/wlpl/inwinvoc/...",
// ));
```

### 3. Reporting flow at runtime

1. Build the canonical XML payload (use the AEAT VeriFactu
   XSDs).
2. Look up the previous invoice's `recorded_hash_hex` from
   the engine's local chain store. Pass it as
   `previous_hash_hex`; only the chain-root invoice uses
   `None`.
3. `provider.register_invoice(&request)` →
   `VeriFactuRegisterEnvelope`.
4. Persist the returned `recorded_hash_hex` as the new chain
   tip; embed the `csv` into the printed invoice's QR via
   `qr_payload(...)`.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/verifactu-invoice.xml` | The canonical XML accepted by the AEAT. |
| `verifactu/recorded-hash.txt` | The AEAT-recorded SHA-256 (chain pointer). |
| `verifactu/csv.txt` | The CSV embedded in the printed-invoice QR. |
| `verifactu/envelope.json` | Serialised `VeriFactuRegisterEnvelope`. |
| `verifactu/qr-payload.txt` | The QR-code URL. |

## Chain-root handling

The first invoice an issuer reports has no previous-hash; the
engine passes `previous_hash_hex: None` to declare it the
chain root. The AEAT records the chain root specially. From
the second invoice onward, the engine MUST supply the
previous invoice's `recorded_hash_hex` — any gap is treated
as evidence of deleted invoices by the AEAT.

## Validating

```bash
cargo test -p invoicekit-report-es-verifactu
```

13 unit tests cover the lifecycle: accepted happy path, CSV
serial increment, chained previous-hash, all four error
variants (bad-hash, bad-NIF, empty XML, transport), NIF +
SHA validation helpers, QR rendering, and the serde
round-trip.

## Status today

- Substrate: shipped on main (commit `2133834`).
- Live SOAP/REST impl: open follow-up in
  `crates/report-es-verifactu-http`.
- Bead T-812 (Country crate: Spain VeriFactu) tracks the
  higher-level country adapter on top of this reporting
  layer.

## References

- AEAT VeriFactu portal:
  <https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu.html>
- Shipped Mock + tests: `crates/report-es-verifactu/src/lib.rs`.
