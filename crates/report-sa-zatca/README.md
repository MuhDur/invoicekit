<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-sa-zatca — Saudi Arabia (ZATCA) Phase 2 / Fatoora report adapter, ZATCA UBL 2.1

Saudi Arabia ZATCA Phase 2 (Fatoora) e-invoice report adapter. Saudi Arabia runs a Continuous Transaction Control regime: standard (B2B/B2G) invoices are *cleared* by the Zakat, Tax and Customs Authority portal before reaching the buyer, and simplified (B2C) invoices are *reported* afterward. The wire format is UBL 2.1 carrying a ZATCA cryptographic-stamp extension, an invoice-hash chain, an Invoice Counter Value, and a Tag-Length-Value QR code.

## Coverage

EN 16931 / UBL 2.1 family path with a ZATCA-specific envelope. This crate does **not** mint a fully bespoke national serializer from scratch. The UBL 2.1 spine is produced by `invoicekit-format-ubl` (`to_xml`); `to_zatca_ubl_xml` then splices in the ZATCA bits a plain UBL document does not carry: the `UBLExtensions` cryptographic-stamp envelope, `cbc:ProfileID` (`reporting:1.0`), the invoice `cbc:UUID`, the `cbc:InvoiceTypeCode` with the ZATCA function flag in `@name`, and the ICV / PIH `cac:AdditionalDocumentReference` chain links. Output is deterministic by construction.

Honest residuals, mirroring the module doc-comment:

- **The cryptographic-stamp slot is a placeholder.** The injected `UBLExtensions` is the well-formed empty `xades` extension slot ZATCA's signing tooling expects; the real signature value is filled in by the stamp step.
- **The injected `cbc:CompanyID` is an auditability convenience.** Real ZATCA carries the seller VAT inside `cac:AccountingSupplierParty`, which the UBL spine already emits; the extra marker is so structural validators can assert seller identity without re-parsing the party block.
- **Only `Invoice` and `CreditNote` map.** They serialize to `cbc:InvoiceTypeCode` `388` and `381`. `DebitNote`, `ProForma`, and `SelfBilled` are rejected with `ZatcaUblError::UnsupportedDocumentType`.
- **Reference validation is out-of-process.** Local validation covers Saudi identifier and chain-state *shapes* only (see Capabilities). Reference-grade ZATCA validation stays an external JVM backend and is labelled as such in the capability matrix; it is not embedded here.
- **Simplified-invoice QR Tags 6–8 are placeholders.** The mock provider derives Tag 6 (invoice hash) from the payload but fills Tags 7–8 (signature value, public key) from the CSID id until the real secp256k1 provider lands.
- **Transmission is offline-only.** The provider here is `MockZatcaReportProvider`: deterministic and offline. Live Fatoora transmission (the ZATCA compliance/production REST API with a real CSID) is bring-your-own-credentials and lands in a follow-up `report-sa-zatca-http` crate.

## Capabilities

- **Serialize** — `to_zatca_ubl_xml(&CommercialDocument, &ZatcaUblContext)` projects an `invoicekit_ir::CommercialDocument` to deterministic ZATCA Phase 2 UBL 2.1 XML (UBL spine via `invoicekit-format-ubl` plus the ZATCA envelope). `build_qr_fields` constructs the five mandatory QR TLV fields (Tag 1 seller name, Tag 2 VAT number, Tag 3 timestamp, Tag 4 tax-inclusive total, Tag 5 VAT total); VAT-total summation uses checked `Decimal` addition and surfaces `AmountOverflow` rather than panicking.
- **Validate (local)** — `validate_saudi_vat_number` (15 digits, starts and ends with `3`, `1` at position 11, optional `SA` prefix stripped), `validate_invoice_counter_value` (monotonic per-device counter, starts at 1, `0` invalid), `validate_previous_invoice_hash` (44- or 88-char padded base64 SHA-256).
- **Sign + stamp** — `MockZatcaReportProvider` composes `invoicekit_signer_zatca::MockPhase2Provider`, so the ZATCA cryptographic-stamp path, QR TLV envelope, and invoice-hash synthesis are exercised through the real signer substrate rather than re-faked. The provider is deterministic and offline.
- **Clearance vs reporting** — `ZatcaReportProvider::report` returns a typed `ZatcaReport` (audit receipt plus QR TLV bytes plus the full stamp envelope). The verdict `ZatcaClearanceKind` maps the signer's `ReportingStatus` and the invoice mode to `Cleared` (standard/B2B clearance), `Reported` (simplified/B2C), `AcceptedWithWarnings`, or `Rejected`.
- **Evidence** — `ZatcaReportEnvelope` carries the clearance verdict, mode, the real `cbc:UUID` echoed verbatim (never synthesized from the counter), the invoice hash (which becomes the next invoice's PIH), the counter, the recorded timestamp, the stamp signature, and a rejection reason when applicable — for the caller to bundle into a signed `.ikb` evidence bundle.

Rejection is a verdict, not an error. A portal refusal is surfaced as an `Ok` envelope with `ZatcaClearanceKind::Rejected`, never as `Err`. `Err` (`ZatcaReportError`) is reserved for pre-wire shape failures (bad VAT, bad ICV, bad PIH, empty payload) and transport/TLS/DNS faults.

## New IR fields

None. This crate adds no IR fields. The ZATCA Phase 2 fields that are not part of the jurisdiction-agnostic IR — `cbc:UUID`, the Invoice Counter Value, the Previous Invoice Hash, and the invoice mode (standard vs simplified) — are carried out-of-band in `ZatcaUblContext`, not read back from new IR line classifications, document references, or VAT-exemption fields.

## Public API

- `to_zatca_ubl_xml(&CommercialDocument, &ZatcaUblContext) -> Result<String, ZatcaUblError>`
- `build_qr_fields(&CommercialDocument, timestamp_rfc3339) -> Result<BTreeMap<QrField, String>, ZatcaUblError>`
- `validate_saudi_vat_number`, `validate_invoice_counter_value`, `validate_previous_invoice_hash`
- `ZatcaUblContext` (`genesis`, `GENESIS_PIH`), `ZatcaUblError`
- `ZatcaReportProvider` trait; `MockZatcaReportProvider` (`new`, `with_forced_status`)
- `ZatcaReportRequest`, `ZatcaReport`, `ZatcaReportEnvelope`, `ZatcaReportError`
- `ZatcaClearanceKind` (`is_accepted`, `from_reporting_status`)
- Constants `ZATCA_PROFILE_ID`, `ZATCA_INVOICE_TYPE_LIST_URI`
- Re-exported signer substrate: `CsidRecord`, `ReportingStatus`, `ZatcaEnvironment`, `ZatcaStampEnvelope`, `InvoiceMode` (`ZatcaInvoiceMode`), `ZatcaQrField`, `Signature`
- `crate_name()`

## References

- ZATCA (Zakat, Tax and Customs Authority) Phase 2 / Fatoora e-invoicing regime — UBL 2.1 with the ZATCA `UBLExtensions` cryptographic-stamp envelope, Invoice Counter Value (ICV), Previous Invoice Hash (PIH) chain, and TLV QR code.
- UN/CEFACT code list 1001 (`InvoiceTypeCode`): `388` invoice, `381` credit note.
- UBL InvoiceTypeCode list URN: `urn:oasis:names:specification:ubl:codelist:gc:InvoiceTypeCode-2.1`.
- ZATCA `cbc:ProfileID` reporting profile: `reporting:1.0`.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
