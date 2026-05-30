<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-vn-gdt

Vietnam — General Department of Taxation (GDT) e-Invoice clearance adapter.

Submits a caller-supplied signed XML payload to the GDT clearance regime at `hoadondientu.gdt.gov.vn` and returns the `mã CQT` (tax authority code), status, and recorded timestamp. This crate does not build or sign the XML; it carries an opaque payload to the authority and surfaces the verdict.

## Capabilities

- **Transmit (typed surface only).** Defines the `GdtProvider` trait — one `submit_invoice` call that takes a `GdtSubmitRequest` (tenant, environment, issuer MST, signed XML bytes) and returns a `GdtSubmitEnvelope` (`mã CQT`, status, RFC-3339 recorded timestamp, optional rejection message).
- **Pre-wire shape validation.** `validate_mst` requires the issuer tax code (mã số thuế / MST) to be 10 or 13 ASCII digits — 13 when including the 3-digit branch suffix — and rejects anything else. `submit_invoice` also rejects an empty XML payload. These are shape checks only.
- **Environment selector.** `GdtEnvironment` names the sandbox (`hoadondientu-test.gdt.gov.vn`) and production (`hoadondientu.gdt.gov.vn`). The selector is carried on the request; no transport in this crate connects to either.
- **Verdict modelling.** An authority refusal is a `GdtStatus::Rejected` carried inside the `Ok` envelope, never an `Err`, so the engine persists the rejection alongside its audit trail. A refused submission gets no `mã CQT` and carries the GDT reason text (`thông báo`). `GdtError` is reserved for pre-wire shape failures (bad MST, empty payload) and transport failures.
- **Deterministic mock.** `MockGdtProvider` runs the shape validation and returns a fixed-timestamp envelope with a monotonic `VN-…` code; `with_forced_status` / `with_rejection` drive the `Rejected` branch so callers can exercise the verdict path without a live backend.

## Coverage

Opaque-payload / partner model. The crate does **not** serialize GDT e-Invoice XML from the InvoiceKit intermediate representation and does **not** sign it with the GDT-registered digital certificate. The caller supplies an already-signed `invoice_xml: Vec<u8>` blob; this crate validates the issuer MST shape, refuses an empty payload, and conveys the payload to the GDT.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockGdtProvider` ships. The live GDT REST integration lands in a follow-up `report-vn-gdt-http` crate. Until then nothing in this crate reaches `hoadondientu.gdt.gov.vn`.
- **Authority rejection is a verdict, not an error.** A GDT `Bị từ chối` refusal surfaces inside the envelope as `GdtStatus::Rejected` with the `thông báo` reason, never as an `Err`. Only pre-wire shape failures return `Err`.
- **MST validation is shape only.** `validate_mst` confirms length (10 or 13) and ASCII-digit content; it does not confirm the MST is registered with the GDT or active.
- **No EN 16931 / UBL path.** This crate does not route through the European format family. (`invoicekit-ir`, `invoicekit-format-ubl`, `invoicekit-canonical`, `invoicekit-evidence`, and `invoicekit-verify` are dev-dependencies used by tests only.)

## References

Only sources named in the source are listed.

- GDT e-Invoice portal (production): `hoadondientu.gdt.gov.vn`
- GDT e-Invoice portal (sandbox): `hoadondientu-test.gdt.gov.vn`

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
