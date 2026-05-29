# invoicekit-validate

The typed validation-result schema that every InvoiceKit validator backend speaks.

## What it does

This crate is a contract, not a validator. It does not run rules and it does not call out to any worker. It defines the shape of a single validation finding and the shape of a full explain plan, and it generates the JSON Schema that the generated bindings (TypeScript, Python, Java, .NET) consume so the result format is identical on both sides of a network boundary.

Every backend produces results in this shape: the hand-written Rust rules in `invoicekit-validate-ubl-cii`, the JVM sidecars (KoSIT, phive, Saxon, ZATCA), the per-country REST validators, the Peppol access-point partner validators, the local command-line invocations, and the explicit "no public reference exists" path. The Rust types are the source of truth; `schemars` derives the JSON Schema from them, and continuous integration re-derives it and asserts byte-equality against the copy committed under `schemas/`.

The rule logic itself lives elsewhere. The rule registry is `invoicekit-rulepack`; the EN 16931 / UBL / CII rule implementations are `invoicekit-validate-ubl-cii`. This crate only owns the wire contract those produce.

## Public API

Value objects, each with a checked constructor that returns `Result<_, ValidateError>`:

- `ValidationResult` — one finding. Fields: `rule_id`, `severity`, `term`, `location`, optional `suggested_fix`, `citation`, optional `trace`. Built with `ValidationResult::new(..)`, then `.with_suggested_fix(..)` and `.with_trace(..)`.
- `RuleId` — a non-empty rule identifier such as `BR-01`. `RuleId::new`, `.as_str()`.
- `Severity` — `Fatal`, `Error`, `Warning`, `Info`.
- `BusinessTerm` — an EN 16931 business term (`BusinessTerm::business_term("BT-1")`) or business group (`BusinessTerm::business_group("BG-25")`). Codes must match `BT-<n>` / `BG-<n>` with positive `n`.
- `Location` — `Location::json_pointer("/path")` (RFC 6901; the empty pointer is allowed and means the whole document) or `Location::xpath("/Invoice/cbc:ID")`. The two forms are distinct so a backend cannot mix them up.
- `SuggestedFix` — `SuggestedFix::new("summary")`, optional `.with_patch(..)`. The patch body (JSON Patch for JSON Pointer locations, XSLT for XPath) is stored as opaque bytes; the consuming user interface owns its format.
- `Citation` — `Citation::new(source, section, url)` linking a finding back to its authority (`EN 16931`, `Peppol BIS 3.0`, `XRechnung 3.0`, …).
- `ValidationTrace` — optional backend trace context: `backend`, `trace_id`, opaque `details`.

Explain-plan types (a complete ordered account of a validator run):

- `ValidationExplainPlan` — `schema_version`, `backend`, `trace_id`, ordered `steps`. `.to_markdown()` renders a deterministic Markdown narrative.
- `RuleEvaluationStep`, `RuleEvaluationDecision` (`Pass` / `Info` / `Warning` / `Fail`), `ExplainPlanCitation`.
- `explain_plan_from_results(backend, trace_id, ordered_rule_ids, findings, fallback_citation)` — builds a plan from an ordered rule inventory plus the ordinary findings stream. Rules with no finding become `pass` steps; rules with findings reuse the finding's location and citation.

Schema and metadata:

- `validation_result_schema() -> serde_json::Value`
- `validation_explain_plan_schema() -> serde_json::Value`
- `crate_name() -> &'static str`
- `ValidateError` — `BlankField`, `InvalidBusinessTerm`, `InvalidLocation`.

The serialized form uses `deny_unknown_fields` on the contract structs, so an unrecognized field in incoming JSON is rejected rather than silently dropped.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
                                               ^^^^^^^^
```

`invoicekit-validate` is the data type at the `validate` stage. A validator backend runs over a canonical or format-bound document and emits a stream of `ValidationResult` values plus, optionally, a `ValidationExplainPlan`. Downstream consumers — the command-line tool, the language-server diagnostics, the reconcile outbox, the evidence bundle — depend on this crate, not on any one backend.

## Example

```rust
use invoicekit_validate::{
    BusinessTerm, Citation, Location, RuleId, Severity, SuggestedFix, ValidationResult,
};

let result = ValidationResult::new(
    RuleId::new("BR-01").unwrap(),
    Severity::Error,
    BusinessTerm::business_term("BT-1").unwrap(),
    Location::json_pointer("/document_number").unwrap(),
    Citation::new("EN 16931", "BR-01", None).unwrap(),
)
.with_suggested_fix(SuggestedFix::new("Set document_number to a non-empty value").unwrap());

let json = serde_json::to_value(&result).unwrap();
let parsed: ValidationResult = serde_json::from_value(json).unwrap();
assert_eq!(parsed, result);
```

To regenerate the JSON Schema files, run the bundled examples:

```sh
cargo run -p invoicekit-validate --example emit_schema
cargo run -p invoicekit-validate --example emit_explain_plan_schema
```

## License

Apache-2.0, in line with the rest of the InvoiceKit workspace.
