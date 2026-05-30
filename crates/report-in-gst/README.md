<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-in-gst

India — Goods and Services Tax Network / National Informatics Centre (NIC) Invoice Registration Portal (IRP), `INV-01` e-invoice JSON.

Serializes the InvoiceKit commercial document to the real `INV-01` e-invoice JSON the NIC Invoice Registration Portal validates, and defines the `IrpProvider` integration surface for submitting that JSON to an IRP. The live IRP REST transport is not in this crate.

## Capabilities

- **Serialize the national format.** `to_inv01_json` emits deterministic `INV-01` e-invoice JSON (schema version `1.1`) — the JSON the NIC IRP actually accepts, with the schema's abbreviated PascalCase keys (`Version`, `TranDtls`, `DocDtls`, `SellerDtls`, `BuyerDtls`, `ItemList`, `ValDtls`), not a UBL/EN 16931 re-skin. Output is byte-stable: fixed key order, amounts at fixed scale 2, quantities at scale 3, no timestamps.
- **GST tax split.** Intra-state supplies (supplier and buyer share the leading two-digit GSTIN state code) split each line's tax into `CgstAmt` + `SgstAmt` at half the headline rate; inter-state supplies and exports (no buyer GSTIN) charge the full rate as `IgstAmt`. Overflow on the per-line multiply and the document-level accumulators surfaces as a typed error, not a panic.
- **Shape validation helpers.** `validate_gstin` checks the 15-character ASCII-alphanumeric GSTIN shape; `validate_hsn_sac` checks the 4–8 ASCII-digit HSN/SAC shape. These are pre-wire shape checks only — not the IRP modulo checksum.
- **IRP integration surface.** The `IrpProvider` trait captures the IRP `register_invoice` request/response wire shape (`IrpRegisterRequest` / `IrpRegisterEnvelope`, carrying `IrpStatus`, the 64-char IRN, ack number, signed QR base-64 PNG, and signed-invoice JWS). `MockIrpProvider` is a deterministic in-process implementation for tests and cassette replay; it does not call any IRP.

This crate does NOT sign, transmit, or produce evidence bundles. The IRP itself assigns the IRN, signs the JWS, and returns the signed QR — `IrpRegisterRequest.invoice_json` is sent unsigned. A real IRP backend (HTTP transport) is a feature-flagged `report-in-gst-http` / `report-in-gst-nic` follow-up; only the mock ships here.

## Coverage

The serializer emits the IRP-mandatory party and item fields mapped from the IR: party `LglNm`, `Addr1` (with overflow lines folded into `Addr2`), `Loc`, `Pin`, `Stcd`, the buyer `Pos` (place of supply), and per-item `PrdDesc`, `IsServc`, `Unit`, `Qty`, `UnitPrice`, amounts, `GstRt`, and the tax split. Documented residuals and synthesized defaults:

- **Placeholder HSN.** A line with no IR classification falls back to `HsnCd` `"9983"` (the SAC heading for "Other professional, technical and business services"), flagged as a service. This keeps the field schema-valid without inventing line-level data the IR does not carry.
- **Unit mapping.** The IR unit code is mapped onto the IRP unit-quantity-code (UQC) set; an absent or unrecognized code defaults to `OTH` ("Others").
- **`Gstin` placeholder.** A party with no GSTIN (export / business-to-consumer buyer) serializes as `URP` ("Unregistered Person") and is treated as inter-state.
- **State code fallback.** When a party carries no GSTIN, `Stcd` falls back to the address subdivision, then to `"96"` (the IRP "Other Country" code).
- **Document types.** Only `Invoice` (`INV`), `CreditNote` (`CRN`), and `DebitNote` (`DBN`) map; `ProForma` and `SelfBilled` return `Inv01Error::UnsupportedDocumentType`.
- **`SupTyp`.** The supply type (`B2B`, `SEZWP`, `SEZWOP`, `EXPWP`, `EXPWOP`, `DEXP`) comes from `Inv01Context`; it defaults to `B2B`.

## New IR fields

The serializer reads two IR fields beyond the core commercial surface:

- **Line classification (EN 16931 BT-158 + BT-158-1 scheme id).** The chosen classification's `code` becomes `HsnCd` — preferring an `HSN`/`SAC`-scheme entry, else the first. `IsServc` is derived from the chosen classification (a `SAC` scheme or a chapter-99 code marks a service) so it always agrees with `HsnCd`.
- **Document references classified as a preceding invoice.** Emitted as `RefDtls.PrecDocDtls` (`InvNo` = reference id, `InvDt` = its issue date as dd/mm/yyyy), only when such a reference is present.

## References

- NIC IRP e-Invoice JSON schema `INV-01`, schema version `1.1`: <https://einvoice1.gst.gov.in/Documents/EINVOICE_SCHEMA.xlsx>
- NIC IRP bulk-generation tools: <https://einvoice1.gst.gov.in/Others/BulkGenerationTools>
- Tax-split basis: Central GST Act 2017 / Integrated GST Act 2017 (CGST + SGST for intra-state, IGST for inter-state).
- IRP backends: government IRP1 `einvoice1.gst.gov.in`, IRP2 `einvoice2.gst.gov.in`, NIC API sandbox `einv-apisandbox.nic.in`.

## License

Apache-2.0
