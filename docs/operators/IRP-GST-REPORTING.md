# India IRP / GST e-invoicing

`crates/report-in-gst` ships the typed Provider trait +
envelope + Mock implementation for India's GST e-invoicing
mandate via the Invoice Registration Portal (IRP). Every
notified Indian taxpayer (₹5 crore turnover threshold at time
of writing) issues B2B invoices through an IRP; the IRP
validates the payload, assigns a 64-character **IRN**
(Invoice Reference Number), signs a JWS over the invoice,
and returns a base-64 signed QR for the printed invoice.

## Why a runbook and not a fully-wired live impl

- A registered **GSP** (GST Suvidha Provider) or direct NIC
  account for IRP1 (`einvoice1.gst.gov.in`) / IRP2
  (`einvoice2.gst.gov.in`).
- Username + password + the IRP's signed `auth_token` (renewed
  every 6 hours).
- A 15-character GSTIN tied to the issuer's PAN.
- A pinned IRP schema bundle for the `version` the operator
  targets (Schema-1.1 at time of writing).

The Mock in `report_in_gst::MockIrpProvider` covers everything
except those operator-side credentials.

## Crate shape (already shipped)

`crates/report-in-gst/src/lib.rs`:

| Item | Role |
|---|---|
| `IrpBackend` | `Nic1` / `Nic2` / `Gsp(String)`. Vendor label on the Gsp variant pins cassettes to a specific private IRP (IRIS, EY, Cygnet, etc.). |
| `IrpEnvironment` | `Sandbox` vs `Production`. |
| `IrpStatus` | `Accepted` / `Duplicate` / `Rejected`. Duplicate returns the existing IRN so the engine reconciles instead of issuing fresh. |
| `IrpRegisterRequest` | Tenant, env, backend, issuer GSTIN, optional buyer GSTIN, canonical IRP JSON payload. |
| `IrpRegisterEnvelope` | Status, IRN, ack_no, ack_dt, signed_qr_code (base-64), signed_invoice_jws, error_message. |
| `IrpError` | `BadJson` / `BadGstin` / `Transport`. |
| `IrpProvider` (trait) | `register_invoice(request)`. |
| `MockIrpProvider` | 14 unit tests. Synthesises 64-hex IRN + serial ACK + mock QR + mock JWS; tracks seen IRNs to emit `Duplicate` on resubmit. |
| `validate_gstin(s)` | 15-char ASCII alphanumeric check. |
| `validate_hsn_sac(c)` | 4-to-8-digit Harmonised System / Services Accounting Code check. |

## What the operator does

### 1. Pick a backend + acquire credentials

- **NIC IRP1 / IRP2** — government-run, free. Register at
  `https://einvoice1.gst.gov.in/`. Production access requires
  the taxpayer to be activated on the IRP roster.
- **Private GSP / IRP** — additional ergonomics, integrations,
  and SLA. Pick from the IRP-accredited GSP list at
  `https://www.gstn.org.in/gsp`.

GSPs forward to NIC under the hood; the wire shape from the
engine's perspective is identical.

### 2. Wire `IrpProvider` in the engine

```rust
use invoicekit_report_in_gst::{IrpProvider, MockIrpProvider};

let provider: Box<dyn IrpProvider> = Box::new(MockIrpProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_report_in_gst_http::HttpIrpProvider::new(
//     /* base_url */ "https://einv-apisandbox.nic.in",
//     /* username */ user,
//     /* password */ pw_secret,
//     /* client_id + secret */ client_id, client_secret_ref,
// ));
```

### 3. Reporting flow at runtime

1. Build the canonical IRP JSON payload (Schema-1.1 at time
   of writing — use the IRP-published schema bundle).
2. Validate `issuer_gstin` and `buyer_gstin` shape (the
   provider validates again before the wire — passing them
   pre-checked is faster).
3. `provider.register_invoice(&request)` →
   `IrpRegisterEnvelope`.
4. On `Accepted`: embed the `signed_qr_code` into the printed
   PDF, persist the `irn` + `ack_no` for reconciliation,
   archive the `signed_invoice_jws` in the evidence bundle.
5. On `Duplicate`: reconcile the existing IRN with the
   engine's local outbox state — the IRP already accepted the
   invoice the first time.
6. On `Rejected`: surface `error_message` to the engine's
   audit log, fix the payload, resubmit.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/irp-invoice.json` | Canonical IRP JSON the engine submitted. |
| `irp/envelope.json` | Serialised `IrpRegisterEnvelope`. |
| `irp/irn.txt` | The 64-char IRN (chain pointer). |
| `irp/signed-qr.b64` | Base-64 PNG of the signed QR. |
| `irp/signed-invoice.jws` | The IRP-signed JWS (for offline verification). |

## Duplicate detection

The IRP computes the IRN as SHA-256 over a canonical
projection of the invoice fields (issuer GSTIN + invoice
number + financial year). Two registrations of the same
invoice produce the same IRN — the IRP returns
`IrpStatus::Duplicate` with the existing IRN rather than
double-registering. The Mock implements this contract via an
internal `BTreeSet<String>` of seen IRNs so cassette-replay
tests can exercise both the first-submit + resubmit paths
without spinning up a real IRP.

## Validating

```bash
cargo test -p invoicekit-report-in-gst
```

14 unit tests cover the lifecycle: accepted happy path,
duplicate detection, all error paths (empty payload, bad
issuer/buyer GSTIN), export-without-buyer acceptance, GSTIN
and HSN/SAC validation, backend + envelope serde
round-trips.

## Status today

- Substrate: shipped on main (commit `3bdd13c`).
- Live IRP HTTP impl: open follow-up. Best path is one crate
  per backend behind feature flags — `report-in-gst-nic` for
  the government IRPs, plus `report-in-gst-<gsp>` per
  contracted GSP.
- Bead T-820 (Country crate: India IRP / GST) tracks the
  higher-level country adapter (state machine + e-waybill +
  e-invoice cancellation) on top of this reporting layer.

## References

- GST e-invoice portal: <https://einvoice1.gst.gov.in/>.
- NIC e-invoice API docs: <https://einv-apisandbox.nic.in/>.
- GSP roster: <https://www.gstn.org.in/gsp>.
- Shipped Mock + tests: `crates/report-in-gst/src/lib.rs`.
