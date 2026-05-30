<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-za-sars — South Africa (South African Revenue Service, SARS e-Invoicing)

Typed submission adapter for South Africa's South African Revenue Service (SARS) e-Invoicing regime. Issuers submit typed JSON envelopes to SARS and receive a Reference plus an acceptance status; this crate carries the already-signed payload as an opaque blob and surfaces the per-invoice verdict — it does not build or sign that payload itself.

## Capabilities

- **Transmit (typed surface only).** Defines the `SarsProvider` trait with one method, `submit_invoice`, taking a `SarsSubmitRequest` (tenant id, environment, issuer VAT registration, and `payload: Vec<u8>`) and returning a `SarsSubmitEnvelope` (SARS-assigned reference, status, RFC-3339 UTC recorded timestamp, optional rejection reason).
- **Local validation before the wire.** `validate_vat` enforces the issuer VAT registration shape (10 ASCII digits starting with `4`); `submit_invoice` additionally rejects an empty payload. Both fail before the wire as `SarsError::BadVat` / `SarsError::BadPayload`.
- **Verdict modeling.** A SARS `Rejected` decision is surfaced inside the returned envelope (`SarsStatus::Rejected` with a `reason`), not as a transport error, so the engine can persist the rejection alongside its audit trail. Only pre-wire validation failures and transport failures are `SarsError`.
- **Deterministic mock.** `MockSarsProvider` issues serial references (`ZA-000000000001`, …) and a fixed timestamp for tests and golden fixtures; `with_fixed_recorded_at` overrides the timestamp.

It does **not** serialize the SARS document format, compute the canonical JSON, sign anything, perform the EN 16931 / UBL 2.1 family path, or talk to SARS over the network. The `payload` field is a pre-built, pre-signed blob supplied by the caller.

## Coverage

**Opaque payload / bring-your-own.** This is the bring-your-own-payload model: the caller produces the canonical signed JSON the issuer intends to submit, and this crate passes it through. The crate owns only the request/response shapes and the pre-wire checks above; it does not construct, canonicalize, or sign the SARS payload.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockSarsProvider` ships, returning `Accepted` with a synthetic reference. As the module doc-comment states, the live SARS REST integration lands in a follow-up `report-za-sars-http` crate; the typed surface here is what that crate will implement.
- **Validation is intentionally minimal.** The VAT check is a shape check (10 ASCII digits starting with `4`), not a registration check.

## References

The source cites no external regulator or specification URLs. The only forward reference in the module doc-comment is the planned follow-up `report-za-sars-http` crate for the live SARS REST integration.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
