# invoicekit-report-br-nfe — Brazil / SEFAZ / NF-e (Nota Fiscal Eletronica)

National-clearance report adapter for Brazilian NF-e. Serializes an InvoiceKit IR document to the real national infNFe XML (layout `4.00`), validates the issuer's CNPJ/CPF, and exercises the NF-e sign-and-submit lifecycle offline through a deterministic mock provider.

Brazil is a per-state national-clearance jurisdiction: the invoice is serialized to infNFe XML (inside `<NFe>`), signed with the taxpayer's ICP-Brasil A1 certificate, and submitted to the state Secretaria da Fazenda (SEFAZ), which returns a numeric `cStat`, a 44-digit `chave de acesso`, and a `protocolo de autorizacao`.

## Capabilities

- **Serialize (native NF-e)** — `to_inf_nfe_xml` turns a `invoicekit_ir::CommercialDocument` into deterministic infNFe XML: `<ide>`, `<emit>`, `<dest>`, one `<det>` per line, and `<total><ICMSTot>`, all inside `<NFe><infNFe versao="4.00">`. This is the real national format; the UBL and CII serializers do not emit it. Output is byte-stable by construction (fixed element order, no maps, amounts at fixed scale 2). The `infNFe` `Id` attribute embeds the 44-digit chave de acesso.
- **Validate (local)** — `validate_cnpj`, `validate_cpf`, and `validate_brazil_tax_id` enforce the Brazilian taxpayer-id shapes (14-digit CNPJ, 11-digit CPF) with their two mod-11 check digits, ignoring punctuation. Reference-grade SEFAZ schema validation is an external backend and is not performed here.
- **Sign + transmit (offline mock)** — `MockNfeReportProvider` (implementing the `NfeReportProvider` trait) composes the existing `invoicekit_signer_nfe::MockNfeProvider` so the NF-e signature path, chave-de-acesso synthesis, and protocolo assignment are exercised, not re-implemented. It returns a typed `NfeReportEnvelope` carrying `chave_acesso`, `protocolo_autorizacao`, `c_stat`, status, and the XAdES signature receipt. `with_forced_c_stat` drives the denial path. Live SEFAZ transmission is out of scope here (see Coverage).
- **Evidence** — the caller bundles the canonical document, infNFe XML, signed XML, and receipt into a signed evidence bundle. This crate produces the signed infNFe bytes (`NfeReport::signed_nfe_xml`) and the receipt; it does not assemble the bundle itself.

Rejection is not an error: a SEFAZ denial (`cStat` `110`/`205`/`301`/`302`) is surfaced as an `Ok` `NfeReportEnvelope` whose `status` is a denial, never as `Err`. `Err` (`NfeReportError`) is reserved for pre-wire shape failures (bad tax id, empty payload) and transport faults.

## Coverage

The native infNFe serializer emits the mandatory NF-e spine only — `ide` / `emit` / `dest` / `det` / `total` (`ICMSTot`). It is not the full NF-e 4.00 schema. Documented residuals and simplifications present in the source:

- **Live transmission** is not implemented. The bundled `Mock*` providers are deterministic and offline; per-state SOAP web services over the ICP-Brasil mutual-TLS channel are bring-your-own-credentials and land in a follow-up `report-br-nfe-http` crate.
- **Document types** — only `Invoice` (`finNFe = 1`) and `CreditNote` (`finNFe = 4`, devolucao/retorno) map. Debit notes, pro-forma, and self-billed documents are rejected with `UnsupportedDocumentType`.
- **`mod`** is fixed to `55` (NF-e); NFC-e (`65`) is not emitted.
- **Tax** — `<total><ICMSTot>` carries only `vBC`, `vICMS`, `vProd`, and `vNF`, summed from the IR tax summary. Per-line ICMS/IPI/PIS/COFINS tax groups inside `<det>` are not emitted.
- **`<det>`** carries `cProd`, `xProd`, optional `NCM`, `uCom`, `qCom`, `vUnCom`, `vProd` only.
- **UF / IBGE codes** — `cUF` is mapped for a subset of states (SP, RJ, MG, PR, RS, SC, BA, DF) with a `99` catch-all for others. Party `UF` is derived from the address subdivision, falling back to `SP` for Brazil and `EX` for foreign addresses.
- **`dhEmi`** is synthesized from the IR issue date with a fixed `T00:00:00-03:00` offset; `CEP` is zero-padded to 8 digits; `cPais`/`xPais` are fixed to `1058`/`BRASIL`.

## New IR fields

The serializer reads one IR line classification: the NF-e `<NCM>` (8-digit Mercosur Common Nomenclature code, NF-e 4.00 tag I05) is sourced from the first `DocumentLine` classification whose `scheme_id` is `NCM` (matched case-insensitively, EN 16931 BT-158). It is emitted between `<xProd>` and `<uCom>` only when such a classification is present; a line with no NCM classification serializes without an `<NCM>` element. Non-NCM classification schemes are ignored.

## References

- NF-e XML namespace: `http://www.portalfiscal.inf.br/nfe` (Portal da Nota Fiscal Eletronica), emitted as the `<NFe>` root namespace and the infNFe layout `versao="4.00"`.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
