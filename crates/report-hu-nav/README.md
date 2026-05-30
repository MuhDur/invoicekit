# invoicekit-report-hu-nav — Hungary, NAV Online Számla (`InvoiceData` / RTIR XML)

Reporting adapter for Hungary's Nemzeti Adó- és Vámhivatal (NAV, the National Tax and Customs Administration) Online Számla v3.0 endpoints. It serializes the InvoiceKit commercial document to the real NAV `InvoiceData` XML and carries a typed `manageInvoice` transport surface with a deterministic mock; the live REST integration lands in a follow-up `report-hu-nav-http` crate.

## Capabilities

- **Serialize the real national format.** `to_invoice_data_xml` emits NAV Online Számla `InvoiceData` (RTIR) XML per the Online Számla Interface Specification v3.0 (`invoiceData.xsd`). This is the actual Hungarian national document, not a UBL relabelling: element names (`invoiceNumber`, `invoiceIssueDate`, `supplierInfo`/`customerInfo`, `invoiceLines`/`line`, `invoiceSummary`/`summaryNormal`) and the 8+1+2 `taxNumber` decomposition (`taxpayerId`/`vatCode`/`countyCode`) come straight from that schema, in the `http://schemas.nav.gov.hu/OSA/3.0/data` and `.../base` namespaces. Output is byte-stable by construction (fixed element order, no maps, amounts at fixed scale 2).
- **Local tax-id shape validation.** `validate_tax_id` checks a Hungarian tax id against the accepted shapes (8 / 9 / 11 digits, optionally hyphenated as `12345678-1-23`). Shape only — no checksum, no registry lookup.
- **Typed transport surface.** The `NavProvider` trait models the NAV flow (validate issuer tax id, exchange credentials for a one-shot token, POST the `manageInvoiceRequest` XML, return the NAV envelope) over the `Create` / `Modify` / `Storno` / `Annul` operations and `Test` / `Production` environments. `NavManageEnvelope` carries the NAV-assigned transaction id, observed `NavStatus` (`Received` / `InProgress` / `Done` / `Aborted`), recorded timestamp, and the `validationResult` text when aborted.

This crate does **not** sign, transmit over the wire, or produce evidence bundles. The only `NavProvider` implementation it ships is `MockNavProvider`, a deterministic in-memory stub (serial transaction ids, fixed timestamp) for cassette-replay tests. There is no live HTTP, TLS, token exchange, or signing here — those land in the follow-up `report-hu-nav-http` crate behind a feature flag.

## Coverage

- **Document types.** `Invoice`, `CreditNote`, and `DebitNote` map to the NAV `invoiceCategory` `NORMAL`. A credit/debit note is reported as a `NORMAL` invoice whose reversal is carried by the upstream `MODIFY`/`STORNO` `invoiceOperation`, so it still maps to `NORMAL` here. `ProForma` and `SelfBilled` return `UnsupportedDocumentType`.
- **Tax number.** Extracted from a party's `vat` scheme id (else its first tax id), the `HU` prefix stripped, then split 8+1+2. The 8-digit core is required; the 1-digit VAT code and 2-digit county code are optional, mirroring the schema. A supplier with no usable tax id returns `MissingSupplierTaxId`.
- **Customer.** `customerVatStatus` is always emitted as `DOMESTIC` (a Hungarian taxable buyer); a non-domestic buyer is not distinguished. `customerVatData`/`customerTaxNumber` is emitted only when the customer carries a usable tax id.
- **Currency / exchange rate.** `currencyCode` is taken from the document; `exchangeRate` is hard-coded to `1`. A foreign-currency invoice would need a real exchange rate and HUF conversion that this crate does not compute — the `*HUF` amount fields are filled with the same value as the document-currency fields.
- **VAT per line.** Each line's `vatPercentage` is looked up from the tax-summary entry matching the line's tax category, defaulting to zero; `lineVatAmount` is computed as `net * rate / 100` rounded to 2 places. Untrusted amounts that overflow `Decimal` during line VAT, the net/VAT/gross totals, return a typed `TotalsUnrepresentable` rather than panicking.
- **Modification index.** When an `invoiceReference` is emitted, `modificationIndex` is set to the minimal defensible value `1` and `modifyWithoutMaster` to `false`; the IR carries no modification sequence, so no national value is derived.

The transport side is mock-only. `validate_tax_id` is shape validation, not a check-digit or taxpayer-registry validation.

## New IR fields

The serializer emits a **document reference** from the IR: the first reference classifying as a preceding invoice (`ReferenceKindClass::PrecedingInvoice`) is written verbatim into NAV `invoiceReference`/`originalInvoiceNumber`, placed as the first child of `invoice` before `invoiceHead`, per the `InvoiceType` element order in `invoiceData.xsd`. Non-preceding references (for example a purchase order) do not synthesize an `invoiceReference`. The id is emitted verbatim — not normalized, prefixed, or otherwise transformed.

## References

- NAV Online Számla Interfész-specifikáció (Interface Specification) v3.0 — `InvoiceData` / `invoiceData.xsd`, namespaces `http://schemas.nav.gov.hu/OSA/3.0/data` and `http://schemas.nav.gov.hu/OSA/3.0/base`: <https://onlineszamla.nav.gov.hu/dokumentaciok>
- NAV Online Számla v3.0 endpoints: `api.onlineszamla.nav.gov.hu` (production), `api-test.onlineszamla.nav.gov.hu` (test).

## License

Apache-2.0.
