// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-121: pathological invoice generator for differential testing.
//!
//! Build a catalogue of [`CommercialDocument`] instances that
//! exercise edge cases the bead enumerates (zero amount, negative
//! amount, allowance > total, mixed-rate VAT, single line, large
//! line count) and a helper that emits each through every
//! shipped serializer. T-123 (the differential harness) consumes
//! the same catalogue via [`generate_adversarial_corpus`] so the
//! two beads stay in lockstep on what "pathological" means.

use invoicekit_format_cii::{to_xml as cii_to_xml, CiiError};
use invoicekit_format_gobl::to_gobl;
use invoicekit_format_ubl::{to_xml as ubl_to_xml, UblError};
use invoicekit_ir::{
    Attachment, CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly,
    DecimalValue, DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference,
    DocumentType, IrError, Iso4217Code, LocalizedString, MonetaryTotal, Party, PartyTaxId,
    PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion,
    TaxCategorySummary,
};
use invoicekit_profile_peppol_bis::{to_peppol_bis_3_0_xml, PeppolBisError};
use invoicekit_profile_peppol_pint::{to_peppol_pint_xml, PintCountry, PintError};
use invoicekit_profile_xrechnung::{to_xrechnung_3_x_xml, XRechnungError, XRechnungOptions};
use rust_decimal::Decimal;
use thiserror::Error;

/// Stable identifier for one adversarial scenario in the corpus.
///
/// Add a new variant only when there is a real differential test
/// case that needs it. Removing a variant is a breaking change for
/// the T-123 differential harness.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AdversarialScenario {
    /// Single line with a zero unit price and zero totals.
    ZeroAmountLine,
    /// Single line whose unit price is negative (refund-style
    /// invoice). Most validators flag this on EN 16931 BR-CO-14.
    NegativeAmountLine,
    /// Allowance total exceeds the line extension sum, producing a
    /// negative tax-exclusive amount. Trips BR-CO-15 in most
    /// EN 16931 profiles.
    AllowanceGreaterThanTotals,
    /// Two lines under different VAT category codes
    /// (`S` standard rate and `Z` zero rate) so the tax summary
    /// must carry two distinct buckets.
    MixedVatRates,
    /// Single line invoice — minimum valid invoice shape.
    SingleLine,
    /// 50 lines — stresses serializer per-line allocations.
    HighLineCount,
    /// Unicode-heavy supplier name + line descriptions (full-width
    /// CJK, Arabic with right-to-left override, ZWJ emoji).
    UnicodeStress,
}

impl AdversarialScenario {
    /// All scenarios in stable iteration order. Used by callers
    /// that want the full corpus without enumerating manually.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::ZeroAmountLine,
            Self::NegativeAmountLine,
            Self::AllowanceGreaterThanTotals,
            Self::MixedVatRates,
            Self::SingleLine,
            Self::HighLineCount,
            Self::UnicodeStress,
        ]
    }

    /// Operator-readable name suitable for log lines / test failure
    /// messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ZeroAmountLine => "zero-amount-line",
            Self::NegativeAmountLine => "negative-amount-line",
            Self::AllowanceGreaterThanTotals => "allowance-greater-than-totals",
            Self::MixedVatRates => "mixed-vat-rates",
            Self::SingleLine => "single-line",
            Self::HighLineCount => "high-line-count",
            Self::UnicodeStress => "unicode-stress",
        }
    }
}

/// Errors surfaced by the generator.
#[derive(Debug, Error)]
pub enum AdversarialError {
    /// The IR layer rejected the constructed document. This means
    /// the scenario falls outside the layered invoice model's
    /// invariants (e.g. the negative-amount scenario would fail
    /// `MonetaryAmount::new` if the IR enforced non-negativity).
    /// We surface it so the caller can decide whether to drop the
    /// scenario or relax the IR contract.
    #[error("IR construction failed for {scenario}: {source}")]
    Ir {
        /// Scenario that triggered the failure.
        scenario: &'static str,
        /// Underlying IR error.
        #[source]
        source: IrError,
    },
}

/// Build the full corpus of adversarial documents.
///
/// Returns one `(scenario, document)` per registered scenario.
/// Construction-time IR errors are surfaced rather than silently
/// dropped — that signal is the whole point of T-123.
///
/// # Errors
///
/// Returns the first [`AdversarialError`] encountered. Callers that
/// want to inspect every error individually can iterate
/// [`AdversarialScenario::all`] and call [`build_scenario`] per
/// variant.
pub fn generate_adversarial_corpus(
) -> Result<Vec<(AdversarialScenario, CommercialDocument)>, AdversarialError> {
    let mut out = Vec::with_capacity(AdversarialScenario::all().len());
    for scenario in AdversarialScenario::all() {
        let document = build_scenario(*scenario)?;
        out.push((*scenario, document));
    }
    Ok(out)
}

/// Build a single adversarial document.
///
/// # Errors
///
/// Returns [`AdversarialError::Ir`] when the IR rejects the
/// constructed shape.
pub fn build_scenario(
    scenario: AdversarialScenario,
) -> Result<CommercialDocument, AdversarialError> {
    let parts = match scenario {
        AdversarialScenario::ZeroAmountLine => zero_amount_parts(),
        AdversarialScenario::NegativeAmountLine => negative_amount_parts(),
        AdversarialScenario::AllowanceGreaterThanTotals => allowance_over_totals_parts(),
        AdversarialScenario::MixedVatRates => mixed_vat_parts(),
        AdversarialScenario::SingleLine => single_line_parts(),
        AdversarialScenario::HighLineCount => high_line_count_parts(),
        AdversarialScenario::UnicodeStress => unicode_stress_parts(),
    };
    CommercialDocument::new(parts).map_err(|source| AdversarialError::Ir {
        scenario: scenario.name(),
        source,
    })
}

/// One serializer's response to a single scenario.
#[derive(Debug)]
pub struct SerializerOutcome {
    /// Stable identifier for the serializer.
    pub serializer: &'static str,
    /// `Some(bytes)` when the serializer accepted the input;
    /// `None` when the serializer returned a typed error. The
    /// stringified error is captured in `error`.
    pub output: Option<String>,
    /// Stringified serializer error, when any.
    pub error: Option<String>,
}

/// Emit `document` through every shipped serializer and return one
/// [`SerializerOutcome`] per serializer.
///
/// The differential harness (T-123) consumes this directly: a
/// scenario that succeeds on one serializer and fails on another
/// is a useful divergence; a scenario that succeeds everywhere with
/// byte-different output is the more interesting kind.
#[must_use]
pub fn emit_through_every_serializer(document: &CommercialDocument) -> Vec<SerializerOutcome> {
    let mut outcomes = vec![
        run::<UblError>("format-ubl", ubl_to_xml(document)),
        run::<CiiError>("format-cii", cii_to_xml(document)),
        run::<XRechnungError>(
            "profile-xrechnung",
            to_xrechnung_3_x_xml(document, &XRechnungOptions::default()),
        ),
        run::<PeppolBisError>("profile-peppol-bis", to_peppol_bis_3_0_xml(document)),
    ];
    for country in [
        PintCountry::AustraliaNewZealand,
        PintCountry::Singapore,
        PintCountry::Japan,
        PintCountry::UnitedArabEmirates,
        PintCountry::Malaysia,
    ] {
        outcomes.push(run::<PintError>(
            pint_serializer_name(country),
            to_peppol_pint_xml(document, country),
        ));
    }
    // GOBL projection emits JSON; we wrap it through serde_json to
    // produce a string the differential harness can hash.
    outcomes.push(match to_gobl(document) {
        Ok(envelope) => SerializerOutcome {
            serializer: "format-gobl",
            output: Some(serde_json::to_string(&envelope.document).unwrap_or_default()),
            error: None,
        },
        Err(err) => SerializerOutcome {
            serializer: "format-gobl",
            output: None,
            error: Some(format!("{err}")),
        },
    });
    outcomes
}

fn run<E: std::fmt::Display>(
    serializer: &'static str,
    result: Result<String, E>,
) -> SerializerOutcome {
    match result {
        Ok(output) => SerializerOutcome {
            serializer,
            output: Some(output),
            error: None,
        },
        Err(err) => SerializerOutcome {
            serializer,
            output: None,
            error: Some(format!("{err}")),
        },
    }
}

const fn pint_serializer_name(country: PintCountry) -> &'static str {
    match country {
        PintCountry::AustraliaNewZealand => "profile-peppol-pint:au-nz",
        PintCountry::Singapore => "profile-peppol-pint:sg",
        PintCountry::Japan => "profile-peppol-pint:jp",
        PintCountry::UnitedArabEmirates => "profile-peppol-pint:ae",
        PintCountry::Malaysia => "profile-peppol-pint:my",
    }
}

// ----- Scenario builders -----------------------------------------

fn base_parts(id_suffix: &str) -> CommercialDocumentParts {
    CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,
        id: DocumentId::new(format!("adv-{id_suffix}")).unwrap(),
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-27").unwrap(),
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        document_number: DocumentNumber::new(format!("ADV-{id_suffix}")).unwrap(),
        currency: Iso4217Code::new("EUR").unwrap(),
        supplier: party("supplier", "DE", 1),
        customer: party("customer", "FR", 2),
        payee: None,
        payment_terms: Some(PaymentTerms {
            description: "Net 30".to_owned(),
            due_date: Some(DateOnly::new("2026-06-26").unwrap()),
        }),
        payment_instructions: vec![PaymentInstruction {
            kind: PaymentInstructionKind::IbanBic,
            account: Some("DE89370400440532013000".to_owned()),
            reference: Some("RF0001".to_owned()),
        }],
        lines: Vec::new(),
        tax_summary: Vec::new(),
        monetary_total: zero_totals(),
        attachments: Vec::<Attachment>::new(),
        references: Vec::<DocumentReference>::new(),
        notes: Vec::<LocalizedString>::new(),
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: "tenant-adversarial".to_owned(),
            trace_id: "trace-adversarial".to_owned(),
            source_system: None,
        },
    }
}

fn party(role: &str, country: &str, idx: u32) -> Party {
    Party {
        id: Some(format!("{role}-{idx}")),
        name: format!("{role} {idx} GmbH"),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: format!("DE{idx:09}"),
        }],
        address: PostalAddress {
            lines: vec![format!("{role} Street {idx}")],
            city: "Berlin".to_owned(),
            subdivision: None,
            postal_code: format!("{:05}", 10_000 + idx),
            country: CountryCode::new(country).unwrap(),
        },
        contact: Some(Contact {
            name: Some(format!("{role} contact")),
            email: None,
            phone: None,
        }),
    }
}

fn zero_totals() -> MonetaryTotal {
    MonetaryTotal {
        line_extension_amount: DecimalValue::new(Decimal::ZERO),
        tax_exclusive_amount: DecimalValue::new(Decimal::ZERO),
        tax_inclusive_amount: DecimalValue::new(Decimal::ZERO),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(Decimal::ZERO),
    }
}

fn one_line(
    id: &str,
    description: &str,
    quantity: i64,
    unit_price: Decimal,
    line_amount: Decimal,
    tax_category: Option<&str>,
) -> DocumentLine {
    DocumentLine {
        id: id.to_owned(),
        description: description.to_owned(),
        quantity: DecimalValue::new(Decimal::from(quantity)),
        unit_code: Some("EA".to_owned()),
        unit_price: DecimalValue::new(unit_price),
        line_extension_amount: DecimalValue::new(line_amount),
        tax_category: tax_category.map(str::to_owned),
        extensions: Vec::new(),
    }
}

fn zero_amount_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("zero-amount");
    parts.lines = vec![one_line(
        "L1",
        "Zero-amount line",
        1,
        Decimal::ZERO,
        Decimal::ZERO,
        Some("Z"),
    )];
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "Z".to_owned(),
        taxable_amount: DecimalValue::new(Decimal::ZERO),
        tax_amount: DecimalValue::new(Decimal::ZERO),
        tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
    }];
    parts
}

fn negative_amount_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("negative-amount");
    let unit_price = Decimal::new(-1000, 2); // -10.00
    parts.lines = vec![one_line(
        "L1",
        "Negative-amount refund line",
        1,
        unit_price,
        unit_price,
        Some("S"),
    )];
    parts.monetary_total.line_extension_amount = DecimalValue::new(unit_price);
    parts.monetary_total.tax_exclusive_amount = DecimalValue::new(unit_price);
    let tax_amount = (unit_price * Decimal::new(19, 2)).round_dp(2);
    let inclusive = unit_price + tax_amount;
    parts.monetary_total.tax_inclusive_amount = DecimalValue::new(inclusive);
    parts.monetary_total.payable_amount = DecimalValue::new(inclusive);
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: DecimalValue::new(unit_price),
        tax_amount: DecimalValue::new(tax_amount),
        tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
    }];
    parts
}

fn allowance_over_totals_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("allowance-over-totals");
    let unit = Decimal::new(1000, 2); // 10.00
    parts.lines = vec![one_line(
        "L1",
        "Single line under enormous allowance",
        1,
        unit,
        unit,
        Some("S"),
    )];
    let allowance = Decimal::new(5000, 2); // 50.00 allowance > 10.00 line total
    let tax_exclusive = unit - allowance;
    let tax_amount = (tax_exclusive * Decimal::new(19, 2)).round_dp(2);
    let inclusive = tax_exclusive + tax_amount;
    parts.monetary_total = MonetaryTotal {
        line_extension_amount: DecimalValue::new(unit),
        tax_exclusive_amount: DecimalValue::new(tax_exclusive),
        tax_inclusive_amount: DecimalValue::new(inclusive),
        allowance_total_amount: Some(DecimalValue::new(allowance)),
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(inclusive),
    };
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: DecimalValue::new(tax_exclusive),
        tax_amount: DecimalValue::new(tax_amount),
        tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
    }];
    parts
}

fn mixed_vat_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("mixed-vat-rates");
    let standard_line = Decimal::new(10000, 2); // 100.00
    let zero_line = Decimal::new(5000, 2); // 50.00
    parts.lines = vec![
        one_line(
            "L1",
            "Standard-rate widget",
            1,
            standard_line,
            standard_line,
            Some("S"),
        ),
        one_line(
            "L2",
            "Zero-rate exported widget",
            1,
            zero_line,
            zero_line,
            Some("Z"),
        ),
    ];
    let line_total = standard_line + zero_line;
    let standard_tax = (standard_line * Decimal::new(19, 2)).round_dp(2);
    let inclusive = line_total + standard_tax;
    parts.monetary_total = MonetaryTotal {
        line_extension_amount: DecimalValue::new(line_total),
        tax_exclusive_amount: DecimalValue::new(line_total),
        tax_inclusive_amount: DecimalValue::new(inclusive),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(inclusive),
    };
    parts.tax_summary = vec![
        TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: DecimalValue::new(standard_line),
            tax_amount: DecimalValue::new(standard_tax),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        },
        TaxCategorySummary {
            category_code: "Z".to_owned(),
            taxable_amount: DecimalValue::new(zero_line),
            tax_amount: DecimalValue::new(Decimal::ZERO),
            tax_rate: Some(DecimalValue::new(Decimal::ZERO)),
        },
    ];
    parts
}

fn single_line_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("single-line");
    let unit = Decimal::new(5000, 2); // 50.00
    parts.lines = vec![one_line("L1", "Single line item", 1, unit, unit, Some("S"))];
    let tax = (unit * Decimal::new(19, 2)).round_dp(2);
    let inclusive = unit + tax;
    parts.monetary_total = MonetaryTotal {
        line_extension_amount: DecimalValue::new(unit),
        tax_exclusive_amount: DecimalValue::new(unit),
        tax_inclusive_amount: DecimalValue::new(inclusive),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(inclusive),
    };
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: DecimalValue::new(unit),
        tax_amount: DecimalValue::new(tax),
        tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
    }];
    parts
}

fn high_line_count_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("high-line-count");
    let per_line = Decimal::new(100, 2); // 1.00 per line
    let mut total = Decimal::ZERO;
    parts.lines = Vec::with_capacity(50);
    for idx in 1..=50u32 {
        parts.lines.push(one_line(
            &format!("L{idx}"),
            "Line item",
            1,
            per_line,
            per_line,
            Some("S"),
        ));
        total += per_line;
    }
    let tax = (total * Decimal::new(19, 2)).round_dp(2);
    let inclusive = total + tax;
    parts.monetary_total = MonetaryTotal {
        line_extension_amount: DecimalValue::new(total),
        tax_exclusive_amount: DecimalValue::new(total),
        tax_inclusive_amount: DecimalValue::new(inclusive),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(inclusive),
    };
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: DecimalValue::new(total),
        tax_amount: DecimalValue::new(tax),
        tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
    }];
    parts
}

fn unicode_stress_parts() -> CommercialDocumentParts {
    let mut parts = base_parts("unicode-stress");
    "オフィス家具株式会社 - مكتب الأثاث".clone_into(&mut parts.supplier.name);
    let unit = Decimal::new(12345, 2);
    parts.lines = vec![one_line(
        "L1",
        "Description with full-width CJK 日本語 + Arabic مرحبا + ZWJ emoji 👨‍👩‍👧‍👦",
        1,
        unit,
        unit,
        Some("S"),
    )];
    let tax = (unit * Decimal::new(19, 2)).round_dp(2);
    let inclusive = unit + tax;
    parts.monetary_total = MonetaryTotal {
        line_extension_amount: DecimalValue::new(unit),
        tax_exclusive_amount: DecimalValue::new(unit),
        tax_inclusive_amount: DecimalValue::new(inclusive),
        allowance_total_amount: None,
        charge_total_amount: None,
        prepaid_amount: None,
        payable_amount: DecimalValue::new(inclusive),
    };
    parts.tax_summary = vec![TaxCategorySummary {
        category_code: "S".to_owned(),
        taxable_amount: DecimalValue::new(unit),
        tax_amount: DecimalValue::new(tax),
        tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
    }];
    parts
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_adversarial_generator::crate_name(),
///     "invoicekit-adversarial-generator"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-adversarial-generator"
}

#[cfg(test)]
mod tests {
    use super::{
        build_scenario, crate_name, emit_through_every_serializer, generate_adversarial_corpus,
        AdversarialScenario,
    };

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-adversarial-generator");
    }

    #[test]
    fn corpus_covers_every_registered_scenario() {
        let corpus = generate_adversarial_corpus().expect("corpus must build");
        assert_eq!(corpus.len(), AdversarialScenario::all().len());
        // Stable order.
        for (got, expected) in corpus.iter().zip(AdversarialScenario::all()) {
            assert_eq!(got.0, *expected);
        }
    }

    #[test]
    fn every_scenario_emits_through_every_serializer() {
        // The bead's gate "Emits via every serializer" is satisfied
        // when every scenario produces a SerializerOutcome for every
        // serializer (including the typed-error outcomes — a typed
        // error is signal, not a regression).
        for scenario in AdversarialScenario::all() {
            let document = build_scenario(*scenario).expect("scenario builds");
            let outcomes = emit_through_every_serializer(&document);
            assert!(
                outcomes.len() >= 10,
                "scenario {:?} only produced {} outcomes",
                scenario,
                outcomes.len()
            );
            // No two outcomes for the same serializer.
            let mut names: Vec<&str> = outcomes.iter().map(|o| o.serializer).collect();
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            assert_eq!(
                before,
                names.len(),
                "duplicate serializer names in outcomes for {scenario:?}"
            );
            // Every outcome populated exactly one of (output, error).
            for outcome in &outcomes {
                assert_ne!(
                    outcome.output.is_some(),
                    outcome.error.is_some(),
                    "{:?}/{} populated both output and error",
                    scenario,
                    outcome.serializer
                );
            }
        }
    }

    #[test]
    fn mixed_vat_scenario_has_two_distinct_tax_buckets() {
        let document = build_scenario(AdversarialScenario::MixedVatRates).expect("builds");
        assert_eq!(document.tax_summary.len(), 2);
        let mut codes: Vec<&str> = document
            .tax_summary
            .iter()
            .map(|t| t.category_code.as_str())
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, vec!["S", "Z"]);
    }

    #[test]
    fn high_line_count_scenario_has_50_lines() {
        let document = build_scenario(AdversarialScenario::HighLineCount).expect("builds");
        assert_eq!(document.lines.len(), 50);
    }

    #[test]
    fn allowance_scenario_records_allowance_exceeding_line_total() {
        let document =
            build_scenario(AdversarialScenario::AllowanceGreaterThanTotals).expect("builds");
        let allowance = document
            .monetary_total
            .allowance_total_amount
            .as_ref()
            .expect("allowance recorded");
        let line_total = &document.monetary_total.line_extension_amount;
        assert!(
            allowance.inner() > line_total.inner(),
            "allowance {allowance:?} should exceed line total {line_total:?}"
        );
    }

    #[test]
    fn scenario_names_are_kebab_case_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for scenario in AdversarialScenario::all() {
            let name = scenario.name();
            assert!(!name.is_empty());
            for c in name.chars() {
                assert!(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            }
            assert!(seen.insert(name), "duplicate scenario name: {name}");
        }
    }
}
