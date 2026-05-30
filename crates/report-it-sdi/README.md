<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-it-sdi — Italy / Agenzia delle Entrate SDI / FatturaPA (FatturaElettronica)

National-clearance report adapter for the Italian Sistema di Interscambio. Serializes an InvoiceKit IR document to the real national FatturaPA (`FatturaElettronica`) XML, validates the issuer's Partita IVA / Codice Fiscale and `ProgressivoInvio`, and exercises the SDI sign-and-submit lifecycle offline through a deterministic mock provider.

Italy is a national-clearance jurisdiction: a B2B/B2G e-invoice is serialized to FatturaPA XML, XAdES-signed by the issuer, and submitted to the Agenzia delle Entrate Sistema di Interscambio (SDI), which returns one of five receipt kinds.

## Capabilities

- **Serialize (native FatturaPA)** — `to_fattura_pa_xml` turns a `invoicekit_ir::CommercialDocument` into deterministic FatturaPA `FatturaElettronica` XML, version `FPR12` (private B2B/B2G): `FatturaElettronicaHeader` (`DatiTrasmissione`, `CedentePrestatore`, `CessionarioCommittente`) and `FatturaElettronicaBody` (`DatiGenerali`, `DatiBeniServizi` with one `DettaglioLinee` per line and `DatiRiepilogo` per tax band). This is the real national format; the UBL and CII serializers do not emit it. Output is byte-stable by construction (fixed element order, no maps, amounts at fixed scale 2). The transmission-level `ProgressivoInvio` and `CodiceDestinatario` are supplied out-of-band via `FatturaPaContext`.
- **Validate (local)** — `validate_italian_tax_id` enforces the Partita IVA (11 digits) and Codice Fiscale (16 alphanumeric) shapes; `validate_progressivo` enforces the `ProgressivoInvio` shape (1..=5 alphanumeric). These are shape checks only. Reference-grade Schematron validation stays an external (JVM) backend and is not performed here.
- **Sign + transmit (offline mock)** — `MockSdiReportProvider` (implementing the `SdiReportProvider` trait) composes the existing `invoicekit_signer_sdi::MockSdiProvider` so the SDI XAdES signature path and `IdentificativoSdI` synthesis are exercised, not re-implemented. It returns a typed `SdiReportEnvelope` carrying `identificativo_sdi`, `receipt_kind`, the echoed `progressivo_invio`, a recorded UTC timestamp, the XAdES signature receipt, and a `reason` on rejection. `with_forced_receipt` drives any of the five receipt kinds, including the rejection path.
- **Evidence** — the caller bundles the canonical document, FatturaPA XML, signed XML, and receipt into a signed `.ikb` evidence bundle. This crate produces the signed FatturaPA bytes (`SdiReport::signed_fattura_xml`) and the receipt; it does not assemble the bundle itself.

Rejection is not an error: when SDI refuses an invoice it returns a `Notifica di Scarto` (NS), surfaced as an `Ok` `SdiReportEnvelope` whose `receipt_kind` is `SdiReceiptKind::NotificaScarto`, never as `Err`. `Err` (`SdiReportError`) is reserved for pre-wire shape failures (bad tax id, bad progressivo, empty payload) and transport faults.

## Coverage

The native FatturaPA serializer emits the mandatory FatturaElettronica spine — `DatiTrasmissione`, both party blocks, `DatiGeneraliDocumento`, `DettaglioLinee`, and `DatiRiepilogo`. It is not the full FatturaPA v1.2 schema. Documented residuals and simplifications present in the source:

- **Live transmission** is not implemented. The bundled `Mock*` providers are deterministic and offline; the Aruba/Infocert/Namirial web-service or PEC channel is bring-your-own-credentials and lands in a follow-up `report-it-sdi-http` crate.
- **Document types** — only `Invoice` (`TD01`), `CreditNote` (`TD04`), and `DebitNote` (`TD05`) map. Pro-forma and self-billed documents are rejected with `UnsupportedDocumentType`.
- **`Natura` is not emitted.** The coded VAT-exemption / reverse-charge value (`DatiRiepilogo` 2.2.2.2) is a controlled-list value the crate does not derive or map; only the free-text `RiferimentoNormativo` is emitted (see New IR fields).
- **`RegimeFiscale`** is fixed to `RF01` (ordinary regime) on the supplier.
- **Party fiscal id** — `(IdPaese, IdCodice)` is taken from the party's `vat`-scheme tax id (else the first tax id), with a leading 2-letter country prefix stripped. The supplier requires one (`MissingSupplierTaxId` otherwise); the customer's is optional.
- **`Provincia`** is emitted only when the address subdivision is exactly two ASCII letters; otherwise omitted.
- The intervening optional XSD blocks (`DatiRitenuta`, `DatiBollo`, `EsigibilitaIVA`, and the 2.1.2..2.1.5 groups) are not emitted.

## New IR fields

The serializer reads three IR fields that map to optional national elements, each emitted only when populated (a document without them produces byte-for-byte the prior output):

- **Line classifications** → `DettaglioLinee/CodiceArticolo` (FatturaPA 2.2.1.3): one group per `DocumentLine` classification, carrying the classification `scheme_id` verbatim as `CodiceTipo` and the `code` verbatim as `CodiceValore`. No national catalogue lookup is performed.
- **Document references** → `DatiGenerali/DatiFattureCollegate` (FatturaPA 2.1.6): one block per reference that classifies as a `PrecedingInvoice` (links a credit/debit note to the invoice it refers back to), with the reference id verbatim in `IdDocumento` and the referenced issue date in `Data` when supplied. Other reference kinds are skipped.
- **VAT exemption reason** → `DatiRiepilogo/RiferimentoNormativo` (FatturaPA 2.2.2.8): the tax band's `exemption_reason` (EN 16931 BT-120) verbatim. The coded `Natura` is not derived (see Coverage).

## References

- FatturaPA XML namespace: `http://ivaservizi.agenziaentrate.gov.it/docs/xsd/fatture/v1.2` (Agenzia delle Entrate), emitted as the `FatturaElettronica` root namespace with transmission format `FPR12`.

(The reference above is the one named in the crate's source; no external specification URLs are cited.)

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
