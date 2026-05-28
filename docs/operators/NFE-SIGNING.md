# Brazil NF-e federal certificate flow (T-083b5)

`crates/signer-nfe` ships the typed Provider trait + envelope
+ Mock implementation for **Nota Fiscal eletrônica** (NF-e),
Brazil's federal goods-invoice clearance. NF-e clears
per-state at the *Secretaria da Fazenda* (SEFAZ); the engine
signs with an ICP-Brasil A1 certificate, posts to the
state-specific SEFAZ web service, and persists the
authorisation protocol + 44-character `chave de acesso`.

## Why a runbook and not a fully-wired live impl

Live NF-e integration requires:

- An **ICP-Brasil A1** PJ certificate (Pessoa Jurídica) tied
  to the issuer's CNPJ. A1 means software-stored; A3 (smart
  card) is also supported but outside the scope of the
  current trait surface.
- Per-state SEFAZ endpoints. Each Brazilian state operates
  its own SEFAZ; the engine routes by `UfCode`.
- Pinned SEFAZ XSD bundle per release (NF-e 4.00 is the
  current version).

The Mock in `signer-nfe::MockNfeProvider` covers everything
except those operator-side credentials.

## Crate shape (already shipped)

`crates/signer-nfe/src/lib.rs`:

| Item | Role |
|---|---|
| `UfCode` | `SP` / `RJ` / `MG` / `PR` / `RS` / `SC` / `BA` / `DF` / `Other(String)` — Brazilian state codes for SEFAZ routing. |
| `NfeEnvironment` | `Homologacao` (tpAmb=2, sandbox) vs `Producao` (tpAmb=1). |
| `IcpBrasilCertificate` | `{ cnpj, p12_pem, passphrase, certificate_pem }` — the ICP-Brasil-issued material the engine signs with. |
| `NfeStatus` | `Authorized` (cStat=100) / `Denied` / others. Typed mapping of SEFAZ status codes. |
| `NfeStampEnvelope` | What `submit` returns: chave de acesso + protocolo + status + xml authorised. |
| `NfeSubmitRequest` | What the operator passes in. |
| `NfeError` | Typed transport / validation / refusal errors. |
| `NfeProvider` (trait) | `submit(certificate, request) -> NfeStampEnvelope`. |
| `MockNfeProvider` | Deterministic in-memory backend (7 unit tests). |
| `build_chave_acesso` | Standalone helper; builds the 44-character access key per SEFAZ rules (UF + AAMM + CNPJ + mod + série + nNF + tpEmis + cNF + cDV). |
| `cnpj_padded_14` | Standalone helper; pads CNPJ to 14 digits. |
| `nfe_status_descricao` | Standalone helper; maps `cStat` integers to Portuguese descriptions. |

## What the operator does

### 1. Acquire an ICP-Brasil A1 certificate

- Buy from any ICP-Brasil-accredited authority (Certisign,
  Serasa Experian, Soluti, AC SOLUTI, Valid, etc.).
- A1 ships as a `.pfx`/`.p12` after identity verification at
  the CA's office or via video conference (depending on the
  CA's policy).
- PEM-convert and load into `IcpBrasilCertificate`.

Same secret class as a TLS private key. Store via OS secret
store or KMS.

### 2. Wire `NfeProvider` in the engine

```rust
use invoicekit_signer_nfe::{NfeProvider, MockNfeProvider, UfCode, NfeEnvironment};

// Today, until the live PR lands:
let provider: Box<dyn NfeProvider> = Box::new(MockNfeProvider::default());

// After the live impl PR lands:
// let provider = Box::new(invoicekit_signer_nfe_http::HttpNfeProvider::new(
//     /* sefaz_endpoints */ load_state_endpoints(),
//     /* tls_ca_bundle */ trusted_roots,
// ));
```

### 3. Submission flow at runtime

1. Build the NF-e 4.00 XML payload (use a future
   `crates/format-nfe`).
2. Compute the `chave de acesso` via `build_chave_acesso(...)`
   from the invoice's structured fields. Embed in the XML.
3. Sign the XML with the ICP-Brasil certificate via XMLDSig
   enveloped signature.
4. `provider.submit(&certificate, &request)` →
   `NfeStampEnvelope`. The provider routes to the correct
   state SEFAZ endpoint by `UfCode`.
5. Persist the 44-char chave de acesso + the SEFAZ
   `protocolo de autorização`. Both are required to query the
   NF-e portal and to issue a *cancelamento* or *carta de
   correção* later.

### 4. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/nfe-4.00.xml` | Signed + authorised NF-e XML (procNFe). |
| `nfe/chave-acesso.txt` | The 44-char access key. |
| `nfe/envelope.json` | Serialised `NfeStampEnvelope` (status, protocolo, generated_at). |
| `nfe/cert-public.pem` | Public half of the ICP-Brasil certificate. |

## State routing

`UfCode` determines the SEFAZ endpoint. A handful of states
publish their own; the rest fall through to the SVAN
(Sefaz Virtual Ambiente Nacional) or SVRS (Sefaz Virtual Rio
Grande do Sul) consolidations. The live `HttpNfeProvider` will
ship the routing table as a versioned codelist under
`crates/codelists/data/nfe/`.

`UfCode::Other(state)` is a deliberate escape hatch for any
state not in the enum's specific list — pass the two-letter
code (e.g. `"AM"`, `"MT"`).

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-nfe
```

7 unit tests cover both environments, three UF routes,
chave-de-acesso assembly, the typed status descricoes, and
the typed error surface.

## Validating with the live backend

1. Set `NFE_CERT_PATH`, `NFE_CERT_PASSPHRASE`, `NFE_CNPJ`,
   `NFE_ENV=homologacao|producao` in the environment.
2. Run the new `tools/nfe-live-smoke/` binary against the
   homologação SVAN.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## Status today

- Mock impl: shipped on main (T-083b5, closed).
- Live HTTP impl: open follow-up. Belongs in
  `crates/signer-nfe-http`. The live crate adds `reqwest` +
  `rustls` + pinned ICP-Brasil roots.
- Brazil country crate (T-822) tracks the higher-level wiring
  (NF-e + NFS-e + per-state catalog) on top of this signer.

## References

- Portal Nacional da NF-e: <https://www.nfe.fazenda.gov.br>
- Per-state SEFAZ portals are listed under "Documentos" on
  the national portal.
- The shipped Mock + tests: `crates/signer-nfe/src/lib.rs`.
