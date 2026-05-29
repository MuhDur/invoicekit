# invoicekit-profile-factur-x

A profile view of the InvoiceKit invoice model for the six Factur-X / ZUGFeRD profiles, with a lossiness ledger that explains what downgrading drops.

## What it does

Factur-X (the French name) and ZUGFeRD (the German name) are the same hybrid PDF/A-3 invoice standard: a human-readable PDF carrying embedded Cross Industry Invoice (CII D16B) XML. The standard publishes six profiles, from MINIMUM (header data only) up to EXTENDED and the German B2G XRECHNUNG profile. Each profile carries a different subset of the invoice and is identified by a canonical guideline URN that ZUGFeRD validators look for.

This crate projects a single `CommercialDocument` (the shared InvoiceKit intermediate representation) onto a chosen profile. It does two things:

1. Stamps the projected document with the target profile's guideline URN, so the CII serialiser emits the correct `GuidelineSpecifiedDocumentContextParameter`.
2. Returns a `LossinessLedger` recording every field the target profile keeps (`preserved`) and every field it cannot carry (`lost`).

It does not write CII XML itself. Serialisation is delegated to `invoicekit-format-cii`; this crate decides the profile and accounts for the loss.

One honest caveat, straight from the source: the projected IR still holds the full document. Profile narrowing for MINIMUM and BASIC WL (which omit lines, tax summary, and notes) is recorded in the ledger as lost at *write time*, but the underlying CII serialiser does not yet drop those elements based on profile context. That follow-up lives in `invoicekit-format-cii`. The exception is per-line extensions, which this crate does physically clear for every profile except EXTENDED.

## Public API

- `FacturXProfile` — the six profiles as an enum (`Minimum`, `BasicWl`, `Basic`, `En16931`, `Extended`, `Xrechnung`), ordered least to most expressive so comparison detects downgrade direction. Helpers: `all()`, `name()`, `guideline_urn()`, `carries_lines()`, `carries_line_allowances()`, `requires_leitweg_id()`.
- `project(source, target) -> Result<ProjectedDocument, FacturXError>` — the core operation. Stamps the guideline URN and builds the ledger.
- `ProjectedDocument` — holds the projected `document` and its `ledger`.
- `downgrade(source, source_profile, target)` — `project` with a guard that the target is strictly less expressive than the source.
- `upgrade(source, source_profile, target)` — `project` with a guard that the target is strictly more expressive. Upgrades are lossless: the ledger reports `preserved` entries only.
- `to_factur_x_cii_xml(source, target) -> Result<String, FacturXError>` — project and serialise to CII XML in one call, discarding the ledger.
- `FacturXError` — `Ir`, `Cii`, and `ProfileGuard` (named profile rejected the input, e.g. XRECHNUNG without a Leitweg-ID, or a downgrade pointed the wrong direction).
- `crate_name()` — the canonical package name string.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
                              ^^^^^^^^^^^^^^
```

This is a profile crate. It sits between the shared IR and the CII serialiser. A caller takes a `CommercialDocument`, picks a Factur-X profile, and gets back IR ready for `invoicekit-format-cii` to write, plus a ledger to attach as evidence of what the format conversion cost. The same intermediate representation feeds the other profile crates (`profile-peppol-bis`, `profile-peppol-pint`, `profile-xrechnung`).

The guideline URNs are sourced from the Factur-X 1.0, ZUGFeRD 2.1, and XRechnung 3.0.2 specifications.

## Usage

Project to a profile and inspect what the conversion preserved or dropped:

```rust
use invoicekit_profile_factur_x::{project, FacturXProfile};

// `doc` is an already-built invoicekit_ir::CommercialDocument.
let projected = project(&doc, FacturXProfile::Basic)?;

for entry in &projected.ledger.lost {
    eprintln!("dropped {}: {}", entry.path, entry.reason);
}

// projected.document is ready for invoicekit_format_cii::to_xml.
# Ok::<(), invoicekit_profile_factur_x::FacturXError>(())
```

Project and serialise in one step:

```rust
use invoicekit_profile_factur_x::{to_factur_x_cii_xml, FacturXProfile};

let xml = to_factur_x_cii_xml(&doc, FacturXProfile::En16931)?;
assert!(xml.contains("urn:cen.eu:en16931:2017"));
# Ok::<(), invoicekit_profile_factur_x::FacturXError>(())
```

XRECHNUNG enforces a hard requirement: the customer party must carry a Leitweg-ID (BT-10 / `BuyerReference`). Projecting without one returns `FacturXError::ProfileGuard`.

## Status

Workspace member, not published to crates.io. The profile model, the lossiness ledger, and the guideline-URN stamping are implemented and tested across all six profiles (each has a valid-projection and an invalid-input case). The known scaffold edge is noted above: profile-aware element omission for MINIMUM / BASIC WL is recorded in the ledger but not yet enforced in the CII writer.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
