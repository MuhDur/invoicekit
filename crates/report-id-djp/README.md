<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-id-djp — Indonesia / DJP e-Faktur (PPN VAT clearance)

Typed submission surface for Indonesia's Direktorat Jenderal Pajak (DJP, Directorate General of Taxes) e-Faktur electronic VAT (Pajak Pertambahan Nilai) clearance regime. This crate carries an already-built, already-signed Faktur XML payload to DJP; it does not itself serialize or sign that XML.

## Capabilities

- **Local validation** before the wire: issuer NPWP shape (`validate_npwp` — 15 legacy or 16 PMK 112/2022 ASCII digits), NSFP shape (`validate_nsfp` — exactly 16 ASCII digits, no separators), and a non-empty Faktur XML check.
- **Typed transmission surface**: the `DjpProvider` trait submits one Faktur via `submit_faktur(&DjpSubmitRequest) -> Result<DjpSubmitEnvelope, DjpError>`. The request carries the tenant, environment (`Uat` / `Production`), `kode_jenis`, NPWP, NSFP, and the caller-supplied `faktur_xml: Vec<u8>`. The envelope returns DJP's `nomor_referensi`, the echoed NSFP, a `DjpStatus`, a timestamp, and an optional `alasan` (rejection reason).
- **Type-code mapping**: `FakturKodeJenis` mirrors DJP's `kode_jenis` taxonomy and resolves each variant to its two-digit code via `.code()` (01 standard, 02 government collector, 03 other collector, 04 DPP custom basis, 06 other, 07 export, 08 free/exempt, 09 retail).
- **Deterministic mock**: `MockDjpProvider` returns an `Approved` envelope with a serial `nomor_referensi` and a fixed timestamp, for tests and pipeline wiring.

A DJP-side `Rejected` verdict is not an `Err`; it is surfaced inside the envelope as `DjpStatus::Rejected` so the engine can persist the rejection in its audit trail. `DjpError` is reserved for pre-wire validation failures (`BadXml`, `BadNpwp`, `BadNsfp`) and `Transport` failures.

## Coverage

Opaque-payload / bring-your-own model. The crate does **not** generate Faktur XML and does **not** sign it: the `faktur_xml` field is supplied by the caller as a pre-built, pre-signed byte blob, and validation on it goes no further than rejecting an empty payload. There is no EN 16931 / Universal Business Language family path in this crate.

No live transport ships here. Only `MockDjpProvider` is implemented; the real DJP REST integration lands in a follow-up `report-id-djp-http` crate. NSFP serials are consumed (passed in), not allocated by this crate. NPWP and NSFP checks are shape-only — length and ASCII digits — not checksum or registry validation.

## References

- DJP e-Faktur — `https://efaktur.pajak.go.id` (production), `https://efaktur-uat.pajak.go.id` (UAT sandbox)
- PMK 112/2022 — 16-digit NPWP transition

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
