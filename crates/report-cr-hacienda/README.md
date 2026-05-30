# invoicekit-report-cr-hacienda

Costa Rica — Ministerio de Hacienda ATV (Administración Tributaria Virtual) clearance adapter for electronic comprobantes submitted through `api.comprobanteselectronicos.go.cr`.

This crate is the typed submission surface for Hacienda's clearance regime: it carries an already-signed comprobante XML payload to the authority and surfaces the `recibido` / `aceptado` / `rechazado` verdict. It does **not** build the comprobante XML, compute the Clave Numérica, or sign — those are produced upstream and handed to this crate as inputs.

## Capabilities

- **Typed transport surface.** `HaciendaProvider::submit_comprobante` takes a `HaciendaSubmitRequest` (tenant, environment, document kind, issuer cédula, Clave Numérica, consecutivo, and the signed `comprobante_xml` bytes) and returns a `HaciendaSubmitEnvelope` (echoed Clave Numérica, status, Hacienda timestamp, and the rejection `mensaje`).
- **Pre-wire shape validation.** `validate_cedula` (9–12 ASCII digits), `validate_clave_numerica` (exactly 50 ASCII digits), and `validate_consecutivo` (exactly 20 ASCII digits) reject malformed identifiers before anything reaches the wire. These are length-and-digit shape checks only — not check-digit or registry validation.
- **Document-kind taxonomy.** `HaciendaDocumentKind` maps to Hacienda's `tipoDocumento` codes: 01 Factura Electrónica, 02 Nota de Débito, 03 Nota de Crédito, 04 Tiquete Electrónico (B2C), 08 Factura Electrónica de Compra, 09 Factura Electrónica de Exportación. `code()` returns the two-digit string.
- **Verdict modelling.** An authority refusal is a `HaciendaStatus::Rechazado` carried inside the `Ok` envelope, never an `Err`, so the engine persists the rejection alongside its audit trail. `HaciendaError` is reserved for local shape failures and transport failures.
- **Deterministic mock.** `MockHaciendaProvider` runs the shape validation and returns a fixed-timestamp envelope; `with_forced_status` drives the `Rechazado` / `Recibido` branches so callers can exercise the verdict paths without a live backend.

This crate does **not** serialize the comprobante XML, does **not** compute or check the Clave Numérica beyond its digit shape, does **not** sign with the Banco Central CR (BCCR) certificate, and does **not** perform live HTTP transport. There is no native format serializer here.

## Coverage

**Opaque payload (bring-your-own signed XML).** The signed comprobante is passed in as `comprobante_xml: Vec<u8>` — canonical, already-signed bytes produced by the engine — and forwarded as an opaque blob. The only inspection this crate performs on it is a non-empty check (empty payloads are rejected with `HaciendaError::BadXml`).

Documented residuals:

- **No live transport.** `MockHaciendaProvider` is the only provider that ships. The live ATV REST integration against `api.comprobanteselectronicos.go.cr/recepcion[-sandbox]` lands in a follow-up `report-cr-hacienda-http` crate.
- **Shape-only identifier checks.** Cédula, Clave Numérica, and consecutivo are validated for length and ASCII-digit content, not for check digits, the internal Clave field structure, or registry existence.
- **No EN 16931 / UBL path.** This crate does not route through the European format family. (`invoicekit-format-ubl` and friends are dev-dependencies used by tests only.)

## References

- Ministerio de Hacienda — Comprobantes Electrónicos / ATV: `api.comprobanteselectronicos.go.cr`
- `tipoDocumento` taxonomy and the `MensajeHacienda` response document, as cited in the module documentation.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
