<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-ng-firs — Nigeria (Federal Inland Revenue Service, FIRS e-Invoicing)

Typed submission surface for Nigeria's Federal Inland Revenue Service (FIRS) e-Invoicing clearance regime. The crate carries an already-built, already-signed FIRS JSON payload to the clearance gateway and surfaces the per-invoice verdict; it does not build or sign that payload itself.

FIRS runs a clearance regime: issuers submit a typed JSON envelope to the FIRS portal and FIRS returns an Invoice Reference Number (IRN) and an acceptance status. This crate is the typed Rust surface for that request and response.

## Capabilities

- **Transmit (typed surface only).** `FirsProvider::submit_invoice` takes a `FirsSubmitRequest` (tenant id, environment, issuer TIN, and a `payload: Vec<u8>`) and returns a `FirsSubmitEnvelope` (FIRS-issued IRN, status, RFC 3339 UTC recorded-at timestamp, and an optional rejection reason).
- **Local validation** before the wire. `validate_tin` checks the issuer TIN against the 12-ASCII-digit shape (hyphens stripped); `submit_invoice` additionally rejects an empty payload. Both fail as `FirsError::BadTin` / `FirsError::BadPayload`.
- **Status surfacing.** A FIRS `Rejected` verdict is not an `Err`; it is returned inside the envelope as `FirsStatus::Rejected` (with `reason`) so the engine can persist the refusal alongside its audit trail. `FirsError` is reserved for pre-wire validation failures and `Transport` failures.
- **Deterministic mock.** `MockFirsProvider` returns an `Accepted` envelope with a serial IRN (`NG-…`) and a fixed timestamp, for tests and pipeline wiring; `with_fixed_recorded_at` sets a custom timestamp.

It does **not** serialize a national format, does **not** compute the canonical payload, does **not** sign, and does **not** run the EN 16931 / Universal Business Language family path. It does not produce evidence bundles.

## Coverage

Opaque-payload / bring-your-own model. The `payload` field is a pre-built, pre-signed byte blob supplied by the caller; this crate validates the request shape and defines the submission contract, nothing more.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockFirsProvider` ships. The live FIRS REST integration lands in a follow-up `report-ng-firs-http` crate. `FirsError::Transport` exists for that wire; no HTTP/TLS/DNS code is present in this crate.
- **TIN check is shape-only** — 12 ASCII digits with an optional hyphen after the eighth digit — not a checksum or registry lookup.
- **IRNs are issued by FIRS**, not allocated here; the mock's serial `NG-…` IRN is a stand-in, not a FIRS-conformant identifier.
- **`FirsEnvironment`** selects `Sandbox` or `Production`; the mock ignores the distinction.

## References

Sourced from the module doc-comment. The Federal Inland Revenue Service (FIRS) Nigeria e-Invoicing clearance regime and its IRN-returning portal. No specification or regulator URLs are cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
