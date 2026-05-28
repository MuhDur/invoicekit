# Poland KSeF certificate flow (T-083b3)

`crates/signer-ksef` ships the typed Provider trait + envelope
+ Mock implementation for Poland's **Krajowy System e-Faktur**
(KSeF). This runbook captures everything an operator needs to
go from the Mock impl to a live KSeF integration without
re-deriving the protocol surface.

## Why a runbook and not a fully-wired live impl

Wiring a real KSeF backend requires three things the engine
maintainer cannot ship on the operator's behalf:

- A KSeF account on `ksef-test.mf.gov.pl` (demo) or
  `ksef.mf.gov.pl` (production).
- A qualified electronic signature (kwalifikowany podpis
  elektroniczny) or a KSeF-issued authorisation token tied to
  the issuer's NIP.
- A pinned KSeF schema bundle for the FA(2) / FA(3) format
  release the operator targets.

The Mock implementation in `signer-ksef::MockKsefProvider`
covers everything *except* those operator-side credentials, so
unit tests and CI can exercise the full flow deterministically.
This runbook documents the swap-in points so the live impl PR
lands as a single contained change.

## Crate shape (already shipped)

`crates/signer-ksef/src/lib.rs`:

| Item | Role |
|---|---|
| `KsefEnvironment` | `Demo` (sandbox) vs `Production` selector. |
| `AuthMode` | `QualifiedSignature` vs `AuthorisationToken`. |
| `SessionToken` | `{ session_token, expires_at, environment }` for the live HTTP layer. |
| `KsefAcceptance` | `{ numer_ksef, upo_url, accepted_at }` — what production callers carry forward. |
| `KsefStampEnvelope` | What `submit` returns: invoice hash + session token + acceptance. |
| `KsefSubmitRequest` | What the operator passes in: `{ tenant_id, environment, invoice_xml_blake3_hex, auth_mode, nip }`. |
| `KsefError` | Typed transport / validation / refusal errors. |
| `KsefProvider` (trait) | Two methods: `init_session(env, nip, auth) -> SessionToken` and `submit(token, request) -> KsefAcceptance`. |
| `MockKsefProvider` | Deterministic in-memory backend (10 unit tests). |

The trait surface is what the live implementation has to satisfy.

## What the operator does

### 1. Obtain KSeF credentials

- **Demo (`KsefEnvironment::Demo`)** — register at
  `https://ksef-test.mf.gov.pl/web`, link a Polish NIP, and
  generate a sandbox authorisation token. No qualified
  signature required on the demo tier.
- **Production (`KsefEnvironment::Production`)** — register at
  `https://ksef.mf.gov.pl`. The first session must use a
  qualified electronic signature; subsequent sessions can use
  the authorisation token KSeF returns.

Both tokens are short-lived (the API enforces ~2 hours per
session, refreshed via the SDK). Treat the token as a secret
of the same class as a database password.

### 2. Wire `KsefProvider` in your engine code

The engine accepts any `KsefProvider`. Swap the Mock for the
live impl at engine construction time — no other call sites
need to change.

```rust
use invoicekit_signer_ksef::{KsefEnvironment, KsefProvider, MockKsefProvider};

// Until the live HTTP impl PR lands:
let provider: Box<dyn KsefProvider> = Box::new(MockKsefProvider::default());

// After the live HTTP impl PR lands (will live in
// `crates/signer-ksef-http` so the substrate stays pure):
// let provider = Box::new(invoicekit_signer_ksef_http::HttpKsefProvider::new(
//     KsefEnvironment::Production,
//     /* base_url */ "https://ksef.mf.gov.pl/api/v1",
//     /* tls_ca_bundle */ trusted_roots,
// ));
```

### 3. Submission flow at runtime

The flow your engine code runs is fixed by the trait:

1. `provider.init_session(env, nip, auth_mode)` →
   `SessionToken`. Cache by `(env, nip)` until `expires_at`.
2. Build the FA(2)/FA(3) XML payload (use `crates/format-cii`
   downstream of the canonical model).
3. Compute BLAKE3 over the XML and pack into
   `KsefSubmitRequest`.
4. `provider.submit(token, request)` → `KsefAcceptance`. Carry
   the `numer_ksef` (KSeF identifier) + `upo_url` (URL of the
   Urzędowe Poświadczenie Odbioru — official acceptance
   receipt) into the evidence bundle as additional artefacts.

### 4. Evidence bundle artefacts

Wire the result into `invoicekit-evidence::EvidenceBundle.artefacts`:

| Artefact id | Bytes |
|---|---|
| `formats/ksef-fa3.xml` | Submitted FA(3) XML, byte-identical to what KSeF accepted. |
| `ksef/session-token.json` | Serialised `SessionToken` (do not persist the secret beyond bundle assembly; treat the file inside the `.ikb` as sensitive). |
| `ksef/acceptance.json` | Serialised `KsefAcceptance` with the `numer_ksef` + `upo_url`. |
| `ksef/upo.xml` | Fetched UPO XML from `upo_url` once available (production usually returns within ~5 minutes). |

These ids are stable — `invoicekit verify` and
`invoicekit replay` keyed on them in the existing tests.

## Validating with the Mock today

Until the live HTTP impl lands, the full pipeline can still be
exercised via `MockKsefProvider`:

```bash
cargo test -p invoicekit-signer-ksef
```

10 unit tests cover session init, submit, double-init,
mismatched NIP, expired token, and the typed error surface.

## Validating with the live backend

Once the live `HttpKsefProvider` PR lands:

1. Set `KSEF_BASE_URL`, `KSEF_NIP`, `KSEF_AUTH_TOKEN` in the
   environment (never check tokens into the repo).
2. Run the new `tools/ksef-live-smoke/` binary against the
   demo backend. It packs a tiny test invoice, runs
   `init_session` + `submit`, fetches the UPO, and writes
   `dist/ksef-live.ikb`.
3. Verify the bundle round-trips through `invoicekit verify`
   and `invoicekit replay`. Drift here means the API contract
   shifted under the operator (KSeF changes happen — track
   them via the schema watcher in `tools/source-watch-bot`).
4. The KSeF maintenance window (Wednesdays 02:00–06:00 CET)
   produces `KsefError::Transport` rather than `Refused` — the
   trait error vocabulary already maps this correctly.

## Status today

- Mock impl: shipped on main (T-083b3, closed).
- Live HTTP impl: open follow-up. Belongs in a new
  `crates/signer-ksef-http/` crate so the substrate stays
  dependency-free; the live crate adds `reqwest` + `rustls` +
  the pinned KSeF root certificates.
- Bead T-800 (KSeF archetype) tracks the higher-level
  archetype lock-in: state machine + cassette set + reconciliation
  API on top of this signer.

## References

- KSeF developer portal: <https://www.podatki.gov.pl/ksef/>
- FA(3) schema (XSD bundle): pinned in
  `crates/codelists/data/ksef/` once the live impl PR lands.
- The shipped Mock + tests: `crates/signer-ksef/src/lib.rs`.
