// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-validate` — typed validation result schema for every InvoiceKit backend.
//!
//! Every validator backend — the hand-written `invoicekit-validate-ubl-cii`
//! rules, the JVM sidecars (KoSIT, phive, Saxon, ZATCA), the per-country
//! REST validators (e.g. Spain VeriFactu live check), the Peppol access-point
//! partner validators, the local CLI invocations, and the explicit
//! "no public reference exists" path — produces results in the same shape.
//! This crate defines that shape, and ships the JSON Schema that the
//! generated bindings (TypeScript, Python, Java, .NET) consume so the
//! validator contract is byte-equivalent across the network boundary.
//!
//! The schema is derived from the Rust types via [`schemars`], so the JSON
//! Schema regenerates whenever the Rust source of truth changes; CI
//! re-derives it and asserts byte-equality against the committed copy.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One validation finding emitted by a backend.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationResult {
    /// Identifier of the rule that produced this finding.
    pub rule_id: RuleId,
    /// Severity that the rule pack assigned to the finding.
    pub severity: Severity,
    /// EN 16931 business term or business group implicated.
    pub term: BusinessTerm,
    /// Pointer into the source document at which the finding applies.
    pub location: Location,
    /// Optional remediation hint shown to humans and consumed by autofixers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<SuggestedFix>,
    /// Citation back to the authoritative source for this rule.
    pub citation: Citation,
    /// Optional per-result trace context, owned by the T-032a trace extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<ValidationTrace>,
}

impl ValidationResult {
    /// Build a result from the minimum required fields.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_validate::{BusinessTerm, Citation, Location, RuleId, Severity, ValidationResult};
    ///
    /// let result = ValidationResult::new(
    ///     RuleId::new("BR-01").unwrap(),
    ///     Severity::Error,
    ///     BusinessTerm::business_term("BT-1").unwrap(),
    ///     Location::json_pointer("/document_number").unwrap(),
    ///     Citation::new("EN 16931", "BR-01", None).unwrap(),
    /// );
    ///
    /// assert_eq!(result.rule_id.as_str(), "BR-01");
    /// assert_eq!(result.severity, Severity::Error);
    /// ```
    #[must_use]
    pub fn new(
        rule_id: RuleId,
        severity: Severity,
        term: BusinessTerm,
        location: Location,
        citation: Citation,
    ) -> Self {
        Self {
            rule_id,
            severity,
            term,
            location,
            suggested_fix: None,
            citation,
            trace: None,
        }
    }

    /// Attach a suggested fix to the result.
    #[must_use]
    pub fn with_suggested_fix(mut self, fix: SuggestedFix) -> Self {
        self.suggested_fix = Some(fix);
        self
    }

    /// Attach a trace context to the result.
    #[must_use]
    pub fn with_trace(mut self, trace: ValidationTrace) -> Self {
        self.trace = Some(trace);
        self
    }
}

/// Stable rule identifier as declared by the rule pack (e.g. `BR-01`).
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RuleId(String);

impl RuleId {
    /// Build a non-empty rule identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `value` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// let id = invoicekit_validate::RuleId::new("BR-01").unwrap();
    /// assert_eq!(id.as_str(), "BR-01");
    /// ```
    pub fn new(value: impl Into<String>) -> Result<Self, ValidateError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ValidateError::BlankField("rule_id"));
        }
        Ok(Self(value))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Severity assigned by the rule pack.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Hard failure that prevents the document from being signed or transmitted.
    Fatal,
    /// Document is invalid but reconciliation may still proceed.
    Error,
    /// Document is valid but the rule pack flags a quality concern.
    Warning,
    /// Purely informational; never blocks workflow.
    Info,
}

/// EN 16931 business term (BT) or business group (BG) reference.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusinessTerm {
    /// Business term — an atomic invoice field.
    BusinessTerm {
        /// Term code, e.g. `BT-1`.
        code: String,
    },
    /// Business group — a structured collection of business terms.
    BusinessGroup {
        /// Group code, e.g. `BG-25`.
        code: String,
    },
}

// Same-name-as-type: the constructor mirrors the variant name on purpose so
// call sites read `BusinessTerm::business_term("BT-1")` and not a synthetic
// alias; the enum variant's name comes from the EN 16931 vocabulary.
#[allow(clippy::self_named_constructors)]
impl BusinessTerm {
    /// Build a `BT-*` business term.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::InvalidBusinessTerm`] when `code` is not a
    /// `BT-N` form with `N` a positive integer.
    ///
    /// # Examples
    ///
    /// ```
    /// let bt = invoicekit_validate::BusinessTerm::business_term("BT-1").unwrap();
    /// assert!(matches!(bt, invoicekit_validate::BusinessTerm::BusinessTerm { .. }));
    /// ```
    pub fn business_term(code: impl Into<String>) -> Result<Self, ValidateError> {
        let code = code.into();
        if !is_term_code(&code, "BT") {
            return Err(ValidateError::InvalidBusinessTerm(code));
        }
        Ok(Self::BusinessTerm { code })
    }

    /// Build a `BG-*` business group.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::InvalidBusinessTerm`] when `code` is not a
    /// `BG-N` form with `N` a positive integer.
    ///
    /// # Examples
    ///
    /// ```
    /// let bg = invoicekit_validate::BusinessTerm::business_group("BG-25").unwrap();
    /// assert!(matches!(bg, invoicekit_validate::BusinessTerm::BusinessGroup { .. }));
    /// ```
    pub fn business_group(code: impl Into<String>) -> Result<Self, ValidateError> {
        let code = code.into();
        if !is_term_code(&code, "BG") {
            return Err(ValidateError::InvalidBusinessTerm(code));
        }
        Ok(Self::BusinessGroup { code })
    }

    /// Underlying code string.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::BusinessTerm { code } | Self::BusinessGroup { code } => code,
        }
    }
}

fn is_term_code(code: &str, prefix: &str) -> bool {
    let Some(rest) = code.strip_prefix(prefix) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('-') else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) && rest != "0"
}

/// Source-document pointer.
///
/// XML-backed documents use XPath; JSON-backed documents use RFC 6901 JSON
/// Pointer. The two forms are distinct types so a backend cannot mix them
/// up by accident.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    /// RFC 6901 JSON Pointer (`/path/to/field`).
    JsonPointer {
        /// Pointer body, beginning with `/`.
        pointer: String,
    },
    /// XPath 1.0 expression (`/Invoice/cbc:ID/text()`).
    XPath {
        /// XPath expression as written by the backend.
        expression: String,
    },
}

impl Location {
    /// Build a JSON Pointer location.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::InvalidLocation`] when the pointer is empty
    /// or does not begin with `/`.
    ///
    /// # Examples
    ///
    /// ```
    /// let loc = invoicekit_validate::Location::json_pointer("/lines/0/quantity").unwrap();
    /// assert!(matches!(loc, invoicekit_validate::Location::JsonPointer { .. }));
    /// ```
    pub fn json_pointer(pointer: impl Into<String>) -> Result<Self, ValidateError> {
        let pointer = pointer.into();
        // RFC 6901 §3 allows the empty pointer "" (refers to the root); otherwise
        // every pointer must begin with `/`. Empty pointers are useful for
        // "the whole document failed", so we permit both.
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err(ValidateError::InvalidLocation(pointer));
        }
        Ok(Self::JsonPointer { pointer })
    }

    /// Build an XPath location.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::InvalidLocation`] when the expression is
    /// blank.
    ///
    /// # Examples
    ///
    /// ```
    /// let loc = invoicekit_validate::Location::xpath("/Invoice/cbc:ID").unwrap();
    /// assert!(matches!(loc, invoicekit_validate::Location::XPath { .. }));
    /// ```
    pub fn xpath(expression: impl Into<String>) -> Result<Self, ValidateError> {
        let expression = expression.into();
        if expression.trim().is_empty() {
            return Err(ValidateError::InvalidLocation(expression));
        }
        Ok(Self::XPath { expression })
    }
}

/// Concrete remediation hint.
///
/// Backends populate this when they can deterministically infer a fix
/// (e.g. set `currency` to `EUR`, drop the duplicate allowance line); UIs
/// can then offer the user a one-click apply.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct SuggestedFix {
    /// Short human-readable summary.
    pub summary: String,
    /// Optional patch body — JSON Patch (RFC 6902) for JSON Pointer
    /// locations, XSLT for XPath locations. Format is owned by the
    /// consuming UI; this crate only stores the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

impl SuggestedFix {
    /// Build a summary-only fix.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `summary` is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// let fix = invoicekit_validate::SuggestedFix::new("Set currency to EUR").unwrap();
    /// assert_eq!(fix.summary, "Set currency to EUR");
    /// ```
    pub fn new(summary: impl Into<String>) -> Result<Self, ValidateError> {
        let summary = summary.into();
        if summary.trim().is_empty() {
            return Err(ValidateError::BlankField("suggested_fix.summary"));
        }
        Ok(Self {
            summary,
            patch: None,
        })
    }

    /// Attach a structured patch body to the fix.
    #[must_use]
    pub fn with_patch(mut self, patch: impl Into<String>) -> Self {
        self.patch = Some(patch.into());
        self
    }
}

/// Authoritative citation for the rule.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Citation {
    /// Source document (e.g. `EN 16931`, `Peppol BIS 3.0`, `XRechnung 3.0`).
    pub source: String,
    /// Section identifier inside the source (e.g. `BR-01`, `§5.2`).
    pub section: String,
    /// Optional URL that resolves to the cited section online.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Citation {
    /// Build a citation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `source` or `section` is
    /// blank.
    ///
    /// # Examples
    ///
    /// ```
    /// let cite = invoicekit_validate::Citation::new("EN 16931", "BR-01", None).unwrap();
    /// assert_eq!(cite.source, "EN 16931");
    /// ```
    pub fn new(
        source: impl Into<String>,
        section: impl Into<String>,
        url: Option<String>,
    ) -> Result<Self, ValidateError> {
        let source = source.into();
        let section = section.into();
        if source.trim().is_empty() {
            return Err(ValidateError::BlankField("citation.source"));
        }
        if section.trim().is_empty() {
            return Err(ValidateError::BlankField("citation.section"));
        }
        Ok(Self {
            source,
            section,
            url,
        })
    }
}

/// Citation form embedded in explain-plan steps.
///
/// This intentionally has a distinct type name from [`Citation`] so generated
/// TypeScript bindings can re-export validation-result and explain-plan types
/// from one flat package without duplicate `Citation` symbols.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ExplainPlanCitation {
    /// Source document (e.g. `EN 16931`, `Peppol BIS 3.0`, `XRechnung 3.0`).
    pub source: String,
    /// Section identifier inside the source (e.g. `BR-01`, `§5.2`).
    pub section: String,
    /// Optional URL that resolves to the cited section online.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl From<Citation> for ExplainPlanCitation {
    fn from(value: Citation) -> Self {
        Self {
            source: value.source,
            section: value.section,
            url: value.url,
        }
    }
}

impl From<&Citation> for ExplainPlanCitation {
    fn from(value: &Citation) -> Self {
        Self {
            source: value.source.clone(),
            section: value.section.clone(),
            url: value.url.clone(),
        }
    }
}

/// Optional trace context owned by the T-032a extension.
///
/// Carries the backend identifier (`rust-native`, `jvm:kosit`, …), the
/// trace identifier the backend assigned, and any backend-specific
/// debug fields. The schema deliberately leaves the inner shape opaque so
/// new backends can ship without modifying this crate.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ValidationTrace {
    /// Backend identifier, e.g. `rust-native`, `jvm:kosit`, `rest:official`,
    /// `partner`, `cli:invoicekit`, `none`.
    pub backend: String,
    /// Trace identifier emitted by the backend; correlated by the
    /// `invoicekit-reconcile` outbox.
    pub trace_id: String,
    /// Backend-specific debug payload. Schema is opaque to this crate;
    /// the trace consumer (typically the support bundle redactor) owns
    /// any further structure.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub details: serde_json::Value,
}

impl ValidationTrace {
    /// Build a trace from the minimum required fields.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `backend` or `trace_id`
    /// is blank.
    ///
    /// # Examples
    ///
    /// ```
    /// let trace = invoicekit_validate::ValidationTrace::new("rust-native", "trace-abc").unwrap();
    /// assert_eq!(trace.backend, "rust-native");
    /// ```
    pub fn new(
        backend: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Result<Self, ValidateError> {
        let backend = backend.into();
        let trace_id = trace_id.into();
        if backend.trim().is_empty() {
            return Err(ValidateError::BlankField("trace.backend"));
        }
        if trace_id.trim().is_empty() {
            return Err(ValidateError::BlankField("trace.trace_id"));
        }
        Ok(Self {
            backend,
            trace_id,
            details: serde_json::Value::Null,
        })
    }
}

/// Complete ordered explanation of a validator run.
///
/// The plan is intentionally backend-neutral: a pure Rust validator, JVM
/// sidecar, or partner validator can all emit the same ordered rule steps.
/// Each step records where the rule evaluated, the machine-readable inputs the
/// backend considered safe to expose, its decision, and citations for audit.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationExplainPlan {
    /// Schema version for the explain-plan wire contract.
    pub schema_version: String,
    /// Backend identifier, e.g. `rust-native`, `jvm:kosit`, or `partner`.
    pub backend: String,
    /// Deterministic trace identifier assigned by the caller.
    pub trace_id: String,
    /// Ordered rule-evaluation steps.
    pub steps: Vec<RuleEvaluationStep>,
}

impl ValidationExplainPlan {
    /// Build a complete ordered explain plan.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `backend` or `trace_id` is
    /// blank.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_validate::{
    ///     explain_plan_from_results, Citation, RuleId, ValidationResult,
    /// };
    ///
    /// let rules = ["BR-01"];
    /// let findings: Vec<ValidationResult> = Vec::new();
    /// let plan = explain_plan_from_results(
    ///     "rust-native",
    ///     "trace-1",
    ///     &rules,
    ///     &findings,
    ///     |rule| Citation::new("EN 16931", rule, None),
    /// )
    /// .unwrap();
    ///
    /// assert_eq!(plan.steps[0].rule_id, RuleId::new("BR-01").unwrap());
    /// ```
    pub fn new(
        backend: impl Into<String>,
        trace_id: impl Into<String>,
        steps: Vec<RuleEvaluationStep>,
    ) -> Result<Self, ValidateError> {
        let backend = backend.into();
        let trace_id = trace_id.into();
        if backend.trim().is_empty() {
            return Err(ValidateError::BlankField("explain_plan.backend"));
        }
        if trace_id.trim().is_empty() {
            return Err(ValidateError::BlankField("explain_plan.trace_id"));
        }
        Ok(Self {
            schema_version: "1.0".to_owned(),
            backend,
            trace_id,
            steps,
        })
    }

    /// Render a stable human-readable Markdown narrative.
    ///
    /// The output is deterministic: rules stay in the same order as
    /// [`Self::steps`], citations are rendered in vector order, and JSON inputs
    /// are emitted from a [`BTreeMap`].
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Validation Explain Plan\n\n");
        let _ = writeln!(out, "- Schema: {}", self.schema_version);
        let _ = writeln!(out, "- Backend: {}", self.backend);
        let _ = writeln!(out, "- Trace: {}", self.trace_id);
        let _ = writeln!(out, "- Rules evaluated: {}\n", self.steps.len());
        out.push_str("| Rule | Decision | Evaluated at | Citations |\n");
        out.push_str("| --- | --- | --- | --- |\n");
        for step in &self.steps {
            let citations = step
                .citations
                .iter()
                .map(ExplainPlanCitation::to_markdown_cell)
                .collect::<Vec<_>>()
                .join("<br>");
            let _ = writeln!(
                out,
                "| {} | {} | `{}` | {} |",
                step.rule_id,
                step.decision.as_str(),
                escape_markdown_cell(&step.evaluated_at_path),
                citations
            );
            if !step.inputs.is_empty() {
                let inputs =
                    serde_json::to_string(&step.inputs).unwrap_or_else(|_| "{}".to_owned());
                let _ = writeln!(
                    out,
                    "|  | inputs |  | `{}` |",
                    escape_markdown_cell(&inputs)
                );
            }
        }
        out
    }
}

/// One rule evaluation inside a [`ValidationExplainPlan`].
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEvaluationStep {
    /// Identifier of the evaluated rule.
    pub rule_id: RuleId,
    /// Document path at which the rule evaluated.
    pub evaluated_at_path: String,
    /// Machine-readable inputs the backend considered safe to expose.
    pub inputs: BTreeMap<String, serde_json::Value>,
    /// Decision produced by the rule.
    pub decision: RuleEvaluationDecision,
    /// Citations that justify the rule.
    pub citations: Vec<ExplainPlanCitation>,
}

impl RuleEvaluationStep {
    /// Build a rule-evaluation step.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::BlankField`] when `evaluated_at_path` is blank.
    pub fn new(
        rule_id: RuleId,
        evaluated_at_path: impl Into<String>,
        decision: RuleEvaluationDecision,
        citations: Vec<ExplainPlanCitation>,
    ) -> Result<Self, ValidateError> {
        let evaluated_at_path = evaluated_at_path.into();
        if evaluated_at_path.trim().is_empty() {
            return Err(ValidateError::BlankField("explain_plan.evaluated_at_path"));
        }
        Ok(Self {
            rule_id,
            evaluated_at_path,
            inputs: BTreeMap::new(),
            decision,
            citations,
        })
    }

    /// Attach one structured input to the step.
    #[must_use]
    pub fn with_input(mut self, name: impl Into<String>, value: serde_json::Value) -> Self {
        self.inputs.insert(name.into(), value);
        self
    }
}

/// Decision emitted for one rule evaluation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEvaluationDecision {
    /// The rule evaluated and produced no finding.
    Pass,
    /// The rule produced an informational finding.
    Info,
    /// The rule produced a warning finding.
    Warning,
    /// The rule produced an error or fatal finding.
    Fail,
}

impl RuleEvaluationDecision {
    /// Stable display string used by the Markdown renderer.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Fail => "fail",
        }
    }
}

/// Build an explain plan from an ordered rule inventory and validator findings.
///
/// The caller owns the rule order. Rules that produced no findings become
/// `pass` steps evaluated at `/`; findings reuse their result location and
/// citation so the explain-plan stays linked to the ordinary
/// [`ValidationResult`] stream.
///
/// # Errors
///
/// Returns [`ValidateError`] when any rule id, backend field, or fallback
/// citation is invalid.
pub fn explain_plan_from_results<F>(
    backend: impl Into<String>,
    trace_id: impl Into<String>,
    ordered_rule_ids: &[&str],
    findings: &[ValidationResult],
    mut fallback_citation: F,
) -> Result<ValidationExplainPlan, ValidateError>
where
    F: FnMut(&str) -> Result<Citation, ValidateError>,
{
    let mut steps = Vec::with_capacity(ordered_rule_ids.len());
    for rule_id in ordered_rule_ids {
        let matching: Vec<&ValidationResult> = findings
            .iter()
            .filter(|finding| finding.rule_id.as_str() == *rule_id)
            .collect();
        let (decision, path, citations) = if matching.is_empty() {
            (
                RuleEvaluationDecision::Pass,
                "/".to_owned(),
                vec![fallback_citation(rule_id)?.into()],
            )
        } else {
            let decision = strongest_decision(&matching);
            let path = matching
                .first()
                .map_or("/", |finding| location_path(&finding.location))
                .to_owned();
            let citations = matching
                .iter()
                .map(|finding| ExplainPlanCitation::from(&finding.citation))
                .collect();
            (decision, path, citations)
        };
        let step = RuleEvaluationStep::new(RuleId::new(*rule_id)?, path, decision, citations)?
            .with_input("finding_count", serde_json::json!(matching.len()));
        steps.push(step);
    }
    ValidationExplainPlan::new(backend, trace_id, steps)
}

fn strongest_decision(findings: &[&ValidationResult]) -> RuleEvaluationDecision {
    if findings
        .iter()
        .any(|finding| matches!(finding.severity, Severity::Fatal | Severity::Error))
    {
        RuleEvaluationDecision::Fail
    } else if findings
        .iter()
        .any(|finding| matches!(finding.severity, Severity::Warning))
    {
        RuleEvaluationDecision::Warning
    } else {
        RuleEvaluationDecision::Info
    }
}

fn location_path(location: &Location) -> &str {
    match location {
        Location::JsonPointer { pointer } => pointer,
        Location::XPath { expression } => expression,
    }
}

impl ExplainPlanCitation {
    fn to_markdown_cell(&self) -> String {
        self.url.as_ref().map_or_else(
            || {
                format!(
                    "{} {}",
                    escape_markdown_cell(&self.source),
                    escape_markdown_cell(&self.section)
                )
            },
            |url| {
                format!(
                    "{} {} ({})",
                    escape_markdown_cell(&self.source),
                    escape_markdown_cell(&self.section),
                    escape_markdown_cell(url)
                )
            },
        )
    }
}

fn escape_markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// Errors emitted while constructing or validating `ValidationResult` value objects.
#[derive(Debug, Error)]
pub enum ValidateError {
    /// A required string field was blank.
    #[error("required field `{0}` was blank")]
    BlankField(&'static str),
    /// A business-term code did not match `BT-N` / `BG-N`.
    #[error("invalid business term `{0}`; expected `BT-<n>` or `BG-<n>` with positive n")]
    InvalidBusinessTerm(String),
    /// A location pointer was malformed.
    #[error("invalid location `{0}`")]
    InvalidLocation(String),
}

/// Generate the JSON Schema for [`ValidationResult`].
///
/// The output is a `serde_json::Value` so callers can serialize it to disk
/// or stream it directly through their preferred writer. The same function
/// is used by the CI gate that re-derives the schema and asserts byte
/// equality against the committed copy under `schemas/`.
///
/// # Panics
///
/// Panics only if `schemars` emits a schema that fails to serialize back to
/// a `serde_json::Value`, which the `schemars` documentation rules out for
/// any type that derives `JsonSchema` (and `ValidationResult` does).
///
/// # Examples
///
/// ```
/// let schema = invoicekit_validate::validation_result_schema();
/// // Top-level object exposes the JSON Schema draft URI under "$schema".
/// assert!(schema.get("$schema").is_some());
/// // `title` is the Rust type name.
/// assert_eq!(schema.get("title").and_then(|v| v.as_str()), Some("ValidationResult"));
/// ```
#[must_use]
pub fn validation_result_schema() -> serde_json::Value {
    let schema = schema_for!(ValidationResult);
    serde_json::to_value(schema).expect("schemars output is always serializable")
}

/// Generate the JSON Schema for [`ValidationExplainPlan`].
///
/// # Examples
///
/// ```
/// let schema = invoicekit_validate::validation_explain_plan_schema();
/// assert_eq!(
///     schema.get("title").and_then(|value| value.as_str()),
///     Some("ValidationExplainPlan")
/// );
/// ```
///
/// # Panics
///
/// Panics only if `schemars` emits a schema that fails to serialize back to
/// a `serde_json::Value`, which the `schemars` documentation rules out for
/// any type that derives `JsonSchema` (and `ValidationExplainPlan` does).
#[must_use]
pub fn validation_explain_plan_schema() -> serde_json::Value {
    let schema = schema_for!(ValidationExplainPlan);
    serde_json::to_value(schema).expect("schemars output is always serializable")
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_validate::crate_name(), "invoicekit-validate");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-validate"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-validate");
    }

    #[test]
    fn happy_path_round_trips_through_json() {
        let result = ValidationResult::new(
            RuleId::new("BR-01").unwrap(),
            Severity::Error,
            BusinessTerm::business_term("BT-1").unwrap(),
            Location::json_pointer("/document_number").unwrap(),
            Citation::new("EN 16931", "BR-01", None).unwrap(),
        )
        .with_suggested_fix(SuggestedFix::new("Set document_number to a non-empty value").unwrap())
        .with_trace(ValidationTrace::new("rust-native", "trace-1").unwrap());

        let json = serde_json::to_value(&result).unwrap();
        let parsed: ValidationResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, result);
    }

    #[test]
    fn blank_rule_id_is_rejected() {
        let err = RuleId::new("   ").unwrap_err();
        assert!(matches!(err, ValidateError::BlankField("rule_id")));
    }

    #[test]
    fn invalid_business_term_is_rejected() {
        let err = BusinessTerm::business_term("BT-").unwrap_err();
        assert!(matches!(err, ValidateError::InvalidBusinessTerm(_)));
        let err = BusinessTerm::business_term("BT-0").unwrap_err();
        assert!(matches!(err, ValidateError::InvalidBusinessTerm(_)));
        let err = BusinessTerm::business_term("XX-1").unwrap_err();
        assert!(matches!(err, ValidateError::InvalidBusinessTerm(_)));
    }

    #[test]
    fn invalid_json_pointer_is_rejected() {
        let err = Location::json_pointer("does-not-start-with-slash").unwrap_err();
        assert!(matches!(err, ValidateError::InvalidLocation(_)));
    }

    #[test]
    fn empty_json_pointer_is_accepted_per_rfc6901() {
        let loc = Location::json_pointer("").unwrap();
        assert!(matches!(loc, Location::JsonPointer { .. }));
    }

    #[test]
    fn xpath_blank_is_rejected() {
        let err = Location::xpath("   ").unwrap_err();
        assert!(matches!(err, ValidateError::InvalidLocation(_)));
    }

    #[test]
    fn unknown_field_in_json_is_rejected() {
        // serde(deny_unknown_fields) protects the validator-network contract.
        let bad = serde_json::json!({
            "rule_id": "BR-01",
            "severity": "error",
            "term": {"kind": "business_term", "code": "BT-1"},
            "location": {"kind": "json_pointer", "pointer": "/x"},
            "citation": {"source": "EN 16931", "section": "BR-01"},
            "unexpected_extension": true
        });
        let err = serde_json::from_value::<ValidationResult>(bad).unwrap_err();
        assert!(err.to_string().contains("unexpected_extension"));
    }

    #[test]
    fn schema_describes_required_top_level_fields() {
        let schema = validation_result_schema();
        let required = schema
            .pointer("/required")
            .and_then(serde_json::Value::as_array)
            .expect("required array present");
        let required_names: Vec<&str> = required
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        for name in ["rule_id", "severity", "term", "location", "citation"] {
            assert!(
                required_names.contains(&name),
                "{name} not marked required in schema"
            );
        }
    }

    #[test]
    fn explain_plan_marks_missing_findings_as_pass() {
        let plan = explain_plan_from_results("rust-native", "trace-1", &["BR-01"], &[], |rule| {
            Citation::new("EN 16931", rule, None)
        })
        .unwrap();

        assert_eq!(plan.schema_version, "1.0");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].decision, RuleEvaluationDecision::Pass);
        assert_eq!(plan.steps[0].evaluated_at_path, "/");
        assert_eq!(
            plan.steps[0].inputs.get("finding_count"),
            Some(&serde_json::json!(0))
        );
    }

    #[test]
    fn explain_plan_reuses_finding_location_and_citation_on_fail() {
        let finding = ValidationResult::new(
            RuleId::new("BR-01").unwrap(),
            Severity::Error,
            BusinessTerm::business_term("BT-1").unwrap(),
            Location::xpath("/ubl:Invoice/cbc:ID").unwrap(),
            Citation::new(
                "EN 16931",
                "BR-01",
                Some("https://example.test/br-01".into()),
            )
            .unwrap(),
        );
        let plan =
            explain_plan_from_results("rust-native", "trace-2", &["BR-01"], &[finding], |rule| {
                Citation::new("fallback", rule, None)
            })
            .unwrap();

        assert_eq!(plan.steps[0].decision, RuleEvaluationDecision::Fail);
        assert_eq!(plan.steps[0].evaluated_at_path, "/ubl:Invoice/cbc:ID");
        assert_eq!(plan.steps[0].citations[0].source, "EN 16931");
    }

    #[test]
    fn explain_plan_schema_describes_steps() {
        let schema = validation_explain_plan_schema();
        assert_eq!(
            schema.get("title").and_then(serde_json::Value::as_str),
            Some("ValidationExplainPlan")
        );
        let required = schema
            .pointer("/required")
            .and_then(serde_json::Value::as_array)
            .expect("required array present");
        let required_names: Vec<&str> = required
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        for name in ["schema_version", "backend", "trace_id", "steps"] {
            assert!(
                required_names.contains(&name),
                "{name} not marked required in schema"
            );
        }
    }

    #[test]
    fn explain_plan_markdown_is_stable_and_readable() {
        let plan = explain_plan_from_results("rust-native", "trace-md", &["BR-01"], &[], |rule| {
            Citation::new("EN 16931", rule, None)
        })
        .unwrap();
        let markdown = plan.to_markdown();

        assert!(markdown.contains("# Validation Explain Plan"));
        assert!(markdown.contains("| BR-01 | pass | `/` | EN 16931 BR-01 |"));
        assert!(markdown.contains("`{\"finding_count\":0}`"));
    }

    proptest! {
        /// Every Severity enum variant serializes to a snake_case string and
        /// round-trips through JSON without loss.
        #[test]
        fn severity_round_trips_through_json(variant in prop_oneof![
            Just(Severity::Fatal),
            Just(Severity::Error),
            Just(Severity::Warning),
            Just(Severity::Info),
        ]) {
            let json = serde_json::to_value(variant).unwrap();
            let back: Severity = serde_json::from_value(json).unwrap();
            prop_assert_eq!(back, variant);
        }
    }
}
