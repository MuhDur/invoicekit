# invoicekit-report-do-dgii — Dominican Republic (DGII e-CF)

Typed clearance-submission surface for the Dominican Republic's DGII e-CF (e-Comprobante Fiscal) system. Carries a pre-built signed e-CF XML payload to DGII as an opaque blob; it does not build or serialize the e-CF XML itself.

The Dirección General de Impuestos Internos (DGII) runs the Dominican Republic's e-invoicing clearance: issuers sign XML with a DGII-issued certificate, attach an e-NCF (Número de Comprobante Fiscal electrónico), and submit over REST. DGII returns `Aceptado` / `Rechazado` plus a TrackId for asynchronous reconciliation.

## Capabilities

- **Local shape validation** of the submission envelope before the wire:
  - issuer RNC (Registro Nacional del Contribuyente) — 9 or 11 ASCII digits, via `validate_rnc`;
  - e-NCF — `E` + 12 ASCII digits (2-digit type + 10-digit sequential), via `validate_e_ncf`;
  - non-empty `ecf_xml` payload.
- **Transmit surface** — the `DgiiProvider` trait with one method, `submit_ecf`, returning a `DgiiSubmitEnvelope` (TrackId, echoed e-NCF, status, received-at timestamp, optional DGII message). A DGII `Rechazado` verdict is returned inside the envelope, not as an error, so the engine persists the rejection in its audit trail.
- **Deterministic mock** — `MockDgiiProvider`, with fixed timestamps and serial TrackIds, for tests and dry runs.
- **e-CF document classes** — the `DgiiDocumentKind` enum maps the DGII catálogo codes (31, 32, 33, 34, 41, 43, 44, 45, 46, 47) via `DgiiDocumentKind::code`.
- **Environment selection** — `DgiiEnvironment::Sandbox` (TestECF) / `Produccion`.

The crate does **not** serialize e-CF XML, sign it, or compute the e-NCF. The signed XML is supplied by the caller in `DgiiSubmitRequest::ecf_xml` and treated as an opaque payload.

## Coverage

Opaque-payload (bring-your-own) model. This crate is the typed surface plus a deterministic `MockDgiiProvider`. It validates the RNC and e-NCF shapes and rejects an empty payload, then hands the caller's signed XML to a provider; the e-CF XML structure itself is never inspected beyond emptiness.

Documented residuals:

- **No live transport.** Only `MockDgiiProvider` ships here; the real DGII REST integration lands in a follow-up `report-do-dgii-http` crate (stated in the module doc-comment and `Cargo.toml` description).
- **e-NCF shape only.** `validate_e_ncf` checks the 13-character `E` + 12-digit form; it does not check that the 2-digit type segment matches the `DgiiDocumentKind` of the request, nor any sequence-authorization rules.
- **RNC shape only.** `validate_rnc` checks digit count and ASCII digits; it does not verify the RNC against any registry or compute a check digit.

## References

The module doc-comment cites the DGII transport endpoints:

- DGII e-CF sandbox — `ecf.dgii.gov.do/testecf` (TestECF).
- DGII e-CF production — `ecf.dgii.gov.do/ecf`.

No external specification documents are cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
