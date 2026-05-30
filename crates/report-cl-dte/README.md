<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-cl-dte

Chile — Servicio de Impuestos Internos (SII) — DTE (Documento Tributario Electrónico) reporting adapter.

Serializes an InvoiceKit IR document to Chile's real national DTE XML and exposes a typed SII submission surface. The live SII SOAP integration is not in this crate; it lands in a follow-up `report-cl-dte-http` crate behind a feature flag.

## Capabilities

- **Serialize the real national format.** `to_dte_xml` emits Chile's actual SII `DTE`/`Documento` tree as defined by the SII "Formato de Documentos Tributarios Electrónicos", with its real Spanish element names (`Encabezado`, `IdDoc`, `TipoDTE`, `Folio`, `FchEmis`, `Emisor`/`RUTEmisor`/`RznSoc`/`GiroEmis`, `Receptor`/`RUTRecep`/`RznSocRecep`, `Totales`/`MntNeto`/`TasaIVA`/`IVA`/`MntTotal`, and per-line `Detalle` with `NroLinDet`, `CdgItem`, `NmbItem`, `QtyItem`, `PrcItem`, `MontoItem`). This is not UBL or CII relabeled. Output is byte-stable: fixed element order, no maps, no timestamps.
- **Document-type mapping.** IR `DocumentType` maps to the SII `TipoDTE` code: Invoice → 33 (Factura Electrónica), CreditNote → 61 (Nota de Crédito), DebitNote → 56 (Nota de Débito). ProForma and SelfBilled have no mapping and return an error.
- **Typed submission surface.** The `SiiProvider` trait models `submit_dte` (validate issuer RUT shape and folio, POST the signed DTE XML, return a TrackId envelope) and `query_track_id`. Verdicts are surfaced as `SiiStatus` (`Recibido`, `Aceptado`, `AceptadoConReparos`, `Rechazado`); a SII `Rechazado` is a receipt status carried in the envelope, not a transport error.
- **Local RUT shape validation.** `validate_rut` checks the `NNNNNNNN-X` shape (1-8 digits, dash, digit or `K`). It does not compute or verify the modulo-11 check digit.

What this crate does **not** do: it does not sign DTE XML, does not consume folios from a CAF bundle, and does not talk to the SII over the wire. `SiiSubmitRequest` accepts an already-signed `dte_xml` payload and a folio the caller supplies. The only provider implementation shipped here is `MockSiiProvider`, a deterministic offline mock (fixed timestamps, serial TrackIds, optional forced verdict for exercising the `Rechazado` / `AceptadoConReparos` branches).

## Coverage

Native serialization of the SII DTE tree covers the document header (`IdDoc`, `Emisor`, `Receptor`), the `Totales` summary, and per-line `Detalle`. `TasaIVA` carries the single IVA percentage in effect (Chile's standard IVA is a flat 19 %; a fully exempt document renders `TasaIVA` 0). Monetary fields (`MntNeto`, `IVA`, `MntTotal`, `MontoItem`) render as integer Chilean pesos, matching the SII format, which types these as integers (CLP has no minor unit).

Encoding: the serializer returns a UTF-8 string with a UTF-8 XML declaration. Production SII submission re-encodes to ISO-8859-1 at the wire; that transcoding is deferred to the follow-up `report-cl-dte-http` crate.

Documented residuals — national elements deliberately **not** emitted, pinned by tests so a future change cannot silently emit a wrong element:

- **`Referencia` is not emitted.** The SII `Referencia` block makes `TpoDocRef` (the SII tipo of the *referenced* document) mandatory, and the IR `DocumentReference` carries no field telling us that tipo. A preceding-invoice reference is therefore skipped rather than emitted with an invented or missing `TpoDocRef`.
- **Tax-summary exemption reason / code are not emitted.** Chile signals exemption structurally (line-level `IndExe`, document-level `MntExe`), not via a free-text reason or a CEF `VATEX` / IT `Natura` code. No SII DTE element carries the IR's `exemption_reason` / `exemption_reason_code` verbatim, so these fields are dropped.
- **`scheme_version` is not emitted.** An IR classification's `scheme_version` has no home in the SII `Detalle`, so it is not serialized.

The `DteKind` enum enumerates eight common SII tipo codes (33, 34, 39, 41, 46, 52, 56, 61), but `to_dte_xml` only maps the three document types the IR exposes (33, 56, 61).

## New IR fields

This crate emits **line classifications** from the IR. Each `ItemClassification` on a line becomes one SII `Detalle/CdgItem` block, positioned after `NroLinDet` and before `NmbItem`, with `TpoCodigo` set from the IR `scheme_id` and `VlrCodigo` from the IR `code` — both copied verbatim. InvoiceKit does not derive, translate, or map any national code; `scheme_version` is not emitted, and a line with no classifications emits no `CdgItem`.

This crate does **not** emit document references or VAT exemption reason/code from the IR (see Coverage residuals above).

## References

- SII, "Formato de Documentos Tributarios Electrónicos" — the `DTE`/`Documento` schema, element names, and `TipoDTE` taxonomy serialized by this crate (`http://www.sii.cl/SiiDte` namespace, version 1.0).
- SII certification host `maullin.sii.cl` and production host `palena.sii.cl`, named by the `SiiEnvironment` selector.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
