# Mexico CFDI 4.0 PAC signing flow (T-083b2)

`crates/signer-cfdi` ships the typed Provider trait + envelope
+ Mock implementation for **Comprobante Fiscal Digital por
Internet** (CFDI) 4.0 in Mexico. CFDI invoices are sealed by a
**PAC** (Proveedor Autorizado de Certificación) — a SAT-licensed
third party that timbra (stamps) the invoice with a UUID after
validating the seller's *cadena original* signature.

## Why a runbook and not a fully-wired live impl

Live CFDI integration requires:

- A SAT-issued **Certificado de Sello Digital** (CSD) per
  issuer RFC — `.cer` + `.key` files plus passphrase.
- A commercial PAC account (Solución Factible, Edicom, Finkok,
  Diverza, etc.) with API credentials.
- A pinned SAT schema bundle for the CFDI 4.0 release the
  operator targets (including catalogs SAT publishes monthly).

The Mock in `signer-cfdi::MockCfdiPacProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/signer-cfdi/src/lib.rs`:

| Item | Role |
|---|---|
| `CfdiKind` | `Ingreso` / `Egreso` / `Traslado` / `Nomina` / `Pago` / `Retencion` — six SAT-defined invoice categories. |
| `PacEnvironment` | `Sandbox` vs `Production`. PAC vendors expose different base URLs per environment. |
| `CertificadoSelloDigital` | `{ rfc, cer_pem, key_pem, no_certificado }` — the SAT-issued CSD material the engine signs *cadena original* with before handing off to the PAC. |
| `CfdiStampEnvelope` | What `seal` returns: UUID + selloSAT + noCertificadoSAT + fecha_timbrado. |
| `CfdiSignRequest` | What the operator passes in: `{ tenant_id, environment, kind, csd, xml_unsealed }`. |
| `CfdiError` | Typed transport / validation / refusal errors. |
| `CfdiPacProvider` (trait) | `seal(request) -> CfdiStampEnvelope`. |
| `MockCfdiPacProvider` | Deterministic in-memory backend (10 unit tests). |
| `compute_cadena_original` | Standalone helper; produces the canonical pipe-separated string SAT signs over. |
| `wrap_pac_seal` | Standalone helper; embeds the PAC's `Complemento/TimbreFiscalDigital` into the unsealed XML. |

## What the operator does

### 1. Obtain CSD + PAC credentials

- **CSD**: download from the SAT portal (`https://portalsat.plataforma.sat.gob.mx`).
  Two files: a `.cer` (the X.509 certificate) and an
  `.key` (the encrypted private key). PEM-convert both before
  loading into `CertificadoSelloDigital`.
- **PAC**: sign up with a SAT-authorized PAC. Pick whichever
  has the API ergonomics your team prefers — the trait is
  identical across PACs.

The `.key` material is the same secret class as a TLS private
key. Store via OS secret store or KMS.

### 2. Wire `CfdiPacProvider` in the engine

```rust
use invoicekit_signer_cfdi::{CfdiPacProvider, MockCfdiPacProvider, PacEnvironment};

// Today, until the live PR lands:
let provider: Box<dyn CfdiPacProvider> = Box::new(MockCfdiPacProvider::default());

// After the live impl PR lands (in `crates/signer-cfdi-http`):
// let provider = Box::new(invoicekit_signer_cfdi_http::SolucionFactibleProvider::new(
//     PacEnvironment::Production,
//     /* api_user */ "...",
//     /* api_key */ secret,
// ));
```

### 3. Submission flow at runtime

1. Build the unsealed CFDI 4.0 XML (use `crates/format-ubl`
   extended with the SAT namespace, or a future
   `crates/format-cfdi`).
2. Compute the *cadena original* via
   `compute_cadena_original(&unsealed_xml)`.
3. Sign the cadena with the CSD private key (XMLDSig / RSA).
4. Embed the signature into the unsealed XML to produce the
   *XML pre-timbrado*.
5. `provider.seal(&request)` → `CfdiStampEnvelope`. The PAC
   re-validates the cadena + signature, then returns the
   `TimbreFiscalDigital` (UUID + selloSAT).
6. Call `wrap_pac_seal(&pre_timbrado, &envelope)` to produce
   the final cleared XML.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/cfdi-4.0.xml` | Final cleared CFDI XML (post `wrap_pac_seal`). |
| `cfdi/cadena-original.txt` | The canonical string the seller signed. |
| `cfdi/envelope.json` | Serialised `CfdiStampEnvelope` (UUID, selloSAT, fecha_timbrado). |
| `cfdi/csd-public.pem` | Public half of the CSD. |

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-cfdi
```

10 unit tests cover the six `CfdiKind`s, both environments,
cadena hash mismatch handling, and a PAC-side rejection error
shape.

## Validating with the live backend

1. Set `CFDI_PAC_VENDOR=solucion-factible|edicom|finkok|...`,
   `CFDI_PAC_USER`, `CFDI_PAC_KEY`, `CFDI_CSD_RFC`,
   `CFDI_CSD_CER_PATH`, `CFDI_CSD_KEY_PATH`,
   `CFDI_CSD_PASSPHRASE` in the environment.
2. Run the new `tools/cfdi-live-smoke/` binary against the
   PAC's sandbox.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## Status today

- Mock impl: shipped on main (T-083b2, closed).
- Live HTTP impl: open follow-up. The live crate will likely
  ship as `crates/signer-cfdi-http` with one feature flag per
  PAC vendor so operators only pull in the SDK they need.
- Mexico country crate (T-821) tracks the higher-level wiring
  (state machine + cassette set + tax catalog integration) on
  top of this signer.

## References

- SAT CFDI portal: <https://www.sat.gob.mx/personas/iniciar-sesion>
- The shipped Mock + tests: `crates/signer-cfdi/src/lib.rs`.
