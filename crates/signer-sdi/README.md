<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-signer-sdi — Italy SDI (Sistema di Interscambio) signing adapter, XAdES-BES over FatturaPA

Layers the Agenzia delle Entrate FatturaPA / SDI contract on top of `invoicekit-signer`. Italian e-invoices must be XAdES-BES-signed by the issuer, then delivered to SDI, which routes the invoice to the buyer and returns one of five receipt kinds.

## Capabilities

- **Provider surface** — the `SdiProvider` trait (`provider_name`, `submit`) that every SDI integration implements (Aruba, Infocert, Namirial, ...). It takes an `SdiSubmitRequest` (FatturaPA XML bytes, qualified certificate, transport, `ProgressivoInvio`) and returns a typed `SdiStampEnvelope`.
- **Typed SDI vocabulary** — `SdiTransport` (`WebService` for the REST/SOAP API, `Pec` for certified email), `SdiReceiptKind` (RC `RicevutaConsegna`, NS `NotificaScarto`, MC `MancataConsegna`, NE `NotificaEsito`, MT `Metadata`) with an `is_delivered` predicate, `ArubaQualifiedCertificate` (serial, codice fiscale, subject DN, PEM bytes), and `SdiError`.
- **Stamp envelope** — `SdiStampEnvelope` carries the `Signature` receipt, the `IdentificativoSdI` SDI assigns post-acceptance, the receipt kind, the echoed `ProgressivoInvio`, the signed FatturaPA bytes, and the transport used.
- **Deterministic mock provider** — `MockSdiProvider` exercises the submit path end to end against an injected `Signer`: it calls the signer, synthesizes a sequential `IT`-prefixed `IdentificativoSdI`, records every submission, and returns an envelope. `with_forced_receipt` drives any of the five receipt kinds, including the rejection path.

## Mode

**Mock only.** No real cryptographic XAdES-BES signer ships in this crate. `MockSdiProvider` does not produce a real XAdES signature: it delegates to whatever `Signer` is injected (the workspace `SoftwareSigner` in tests) and wraps the FatturaPA bytes in a literal `<XAdES-stub>...</XAdES-stub>` tag so callers can verify the envelope shape. There is no live SDI call — `WebService` and `Pec` are typed enum values, not wired transports.

The live path needs:

- A QTSP-issued qualified certificate and its private key — bring-your-own-credentials. The customer holds the cert (the `ArubaQualifiedCertificate` fields describe it); InvoiceKit does not issue or escrow it.
- A real XAdES-BES signer producing the enveloped signature over the FatturaPA XML.
- A real SDI transport (Aruba/Infocert/Namirial `WebService` or `Pec`) to submit and collect receipts.

Per `Cargo.toml`, real XAdES-BES signing plus SDI HTTP/PEC integration is planned behind a future `sdi-http` feature flag; it is not present today.

## Residuals

Documented in the module doc-comment and source:

- Only `MockSdiProvider` ships. The real provider path (signer + transport) is not implemented.
- The "signed" envelope is a `<XAdES-stub>` wrapper, not a conformant XAdES-BES signature.
- The two `SdiTransport` variants are type-level only; neither WebService nor PEC delivery is performed.
- `certificate_pem` is treated as opaque bytes; the certificate is not parsed or validated.
- `submissions()` and the `IdentificativoSdI` counter use a `Mutex` and will panic if a prior holder panicked.

## References

Named in the source; no external URLs are cited:

- Agenzia delle Entrate — FatturaPA / Sistema di Interscambio (SDI).
- XAdES-BES — the signature profile Italian e-invoices require (named in the doc-comment; not implemented here).

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
