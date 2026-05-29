# invoicekit-profile-peppol-pint

A profile view that projects the InvoiceKit invoice model onto Peppol PINT (International), stamping the per-country `CustomizationID` and `ProfileID` before handing off to the UBL serialiser.

## What it does

Peppol PINT is OpenPeppol's international invoice profile family. Each member authority publishes its own profile, but they all share the same EN 16931 / UBL Invoice backbone. The practical difference between them is two URN values: the `cbc:CustomizationID` (the country authority's PINT profile identifier) and the `cbc:ProfileID` (the transaction identifier).

This crate takes a single `CommercialDocument` (the shared InvoiceKit intermediate representation), picks the URN pair for a chosen country, and writes the projected document into the UBL document-fields extension so the serialiser emits those two top-level elements with the right values. It does not write XML itself — serialisation is delegated to `invoicekit-format-ubl`.

The projection is deliberately narrow. It upserts `CustomizationID` and `ProfileID` and leaves the rest of the document alone. If the source already carried those elements (for example, a Peppol BIS or XRechnung `CustomizationID`), they are overwritten with the PINT values rather than duplicated.

## Public API

- `PintCountry` — the supported PINT authorities as an enum: `AustraliaNewZealand` (the joint AU/NZ profile), `Singapore`, `Japan`, `UnitedArabEmirates`, `Malaysia`.
  - `PintCountry::from_alpha2(code) -> Option<PintCountry>` — lookup by ISO 3166-1 alpha-2 code. Returns `None` for any country that is not yet a PINT authority here. Both `"AU"` and `"NZ"` map to the joint profile.
  - `customization_id(self) -> &'static str` — the `CustomizationID` URN, e.g. `urn:peppol:pint:billing-1@sg-1`.
  - `profile_id(self) -> &'static str` — the billing transaction `ProfileID`, currently `urn:peppol:bis:billing` for every authority. It is a method (not a constant) so the shape parallels `customization_id`.
- `to_peppol_pint_xml(document, country) -> Result<String, PintError>` — the one operation: project for the given country and serialise to UBL XML.
- `PintError` — `Ubl` (the underlying UBL serialiser rejected the projection) and `Ir` (the IR rejected the projected extension payload).
- `crate_name() -> &'static str` — the canonical package name string.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
                             ^^^^^^^^^^^^^^
```

This is a profile crate. It sits between the shared IR and the UBL serialiser. A caller builds a `CommercialDocument`, chooses a PINT country, and gets UBL XML carrying that country's PINT identifiers. The same intermediate representation feeds the sibling profile crates (`profile-peppol-bis`, `profile-xrechnung`, `profile-factur-x`).

## Usage

```rust
use invoicekit_profile_peppol_pint::{to_peppol_pint_xml, PintCountry};

// `doc` is an already-built invoicekit_ir::CommercialDocument.
let country = PintCountry::from_alpha2("SG").expect("SG is a PINT authority");
let xml = to_peppol_pint_xml(&doc, country)?;

assert!(xml.contains("urn:peppol:pint:billing-1@sg-1"));
assert!(xml.contains("urn:peppol:bis:billing"));
# Ok::<(), invoicekit_profile_peppol_pint::PintError>(())
```

## Status

Workspace member, not published to crates.io. The five strict-gate authorities (AU/NZ joint, SG, JP, AE, MY) are implemented and tested: an integration test projects ten-plus UBL fixtures across all five countries and asserts each result carries the expected `CustomizationID` and `ProfileID`, including the case where a pre-existing BIS or XRechnung `CustomizationID` is replaced.

Honest scope note: this crate handles the URN identification only. It does not enforce country-specific PINT business rules (the per-authority field requirements and constraints), and it does not narrow the document to a country's allowed subset — the projected IR still carries the full document. The `ProfileID` is the same Peppol BIS billing URN for every authority today; if PINT authorities diverge on transaction profiles later, `profile_id` is the place that changes.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
