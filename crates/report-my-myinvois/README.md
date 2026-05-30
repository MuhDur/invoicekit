# invoicekit-report-my-myinvois — Malaysia / LHDNM / MyInvois clearance

Typed adapter surface for Malaysia's MyInvois near-real-time clearance portal, operated by Lembaga Hasil Dalam Negeri Malaysia (LHDNM, the Inland Revenue Board). It models the submit → UUID/Long-ID → validate/cancel lifecycle as a Rust surface plus a deterministic mock; the invoice payload itself is supplied by the caller as an opaque blob.

## Capabilities

- **Typed request/response surface.** `MyInvoisSubmitRequest` carries the tenant, environment (`Sandbox` / `Production`), document class (`MyInvoisDocumentKind`), issuer TIN, issuer BRN, optional buyer TIN, and the UBL XML payload bytes. `MyInvoisSubmitEnvelope` returns the LHDNM-assigned `uuid`, the 64-char `content_hash_hex`, the `long_id` (the code the buyer uses to validate on the public portal), the `MyInvoisStatus` verdict, the recorded timestamp, and an optional rejection reason.
- **Document-class taxonomy.** `MyInvoisDocumentKind` enumerates the eight LHDNM classes — `Invoice`, `CreditNote`, `DebitNote`, `RefundNote` and their self-billed counterparts — and `code()` returns the LHDNM `eInvoiceTypeCode` for each (`01`–`04`, `11`–`14`).
- **Local shape validation.** `validate_tin` enforces a `C` prefix plus exactly 10 ASCII digits; `validate_brn` enforces exactly 12 ASCII digits. Both run before any transport and are surfaced as `MyInvoisError::BadTin` / `MyInvoisError::BadBrn`. The mock also rejects an empty payload as `MyInvoisError::BadXml`.
- **Deterministic mock provider.** `MockMyInvoisProvider` implements the `MyInvoisProvider` trait (`submit_invoice` + `cancel_invoice`) with fixed timestamps and incrementing serials, deriving a synthesized UUID and content hash from the payload so cassette-replay tests stay byte-identical across runs. `cancel_invoice` always returns `MyInvoisStatus::Cancelled`.

This crate does **not** serialize the national payload, sign anything, or talk to LHDNM. The `invoice_xml` field is an **opaque blob** — canonical UBL XML conforming to LHDNM's Peppol-derived schema — supplied by the caller; this crate validates the envelope shape around it, not its contents. Per the module doc-comment, the live LHDNM REST integration lands in a follow-up `report-my-myinvois-http` crate behind a feature flag.

## Coverage

This is a **clearance adapter**, not a national-format serializer. It models the MyInvois flow — submit one invoice, receive the LHDNM-issued UUID, content hash, and Long ID, then validate or cancel within the 72-hour grace window — as a typed Rust surface plus a deterministic mock.

Documented residuals and boundaries, mirroring the module doc-comment:

- **No payload serialization.** The UBL XML body is opaque to this crate; the caller produces it. The UBL/CII serializers in the workspace are a separate concern.
- **No live transport.** No HTTP, TLS, or DNS to LHDNM. Only `MockMyInvoisProvider` ships; the real transport is the deferred `report-my-myinvois-http` crate. The environment hosts (`preprod-api.myinvois.hasil.gov.my` sandbox, `api.myinvois.hasil.gov.my` production) are recorded on `MyInvoisEnvironment` but not contacted.
- **No signing.** The portal returns a signed acknowledgement; this crate does not mint or verify any signature. The mock's `content_hash_hex` is a synthesized placeholder derived from the payload length and first bytes, not a real BLAKE3/SHA-256 digest of canonical content.
- **Validation is shape-only.** TIN and BRN are checked for prefix/length and ASCII-digit composition; there is no check-digit, registry, or business-rule validation.
- **LHDNM verdicts are data, not errors.** A `Rejected` outcome is returned inside `MyInvoisSubmitEnvelope` (so the engine can persist it in the audit trail), not raised as `MyInvoisError`. `MyInvoisError` is reserved for pre-wire shape failures and transport faults.

## References

Authorities and endpoints named in the source:

- LHDNM — Lembaga Hasil Dalam Negeri Malaysia (Inland Revenue Board), operator of the MyInvois portal.
- `eInvoiceTypeCode` — the LHDNM document-class taxonomy mirrored by `MyInvoisDocumentKind` (`01`–`04`, `11`–`14`).
- Environment hosts: `preprod-api.myinvois.hasil.gov.my` (sandbox), `api.myinvois.hasil.gov.my` (production).

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
