# invoicekit-profile-peppol-bis

Project the InvoiceKit core invoice into the Peppol BIS Billing 3.0 UBL profile.

## What it does

Peppol BIS Billing 3.0 is a customization (a CIUS) of EN 16931 carried as UBL.
Compared to a plain EN 16931 / UBL invoice, the only thing this profile changes
on the wire is two header URNs: a fixed `cbc:CustomizationID` and a fixed
`cbc:ProfileID`. Those two values are what the reference validator dispatches its
`BR-*` and `PEPPOL-*` Schematron rules from.

This crate does exactly that and nothing more. It takes a `CommercialDocument`,
sets (or overwrites) the Peppol customization and profile IDs in the UBL
document-fields extension, leaves every other field untouched, and serializes the
result to UBL XML. Unlike XRechnung, Peppol BIS does not require a Leitweg-ID
`BuyerReference`, so the projection does not add one.

It is a thin, single-responsibility view over the core invoice. It is not a
validator, a renderer, or a transmitter.

## Public API

- `to_peppol_bis_3_0_xml(document: &CommercialDocument) -> Result<String, PeppolBisError>`
  — the entry point. Projects the document into the Peppol BIS header URNs and
  returns UBL XML.
- `PEPPOL_BIS_3_0_CUSTOMIZATION_ID` — the `CustomizationID` URN published by
  OpenPeppol for BIS Billing 3.0.
- `PEPPOL_BIS_3_0_PROFILE_ID` — the `ProfileID` URN for the billing transaction.
- `PeppolBisError` — wraps a failure from the UBL serializer (`Ubl`) or from the
  intermediate representation (`Ir`).
- `crate_name()` — the package name, for operator logs.

The actual XML serialization is delegated to `invoicekit-format-ubl`; this crate
only manages the customization/profile header fields on top of it.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> [format/profile] -> validate -> render/intake -> transmit -> evidence
```

This is a **profile** crate. It sits beside `format-ubl` in the format/profile
stage. The usual flow: an invoice arrives as `CommercialDocument` (parsed from
UBL via `invoicekit-format-ubl::from_xml`, or built from the intermediate
representation), this crate stamps it with the Peppol BIS Billing 3.0 URNs, and
the resulting XML goes on to validation (the phive-based validator worker, which
selects the Peppol BIS rule set from the `CustomizationID`) and then transmission.

## Usage

Project a UBL document that is already in memory:

```rust
use invoicekit_format_ubl::from_xml;
use invoicekit_profile_peppol_bis::{
    to_peppol_bis_3_0_xml, PEPPOL_BIS_3_0_CUSTOMIZATION_ID,
};

let (document, _ledger) = from_xml(ubl_xml)?;
let peppol_xml = to_peppol_bis_3_0_xml(&document)?;

assert!(peppol_xml.contains(PEPPOL_BIS_3_0_CUSTOMIZATION_ID));
# Ok::<(), Box<dyn std::error::Error>>(())
```

If the input already carried a different customization (for example an XRechnung
URN), the projection replaces it with the Peppol BIS one.

## Status

The projection and its URN constants are real and exercised against the synthetic
UBL conformance corpus (the integration test runs ≥20 cross-border fixtures
through `to_peppol_bis_3_0_xml` and checks the headers land). Scope is deliberately
narrow: it only touches `cbc:CustomizationID` and `cbc:ProfileID`. Rule
conformance itself is the validator worker's job, not this crate's.

## License

Apache-2.0. Part of the InvoiceKit workspace.
