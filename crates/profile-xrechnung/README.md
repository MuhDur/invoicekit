# invoicekit-profile-xrechnung

Projects a core InvoiceKit invoice onto German XRechnung 3.x — the Universal Business Language (UBL) form used for German business-to-government invoicing.

## What it does

XRechnung is the German Core Invoice Usage Specification (CIUS) built on top of EN 16931 and Peppol BIS Billing 3.0. It does not invent a new XML shape. Compared to a plain EN 16931 UBL invoice it pins three things at the document header:

1. A fixed `CustomizationID` URN (`XRECHNUNG_3_CUSTOMIZATION_ID`) that the German validator, KoSIT, uses to pick its CIUS-DE rule scenarios.
2. A fixed `ProfileID` URN (`XRECHNUNG_PROFILE_ID`) — the same Peppol BIS Billing 3.0 transaction profile XRechnung reuses.
3. A `BuyerReference` carrying the Leitweg-ID, which is mandatory for invoices sent to German federal, state, or municipal authorities (rule BR-DE-15).

This crate is deliberately small. The UBL serializer in `invoicekit-format-ubl` already emits EN 16931-compliant XML. The projection just injects those three overrides into the UBL document-fields extension and then delegates to `invoicekit_format_ubl::to_xml`. The input document is cloned, not mutated.

It does not run the KoSIT validator and does not write UBL XML itself. It decides what makes an invoice XRechnung-shaped and hands the rest off.

## Public API

- `to_xrechnung_3_x_xml(document, options) -> Result<String, XRechnungError>` — the one operation. Takes a `&CommercialDocument` and `&XRechnungOptions`, returns XRechnung 3.x UBL XML.
- `XRechnungOptions` — currently one field, `leitweg_id: Option<String>`. With a Leitweg-ID the projection writes the B2G `BuyerReference`; without one it treats the document as business-to-business and leaves whatever `BuyerReference` the upstream document already carried.
- `XRechnungError` — `Ubl` (the serializer rejected the projection), `Ir` (the intermediate representation refused the projected extension payload), `MissingLeitwegId` (empty Leitweg-ID supplied), and `InvalidLeitwegId` (failed the shape check).
- `XRECHNUNG_3_CUSTOMIZATION_ID`, `XRECHNUNG_PROFILE_ID` — the two frozen URN constants.
- `coverage::BR_DE_COVERAGE` — a hand-maintained matrix of the German BR-DE-* business rules, each row (`BrDeRow`) recording whether the rule is enforced structurally by this projection (`rust_enforced`) or left to KoSIT at runtime (`kosit_enforced`). Helpers: `coverage::br_de_row_count()`, `coverage::br_de_rust_enforced_count()`.
- `crate_name()` — the canonical package name string.

The Leitweg-ID check is a shape check only: non-empty, ASCII alphanumeric with `-` separators. The checksum digit is left to the KoSIT validator.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
                              ^^^^^^^^^^^^^^
```

This is a profile crate. It sits between the shared IR (`invoicekit-ir`) and the UBL serializer (`invoicekit-format-ubl`). A caller takes a `CommercialDocument`, optionally supplies a Leitweg-ID, and gets back XRechnung-stamped UBL XML ready to hand to the validator. It is a sibling of the other profile crates (`profile-factur-x`, `profile-peppol-bis`, `profile-peppol-pint`).

The URN constants are frozen against the KoSIT XRechnung 3.x configuration; the `coverage` matrix is maintained against the XRechnung 3.0.2 specification.

## Usage

```rust
use invoicekit_profile_xrechnung::{to_xrechnung_3_x_xml, XRechnungOptions};

// B2G: supply the Leitweg-ID for the receiving authority.
let options = XRechnungOptions {
    leitweg_id: Some("04011000-1234512345-06".to_owned()),
};

// `doc` is an already-built invoicekit_ir::CommercialDocument.
let xml = to_xrechnung_3_x_xml(&doc, &options)?;
assert!(xml.contains("urn:xoev-de:kosit:standard:xrechnung_3.0"));
# Ok::<(), invoicekit_profile_xrechnung::XRechnungError>(())
```

For a business-to-business invoice, use `XRechnungOptions::default()` (no Leitweg-ID). The CIUS-DE customization is still stamped; the `BuyerReference` passes through unchanged.

## Status

Workspace member, not published to crates.io. The projection — header trio injection plus Leitweg-ID shape validation — is implemented and tested. An integration test projects 30+ UBL conformance fixtures and checks each carries the customization, profile, and Leitweg-ID. Runtime CIUS-DE rule enforcement is KoSIT's job, not this crate's: the `coverage` matrix records which BR-DE-* rules the projection happens to enforce structurally versus which rely on KoSIT. The KoSIT parity gate needs the `validator-configuration-xrechnung` scenarios bundle on disk (env var `INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS`) and self-skips when it is absent.

## License

Apache-2.0. Copyright the InvoiceKit Authors.
