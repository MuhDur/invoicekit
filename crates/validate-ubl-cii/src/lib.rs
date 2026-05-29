// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Pure-Rust EN 16931 validation for UBL and CII XML.
//!
//! This crate intentionally validates raw XML. Invalid invoices must produce
//! typed [`invoicekit_validate::ValidationResult`] values instead of being
//! rejected before the validator can name the violated BR/BR-CO rule.

use std::collections::BTreeSet;
use std::str;

use invoicekit_rulepack::{Manifest, Registry, RulepackError};
use invoicekit_validate::{
    BusinessTerm, Citation, Location, RuleId, Severity, SuggestedFix, ValidateError,
    ValidationResult,
};
use quick_xml::encoding::{Decoder, EncodingError};
use quick_xml::events::{attributes::AttrError, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rust_decimal::Decimal;
use thiserror::Error;

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_validate_ubl_cii::crate_name(), "invoicekit-validate-ubl-cii");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-validate-ubl-cii"
}

/// Rule inventory checked against T-031's upstream coverage-matrix bead.
pub const EN16931_BR_CO_COVERAGE_JSON: &str =
    include_str!("../../rulepack/data/en16931-br-co-coverage.json");

const COVERAGE_RULE_TOTAL: usize = 86;
const COVERAGE_IMPLEMENTED_NOW: usize = 86;
const COVERAGE_DEFERRED_IR_GAP: usize = 0;

const UBL_INVOICE_NAMESPACE_URI: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const UBL_CREDIT_NOTE_NAMESPACE_URI: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";
const CII_RSM_NAMESPACE_URI: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";
const VAT_ID_COUNTRY_PREFIXES: &str = " 1A AD AE AF AG AI AL AM AN AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH EL ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS XI YE YT ZA ZM ZW ";
const VAT_CATEGORY_UNCL5305_CODES: &str = " AE L M E S Z G O K B ";

const IMPLEMENTED_RULE_IDS: &[&str] = &[
    "BR-01", "BR-02", "BR-03", "BR-04", "BR-05", "BR-06", "BR-07", "BR-08", "BR-09", "BR-10",
    "BR-11", "BR-12", "BR-13", "BR-14", "BR-15", "BR-16", "BR-17", "BR-18", "BR-19", "BR-20",
    "BR-21", "BR-22", "BR-23", "BR-24", "BR-25", "BR-26", "BR-27", "BR-28", "BR-29", "BR-30",
    "BR-31", "BR-32", "BR-33", "BR-36", "BR-37", "BR-38", "BR-41", "BR-42", "BR-43", "BR-44",
    "BR-45", "BR-46", "BR-47", "BR-48", "BR-49", "BR-50", "BR-51", "BR-52", "BR-53", "BR-54",
    "BR-55", "BR-56", "BR-57", "BR-61", "BR-62", "BR-63", "BR-64", "BR-65", "BR-CO-03", "BR-CO-04",
    "BR-AE-05", "BR-AE-08", "BR-AE-10", "BR-CL-17", "BR-CL-18", "BR-CO-05", "BR-CO-06", "BR-CO-07",
    "BR-CO-08", "BR-CO-09", "BR-CO-10", "BR-CO-11", "BR-CO-12", "BR-CO-13", "BR-CO-14", "BR-CO-15",
    "BR-CO-16", "BR-CO-17", "BR-CO-18", "BR-CO-19", "BR-CO-20", "BR-CO-21", "BR-CO-22", "BR-CO-23",
    "BR-CO-24", "BR-CO-26",
];

const DEFERRED_RULE_IDS: &[&str] = &[];

/// EN 16931 profile URN used for the global rulepack lookup.
pub const EN16931_PROFILE_URN: &str = "urn:cen.eu:en16931:2017";

const DEFAULT_RULEPACK_COUNTRY: &str = "global";
const LATEST_RULEPACK_LOOKUP_DATE: &str = "9999-12-31";

/// Options controlling EN 16931 validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationOptions {
    /// Optional ISO `YYYY-MM-DD` date used to select an effective rulepack.
    pub validation_date: Option<String>,
    /// Rulepack country selector. Defaults to `global` for EN 16931.
    pub country: String,
    /// Rulepack profile selector. Defaults to [`EN16931_PROFILE_URN`].
    pub profile: String,
}

impl ValidationOptions {
    /// Use a specific effective date for rulepack selection.
    #[must_use]
    pub fn with_validation_date(mut self, validation_date: impl Into<String>) -> Self {
        self.validation_date = Some(validation_date.into());
        self
    }
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self {
            validation_date: None,
            country: DEFAULT_RULEPACK_COUNTRY.to_owned(),
            profile: EN16931_PROFILE_URN.to_owned(),
        }
    }
}

/// Rulepack selected for a validation run, persisted for audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RulepackAudit {
    /// Rulepack identifier selected from the registry.
    pub rulepack_id: String,
    /// Upstream artifact version.
    pub upstream_version: String,
    /// Inclusive effective-window start.
    pub effective_from: String,
    /// Optional inclusive effective-window end.
    pub effective_to: Option<String>,
    /// Source URL carried by the rulepack manifest.
    pub source_url: String,
    /// Date the upstream artifact was retrieved.
    pub retrieved_at: String,
    /// Signature algorithm used by the manifest.
    pub signature_alg: String,
    /// Date selector used by the caller, or `latest` for default validation.
    pub selected_for_date: String,
    /// Rule ids disabled by the selected rulepack body policy.
    pub disabled_rules: Vec<String>,
}

/// XML syntax family accepted by the validator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentSyntax {
    /// Universal Business Language 2.1 `Invoice` or `CreditNote`.
    Ubl,
    /// UN/CEFACT Cross Industry Invoice.
    Cii,
}

/// EN 16931 coverage state represented by this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct En16931Coverage {
    /// Total BR/BR-CO rules in the checked coverage matrix.
    pub total: usize,
    /// Rules implemented by this crate.
    pub implemented: usize,
    /// Rules deliberately deferred pending the remaining T-031 validator slice.
    pub deferred_ir_gap: usize,
}

impl En16931Coverage {
    /// Coverage constants for the current rule-pack matrix.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            total: COVERAGE_RULE_TOTAL,
            implemented: COVERAGE_IMPLEMENTED_NOW,
            deferred_ir_gap: COVERAGE_DEFERRED_IR_GAP,
        }
    }
}

/// Successful validation pass output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct En16931Report {
    /// Parsed XML syntax family.
    pub syntax: DocumentSyntax,
    /// Rule findings emitted by implemented BR/BR-CO checks.
    pub findings: Vec<ValidationResult>,
    /// Rule inventory counts tied to the checked coverage matrix.
    pub coverage: En16931Coverage,
    /// Rulepack selected for this validation run.
    pub rulepack: RulepackAudit,
}

/// Rule deferred by the current Rust validator implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeferredRule {
    /// Rule identifier.
    pub id: &'static str,
    /// Reason the rule is not implemented in this crate yet.
    pub reason: &'static str,
}

/// Errors emitted by the Rust EN 16931 validator.
#[derive(Debug, Error)]
pub enum En16931Error {
    /// XML decoding failed.
    #[error("xml error: {0}")]
    Xml(#[from] quick_xml::Error),
    /// XML text decoding failed.
    #[error("xml encoding error: {0}")]
    Encoding(#[from] EncodingError),
    /// XML attribute decoding failed.
    #[error("xml attribute error: {0}")]
    Attribute(#[from] AttrError),
    /// XML used a non-UTF-8 name.
    #[error("xml name is not UTF-8")]
    Utf8(#[from] str::Utf8Error),
    /// XML had no supported document root.
    #[error(
        "unsupported document root `{0}`; expected UBL Invoice/CreditNote or CrossIndustryInvoice"
    )]
    UnsupportedRoot(String),
    /// XML nesting was malformed.
    #[error("malformed XML nesting near `{0}`")]
    MalformedXml(String),
    /// A compile-time validation-result constant was invalid.
    #[error("validation result construction failed: {0}")]
    Validation(#[from] ValidateError),
    /// Rulepack registry or manifest failed.
    #[error("rulepack error: {0}")]
    Rulepack(#[from] RulepackError),
    /// Validation date did not use `YYYY-MM-DD`.
    #[error("invalid validation date `{0}`; expected a valid YYYY-MM-DD calendar date")]
    InvalidValidationDate(String),
    /// No rulepack covers the requested selector.
    #[error("no rulepack covers country `{country}`, profile `{profile}`, date `{date}`")]
    RulepackNotFound {
        /// Rulepack country selector.
        country: String,
        /// Rulepack profile selector.
        profile: String,
        /// Requested effective date.
        date: String,
    },
    /// Rulepack body policy is malformed.
    #[error("rulepack `{rulepack_id}` has invalid policy: {message}")]
    RulepackPolicy {
        /// Rulepack identifier.
        rulepack_id: String,
        /// Human-readable policy error.
        message: String,
    },
}

/// Return the implemented BR/BR-CO rule identifiers.
#[must_use]
pub const fn implemented_rule_ids() -> &'static [&'static str] {
    IMPLEMENTED_RULE_IDS
}

/// Return the rule identifiers intentionally deferred by current IR gaps.
#[must_use]
pub fn deferred_rules() -> Vec<DeferredRule> {
    DEFERRED_RULE_IDS
        .iter()
        .copied()
        .map(|id| DeferredRule {
            id,
            reason: "Deferred pending typed IR or reference-validator parity work in T-031",
        })
        .collect()
}

/// Validate UBL or CII XML against the implemented EN 16931 BR/BR-CO rules.
///
/// # Errors
///
/// Returns [`En16931Error`] when the XML cannot be parsed or the root syntax
/// is not UBL `Invoice`, UBL `CreditNote`, or CII `CrossIndustryInvoice`.
pub fn validate_xml(input: &str) -> Result<En16931Report, En16931Error> {
    validate_xml_with_options(input, &ValidationOptions::default())
}

/// Validate XML against the EN 16931 rulepack effective on `validation_date`.
///
/// # Errors
///
/// Returns [`En16931Error`] when XML parsing, date validation, rulepack
/// selection, or rule evaluation fails.
pub fn validate_xml_on_date(
    input: &str,
    validation_date: impl Into<String>,
) -> Result<En16931Report, En16931Error> {
    validate_xml_with_options(
        input,
        &ValidationOptions::default().with_validation_date(validation_date),
    )
}

/// Validate XML with explicit rulepack selection options.
///
/// # Errors
///
/// Returns [`En16931Error`] when XML parsing, date validation, rulepack
/// selection, or rule evaluation fails.
pub fn validate_xml_with_options(
    input: &str,
    options: &ValidationOptions,
) -> Result<En16931Report, En16931Error> {
    let registry = Registry::seeded()?;
    validate_xml_with_registry(input, options, &registry)
}

/// Validate XML with a caller-supplied rulepack registry.
///
/// This is primarily used by tests and by future hot-reload integrations that
/// need to validate against a freshly loaded registry snapshot.
///
/// # Errors
///
/// Returns [`En16931Error`] when XML parsing, date validation, rulepack
/// selection, or rule evaluation fails.
pub fn validate_xml_with_registry(
    input: &str,
    options: &ValidationOptions,
    registry: &Registry,
) -> Result<En16931Report, En16931Error> {
    let lookup_date = options
        .validation_date
        .as_deref()
        .unwrap_or(LATEST_RULEPACK_LOOKUP_DATE);
    validate_iso_date(lookup_date)?;
    let selected_for_date = options
        .validation_date
        .clone()
        .unwrap_or_else(|| "latest".to_owned());
    let manifest = registry
        .pack_for(&options.country, &options.profile, lookup_date)
        .ok_or_else(|| En16931Error::RulepackNotFound {
            country: options.country.clone(),
            profile: options.profile.clone(),
            date: lookup_date.to_owned(),
        })?;
    let policy = RulepackPolicy::from_manifest(manifest)?;

    let root = parse_xml(input)?;
    let syntax = match (root.name.as_str(), root.namespace_uri.as_deref()) {
        ("Invoice", Some(UBL_INVOICE_NAMESPACE_URI))
        | ("CreditNote", Some(UBL_CREDIT_NOTE_NAMESPACE_URI)) => DocumentSyntax::Ubl,
        ("CrossIndustryInvoice", Some(CII_RSM_NAMESPACE_URI)) => DocumentSyntax::Cii,
        (name, namespace) => {
            return Err(En16931Error::UnsupportedRoot(format!(
                "{name} [{}]",
                namespace.unwrap_or("unbound")
            )))
        }
    };
    let line_nodes = collect_lines(syntax, &root);
    let tax_summary_nodes = collect_tax_summaries(syntax, &root);
    let tax_representative_nodes = collect_tax_representatives(syntax, &root);
    let payment_means_nodes = collect_payment_means(syntax, &root);
    let allowance_nodes = collect_document_allowance_charges(syntax, &root, false);
    let charge_nodes = collect_document_allowance_charges(syntax, &root, true);
    let invoice_period_nodes = collect_invoice_periods(syntax, &root);
    let ctx = ValidationContext {
        syntax,
        root: &root,
        line_nodes,
        tax_summary_nodes,
        tax_representative_nodes,
        payment_means_nodes,
        allowance_nodes,
        charge_nodes,
        invoice_period_nodes,
    };
    let mut findings = Vec::new();

    run_br_rules(&ctx, &mut findings)?;
    run_br_ae_rules(&ctx, &mut findings)?;
    run_br_cl_rules(&ctx, &mut findings)?;
    run_br_co_rules(&ctx, &mut findings)?;
    policy.retain_enabled_findings(&mut findings);

    Ok(En16931Report {
        syntax,
        findings,
        coverage: En16931Coverage::current(),
        rulepack: RulepackAudit::from_manifest(manifest, selected_for_date, &policy),
    })
}

#[derive(Debug)]
struct RulepackPolicy {
    disabled_all: bool,
    disabled_rules: BTreeSet<String>,
}

impl RulepackPolicy {
    fn from_manifest(manifest: &Manifest) -> Result<Self, En16931Error> {
        let Some(disabled) = manifest.body.get("disabled_rules") else {
            return Ok(Self {
                disabled_all: false,
                disabled_rules: BTreeSet::new(),
            });
        };
        let rules = disabled
            .as_array()
            .ok_or_else(|| En16931Error::RulepackPolicy {
                rulepack_id: manifest.rulepack_id.clone(),
                message: "`disabled_rules` must be an array".to_owned(),
            })?;
        let mut disabled_all = false;
        let mut disabled_rules = BTreeSet::new();
        for rule in rules {
            let rule = rule.as_str().ok_or_else(|| En16931Error::RulepackPolicy {
                rulepack_id: manifest.rulepack_id.clone(),
                message: "`disabled_rules` entries must be strings".to_owned(),
            })?;
            if rule == "*" {
                disabled_all = true;
            } else {
                disabled_rules.insert(rule.to_owned());
            }
        }
        Ok(Self {
            disabled_all,
            disabled_rules,
        })
    }

    fn retain_enabled_findings(&self, findings: &mut Vec<ValidationResult>) {
        if self.disabled_all {
            findings.clear();
            return;
        }
        findings.retain(|finding| !self.disabled_rules.contains(finding.rule_id.as_str()));
    }

    fn disabled_rules_for_audit(&self) -> Vec<String> {
        if self.disabled_all {
            vec!["*".to_owned()]
        } else {
            self.disabled_rules.iter().cloned().collect()
        }
    }
}

impl RulepackAudit {
    fn from_manifest(
        manifest: &Manifest,
        selected_for_date: String,
        policy: &RulepackPolicy,
    ) -> Self {
        Self {
            rulepack_id: manifest.rulepack_id.clone(),
            upstream_version: manifest.upstream_version.clone(),
            effective_from: manifest.effective_from.clone(),
            effective_to: manifest.effective_to.clone(),
            source_url: manifest.source_url.clone(),
            retrieved_at: manifest.retrieved_at.clone(),
            signature_alg: manifest.signature_alg.clone(),
            selected_for_date,
            disabled_rules: policy.disabled_rules_for_audit(),
        }
    }
}

fn validate_iso_date(value: &str) -> Result<(), En16931Error> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return Err(En16931Error::InvalidValidationDate(value.to_owned()));
    }
    let year = bytes
        .get(0..4)
        .and_then(parse_ascii_digits)
        .and_then(|year| i32::try_from(year).ok())
        .ok_or_else(|| En16931Error::InvalidValidationDate(value.to_owned()))?;
    let month = bytes
        .get(5..7)
        .and_then(parse_ascii_digits)
        .ok_or_else(|| En16931Error::InvalidValidationDate(value.to_owned()))?;
    let day = bytes
        .get(8..10)
        .and_then(parse_ascii_digits)
        .ok_or_else(|| En16931Error::InvalidValidationDate(value.to_owned()))?;
    if date_minutes(year, month, day, 0).is_none() {
        return Err(En16931Error::InvalidValidationDate(value.to_owned()));
    }
    Ok(())
}

#[derive(Debug)]
struct ValidationContext<'a> {
    syntax: DocumentSyntax,
    root: &'a XmlNode,
    line_nodes: Vec<&'a XmlNode>,
    tax_summary_nodes: Vec<&'a XmlNode>,
    // Document-level node sets that several BR/BR-CO rules re-derive. Each was
    // previously recomputed by re-walking the DOM (CII via recursive
    // `descendants`) on every call — `document_allowance_charges` alone is hit
    // 10x per validation. They are collected once here; the helpers below return
    // slices into these vectors. The traversal is identical to the per-call
    // version, so the cached node sets are the same nodes in the same order.
    tax_representative_nodes: Vec<&'a XmlNode>,
    payment_means_nodes: Vec<&'a XmlNode>,
    allowance_nodes: Vec<&'a XmlNode>,
    charge_nodes: Vec<&'a XmlNode>,
    invoice_period_nodes: Vec<&'a XmlNode>,
}

fn run_br_rules(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    br_01(ctx, findings)?;
    br_02(ctx, findings)?;
    br_03(ctx, findings)?;
    br_04(ctx, findings)?;
    br_05(ctx, findings)?;
    br_06(ctx, findings)?;
    br_07(ctx, findings)?;
    br_08(ctx, findings)?;
    br_09(ctx, findings)?;
    br_10(ctx, findings)?;
    br_11(ctx, findings)?;
    br_12(ctx, findings)?;
    br_13(ctx, findings)?;
    br_14(ctx, findings)?;
    br_15(ctx, findings)?;
    br_16(ctx, findings)?;
    br_17(ctx, findings)?;
    br_18(ctx, findings)?;
    br_19(ctx, findings)?;
    br_20(ctx, findings)?;
    br_21(ctx, findings)?;
    br_22(ctx, findings)?;
    br_23(ctx, findings)?;
    br_24(ctx, findings)?;
    br_25(ctx, findings)?;
    br_26(ctx, findings)?;
    br_27(ctx, findings)?;
    br_28(ctx, findings)?;
    br_29(ctx, findings)?;
    br_30(ctx, findings)?;
    br_31(ctx, findings)?;
    br_32(ctx, findings)?;
    br_33(ctx, findings)?;
    br_36(ctx, findings)?;
    br_37(ctx, findings)?;
    br_38(ctx, findings)?;
    br_41(ctx, findings)?;
    br_42(ctx, findings)?;
    br_43(ctx, findings)?;
    br_44(ctx, findings)?;
    br_45(ctx, findings)?;
    br_46(ctx, findings)?;
    br_47(ctx, findings)?;
    br_48(ctx, findings)?;
    br_49(ctx, findings)?;
    br_50(ctx, findings)?;
    br_51(ctx, findings)?;
    br_52(ctx, findings)?;
    br_53(ctx, findings)?;
    br_54(ctx, findings)?;
    br_55(ctx, findings)?;
    br_56(ctx, findings)?;
    br_57(ctx, findings)?;
    br_61(ctx, findings)?;
    br_62(ctx, findings)?;
    br_63(ctx, findings)?;
    br_64(ctx, findings)?;
    br_65(ctx, findings)
}

fn run_br_co_rules(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    br_co_03(ctx, findings)?;
    br_co_04(ctx, findings)?;
    br_co_05(ctx, findings);
    br_co_06(ctx, findings);
    br_co_07(ctx, findings);
    br_co_08(ctx, findings);
    br_co_09(ctx, findings)?;
    br_co_10(ctx, findings)?;
    br_co_11(ctx, findings)?;
    br_co_12(ctx, findings)?;
    br_co_13(ctx, findings)?;
    br_co_14(ctx, findings)?;
    br_co_15(ctx, findings)?;
    br_co_16(ctx, findings)?;
    br_co_17(ctx, findings)?;
    br_co_18(ctx, findings)?;
    br_co_19(ctx, findings)?;
    br_co_20(ctx, findings)?;
    br_co_21(ctx, findings)?;
    br_co_22(ctx, findings)?;
    br_co_23(ctx, findings)?;
    br_co_24(ctx, findings)?;
    br_co_26(ctx, findings)
}

fn run_br_ae_rules(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    br_ae_05(ctx, findings)?;
    br_ae_08(ctx, findings)?;
    br_ae_10(ctx, findings)
}

fn run_br_cl_rules(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    br_cl_17(ctx, findings)?;
    br_cl_18(ctx, findings)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct XmlAttribute {
    name: String,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct XmlNode {
    name: String,
    namespace_uri: Option<String>,
    attrs: Vec<XmlAttribute>,
    text: String,
    children: Vec<Self>,
}

impl XmlNode {
    fn new(name: String, namespace_uri: Option<String>, attrs: Vec<XmlAttribute>) -> Self {
        Self {
            name,
            namespace_uri,
            attrs,
            text: String::new(),
            children: Vec::new(),
        }
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name == name)
            .map(|attr| attr.value.as_str())
    }

    fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn descendants_named<'a>(&'a self, name: &str, out: &mut Vec<&'a Self>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.descendants_named(name, out);
        }
    }

    fn path<'a>(&'a self, path: &[&str]) -> Option<&'a Self> {
        let mut node = self;
        for part in path {
            node = node.child(part)?;
        }
        Some(node)
    }

    fn path_text(&self, path: &[&str]) -> Option<&str> {
        non_blank(self.path(path)?.text.as_str())
    }
}

fn parse_xml(input: &str) -> Result<XmlNode, En16931Error> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut xml_version = XmlVersion::Explicit1_0;
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root = None;

    loop {
        match reader.read_event()? {
            Event::Decl(decl) => {
                let version = decl.version()?;
                xml_version = if version.as_ref() == b"1.1" {
                    XmlVersion::Explicit1_1
                } else {
                    XmlVersion::Explicit1_0
                };
            }
            Event::Start(start) => {
                let is_root = stack.is_empty() && root.is_none();
                let name = local_name(start.name().as_ref())?;
                let namespace_uri = if is_root {
                    root_namespace_uri(&start, reader.decoder(), xml_version)?
                } else {
                    None
                };
                let node = XmlNode::new(
                    name,
                    namespace_uri,
                    read_attrs(reader.decoder(), &start, xml_version)?,
                );
                stack.push(node);
            }
            Event::Empty(start) => {
                let is_root = stack.is_empty() && root.is_none();
                let name = local_name(start.name().as_ref())?;
                let namespace_uri = if is_root {
                    root_namespace_uri(&start, reader.decoder(), xml_version)?
                } else {
                    None
                };
                let node = XmlNode::new(
                    name,
                    namespace_uri,
                    read_attrs(reader.decoder(), &start, xml_version)?,
                );
                push_node(&mut stack, &mut root, node)?;
            }
            Event::End(end) => {
                let name = local_name(end.name().as_ref())?;
                let Some(node) = stack.pop() else {
                    return Err(En16931Error::MalformedXml(name));
                };
                if node.name != name {
                    return Err(En16931Error::MalformedXml(format!(
                        "{} / {name}",
                        node.name
                    )));
                }
                push_node(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(text.xml_content(xml_version)?.as_ref());
                }
            }
            Event::CData(cdata) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(cdata.xml_content(xml_version)?.as_ref());
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push('&');
                    node.text
                        .push_str(reference.xml_content(xml_version)?.as_ref());
                    node.text.push(';');
                }
            }
            Event::DocType(_) => return Err(En16931Error::UnsupportedRoot("DOCTYPE".to_owned())),
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }

    if !stack.is_empty() {
        return Err(En16931Error::MalformedXml("unclosed element".to_owned()));
    }
    root.ok_or_else(|| En16931Error::UnsupportedRoot("empty document".to_owned()))
}

fn push_node(
    stack: &mut [XmlNode],
    root: &mut Option<XmlNode>,
    node: XmlNode,
) -> Result<(), En16931Error> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.is_none() {
        *root = Some(node);
    } else {
        return Err(En16931Error::MalformedXml("multiple roots".to_owned()));
    }
    Ok(())
}

fn read_attrs(
    decoder: Decoder,
    start: &BytesStart<'_>,
    xml_version: XmlVersion,
) -> Result<Vec<XmlAttribute>, En16931Error> {
    let mut attrs = Vec::new();
    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        let raw = str::from_utf8(attr.key.as_ref())?;
        if raw == "xmlns" || raw.starts_with("xmlns:") {
            continue;
        }
        let raw_name = local_name(attr.key.as_ref())?;
        let value = attr
            .decoded_and_normalized_value(xml_version, decoder)?
            .into_owned();
        attrs.push(XmlAttribute {
            name: raw_name,
            value,
        });
    }
    Ok(attrs)
}

fn root_namespace_uri(
    start: &BytesStart<'_>,
    decoder: Decoder,
    xml_version: XmlVersion,
) -> Result<Option<String>, En16931Error> {
    let raw_name = start.name();
    let qname = str::from_utf8(raw_name.as_ref())?;
    let namespace_attr = qname.rsplit_once(':').map_or("xmlns", |(prefix, _)| prefix);
    let prefixed_namespace_attr;
    let namespace_attr = if namespace_attr == "xmlns" {
        namespace_attr
    } else {
        prefixed_namespace_attr = format!("xmlns:{namespace_attr}");
        prefixed_namespace_attr.as_str()
    };

    for attr in start.attributes().with_checks(true) {
        let attr = attr?;
        if str::from_utf8(attr.key.as_ref())? == namespace_attr {
            return Ok(Some(
                attr.decoded_and_normalized_value(xml_version, decoder)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

fn local_name(raw: &[u8]) -> Result<String, En16931Error> {
    let name = str::from_utf8(raw)?;
    Ok(name
        .rsplit_once(':')
        .map_or(name, |(_, local)| local)
        .to_owned())
}

fn non_blank(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn fail(
    findings: &mut Vec<ValidationResult>,
    rule_id: &str,
    term: &str,
    location: &str,
    fix: &str,
) -> Result<(), En16931Error> {
    fail_with_severity(findings, rule_id, Severity::Error, term, location, fix)
}

fn fail_with_severity(
    findings: &mut Vec<ValidationResult>,
    rule_id: &str,
    severity: Severity,
    term: &str,
    location: &str,
    fix: &str,
) -> Result<(), En16931Error> {
    let term = if term.starts_with("BG-") {
        BusinessTerm::business_group(term)?
    } else {
        BusinessTerm::business_term(term)?
    };
    findings.push(
        ValidationResult::new(
            RuleId::new(rule_id)?,
            severity,
            term,
            Location::xpath(location)?,
            Citation::new(
                "ConnectingEurope/eInvoicing-EN16931 validation-1.3.16",
                rule_id,
                Some("https://github.com/ConnectingEurope/eInvoicing-EN16931".to_owned()),
            )?,
        )
        .with_suggested_fix(SuggestedFix::new(fix)?),
    );
    Ok(())
}

fn require_text(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
    rule_id: &str,
    term: &str,
    location: &str,
    value: Option<&str>,
    fix: &str,
) -> Result<(), En16931Error> {
    let _ = ctx;
    if value.is_none() {
        fail(findings, rule_id, term, location, fix)?;
    }
    Ok(())
}

fn descendants<'a>(node: &'a XmlNode, name: &str) -> Vec<&'a XmlNode> {
    let mut out = Vec::new();
    node.descendants_named(name, &mut out);
    out
}

fn lines<'ctx, 'doc>(ctx: &'ctx ValidationContext<'doc>) -> &'ctx [&'doc XmlNode] {
    &ctx.line_nodes
}

fn tax_summaries<'ctx, 'doc>(ctx: &'ctx ValidationContext<'doc>) -> &'ctx [&'doc XmlNode] {
    &ctx.tax_summary_nodes
}

fn collect_lines(syntax: DocumentSyntax, root: &XmlNode) -> Vec<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => root
            .children
            .iter()
            .filter(|child| child.name == "InvoiceLine" || child.name == "CreditNoteLine")
            .collect(),
        DocumentSyntax::Cii => descendants(root, "IncludedSupplyChainTradeLineItem"),
    }
}

fn collect_tax_summaries(syntax: DocumentSyntax, root: &XmlNode) -> Vec<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => descendants(root, "TaxSubtotal"),
        DocumentSyntax::Cii => root
            .path(&[
                "SupplyChainTradeTransaction",
                "ApplicableHeaderTradeSettlement",
            ])
            .map(|settlement| settlement.children_named("ApplicableTradeTax").collect())
            .unwrap_or_default(),
    }
}

fn differs_when_both_present(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left != right,
        _ => true,
    }
}

fn is_vat_code(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("VAT")
}

fn is_category_o(value: &str) -> bool {
    value.trim() == "O"
}

fn has_vat_tax_scheme(node: &XmlNode) -> bool {
    node.path_text(&["TaxScheme", "ID"])
        .is_some_and(is_vat_code)
}

fn ubl_vat_tax_category(tax: &XmlNode) -> Option<&XmlNode> {
    tax.path(&["TaxCategory"])
        .filter(|node| has_vat_tax_scheme(node))
}

fn cii_vat_tax(tax: &XmlNode) -> Option<&XmlNode> {
    tax.path_text(&["TypeCode"])
        .is_some_and(is_vat_code)
        .then_some(tax)
}

fn line_vat_tax_category(syntax: DocumentSyntax, line: &XmlNode) -> Option<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => line
            .path(&["Item", "ClassifiedTaxCategory"])
            .filter(|node| has_vat_tax_scheme(node)),
        DocumentSyntax::Cii => line
            .path(&["SpecifiedLineTradeSettlement", "ApplicableTradeTax"])
            .and_then(cii_vat_tax),
    }
}

fn tax_category_code_is_uncl5305(value: &str) -> bool {
    let normalized = value.trim();
    !normalized.contains(' ')
        && VAT_CATEGORY_UNCL5305_CODES
            .split_whitespace()
            .any(|code| code == normalized)
}

fn category_code(node: &XmlNode, syntax: DocumentSyntax) -> Option<&str> {
    match syntax {
        DocumentSyntax::Ubl => node.path_text(&["ID"]),
        DocumentSyntax::Cii => node.path_text(&["CategoryCode"]),
    }
}

fn category_rate(node: &XmlNode, syntax: DocumentSyntax) -> Option<Decimal> {
    match syntax {
        DocumentSyntax::Ubl => node.path_text(&["Percent"]).and_then(decimal),
        DocumentSyntax::Cii => node.path_text(&["RateApplicablePercent"]).and_then(decimal),
    }
}

fn has_tax_exemption_reason(node: &XmlNode, syntax: DocumentSyntax) -> bool {
    match syntax {
        DocumentSyntax::Ubl => {
            node.path_text(&["TaxExemptionReason"]).is_some()
                || node.path_text(&["TaxExemptionReasonCode"]).is_some()
        }
        DocumentSyntax::Cii => {
            node.path_text(&["ExemptionReason"]).is_some()
                || node.path_text(&["ExemptionReasonCode"]).is_some()
        }
    }
}

fn cii_header_settlement<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc XmlNode> {
    cii_header_settlement_of(ctx.root)
}

fn cii_header_settlement_of(root: &XmlNode) -> Option<&XmlNode> {
    root.path(&[
        "SupplyChainTradeTransaction",
        "ApplicableHeaderTradeSettlement",
    ])
}

fn cii_monetary_total<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc XmlNode> {
    cii_header_settlement(ctx)?.child("SpecifiedTradeSettlementHeaderMonetarySummation")
}

fn ubl_monetary_total<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc XmlNode> {
    ctx.root.child("LegalMonetaryTotal")
}

fn seller_party<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc XmlNode> {
    match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path(&["AccountingSupplierParty", "Party"]),
        DocumentSyntax::Cii => ctx.root.path(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "SellerTradeParty",
        ]),
    }
}

fn buyer_party<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc XmlNode> {
    match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path(&["AccountingCustomerParty", "Party"]),
        DocumentSyntax::Cii => ctx.root.path(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "BuyerTradeParty",
        ]),
    }
}

fn collect_tax_representatives(syntax: DocumentSyntax, root: &XmlNode) -> Vec<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => root.children_named("TaxRepresentativeParty").collect(),
        DocumentSyntax::Cii => descendants(root, "SellerTaxRepresentativeTradeParty"),
    }
}

fn tax_representatives<'ctx, 'doc>(ctx: &'ctx ValidationContext<'doc>) -> &'ctx [&'doc XmlNode] {
    &ctx.tax_representative_nodes
}

fn collect_payment_means(syntax: DocumentSyntax, root: &XmlNode) -> Vec<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => root.children_named("PaymentMeans").collect(),
        DocumentSyntax::Cii => descendants(root, "SpecifiedTradeSettlementPaymentMeans"),
    }
}

fn payment_means<'ctx, 'doc>(ctx: &'ctx ValidationContext<'doc>) -> &'ctx [&'doc XmlNode] {
    &ctx.payment_means_nodes
}

fn payment_code(payment: &XmlNode, syntax: DocumentSyntax) -> Option<&str> {
    match syntax {
        DocumentSyntax::Ubl => payment.path_text(&["PaymentMeansCode"]),
        DocumentSyntax::Cii => payment.path_text(&["TypeCode"]),
    }
}

fn payment_account_node(payment: &XmlNode, syntax: DocumentSyntax) -> Option<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => payment.child("PayeeFinancialAccount"),
        DocumentSyntax::Cii => payment.child("PayeePartyCreditorFinancialAccount"),
    }
}

fn payment_account_id(payment: &XmlNode, syntax: DocumentSyntax) -> Option<&str> {
    let account = payment_account_node(payment, syntax)?;
    match syntax {
        DocumentSyntax::Ubl => account.path_text(&["ID"]),
        DocumentSyntax::Cii => account
            .path_text(&["IBANID"])
            .or_else(|| account.path_text(&["ProprietaryID"])),
    }
}

fn is_credit_transfer_code(value: &str) -> bool {
    matches!(value.trim(), "30" | "58")
}

fn attr_is_non_blank(node: &XmlNode, name: &str) -> bool {
    node.attr(name).and_then(non_blank).is_some()
}

fn charge_indicator(node: &XmlNode) -> Option<&str> {
    node.path_text(&["ChargeIndicator"])
        .or_else(|| node.path_text(&["ChargeIndicator", "Indicator"]))
        .or_else(|| node.path_text(&["Indicator"]))
}

fn is_true_indicator(value: &str) -> bool {
    matches!(value.trim(), "true" | "1")
}

fn is_false_indicator(value: &str) -> bool {
    matches!(value.trim(), "false" | "0")
}

fn indicator_selects_charge(node: &XmlNode, charge: bool) -> bool {
    charge_indicator(node).is_some_and(|indicator| {
        if charge {
            is_true_indicator(indicator)
        } else {
            is_false_indicator(indicator)
        }
    })
}

fn has_allowance_charge_reason(node: &XmlNode, syntax: DocumentSyntax) -> bool {
    match syntax {
        DocumentSyntax::Ubl => {
            node.path_text(&["AllowanceChargeReason"]).is_some()
                || node.path_text(&["AllowanceChargeReasonCode"]).is_some()
        }
        DocumentSyntax::Cii => {
            node.path_text(&["Reason"]).is_some() || node.path_text(&["ReasonCode"]).is_some()
        }
    }
}

fn has_allowance_charge_amount(node: &XmlNode, syntax: DocumentSyntax) -> bool {
    match syntax {
        DocumentSyntax::Ubl => node.path_text(&["Amount"]).is_some(),
        DocumentSyntax::Cii => node.path_text(&["ActualAmount"]).is_some(),
    }
}

fn allowance_charge_amount(node: &XmlNode, syntax: DocumentSyntax) -> Option<Decimal> {
    match syntax {
        DocumentSyntax::Ubl => node.path_text(&["Amount"]).and_then(decimal),
        DocumentSyntax::Cii => node.path_text(&["ActualAmount"]).and_then(decimal),
    }
}

fn has_allowance_charge_vat_category(node: &XmlNode, syntax: DocumentSyntax) -> bool {
    match syntax {
        DocumentSyntax::Ubl => node
            .children_named("TaxCategory")
            .any(|category| has_vat_tax_scheme(category) && category.path_text(&["ID"]).is_some()),
        DocumentSyntax::Cii => node.children_named("CategoryTradeTax").any(|tax| {
            tax.path_text(&["TypeCode"]).is_some_and(is_vat_code)
                && tax.path_text(&["CategoryCode"]).is_some()
        }),
    }
}

fn collect_document_allowance_charges(
    syntax: DocumentSyntax,
    root: &XmlNode,
    charge: bool,
) -> Vec<&XmlNode> {
    let matches_indicator = |node: &&XmlNode| indicator_selects_charge(node, charge);
    match syntax {
        DocumentSyntax::Ubl => root
            .children_named("AllowanceCharge")
            .filter(matches_indicator)
            .collect(),
        DocumentSyntax::Cii => cii_header_settlement_of(root)
            .map(|settlement| {
                settlement
                    .children_named("SpecifiedTradeAllowanceCharge")
                    .filter(matches_indicator)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn document_allowance_charges<'ctx, 'doc>(
    ctx: &'ctx ValidationContext<'doc>,
    charge: bool,
) -> &'ctx [&'doc XmlNode] {
    if charge {
        &ctx.charge_nodes
    } else {
        &ctx.allowance_nodes
    }
}

fn line_allowance_charges<'doc>(
    ctx: &ValidationContext<'doc>,
    line: &'doc XmlNode,
    charge: bool,
) -> Vec<&'doc XmlNode> {
    let matches_indicator = |node: &&XmlNode| indicator_selects_charge(node, charge);
    match ctx.syntax {
        DocumentSyntax::Ubl => line
            .children_named("AllowanceCharge")
            .filter(matches_indicator)
            .collect(),
        DocumentSyntax::Cii => line
            .path(&["SpecifiedLineTradeSettlement"])
            .map(|settlement| {
                settlement
                    .children_named("SpecifiedTradeAllowanceCharge")
                    .filter(matches_indicator)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn document_allowance_charges_all<'doc>(ctx: &ValidationContext<'doc>) -> Vec<&'doc XmlNode> {
    match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.children_named("AllowanceCharge").collect(),
        DocumentSyntax::Cii => cii_header_settlement(ctx)
            .map(|settlement| {
                settlement
                    .children_named("SpecifiedTradeAllowanceCharge")
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn line_allowance_charges_all<'doc>(
    ctx: &ValidationContext<'doc>,
    line: &'doc XmlNode,
) -> Vec<&'doc XmlNode> {
    match ctx.syntax {
        DocumentSyntax::Ubl => line.children_named("AllowanceCharge").collect(),
        DocumentSyntax::Cii => line
            .path(&["SpecifiedLineTradeSettlement"])
            .map(|settlement| {
                settlement
                    .children_named("SpecifiedTradeAllowanceCharge")
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn collect_invoice_periods(syntax: DocumentSyntax, root: &XmlNode) -> Vec<&XmlNode> {
    match syntax {
        DocumentSyntax::Ubl => root.children_named("InvoicePeriod").collect(),
        DocumentSyntax::Cii => cii_header_settlement_of(root)
            .map(|settlement| {
                settlement
                    .children_named("BillingSpecifiedPeriod")
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn invoice_periods<'ctx, 'doc>(ctx: &'ctx ValidationContext<'doc>) -> &'ctx [&'doc XmlNode] {
    &ctx.invoice_period_nodes
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ComparableDate(i64);

fn period_start(period: &XmlNode, syntax: DocumentSyntax) -> (bool, Option<ComparableDate>) {
    match syntax {
        DocumentSyntax::Ubl => period_date(period, "StartDate", parse_ubl_date),
        DocumentSyntax::Cii => cii_period_date(period, "StartDateTime"),
    }
}

fn period_end(period: &XmlNode, syntax: DocumentSyntax) -> (bool, Option<ComparableDate>) {
    match syntax {
        DocumentSyntax::Ubl => period_date(period, "EndDate", parse_ubl_date),
        DocumentSyntax::Cii => cii_period_date(period, "EndDateTime"),
    }
}

fn period_date(
    period: &XmlNode,
    name: &str,
    parse: fn(&str) -> Option<ComparableDate>,
) -> (bool, Option<ComparableDate>) {
    let Some(node) = period.child(name) else {
        return (false, None);
    };
    (true, non_blank(node.text.as_str()).and_then(parse))
}

fn cii_period_date(period: &XmlNode, name: &str) -> (bool, Option<ComparableDate>) {
    let Some(node) = period.child(name) else {
        return (false, None);
    };
    let date = node
        .child("DateTimeString")
        .filter(|date| date.attr("format") == Some("102"))
        .and_then(|date| non_blank(date.text.as_str()))
        .and_then(parse_cii_date);
    (true, date)
}

fn parse_ubl_date(value: &str) -> Option<ComparableDate> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = i32::try_from(parse_ascii_digits(bytes.get(0..4)?)?).ok()?;
    let month = parse_ascii_digits(bytes.get(5..7)?)?;
    let day = parse_ascii_digits(bytes.get(8..10)?)?;
    let timezone = bytes.get(10..)?;
    let offset = match timezone {
        b"" | b"Z" => 0,
        timezone if timezone.len() == 6 => {
            let sign = match *timezone.first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if timezone.get(3) != Some(&b':') {
                return None;
            }
            let hours = i64::from(parse_ascii_digits(timezone.get(1..3)?)?);
            let minutes = i64::from(parse_ascii_digits(timezone.get(4..6)?)?);
            if hours > 14 || minutes > 59 {
                return None;
            }
            sign * (hours * 60 + minutes)
        }
        _ => return None,
    };
    date_minutes(year, month, day, offset)
}

fn parse_cii_date(value: &str) -> Option<ComparableDate> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 8 {
        return None;
    }
    let year = i32::try_from(parse_ascii_digits(bytes.get(0..4)?)?).ok()?;
    let month = parse_ascii_digits(bytes.get(4..6)?)?;
    let day = parse_ascii_digits(bytes.get(6..8)?)?;
    date_minutes(year, month, day, 0)
}

fn parse_ascii_digits(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        if byte.is_ascii_digit() {
            Some(value * 10 + u32::from(byte - b'0'))
        } else {
            None
        }
    })
}

fn date_minutes(year: i32, month: u32, day: u32, offset_minutes: i64) -> Option<ComparableDate> {
    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let month = i32::try_from(month).ok()?;
    let day = i32::try_from(day).ok()?;
    Some(ComparableDate(
        days_from_civil(year, month, day) * 1_440 - offset_minutes,
    ))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146_097 + doe - 719_468)
}

fn is_valid_vat_identifier_prefix(value: &str) -> bool {
    let prefix: String = value.chars().take(2).collect();
    let needle = format!(" {prefix} ");
    prefix.len() == 2 && VAT_ID_COUNTRY_PREFIXES.contains(needle.as_str())
}

fn document_currency<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc str> {
    match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["DocumentCurrencyCode"]),
        DocumentSyntax::Cii => cii_header_settlement(ctx)
            .and_then(|settlement| settlement.path_text(&["InvoiceCurrencyCode"])),
    }
}

fn tax_currency<'doc>(ctx: &ValidationContext<'doc>) -> Option<&'doc str> {
    match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["TaxCurrencyCode"]),
        DocumentSyntax::Cii => cii_header_settlement(ctx)
            .and_then(|settlement| settlement.path_text(&["TaxCurrencyCode"])),
    }
}

fn ubl_tax_amounts_for_currency<'a>(root: &'a XmlNode, currency: &str) -> Vec<&'a XmlNode> {
    root.children_named("TaxTotal")
        .filter_map(|tax_total| tax_total.child("TaxAmount"))
        .filter(|amount| amount.attr("currencyID") == Some(currency))
        .collect()
}

fn cii_tax_total_amounts_for_currency<'a>(
    ctx: &ValidationContext<'a>,
    currency: &str,
) -> Vec<&'a XmlNode> {
    cii_monetary_total(ctx)
        .map(|total| {
            total
                .children_named("TaxTotalAmount")
                .filter(|amount| amount.attr("currencyID") == Some(currency))
                .collect()
        })
        .unwrap_or_default()
}

fn cii_tax_total_amount_for_currency<'a>(
    ctx: &ValidationContext<'a>,
    currency: &str,
) -> Option<&'a XmlNode> {
    cii_tax_total_amounts_for_currency(ctx, currency)
        .into_iter()
        .next()
}

fn header_amount(ctx: &ValidationContext<'_>, name: &str) -> Option<Decimal> {
    match ctx.syntax {
        DocumentSyntax::Ubl => decimal(ubl_monetary_total(ctx)?.path_text(&[name])?),
        DocumentSyntax::Cii => decimal(cii_monetary_total(ctx)?.path_text(&[name])?),
    }
}

fn line_amounts(ctx: &ValidationContext<'_>) -> Vec<Decimal> {
    lines(ctx)
        .iter()
        .copied()
        .filter_map(|line| match ctx.syntax {
            DocumentSyntax::Ubl => decimal(line.path_text(&["LineExtensionAmount"])?),
            DocumentSyntax::Cii => decimal(line.path_text(&[
                "SpecifiedLineTradeSettlement",
                "SpecifiedTradeSettlementLineMonetarySummation",
                "LineTotalAmount",
            ])?),
        })
        .collect()
}

fn decimal(value: &str) -> Option<Decimal> {
    value.trim().parse::<Decimal>().ok()
}

fn rounded_2(value: Decimal) -> Decimal {
    value.round_dp(2)
}

fn rounded_0(value: Decimal) -> Decimal {
    value.round_dp(0)
}

fn br_co_17_within_tolerance(syntax: DocumentSyntax, actual: Decimal, expected: Decimal) -> bool {
    match syntax {
        DocumentSyntax::Ubl => actual - Decimal::ONE < expected && actual + Decimal::ONE > expected,
        DocumentSyntax::Cii => {
            actual - Decimal::ONE <= expected && actual + Decimal::ONE >= expected
        }
    }
}

fn br_01(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["CustomizationID"]),
        DocumentSyntax::Cii => ctx.root.path_text(&[
            "ExchangedDocumentContext",
            "GuidelineSpecifiedDocumentContextParameter",
            "ID",
        ]),
    };
    require_text(
        ctx,
        findings,
        "BR-01",
        "BT-24",
        "/document/profile",
        value,
        "Set the EN 16931 profile specification identifier",
    )
}

fn br_02(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["ID"]),
        DocumentSyntax::Cii => ctx.root.path_text(&["ExchangedDocument", "ID"]),
    };
    require_text(
        ctx,
        findings,
        "BR-02",
        "BT-1",
        "/document/id",
        value,
        "Set the invoice number",
    )
}

fn br_03(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["IssueDate"]),
        DocumentSyntax::Cii => ctx
            .root
            .path(&["ExchangedDocument", "IssueDateTime", "DateTimeString"])
            .filter(|node| node.attr("format") == Some("102"))
            .and_then(|node| non_blank(node.text.as_str())),
    };
    require_text(
        ctx,
        findings,
        "BR-03",
        "BT-2",
        "/document/issue_date",
        value,
        "Set the invoice issue date",
    )
}

fn br_04(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx
            .root
            .path_text(&["InvoiceTypeCode"])
            .or_else(|| ctx.root.path_text(&["CreditNoteTypeCode"])),
        DocumentSyntax::Cii => ctx.root.path_text(&["ExchangedDocument", "TypeCode"]),
    };
    require_text(
        ctx,
        findings,
        "BR-04",
        "BT-3",
        "/document/type_code",
        value,
        "Set the invoice type code",
    )
}

fn br_05(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&["DocumentCurrencyCode"]),
        DocumentSyntax::Cii => {
            cii_header_settlement(ctx).and_then(|s| s.path_text(&["InvoiceCurrencyCode"]))
        }
    };
    require_text(
        ctx,
        findings,
        "BR-05",
        "BT-5",
        "/document/currency",
        value,
        "Set the document currency code",
    )
}

fn br_06(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&[
            "AccountingSupplierParty",
            "Party",
            "PartyLegalEntity",
            "RegistrationName",
        ]),
        DocumentSyntax::Cii => ctx.root.path_text(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "SellerTradeParty",
            "Name",
        ]),
    };
    require_text(
        ctx,
        findings,
        "BR-06",
        "BT-27",
        "/supplier/name",
        value,
        "Set the seller name",
    )
}

fn br_07(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&[
            "AccountingCustomerParty",
            "Party",
            "PartyLegalEntity",
            "RegistrationName",
        ]),
        DocumentSyntax::Cii => ctx.root.path_text(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "BuyerTradeParty",
            "Name",
        ]),
    };
    require_text(
        ctx,
        findings,
        "BR-07",
        "BT-44",
        "/customer/name",
        value,
        "Set the buyer name",
    )
}

fn br_08(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let present = match ctx.syntax {
        DocumentSyntax::Ubl => ctx
            .root
            .path(&["AccountingSupplierParty", "Party", "PostalAddress"])
            .is_some(),
        DocumentSyntax::Cii => ctx
            .root
            .path(&[
                "SupplyChainTradeTransaction",
                "ApplicableHeaderTradeAgreement",
                "SellerTradeParty",
                "PostalTradeAddress",
            ])
            .is_some(),
    };
    if !present {
        fail(
            findings,
            "BR-08",
            "BG-5",
            "/supplier/address",
            "Add the seller postal address",
        )?;
    }
    Ok(())
}

fn br_09(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&[
            "AccountingSupplierParty",
            "Party",
            "PostalAddress",
            "Country",
            "IdentificationCode",
        ]),
        DocumentSyntax::Cii => ctx.root.path_text(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "SellerTradeParty",
            "PostalTradeAddress",
            "CountryID",
        ]),
    };
    require_text(
        ctx,
        findings,
        "BR-09",
        "BT-40",
        "/supplier/address/country",
        value,
        "Set the seller country code",
    )
}

fn br_10(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let present = match ctx.syntax {
        DocumentSyntax::Ubl => ctx
            .root
            .path(&["AccountingCustomerParty", "Party", "PostalAddress"])
            .is_some(),
        DocumentSyntax::Cii => ctx
            .root
            .path(&[
                "SupplyChainTradeTransaction",
                "ApplicableHeaderTradeAgreement",
                "BuyerTradeParty",
                "PostalTradeAddress",
            ])
            .is_some(),
    };
    if !present {
        fail(
            findings,
            "BR-10",
            "BG-8",
            "/customer/address",
            "Add the buyer postal address",
        )?;
    }
    Ok(())
}

fn br_11(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.path_text(&[
            "AccountingCustomerParty",
            "Party",
            "PostalAddress",
            "Country",
            "IdentificationCode",
        ]),
        DocumentSyntax::Cii => ctx.root.path_text(&[
            "SupplyChainTradeTransaction",
            "ApplicableHeaderTradeAgreement",
            "BuyerTradeParty",
            "PostalTradeAddress",
            "CountryID",
        ]),
    };
    require_text(
        ctx,
        findings,
        "BR-11",
        "BT-55",
        "/customer/address/country",
        value,
        "Set the buyer country code",
    )
}

fn br_12(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => {
            ubl_monetary_total(ctx).and_then(|n| n.path_text(&["LineExtensionAmount"]))
        }
        DocumentSyntax::Cii => {
            cii_monetary_total(ctx).and_then(|n| n.path_text(&["LineTotalAmount"]))
        }
    };
    require_text(
        ctx,
        findings,
        "BR-12",
        "BT-106",
        "/monetary_total/line_extension_amount",
        value,
        "Set the sum of invoice line net amounts",
    )
}

fn br_13(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => {
            ubl_monetary_total(ctx).and_then(|n| n.path_text(&["TaxExclusiveAmount"]))
        }
        DocumentSyntax::Cii => {
            cii_monetary_total(ctx).and_then(|n| n.path_text(&["TaxBasisTotalAmount"]))
        }
    };
    require_text(
        ctx,
        findings,
        "BR-13",
        "BT-109",
        "/monetary_total/tax_exclusive_amount",
        value,
        "Set the invoice total without VAT",
    )
}

fn br_14(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => {
            ubl_monetary_total(ctx).and_then(|n| n.path_text(&["TaxInclusiveAmount"]))
        }
        DocumentSyntax::Cii => {
            cii_monetary_total(ctx).and_then(|n| n.path_text(&["GrandTotalAmount"]))
        }
    };
    require_text(
        ctx,
        findings,
        "BR-14",
        "BT-112",
        "/monetary_total/tax_inclusive_amount",
        value,
        "Set the invoice total with VAT",
    )
}

fn br_15(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let value = match ctx.syntax {
        DocumentSyntax::Ubl => {
            ubl_monetary_total(ctx).and_then(|n| n.path_text(&["PayableAmount"]))
        }
        DocumentSyntax::Cii => {
            cii_monetary_total(ctx).and_then(|n| n.path_text(&["DuePayableAmount"]))
        }
    };
    require_text(
        ctx,
        findings,
        "BR-15",
        "BT-115",
        "/monetary_total/payable_amount",
        value,
        "Set the amount due for payment",
    )
}

fn br_16(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    if lines(ctx).is_empty() {
        fail(
            findings,
            "BR-16",
            "BG-25",
            "/lines",
            "Add at least one invoice line",
        )?;
    }
    Ok(())
}

fn br_17(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let valid_payee = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.child("PayeeParty").map(|payee| {
            let seller = ctx.root.path(&["AccountingSupplierParty", "Party"]);
            let seller_name = seller.and_then(|node| node.path_text(&["PartyName", "Name"]));
            let seller_id = seller.and_then(|node| node.path_text(&["PartyIdentification", "ID"]));
            let payee_name = payee.path_text(&["PartyName", "Name"]);
            let payee_id = payee.path_text(&["PartyIdentification", "ID"]);

            payee_name.is_some()
                && differs_when_both_present(payee_name, seller_name)
                && differs_when_both_present(payee_id, seller_id)
        }),
        DocumentSyntax::Cii => ctx
            .root
            .path(&[
                "SupplyChainTradeTransaction",
                "ApplicableHeaderTradeSettlement",
                "PayeeTradeParty",
            ])
            .map(|payee| {
                let seller = ctx.root.path(&[
                    "SupplyChainTradeTransaction",
                    "ApplicableHeaderTradeAgreement",
                    "SellerTradeParty",
                ]);
                let seller_name = seller.and_then(|node| node.path_text(&["Name"]));
                let seller_id = seller.and_then(|node| node.path_text(&["ID"]));
                let seller_legal_id =
                    seller.and_then(|node| node.path_text(&["SpecifiedLegalOrganization", "ID"]));
                let payee_name = payee.path_text(&["Name"]);
                let payee_id = payee.path_text(&["ID"]);
                let payee_legal_id = payee.path_text(&["SpecifiedLegalOrganization", "ID"]);

                payee_name.is_some()
                    && differs_when_both_present(payee_name, seller_name)
                    && differs_when_both_present(payee_id, seller_id)
                    && differs_when_both_present(payee_legal_id, seller_legal_id)
            }),
    };
    if valid_payee == Some(false) {
        fail(
            findings,
            "BR-17",
            "BT-59",
            "/payee/name",
            "Set a payee name and identifier that differ from the seller when a payee party is present",
        )?;
    }
    Ok(())
}

fn br_18(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, representative) in tax_representatives(ctx).iter().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => representative.path_text(&["PartyName", "Name"]),
            DocumentSyntax::Cii => representative.path_text(&["Name"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-18",
                "BT-62",
                &format!("/seller/tax_representative/{index}/name"),
                "Set the seller tax representative name",
            )?;
        }
    }
    Ok(())
}

fn br_19(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, representative) in tax_representatives(ctx).iter().enumerate() {
        let present = match ctx.syntax {
            DocumentSyntax::Ubl => representative.child("PostalAddress").is_some(),
            DocumentSyntax::Cii => representative.child("PostalTradeAddress").is_some(),
        };
        if !present {
            fail(
                findings,
                "BR-19",
                "BG-12",
                &format!("/seller/tax_representative/{index}/address"),
                "Add the seller tax representative postal address",
            )?;
        }
    }
    Ok(())
}

fn br_20(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, representative) in tax_representatives(ctx).iter().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl if representative.child("PostalAddress").is_some() => {
                representative.path_text(&["PostalAddress", "Country", "IdentificationCode"])
            }
            DocumentSyntax::Ubl => continue,
            DocumentSyntax::Cii => representative.path_text(&["PostalTradeAddress", "CountryID"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-20",
                "BT-69",
                &format!("/seller/tax_representative/{index}/address/country"),
                "Set the seller tax representative country code",
            )?;
        }
    }
    Ok(())
}

fn br_21(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line.path_text(&["ID"]),
            DocumentSyntax::Cii => line.path_text(&["AssociatedDocumentLineDocument", "LineID"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-21",
                "BT-126",
                &format!("/lines/{index}/id"),
                "Set the invoice line identifier",
            )?;
        }
    }
    Ok(())
}

fn br_22(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line
                .path_text(&["InvoicedQuantity"])
                .or_else(|| line.path_text(&["CreditedQuantity"])),
            DocumentSyntax::Cii => {
                line.path_text(&["SpecifiedLineTradeDelivery", "BilledQuantity"])
            }
        };
        if value.is_none() {
            fail(
                findings,
                "BR-22",
                "BT-129",
                &format!("/lines/{index}/quantity"),
                "Set the invoice line quantity",
            )?;
        }
    }
    Ok(())
}

fn br_23(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let unit = match ctx.syntax {
            DocumentSyntax::Ubl => line
                .child("InvoicedQuantity")
                .or_else(|| line.child("CreditedQuantity"))
                .and_then(|node| node.attr("unitCode"))
                .and_then(non_blank),
            DocumentSyntax::Cii => line
                .path(&["SpecifiedLineTradeDelivery", "BilledQuantity"])
                .and_then(|node| node.attr("unitCode"))
                .and_then(non_blank),
        };
        if unit.is_none() {
            fail(
                findings,
                "BR-23",
                "BT-130",
                &format!("/lines/{index}/unit_code"),
                "Set the invoice line unit code",
            )?;
        }
    }
    Ok(())
}

fn br_24(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line.path_text(&["LineExtensionAmount"]),
            DocumentSyntax::Cii => line.path_text(&[
                "SpecifiedLineTradeSettlement",
                "SpecifiedTradeSettlementLineMonetarySummation",
                "LineTotalAmount",
            ]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-24",
                "BT-131",
                &format!("/lines/{index}/line_extension_amount"),
                "Set the invoice line net amount",
            )?;
        }
    }
    Ok(())
}

fn br_25(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line.path_text(&["Item", "Name"]),
            DocumentSyntax::Cii => line.path_text(&["SpecifiedTradeProduct", "Name"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-25",
                "BT-153",
                &format!("/lines/{index}/description"),
                "Set the item name",
            )?;
        }
    }
    Ok(())
}

fn br_26(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line.path_text(&["Price", "PriceAmount"]),
            DocumentSyntax::Cii => line.path_text(&[
                "SpecifiedLineTradeAgreement",
                "NetPriceProductTradePrice",
                "ChargeAmount",
            ]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-26",
                "BT-146",
                &format!("/lines/{index}/unit_price"),
                "Set the item net price",
            )?;
        }
    }
    Ok(())
}

fn br_27(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let amount = match ctx.syntax {
            DocumentSyntax::Ubl => line.path_text(&["Price", "PriceAmount"]).and_then(decimal),
            DocumentSyntax::Cii => line
                .path_text(&[
                    "SpecifiedLineTradeAgreement",
                    "NetPriceProductTradePrice",
                    "ChargeAmount",
                ])
                .and_then(decimal),
        };
        if amount.is_some_and(|amount| amount < Decimal::ZERO) {
            fail(
                findings,
                "BR-27",
                "BT-146",
                &format!("/lines/{index}/unit_price"),
                "Set a non-negative item net price",
            )?;
        }
    }
    Ok(())
}

fn br_28(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let amount = match ctx.syntax {
            DocumentSyntax::Ubl => line
                .path_text(&["Price", "AllowanceCharge", "BaseAmount"])
                .and_then(decimal),
            DocumentSyntax::Cii => line
                .path_text(&[
                    "SpecifiedLineTradeAgreement",
                    "GrossPriceProductTradePrice",
                    "ChargeAmount",
                ])
                .and_then(decimal),
        };
        if amount.is_some_and(|amount| amount < Decimal::ZERO) {
            fail(
                findings,
                "BR-28",
                "BT-148",
                &format!("/lines/{index}/gross_price"),
                "Set a non-negative item gross price",
            )?;
        }
    }
    Ok(())
}

fn br_29(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, period) in invoice_periods(ctx).iter().enumerate() {
        let (has_start, start) = period_start(period, ctx.syntax);
        let (has_end, end) = period_end(period, ctx.syntax);
        if has_start && has_end && !matches!((start, end), (Some(start), Some(end)) if end >= start)
        {
            fail(
                findings,
                "BR-29",
                "BT-74",
                &format!("/invoice_periods/{index}/end_date"),
                "Set an invoicing period end date later than or equal to the start date",
            )?;
        }
    }
    Ok(())
}

fn br_30(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        let periods: Vec<&XmlNode> = match ctx.syntax {
            DocumentSyntax::Ubl => line.children_named("InvoicePeriod").collect(),
            DocumentSyntax::Cii => line
                .path(&["SpecifiedLineTradeSettlement"])
                .map(|settlement| {
                    settlement
                        .children_named("BillingSpecifiedPeriod")
                        .collect()
                })
                .unwrap_or_default(),
        };
        for (period_index, period) in periods.into_iter().enumerate() {
            let (has_start, start) = period_start(period, ctx.syntax);
            let (has_end, end) = period_end(period, ctx.syntax);
            if has_start
                && has_end
                && !matches!((start, end), (Some(start), Some(end)) if end >= start)
            {
                fail(
                    findings,
                    "BR-30",
                    "BT-135",
                    &format!("/lines/{line_index}/periods/{period_index}/end_date"),
                    "Set an invoice line period end date later than or equal to the start date",
                )?;
            }
        }
    }
    Ok(())
}

fn br_31(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, false).iter().enumerate() {
        if !has_allowance_charge_amount(charge, ctx.syntax) {
            fail(
                findings,
                "BR-31",
                "BT-92",
                &format!("/document_allowances/{index}/amount"),
                "Set the document-level allowance amount",
            )?;
        }
    }
    Ok(())
}

fn br_32(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, false).iter().enumerate() {
        if !has_allowance_charge_vat_category(charge, ctx.syntax) {
            fail(
                findings,
                "BR-32",
                "BT-95",
                &format!("/document_allowances/{index}/vat_category"),
                "Set the document-level allowance VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_33(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, false).iter().enumerate() {
        if !has_allowance_charge_reason(charge, ctx.syntax) {
            fail(
                findings,
                "BR-33",
                "BT-97",
                &format!("/document_allowances/{index}/reason"),
                "Set a document-level allowance reason or reason code",
            )?;
        }
    }
    Ok(())
}

fn br_36(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, true).iter().enumerate() {
        if !has_allowance_charge_amount(charge, ctx.syntax) {
            fail(
                findings,
                "BR-36",
                "BT-99",
                &format!("/document_charges/{index}/amount"),
                "Set the document-level charge amount",
            )?;
        }
    }
    Ok(())
}

fn br_37(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, true).iter().enumerate() {
        if !has_allowance_charge_vat_category(charge, ctx.syntax) {
            fail(
                findings,
                "BR-37",
                "BT-102",
                &format!("/document_charges/{index}/vat_category"),
                "Set the document-level charge VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_38(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, charge) in document_allowance_charges(ctx, true).iter().enumerate() {
        if !has_allowance_charge_reason(charge, ctx.syntax) {
            fail(
                findings,
                "BR-38",
                "BT-104",
                &format!("/document_charges/{index}/reason"),
                "Set a document-level charge reason or reason code",
            )?;
        }
    }
    Ok(())
}

fn br_41(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        for (index, charge) in line_allowance_charges(ctx, line, false)
            .into_iter()
            .enumerate()
        {
            if !has_allowance_charge_amount(charge, ctx.syntax) {
                fail(
                    findings,
                    "BR-41",
                    "BT-136",
                    &format!("/lines/{line_index}/allowances/{index}/amount"),
                    "Set the invoice line allowance amount",
                )?;
            }
        }
    }
    Ok(())
}

fn br_42(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        for (index, charge) in line_allowance_charges(ctx, line, false)
            .into_iter()
            .enumerate()
        {
            if !has_allowance_charge_reason(charge, ctx.syntax) {
                fail(
                    findings,
                    "BR-42",
                    "BT-139",
                    &format!("/lines/{line_index}/allowances/{index}/reason"),
                    "Set an invoice line allowance reason or reason code",
                )?;
            }
        }
    }
    Ok(())
}

fn br_43(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        for (index, charge) in line_allowance_charges(ctx, line, true)
            .into_iter()
            .enumerate()
        {
            if !has_allowance_charge_amount(charge, ctx.syntax) {
                fail(
                    findings,
                    "BR-43",
                    "BT-141",
                    &format!("/lines/{line_index}/charges/{index}/amount"),
                    "Set the invoice line charge amount",
                )?;
            }
        }
    }
    Ok(())
}

fn br_44(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        for (index, charge) in line_allowance_charges(ctx, line, true)
            .into_iter()
            .enumerate()
        {
            if !has_allowance_charge_reason(charge, ctx.syntax) {
                fail(
                    findings,
                    "BR-44",
                    "BT-144",
                    &format!("/lines/{line_index}/charges/{index}/reason"),
                    "Set an invoice line charge reason or reason code",
                )?;
            }
        }
    }
    Ok(())
}

fn br_45(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => tax.path_text(&["TaxableAmount"]),
            DocumentSyntax::Cii => tax.path_text(&["BasisAmount"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-45",
                "BT-116",
                &format!("/tax_summary/{index}/taxable_amount"),
                "Set the VAT category taxable amount",
            )?;
        }
    }
    Ok(())
}

fn br_46(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => tax.path_text(&["TaxAmount"]),
            DocumentSyntax::Cii => tax.path_text(&["CalculatedAmount"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-46",
                "BT-117",
                &format!("/tax_summary/{index}/tax_amount"),
                "Set the VAT category tax amount",
            )?;
        }
    }
    Ok(())
}

fn br_47(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => {
                ubl_vat_tax_category(tax).and_then(|node| node.path_text(&["ID"]))
            }
            DocumentSyntax::Cii => {
                cii_vat_tax(tax).and_then(|node| node.path_text(&["CategoryCode"]))
            }
        };
        if value.is_none() {
            fail(
                findings,
                "BR-47",
                "BT-118",
                &format!("/tax_summary/{index}/category_code"),
                "Set the VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_48(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let (category, rate) = match ctx.syntax {
            DocumentSyntax::Ubl => (
                ubl_vat_tax_category(tax).and_then(|node| node.path_text(&["ID"])),
                ubl_vat_tax_category(tax).and_then(|node| node.path_text(&["Percent"])),
            ),
            DocumentSyntax::Cii => (
                cii_vat_tax(tax).and_then(|node| node.path_text(&["CategoryCode"])),
                cii_vat_tax(tax).and_then(|node| node.path_text(&["RateApplicablePercent"])),
            ),
        };
        if !category.is_some_and(is_category_o) && rate.is_none() {
            fail(
                findings,
                "BR-48",
                "BT-119",
                &format!("/tax_summary/{index}/tax_rate"),
                "Set the VAT category rate unless the category is O",
            )?;
        }
    }
    Ok(())
}

fn br_49(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, payment) in payment_means(ctx).iter().enumerate() {
        if payment_code(payment, ctx.syntax).is_none() {
            fail(
                findings,
                "BR-49",
                "BT-81",
                &format!("/payment_means/{index}/type_code"),
                "Set the payment means type code",
            )?;
        }
    }
    Ok(())
}

fn br_50(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, payment) in payment_means(ctx).iter().enumerate() {
        let has_credit_transfer_code =
            payment_code(payment, ctx.syntax).is_some_and(is_credit_transfer_code);
        if has_credit_transfer_code
            && payment_account_node(payment, ctx.syntax).is_some()
            && payment_account_id(payment, ctx.syntax).is_none()
        {
            fail(
                findings,
                "BR-50",
                "BT-84",
                &format!("/payment_means/{index}/account/id"),
                "Set the payment account identifier",
            )?;
        }
    }
    Ok(())
}

fn br_51(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    match ctx.syntax {
        DocumentSyntax::Ubl => {
            for (index, card) in ctx
                .root
                .children_named("PaymentMeans")
                .filter_map(|payment| payment.child("CardAccount"))
                .enumerate()
            {
                if card
                    .path_text(&["PrimaryAccountNumberID"])
                    .is_some_and(|value| value.trim().chars().count() > 10)
                {
                    fail_with_severity(
                        findings,
                        "BR-51",
                        Severity::Warning,
                        "BT-87",
                        &format!("/payment_card/{index}/primary_account_number"),
                        "Store at most the first six and last four payment card digits",
                    )?;
                }
            }
        }
        DocumentSyntax::Cii => {
            for (index, card) in descendants(ctx.root, "ApplicableTradeSettlementFinancialCard")
                .into_iter()
                .enumerate()
            {
                if card
                    .path_text(&["ID"])
                    .is_some_and(|value| value.trim().chars().count() > 10)
                {
                    fail(
                        findings,
                        "BR-51",
                        "BT-87",
                        &format!("/payment_card/{index}/primary_account_number"),
                        "Store at most the first six and last four payment card digits",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_52(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let references = match ctx.syntax {
        DocumentSyntax::Ubl => ctx
            .root
            .children_named("AdditionalDocumentReference")
            .collect(),
        DocumentSyntax::Cii => descendants(ctx.root, "AdditionalReferencedDocument"),
    };
    for (index, reference) in references.into_iter().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => reference.path_text(&["ID"]),
            DocumentSyntax::Cii => reference.path_text(&["IssuerAssignedID"]),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-52",
                "BT-122",
                &format!("/supporting_documents/{index}/reference"),
                "Set the supporting document reference",
            )?;
        }
    }
    Ok(())
}

fn br_53(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let Some(tax_currency) = tax_currency(ctx) else {
        return Ok(());
    };
    let valid = match ctx.syntax {
        DocumentSyntax::Ubl => !ubl_tax_amounts_for_currency(ctx.root, tax_currency).is_empty(),
        DocumentSyntax::Cii => {
            let invoice_currency = document_currency(ctx);
            invoice_currency != Some(tax_currency)
                && cii_tax_total_amount_for_currency(ctx, tax_currency).is_some()
        }
    };
    if !valid {
        fail(
            findings,
            "BR-53",
            "BT-111",
            "/tax_total/accounting_currency_amount",
            "Provide the invoice total VAT amount in the VAT accounting currency",
        )?;
    }
    Ok(())
}

fn br_54(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    match ctx.syntax {
        DocumentSyntax::Ubl => {
            let mut index = 0;
            for line in lines(ctx) {
                let Some(item) = line.child("Item") else {
                    continue;
                };
                for attribute in item.children_named("AdditionalItemProperty") {
                    if attribute.path_text(&["Name"]).is_none()
                        || attribute.path_text(&["Value"]).is_none()
                    {
                        fail(
                            findings,
                            "BR-54",
                            "BG-32",
                            &format!("/item_attributes/{index}"),
                            "Set both the item attribute name and value",
                        )?;
                    }
                    index += 1;
                }
            }
        }
        DocumentSyntax::Cii => {
            for (index, attribute) in descendants(ctx.root, "ApplicableProductCharacteristic")
                .into_iter()
                .enumerate()
            {
                if attribute.path_text(&["Description"]).is_none()
                    || attribute.path_text(&["Value"]).is_none()
                {
                    fail(
                        findings,
                        "BR-54",
                        "BG-32",
                        &format!("/item_attributes/{index}"),
                        "Set both the item attribute name and value",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_55(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    match ctx.syntax {
        DocumentSyntax::Ubl => {
            for (index, reference) in ctx
                .root
                .children_named("BillingReference")
                .filter_map(|billing| billing.child("InvoiceDocumentReference"))
                .enumerate()
            {
                if reference.path_text(&["ID"]).is_none() {
                    fail(
                        findings,
                        "BR-55",
                        "BT-25",
                        &format!("/references/{index}/id"),
                        "Set the preceding invoice reference identifier",
                    )?;
                }
            }
        }
        DocumentSyntax::Cii => {
            for (index, reference) in descendants(ctx.root, "AdditionalReferencedDocument")
                .into_iter()
                .enumerate()
            {
                if reference.path_text(&["IssuerAssignedID"]).is_none() {
                    fail(
                        findings,
                        "BR-55",
                        "BT-25",
                        &format!("/references/{index}/id"),
                        "Set the preceding invoice reference identifier",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_56(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, representative) in tax_representatives(ctx).iter().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => representative
                .children_named("PartyTaxScheme")
                .find(|scheme| has_vat_tax_scheme(scheme))
                .and_then(|scheme| scheme.path_text(&["CompanyID"])),
            DocumentSyntax::Cii => representative
                .children_named("SpecifiedTaxRegistration")
                .find(|registration| {
                    registration
                        .child("ID")
                        .is_some_and(|id| id.attr("schemeID") == Some("VA"))
                })
                .and_then(|registration| registration.path_text(&["ID"])),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-56",
                "BT-63",
                &format!("/seller/tax_representative/{index}/vat_id"),
                "Set the seller tax representative VAT identifier",
            )?;
        }
    }
    Ok(())
}

fn br_57(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    match ctx.syntax {
        DocumentSyntax::Ubl => {
            for (index, address) in ctx
                .root
                .children_named("Delivery")
                .filter_map(|delivery| delivery.path(&["DeliveryLocation", "Address"]))
                .enumerate()
            {
                if address
                    .path_text(&["Country", "IdentificationCode"])
                    .is_none()
                {
                    fail(
                        findings,
                        "BR-57",
                        "BT-80",
                        &format!("/delivery/{index}/address/country"),
                        "Set the deliver-to country code",
                    )?;
                }
            }
        }
        DocumentSyntax::Cii => {
            for (index, delivery) in descendants(ctx.root, "ApplicableHeaderTradeDelivery")
                .into_iter()
                .enumerate()
            {
                let address = delivery.path(&["ShipToTradeParty", "PostalTradeAddress"]);
                if address.is_some_and(|address| address.path_text(&["CountryID"]).is_none()) {
                    fail(
                        findings,
                        "BR-57",
                        "BT-80",
                        &format!("/delivery/{index}/address/country"),
                        "Set the deliver-to country code",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_61(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, payment) in payment_means(ctx).iter().enumerate() {
        let Some(code) = payment_code(payment, ctx.syntax) else {
            continue;
        };
        if !is_credit_transfer_code(code) {
            continue;
        }
        let invalid = match ctx.syntax {
            DocumentSyntax::Ubl => payment_account_id(payment, ctx.syntax).is_none(),
            DocumentSyntax::Cii => {
                payment_account_node(payment, ctx.syntax).is_some_and(|account| {
                    account.child("IBANID").is_none() && account.child("ProprietaryID").is_none()
                })
            }
        };
        if invalid {
            fail(
                findings,
                "BR-61",
                "BT-84",
                &format!("/payment_means/{index}/account/id"),
                "Set the payment account identifier for credit transfer payments",
            )?;
        }
    }
    Ok(())
}

fn br_62(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let missing = match ctx.syntax {
        DocumentSyntax::Ubl => seller_party(ctx)
            .and_then(|seller| seller.child("EndpointID"))
            .is_some_and(|endpoint| !attr_is_non_blank(endpoint, "schemeID")),
        DocumentSyntax::Cii => seller_party(ctx).is_some_and(|seller| {
            seller
                .child("URIUniversalCommunication")
                .is_some_and(|communication| {
                    !communication
                        .child("URIID")
                        .is_some_and(|uri| attr_is_non_blank(uri, "schemeID"))
                })
        }),
    };
    if missing {
        fail(
            findings,
            "BR-62",
            "BT-34",
            "/seller/electronic_address/scheme",
            "Set the seller electronic address scheme identifier",
        )?;
    }
    Ok(())
}

fn br_63(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let missing = match ctx.syntax {
        DocumentSyntax::Ubl => buyer_party(ctx)
            .and_then(|buyer| buyer.child("EndpointID"))
            .is_some_and(|endpoint| !attr_is_non_blank(endpoint, "schemeID")),
        DocumentSyntax::Cii => buyer_party(ctx).is_some_and(|buyer| {
            buyer
                .child("URIUniversalCommunication")
                .is_some_and(|communication| {
                    !communication
                        .child("URIID")
                        .is_some_and(|uri| attr_is_non_blank(uri, "schemeID"))
                })
        }),
    };
    if missing {
        fail(
            findings,
            "BR-63",
            "BT-49",
            "/buyer/electronic_address/scheme",
            "Set the buyer electronic address scheme identifier",
        )?;
    }
    Ok(())
}

fn br_64(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        match ctx.syntax {
            DocumentSyntax::Ubl => {
                for id in line
                    .path(&["Item", "StandardItemIdentification"])
                    .into_iter()
                    .filter_map(|node| node.child("ID"))
                {
                    if !attr_is_non_blank(id, "schemeID") {
                        fail(
                            findings,
                            "BR-64",
                            "BT-157",
                            &format!("/lines/{index}/standard_item_id/scheme"),
                            "Set the item standard identifier scheme identifier",
                        )?;
                    }
                }
            }
            DocumentSyntax::Cii => {
                if line
                    .path(&["SpecifiedTradeProduct", "GlobalID"])
                    .is_some_and(|id| !attr_is_non_blank(id, "schemeID"))
                {
                    fail(
                        findings,
                        "BR-64",
                        "BT-157",
                        &format!("/lines/{index}/standard_item_id/scheme"),
                        "Set the item standard identifier scheme identifier",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_65(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        match ctx.syntax {
            DocumentSyntax::Ubl => {
                let Some(item) = line.child("Item") else {
                    continue;
                };
                for classification in item.children_named("CommodityClassification") {
                    if classification
                        .child("ItemClassificationCode")
                        .is_some_and(|code| !attr_is_non_blank(code, "listID"))
                    {
                        fail(
                            findings,
                            "BR-65",
                            "BT-158",
                            &format!("/lines/{index}/classification/scheme"),
                            "Set the item classification identifier scheme identifier",
                        )?;
                    }
                }
            }
            DocumentSyntax::Cii => {
                for classification in descendants(line, "ClassCode") {
                    if !attr_is_non_blank(classification, "listID") {
                        fail(
                            findings,
                            "BR-65",
                            "BT-158",
                            &format!("/lines/{index}/classification/scheme"),
                            "Set the item classification identifier scheme identifier",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn br_ae_05(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let Some(category) = line_vat_tax_category(ctx.syntax, line) else {
            continue;
        };
        if category_code(category, ctx.syntax) == Some("AE")
            && category_rate(category, ctx.syntax) != Some(Decimal::ZERO)
        {
            fail(
                findings,
                "BR-AE-05",
                "BT-152",
                &format!("/lines/{index}/tax_rate"),
                "Set the invoice line reverse-charge VAT rate to zero",
            )?;
        }
    }
    Ok(())
}

fn br_ae_08(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    if ctx.syntax != DocumentSyntax::Ubl {
        return Ok(());
    }
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let Some(category) = ubl_vat_tax_category(tax) else {
            continue;
        };
        if category.path_text(&["ID"]) != Some("AE") {
            continue;
        }
        let Some(taxable) = tax.path_text(&["TaxableAmount"]).and_then(decimal) else {
            continue;
        };
        let expected = rounded_2(
            lines(ctx)
                .iter()
                .copied()
                .filter(|line| {
                    line.path(&["Item", "ClassifiedTaxCategory"])
                        .and_then(|category| category.path_text(&["ID"]))
                        == Some("AE")
                })
                .filter_map(|line| line.path_text(&["LineExtensionAmount"]).and_then(decimal))
                .sum::<Decimal>()
                + document_allowance_charges(ctx, true)
                    .iter()
                    .filter(|charge| {
                        charge
                            .path(&["TaxCategory"])
                            .and_then(|category| category.path_text(&["ID"]))
                            == Some("AE")
                    })
                    .filter_map(|charge| allowance_charge_amount(charge, ctx.syntax))
                    .sum::<Decimal>()
                - document_allowance_charges(ctx, false)
                    .iter()
                    .filter(|charge| {
                        charge
                            .path(&["TaxCategory"])
                            .and_then(|category| category.path_text(&["ID"]))
                            == Some("AE")
                    })
                    .filter_map(|charge| allowance_charge_amount(charge, ctx.syntax))
                    .sum::<Decimal>(),
        );
        if taxable != expected {
            fail(
                findings,
                "BR-AE-08",
                "BT-116",
                &format!("/tax_summary/{index}/taxable_amount"),
                "Set reverse-charge VAT taxable amount to matching line amounts plus charges minus allowances",
            )?;
        }
    }
    Ok(())
}

fn br_ae_10(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let category = match ctx.syntax {
            DocumentSyntax::Ubl => ubl_vat_tax_category(tax),
            DocumentSyntax::Cii => cii_vat_tax(tax),
        };
        let Some(category) = category else {
            continue;
        };
        if category_code(category, ctx.syntax) == Some("AE")
            && !has_tax_exemption_reason(category, ctx.syntax)
        {
            fail(
                findings,
                "BR-AE-10",
                "BT-121",
                &format!("/tax_summary/{index}/exemption_reason"),
                "Set a reverse-charge VAT exemption reason or reason code",
            )?;
        }
    }
    Ok(())
}

fn br_cl_17(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let category = match ctx.syntax {
            DocumentSyntax::Ubl => ubl_vat_tax_category(tax),
            DocumentSyntax::Cii => cii_vat_tax(tax),
        };
        let Some(code) = category.and_then(|node| category_code(node, ctx.syntax)) else {
            continue;
        };
        if !tax_category_code_is_uncl5305(code) {
            fail(
                findings,
                "BR-CL-17",
                "BT-118",
                &format!("/tax_summary/{index}/category_code"),
                "Use a UNCL5305 VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_cl_18(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let Some(category) = line_vat_tax_category(ctx.syntax, line) else {
            continue;
        };
        let Some(code) = category_code(category, ctx.syntax) else {
            continue;
        };
        if !tax_category_code_is_uncl5305(code) {
            fail(
                findings,
                "BR-CL-18",
                "BT-151",
                &format!("/lines/{index}/tax_category"),
                "Use a UNCL5305 invoice line VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_co_03(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let has_date = match ctx.syntax {
        DocumentSyntax::Ubl => ctx.root.child("TaxPointDate").is_some(),
        DocumentSyntax::Cii => !descendants(ctx.root, "TaxPointDate").is_empty(),
    };
    let has_code = match ctx.syntax {
        DocumentSyntax::Ubl => invoice_periods(ctx)
            .iter()
            .any(|period| period.child("DescriptionCode").is_some()),
        DocumentSyntax::Cii => !descendants(ctx.root, "DueDateTypeCode").is_empty(),
    };
    if has_date && has_code {
        fail(
            findings,
            "BR-CO-03",
            "BT-8",
            "/tax_point",
            "Use either the VAT point date or the VAT point date code, not both",
        )?;
    }
    Ok(())
}

fn br_co_04(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, line) in lines(ctx).iter().copied().enumerate() {
        let value = match ctx.syntax {
            DocumentSyntax::Ubl => line
                .path(&["Item", "ClassifiedTaxCategory"])
                .filter(|node| has_vat_tax_scheme(node))
                .and_then(|node| node.path_text(&["ID"])),
            DocumentSyntax::Cii => line
                .path(&["SpecifiedLineTradeSettlement", "ApplicableTradeTax"])
                .and_then(cii_vat_tax)
                .and_then(|node| node.path_text(&["CategoryCode"])),
        };
        if value.is_none() {
            fail(
                findings,
                "BR-CO-04",
                "BT-151",
                &format!("/lines/{index}/tax_category"),
                "Set the invoice line VAT category code",
            )?;
        }
    }
    Ok(())
}

fn br_co_05(_ctx: &ValidationContext<'_>, _findings: &mut Vec<ValidationResult>) {
    // Reference parity: the pinned CEN/KoSIT UBL and CII Schematron
    // rows for BR-CO-05 assert true(). The semantic equivalence of
    // BT-97 free text and BT-98 reason code is not machine-enforced
    // by the reference artifact, so the correct oracle-matching
    // behavior is to emit no finding.
}

fn br_co_06(_ctx: &ValidationContext<'_>, _findings: &mut Vec<ValidationResult>) {
    // Reference parity: the pinned CEN/KoSIT UBL and CII Schematron
    // rows for BR-CO-06 assert true(). See BR-CO-05 for why the
    // Rust validator intentionally emits no finding here.
}

fn br_co_07(_ctx: &ValidationContext<'_>, _findings: &mut Vec<ValidationResult>) {
    // Reference parity: the pinned CEN/KoSIT UBL and CII Schematron
    // rows for BR-CO-07 assert true(). See BR-CO-05 for why the
    // Rust validator intentionally emits no finding here.
}

fn br_co_08(_ctx: &ValidationContext<'_>, _findings: &mut Vec<ValidationResult>) {
    // Reference parity: the pinned CEN/KoSIT UBL and CII Schematron
    // rows for BR-CO-08 assert true(). See BR-CO-05 for why the
    // Rust validator intentionally emits no finding here.
}

fn br_co_09(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let vat_ids: Vec<&str> = match ctx.syntax {
        DocumentSyntax::Ubl => descendants(ctx.root, "PartyTaxScheme")
            .into_iter()
            .filter(|scheme| has_vat_tax_scheme(scheme))
            .filter_map(|scheme| scheme.path_text(&["CompanyID"]))
            .collect(),
        DocumentSyntax::Cii => descendants(ctx.root, "SpecifiedTaxRegistration")
            .into_iter()
            .filter_map(|registration| registration.child("ID"))
            .filter(|id| id.attr("schemeID") == Some("VA"))
            .filter_map(|id| non_blank(id.text.as_str()))
            .collect(),
    };
    for (index, vat_id) in vat_ids.into_iter().enumerate() {
        if !is_valid_vat_identifier_prefix(vat_id) {
            fail(
                findings,
                "BR-CO-09",
                "BT-31",
                &format!("/vat_identifiers/{index}"),
                "Prefix VAT identifiers with an ISO 3166-1 alpha-2 country code, EL, or XI",
            )?;
        }
    }
    Ok(())
}

fn br_co_10(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let total_name = match ctx.syntax {
        DocumentSyntax::Ubl => "LineExtensionAmount",
        DocumentSyntax::Cii => "LineTotalAmount",
    };
    let Some(total) = header_amount(ctx, total_name) else {
        return Ok(());
    };
    let sum = rounded_2(line_amounts(ctx).into_iter().sum());
    if total != sum {
        fail(
            findings,
            "BR-CO-10",
            "BT-106",
            "/monetary_total/line_extension_amount",
            "Set the line net total to the rounded sum of line net amounts",
        )?;
    }
    Ok(())
}

fn br_co_11(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let allowances = document_allowance_charges(ctx, false);
    let total = header_amount(ctx, "AllowanceTotalAmount");
    if allowances.is_empty() && total.is_none() {
        return Ok(());
    }
    let sum = rounded_2(
        allowances
            .iter()
            .filter_map(|charge| allowance_charge_amount(charge, ctx.syntax))
            .sum(),
    );
    if total != Some(sum) {
        fail(
            findings,
            "BR-CO-11",
            "BT-107",
            "/monetary_total/allowance_total_amount",
            "Set the document allowance total to the rounded sum of document allowance amounts",
        )?;
    }
    Ok(())
}

fn br_co_12(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let charges = document_allowance_charges(ctx, true);
    let total = header_amount(ctx, "ChargeTotalAmount");
    if charges.is_empty() && total.is_none() {
        return Ok(());
    }
    let sum = rounded_2(
        charges
            .iter()
            .filter_map(|charge| allowance_charge_amount(charge, ctx.syntax))
            .sum(),
    );
    if total != Some(sum) {
        fail(
            findings,
            "BR-CO-12",
            "BT-108",
            "/monetary_total/charge_total_amount",
            "Set the document charge total to the rounded sum of document charge amounts",
        )?;
    }
    Ok(())
}

fn br_co_13(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let (tax_exclusive_name, line_total_name, allowance_name, charge_name) = match ctx.syntax {
        DocumentSyntax::Ubl => (
            "TaxExclusiveAmount",
            "LineExtensionAmount",
            "AllowanceTotalAmount",
            "ChargeTotalAmount",
        ),
        DocumentSyntax::Cii => (
            "TaxBasisTotalAmount",
            "LineTotalAmount",
            "AllowanceTotalAmount",
            "ChargeTotalAmount",
        ),
    };
    let Some(tax_exclusive) = header_amount(ctx, tax_exclusive_name) else {
        return Ok(());
    };
    let line_total = header_amount(ctx, line_total_name).unwrap_or(Decimal::ZERO);
    let allowance = header_amount(ctx, allowance_name).unwrap_or(Decimal::ZERO);
    let charge = header_amount(ctx, charge_name).unwrap_or(Decimal::ZERO);
    let expected = rounded_2(line_total - allowance + charge);
    if tax_exclusive != expected {
        fail(
            findings,
            "BR-CO-13",
            "BT-109",
            "/monetary_total/tax_exclusive_amount",
            "Set the VAT-exclusive total to line total minus allowances plus charges",
        )?;
    }
    Ok(())
}

fn br_co_14(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    match ctx.syntax {
        DocumentSyntax::Ubl => {
            for (index, tax_total) in ctx.root.children_named("TaxTotal").enumerate() {
                let subtotals: Vec<&XmlNode> = tax_total.children_named("TaxSubtotal").collect();
                if subtotals.is_empty() {
                    continue;
                }
                let Some(total) = tax_total.path_text(&["TaxAmount"]).and_then(decimal) else {
                    continue;
                };
                let sum = rounded_2(
                    subtotals
                        .into_iter()
                        .filter_map(|subtotal| subtotal.path_text(&["TaxAmount"]).and_then(decimal))
                        .sum(),
                );
                if total != sum {
                    fail(
                        findings,
                        "BR-CO-14",
                        "BT-110",
                        &format!("/tax_totals/{index}/tax_amount"),
                        "Set the invoice total VAT amount to the sum of VAT category tax amounts",
                    )?;
                }
            }
        }
        DocumentSyntax::Cii => {
            let Some(currency) = document_currency(ctx) else {
                return Ok(());
            };
            let totals = cii_tax_total_amounts_for_currency(ctx, currency);
            if totals.is_empty() {
                return Ok(());
            }
            let taxes = cii_header_settlement(ctx)
                .map(|settlement| {
                    settlement
                        .children_named("ApplicableTradeTax")
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let sum = rounded_2(
                taxes
                    .into_iter()
                    .filter_map(|tax| tax.path_text(&["CalculatedAmount"]).and_then(decimal))
                    .sum(),
            );
            for (index, total) in totals.into_iter().enumerate() {
                if decimal(total.text.as_str()) != Some(sum) {
                    fail(
                        findings,
                        "BR-CO-14",
                        "BT-110",
                        &format!("/tax_totals/{index}/tax_amount"),
                        "Set the invoice total VAT amount to the sum of VAT category tax amounts",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn br_co_15(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let Some(currency) = document_currency(ctx) else {
        return Ok(());
    };
    let valid = match ctx.syntax {
        DocumentSyntax::Ubl => {
            let amounts = ubl_tax_amounts_for_currency(ctx.root, currency);
            if amounts.len() == 1 {
                let tax = amounts
                    .first()
                    .and_then(|amount| decimal(amount.text.as_str()))
                    .unwrap_or(Decimal::ZERO);
                let inclusive = header_amount(ctx, "TaxInclusiveAmount");
                let exclusive = header_amount(ctx, "TaxExclusiveAmount");
                match (inclusive, exclusive) {
                    (Some(inclusive), Some(exclusive)) => inclusive == rounded_2(exclusive + tax),
                    _ => true,
                }
            } else {
                false
            }
        }
        DocumentSyntax::Cii => {
            let inclusive = header_amount(ctx, "GrandTotalAmount");
            let exclusive = header_amount(ctx, "TaxBasisTotalAmount");
            match (inclusive, exclusive) {
                (Some(inclusive), Some(exclusive)) => {
                    let taxes = cii_tax_total_amounts_for_currency(ctx, currency);
                    let with_tax = taxes.len() == 1
                        && taxes
                            .first()
                            .and_then(|tax| decimal(tax.text.as_str()))
                            .is_some_and(|tax| inclusive == rounded_2(exclusive + tax));
                    with_tax || inclusive == exclusive
                }
                _ => true,
            }
        }
    };
    if !valid {
        fail(
            findings,
            "BR-CO-15",
            "BT-112",
            "/monetary_total/tax_inclusive_amount",
            "Set the VAT-inclusive total to VAT-exclusive total plus invoice VAT total",
        )?;
    }
    Ok(())
}

fn br_co_16(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let (payable, inclusive, prepaid, rounding) = match ctx.syntax {
        DocumentSyntax::Ubl => (
            header_amount(ctx, "PayableAmount"),
            header_amount(ctx, "TaxInclusiveAmount"),
            header_amount(ctx, "PrepaidAmount"),
            header_amount(ctx, "PayableRoundingAmount"),
        ),
        DocumentSyntax::Cii => (
            header_amount(ctx, "DuePayableAmount"),
            header_amount(ctx, "GrandTotalAmount"),
            header_amount(ctx, "TotalPrepaidAmount"),
            header_amount(ctx, "RoundingAmount"),
        ),
    };
    let Some(payable) = payable else {
        return Ok(());
    };
    let Some(inclusive) = inclusive else {
        return Ok(());
    };
    let expected =
        rounded_2(inclusive - prepaid.unwrap_or(Decimal::ZERO) + rounding.unwrap_or(Decimal::ZERO));
    if payable != expected {
        fail(
            findings,
            "BR-CO-16",
            "BT-115",
            "/monetary_total/payable_amount",
            "Set payable amount to VAT-inclusive total minus prepaid amount plus rounding amount",
        )?;
    }
    Ok(())
}

fn br_co_17(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, tax) in tax_summaries(ctx).iter().copied().enumerate() {
        let (taxable, tax_amount, rate) = match ctx.syntax {
            DocumentSyntax::Ubl => (
                tax.path_text(&["TaxableAmount"]).and_then(decimal),
                tax.path_text(&["TaxAmount"]).and_then(decimal),
                ubl_vat_tax_category(tax)
                    .and_then(|node| node.path_text(&["Percent"]))
                    .and_then(decimal),
            ),
            DocumentSyntax::Cii => (
                tax.path_text(&["BasisAmount"]).and_then(decimal),
                tax.path_text(&["CalculatedAmount"]).and_then(decimal),
                cii_vat_tax(tax)
                    .and_then(|node| node.path_text(&["RateApplicablePercent"]))
                    .and_then(decimal),
            ),
        };
        let Some(tax_amount) = tax_amount else {
            continue;
        };
        let valid = match (taxable, rate) {
            (_, Some(rate)) if rounded_0(rate) == Decimal::ZERO => {
                rounded_0(tax_amount) == Decimal::ZERO
            }
            (Some(taxable), Some(rate)) => {
                let expected = rounded_2(taxable.abs() * rate / Decimal::new(100, 0));
                br_co_17_within_tolerance(ctx.syntax, tax_amount.abs(), expected)
            }
            (_, None) | (None, Some(_)) => rounded_0(tax_amount) == Decimal::ZERO,
        };
        if !valid {
            fail(findings, "BR-CO-17", "BT-117", &format!("/tax_summary/{index}/tax_amount"), "Set VAT tax amount to taxable amount multiplied by VAT rate, rounded to two decimals")?;
        }
    }
    Ok(())
}

fn br_co_18(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    if tax_summaries(ctx).is_empty() {
        fail(
            findings,
            "BR-CO-18",
            "BG-23",
            "/tax_summary",
            "Add at least one VAT breakdown group",
        )?;
    }
    Ok(())
}

fn br_co_19(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (index, period) in invoice_periods(ctx).iter().enumerate() {
        let has_date = match ctx.syntax {
            DocumentSyntax::Ubl => {
                period.child("StartDate").is_some() || period.child("EndDate").is_some()
            }
            DocumentSyntax::Cii => {
                period.child("StartDateTime").is_some() || period.child("EndDateTime").is_some()
            }
        };
        let allowed_code_only = ctx.syntax == DocumentSyntax::Ubl
            && period.child("DescriptionCode").is_some()
            && !has_date;
        if !has_date && !allowed_code_only {
            fail(
                findings,
                "BR-CO-19",
                "BG-14",
                &format!("/invoice_periods/{index}"),
                "Set an invoicing period start date, end date, or allowed date code",
            )?;
        }
    }
    Ok(())
}

fn br_co_20(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        let periods: Vec<&XmlNode> = match ctx.syntax {
            DocumentSyntax::Ubl => line.children_named("InvoicePeriod").collect(),
            DocumentSyntax::Cii => line
                .path(&["SpecifiedLineTradeSettlement"])
                .map(|settlement| {
                    settlement
                        .children_named("BillingSpecifiedPeriod")
                        .collect()
                })
                .unwrap_or_default(),
        };
        for (period_index, period) in periods.into_iter().enumerate() {
            let has_date = match ctx.syntax {
                DocumentSyntax::Ubl => {
                    period.child("StartDate").is_some() || period.child("EndDate").is_some()
                }
                DocumentSyntax::Cii => {
                    period.child("StartDateTime").is_some() || period.child("EndDateTime").is_some()
                }
            };
            if !has_date {
                fail(
                    findings,
                    "BR-CO-20",
                    "BG-26",
                    &format!("/lines/{line_index}/periods/{period_index}"),
                    "Set an invoice line period start or end date",
                )?;
            }
        }
    }
    Ok(())
}

fn br_co_21(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let charges = document_allowance_charges_all(ctx);
    for (index, charge) in charges.into_iter().enumerate() {
        if charge_indicator(charge).is_some_and(is_false_indicator)
            && !has_allowance_charge_reason(charge, ctx.syntax)
        {
            fail(
                findings,
                "BR-CO-21",
                "BT-97",
                &format!("/document_allowances/{index}/reason"),
                "Set a document-level allowance reason or reason code",
            )?;
        }
    }
    Ok(())
}

fn br_co_22(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let charges = document_allowance_charges_all(ctx);
    for (index, charge) in charges.into_iter().enumerate() {
        if charge_indicator(charge).is_some_and(is_true_indicator)
            && !has_allowance_charge_reason(charge, ctx.syntax)
        {
            fail(
                findings,
                "BR-CO-22",
                "BT-104",
                &format!("/document_charges/{index}/reason"),
                "Set a document-level charge reason or reason code",
            )?;
        }
    }
    Ok(())
}

fn br_co_23(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        let charges = line_allowance_charges_all(ctx, line);
        for (index, charge) in charges.into_iter().enumerate() {
            if charge_indicator(charge).is_some_and(is_false_indicator)
                && !has_allowance_charge_reason(charge, ctx.syntax)
            {
                fail(
                    findings,
                    "BR-CO-23",
                    "BT-139",
                    &format!("/lines/{line_index}/allowances/{index}/reason"),
                    "Set an invoice line allowance reason or reason code",
                )?;
            }
        }
    }
    Ok(())
}

fn br_co_24(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    for (line_index, line) in lines(ctx).iter().copied().enumerate() {
        let charges = line_allowance_charges_all(ctx, line);
        for (index, charge) in charges.into_iter().enumerate() {
            if charge_indicator(charge).is_some_and(is_true_indicator)
                && !has_allowance_charge_reason(charge, ctx.syntax)
            {
                fail(
                    findings,
                    "BR-CO-24",
                    "BT-144",
                    &format!("/lines/{line_index}/charges/{index}/reason"),
                    "Set an invoice line charge reason or reason code",
                )?;
            }
        }
    }
    Ok(())
}

fn br_co_26(
    ctx: &ValidationContext<'_>,
    findings: &mut Vec<ValidationResult>,
) -> Result<(), En16931Error> {
    let present = match ctx.syntax {
        DocumentSyntax::Ubl => seller_party(ctx).is_some_and(|seller| {
            seller
                .children_named("PartyIdentification")
                .any(|identification| {
                    identification.child("ID").is_some_and(|id| {
                        non_blank(id.text.as_str()).is_some()
                            && id
                                .attr("schemeID")
                                .is_none_or(|scheme| scheme.trim() != "SEPA")
                    })
                })
                || seller
                    .children_named("PartyLegalEntity")
                    .any(|legal| legal.path_text(&["CompanyID"]).is_some())
                || seller.children_named("PartyTaxScheme").any(|scheme| {
                    has_vat_tax_scheme(scheme) && scheme.path_text(&["CompanyID"]).is_some()
                })
        }),
        DocumentSyntax::Cii => seller_party(ctx).is_some_and(|seller| {
            seller.path_text(&["ID"]).is_some()
                || seller.path_text(&["GlobalID"]).is_some()
                || seller
                    .path_text(&["SpecifiedLegalOrganization", "ID"])
                    .is_some()
                || seller
                    .children_named("SpecifiedTaxRegistration")
                    .any(|registration| {
                        registration.child("ID").is_some_and(|id| {
                            id.attr("schemeID") == Some("VA")
                                && non_blank(id.text.as_str()).is_some()
                        })
                    })
        }),
    };
    if !present {
        fail(
            findings,
            "BR-CO-26",
            "BT-29",
            "/seller/identifier",
            "Set a seller identifier, legal registration identifier, or VAT identifier",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Instant;

    use invoicekit_rulepack::{Manifest, Registry};
    use serde_json::{json, Value};

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-validate-ubl-cii");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn implemented_inventory_matches_checked_coverage_matrix() {
        let matrix: Value = serde_json::from_str(EN16931_BR_CO_COVERAGE_JSON).unwrap();
        assert_eq!(matrix["counts"]["rules_total"].as_u64(), Some(81));
        assert_eq!(
            matrix["counts"]["validator_testable_now"].as_u64(),
            Some(34)
        );
        assert_eq!(matrix["counts"]["blocked_by_ir_gaps"].as_u64(), Some(47));

        let testable: BTreeSet<String> = matrix["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|rule| {
                rule["rust_validator_testability"]["positive"] == true
                    && rule["rust_validator_testability"]["negative"] == true
            })
            .map(|rule| rule["id"].as_str().unwrap().to_owned())
            .collect();
        let implemented: BTreeSet<String> = implemented_rule_ids()
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        let deferred: BTreeSet<String> = deferred_rules()
            .into_iter()
            .map(|rule| rule.id.to_owned())
            .collect();
        assert!(
            testable.is_subset(&implemented),
            "matrix-testable rules not implemented: {:?}",
            testable.difference(&implemented).collect::<Vec<_>>()
        );
        assert!(
            implemented.is_disjoint(&deferred),
            "implemented/deferred overlap: {:?}",
            implemented.intersection(&deferred).collect::<Vec<_>>()
        );
        assert_eq!(implemented.len(), COVERAGE_IMPLEMENTED_NOW);
        assert_eq!(deferred.len(), COVERAGE_DEFERRED_IR_GAP);
        assert_eq!(implemented.len() + deferred.len(), COVERAGE_RULE_TOTAL);
    }

    #[test]
    fn valid_ubl_invoice_has_no_findings() {
        let report = validate_xml(valid_ubl()).unwrap();
        assert_eq!(report.syntax, DocumentSyntax::Ubl);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.coverage, En16931Coverage::current());
        assert_eq!(
            report.rulepack.rulepack_id,
            "urn:invoicekit:rulepack:en16931:cen:2024-01"
        );
        assert_eq!(report.rulepack.selected_for_date, "latest");
    }

    #[test]
    fn date_pinned_rulepack_can_validate_against_historical_policy() {
        let registry = transition_registry();

        let pre_change = validate_xml_with_registry(
            known_bad_ubl(),
            &ValidationOptions::default().with_validation_date("2023-06-01"),
            &registry,
        )
        .unwrap();
        assert!(pre_change.findings.is_empty(), "{:?}", pre_change.findings);
        assert_eq!(
            pre_change.rulepack.rulepack_id,
            "urn:test:en16931:pre-change"
        );
        assert_eq!(pre_change.rulepack.disabled_rules, vec!["*"]);
        assert_eq!(pre_change.rulepack.selected_for_date, "2023-06-01");

        let post_change = validate_xml_with_registry(
            known_bad_ubl(),
            &ValidationOptions::default().with_validation_date("2024-06-01"),
            &registry,
        )
        .unwrap();
        assert!(
            post_change
                .findings
                .iter()
                .any(|finding| finding.rule_id.as_str() == "BR-01"),
            "{:?}",
            post_change.findings
        );
        assert_eq!(
            post_change.rulepack.rulepack_id,
            "urn:test:en16931:post-change"
        );
        assert!(post_change.rulepack.disabled_rules.is_empty());
    }

    #[test]
    fn invalid_validation_date_is_rejected() {
        let err = validate_xml_on_date(valid_ubl(), "2024/01/01").unwrap_err();
        assert!(matches!(err, En16931Error::InvalidValidationDate(_)));

        let impossible = validate_xml_on_date(valid_ubl(), "2024-02-31").unwrap_err();
        assert!(matches!(impossible, En16931Error::InvalidValidationDate(_)));
    }

    fn transition_registry() -> Registry {
        let mut registry = Registry::default();
        registry
            .insert(test_manifest(
                "urn:test:en16931:pre-change",
                "2020-01-01",
                Some("2023-12-31"),
                json!({"disabled_rules": ["*"]}),
            ))
            .unwrap();
        registry
            .insert(test_manifest(
                "urn:test:en16931:post-change",
                "2024-01-01",
                None,
                json!({}),
            ))
            .unwrap();
        registry
    }

    fn known_bad_ubl() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2"></ubl:Invoice>"#
    }

    fn test_manifest(
        rulepack_id: &str,
        effective_from: &str,
        effective_to: Option<&str>,
        body: Value,
    ) -> Manifest {
        let mut codelist_versions = BTreeMap::new();
        codelist_versions.insert("en16931-vat-categories".to_owned(), "test".to_owned());
        let signature = blake3::hash(&serde_json::to_vec(&body).unwrap())
            .to_hex()
            .to_string();
        Manifest {
            rulepack_id: rulepack_id.to_owned(),
            country: "global".to_owned(),
            profile: EN16931_PROFILE_URN.to_owned(),
            upstream_version: "test".to_owned(),
            effective_from: effective_from.to_owned(),
            effective_to: effective_to.map(str::to_owned),
            source_url: "https://example.invalid/rulepack".to_owned(),
            retrieved_at: "2026-05-28".to_owned(),
            codelist_versions,
            upstream_checksum_blake3: "0".repeat(64),
            generated_metadata: invoicekit_rulepack::GeneratedMetadata {
                generator: "test".to_owned(),
                generated_at: "2026-05-28".to_owned(),
                notes: "synthetic transition fixture".to_owned(),
            },
            parity_fixtures: invoicekit_rulepack::ParityFixtures {
                oracle: "jvm:phive".to_owned(),
                fixture_set_id: "synthetic".to_owned(),
                expected_parity_pct: 99.9,
            },
            known_gaps: Vec::new(),
            signature_alg: "blake3:identity".to_owned(),
            signature,
            body,
        }
    }

    #[test]
    fn valid_cii_invoice_has_no_findings() {
        let report = validate_xml(valid_cii()).unwrap();
        assert_eq!(report.syntax, DocumentSyntax::Cii);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn namespace_less_ubl_root_is_not_accepted() {
        let xml = format!(
            r#"<Invoice xmlns:cac="{cac}" xmlns:cbc="{cbc}"></Invoice>"#,
            cac = "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2",
            cbc = "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2",
        );
        assert!(matches!(
            validate_xml(&xml),
            Err(En16931Error::UnsupportedRoot(_))
        ));
    }

    #[test]
    fn wrong_namespace_ubl_root_is_not_accepted() {
        let xml = r#"<bad:Invoice xmlns:bad="urn:example:wrong"></bad:Invoice>"#;
        assert!(matches!(
            validate_xml(xml),
            Err(En16931Error::UnsupportedRoot(_))
        ));
    }

    #[test]
    fn cii_issue_date_requires_format_102() {
        let xml = replace(
            valid_cii(),
            r#"<udt:DateTimeString format="102">20260527</udt:DateTimeString>"#,
            r#"<udt:DateTimeString format="203">20260527</udt:DateTimeString>"#,
        );
        assert_emits_rule(&xml, "BR-03");
    }

    #[test]
    fn payee_must_differ_from_seller() {
        let xml = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:PayeeParty><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName></cac:PayeeParty>",
        );
        assert_emits_rule(&xml, "BR-17");
    }

    #[test]
    fn vat_breakdown_rules_require_vat_tax_scheme() {
        let non_vat_summary = tax_subtotal().replace(
            "<cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>",
            "<cac:TaxScheme><cbc:ID>GST</cbc:ID></cac:TaxScheme>",
        );
        let xml = replace(valid_ubl(), tax_subtotal(), &non_vat_summary);

        assert_emits_rule(&xml, "BR-47");
        assert_emits_rule(&xml, "BR-48");
        assert_emits_rule(&xml, "BR-CO-17");
    }

    #[test]
    fn line_vat_category_requires_vat_tax_scheme() {
        let non_vat_line = invoice_line().replace(
            "<cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme>",
            "<cac:TaxScheme><cbc:ID>GST</cbc:ID></cac:TaxScheme>",
        );
        let xml = replace(valid_ubl(), invoice_line(), &non_vat_line);

        assert_emits_rule(&xml, "BR-CO-04");
    }

    #[test]
    fn br_co_17_uses_absolute_amounts_and_reference_tolerance() {
        let xml = replace(
            valid_ubl(),
            "<cbc:TaxAmount>19.00</cbc:TaxAmount><cac:TaxCategory>",
            "<cbc:TaxAmount>-18.01</cbc:TaxAmount><cac:TaxCategory>",
        );
        let report = validate_xml(&xml).unwrap();
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.rule_id.as_str() == "BR-CO-17"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn cii_missing_vat_rate_requires_zero_tax_amount() {
        let xml = replace(
            valid_cii(),
            "<ram:RateApplicablePercent>19.00</ram:RateApplicablePercent>",
            "",
        );
        assert_emits_rule(&xml, "BR-CO-17");
    }

    #[test]
    fn fresh_eyes_regressions_match_source_predicates() {
        let padded_card = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:PaymentMeans><cbc:PaymentMeansCode>48</cbc:PaymentMeansCode><cac:CardAccount><cbc:PrimaryAccountNumberID> 1234567890 </cbc:PrimaryAccountNumberID></cac:CardAccount></cac:PaymentMeans>",
        );
        assert_not_emits_rule(&padded_card, "BR-51");

        let long_card = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:PaymentMeans><cbc:PaymentMeansCode>48</cbc:PaymentMeansCode><cac:CardAccount><cbc:PrimaryAccountNumberID>12345678901</cbc:PrimaryAccountNumberID></cac:CardAccount></cac:PaymentMeans>",
        );
        assert_emits_rule_with_severity(&long_card, "BR-51", Severity::Warning);

        let no_tax_rep_address = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:TaxRepresentativeParty><cac:PartyName><cbc:Name>Tax Rep</cbc:Name></cac:PartyName><cac:PartyTaxScheme><cbc:CompanyID>DE123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme></cac:TaxRepresentativeParty>",
        );
        assert_emits_rule(&no_tax_rep_address, "BR-19");
        assert_not_emits_rule(&no_tax_rep_address, "BR-20");

        let cii_credit_transfer_without_account = replace(
            valid_cii(),
            "<ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
            "<ram:SpecifiedTradeSettlementPaymentMeans><ram:TypeCode>30</ram:TypeCode></ram:SpecifiedTradeSettlementPaymentMeans><ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
        );
        assert_not_emits_rule(&cii_credit_transfer_without_account, "BR-61");

        let cii_credit_transfer_with_empty_account_child = replace(
            valid_cii(),
            "<ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
            "<ram:SpecifiedTradeSettlementPaymentMeans><ram:TypeCode>30</ram:TypeCode><ram:PayeePartyCreditorFinancialAccount><ram:IBANID></ram:IBANID></ram:PayeePartyCreditorFinancialAccount></ram:SpecifiedTradeSettlementPaymentMeans><ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
        );
        assert_emits_rule(&cii_credit_transfer_with_empty_account_child, "BR-50");
        assert_not_emits_rule(&cii_credit_transfer_with_empty_account_child, "BR-61");

        let cii_seller_uri_without_id = replace(
            valid_cii(),
            "<ram:Name>Supplier GmbH</ram:Name>",
            "<ram:Name>Supplier GmbH</ram:Name><ram:URIUniversalCommunication></ram:URIUniversalCommunication>",
        );
        assert_emits_rule(&cii_seller_uri_without_id, "BR-62");

        let cii_buyer_uri_without_id = replace(
            valid_cii(),
            "<ram:Name>Customer BV</ram:Name>",
            "<ram:Name>Customer BV</ram:Name><ram:URIUniversalCommunication></ram:URIUniversalCommunication>",
        );
        assert_emits_rule(&cii_buyer_uri_without_id, "BR-63");
    }

    #[test]
    fn final_slice_regressions_match_source_predicates() {
        let ubl_timezone_order = insert_before(
            valid_ubl(),
            "<cac:AccountingSupplierParty>",
            "<cac:InvoicePeriod><cbc:StartDate>2026-05-27-01:00</cbc:StartDate><cbc:EndDate>2026-05-27Z</cbc:EndDate></cac:InvoicePeriod>",
        );
        assert_emits_rule(&ubl_timezone_order, "BR-29");

        let cii_header_period_bad_start_format = replace(
            valid_cii(),
            "<ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
            "<ram:BillingSpecifiedPeriod><ram:StartDateTime><udt:DateTimeString format=\"203\">20260527</udt:DateTimeString></ram:StartDateTime><ram:EndDateTime><udt:DateTimeString format=\"102\">20260528</udt:DateTimeString></ram:EndDateTime></ram:BillingSpecifiedPeriod><ram:SpecifiedTradeSettlementHeaderMonetarySummation>",
        );
        assert_emits_rule(&cii_header_period_bad_start_format, "BR-29");

        let cii_line_period_bad_start_format = replace(
            valid_cii(),
            "<ram:SpecifiedLineTradeSettlement><ram:ApplicableTradeTax>",
            "<ram:SpecifiedLineTradeSettlement><ram:BillingSpecifiedPeriod><ram:StartDateTime><udt:DateTimeString format=\"203\">20260527</udt:DateTimeString></ram:StartDateTime><ram:EndDateTime><udt:DateTimeString format=\"102\">20260528</udt:DateTimeString></ram:EndDateTime></ram:BillingSpecifiedPeriod><ram:ApplicableTradeTax>",
        );
        assert_emits_rule(&cii_line_period_bad_start_format, "BR-30");

        let lowercase_vat_prefix = replace(
            valid_ubl(),
            "<cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName>",
            "<cac:PartyTaxScheme><cbc:CompanyID>de123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName>",
        );
        assert_emits_rule(&lowercase_vat_prefix, "BR-CO-09");

        let duplicate_cii_tax_total = replace(
            valid_cii(),
            "<ram:TaxTotalAmount currencyID=\"EUR\">19.00</ram:TaxTotalAmount><ram:GrandTotalAmount>",
            "<ram:TaxTotalAmount currencyID=\"EUR\">19.00</ram:TaxTotalAmount><ram:TaxTotalAmount currencyID=\"EUR\">18.00</ram:TaxTotalAmount><ram:GrandTotalAmount>",
        );
        assert_emits_rule(&duplicate_cii_tax_total, "BR-CO-14");
        assert_emits_rule(&duplicate_cii_tax_total, "BR-CO-15");
    }

    #[test]
    fn br_co_05_through_08_are_reference_non_enforceable() {
        let document_allowance = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason><cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>",
        );
        let document_charge = insert_before(
            valid_ubl(),
            "<cac:TaxTotal>",
            "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason><cbc:AllowanceChargeReasonCode>FC</cbc:AllowanceChargeReasonCode><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>",
        );
        let line_allowance = replace(
            valid_ubl(),
            "<cac:Item><cbc:Name>Implementation service</cbc:Name>",
            "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason><cbc:AllowanceChargeReasonCode>95</cbc:AllowanceChargeReasonCode><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>",
        );
        let line_charge = replace(
            valid_ubl(),
            "<cac:Item><cbc:Name>Implementation service</cbc:Name>",
            "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason><cbc:AllowanceChargeReasonCode>FC</cbc:AllowanceChargeReasonCode><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>",
        );

        for rule_id in ["BR-CO-05", "BR-CO-06", "BR-CO-07", "BR-CO-08"] {
            assert!(implemented_rule_ids().contains(&rule_id));
        }
        assert_not_emits_rule(&document_allowance, "BR-CO-05");
        assert_not_emits_rule(&document_charge, "BR-CO-06");
        assert_not_emits_rule(&line_allowance, "BR-CO-07");
        assert_not_emits_rule(&line_charge, "BR-CO-08");
    }

    #[test]
    fn one_megabyte_invoice_validates_under_25ms_in_release_builds() {
        if cfg!(debug_assertions) {
            return;
        }
        let xml = large_ubl_invoice(2_500);
        assert!(xml.len() > 1_000_000, "fixture is {} bytes", xml.len());

        let start = Instant::now();
        let report = validate_xml(&xml).unwrap();
        let elapsed = start.elapsed();

        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert!(
            elapsed.as_millis() < 25,
            "expected <25ms p95-size validation, got {elapsed:?}"
        );
    }

    #[test]
    fn implemented_rules_have_negative_ubl_cases() {
        let cases = [
            ("BR-01", replace(valid_ubl(), "<cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>", "<cbc:CustomizationID></cbc:CustomizationID>")),
            ("BR-02", replace(valid_ubl(), "<cbc:ID>INV-1</cbc:ID>", "<cbc:ID></cbc:ID>")),
            ("BR-03", replace(valid_ubl(), "<cbc:IssueDate>2026-05-27</cbc:IssueDate>", "<cbc:IssueDate></cbc:IssueDate>")),
            ("BR-04", replace(valid_ubl(), "<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>", "<cbc:InvoiceTypeCode></cbc:InvoiceTypeCode>")),
            ("BR-05", replace(valid_ubl(), "<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>", "<cbc:DocumentCurrencyCode></cbc:DocumentCurrencyCode>")),
            ("BR-06", replace(valid_ubl(), "<cbc:RegistrationName>Supplier GmbH</cbc:RegistrationName>", "<cbc:RegistrationName></cbc:RegistrationName>")),
            ("BR-07", replace(valid_ubl(), "<cbc:RegistrationName>Customer BV</cbc:RegistrationName>", "<cbc:RegistrationName></cbc:RegistrationName>")),
            ("BR-08", replace(valid_ubl(), supplier_address(), "")),
            ("BR-09", replace(valid_ubl(), "<cbc:IdentificationCode>DE</cbc:IdentificationCode>", "<cbc:IdentificationCode></cbc:IdentificationCode>")),
            ("BR-10", replace(valid_ubl(), customer_address(), "")),
            ("BR-11", replace(valid_ubl(), "<cbc:IdentificationCode>NL</cbc:IdentificationCode>", "<cbc:IdentificationCode></cbc:IdentificationCode>")),
            ("BR-12", replace(valid_ubl(), "<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount>", "<cac:LegalMonetaryTotal>")),
            ("BR-13", replace(valid_ubl(), "<cbc:TaxExclusiveAmount>100.00</cbc:TaxExclusiveAmount>", "")),
            ("BR-14", replace(valid_ubl(), "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount>", "")),
            ("BR-15", replace(valid_ubl(), "<cbc:PayableAmount>119.00</cbc:PayableAmount>", "")),
            ("BR-16", replace(valid_ubl(), invoice_line(), "")),
            ("BR-17", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:PayeeParty><cac:PartyName><cbc:Name></cbc:Name></cac:PartyName></cac:PayeeParty>")),
            ("BR-18", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:TaxRepresentativeParty><cac:PostalAddress><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyTaxScheme><cbc:CompanyID>DE123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme></cac:TaxRepresentativeParty>")),
            ("BR-19", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:TaxRepresentativeParty><cac:PartyName><cbc:Name>Tax Rep</cbc:Name></cac:PartyName><cac:PartyTaxScheme><cbc:CompanyID>DE123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme></cac:TaxRepresentativeParty>")),
            ("BR-20", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:TaxRepresentativeParty><cac:PartyName><cbc:Name>Tax Rep</cbc:Name></cac:PartyName><cac:PostalAddress><cac:Country><cbc:IdentificationCode></cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyTaxScheme><cbc:CompanyID>DE123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme></cac:TaxRepresentativeParty>")),
            ("BR-21", replace(valid_ubl(), "<cbc:ID>1</cbc:ID>", "<cbc:ID></cbc:ID>")),
            ("BR-22", replace(valid_ubl(), "<cbc:InvoicedQuantity unitCode=\"C62\">1</cbc:InvoicedQuantity>", "")),
            ("BR-23", replace(valid_ubl(), "<cbc:InvoicedQuantity unitCode=\"C62\">1</cbc:InvoicedQuantity>", "<cbc:InvoicedQuantity>1</cbc:InvoicedQuantity>")),
            ("BR-24", replace(valid_ubl(), "<cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cac:Item>", "<cac:Item>")),
            ("BR-25", replace(valid_ubl(), "<cbc:Name>Implementation service</cbc:Name>", "<cbc:Name></cbc:Name>")),
            ("BR-26", replace(valid_ubl(), "<cbc:PriceAmount>100.00</cbc:PriceAmount>", "")),
            ("BR-27", replace(valid_ubl(), "<cbc:PriceAmount>100.00</cbc:PriceAmount>", "<cbc:PriceAmount>-1.00</cbc:PriceAmount>")),
            ("BR-28", replace(valid_ubl(), "<cac:Price><cbc:PriceAmount>100.00</cbc:PriceAmount></cac:Price>", "<cac:Price><cbc:PriceAmount>100.00</cbc:PriceAmount><cac:AllowanceCharge><cbc:BaseAmount>-1.00</cbc:BaseAmount></cac:AllowanceCharge></cac:Price>")),
            ("BR-29", insert_before(valid_ubl(), "<cac:AccountingSupplierParty>", "<cac:InvoicePeriod><cbc:StartDate>2026-05-28</cbc:StartDate><cbc:EndDate>2026-05-27</cbc:EndDate></cac:InvoicePeriod>")),
            ("BR-30", replace(valid_ubl(), "<cac:InvoiceLine><cbc:ID>1</cbc:ID>", "<cac:InvoiceLine><cac:InvoicePeriod><cbc:StartDate>2026-05-28</cbc:StartDate><cbc:EndDate>2026-05-27</cbc:EndDate></cac:InvoicePeriod><cbc:ID>1</cbc:ID>")),
            ("BR-31", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>")),
            ("BR-32", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge>")),
            ("BR-33", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>")),
            ("BR-36", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>")),
            ("BR-37", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge>")),
            ("BR-38", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>")),
            ("BR-41", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-42", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-43", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-44", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-45", replace(valid_ubl(), "<cbc:TaxableAmount>100.00</cbc:TaxableAmount>", "")),
            ("BR-46", replace(valid_ubl(), "<cac:TaxSubtotal><cbc:TaxableAmount>100.00</cbc:TaxableAmount><cbc:TaxAmount>19.00</cbc:TaxAmount>", "<cac:TaxSubtotal><cbc:TaxableAmount>100.00</cbc:TaxableAmount>")),
            ("BR-47", replace(valid_ubl(), "<cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent>", "<cac:TaxCategory><cbc:Percent>19.00</cbc:Percent>")),
            ("BR-48", replace(valid_ubl(), "<cbc:Percent>19.00</cbc:Percent>", "")),
            ("BR-49", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:PaymentMeans></cac:PaymentMeans>")),
            ("BR-50", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:PaymentMeans><cbc:PaymentMeansCode>30</cbc:PaymentMeansCode><cac:PayeeFinancialAccount><cbc:ID></cbc:ID></cac:PayeeFinancialAccount></cac:PaymentMeans>")),
            ("BR-51", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:PaymentMeans><cbc:PaymentMeansCode>48</cbc:PaymentMeansCode><cac:CardAccount><cbc:PrimaryAccountNumberID>12345678901</cbc:PrimaryAccountNumberID></cac:CardAccount></cac:PaymentMeans>")),
            ("BR-52", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AdditionalDocumentReference><cbc:ID></cbc:ID></cac:AdditionalDocumentReference>")),
            ("BR-53", insert_before(valid_ubl(), "<cac:AccountingSupplierParty>", "<cbc:TaxCurrencyCode>USD</cbc:TaxCurrencyCode>")),
            ("BR-54", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:Item><cbc:Name>Implementation service</cbc:Name><cac:AdditionalItemProperty><cbc:Name>Color</cbc:Name></cac:AdditionalItemProperty>")),
            ("BR-55", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:BillingReference><cac:InvoiceDocumentReference><cbc:ID></cbc:ID></cac:InvoiceDocumentReference></cac:BillingReference>")),
            ("BR-56", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:TaxRepresentativeParty><cac:PartyName><cbc:Name>Tax Rep</cbc:Name></cac:PartyName><cac:PostalAddress><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress></cac:TaxRepresentativeParty>")),
            ("BR-57", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:Delivery><cac:DeliveryLocation><cac:Address><cac:Country><cbc:IdentificationCode></cbc:IdentificationCode></cac:Country></cac:Address></cac:DeliveryLocation></cac:Delivery>")),
            ("BR-61", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:PaymentMeans><cbc:PaymentMeansCode>30</cbc:PaymentMeansCode></cac:PaymentMeans>")),
            ("BR-62", replace(valid_ubl(), "<cac:Party><cac:PartyIdentification>", "<cac:Party><cbc:EndpointID>supplier.example</cbc:EndpointID><cac:PartyIdentification>")),
            ("BR-63", replace(valid_ubl(), "<cac:AccountingCustomerParty><cac:Party><cac:PartyName>", "<cac:AccountingCustomerParty><cac:Party><cbc:EndpointID>buyer.example</cbc:EndpointID><cac:PartyName>")),
            ("BR-64", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:Item><cbc:Name>Implementation service</cbc:Name><cac:StandardItemIdentification><cbc:ID>1234567890123</cbc:ID></cac:StandardItemIdentification>")),
            ("BR-65", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:Item><cbc:Name>Implementation service</cbc:Name><cac:CommodityClassification><cbc:ItemClassificationCode>1234</cbc:ItemClassificationCode></cac:CommodityClassification>")),
            ("BR-AE-05", replace(valid_ubl(), "<cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID>", "<cac:ClassifiedTaxCategory><cbc:ID>AE</cbc:ID>")),
            ("BR-AE-08", replace(&replace(&replace(valid_ubl(), "<cac:TaxCategory><cbc:ID>S</cbc:ID>", "<cac:TaxCategory><cbc:ID>AE</cbc:ID>"), "<cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID>", "<cac:ClassifiedTaxCategory><cbc:ID>AE</cbc:ID>"), "<cbc:TaxableAmount>100.00</cbc:TaxableAmount>", "<cbc:TaxableAmount>99.00</cbc:TaxableAmount>")),
            ("BR-AE-10", replace(valid_ubl(), "<cac:TaxCategory><cbc:ID>S</cbc:ID>", "<cac:TaxCategory><cbc:ID>AE</cbc:ID>")),
            ("BR-CL-17", replace(valid_ubl(), "<cac:TaxCategory><cbc:ID>S</cbc:ID>", "<cac:TaxCategory><cbc:ID>AA</cbc:ID>")),
            ("BR-CL-18", replace(valid_ubl(), "<cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID>", "<cac:ClassifiedTaxCategory><cbc:ID>AA</cbc:ID>")),
            ("BR-CO-03", insert_before(valid_ubl(), "<cbc:DocumentCurrencyCode>", "<cbc:TaxPointDate>2026-05-27</cbc:TaxPointDate><cac:InvoicePeriod><cbc:DescriptionCode>3</cbc:DescriptionCode></cac:InvoicePeriod>")),
            ("BR-CO-04", replace(valid_ubl(), "<cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID>", "<cac:ClassifiedTaxCategory>")),
            ("BR-CO-09", replace(valid_ubl(), "<cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName>", "<cac:PartyTaxScheme><cbc:CompanyID>ZZ123456789</cbc:CompanyID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:PartyTaxScheme><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName>")),
            ("BR-CO-10", replace(valid_ubl(), "<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount>", "<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>101.00</cbc:LineExtensionAmount>")),
            ("BR-CO-11", replace(&insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Discount</cbc:AllowanceChargeReason><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>"), "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount>", "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount><cbc:AllowanceTotalAmount>2.00</cbc:AllowanceTotalAmount>")),
            ("BR-CO-12", replace(&insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:AllowanceChargeReason>Freight</cbc:AllowanceChargeReason><cbc:Amount>1.00</cbc:Amount><cac:TaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:AllowanceCharge>"), "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount>", "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount><cbc:ChargeTotalAmount>2.00</cbc:ChargeTotalAmount>")),
            ("BR-CO-13", replace(valid_ubl(), "<cbc:TaxExclusiveAmount>100.00</cbc:TaxExclusiveAmount>", "<cbc:TaxExclusiveAmount>101.00</cbc:TaxExclusiveAmount>")),
            ("BR-CO-14", replace(valid_ubl(), "<cbc:TaxAmount currencyID=\"EUR\">19.00</cbc:TaxAmount>", "<cbc:TaxAmount currencyID=\"EUR\">18.00</cbc:TaxAmount>")),
            ("BR-CO-15", replace(valid_ubl(), "<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount>", "<cbc:TaxInclusiveAmount>120.00</cbc:TaxInclusiveAmount>")),
            ("BR-CO-16", replace(valid_ubl(), "<cbc:PayableAmount>119.00</cbc:PayableAmount>", "<cbc:PayableAmount>118.00</cbc:PayableAmount>")),
            ("BR-CO-17", replace(valid_ubl(), "<cbc:TaxAmount>19.00</cbc:TaxAmount><cac:TaxCategory>", "<cbc:TaxAmount>18.00</cbc:TaxAmount><cac:TaxCategory>")),
            ("BR-CO-18", replace(valid_ubl(), tax_subtotal(), "")),
            ("BR-CO-19", insert_before(valid_ubl(), "<cac:AccountingSupplierParty>", "<cac:InvoicePeriod></cac:InvoicePeriod>")),
            ("BR-CO-20", replace(valid_ubl(), "<cac:InvoiceLine><cbc:ID>1</cbc:ID>", "<cac:InvoiceLine><cac:InvoicePeriod></cac:InvoicePeriod><cbc:ID>1</cbc:ID>")),
            ("BR-CO-21", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge>")),
            ("BR-CO-22", insert_before(valid_ubl(), "<cac:TaxTotal>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge>")),
            ("BR-CO-23", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>false</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-CO-24", replace(valid_ubl(), "<cac:Item><cbc:Name>Implementation service</cbc:Name>", "<cac:AllowanceCharge><cbc:ChargeIndicator>true</cbc:ChargeIndicator><cbc:Amount>1.00</cbc:Amount></cac:AllowanceCharge><cac:Item><cbc:Name>Implementation service</cbc:Name>")),
            ("BR-CO-26", replace(valid_ubl(), "<cac:PartyIdentification><cbc:ID>SUPPLIER-1</cbc:ID></cac:PartyIdentification>", "")),
        ];

        for (rule_id, xml) in cases {
            let report = validate_xml(&xml).unwrap();
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.rule_id.as_str() == rule_id),
                "{rule_id} was not emitted; got {:?}",
                report
                    .findings
                    .iter()
                    .map(|finding| finding.rule_id.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    fn assert_emits_rule(xml: &str, rule_id: &str) {
        let report = validate_xml(xml).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.rule_id.as_str() == rule_id),
            "{rule_id} was not emitted; got {:?}",
            report
                .findings
                .iter()
                .map(|finding| finding.rule_id.as_str())
                .collect::<Vec<_>>()
        );
    }

    fn assert_not_emits_rule(xml: &str, rule_id: &str) {
        let report = validate_xml(xml).unwrap();
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.rule_id.as_str() == rule_id),
            "{rule_id} was emitted; got {:?}",
            report
                .findings
                .iter()
                .map(|finding| finding.rule_id.as_str())
                .collect::<Vec<_>>()
        );
    }

    fn assert_emits_rule_with_severity(xml: &str, rule_id: &str, severity: Severity) {
        let report = validate_xml(xml).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.rule_id.as_str() == rule_id && finding.severity == severity),
            "{rule_id} with {severity:?} was not emitted; got {:?}",
            report
                .findings
                .iter()
                .map(|finding| (finding.rule_id.as_str(), finding.severity))
                .collect::<Vec<_>>()
        );
    }

    fn replace(input: &str, from: &str, to: &str) -> String {
        assert!(input.contains(from), "fixture missing replacement: {from}");
        input.replacen(from, to, 1)
    }

    fn insert_before(input: &str, marker: &str, insert: &str) -> String {
        assert!(input.contains(marker), "fixture missing marker: {marker}");
        input.replacen(marker, &format!("{insert}{marker}"), 1)
    }

    fn supplier_address() -> &'static str {
        "<cac:PostalAddress><cbc:StreetName>Main Street 1</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
    }

    fn customer_address() -> &'static str {
        "<cac:PostalAddress><cbc:StreetName>Main Street 2</cbc:StreetName><cbc:CityName>Amsterdam</cbc:CityName><cbc:PostalZone>1000AA</cbc:PostalZone><cac:Country><cbc:IdentificationCode>NL</cbc:IdentificationCode></cac:Country></cac:PostalAddress>"
    }

    fn invoice_line() -> &'static str {
        "<cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:InvoicedQuantity unitCode=\"C62\">1</cbc:InvoicedQuantity><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cac:Item><cbc:Name>Implementation service</cbc:Name><cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:ClassifiedTaxCategory></cac:Item><cac:Price><cbc:PriceAmount>100.00</cbc:PriceAmount></cac:Price></cac:InvoiceLine>"
    }

    fn tax_subtotal() -> &'static str {
        "<cac:TaxSubtotal><cbc:TaxableAmount>100.00</cbc:TaxableAmount><cbc:TaxAmount>19.00</cbc:TaxAmount><cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:TaxSubtotal>"
    }

    fn valid_ubl() -> &'static str {
        r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
<cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>
<cbc:ID>INV-1</cbc:ID>
<cbc:IssueDate>2026-05-27</cbc:IssueDate>
<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>
<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>
<cac:AccountingSupplierParty><cac:Party><cac:PartyIdentification><cbc:ID>SUPPLIER-1</cbc:ID></cac:PartyIdentification><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main Street 1</cbc:StreetName><cbc:CityName>Berlin</cbc:CityName><cbc:PostalZone>10115</cbc:PostalZone><cac:Country><cbc:IdentificationCode>DE</cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyLegalEntity><cbc:RegistrationName>Supplier GmbH</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingSupplierParty>
<cac:AccountingCustomerParty><cac:Party><cac:PartyName><cbc:Name>Customer BV</cbc:Name></cac:PartyName><cac:PostalAddress><cbc:StreetName>Main Street 2</cbc:StreetName><cbc:CityName>Amsterdam</cbc:CityName><cbc:PostalZone>1000AA</cbc:PostalZone><cac:Country><cbc:IdentificationCode>NL</cbc:IdentificationCode></cac:Country></cac:PostalAddress><cac:PartyLegalEntity><cbc:RegistrationName>Customer BV</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingCustomerParty>
<cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:InvoicedQuantity unitCode="C62">1</cbc:InvoicedQuantity><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cac:Item><cbc:Name>Implementation service</cbc:Name><cac:ClassifiedTaxCategory><cbc:ID>S</cbc:ID><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:ClassifiedTaxCategory></cac:Item><cac:Price><cbc:PriceAmount>100.00</cbc:PriceAmount></cac:Price></cac:InvoiceLine>
<cac:TaxTotal><cbc:TaxAmount currencyID="EUR">19.00</cbc:TaxAmount><cac:TaxSubtotal><cbc:TaxableAmount>100.00</cbc:TaxableAmount><cbc:TaxAmount>19.00</cbc:TaxAmount><cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:TaxSubtotal></cac:TaxTotal>
<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount>100.00</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount><cbc:PayableAmount>119.00</cbc:PayableAmount></cac:LegalMonetaryTotal>
</ubl:Invoice>"#
    }

    fn large_ubl_invoice(line_count: usize) -> String {
        let line_cents = 10_000usize;
        let total_cents = line_cents * line_count;
        let tax_cents = total_cents * 19 / 100;
        let payable_cents = total_cents + tax_cents;
        let mut lines = String::with_capacity(line_count * invoice_line().len());
        for index in 0..line_count {
            lines.push_str(&invoice_line().replace(
                "<cbc:ID>1</cbc:ID>",
                &format!("<cbc:ID>{}</cbc:ID>", index + 1),
            ));
        }
        format!(
            r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
<cbc:CustomizationID>urn:cen.eu:en16931:2017</cbc:CustomizationID>
<cbc:ID>INV-LARGE</cbc:ID>
<cbc:IssueDate>2026-05-27</cbc:IssueDate>
<cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>
<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>
<cac:AccountingSupplierParty><cac:Party><cac:PartyIdentification><cbc:ID>SUPPLIER-1</cbc:ID></cac:PartyIdentification><cac:PartyName><cbc:Name>Supplier GmbH</cbc:Name></cac:PartyName>{supplier}<cac:PartyLegalEntity><cbc:RegistrationName>Supplier GmbH</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingSupplierParty>
<cac:AccountingCustomerParty><cac:Party><cac:PartyName><cbc:Name>Customer BV</cbc:Name></cac:PartyName>{customer}<cac:PartyLegalEntity><cbc:RegistrationName>Customer BV</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingCustomerParty>
{lines}
<cac:TaxTotal><cbc:TaxAmount currencyID="EUR">{tax}</cbc:TaxAmount><cac:TaxSubtotal><cbc:TaxableAmount>{total}</cbc:TaxableAmount><cbc:TaxAmount>{tax}</cbc:TaxAmount><cac:TaxCategory><cbc:ID>S</cbc:ID><cbc:Percent>19.00</cbc:Percent><cac:TaxScheme><cbc:ID>VAT</cbc:ID></cac:TaxScheme></cac:TaxCategory></cac:TaxSubtotal></cac:TaxTotal>
<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>{total}</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount>{total}</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount>{payable}</cbc:TaxInclusiveAmount><cbc:PayableAmount>{payable}</cbc:PayableAmount></cac:LegalMonetaryTotal>
</ubl:Invoice>"#,
            supplier = supplier_address(),
            customer = customer_address(),
            lines = lines,
            total = money(total_cents),
            tax = money(tax_cents),
            payable = money(payable_cents),
        )
    }

    fn money(cents: usize) -> String {
        format!("{}.{:02}", cents / 100, cents % 100)
    }

    fn valid_cii() -> &'static str {
        r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100" xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100" xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100">
<rsm:ExchangedDocumentContext><ram:GuidelineSpecifiedDocumentContextParameter><ram:ID>urn:cen.eu:en16931:2017</ram:ID></ram:GuidelineSpecifiedDocumentContextParameter></rsm:ExchangedDocumentContext>
<rsm:ExchangedDocument><ram:ID>INV-1</ram:ID><ram:TypeCode>380</ram:TypeCode><ram:IssueDateTime><udt:DateTimeString format="102">20260527</udt:DateTimeString></ram:IssueDateTime></rsm:ExchangedDocument>
<rsm:SupplyChainTradeTransaction>
<ram:IncludedSupplyChainTradeLineItem><ram:AssociatedDocumentLineDocument><ram:LineID>1</ram:LineID></ram:AssociatedDocumentLineDocument><ram:SpecifiedTradeProduct><ram:Name>Implementation service</ram:Name></ram:SpecifiedTradeProduct><ram:SpecifiedLineTradeAgreement><ram:NetPriceProductTradePrice><ram:ChargeAmount>100.00</ram:ChargeAmount></ram:NetPriceProductTradePrice></ram:SpecifiedLineTradeAgreement><ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery><ram:SpecifiedLineTradeSettlement><ram:ApplicableTradeTax><ram:TypeCode>VAT</ram:TypeCode><ram:CategoryCode>S</ram:CategoryCode></ram:ApplicableTradeTax><ram:SpecifiedTradeSettlementLineMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount></ram:SpecifiedTradeSettlementLineMonetarySummation></ram:SpecifiedLineTradeSettlement></ram:IncludedSupplyChainTradeLineItem>
<ram:ApplicableHeaderTradeAgreement><ram:SellerTradeParty><ram:ID>SUPPLIER-1</ram:ID><ram:Name>Supplier GmbH</ram:Name><ram:PostalTradeAddress><ram:LineOne>Main Street 1</ram:LineOne><ram:CityName>Berlin</ram:CityName><ram:PostcodeCode>10115</ram:PostcodeCode><ram:CountryID>DE</ram:CountryID></ram:PostalTradeAddress></ram:SellerTradeParty><ram:BuyerTradeParty><ram:Name>Customer BV</ram:Name><ram:PostalTradeAddress><ram:LineOne>Main Street 2</ram:LineOne><ram:CityName>Amsterdam</ram:CityName><ram:PostcodeCode>1000AA</ram:PostcodeCode><ram:CountryID>NL</ram:CountryID></ram:PostalTradeAddress></ram:BuyerTradeParty></ram:ApplicableHeaderTradeAgreement>
<ram:ApplicableHeaderTradeSettlement><ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode><ram:ApplicableTradeTax><ram:TypeCode>VAT</ram:TypeCode><ram:CalculatedAmount>19.00</ram:CalculatedAmount><ram:BasisAmount>100.00</ram:BasisAmount><ram:CategoryCode>S</ram:CategoryCode><ram:RateApplicablePercent>19.00</ram:RateApplicablePercent></ram:ApplicableTradeTax><ram:SpecifiedTradeSettlementHeaderMonetarySummation><ram:LineTotalAmount>100.00</ram:LineTotalAmount><ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount><ram:TaxTotalAmount currencyID="EUR">19.00</ram:TaxTotalAmount><ram:GrandTotalAmount>119.00</ram:GrandTotalAmount><ram:DuePayableAmount>119.00</ram:DuePayableAmount></ram:SpecifiedTradeSettlementHeaderMonetarySummation></ram:ApplicableHeaderTradeSettlement>
</rsm:SupplyChainTradeTransaction>
</rsm:CrossIndustryInvoice>"#
    }
}
