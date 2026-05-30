# invoicekit-signer-france-ctc — France CTC signing-and-routing surface (XAdES-BES over an EU qualified certificate)

Typed signing-and-routing adapter for France's Continuous Transaction Control (CTC) mandate: sign an invoice with an EU qualified certificate, route it through the public PPF (Portail Public de Facturation) or a private accredited PDP (Plateforme de Dématérialisation Partenaire), and observe the CTC lifecycle. The signing leg shares the path with the eIDAS adapter (T-083a).

## Capabilities

- **Typed surface** — `FrCtcProvider` trait with `submit` (sign + route one invoice) and `poll_status` (fetch the latest lifecycle state of a prior submission).
- **Routing model** — `FrCtcPlatform` selects the public `Ppf` or a private `Pdp { siret }`; `FrCtcEnvironment` selects the `Piste` (sandbox) or `Production` tier. The operator picks both at engine-construction time.
- **Receiver lookup** — `FrCtcReceiver` carries a `Siret`, `Siren`, or `Annuaire` directory identifier.
- **Lifecycle** — `FrCtcStatus` mirrors the DGFiP "cycle de vie": `Submitted`, `Deposited`, `Received`, `Approved`, `Rejected`, `Suspended`.
- **Receipt** — `submit` returns an `FrCtcStampEnvelope` (platform submission id, latest observed status, RFC-3339 UTC timestamp, optional motif-de-rejet string when `Rejected`).
- **Local pre-flight** — `validate_siret` enforces exactly 14 ASCII digits before going to the wire.
- **Typed errors** — `FrCtcError` distinguishes `BadXml`, `SigningFailure`, `PlatformRejection { motif, detail }`, and `Transport`.

The contract is that the operator submits canonical UBL or CII XML and does **not** pre-sign: the provider computes its own hash and signs with the supplied `QualifiedCertificate` (the type from `invoicekit-signer-eidas`, which a caller depends on directly).

## Mode

**Mock-only.** This crate ships the typed surface plus `MockFrCtcProvider`, a deterministic, offline provider for cassette-replay tests: `submit` returns `Deposited` (after a minimal well-formed-XML sanity check), `poll_status` returns `Approved`, with fixed timestamps and serial submission ids prefixed by tier and platform (`PISTE-PPF-`, `PPF-`, `PISTE-PDP-`, `PDP-`). No real cryptographic signing and no network transport happen here — the mock ignores the certificate's key material entirely.

The trait documents the real contract a live provider must satisfy: compute the canonical hash, sign it with the qualified certificate as **XAdES-BES enveloped** per the DGFiP specification, then POST the signed payload to the PPF/PDP endpoint selected by platform + environment. A live path needs an EU qualified certificate held in a Qualified Signature Creation Device (the certificate's PIN/QSCD policy can refuse, surfaced as `SigningFailure`), plus accredited PPF/PDP endpoint credentials. Those real integrations land behind feature flags in follow-up crates (`signer-france-ctc-ppf` for the public portal, `signer-france-ctc-<vendor>` per accredited PDP); none ship here.

## Residuals

- No real signer and no network transport in this crate — only the deterministic mock. Real PDP/PPF integrations are deferred to follow-up crates.
- Signing key material is not exercised: `MockFrCtcProvider::submit` accepts the `QualifiedCertificate` but does not use it.
- Local validation covers the SIRET shape (14 ASCII digits) only; there is no checksum/registry validation and no reference-grade invoice validation here.

## References

- France DGFiP "Spécifications Externes Facture Électronique B2B" v2.x (the 2026+ CTC mandate; defines the PPF/PDP routing, the lifecycle states, and the motif-de-rejet vocabulary).

## License

Apache-2.0. Part of the InvoiceKit workspace.
