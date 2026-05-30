<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-gr-mydata

Greece — IAPR (Independent Authority for Public Revenue, ΑΑΔΕ) **myDATA** reporting adapter. Serializes an InvoiceKit `CommercialDocument` to a myDATA `InvoicesDoc` XML and carries the typed transmission surface (MARK / UID / QR, report request and envelope).

myDATA is Greece's mandatory continuous reporting of invoice summaries to the IAPR REST endpoints. The authority returns a MARK (Μοναδικός Αριθμός Καταχώρησης, unique registration number) and a UID, which the issuer embeds in the printed invoice's QR code.

## Capabilities

- **Serialize the real national format.** `to_invoices_doc_xml` emits an AADE myDATA `InvoicesDoc` XML document — this is the actual Greek reporting format, not UBL relabelled. Element names and nesting follow the myDATA `InvoicesDoc` XSD (namespace `http://www.aade.gr/myDATA/invoice/v1.0`): an `InvoicesDoc` root wrapping one `invoice`, whose children are, in XSD order, `issuer`, `counterpart`, `invoiceHeader`, one `invoiceDetails` per line, and a single `invoiceSummary`. Output is byte-stable by construction (fixed element order, no maps, amounts at fixed scale 2).
- **Shape-level AFM check.** `validate_afm` confirms a Greek tax registration number (ΑΦΜ) is exactly nine ASCII digits. The AFM checksum is a separate concern and is not validated here.
- **QR payload.** `qr_payload` builds the IAPR e-books QR string `{base_url}/?mark={MARK}&uid={UID}` from a report envelope.
- **Transmission surface (typed only).** The `MyDataProvider` trait plus `MyDataReportRequest` / `MyDataReportEnvelope` / `MyDataStatus` / `MyDataMark` / `MyDataUid` / `MyDataEnvironment` model the IAPR report call. The only implementation shipped here is the deterministic `MockMyDataProvider`, which synthesises a MARK and UID for cassette-replay tests. There is no live HTTP transmission in this crate.

This crate does not sign, does not build evidence bundles, and does not perform live transmission. The live REST integration lands in a follow-up `report-gr-mydata-http` crate behind a feature flag, so operators who only need the substrate do not pull in the HTTP stack.

## Coverage

The serializer emits the mandatory myDATA `InvoicesDoc` structure for the supported document types. Documented residuals, mirroring the source's honest notes:

- **Document types.** `Invoice` maps to `invoiceType` `1.1`, `CreditNote` to `5.1`, `SelfBilled` to `3.1`. `DebitNote` and `ProForma` have no `invoiceType` mapping and return `MyDataXmlError::UnsupportedDocumentType`. The default codes are structural; a caller can target a finer sub-code through `MyDataInvoiceCategory` on the report request, but `to_invoices_doc_xml` itself emits the default for the document type.
- **VAT category.** `vatCategory` is derived from the percentage rate per the AADE codelist: `1` = 24%, `2` = 13%, `3` = 6%, `4` = 17%, `5` = 9%, `6` = 4%, and `7` = 0% / excluding VAT. Any zero or unrecognised rate falls to `7`. When `vatCategory` is `7`, the XSD-mandatory `vatExemptionCategory` is emitted; the crate emits a fixed placeholder exemption code (`1`), not a value derived from the document.
- **Per-line VAT is pro-rated.** Each line's VAT is the band tax pro-rated by the line's net over the band's taxable base, rounded to two places. A line with no matching tax-summary entry is treated as excluding-VAT (category `7`).
- **Line classifications are dropped.** `DocumentLine.classifications` carry EN 16931 BT-158 commodity/HS-style codes. myDATA's per-line classification elements (`incomeClassification` / `expensesClassification`) take AADE income/expense catalog codes, a different national catalog with no verbatim BT-158 target. A populated `classifications` does not change the output.
- **Exemption reason text and code are dropped.** myDATA encodes VAT exemption only as the coded integer `vatExemptionCategory`. The free-text BT-120 `exemption_reason` and the BT-121 `exemption_reason_code` (a VATEX/Natura code) have no verbatim myDATA target and do not change the output.
- **VAT-number normalisation.** The two-letter EU-VAT country prefix (e.g. `EL`) is stripped so `vatNumber` is the bare nine-digit AFM the myDATA endpoints expect.
- **Untrusted-amount safety.** Monetary accumulation and the per-line pro-rate use checked arithmetic; amounts near `Decimal::MAX` return `MyDataXmlError::TotalsUnrepresentable` rather than panicking.

## New IR fields

The serializer reads two IR surfaces beyond the core commercial fields:

- **Document references (VAT exemption category aside).** When the IR document carries a preceding-invoice reference — a `DocumentReference` whose EN 16931 classification is `PrecedingInvoice` — the referenced identifier is emitted verbatim (XML-escaped only) as the myDATA `correlatedInvoices` element inside `invoiceHeader`, in XSD sequence order after `invoiceType`. The adapter does not derive, parse, validate, or rewrite the reference id. References of other classes (e.g. an order reference) are not routed here.
- **VAT exemption (coded only).** A line whose resolved `vatCategory` is `7` emits the mandatory `vatExemptionCategory` integer. As noted above, this is a fixed placeholder code, not a mapping from the IR's `exemption_reason` / `exemption_reason_code`.

## References

- IAPR myDATA documentation portal: <https://www.aade.gr/mydata>
- AADE myDATA REST API / technical specifications and versions (the `InvoicesDoc` XSD and codelists, namespace `http://www.aade.gr/myDATA/invoice/v1.0`): <https://www.aade.gr/en/mydata/technical-specifications-versions-mydata>

## License

Apache-2.0.
