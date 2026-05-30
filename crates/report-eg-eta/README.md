<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-eg-eta — Egypt (Egyptian Tax Authority, ETA e-Invoicing / e-Receipt)

Typed submission adapter for Egypt's Egyptian Tax Authority (ETA) e-Invoicing and e-Receipt clearance. It carries an already-signed ETA JSON payload as an opaque blob to the clearance gateway and surfaces the per-document verdict; it does not build or sign that payload itself.

## Capabilities

- **Transmit (typed surface only).** Defines `EtaProvider::submit`, which takes an `EtaSubmitRequest` (tenant id, environment, document kind, issuer tax/national id, and a `payload: Vec<u8>`) and returns an `EtaSubmitEnvelope` (ETA UUID, Long ID, SHA-256 content-hash hex, status, submitted-at timestamp, and an optional rejection reason).
- **Local validation.** `validate_tax_or_national_id` enforces the issuer identifier shape (9 ASCII digits for a tax registration number, 14 for a business-to-consumer national id); `submit` additionally rejects an empty payload. Both fail before the wire as `EtaError::BadId` / `EtaError::BadPayload`.
- **Deterministic mock.** `MockEtaProvider` returns fixed timestamps and incrementing serials, with `with_forced_verdict` to drive the `Valid` and `Invalid` clearance paths offline.

What this crate does **not** do: it does not serialize the ETA document format, does not compute the canonical JSON, does not sign, and does not perform the EN 16931 / UBL path. The `payload` field is a pre-built, pre-signed blob supplied by the caller.

## Coverage

Opaque-payload adapter. This is the **bring-your-own-payload** model: the caller produces the canonical signed ETA JSON; this crate validates the request shape and defines the submission contract.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockEtaProvider` ships. The live ETA REST integration against `api.invoicing.eta.gov.eg` (production) / `api.preprod.invoicing.eta.gov.eg` (preprod) lands in a follow-up `report-eg-eta-http` crate.
- **ETA-side verdict is a status, not an error.** ETA runs its server-side document validators after submission and returns a per-document verdict. A `Valid` or `Invalid` verdict arrives after the wire, so `submit` surfaces it through `EtaStatus` inside the returned envelope (with `reason` on `Invalid`) rather than as an `Err`. Only pre-wire shape failures and transport failures are `EtaError`.
- **Document kinds** modelled: invoice (`I`), credit note (`C`), debit note (`D`), and e-receipt (`R`, business-to-consumer).

## References

- ETA SDK — document validation rules: <https://sdk.invoicing.eta.gov.eg/document-validation-rules/>

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
