<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-ro-efactura — Romania / ANAF / RO e-Factura clearance

Typed adapter surface for Romania's RO e-Factura clearance regime, operated by ANAF (Agenția Națională de Administrare Fiscală). Models the upload → validate → clear flow as a Rust trait plus a deterministic mock.

## Capabilities

- **Typed upload request/response surface.** `EFacturaUploadRequest` carries the tenant, environment (`Sandbox` / `Production`), document kind (`Invoice` / `CreditNote` / `SelfBilling`), issuer and optional buyer CUI, and the invoice payload. `EFacturaUploadEnvelope` returns ANAF's `indice_incarcare` (upload index), the `EFacturaStatus` verdict, the recorded RFC-3339 UTC timestamp, and optional `motivare` text when rejected.
- **Local shape validation.** `validate_cui` enforces 2–10 ASCII digits, optionally `RO`-prefixed, surfaced as `EFacturaError::BadCui`. Issuer CUI is always checked; buyer CUI is checked when supplied. An empty payload is rejected as `EFacturaError::BadXml`.
- **Status polling.** `EFacturaProvider::poll_status` retrieves the latest verdict for a previously uploaded `indice_incarcare`.
- **Deterministic mock provider.** `MockEFacturaProvider` implements the `EFacturaProvider` trait with a fixed timestamp and incrementing serial indices, so the upload → poll → cleared lifecycle is executable and reproducible in tests without reaching ANAF.

The crate does **not** serialize the national payload, sign anything, or talk to ANAF. The `invoice_xml` field is an **opaque blob** supplied by the caller — the crate validates the envelope shape around it (CUI plus a non-empty check), not its contents. Per the module doc-comment, the live ANAF REST integration lands in a follow-up `report-ro-efactura-http` crate behind a feature flag.

## Coverage

This is a **clearance adapter**, not a national-format serializer. ANAF expects invoices as UBL 2.1 + the Romanian CIUS (Core Invoice Usage Specification), and this crate's `EFacturaUploadRequest::invoice_xml` is documented as that canonical XML — but the bytes are produced upstream and are opaque to this crate.

Documented residuals and boundaries, mirroring the module doc-comment:

- **No native payload serialization.** The UBL 2.1 + RO CIUS XML is opaque to this crate; the caller produces it (the shared UBL/profile crates upstream do that work).
- **No live transport.** No HTTP, TLS, or DNS to `api.anaf.ro`. Only `MockEFacturaProvider` ships; the real transport is the deferred `report-ro-efactura-http` crate.
- **Validation is shape-only.** The CUI is checked for digit length (2–10) and ASCII-digit composition with an optional `RO` prefix; there is no check-digit, registry, or business-rule validation.
- **ANAF verdicts are data, not errors.** A `Rejected` outcome (with its `motivare` text) is returned inside `EFacturaUploadEnvelope` so the engine can persist it alongside the audit trail. `EFacturaError` is reserved for local validation failures (`BadXml`, `BadCui`) and transport faults (`Transport`).
- **Cleared mesaj XML is not downloaded.** The module documents that ANAF follows a cleared invoice with a signed `mesaj` XML containing its countersignature; this crate models the `Cleared` status but does not fetch or verify that signed message.

## References

Authorities and identifiers named in the source:

- ANAF — Agenția Națională de Administrare Fiscală, operator of the RO e-Factura clearance portal.
- RO e-Factura — the Romanian B2B / B2G clearance regime; payloads are UBL 2.1 + the Romanian CIUS.
- `api.anaf.ro` — clearance host (`/test` sandbox tier, `/prod` production).
- CUI — Codul Unic de Înregistrare (issuer/buyer tax identifier).
- Indice de încărcare — the ANAF-assigned upload index returned per invoice.
- Mesaj — the signed XML ANAF returns for a cleared invoice, carrying its countersignature.

(All references above are those named in the crate's module documentation; no external specification URLs are cited in the source.)

## License

Apache-2.0.
