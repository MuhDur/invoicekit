<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-co-dian

Colombia — DIAN (Dirección de Impuestos y Aduanas Nacionales) e-invoicing clearance adapter.

This crate is the typed transport surface for DIAN's clearance model: it takes an already-signed UBL 2.1 + DIAN CIUS payload as opaque bytes, validates the issuer/buyer NIT shape, and exchanges it for a DIAN envelope (CUFE, track id, status). It does **not** build, sign, or serialize the invoice XML itself.

## Capabilities

- **Local validation.** `validate_nit` checks the Colombian NIT (Número de Identificación Tributaria) shape — 9–11 ASCII digits, optionally hyphenated with a check digit — before anything goes on the wire. `submit_invoice` runs this on the issuer NIT and, when present, the buyer NIT, and rejects an empty payload.
- **Submit / poll surface.** The `DianProvider` trait defines `submit_invoice` (returns a `DianSubmitEnvelope` with the CUFE, DIAN track id, and `DianStatus`) and `query_track_id` (poll an earlier submission). A DIAN `Rechazado` verdict is returned **inside** the envelope as `DianStatus::Rechazado`, not as an `Err`, so the engine can persist the rejection in its audit trail.
- **Deterministic mock.** `MockDianProvider` implements the trait with fixed timestamps and monotonic serials for tests and golden artifacts. It synthesizes a 96-character CUFE and a `DIAN-…` track id; it does not talk to DIAN.

The crate does **not** serialize the national format, sign the payload, compute a real CUFE, or transmit over the live wire. The signed UBL 2.1 + DIAN CIUS XML is supplied by the caller as `DianSubmitRequest::invoice_xml: Vec<u8>`.

## Coverage

Opaque-payload (bring-your-own) adapter. The invoice XML crosses the boundary as bytes; this crate neither parses nor produces it.

Documented residuals:

- **No live transport.** Only `MockDianProvider` ships. The real DIAN SOAP integration is deferred to a follow-up `report-co-dian-http` crate behind a feature flag (stated in the module doc-comment and `Cargo.toml` description).
- **Mock CUFE is not real.** The 96-character CUFE from `MockDianProvider` is synthesized from the serial and the first bytes of the payload. It is **not** a SHA-384 over the invoice's canonical fields, which is what a production CUFE requires.
- **`DianDocumentKind` is a typed taxonomy only.** It enumerates DIAN's `tipo de operación` classes — factura de venta, factura de exportación, nota crédito, nota débito, documento soporte, nómina electrónica — but the crate applies no class-specific logic; the discriminator is carried on the request and echoed through.
- **`DianEnvironment` is a selector only.** `Habilitacion` and `Produccion` name the DIAN endpoints (`vpfe-hab.dian.gov.co` / `vpfe.dian.gov.co`) but no transport in this crate connects to either.

## References

Only sources named in the source are listed.

- DIAN habilitación endpoint: `vpfe-hab.dian.gov.co`
- DIAN production endpoint: `vpfe.dian.gov.co`
- CUFE — Código Único de Facturación Electrónica (96-char SHA-384 over the invoice's canonical fields, per the module doc-comment).

## License

Apache-2.0.
