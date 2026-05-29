# invoicekit-validate-ubl-cii

Pure-Rust EN 16931 business-rule validation for UBL and Cross Industry Invoice XML.

## What it does

Given an invoice as raw XML, this crate parses it, decides whether it is Universal Business Language (UBL `Invoice`/`CreditNote`) or UN/CEFACT Cross Industry Invoice (CII), and runs the EN 16931 business rules (`BR-*`, `BR-CO-*`, plus a few `BR-AE-*` and `BR-CL-*` checks) against it. Each broken rule comes back as a typed finding that names the rule, the business term it touches, an XPath-style location, a citation to the upstream CEN rule source, and a suggested fix.

It validates the XML directly rather than going through the typed invoice model. That is deliberate: an invalid invoice should still reach the validator so it can name the violated rule, instead of being rejected at parse time before anyone learns why. The XML parser here is a small in-house tree builder over `quick-xml`; it does not do XSD schema validation.

Which rules run is selected from a signed rulepack chosen by country, profile, and effective date. The default is the `global` EN 16931 profile (`urn:cen.eu:en16931:2017`) at the latest pack. A rulepack can disable individual rules (or all of them) for a given date window, and the disabled set is recorded in the report for audit.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> [validate] -> render/intake -> transmit -> evidence
```

This is one of the validators. It is the pure-Rust EN 16931 checker for the UBL and CII syntaxes. It depends on `invoicekit-validate` for the shared finding types and on `invoicekit-rulepack` for rulepack selection. It does not replace the isolated JVM reference-validator worker; it is the native path and a parity counterpart to it. The `invoicekit-en16931-findings` binary emits findings as JSON for parity harnesses.

## Coverage

86 EN 16931 rules are implemented, all of them in the current coverage matrix; `deferred_rules()` is empty today. The exact identifiers are returned by `implemented_rule_ids()`, and the coverage counts by `En16931Coverage::current()`. This is real rule logic, not a stub, but it is a curated subset of the full EN 16931 rule set, not the complete schematron — treat the JVM reference worker as the authority where the two disagree.

## Key public API

- `validate_xml(input: &str) -> Result<En16931Report, En16931Error>` — validate against the latest default rulepack.
- `validate_xml_on_date(input, validation_date)` — validate against the rulepack effective on a given `YYYY-MM-DD` date.
- `validate_xml_with_options(input, &ValidationOptions)` — explicit country/profile/date selection.
- `validate_xml_with_registry(input, &ValidationOptions, &Registry)` — validate against a caller-supplied rulepack registry (used by tests and hot-reload paths).
- `ValidationOptions` — `country`, `profile`, optional `validation_date`; `ValidationOptions::default()` selects the `global` EN 16931 profile, latest pack.
- `En16931Report` — `syntax` (`DocumentSyntax::Ubl` or `Cii`), `findings: Vec<ValidationResult>`, `coverage`, and `rulepack: RulepackAudit`.
- `RulepackAudit` — which rulepack ran: id, upstream version, effective window, source URL, signature algorithm, the date selector used, and any disabled rules.
- `implemented_rule_ids()`, `deferred_rules()`, `En16931Coverage::current()` — coverage introspection.
- `EN16931_PROFILE_URN`, `EN16931_BR_CO_COVERAGE_JSON`, `crate_name()` — constants and metadata.
- `En16931Error` — parse, encoding, unsupported-root, invalid-date, rulepack-not-found, and rulepack-policy errors.

## Usage

```rust
use invoicekit_validate_ubl_cii::{validate_xml, DocumentSyntax};

let report = validate_xml(invoice_xml)?;

match report.syntax {
    DocumentSyntax::Ubl => println!("UBL invoice"),
    DocumentSyntax::Cii => println!("CII invoice"),
}

if report.findings.is_empty() {
    println!("passes the implemented EN 16931 rules");
} else {
    for finding in &report.findings {
        println!("{}: {}", finding.rule_id.as_str(), finding.term.code());
    }
}

// Which rulepack decided the outcome, for the evidence trail:
println!("ran {}", report.rulepack.rulepack_id);
# Ok::<(), invoicekit_validate_ubl_cii::En16931Error>(())
```

To pin a historical rule set, use `validate_xml_on_date(invoice_xml, "2024-06-01")`.

## License

Apache-2.0. Part of the InvoiceKit workspace.
