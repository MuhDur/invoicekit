# invoicekit-report-mx-cfdi — Mexico / SAT / CFDI 4.0 (Comprobante Fiscal Digital por Internet)

National-clearance report adapter for the Mexican CFDI. Serializes an InvoiceKit IR document to the real national `cfdi:Comprobante` 4.0 XML, validates the issuer's RFC and the SAT Folio Fiscal, and exercises the seal-and-stamp (timbrado) lifecycle offline through a deterministic mock provider.

Mexico is a national-clearance jurisdiction: the invoice is serialized to CFDI XML, the taxpayer seals it with their Certificado de Sello Digital (CSD), and a Proveedor Autorizado de Certificación (PAC) stamps it — the timbrado — adding the SAT Timbre Fiscal Digital (TFD): a UUID (Folio Fiscal) plus the SAT seal.

## Capabilities

- **Serialize (native CFDI)** — `to_cfdi_xml` turns a `invoicekit_ir::CommercialDocument` into deterministic CFDI 4.0 XML: `cfdi:Comprobante` with `cfdi:Emisor`, `cfdi:Receptor`, one `cfdi:Concepto` per line, and document-level `cfdi:Impuestos`/`cfdi:Traslados`. This is the real national format; the UBL and CII serializers do not emit it. Output is byte-stable by construction (fixed attribute/element order, no maps, amounts at fixed scale 2). The emitted XML is pre-timbrado — it carries no `Sello`/`Certificado` and no `TimbreFiscalDigital`. Comprobante-level SAT fields that are not part of the jurisdiction-agnostic IR (`LugarExpedicion`, `MetodoPago`, `FormaPago`, `RegimenFiscal`, `UsoCFDI`, `DomicilioFiscalReceptor`) are supplied through a separate `CfdiContext`.
- **Validate (local)** — `validate_rfc` enforces the Mexican RFC shape (12 characters for personas morales, 13 for personas físicas: name-prefix letters + 6 date digits + 3-character homoclave). `validate_folio_fiscal` enforces the SAT UUID shape (`8-4-4-4-12` hex). Reference-grade XSD plus cadena-original XSLT validation is an external (JVM) backend and is not performed here.
- **Sign + transmit (offline mock)** — `MockCfdiReportProvider` (implementing the `CfdiReportProvider` trait) composes the existing `invoicekit_signer_cfdi::MockCfdiPacProvider` so the PAC timbrado path (cadena original + selloCFDI + selloSAT + Folio Fiscal synthesis) is exercised, not re-implemented. It returns a typed `CfdiReportEnvelope` carrying `status`, `folio_fiscal`, `pac_certificate_serial`, `fecha_timbrado`, `sello_sat`, and the TFD seal receipt (`signature`); `CfdiReport` also returns the stamped `timbrado_xml` with the TFD complemento appended. `with_rejection` drives the rechazo path. Live PAC transmission is out of scope here (see Coverage).
- **Evidence** — the caller bundles the canonical document, CFDI XML, stamped artifact, and receipt into a signed evidence bundle. This crate produces the stamped CFDI bytes (`CfdiReport::timbrado_xml`) and the receipt; it does not assemble the bundle itself.

Rejection is not an error: a PAC rechazo (e.g. SAT validation code `CFDI40102`) is surfaced as an `Ok` `CfdiReportEnvelope` whose `status` is `TimbradoStatus::Rechazado`, never as `Err`. `Err` (`CfdiReportError`) is reserved for pre-wire shape failures (bad RFC, empty payload) and transport faults.

## Coverage

The native CFDI serializer emits the mandatory comprobante spine only — `Comprobante` / `Emisor` / `Receptor` / `Conceptos` / `Impuestos`. It is not the full CFDI 4.0 schema. Documented residuals and simplifications present in the source:

- **Live transmission** is not implemented. The bundled `Mock*` providers are deterministic and offline; live PAC web services (Solución Factible / Edicom / Facturando) are bring-your-own-credentials and land in a follow-up `report-mx-cfdi-http` crate.
- **Document types** — only `Invoice` (`TipoDeComprobante = I`, Ingreso) and `CreditNote` (`E`, Egreso) map. CFDI has no debit-note comprobante type; debit notes, pro-forma, and self-billed documents are rejected with `UnsupportedDocumentType`.
- **Tax** — only IVA traslados are emitted. Each `cfdi:Traslado` is fixed to `Impuesto="002"` (IVA), `TipoFactor="Tasa"`, with the rate from the IR tax summary. Retenciones (withholding), ISR, IEPS, exempt/`Exento` factors, and per-line tax categories beyond the matched summary entry are not emitted.
- **Fixed attributes** — `Exportacion` is fixed to `01` (no aplica) and per-concepto `ObjetoImp` to `02` (sí objeto de impuesto). `CfdiContext` defaults (`LugarExpedicion=00000`, `MetodoPago=PUE`, `FormaPago=99`, `RegimenFiscal=601`, `UsoCFDI=G03`) are placeholders the caller is expected to override.
- **`ClaveUnidad`** falls back to `H87` (pieza) when the line carries no `unit_code`.
- **Receptor RFC** falls back to the SAT generic-public RFC `XAXX010101000` when the customer carries no usable tax id; a leading `MX` country prefix is stripped from any RFC before use.
- **`Fecha`** is synthesized from the IR issue date with the time-of-day pinned to `T00:00:00` (no timezone) for byte-stability.

## New IR fields

The serializer reads one IR line classification: the SAT `ClaveProdServ` (c_ClaveProdServ product/service catalogue key, EN 16931 BT-158) is sourced from the first `DocumentLine` classification whose `scheme_id` is `ClaveProdServ` (matched case-insensitively). It is emitted on `cfdi:Concepto`; a line with no such classification falls back to the generic catch-all key `01010101`. Non-SAT classification schemes (e.g. HSN) are ignored for this attribute.

## References

- CFDI 4.0 XML namespace: `http://www.sat.gob.mx/cfd/4` (Servicio de Administración Tributaria), emitted as the `cfdi:Comprobante` root namespace with `Version="4.0"`.
- Timbre Fiscal Digital namespace: `http://www.sat.gob.mx/TimbreFiscalDigital` (TFD 1.1), emitted as the `tfd:TimbreFiscalDigital` complemento on the stamped artifact.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
