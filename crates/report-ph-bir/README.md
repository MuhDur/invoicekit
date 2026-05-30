<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-ph-bir — Philippines (Bureau of Internal Revenue, BIR EIS clearance)

Typed submission adapter for the Philippine Bureau of Internal Revenue (BIR) Electronic Invoicing System (EIS), the BIR e-invoicing clearance regime. It carries a caller-supplied canonical JSON payload to EIS and surfaces the per-invoice verdict; it does not build or sign that payload itself.

## Capabilities

- **Transmit (typed surface only).** `EisProvider::submit_invoice` takes an `EisSubmitRequest` (tenant id, environment, document kind, issuer TIN, ATP reference, and a `invoice_json: Vec<u8>`) and returns an `EisSubmitEnvelope` (BIR-assigned reference number, status, RFC-3339 acknowledged-at timestamp, and an optional rejection reason).
- **Local validation** before the wire: `validate_tin` enforces the issuer TIN shape (9 ASCII base digits with an optional `-NNN` three-digit branch suffix); `submit_invoice` additionally rejects an empty ATP and an empty payload. These fail as `EisError::BadTin` / `EisError::MissingAtp` / `EisError::BadJson`.
- **Document-kind taxonomy.** `EisDocumentKind` mirrors BIR's EIS classes: Sales Invoice, Official Receipt (services), Credit Memo, Debit Memo, Billing Invoice.
- **Deterministic mock.** `MockEisProvider` returns an `Acknowledged` envelope with a serial reference number (`BIR-NNNNNNNNNNNN`) and a fixed timestamp, for tests and pipeline wiring.

A BIR-side `Rejected` verdict is not an `Err`; it is surfaced inside the envelope as `EisStatus::Rejected` (with `reason`) so the engine can persist the rejection in its audit trail. `EisError` is reserved for pre-wire shape failures and `Transport` failures.

## Coverage

Opaque-payload adapter. This is the bring-your-own-payload model: the caller produces the canonical `invoice_json`; this crate validates the request shape and defines the submission contract. The crate does **not** serialize the EIS document format, does **not** sign, and has **no** EN 16931 / Universal Business Language family path. Validation of the payload goes no further than rejecting an empty byte blob.

Documented residuals:

- **No live transport.** Only `MockEisProvider` ships. The live BIR EIS REST integration against `eis.bir.gov.ph` (production) / `eis-sandbox.bir.gov.ph` (sandbox) lands in a follow-up `report-ph-bir-http` crate.
- **Shape-only identifier checks.** The TIN check is length and ASCII-digit only — not checksum or BIR registry validation. The ATP (Authority To Print) reference is checked only for non-emptiness, not verified against BIR accreditation.

## References

- BIR EIS — `eis.bir.gov.ph` (production), `eis-sandbox.bir.gov.ph` (sandbox)

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
