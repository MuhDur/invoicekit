<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-pl-ksef — Poland / Ministry of Finance KSeF / FA(3) (FA_VAT)

National-clearance report adapter for Poland's KSeF (Krajowy System e-Faktur). Serializes an InvoiceKit IR document to the real national FA(3) XML, validates the issuer's NIP, and exercises the KSeF sign-and-submit lifecycle offline through a deterministic mock provider.

Poland is a national-clearance jurisdiction: a B2B e-invoice is serialized to the FA(3) schema (`FA_VAT`, root `<Faktura>`), the taxpayer authenticates a KSeF session, submits the XML, and the Ministry of Finance portal returns a KSeF reference number (`Numer KSeF`) plus an official acknowledgement of receipt (`UPO` — Urzędowe Poświadczenie Odbioru).

## Capabilities

- **Serialize (native FA(3)).** `to_fa3_xml` turns a `invoicekit_ir::CommercialDocument` into deterministic FA(3) XML inside `<Faktura>`: a `<Naglowek>` form header (`KodFormularza` system code `FA (3)`, schema version `1-0E`, `WariantFormularza` 3, `DataWytworzeniaFa`, `SystemInfo`), `<Podmiot1>` (sprzedawca) and `<Podmiot2>` (nabywca) parties, and an `<Fa>` body with `KodWaluty`, `P_1`/`P_2` (issue date / invoice number), `RodzajFaktury`, the VAT-summary totals (`P_13_1`, `P_14_1`, `P_15`), and one `<FaWiersz>` per line. This is the real national format; the UBL and CII serializers do not emit it. Output is byte-stable by construction (fixed element order, no maps, amounts at fixed scale 2). The form-level fields that are not part of the jurisdiction-agnostic IR (`DataWytworzeniaFa`, `SystemInfo`) are supplied by the caller via `Fa3Context`.
- **Validate (local).** `validate_nip` enforces the Polish NIP (Numer Identyfikacji Podatkowej): exactly 10 ASCII digits with the official weighted modulo-11 checksum matching the final digit, rejecting a modulo result of 10. Reference-grade XSD validation against the Ministry of Finance FA(3) schema is an external (JVM) backend and is not performed here.
- **Sign + transmit (offline mock).** `MockKsefReportProvider` (implementing the `KsefReportProvider` trait) composes the existing `invoicekit_signer_ksef::MockKsefProvider` so the KSeF session/submit path, the XAdES signature, and the `Numer KSeF` synthesis are exercised, not re-implemented. It validates the issuer NIP, rejects an empty payload, opens a session, signs, submits, and returns a typed `KsefReportEnvelope` carrying `numer_ksef`, `upo_reference`, the `KsefAcceptance` status, the echoed issuer NIP, a recorded timestamp, an optional rejection reason, and the signature receipt. `with_forced_acceptance` drives the rejection path. Live KSeF transmission is out of scope here (see Coverage).
- **Evidence.** The caller bundles the canonical document, FA(3) XML, signed artifact, and receipt into a signed evidence bundle. This crate produces the signed FA(3) bytes (`KsefReport::signed_fa_xml`, a deterministic `<KsefSignedInvoice>` envelope wrapping the FA(3) payload plus the XAdES `SignatureValue`) and the receipt; it does not assemble the bundle itself.

Rejection is not an error: when KSeF refuses an invoice the verdict is surfaced as an `Ok` `KsefReportEnvelope` whose `acceptance` is `KsefAcceptance::Rejected` (with an empty `numer_ksef` and a reason), never as `Err`. `Err` (`KsefReportError`) is reserved for pre-wire shape failures (bad NIP, empty payload) and transport faults.

## Coverage

The native FA(3) serializer emits the mandatory invoice spine only — `<Naglowek>`, `<Podmiot1>`, `<Podmiot2>`, and an `<Fa>` body with the `P_13_1` / `P_14_1` / `P_15` totals and per-line `<FaWiersz>` rows. It is not the full FA(3) schema. Documented residuals and simplifications present in the source:

- **Live transmission** is not implemented. The bundled `Mock*` providers are deterministic and offline. Live KSeF transmission (HTTPS to `ksef-test.mf.gov.pl` / `ksef.mf.gov.pl`, XAdES `InitSession` signing, a NIP-bound qualified certificate or KSeF token) is bring-your-own-credentials and lands in a follow-up `report-pl-ksef-http` crate.
- **Document types** — only `Invoice` (`RodzajFaktury` `VAT`) and credit/debit notes (both mapped to `KOR`, korekta) are representable. Pro-forma and self-billed documents are rejected with `Fa3Error::UnsupportedDocumentType`.
- **Seller NIP is required.** `Podmiot1` must carry a NIP (a `vat`/`nip`-scheme tax id is preferred, else the first tax id, with a leading `PL` prefix stripped); a missing seller NIP is `Fa3Error::MissingSellerNip`. The buyer's NIP is optional (foreign / consumer nabywca).
- **VAT summary is a single rate group.** The IR tax summary is projected into one net/VAT pair (`P_13_1` / `P_14_1`) by summing all categories; `P_15` is the IR gross total. The per-line `P_12` VAT rate is looked up from the tax-summary entry matching the line's tax category, defaulting to `0.00`. Tax-summary totals are accumulated with checked arithmetic; a sum exceeding `Decimal`'s range is the typed error `Fa3Error::TotalsUnrepresentable` (naming the field), not a panic.
- **`<FaWiersz>`** carries `NrWierszaFa`, `P_7` (description), `P_8B` (quantity), `P_9A` (unit net price), `P_11` (line net value), and `P_12` (VAT rate) only.
- **Addresses** — `<Adres>` emits `KodKraju` (ISO 3166-1 alpha-2), `AdresL1` (joined address lines), and `AdresL2` (`postal_code` + ` ` + `city`); the `DataWytworzeniaFa` header timestamp is caller-pinned for byte-stable output.
- **Correction, exemption, and commodity-classification elements are deliberately not emitted.** No FA(3) XSD is vendored in-repo, so the national element names that would carry a correction's preceding-invoice link (`DaneFaKorygowanej` / `NrFaKorygowanej` / `DataWystFaKorygowanej`), the VAT-exemption legal basis (`P_19C` / `P_19`), and a line commodity classification (`CN` / `PKWiU`) are not confirmable from any authoritative source. A wrong element name is worse than an omission, so the serializer emits nothing for these; a regression test pins that populating the corresponding IR fields leaves the FA(3) bytes byte-identical.

## New IR fields

This crate reads none of the new IR fields into the FA(3) output. The IR may carry a preceding-invoice `DocumentReference` (for KOR corrections), `tax_summary` VAT-exemption reason and code, and `DocumentLine` commodity classifications (e.g. a CN code), but the serializer intentionally ignores all three until their FA(3) national element names can be confirmed from an authoritative schema (see Coverage). The values do not appear in the emitted XML.

## References

Only sources named in the source are listed.

- FA(3) namespace: `http://crd.gov.pl/wzor/2025/06/25/06251/` (Polish Ministry of Finance Centralne Repozytorium Dokumentów, 2025 `FA/3` schema), emitted as the `<Faktura>` root namespace.
- KSeF environment endpoints: `ksef-test.mf.gov.pl` (test) / `ksef.mf.gov.pl` (production), named for the bring-your-own-credentials transport deferred to `report-pl-ksef-http`.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
