# invoicekit-report-ke-etims

Kenya / Kenya Revenue Authority (KRA) **eTIMS** (electronic Tax Invoice Management System) adapter.

eTIMS is the KRA clearance regime that replaced the older Tax Invoice Management System (TIMS) hardware requirement: issuers transmit invoices over REST and receive a CU Invoice Number plus a KRA-issued signature. This crate is the typed transmit surface for that exchange, plus a deterministic mock. It carries the signed payload as an opaque blob — it does not build the eTIMS payload itself.

## Capabilities

- **Transmit surface** — the `EtimsProvider` trait with one method, `submit_invoice`, taking an `EtimsSubmitRequest` (tenant id, environment, issuer PIN, payload bytes) and returning an `EtimsSubmitEnvelope` (CU Invoice Number, KRA signature, status, recorded timestamp, optional rejection reason).
- **Local validation before the wire** — KRA PIN shape (`validate_pin`) and a non-empty payload check. Anything else is the regulator's verdict.
- **Verdict modeling** — a KRA `Rejected` decision is surfaced inside the returned envelope (`EtimsStatus::Rejected` with a reason), not as a transport error, so the engine can persist the rejection in its audit trail. Only pre-wire validation failures and transport failures are `EtimsError`.
- **Deterministic mock** — `MockEtimsProvider` issues serial CU numbers (`KE-000000000001`, …) and a fixed timestamp for tests and golden fixtures.

It does **not** serialize the eTIMS national format, validate the cleared document against KRA business rules, sign anything, or talk to KRA over the network.

## Coverage

**Opaque payload / bring-your-own.** The caller supplies `payload: Vec<u8>` — the canonical signed JSON the issuer intends to clear — and this crate passes it through. The crate does not construct, canonicalize, or sign the eTIMS payload; it owns only the request/response shapes and the pre-wire checks above.

There is no live integration here. The mock returns `Accepted` with a synthetic CU number and signature. As the module doc-comment states, the real KRA REST integration lands in a follow-up `report-ke-etims-http` crate; the typed surface in this crate is what that crate will implement.

Local validation is intentionally minimal: PIN shape (`A123456789Z` — leading letter, nine digits, trailing letter) and a non-empty payload. The PIN check is a shape check, not a registration check.

## References

Endpoints named in the source:

- KRA eTIMS sandbox — `etims-api-sbx.kra.go.ke`
- KRA eTIMS production — `etims-api.kra.go.ke`

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
