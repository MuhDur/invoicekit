# France CTC (PPF / PDP) signing-and-routing flow

`crates/signer-france-ctc` ships the typed Provider trait +
envelope + Mock implementation for France's **Continuous
Transaction Control** mandate (CTC). Every B2B invoice transits
through either the public **PPF** (Portail Public de
Facturation, the government's free Chorus Pro-derived
platform) or a private **PDP** (Plateforme de
Dématérialisation Partenaire — a private partner accredited
by the Direction Générale des Finances Publiques, DGFiP).

The engine signs the invoice with an EU qualified certificate
(the same path as eIDAS T-083a), routes via the chosen
PDP/PPF, then receives status callbacks: submitted →
deposited → received → approved / rejected.

## Why a runbook and not a fully-wired live impl

Live CTC integration requires:

- A **PPF account** or a **PDP contract** with an accredited
  partner. The DGFiP publishes the accredited-PDP list at
  <https://www.economie.gouv.fr/cedef/facturation-electronique-entreprises>.
- An **EU qualified certificate** (issued by an EU/EEA-listed
  QTSP) for the signing leg — see
  [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md). For France, common
  QTSPs include Certigna, ChamberSign, DocuSign France, and
  CDC ARKHINEO.
- PISTE sandbox credentials for `piste.gouv.fr` (the test
  tier), or production credentials for `chorus-pro.gouv.fr`.

The Mock in `signer-france-ctc::MockFrCtcProvider` covers
everything except those operator-side credentials.

## Crate shape (already shipped)

`crates/signer-france-ctc/src/lib.rs`:

| Item | Role |
|---|---|
| `FrCtcPlatform` | `Ppf` vs `Pdp { siret }` — routing selector. The PDP variant carries the partner's SIRET so cassettes pin which platform a test recorded against. |
| `FrCtcEnvironment` | `Piste` (sandbox) vs `Production`. |
| `FrCtcReceiver` | `Siret(14)` / `Siren(9)` / `Annuaire(...)` — receiver lookup keys. |
| `FrCtcStatus` | Typed lifecycle: `Submitted` → `Deposited` → `Received` → `Approved` / `Rejected` / `Suspended`. |
| `FrCtcStampEnvelope` | What `submit` returns: submission id, observed status, RFC-3339 timestamp, optional motif de rejet. |
| `FrCtcSubmitRequest` | What the operator passes in: tenant + environment + platform + receiver + canonical XML payload. |
| `FrCtcError` | Typed errors: `BadXml`, `SigningFailure`, `PlatformRejection { motif, detail }`, `Transport`. |
| `FrCtcProvider` (trait) | `submit(certificate, request)` + `poll_status(env, platform, submission_id)`. |
| `MockFrCtcProvider` | Deterministic in-memory backend (12 unit tests). |
| `validate_siret(s)` | Standalone helper that confirms a SIRET is 14 ASCII digits. |

## What the operator does

### 1. Choose PPF or a PDP

- **PPF** is free. Latency and feature ergonomics are
  whatever Chorus Pro / AIFE delivers. Default choice for
  low-volume issuers.
- **PDP** vendors compete on UX, SLA, integrations (ERP
  connectors), and supplementary reporting. The DGFiP
  enforces interoperability — a receiver on PDP-A can still
  accept an invoice routed via PDP-B or PPF.

### 2. Acquire the qualified certificate

Follow the same path as [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md).
For France, the certificate's `notAfter` should comfortably
exceed the invoice's `BT-2 IssueDate` plus the legal retention
window (10 years for French commercial invoices). Renew on
the calendar, not on first-failure-in-production.

### 3. Wire `FrCtcProvider` in the engine

```rust
use invoicekit_signer_france_ctc::{
    FrCtcEnvironment, FrCtcPlatform, FrCtcProvider, MockFrCtcProvider,
};

// Today, until the live impl PR lands:
let provider: Box<dyn FrCtcProvider> = Box::new(MockFrCtcProvider::default());

// After the live impl PRs land (one crate per backend):
// let provider = Box::new(invoicekit_signer_france_ctc_ppf::PpfProvider::new(
//     FrCtcEnvironment::Production,
//     /* api_user */ "...",
//     /* api_key */ secret,
// ));
// // or for a PDP vendor:
// let provider = Box::new(invoicekit_signer_france_ctc_acme::AcmePdpProvider::new(...));
```

### 4. Submission flow at runtime

1. Build the canonical UBL or CII XML (use `crates/format-ubl`
   or `crates/format-cii`).
2. Resolve the receiver:
   - SIRET if the receiver's billing entity is known.
   - SIREN when the legal entity is known but not the
     specific establishment.
   - Annuaire id when the receiver opted into a specific PDP
     and exposed it via the public directory.
3. `provider.submit(&qualified_certificate, &request)` →
   `FrCtcStampEnvelope`. Persist the `submission_id` for
   reconciliation.
4. Run `provider.poll_status(...)` from the engine's
   reconciliation loop until the status reaches `Approved`,
   `Rejected`, or `Suspended`. PPF/PDP callbacks also push
   updates — wire those through the engine's webhook
   handler when the live PR lands.

### 5. Evidence bundle artefacts

| Artefact id | Bytes |
|---|---|
| `formats/fr-ctc.xml` | Signed canonical UBL/CII XML, byte-identical to what PPF/PDP accepted. |
| `fr-ctc/envelope.json` | Serialised `FrCtcStampEnvelope` (submission id + observed status + stamped_at + rejection_reason). |
| `fr-ctc/cert-public.pem` | Public half of the qualified certificate (the private key never enters the bundle). |
| `fr-ctc/status-history.json` | Append-only log of every status transition the engine observed; useful for audit when a receiver disputes the lifecycle. |

## Receiver routing semantics

The PDP/PPF receiver directory resolves which platform the
buyer's invoice inbox lives on. The engine submits to the
**issuer's** platform; the platform handles cross-PDP routing
to the receiver. From the engine's perspective:

- `Annuaire(...)` is the most precise: the receiver
  pre-declared the PDP they accept invoices through.
- `Siret(...)` is fine when the receiver hasn't opted into a
  specific PDP — the public directory resolves to whichever
  platform the receiver registered.
- `Siren(...)` is a last resort when only the legal entity is
  known. The platform may reject with a typed
  `PlatformRejection { motif: "RECEIVER_RESOLUTION", ... }`
  when no establishment can be selected automatically.

## `Rejected` motif handling

When `FrCtcStampEnvelope.status == FrCtcStatus::Rejected`,
the platform supplies a `rejection_reason` string and the
engine surfaces it as a `PlatformRejection { motif, detail }`
on the next `poll_status`. Common DGFiP motifs:

| Motif | Meaning | Operator action |
|---|---|---|
| `NOMENCLATURE` | Code list value missing / wrong (UNCL5305 / VAT category / etc.). | Re-validate against the engine's codelists; fix and resubmit. |
| `SIGNATURE_INVALIDE` | Qualified-cert signature failed re-check. | Verify cert chain freshness; check the QSCD didn't rotate keys mid-submission. |
| `STRUCTURE` | Schema validation failed (BR-* rule). | Re-validate locally before resubmit. |
| `RECEIVER_RESOLUTION` | Receiver lookup ambiguous. | Switch to `Annuaire(...)` or supply a more specific SIRET. |
| `DOUBLON` | Duplicate submission id detected. | Reconcile against the original; this is the engine's idempotency key working. |

## Validating with the Mock

```bash
cargo test -p invoicekit-signer-france-ctc
```

12 unit tests cover both platforms, both environments, serial
submission ids, payload validation, `poll_status` happy +
error paths, SIRET validation (length + digits), and serde
round-trips for both platform variants + the `Rejected`
envelope shape.

## Validating with the live backend

1. Set `FR_CTC_PLATFORM=ppf|pdp:<siret>`,
   `FR_CTC_ENV=piste|production`,
   `FR_CTC_API_USER`, `FR_CTC_API_KEY`,
   `FR_CTC_CERT_PATH`, `FR_CTC_CERT_PASSPHRASE` in the
   environment.
2. Run the new `tools/fr-ctc-live-smoke/` binary against
   PISTE.
3. Verify the produced bundle round-trips through
   `invoicekit verify` and `invoicekit replay`.

## Status today

- Mock impl: shipped on main (commit `fe2b626`).
- Live HTTP impls: open follow-ups. One crate per backend —
  `crates/signer-france-ctc-ppf/` for the public portal,
  plus one per accredited PDP vendor the principal contracts
  with.
- Bead T-811 (Country crate: France PA-PDP) tracks the
  higher-level country adapter (state machine + UBL/CII
  serializer + reconciliation API) on top of this signer.

## References

- DGFiP CTC portal: <https://www.economie.gouv.fr/cedef/facturation-electronique-entreprises>
- AIFE PISTE sandbox: <https://piste.gouv.fr>
- Chorus Pro production: <https://chorus-pro.gouv.fr>
- DGFiP "Spécifications Externes Facture Électronique B2B"
  spec (PDF, downloadable from the CTC portal above) — pinned
  per CTC version in the live PR's docs.
- The shipped Mock + tests: `crates/signer-france-ctc/src/lib.rs`.
- Related runbook for the qualified-signature path:
  [`EIDAS-SIGNING.md`](./EIDAS-SIGNING.md).
