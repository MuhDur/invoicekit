// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit validate` runner.
//!
//! Validates UBL 2.1 Invoice/CreditNote or UN/CEFACT CII XML with the
//! native EN 16931 rule set. `--explain` emits a T-032a explain plan:
//! ordered rule evaluations with paths, inputs, decisions, and citations.
//!
//! Exit codes:
//!
//! * `0` - document parsed and produced no findings.
//! * `1` - document parsed and produced one or more findings.
//! * `2` - usage error, unreadable file, unsupported root, or malformed XML.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_validate::{explain_plan_from_results, Citation, ValidationExplainPlan};
use invoicekit_validate_ubl_cii::{
    implemented_rule_ids, validate_xml_with_options, DocumentSyntax, En16931Coverage, En16931Error,
    En16931Report, RulepackAudit, ValidationOptions,
};
use serde::Serialize;

/// Run `invoicekit validate`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let xml = match fs::read_to_string(&parsed.input) {
        Ok(xml) => xml,
        Err(err) => {
            eprintln!("validate: cannot read {}: {err}", parsed.input.display());
            return ExitCode::from(2);
        }
    };

    let source = parsed.input.to_string_lossy();
    let options = parsed.validation_options();
    let rendered = match render_validation(&xml, &source, parsed.json, parsed.explain, &options) {
        Ok(rendered) => rendered,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };

    println!("{}", rendered.output);
    if rendered.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[derive(Debug)]
struct Args {
    input: PathBuf,
    json: bool,
    explain: bool,
    date: Option<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut json = false;
    let mut explain = false;
    let mut date: Option<String> = None;
    let mut i = 0;
    while let Some(arg) = argv.get(i) {
        match arg.as_str() {
            "--help" | "-h" => return Err(usage_help()),
            "--json" => {
                json = true;
                i += 1;
            }
            "--explain" => {
                explain = true;
                i += 1;
            }
            "--date" => {
                let value = argv.get(i + 1).ok_or_else(|| {
                    format!("validate: --date requires YYYY-MM-DD\n\n{}", usage_help())
                })?;
                date = Some(value.clone());
                i += 2;
            }
            flag if flag.starts_with("--date=") => {
                date = Some(flag.trim_start_matches("--date=").to_owned());
                i += 1;
            }
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "validate: unknown flag {flag:?}\n\n{}",
                    usage_help()
                ));
            }
            positional => {
                if input.is_some() {
                    return Err(format!(
                        "validate: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                input = Some(PathBuf::from(positional));
                i += 1;
            }
        }
    }
    let input = input
        .ok_or_else(|| format!("validate: <file.xml> argument required\n\n{}", usage_help()))?;
    Ok(Args {
        input,
        json,
        explain,
        date,
    })
}

impl Args {
    fn validation_options(&self) -> ValidationOptions {
        let mut options = ValidationOptions::default();
        if let Some(date) = &self.date {
            options = options.with_validation_date(date.clone());
        }
        options
    }
}

fn usage_help() -> String {
    "usage: invoicekit validate <file.xml> [--date=YYYY-MM-DD] [--json] [--explain]\n\nValidate UBL/CII XML with the native EN 16931 rule set. Default output is human-readable; --date selects the effective rule pack, --json prints machine-readable findings, and --explain switches to the ordered rule explain plan.".to_owned()
}

#[derive(Debug)]
struct RenderedValidation {
    output: String,
    ok: bool,
}

fn render_validation(
    xml: &str,
    source: &str,
    json: bool,
    explain: bool,
    options: &ValidationOptions,
) -> Result<RenderedValidation, String> {
    let report =
        validate_xml_with_options(xml, options).map_err(|err| format_validate_error(&err))?;
    let ok = report.findings.is_empty();
    let output = if explain {
        let plan = build_explain_plan(&report, xml, source).map_err(|err| err.to_string())?;
        if json {
            serde_json::to_string_pretty(&plan)
                .map_err(|err| format!("validate: explain-plan JSON serialization failed: {err}"))?
        } else {
            plan.to_markdown()
        }
    } else if json {
        let summary = ValidationSummary::from_report(&report, source);
        serde_json::to_string_pretty(&summary)
            .map_err(|err| format!("validate: report JSON serialization failed: {err}"))?
    } else {
        render_human_summary(&report, source)
    };
    Ok(RenderedValidation { output, ok })
}

fn format_validate_error(err: &En16931Error) -> String {
    format!(
        "validate: {err}. Remediation: provide a UBL 2.1 Invoice/CreditNote or UN/CEFACT CII XML document."
    )
}

fn build_explain_plan(
    report: &En16931Report,
    xml: &str,
    source: &str,
) -> Result<ValidationExplainPlan, invoicekit_validate::ValidateError> {
    let hash = blake3::hash(xml.as_bytes()).to_hex().to_string();
    let trace_id = format!("en16931-{}", &hash[..16]);
    let mut plan = explain_plan_from_results(
        "rust-native:en16931",
        trace_id,
        implemented_rule_ids(),
        &report.findings,
        fallback_citation,
    )?;
    let syntax = syntax_label(report.syntax);
    for step in &mut plan.steps {
        step.inputs
            .insert("document_syntax".to_owned(), serde_json::json!(syntax));
        step.inputs
            .insert("source".to_owned(), serde_json::json!(source));
        step.inputs.insert(
            "rulepack_id".to_owned(),
            serde_json::json!(report.rulepack.rulepack_id),
        );
        step.inputs.insert(
            "rulepack_selected_for_date".to_owned(),
            serde_json::json!(report.rulepack.selected_for_date),
        );
        step.inputs.insert(
            "rulepack_upstream_version".to_owned(),
            serde_json::json!(report.rulepack.upstream_version),
        );
    }
    Ok(plan)
}

fn fallback_citation(rule_id: &str) -> Result<Citation, invoicekit_validate::ValidateError> {
    Citation::new(
        "ConnectingEurope/eInvoicing-EN16931 validation-1.3.16",
        rule_id,
        Some("https://github.com/ConnectingEurope/eInvoicing-EN16931".to_owned()),
    )
}

#[derive(Debug, Serialize)]
struct ValidationSummary<'a> {
    source: &'a str,
    ok: bool,
    syntax: &'static str,
    finding_count: usize,
    coverage: CoverageSummary,
    rulepack: RulepackSummary<'a>,
    findings: &'a [invoicekit_validate::ValidationResult],
}

impl<'a> ValidationSummary<'a> {
    fn from_report(report: &'a En16931Report, source: &'a str) -> Self {
        Self {
            source,
            ok: report.findings.is_empty(),
            syntax: syntax_label(report.syntax),
            finding_count: report.findings.len(),
            coverage: CoverageSummary::from(report.coverage),
            rulepack: RulepackSummary::from(&report.rulepack),
            findings: &report.findings,
        }
    }
}

#[derive(Debug, Serialize)]
struct CoverageSummary {
    total: usize,
    implemented: usize,
    deferred_ir_gap: usize,
}

impl From<En16931Coverage> for CoverageSummary {
    fn from(value: En16931Coverage) -> Self {
        Self {
            total: value.total,
            implemented: value.implemented,
            deferred_ir_gap: value.deferred_ir_gap,
        }
    }
}

#[derive(Debug, Serialize)]
struct RulepackSummary<'a> {
    rulepack_id: &'a str,
    upstream_version: &'a str,
    selected_for_date: &'a str,
    effective_from: &'a str,
    effective_to: Option<&'a str>,
    source_url: &'a str,
    retrieved_at: &'a str,
    signature_alg: &'a str,
    disabled_rules: &'a [String],
}

impl<'a> From<&'a RulepackAudit> for RulepackSummary<'a> {
    fn from(value: &'a RulepackAudit) -> Self {
        Self {
            rulepack_id: &value.rulepack_id,
            upstream_version: &value.upstream_version,
            selected_for_date: &value.selected_for_date,
            effective_from: &value.effective_from,
            effective_to: value.effective_to.as_deref(),
            source_url: &value.source_url,
            retrieved_at: &value.retrieved_at,
            signature_alg: &value.signature_alg,
            disabled_rules: &value.disabled_rules,
        }
    }
}

fn render_human_summary(report: &En16931Report, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "validate: {} {} ({} finding{}, {}/{} rules implemented)",
        source,
        if report.findings.is_empty() {
            "ok"
        } else {
            "failed"
        },
        report.findings.len(),
        if report.findings.len() == 1 { "" } else { "s" },
        report.coverage.implemented,
        report.coverage.total
    );
    let _ = writeln!(out, "syntax: {}", syntax_label(report.syntax));
    let _ = writeln!(
        out,
        "rulepack: {} ({}, selected_for_date={})",
        report.rulepack.rulepack_id,
        report.rulepack.upstream_version,
        report.rulepack.selected_for_date
    );
    for finding in &report.findings {
        let _ = writeln!(
            out,
            "- {} {:?} at {}: {}",
            finding.rule_id,
            finding.severity,
            location_path(&finding.location),
            finding
                .suggested_fix
                .as_ref()
                .map_or("no remediation hint available", |fix| fix.summary.as_str())
        );
    }
    out
}

fn syntax_label(syntax: DocumentSyntax) -> &'static str {
    match syntax {
        DocumentSyntax::Ubl => "ubl",
        DocumentSyntax::Cii => "cii",
    }
}

fn location_path(location: &invoicekit_validate::Location) -> &str {
    match location {
        invoicekit_validate::Location::JsonPointer { pointer } => pointer,
        invoicekit_validate::Location::XPath { expression } => expression,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_validate_ubl_cii::validate_xml;

    const UBL_NS: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
    const CAC_NS: &str = "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2";
    const CBC_NS: &str = "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2";
    const CII_NS: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
    const RAM_NS: &str =
        "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
    const UDT_NS: &str = "urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100";

    #[test]
    fn parse_args_accepts_explain_json() {
        let parsed = parse_args(&[
            "sample.xml".to_owned(),
            "--explain".to_owned(),
            "--json".to_owned(),
        ])
        .unwrap();

        assert_eq!(parsed.input, PathBuf::from("sample.xml"));
        assert!(parsed.explain);
        assert!(parsed.json);
    }

    #[test]
    fn parse_args_accepts_date_eq_and_split_forms() {
        let eq = parse_args(&["sample.xml".to_owned(), "--date=2024-01-01".to_owned()]).unwrap();
        assert_eq!(eq.date.as_deref(), Some("2024-01-01"));

        let split = parse_args(&[
            "sample.xml".to_owned(),
            "--date".to_owned(),
            "2025-02-03".to_owned(),
        ])
        .unwrap();
        assert_eq!(split.date.as_deref(), Some("2025-02-03"));
    }

    #[test]
    fn parse_args_rejects_date_without_value() {
        let err = parse_args(&["sample.xml".to_owned(), "--date".to_owned()]).unwrap_err();
        assert!(err.contains("--date requires"));
    }

    #[test]
    fn parse_args_rejects_extra_positional() {
        let err = parse_args(&["a.xml".to_owned(), "b.xml".to_owned()]).unwrap_err();
        assert!(err.contains("extra positional"));
    }

    #[test]
    fn explain_markdown_contains_ordered_rule_trace() {
        let rendered = render_validation(
            minimal_ubl(),
            "minimal.xml",
            false,
            true,
            &ValidationOptions::default(),
        )
        .unwrap();

        assert!(!rendered.ok);
        assert!(rendered.output.contains("# Validation Explain Plan"));
        assert!(rendered.output.contains("| BR-01 | fail |"));
        assert!(rendered.output.contains("document_syntax"));
        assert!(rendered.output.contains("minimal.xml"));
        assert!(rendered.output.contains("rulepack_id"));
    }

    #[test]
    fn explain_json_is_machine_readable() {
        let rendered = render_validation(
            minimal_ubl(),
            "minimal.xml",
            true,
            true,
            &ValidationOptions::default().with_validation_date("2024-06-01"),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered.output).unwrap();

        assert_eq!(value["schema_version"], "1.0");
        assert_eq!(value["backend"], "rust-native:en16931");
        assert!(value["steps"].as_array().unwrap().len() >= 80);
        assert_eq!(
            value["steps"][0]["inputs"]["rulepack_selected_for_date"],
            "2024-06-01"
        );
    }

    #[test]
    fn five_invoice_shape_snapshots_stay_stable() {
        let cases = [
            ("minimal_ubl", minimal_ubl()),
            ("ubl_with_id", ubl_with_id()),
            ("ubl_with_id_and_date", ubl_with_id_and_date()),
            ("ubl_with_currency", ubl_with_currency()),
            ("minimal_cii", minimal_cii()),
        ];
        let actual = cases
            .into_iter()
            .map(|(name, xml)| explain_snapshot(name, xml))
            .collect::<Vec<_>>()
            .join("\n---\n");
        let expected = "minimal_ubl\nsyntax=ubl\nsteps=86\nfindings=18\nfirst_non_pass=BR-01:fail:/document/profile,BR-02:fail:/document/id,BR-03:fail:/document/issue_date,BR-04:fail:/document/type_code,BR-05:fail:/document/currency,BR-06:fail:/supplier/name\n---\nubl_with_id\nsyntax=ubl\nsteps=86\nfindings=17\nfirst_non_pass=BR-01:fail:/document/profile,BR-03:fail:/document/issue_date,BR-04:fail:/document/type_code,BR-05:fail:/document/currency,BR-06:fail:/supplier/name,BR-07:fail:/customer/name\n---\nubl_with_id_and_date\nsyntax=ubl\nsteps=86\nfindings=16\nfirst_non_pass=BR-01:fail:/document/profile,BR-04:fail:/document/type_code,BR-05:fail:/document/currency,BR-06:fail:/supplier/name,BR-07:fail:/customer/name,BR-08:fail:/supplier/address\n---\nubl_with_currency\nsyntax=ubl\nsteps=86\nfindings=16\nfirst_non_pass=BR-01:fail:/document/profile,BR-04:fail:/document/type_code,BR-06:fail:/supplier/name,BR-07:fail:/customer/name,BR-08:fail:/supplier/address,BR-09:fail:/supplier/address/country\n---\nminimal_cii\nsyntax=cii\nsteps=86\nfindings=18\nfirst_non_pass=BR-01:fail:/document/profile,BR-02:fail:/document/id,BR-03:fail:/document/issue_date,BR-04:fail:/document/type_code,BR-05:fail:/document/currency,BR-06:fail:/supplier/name";
        assert_eq!(actual, expected);
    }

    fn explain_snapshot(name: &str, xml: &str) -> String {
        let report = validate_xml(xml).unwrap();
        let plan = build_explain_plan(&report, xml, name).unwrap();
        let first_non_pass = plan
            .steps
            .iter()
            .filter(|step| step.decision != invoicekit_validate::RuleEvaluationDecision::Pass)
            .take(6)
            .map(|step| {
                format!(
                    "{}:{}:{}",
                    step.rule_id,
                    step.decision.as_str(),
                    step.evaluated_at_path
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{name}\nsyntax={}\nsteps={}\nfindings={}\nfirst_non_pass={first_non_pass}",
            syntax_label(report.syntax),
            plan.steps.len(),
            report.findings.len()
        )
    }

    fn minimal_ubl() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"></ubl:Invoice>"#
    }

    fn ubl_with_id() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cbc:ID>INV-1</cbc:ID></ubl:Invoice>"#
    }

    fn ubl_with_id_and_date() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cbc:ID>INV-1</cbc:ID><cbc:IssueDate>2026-05-28</cbc:IssueDate></ubl:Invoice>"#
    }

    fn ubl_with_currency() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"><cbc:ID>INV-1</cbc:ID><cbc:IssueDate>2026-05-28</cbc:IssueDate><cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode></ubl:Invoice>"#
    }

    fn minimal_cii() -> &'static str {
        r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100" xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100" xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100"></rsm:CrossIndustryInvoice>"#
    }

    #[test]
    fn namespace_constants_are_real_fixture_values() {
        assert!(minimal_ubl().contains(UBL_NS));
        assert!(minimal_ubl().contains(CAC_NS));
        assert!(minimal_ubl().contains(CBC_NS));
        assert!(minimal_cii().contains(CII_NS));
        assert!(minimal_cii().contains(RAM_NS));
        assert!(minimal_cii().contains(UDT_NS));
    }
}
