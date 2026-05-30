# invoicekit-report-fr-ctc — France (DGFiP) CTC / PPF report adapter, Factur-X (EN 16931 CII)

France CTC (Continuous Transaction Control) report adapter for the Chorus Pro / PPF (Portail Public de Facturation) reform. France has **no unique national XML**: the mandate rides the European EN 16931 semantic model, so this crate reuses the existing Factur-X projection and drives the CTC lifecycle around it.

## Coverage level

EN 16931 family path. This crate does **not** mint a bespoke national format. Per the DGFiP "Spécifications Externes Facture Électronique B2B", the French CTC wire carries EN 16931 as Factur-X (the hybrid Cross Industry Invoice syntax). `to_factur_x_xml` delegates to `invoicekit-profile-factur-x` at the `En16931` profile; there is no `report-fr-ctc` XML.

## Capabilities

- **Serialize** — `to_factur_x_xml` projects an `invoicekit_ir::CommercialDocument` to deterministic (byte-stable) Factur-X (EN 16931 profile, CII syntax) XML by delegating to `invoicekit_profile_factur_x::to_factur_x_cii_xml`.
- **Validate (local)** — `validate_siren`, `validate_siret`, and `validate_french_vat` enforce the real French identifier shapes:
  - SIREN: exactly 9 ASCII digits.
  - SIRET: exactly 14 ASCII digits, first 9 a valid SIREN (the trailing 5 are the NIC establishment suffix).
  - French VAT (TVA intracommunautaire): `FR` + a 2-character control key (ASCII digits and/or uppercase letters) + a 9-digit SIREN, 13 characters total.
- **Sign** — `MockFrCtcReportProvider` composes `invoicekit_signer::Signer` for a detached signature over the Factur-X bytes (the qualified-certificate signing leg), keyed by the eIDAS certificate serial.
- **Transmit** — the same provider composes `invoicekit_signer_france_ctc::MockFrCtcProvider`, so the CTC routing and submission-id synthesis is exercised, not re-faked. Routes to the public PPF or a private accredited PDP, across the Piste (sandbox) and Production tiers.
- **Evidence** — the report returns both the typed receipt (`FrCtcReportEnvelope`: submission id, lifecycle verdict, recorded timestamp, detached signature, motif de rejet) and the transmitted Factur-X bytes, for the caller to bundle into a signed `.ikb` evidence bundle.

## Coverage residuals (honest notes)

- **Transmission is offline-only.** The only provider here is `MockFrCtcReportProvider`: deterministic and offline. Live PPF/PDP transmission (Chorus Pro web service or an accredited PDP API) is bring-your-own-credentials and lands in a follow-up `report-fr-ctc-http` crate. This crate ships no real network transport.
- **Reference validation is out-of-process.** Local validation covers French identifier *shapes* only. Reference-grade EN 16931 / CIUS-FR Schematron stays an external JVM backend and is labelled as such in the capability matrix; it is not embedded here.
- **Rejection is a verdict, not an error.** A platform or receiver refusal is surfaced as an `Ok` envelope with `FrCtcLifecycle::Rejected` (carrying the motif de rejet), never as `Err`. `Err` (`FrCtcReportError`) is reserved for pre-wire shape failures (bad identifier, empty payload, signing failure) and transport faults.
- The lifecycle enum is the audit-relevant projection of the signer-layer `FrCtcStatus`: `Deposited` (Déposée), `Received` (Reçue), `Approved` (Approuvée), `Rejected` (Rejetée).

## Public API

- `to_factur_x_xml(&CommercialDocument) -> Result<String, FacturXError>`
- `validate_siren`, `validate_siret`, `validate_french_vat`
- `FrCtcReportProvider` trait; `MockFrCtcReportProvider` (`new`, `with_forced_lifecycle`, `with_rejection_reason`)
- `FrCtcReportRequest`, `FrCtcReport`, `FrCtcReportEnvelope`, `FrCtcReportError`
- `FrCtcLifecycle`, `FrCtcEnvironment`
- Re-exported routing/signing substrate: `FrCtcPlatform`, `FrCtcReceiver`, `Signature`, `QualifiedCertificate`, `QualifiedCertificateId`
- `crate_name()`

## References

- DGFiP "Spécifications Externes Facture Électronique B2B" (the 2026+ French e-invoicing and e-reporting mandate).
- EN 16931 (`urn:cen.eu:en16931:2017`), carried as Factur-X (CII) per the mandate.
- Chorus Pro / PPF (Portail Public de Facturation) and accredited PDP routing.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
