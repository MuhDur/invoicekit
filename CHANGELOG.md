# Changelog

All notable changes to InvoiceKit are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it reaches
`1.0`. Until then, minor versions may carry breaking changes.

## [0.2.0] — 2026-05-30

A coverage-and-trust release: more EN 16931 semantic coverage, and the engine's
correctness verified end-to-end against the genuine reference validators.

### Added — EN 16931 coverage
- **Invoice period (BG-14)** and **delivery date (BT-72)** emitted from the IR in
  both UBL and CII.
- **Document-level allowances and charges (BG-20 / BG-21)** and **line-level
  allowances and charges (BG-27 / BG-28)** in both UBL and CII.
- **Deliver-to information (BG-13 / BG-15)** as UBL `cac:Delivery` /
  CII `ram:ShipToTradeParty`.
- **Document references** now emit their typed elements from the IR across both
  serializers: contract (BT-12), purchase order (BT-13), receiving advice
  (BT-15), despatch advice (BT-16), and preceding invoice (BT-25).
- India GST: INV-01 preceding-invoice link (`RefDtls.PrecDocDtls`).

### Changed — correctness & trust
- **CII BT-7 / BT-72 disentanglement.** The tax point date (BT-7) now serializes
  as per-tax `ram:TaxPointDate` (`udt:DateString`), and the actual delivery date
  (BT-72) as `ram:OccurrenceDateTime` — previously conflated.
- **Battle-tested against real reference validators.** The pure-Rust EN 16931
  validator's `BR-*` / `BR-CO-*` findings match the live KoSIT XRechnung 3.0.2 and
  phive Peppol BIS 3.0 oracles across 722 corpus fixtures (core parity 1.0).
  Factur-X output clears the real veraPDF 1.30.1 verifier at PDF/A-3b **and** 3u
  (30/30). Fuzzing (5 targets) and a Miri check of the C ABI are clean.
- **Documentation honesty.** All crates and tool directories documented and ~25
  doc overclaims corrected; the README now states the real signing/timestamping
  maturity (production-grade BLAKE3 artefact integrity; placeholder software
  signer and mock TSA, with real crypto/TSA a planned track) and the real binding
  maturity (working Go/Java/.NET/Python byte-contract packages; Node and
  WebAssembly are byte-contract stubs).

### Fixed
- Several audit-surfaced serialization bugs: BG-20/21 (6), BG-27/28, the UBL
  `cac:OrderReference` 0..1 both-set double-emit, two CII line-settlement preserve
  bugs, and the `udt:Indicator` parse path.
- The EN 16931 differential parity harness no longer counts national CIUS rules
  (e.g. XRechnung `BR-DE-*`) as core EN 16931, which had produced false parity
  failures on the public dashboard.

## [0.1.1] — 2026-05-29

First **complete** tagged release. Supersedes `v0.1.0`, whose cross-platform
binary jobs were blocked by a `cargo-deny` wildcard-path lint (intra-workspace
`path` dependencies carry no version requirement); fixed via
`allow-wildcard-paths = true`. Content is otherwise identical to `0.1.0`.

## [0.1.0] — 2026-05-29

First tagged release. The Rust engine and the full B2B e-invoicing lifecycle
(create → check → render → read → send → archive) are in place, tested, and
honest about their maturity. Apache-2.0 throughout. `unsafe` forbidden at the
lint level (the single documented exception is the C ABI crate).

### Engine & core
- Deterministic Rust engine with a stable JSON ABI (`invoicekit-engine`), native
  bindings scaffolding (Node/Python/.NET/Java/Go) and a browser/edge WebAssembly
  artifact built from the same engine.
- Layered invoice model (`ir`): a jurisdiction-agnostic commercial document with
  profile views and typed jurisdiction extensions.
- Money, tax, and code lists as first-class crates — fixed-scale decimals at every
  boundary, never floating point.
- Signed, effective-dated rule packs; `validate --date=YYYY-MM-DD` selects the rule
  pack in force on a date.
- Byte-stable canonical serialization (JSON and XML C14N).

### Formats & profiles
- UBL 2.1 and Cross Industry Invoice (CII D16B) serializers.
- Profile projections: Peppol BIS 3.0, Peppol PINT, XRechnung 3.x, Factur-X
  (six profiles).
- National formats with real serializers: FatturaPA (Italy), CFDI 4.0 (Mexico),
  NF-e (Brazil), KSeF FA(3) (Poland).

### Country coverage (honest, local-only end-to-end)
- 34 national report adapters across Europe, Latin America, Asia-Pacific, MENA, and
  Africa, each with an offline end-to-end lifecycle test: build → serialize →
  local validate → sign/transmit (deterministic offline mock) → signed evidence
  bundle → verify.
- Each national clearance adapter composes a dedicated signer crate (SDI, ZATCA,
  KSeF, CFDI, NF-e, France CTC, eIDAS) and encodes the country's real identifiers
  (e.g. Partita IVA/Codice Fiscale, NIP, RFC, CNPJ, SIREN/SIRET) and receipt shapes.
  A clearance rejection is a receipt state, never an error.
- The capability matrix advertises every supported country with explicit, honest
  per-capability maturity (serialize / local validate / reference validate),
  transport, source provenance, and confidence. Ask the binary:
  `invoicekit capabilities --from=<CC> --to=<CC> --date=YYYY-MM-DD --scenario=B2B`.

### Render, intake, evidence
- Deterministic PDF/A-3 rendering (Typst) with embedded machine-readable data;
  verified against veraPDF (`3b` + `3u`) as a release gate.
- Inbound reading from digital PDFs, scans (OCR), and XML with field-level
  provenance (bounding-box citations).
- Every operation can emit a signed `.ikb` evidence bundle (canonical data,
  generated artifacts, validation trace, signatures, RFC 3161 timestamp);
  `invoicekit verify` checks it without executing any shell scripts.

### Tooling
- `invoicekit` CLI: validate, pack, unpack, verify, show, diff, replay, timestamp,
  capabilities, doctor, repl, migrate-archive, codelist-update, version.
- REST shim with a generated OpenAPI 3.1 document.

### Known limitations (honest)
- **Reference-grade validation needs a JVM** worker (KoSIT/phive/Saxon) — by design;
  the pure-Rust checker covers common rules, the conformance path calls the worker.
- **Live Peppol delivery is bring-your-own-credentials**; native AS4 transport is a
  research track. Offline/sandbox transmission is deterministic.
- **National-format serialization** is implemented natively for the flagship
  countries (IT/MX/BR/PL); other countries serialize the EN 16931 / UBL
  representation today, with native national serializers tracked as follow-ups.
- **Inbound right-to-left and CJK vertical scripts** remain a documented gap in the
  digital-PDF intake path.

[0.1.1]: https://github.com/MuhDur/invoicekit/releases/tag/v0.1.1
[0.1.0]: https://github.com/MuhDur/invoicekit/releases/tag/v0.1.0
