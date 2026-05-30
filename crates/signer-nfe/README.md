# invoicekit-signer-nfe

Brazil NF-e federal certificate flow adapter. Layers the per-state SEFAZ clearance contract on top of `invoicekit-signer`, signing the NF-e XML and modeling the `chave de acesso` + `protocolo de autorização` SEFAZ returns.

## What it signs

NF-e (Nota Fiscal Eletrônica) is a per-state tax-clearance flow: the taxpayer's ICP-Brasil-chained A1 certificate signs the NF-e XML, the state SEFAZ validates the document and assigns a 44-character `chave de acesso` plus a `protocolo de autorização`, and the buyer pulls the authorised invoice from the state's portal. The signature standard is **XAdES-BES** over the canonical NF-e XML.

## Capabilities

- `NfeProvider` trait — the surface every SEFAZ integration implements (one per state in production). Methods: `provider_name`, `environment`, `submit`.
- Typed Brazilian state codes (`UfCode`, ISO 3166-2:BR) for the high-volume UFs plus an `Other` catch-all, with IBGE numeric codes used when laying out the access key.
- `NfeEnvironment` — `Homologacao` (SEFAZ sandbox) vs `Producao` (live), including the numeric `tpAmb` value (`1` = produção, `2` = homologação) the NF-e XML carries.
- `IcpBrasilCertificate` — typed A1 certificate reference (X.509 serial, CNPJ, subject DN, PEM bytes).
- `NfeStampEnvelope` — typed result: the underlying `Signature` receipt, `chave_acesso`, `protocolo_autorizacao`, numeric `cStat`, typed `NfeStatus`, `xMotivo` description, signed NF-e XML, UF, and environment.
- `NfeStatus` — typed mapping of known SEFAZ `cStat` codes (100 authorized, 110/205 denied, 215 schema failure, 539 duplicate, else `Other`), with `from_c_stat` and `is_authorized`.
- `build_chave_acesso` — a deterministic 44-character mock access key, seeded with the IBGE UF code, CNPJ, and `nNF` via a fixed template (not a spec-conformant positional chave; the real key is built and validated by SEFAZ).
- `nfe_status_descricao` — canonical `xMotivo` text for known `cStat` codes.
- `MockNfeProvider` — deterministic test provider; records submissions and can be forced to return a specific `cStat`.

## Mode

**Mock-only.** The single provider that ships is `MockNfeProvider`. It performs no SEFAZ network call: it wraps the injected `invoicekit-signer` `Signer`, emits a `<XAdES-stub>...</XAdES-stub>`-wrapped XML body, builds the `chave de acesso` with fixed `cNF`/`cDV` placeholders so keys round-trip stably across runs, and returns a synthetic `protocolo de autorização`. The environment-mismatch and empty-CNPJ checks are real; everything past them is deterministic.

The signature receipt inherits whatever backend the caller passes. The substrate `invoicekit-signer::SoftwareSigner` is a placeholder keyed BLAKE3 MAC, not a real XAdES-BES signature; real RSA/ECDSA and HSM/PKCS#11 backends are tracked separately in the signer crate (T-083a / T-083b).

A live path needs: a real XAdES-BES XML signer driven by an ICP-Brasil A1 (or A3/HSM) certificate, and a real per-state SEFAZ web-service client that submits the signed NF-e and parses the returned `protocolo` and `cStat`. Neither ships here.

## Residuals

From the module documentation and source:

- No real SEFAZ transport. `submit` does not reach any state portal; `NfeError::Unavailable` exists as a shape but is not raised by the mock.
- The signed XML is a `<XAdES-stub>` wrapper, not a conforming XAdES-BES signature.
- `build_chave_acesso` uses fixed `cNF` (`00000000`) and `cDV` (`0`) placeholders for deterministic round-tripping. The real flow computes `cNF` from the emitting system and `cDV` via a mod-11 check digit.
- `UfCode` enumerates only high-volume states; all other UFs collapse to `Other` (IBGE code `99`).

## References

- NF-e specification (SEFAZ / Portal Nacional da NF-e) — `cStat` / `xMotivo` codes, `tpAmb`, `chave de acesso` layout — https://www.nfe.fazenda.gov.br/
- XAdES (XML Advanced Electronic Signatures), ETSI EN 319 132 — https://www.etsi.org/
- ISO 3166-2:BR (Brazilian state codes) — https://www.iso.org/obp/ui/#iso:code:3166:BR

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
