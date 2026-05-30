<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-ec-sri

Ecuador — Servicio de Rentas Internas (SRI) clearance adapter for the SRI comprobante electrónico regime.

Ecuador runs a clearance model: an issuer signs comprobante XML with a Banco Central de Ecuador (BCE) or Security Data certificate, computes a 49-digit Clave de Acceso, and submits to SRI over SOAP, which returns an autorización envelope carrying a `numeroAutorizacion` and `fechaAutorizacion`. This crate is the typed adapter surface for that submission step.

## Capabilities

- **Local shape validation.** `validate_ruc` checks the issuer RUC (exactly 13 ASCII digits); `validate_clave_acceso` checks the Clave de Acceso (exactly 49 ASCII digits). Both run before anything reaches the wire.
- **Submit/verdict modelling.** `SriProvider::submit_comprobante` takes an `SriSubmitRequest` and returns an `SriSubmitEnvelope` with a typed `SriStatus` (`Recibido`, `Autorizado`, `Devuelto`, `NoAutorizado`). SRI refusals (`Devuelto` / `NoAutorizado`) are surfaced inside the envelope, not as `Err`s, so the engine can persist a rejection alongside its audit trail. Only local-validation and transport failures return `SriError`.
- **Document taxonomy.** `SriDocumentKind` maps to SRI's `tipoComprobante` codes: `01` Factura, `03` Liquidación de Compra, `04` Nota de Crédito, `05` Nota de Débito, `06` Guía de Remisión, `07` Comprobante de Retención.
- **Deterministic mock.** `MockSriProvider` validates the request and returns a fixed-timestamp `Autorizado` envelope for tests.

It does **not** serialize the comprobante XML, compute the Clave de Acceso, sign, or render. Those happen upstream in the engine.

## Coverage

Opaque-payload (bring-your-own) adapter. The crate carries the signed comprobante XML through as an opaque `Vec<u8>` (`SriSubmitRequest::comprobante_xml`); it does not generate or parse the SRI XML schema, and it does not layer on the EN 16931 / UBL family. The 49-digit Clave de Acceso and the signed XML are inputs computed elsewhere.

Documented residuals:

- **No live transport.** The SOAP submission against `celcer.sri.gob.ec` (certificación) and `cel.sri.gob.ec` (producción) is modelled by `MockSriProvider` only. The real `SriProvider` transport implementation lands in a follow-up `report-ec-sri-http` crate behind a feature flag.
- **Shape-only validation.** `validate_ruc` and `validate_clave_acceso` check digit count and ASCII-digit composition. They do not verify the RUC check digit or decode the Clave de Acceso structure.
- **No XML schema validation.** `SriError::BadXml` is raised only for an empty payload in the mock; the crate does not validate the comprobante against SRI's XSD.

## References

- Servicio de Rentas Internas (SRI) — Ecuador's tax authority (the `comprobante electrónico` / 49-digit Clave de Acceso scheme).
- SRI clearance endpoints: `celcer.sri.gob.ec` (certificación), `cel.sri.gob.ec` (producción)

## License

Apache-2.0.
