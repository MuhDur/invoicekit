# invoicekit-report-ar-afip — Argentina / AFIP / CAE clearance

Typed adapter surface for Argentina's Administración Federal de Ingresos Públicos (AFIP) CAE (Código de Autorización Electrónico) clearance regime, across the WSFE, WSFEX, and WSMTXCA web services.

## Capabilities

- **Typed request/response surface.** `AfipCaeRequest` carries the tenant, environment (`Homologacion` / `Produccion`), target service (`Wsfe` / `Wsfex` / `Wsmtxca`), letter class (`A`/`B`/`C`/`E`/`M`), issuer CUIT, punto de venta, and a request payload. `AfipCaeEnvelope` returns the 14-digit CAE, its `YYYYMMDD` expiry, the `AfipStatus` verdict, an authorization timestamp, and optional observaciones.
- **Local shape validation.** `validate_cuit` enforces exactly 11 ASCII digits; `validate_punto_venta` enforces exactly 5 ASCII digits. Both run before any transport, surfaced as `AfipError::BadCuit` / `AfipError::BadPuntoVenta`.
- **Deterministic mock provider.** `MockAfipProvider` implements the `AfipProvider` trait with fixed timestamps and incrementing serials, so the request → validate → CAE-envelope contract is executable and reproducible in tests today.

The crate does **not** serialize the national payload, sign anything, or talk to AFIP. The `request_payload` field is an **opaque blob** (XML or JSON, depending on the service) supplied by the caller — this crate validates the envelope shape around it, not its contents. Per the module doc-comment, the live AFIP SOAP integration lands in a follow-up `report-ar-afip-http` crate behind a feature flag.

## Coverage

This is a **clearance adapter**, not a national-format serializer. It models AFIP's CAE flow — request a CAE per invoice from WSFE (domestic, sin detalle), WSFEX (exportación), or WSMTXCA (con detalle de items), then carry the granted CAE and its expiry onto the printed invoice — as a typed Rust surface plus a deterministic mock.

Documented residuals and boundaries, mirroring the module doc-comment:

- **No native payload serialization.** The WSFE/WSFEX/WSMTXCA request body is opaque to this crate; the caller produces it.
- **No live transport.** No SOAP, HTTP, TLS, or DNS to AFIP. Only `MockAfipProvider` ships; the real transport is the deferred `report-ar-afip-http` crate.
- **Validation is shape-only.** CUIT and punto de venta are checked for digit length and ASCII-digit composition; there is no check-digit, registry, or business-rule validation.
- **AFIP verdicts are data, not errors.** A `Rechazado` or `AprobadoConObservaciones` outcome is returned inside `AfipCaeEnvelope` (so the engine can persist it in the audit trail), not raised as `AfipError`. `AfipError` is reserved for local validation failures and transport faults.
- **Letter class is recorded, not enforced.** `AfipLetter` distinguishes the issuer regime (responsable inscripto / monotributista / exento / exportación) that drives IVA discrimination on the printed invoice; this crate carries the letter, it does not derive or validate IVA discrimination.

## References

Authorities and services named in the source:

- AFIP — Administración Federal de Ingresos Públicos.
- CAE — Código de Autorización Electrónico (per-invoice clearance code + expiry).
- WSFE — Factura Electrónica (sin detalle de items).
- WSFEX — Facturación de Exportación.
- WSMTXCA — Factura con detalle de items.
- Environment hosts: `wswhomo.afip.gov.ar` (homologación / sandbox), `servicios1.afip.gov.ar` (producción).

## License

Apache-2.0.
