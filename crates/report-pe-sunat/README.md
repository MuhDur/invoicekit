<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-pe-sunat

Peru — SUNAT (Superintendencia Nacional de Aduanas y de Administración Tributaria) — SEE (Sistema de Emisión Electrónica) clearance adapter.

Peru runs a clearance regime: a B2B issuer signs a typed UBL 2.1 invoice, submits it to SUNAT over SOAP, and receives a CDR (Constancia de Recepción) ZIP carrying a per-invoice `responseCode`. This crate is the typed adapter surface for that submission step; it takes the already-signed UBL XML as opaque bytes and exchanges it for a CDR envelope.

## Capabilities

- **Local shape validation.** `validate_ruc` checks the issuer RUC (Registro Único de Contribuyentes) — exactly 11 ASCII digits. `validate_document_id` checks the series + correlative shape `SSSS-NNNNNNNN` (4-char alphanumeric series, dash, 1–8 digit correlative). Both run before anything reaches the wire.
- **Submit / verdict modelling.** The `SunatProvider` trait defines `submit_document`, which validates the RUC and document id, accepts the signed UBL XML, and returns a `SunatSubmitEnvelope` (CDR `response_code`, a typed `SunatStatus`, the SUNAT-recorded timestamp, and the optional CDR `description`). A SUNAT refusal is surfaced **inside** the envelope as `SunatStatus::Rechazado`, not as an `Err`, so the engine can persist the rejection alongside its audit trail. `SunatError` is reserved for local-validation failures (`BadXml`, `BadRuc`, `BadDocumentId`) and transport faults (`Transport`).
- **Document taxonomy.** `SunatDocumentKind` maps to SUNAT catálogo 06 codes via `code()`: `01` Factura, `03` Boleta de Venta, `07` Nota de Crédito, `08` Nota de Débito, `09` Guía de Remisión Remitente.
- **Deterministic mock.** `MockSunatProvider` implements the trait with a fixed timestamp and monotonic serials, returning an `Aceptado` envelope with `response_code` `0` for valid requests. It is the only provider implementation shipped here.

The crate does **not** serialize the UBL invoice, sign it, build or parse the CDR ZIP, or talk to SUNAT over the wire. The signed UBL 2.1 XML is supplied by the caller as `SunatSubmitRequest::invoice_xml: Vec<u8>`.

## Coverage

Opaque-payload (bring-your-own) adapter. The invoice XML crosses the boundary as bytes; this crate neither parses nor produces the UBL 2.1 schema, and it does not layer on the EN 16931 / UBL family path. The signed XML is an input computed elsewhere.

Documented residuals, mirroring the module doc-comment:

- **No live transport.** Only `MockSunatProvider` ships. The live SUNAT SOAP integration is deferred to a follow-up `report-pe-sunat-http` crate behind a feature flag (stated in the module doc-comment and `Cargo.toml` description).
- **Shape-only validation.** `validate_ruc` checks digit count and ASCII-digit composition; `validate_document_id` checks the `SSSS-NNNNNNNN` shape. Neither verifies the RUC check digit nor validates the XML against a SUNAT XSD. `SunatError::BadXml` is raised only for an empty payload in the mock.
- **`SunatEnvironment` is a selector only.** `Beta` and `Produccion` name the SUNAT hosts (`e-beta.sunat.gob.pe` / `e-factura.sunat.gob.pe`) but no transport in this crate connects to either.
- **`SunatStatus::SinCdr`** models a transport error / timeout where SUNAT returned no CDR (the engine retries); the mock never returns it.

## References

Authorities and endpoints named in the source:

- SUNAT — Superintendencia Nacional de Aduanas y de Administración Tributaria; SEE (Sistema de Emisión Electrónica) clearance regime.
- SUNAT catálogo 06 — the document-class taxonomy (`01`/`03`/`07`/`08`/`09`) mapped by `SunatDocumentKind::code`.
- CDR — Constancia de Recepción; the `responseCode` bands (`0` accepted, `2000–3999` rejected, `4000–4999` warnings) mapped to `SunatStatus`.
- Environment hosts: `e-beta.sunat.gob.pe` (beta / sandbox), `e-factura.sunat.gob.pe` (producción).

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
