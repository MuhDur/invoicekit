// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-intake-witness` — deterministic cross-examination
//! of AI-extracted invoice fields.
//!
//! Every value the AI intake layers (PaddleOCR, SmolDocling,
//! Qwen2.5-VL) emit gets re-validated by the deterministic
//! checks in this crate before the engine commits the
//! [`ExtractedDocument`] to the canonical IR. Mismatches block
//! AI-only emission and surface as [`WitnessFailure`] entries
//! with a stable rule id and the citation paths of the
//! offending fields, so the audit UI can highlight the wrong
//! values for human review.
//!
//! # Rules shipped today
//!
//! * [`rules::LINE_TOTAL_RECONCILES`] — every line's
//!   `quantity * unit_price - line_discount + line_charge`
//!   equals the reported `line_net_amount` within
//!   currency-rounding tolerance.
//! * [`rules::VAT_SUBTOTALS_CLOSE`] — the sum of per-line
//!   VAT amounts across the document equals the reported
//!   document-level VAT total.
//! * [`rules::VAT_ID_VALIDATES`] — the supplier and customer
//!   VAT identifiers are well-formed under the EU country
//!   prefix taxonomy (a precondition for the live VIES round-
//!   trip a follow-up `intake-witness-vies` crate makes).

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

/// Stable rule ids the witness layer reports against.
pub mod rules {
    /// Line totals reconcile inside currency-rounding tolerance.
    pub const LINE_TOTAL_RECONCILES: &str = "witness.line_total.reconciles";
    /// VAT subtotals close across the document.
    pub const VAT_SUBTOTALS_CLOSE: &str = "witness.vat.subtotals_close";
    /// VAT id well-formed under EU prefix taxonomy.
    pub const VAT_ID_VALIDATES: &str = "witness.vat_id.validates";
}

/// One line on the AI-extracted invoice the witness inspects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractedLine {
    /// 0-indexed line number; surfaces in citation paths.
    pub index: u32,
    /// Quantity.
    pub quantity: Decimal,
    /// Unit price (price-per-unit, exclusive of VAT).
    pub unit_price: Decimal,
    /// Line discount; subtracted from `quantity * unit_price`.
    pub line_discount: Decimal,
    /// Line charge; added on top of `quantity * unit_price`.
    pub line_charge: Decimal,
    /// VAT amount the AI extracted for this line.
    pub vat_amount: Decimal,
    /// Net amount the AI extracted for this line.
    pub line_net_amount: Decimal,
}

/// One VAT party identifier the witness inspects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractedParty {
    /// JSON-pointer-style path (e.g. `/supplier/vat_id`).
    pub path: String,
    /// EU VAT identifier the AI extracted, including prefix.
    pub vat_id: String,
}

/// The AI-extracted document the witness re-validates.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExtractedDocument {
    /// All extracted lines, in document order.
    pub lines: Vec<ExtractedLine>,
    /// Document-level VAT total the AI extracted.
    pub document_vat_total: Decimal,
    /// Currency rounding tolerance — defaults to 0.01 (one
    /// minor unit) which matches EN-16931 BR-CO rounding.
    pub rounding_tolerance: Decimal,
    /// VAT parties to validate (typically supplier + customer).
    pub parties: Vec<ExtractedParty>,
}

/// One rule failure the witness surfaces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WitnessFailure {
    /// Stable rule id (see [`rules`]).
    pub rule_id: String,
    /// JSON-pointer-style paths of the cited fields.
    pub cited_fields: Vec<String>,
    /// Operator-facing message explaining the mismatch.
    pub message: String,
}

/// Outcome of a witness run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WitnessOutcome {
    /// All checks passed; the engine may commit the document.
    Passed,
    /// At least one check failed; the engine MUST block emission.
    Failed(Vec<WitnessFailure>),
}

impl WitnessOutcome {
    /// True when the outcome is [`WitnessOutcome::Passed`].
    #[must_use]
    pub const fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// Failures slice (empty when passed).
    #[must_use]
    pub fn failures(&self) -> &[WitnessFailure] {
        match self {
            Self::Passed => &[],
            Self::Failed(f) => f.as_slice(),
        }
    }
}

/// Errors raised by the witness runner itself (distinct from
/// rule failures, which are surfaced as [`WitnessFailure`]).
#[derive(Debug, Error)]
pub enum WitnessError {
    /// Rounding tolerance was negative or non-finite.
    #[error("rounding_tolerance must be >= 0; got {0}")]
    InvalidTolerance(Decimal),
}

/// Run every deterministic check against the extracted
/// document. Returns [`WitnessOutcome::Passed`] when all rules
/// agree, [`WitnessOutcome::Failed`] otherwise.
///
/// # Errors
///
/// Returns [`WitnessError::InvalidTolerance`] when
/// `document.rounding_tolerance` is negative.
pub fn cross_examine(document: &ExtractedDocument) -> Result<WitnessOutcome, WitnessError> {
    if document.rounding_tolerance.is_sign_negative() {
        return Err(WitnessError::InvalidTolerance(document.rounding_tolerance));
    }
    let tolerance = if document.rounding_tolerance.is_zero() {
        Decimal::new(1, 2) // 0.01
    } else {
        document.rounding_tolerance
    };

    let mut failures = Vec::new();
    failures.extend(check_line_totals(document, tolerance));
    failures.extend(check_vat_subtotals(document, tolerance));
    failures.extend(check_vat_ids(document));

    if failures.is_empty() {
        debug!("witness: all checks passed");
        Ok(WitnessOutcome::Passed)
    } else {
        debug!(count = failures.len(), "witness: failures detected");
        Ok(WitnessOutcome::Failed(failures))
    }
}

fn check_line_totals(doc: &ExtractedDocument, tolerance: Decimal) -> Vec<WitnessFailure> {
    let mut out = Vec::new();
    for line in &doc.lines {
        let expected = line.quantity * line.unit_price - line.line_discount + line.line_charge;
        let diff = (expected - line.line_net_amount).abs();
        if diff > tolerance {
            let prefix = format!("/lines/{}", line.index);
            out.push(WitnessFailure {
                rule_id: rules::LINE_TOTAL_RECONCILES.to_owned(),
                cited_fields: vec![
                    format!("{prefix}/quantity"),
                    format!("{prefix}/unit_price"),
                    format!("{prefix}/line_discount"),
                    format!("{prefix}/line_charge"),
                    format!("{prefix}/line_net_amount"),
                ],
                message: format!(
                    "line {} net mismatch: expected {} but got {} (diff {})",
                    line.index, expected, line.line_net_amount, diff
                ),
            });
        }
    }
    out
}

fn check_vat_subtotals(doc: &ExtractedDocument, tolerance: Decimal) -> Vec<WitnessFailure> {
    let line_vat_sum: Decimal = doc.lines.iter().map(|l| l.vat_amount).sum();
    let diff = (line_vat_sum - doc.document_vat_total).abs();
    if diff > tolerance {
        let mut cited: Vec<String> = doc
            .lines
            .iter()
            .map(|l| format!("/lines/{}/vat_amount", l.index))
            .collect();
        cited.push("/document_vat_total".to_owned());
        vec![WitnessFailure {
            rule_id: rules::VAT_SUBTOTALS_CLOSE.to_owned(),
            cited_fields: cited,
            message: format!(
                "vat subtotals do not close: line sum {} vs document total {} (diff {})",
                line_vat_sum, doc.document_vat_total, diff
            ),
        }]
    } else {
        Vec::new()
    }
}

fn check_vat_ids(doc: &ExtractedDocument) -> Vec<WitnessFailure> {
    let mut out = Vec::new();
    for party in &doc.parties {
        if let Err(reason) = validate_eu_vat_id_shape(&party.vat_id) {
            out.push(WitnessFailure {
                rule_id: rules::VAT_ID_VALIDATES.to_owned(),
                cited_fields: vec![party.path.clone()],
                message: format!("vat id {} rejected: {}", party.vat_id, reason),
            });
        }
    }
    out
}

/// Validate the shape of an EU VAT identifier under the
/// canonical country prefix taxonomy. Returns the country code
/// on success.
///
/// This is the deterministic precondition the live VIES
/// round-trip (in the follow-up `intake-witness-vies` crate)
/// runs *after* — VIES sometimes accepts malformed ids in
/// degraded mode, but the witness must not.
///
/// # Errors
///
/// Returns an `Err(&'static str)` reason when the prefix is
/// unknown, the body length is outside `[2, 12]`, or the body
/// contains characters outside `[A-Z0-9+*]` (the union the
/// EU's published validation matrix permits).
pub fn validate_eu_vat_id_shape(vat_id: &str) -> Result<&'static str, &'static str> {
    if vat_id.len() < 4 {
        return Err("too short");
    }
    let (prefix, body) = vat_id.split_at(2);
    let Some(cc) = eu_country_for_prefix(prefix) else {
        return Err("unknown country prefix");
    };
    if !(2..=12).contains(&body.len()) {
        return Err("body length out of range");
    }
    for c in body.chars() {
        if !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '+' || c == '*') {
            return Err("body contains illegal character");
        }
    }
    Ok(cc)
}

fn eu_country_for_prefix(prefix: &str) -> Option<&'static str> {
    static PREFIXES: &[(&str, &str)] = &[
        ("AT", "AT"),
        ("BE", "BE"),
        ("BG", "BG"),
        ("CY", "CY"),
        ("CZ", "CZ"),
        ("DE", "DE"),
        ("DK", "DK"),
        ("EE", "EE"),
        ("EL", "GR"), // Greece — VIES uses EL prefix
        ("ES", "ES"),
        ("FI", "FI"),
        ("FR", "FR"),
        ("HR", "HR"),
        ("HU", "HU"),
        ("IE", "IE"),
        ("IT", "IT"),
        ("LT", "LT"),
        ("LU", "LU"),
        ("LV", "LV"),
        ("MT", "MT"),
        ("NL", "NL"),
        ("PL", "PL"),
        ("PT", "PT"),
        ("RO", "RO"),
        ("SE", "SE"),
        ("SI", "SI"),
        ("SK", "SK"),
        ("XI", "XI"), // Northern Ireland under Windsor framework
    ];
    // Canonicalisation gate: prefix must already be uppercase.
    // The live VIES endpoint accepts mixed case, but the
    // witness re-validates the *AI extraction*, which must
    // commit to a single canonical form before emission.
    PREFIXES
        .iter()
        .find(|(p, _)| *p == prefix)
        .map(|(_, cc)| *cc)
}

/// Group failures by rule id for the audit dashboard.
#[must_use]
pub fn group_failures_by_rule(failures: &[WitnessFailure]) -> BTreeMap<&str, Vec<&WitnessFailure>> {
    let mut out: BTreeMap<&str, Vec<&WitnessFailure>> = BTreeMap::new();
    for f in failures {
        out.entry(f.rule_id.as_str()).or_default().push(f);
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_intake_witness::crate_name(),
///     "invoicekit-intake-witness"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-witness"
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromPrimitive;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn happy_doc() -> ExtractedDocument {
        ExtractedDocument {
            lines: vec![
                ExtractedLine {
                    index: 0,
                    quantity: dec("2"),
                    unit_price: dec("50.00"),
                    line_discount: dec("0"),
                    line_charge: dec("0"),
                    vat_amount: dec("19.00"),
                    line_net_amount: dec("100.00"),
                },
                ExtractedLine {
                    index: 1,
                    quantity: dec("1"),
                    unit_price: dec("200.00"),
                    line_discount: dec("10.00"),
                    line_charge: dec("5.00"),
                    vat_amount: dec("37.05"),
                    line_net_amount: dec("195.00"),
                },
            ],
            document_vat_total: dec("56.05"),
            rounding_tolerance: dec("0.01"),
            parties: vec![
                ExtractedParty {
                    path: "/supplier/vat_id".to_owned(),
                    vat_id: "DE123456789".to_owned(),
                },
                ExtractedParty {
                    path: "/customer/vat_id".to_owned(),
                    vat_id: "FRAA999999999".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-intake-witness");
    }

    #[test]
    fn happy_path_passes_every_rule() {
        let outcome = cross_examine(&happy_doc()).unwrap();
        assert!(outcome.is_passed(), "expected pass, got {outcome:?}");
        assert_eq!(outcome.failures(), &[]);
    }

    #[test]
    fn line_total_mismatch_surfaces_actionable_failure() {
        let mut doc = happy_doc();
        doc.lines[0].line_net_amount = dec("999.00"); // wrong
        let outcome = cross_examine(&doc).unwrap();
        let failures = outcome.failures();
        let lt: Vec<&WitnessFailure> = failures
            .iter()
            .filter(|f| f.rule_id == rules::LINE_TOTAL_RECONCILES)
            .collect();
        assert_eq!(lt.len(), 1);
        assert!(lt[0].cited_fields.contains(&"/lines/0/quantity".to_owned()));
        assert!(lt[0]
            .cited_fields
            .contains(&"/lines/0/line_net_amount".to_owned()));
        assert!(lt[0].message.contains("line 0"));
    }

    #[test]
    fn vat_subtotals_mismatch_surfaces_actionable_failure() {
        let mut doc = happy_doc();
        doc.document_vat_total = dec("9999.99");
        let outcome = cross_examine(&doc).unwrap();
        let vt: Vec<&WitnessFailure> = outcome
            .failures()
            .iter()
            .filter(|f| f.rule_id == rules::VAT_SUBTOTALS_CLOSE)
            .collect();
        assert_eq!(vt.len(), 1);
        assert!(vt[0]
            .cited_fields
            .contains(&"/document_vat_total".to_owned()));
        assert!(vt[0]
            .cited_fields
            .contains(&"/lines/0/vat_amount".to_owned()));
    }

    #[test]
    fn vat_id_invalid_prefix_surfaces_actionable_failure() {
        let mut doc = happy_doc();
        doc.parties[0].vat_id = "ZZ123456789".to_owned();
        let outcome = cross_examine(&doc).unwrap();
        let vi: Vec<&WitnessFailure> = outcome
            .failures()
            .iter()
            .filter(|f| f.rule_id == rules::VAT_ID_VALIDATES)
            .collect();
        assert_eq!(vi.len(), 1);
        assert_eq!(vi[0].cited_fields, vec!["/supplier/vat_id"]);
        assert!(vi[0].message.contains("unknown country prefix"));
    }

    #[test]
    fn vat_id_invalid_body_length_surfaces_failure() {
        let mut doc = happy_doc();
        doc.parties[1].vat_id = "FRA".to_owned(); // body too short
        let outcome = cross_examine(&doc).unwrap();
        let count = outcome
            .failures()
            .iter()
            .filter(|f| f.rule_id == rules::VAT_ID_VALIDATES)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn vat_id_lowercase_prefix_is_rejected() {
        // EU's published matrix says VAT id MUST be uppercase
        // before the deterministic shape check accepts it.
        // Live VIES will accept lowercase but the witness must
        // not — it's the canonicalisation gate.
        assert!(validate_eu_vat_id_shape("de123456789").is_err());
    }

    #[test]
    fn rounding_tolerance_zero_falls_back_to_one_minor_unit() {
        let mut doc = happy_doc();
        doc.rounding_tolerance = Decimal::ZERO;
        doc.lines[0].line_net_amount = dec("100.005"); // within 0.01
        let outcome = cross_examine(&doc).unwrap();
        assert!(outcome.is_passed());
    }

    #[test]
    fn rounding_tolerance_negative_is_rejected() {
        let mut doc = happy_doc();
        doc.rounding_tolerance = Decimal::from_f64(-0.01).unwrap();
        let err = cross_examine(&doc).unwrap_err();
        assert!(matches!(err, WitnessError::InvalidTolerance(_)));
    }

    #[test]
    fn outcome_round_trips_through_serde() {
        let doc = happy_doc();
        let outcome = cross_examine(&doc).unwrap();
        let json = serde_json::to_string(&outcome).unwrap();
        let back: WitnessOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }

    #[test]
    fn group_failures_by_rule_groups_correctly() {
        let mut doc = happy_doc();
        doc.lines[0].line_net_amount = dec("1");
        doc.document_vat_total = dec("1");
        doc.parties[0].vat_id = "ZZ123".to_owned();
        let outcome = cross_examine(&doc).unwrap();
        let groups = group_failures_by_rule(outcome.failures());
        assert!(groups.contains_key(rules::LINE_TOTAL_RECONCILES));
        assert!(groups.contains_key(rules::VAT_SUBTOTALS_CLOSE));
        assert!(groups.contains_key(rules::VAT_ID_VALIDATES));
    }

    #[test]
    fn three_rule_ids_are_stable_strings() {
        assert_eq!(
            rules::LINE_TOTAL_RECONCILES,
            "witness.line_total.reconciles"
        );
        assert_eq!(rules::VAT_SUBTOTALS_CLOSE, "witness.vat.subtotals_close");
        assert_eq!(rules::VAT_ID_VALIDATES, "witness.vat_id.validates");
    }

    #[test]
    fn validate_eu_vat_id_shape_accepts_well_known_prefixes() {
        assert!(validate_eu_vat_id_shape("DE123456789").is_ok());
        assert!(validate_eu_vat_id_shape("FRAA999999999").is_ok());
        assert!(validate_eu_vat_id_shape("ELABC12345678").is_ok());
        assert!(validate_eu_vat_id_shape("XI123456789").is_ok());
    }
}
