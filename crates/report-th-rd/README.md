<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-th-rd

Thailand — Revenue Department (กรมสรรพากร) e-Tax Invoice & e-Receipt clearance adapter.

This crate is the typed submission surface for the Revenue Department's e-Tax Invoice & e-Receipt regime: it carries an already-signed payload to the authority, validates the issuer tax id shape, and surfaces the RD acknowledgement (reference, status, timestamp). It does **not** build, sign, or serialize the payload itself.

## Capabilities

- **Typed transport surface.** `RdProvider::submit_invoice` takes an `RdSubmitRequest` (tenant, environment, flavour, issuer tax id, and the signed `payload` bytes) and returns an `RdSubmitEnvelope` (RD-assigned reference, status, RFC-3339 timestamp, and the rejection `reason` when present).
- **Pre-wire shape validation.** `validate_tax_id` requires exactly 13 ASCII digits and rejects anything else before the payload reaches the wire. `submit_invoice` also rejects an empty payload. These are shape checks only — not check-digit or registry validation.
- **Flavour and environment selectors.** `RdFlavour` distinguishes the full e-Tax Invoice (signed XML over SOAP) from e-Tax Invoice by Email (signed PDF/A-3 over SMTP). `RdEnvironment` names the RD UAT sandbox (`etax-uat.rd.go.th`) and production (`etax.rd.go.th`). Both are carried on the request; no transport in this crate connects to either.
- **Verdict modelling.** An authority refusal is an `RdStatus::Rejected` carried inside the `Ok` envelope, never an `Err`, so the engine persists the rejection alongside its audit trail. `RdError` is reserved for local shape failures and transport failures.
- **Deterministic mock.** `MockRdProvider` runs the shape validation and returns a fixed-timestamp envelope with a monotonic `TH-…` reference; `with_forced_rejection` drives the `Rejected` branch so callers can exercise the verdict path without a live backend.

The crate does **not** serialize the e-Tax Invoice XML or the PDF/A-3, does **not** sign with the RD-registered digital certificate, and does **not** perform live transport. There is no native format serializer here.

## Coverage

**Opaque payload (bring-your-own signed payload).** The signed payload crosses the boundary as `RdSubmitRequest::payload: Vec<u8>` — canonical, already-signed bytes (XML for the e-Tax Invoice flavour, PDF/A-3 for the by-Email flavour) produced upstream and forwarded as an opaque blob. The only inspection this crate performs on it is a non-empty check (empty payloads are rejected with `RdError::BadPayload`).

Documented residuals:

- **No live transport.** `MockRdProvider` is the only provider that ships. The live RD REST integration lands in a follow-up `report-th-rd-http` crate (stated in the module doc-comment and `Cargo.toml` description).
- **Shape-only tax-id check.** The issuer tax id is validated for length (13) and ASCII-digit content only, not for check digit or registry existence.
- **No EN 16931 / UBL path.** This crate does not route through the European format family. (`invoicekit-format-ubl`, `invoicekit-ir`, and friends are dev-dependencies used by tests only.)

## References

Only sources named in the source are listed.

- Thai Revenue Department (กรมสรรพากร) — e-Tax Invoice & e-Receipt regime.
- RD UAT sandbox: `etax-uat.rd.go.th`
- RD production: `etax.rd.go.th`

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
