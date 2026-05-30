<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-tr-efatura — Turkey (Gelir İdaresi Başkanlığı, GİB e-Fatura / e-Arşiv clearance)

Typed clearance adapter for Turkey's GİB (Gelir İdaresi Başkanlığı, the Revenue Administration) e-Fatura and e-Arşiv mandates. It carries an already-built UBL-TR invoice as an opaque payload to the GİB clearance flow and surfaces the per-invoice verdict and the GİB-issued ETTN; it does not build the UBL-TR XML itself.

GİB runs two parallel mandates over the same wire format: **e-Fatura** for business-to-business exchange between registered issuers and receivers, and **e-Arşiv** for non-registered receivers (business-to-consumer, or business-to-business outside the e-Fatura mükellef list), with summaries reported back to GİB.

## Capabilities

- **Transmit (typed surface only).** Defines `EFaturaProvider` with `submit_invoice` (takes an `EFaturaSubmitRequest` — tenant id, environment, mandate, issuer VKN, optional buyer tax id, and the `invoice_xml: Vec<u8>` payload; returns an `EFaturaSubmitEnvelope` carrying the 16-char ETTN, status, RFC-3339 submitted-at timestamp, and an optional message) and `cancel_invoice` (cancel a prior submission by ETTN within the legal window).
- **Local validation.** `validate_vkn` enforces a 10-ASCII-digit VKN (Vergi Kimlik Numarası); `validate_tax_id` accepts either a 10-digit VKN or an 11-digit TCKN (Türkiye Cumhuriyeti Kimlik Numarası). `submit_invoice` validates the issuer VKN, validates the buyer tax id when supplied, and rejects an empty payload. These fail before the wire as `EFaturaError::BadTaxId` / `EFaturaError::BadXml`.
- **Deterministic mock.** `MockEFaturaProvider` returns a `Cleared` envelope per `submit_invoice` and `Cancelled` per `cancel_invoice`, with fixed timestamps and incrementing serial ETTNs (`with_fixed_submitted_at` for a custom timestamp).

It does **not** serialize the UBL-TR format. The `invoice_xml` field is a pre-built canonical UBL-TR payload supplied by the caller; this crate does not build, parse, or inspect it. It does not sign, does not compute a canonical form, does not perform the EN 16931 / UBL path here, and does not produce evidence bundles.

## Coverage

Opaque-payload / bring-your-own. The crate models the GİB submit-and-cancel contract with typed Rust surfaces, but the only implementation shipped is `MockEFaturaProvider`. There is no live network transport in this crate.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockEFaturaProvider` ships. The live GİB SOAP integration lands in a follow-up `report-tr-efatura-http` crate behind a feature flag. `EFaturaError::Transport` exists for that wire; no HTTP / TLS / DNS code is present here.
- **GİB-side verdict is a status, not an error.** A receiver rejection (Red Yanıtı) arrives after submission, so `submit_invoice` surfaces it through `EFaturaStatus::Rejected` inside the returned envelope rather than as an `Err`. `EFaturaError` is reserved for pre-wire shape failures and transport failures.
- **Statuses** modelled: `Submitted`, `Cleared`, `Rejected` (Red Yanıtı), `Cancelled` (İptal).
- **Environment** selects `Sandbox` (GİB sandbox) or `Production` (`efatura.gib.gov.tr`); the mock ignores the distinction.

## References

Sourced from the module doc-comment. The GİB e-Fatura / e-Arşiv clearance portal at `efatura.gib.gov.tr` (production) and the GİB sandbox; the ETTN (Evrensel Tekil Tanımlama Numarası) identifier the issuer prints on the invoice. No specification or regulator URLs are cited in the source.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
