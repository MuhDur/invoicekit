# Italy SDI / Aruba qualified certificate flow (T-083b4)

`crates/signer-sdi` ships the typed Provider trait + envelope
+ Mock implementation for **Sistema di Interscambio** (SDI),
Italy's clearance exchange for B2B/B2G e-invoicing. The path
the engine implements: sign the FatturaPA XML with an Aruba
qualified certificate, then ship to SDI via the Web Service
(SDIVI / SDICoop) or certified email (PEC) transport.

## Why a runbook and not a fully-wired live impl

Live SDI integration requires:

- An Aruba **firma qualificata** (qualified electronic
  signature) certificate — typically a `.p12` bundle issued
  after identity verification (in person or via SPID).
- A PEC mailbox (for the PEC transport) OR an
  *Accreditamento SDICoop* approval from Agenzia delle Entrate
  (for the Web Service transport).
- The SDI codice destinatario `XXXXXXX` for the receiver.

The Mock in `signer-sdi::MockSdiProvider` covers everything
except those operator-side credentials.

## Crate shape (already shipped)

`crates/signer-sdi/src/lib.rs`:

| Item | Role |
|---|---|
| `SdiTransport` | `WebService` (SDICoop, the modern path) vs `Pec` (legacy email path). |
| `SdiReceiptKind` | `RC` (RicevutaConsegna) / `NS` (NotificaScarto) / `MC` (MancataConsegna) / `NE` (NotificaEsito) / `MT` (NotificaMancataConsegna). Five SDI receipt types. |
| `ArubaQualifiedCertificate` | `{ subject_dn, p12_pem, key_passphrase }` — the Aruba-issued material the engine signs FatturaPA with. |
| `SdiStampEnvelope` | What `submit` returns: identificativo SDI + receipt kind + receipt XML. |
| `SdiSubmitRequest` | What the operator passes in. |
| `SdiError` | Typed transport / validation / refusal errors. |
| `SdiProvider` (trait) | `submit(certificate, request) -> SdiStampEnvelope`. |
| `MockSdiProvider` | Deterministic in-memory backend (10 unit tests). |

## What the operator does

### 1. Acquire an Aruba qualified certificate

- Order from `https://www.pec.it/firma-digitale.aspx` (or
  another qualified trust service provider that issues for
  Italian SDI use).
- Complete the identity-verification step (in-person, SPID,
  or video). Aruba ships the `.p12` after verification.
- PEM-convert the `.p12` (preserving the encrypted key) and
  load into `ArubaQualifiedCertificate`.

The certificate is the same secret class as a TLS private key.
Store via OS secret store or KMS.

### 2. Pick a transport

- **Web Service (SDICoop)**: SOAP over mTLS to
  `https://servizi.fatturapa.it/...`. Requires SDICoop
  accreditation from Agenzia delle Entrate.
- **PEC**: send the signed XML as an attachment to
  `sdi01@pec.fatturapa.it` (production) or
  `sdi01@pec-test.fatturapa.it` (test). Lower bar — any PEC
  mailbox works — but slower turnaround.

### 3. Wire `SdiProvider` in the engine

```rust
use invoicekit_signer_sdi::{SdiProvider, MockSdiProvider, SdiTransport};

// Today, until the live PR lands:
let provider: Box<dyn SdiProvider> = Box::new(MockSdiProvider::default());

// After the live impl PR lands (likely two crates, one per
// transport, so PEC-only operators don't pay for the SOAP
// stack):
// let provider = Box::new(invoicekit_signer_sdi_ws::SdiWsProvider::new(...));
// let provider = Box::new(invoicekit_signer_sdi_pec::SdiPecProvider::new(...));
```

### 4. Submission flow at runtime

1. Build the FatturaPA XML (versione 1.2.x). Use a future
   `crates/format-fatturapa` (downstream of canonical).
2. Sign with the Aruba certificate via XAdES-BES enveloped
   signature.
3. `provider.submit(&certificate, &request)` →
   `SdiStampEnvelope`. The transport handles the
   re-validation + transmission.
4. Persist the `IdentificativoSdI` from the envelope; SDI
   responses arrive asynchronously (within ~5 days legally,
   usually within minutes). Subscribe to the relevant receipt
   kinds in the engine state machine.

### 5. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/fatturapa.xml` | Signed FatturaPA XML. |
| `sdi/envelope.json` | Serialised `SdiStampEnvelope` with identificativo + receipt kind. |
| `sdi/receipt.xml` | The SDI receipt XML (RC/NS/MC/NE/MT). |
| `sdi/aruba-cert-public.pem` | Public half of the Aruba certificate. |

## Receipt kind semantics

| Kind | Meaning | Engine action |
|---|---|---|
| `RC` | Successful delivery to the receiver. | Mark `Accepted`. |
| `NS` | Schema rejection. | Mark `Failed`; the invoice never reached the receiver. Regenerate + resubmit. |
| `MC` | SDI couldn't deliver (receiver code lookup failed). | Mark `Pending`; SDI retries for 15 days. |
| `NE` | Receiver-side accept/reject decision (only B2G — public administration). | Mark `Accepted` / `Rejected` based on the inner decision. |
| `MT` | Definitive non-delivery after retries. | Mark `Failed`; deliver out of band. |

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-sdi
```

10 unit tests cover both transports, all five receipt kinds,
certificate validation, and the typed error surface.

## Validating with the live backend

1. Set `SDI_TRANSPORT=ws|pec`, `SDI_ARUBA_P12_PATH`,
   `SDI_ARUBA_PASSPHRASE`, plus the transport-specific creds.
2. Run the new `tools/sdi-live-smoke/` binary against the
   test environment.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## Status today

- Mock impl: shipped on main (T-083b4, closed).
- Live impl: open follow-up. Two crates expected — one per
  transport.
- Italy country crate (T-810) tracks the higher-level wiring
  (state machine, esterometro reporting cassette set, codice
  destinatario lookup) on top of this signer.

## References

- Agenzia delle Entrate SDI portal:
  <https://www.fatturapa.gov.it>
- The shipped Mock + tests: `crates/signer-sdi/src/lib.rs`.
