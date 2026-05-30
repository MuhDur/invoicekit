# invoicekit-signer-cfdi — Mexico CFDI 4.0 PAC stamping (sello digital + Timbre Fiscal Digital)

The CFDI 4.0 signing surface, layered on top of `invoicekit-signer`. A Mexican invoice is closed by a Proveedor Autorizado de Certificación (PAC) — a SAT-authorised intermediary — which issues the digital seal (`sello`) and the Folio Fiscal (UUID) over the document's `cadena original`.

## Capabilities

- **PAC provider surface** — `CfdiPacProvider` is the trait every PAC integration implements: `provider_name`, `environment`, and `stamp(request, target_environment)`. It bundles an underlying `invoicekit_signer::Signer` with the CFDI-specific stamp operation.
- **Typed request and envelope** — `CfdiSignRequest` carries the canonical CFDI 4.0 XML bytes, the taxpayer's `CertificadoSelloDigital` (CSD), the `CfdiKind`, and audit-only operator metadata. `stamp` returns a `CfdiStampEnvelope`: the underlying signer `Signature`, the `UUID` (Folio Fiscal), the `cadena_original`, the taxpayer `selloCFDI`, the PAC's wrapping `selloSAT`, the PAC certificate serial, and the `FechaTimbrado`.
- **Invoice kinds** — `CfdiKind` enumerates the SAT comprobante types the consumer can declare: Ingreso, Egreso, Traslado, Nómina, Pago, Retención.
- **Environment gating** — `PacEnvironment` (Sandbox / Production); `stamp` rejects a request whose target environment does not match the provider's configured environment with `CfdiError::EnvironmentMismatch`.
- **`cadena original` helper** — `compute_cadena_original(cfdi_xml, rfc)` produces the pre-stamp string the seal is computed over. **This is a deterministic stand-in, not the real transform** (see Mode).
- **PAC seal helper** — `wrap_pac_seal(sello_cfdi, pac_certificate_serial)` produces the outer `selloSAT` wrapper. **Deterministic stand-in, not a real signature** (see Mode).
- **Mock provider** — `MockCfdiPacProvider` is the only `CfdiPacProvider` that ships. It is deterministic, records every stamp request (`stamps()`), synthesizes monotonic UUIDs, and pins `FechaTimbrado` (overridable via `with_fixed_fecha_timbrado` for cassette-replay).

## Mode

**Mock-only. No real cryptographic CFDI signer ships in this crate.**

The CFDI signature scheme is RSA-SHA256: the taxpayer signs the `cadena original` with the private key of their SAT-issued Certificado de Sello Digital, and the PAC wraps that inner `selloCFDI` in its own SAT-certificate seal (`selloSAT`) to produce the Timbre Fiscal Digital.

What ships today is the typed surface plus `MockCfdiPacProvider`, which exercises that shape deterministically:

- the seal `Signature` is whatever the injected `invoicekit_signer::Signer` returns (in tests, the in-memory `SoftwareSigner`) — it is not a real RSA-SHA256 CFDI seal;
- `compute_cadena_original` is an FNV-1a digest stand-in formatted as `||4.0|{rfc}|{hex}||`, not the SAT XSLT transform; and
- `wrap_pac_seal` is a `pac:{serial}:{sello}` string concatenation, not a real PAC signature over the inner seal.

The real provider, per the module doc-comment, needs **RSA-SHA256 signing, the SAT `cadena original` XSLT transform, and a PAC sandbox account**. It is slated to land behind a future `cfdi-rsa` feature flag and is not present in this crate. Real PAC integrations (Solución Factible / Edicom / Facturando / etc.) are bring-your-own-credentials: the taxpayer holds the CSD and the PAC account.

## Residuals

Documented in the module doc-comment and source:

- The real provider is gated behind a not-yet-implemented `cfdi-rsa` feature flag.
- `compute_cadena_original` is a deterministic placeholder shaped `||4.0|{rfc}|{hex}||`. The real `cadena original` starts with `||1.0|` and walks the CFDI XML in document order via the SAT-published XSLT; this crate does not run an XSLT engine.
- `wrap_pac_seal` produces a deterministic concatenation, not a SAT-certificate signature.
- The CSD `certificate_pem` field is kept opaque (`Vec<u8>`); the substrate does not parse it. The real provider would.
- `MockCfdiPacProvider` validates only that the CSD `rfc` is non-empty (`CfdiError::CsdInvalid`); it does not check the CSD validity window, even though `CertificadoSelloDigital` carries `not_before` / `not_after`.

## References

- CFDI 4.0 (Comprobante Fiscal Digital por Internet) and the `cadena original` XSLT (`cadenaoriginal_TFD_1_1.xslt`), Servicio de Administración Tributaria (SAT). Referenced by name in the source; no URLs are present.

## License

Apache-2.0. Part of the InvoiceKit workspace; not published independently.
