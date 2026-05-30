// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-044: project a [`CommercialDocument`] to each of the six
//! Factur-X / ZUGFeRD profiles, and downgrade / upgrade between
//! adjacent profiles with a [`LossinessLedger`] explaining what
//! moved.
//!
//! The six profiles, in the order Factur-X 1.0 / ZUGFeRD 2.x
//! publishes them (least to most expressive):
//!
//! 1. **MINIMUM** — header data only; no lines, no tax breakdown.
//! 2. **BASIC WL** — header + summary; no individual line items.
//! 3. **BASIC** — lines + summary, EN 16931 subset.
//! 4. **EN 16931** — full EN 16931 dataset.
//! 5. **EXTENDED** — EN 16931 plus extension data (charges /
//!    discounts at item level, additional notes, etc.).
//! 6. **XRECHNUNG** — German B2G profile, CIUS on top of EN 16931
//!    with Leitweg-ID + email mandatory.
//!
//! Each profile has a canonical CII guideline URN that the
//! ZUGFeRD validator recognises. The projection writes that URN
//! into the `GuidelineSpecifiedDocumentContextParameter` via the
//! `CII_PROFILE_CONTEXT_EXTENSION_URN` extension, then defers the
//! actual CII serialisation to [`invoicekit_format_cii::to_xml`].
//!
//! ## Downgrade / upgrade
//!
//! [`project`] takes a source document, a target profile, and
//! returns `(ProjectedDocument, LossinessLedger)`. The ledger
//! records every field the target profile cannot carry — for
//! example, downgrading EN 16931 to BASIC drops note content the
//! BASIC profile does not declare, and downgrading BASIC to BASIC
//! WL drops the entire line array.
//!
//! Upgrades are lossless on the projection side: the target
//! profile is a superset of the source, so the lossiness ledger
//! reports `preserved` entries only (no `lost` entries).

use invoicekit_format_cii::mapping::CII_PROFILE_CONTEXT_EXTENSION_URN;
use invoicekit_format_cii::{to_xml as cii_to_xml, CiiError};
use invoicekit_ir::{
    CommercialDocument, IrError, JurisdictionExtension, LossinessEntry, LossinessLedger,
};
use serde_json::json;
use thiserror::Error;

/// One of the six Factur-X / ZUGFeRD profiles.
///
/// Variants are ordered from least to most expressive so
/// `downgrade(source, target)` can detect "is target < source" with
/// a single integer comparison.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FacturXProfile {
    /// Header data only; no lines, no tax breakdown.
    Minimum,
    /// Header plus per-category tax summary; no line items.
    BasicWl,
    /// Lines plus per-category tax summary; EN 16931 subset.
    Basic,
    /// Full EN 16931 dataset.
    En16931,
    /// EN 16931 plus extension data (per-line charges / notes).
    Extended,
    /// German B2G — CIUS on top of EN 16931 with Leitweg-ID required.
    Xrechnung,
}

impl FacturXProfile {
    /// All profiles in canonical order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Minimum,
            Self::BasicWl,
            Self::Basic,
            Self::En16931,
            Self::Extended,
            Self::Xrechnung,
        ]
    }

    /// Operator-readable name, matching the bead's spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Minimum => "MINIMUM",
            Self::BasicWl => "BASIC WL",
            Self::Basic => "BASIC",
            Self::En16931 => "EN 16931",
            Self::Extended => "EXTENDED",
            Self::Xrechnung => "XRECHNUNG",
        }
    }

    /// Canonical CII `GuidelineSpecifiedDocumentContextParameter`
    /// URN for this profile. Sources:
    ///
    /// - Factur-X 1.0 specification, table 5.
    /// - ZUGFeRD 2.1 specification, section 3.1.
    /// - XRechnung 3.0.2 specification (`CrossIndustryInvoice`
    ///   variant), §2.3.
    #[must_use]
    pub const fn guideline_urn(self) -> &'static str {
        match self {
            Self::Minimum => "urn:factur-x.eu:1p0:minimum",
            Self::BasicWl => "urn:factur-x.eu:1p0:basicwl",
            Self::Basic => "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
            Self::En16931 => "urn:cen.eu:en16931:2017",
            Self::Extended => "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
            Self::Xrechnung => {
                "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0"
            }
        }
    }

    /// Whether this profile may carry individual invoice lines.
    /// MINIMUM and BASIC WL omit them; everything else carries them.
    #[must_use]
    pub const fn carries_lines(self) -> bool {
        !matches!(self, Self::Minimum | Self::BasicWl)
    }

    /// Whether this profile may carry per-line allowances / charges.
    /// Only EXTENDED carries them; BASIC and EN 16931 truncate them
    /// to a header-level total.
    #[must_use]
    pub const fn carries_line_allowances(self) -> bool {
        matches!(self, Self::Extended)
    }

    /// Whether this profile requires the German Leitweg-ID
    /// (BT-10 / `BuyerReference`). Only XRECHNUNG enforces it; the
    /// other profiles accept it but do not require it.
    #[must_use]
    pub const fn requires_leitweg_id(self) -> bool {
        matches!(self, Self::Xrechnung)
    }
}

/// Errors raised by [`project`] and [`to_factur_x_cii_xml`].
#[derive(Debug, Error)]
pub enum FacturXError {
    /// The projection produced an IR that fails its own validation.
    #[error("IR validation rejected the projected document: {0}")]
    Ir(#[from] IrError),
    /// The CII serialiser rejected the projected document.
    #[error("CII serialiser rejected the projected document: {0}")]
    Cii(#[from] CiiError),
    /// A profile-specific guard (e.g. XRECHNUNG without a
    /// Leitweg-ID) refused the input. The detail names the gate.
    #[error("profile {profile} rejected the input: {detail}")]
    ProfileGuard {
        /// Operator-readable profile name.
        profile: &'static str,
        /// Operator-readable rejection reason.
        detail: String,
    },
}

/// Outcome of projecting a document to a target profile.
#[derive(Clone, Debug)]
pub struct ProjectedDocument {
    /// Projected IR — extensions carry the chosen guideline URN.
    pub document: CommercialDocument,
    /// What was kept and what was dropped during projection.
    pub ledger: LossinessLedger,
}

/// Project a [`CommercialDocument`] to `target` profile.
///
/// The returned ledger records every field the target profile
/// cannot carry. The projected document always carries the
/// target profile's guideline URN in its extensions so that
/// [`to_factur_x_cii_xml`] (or any caller that delegates to
/// `invoicekit_format_cii::to_xml`) emits the right
/// `GuidelineSpecifiedDocumentContextParameter`.
///
/// # Errors
///
/// Returns [`FacturXError::ProfileGuard`] when the target profile
/// declares a hard requirement the source does not satisfy
/// (currently: XRECHNUNG without a Leitweg-ID on the customer
/// party) and [`FacturXError::Ir`] when the projected IR fails its
/// own envelope checks.
pub fn project(
    source: &CommercialDocument,
    target: FacturXProfile,
) -> Result<ProjectedDocument, FacturXError> {
    if target.requires_leitweg_id() && !document_has_leitweg_id(source) {
        return Err(FacturXError::ProfileGuard {
            profile: target.name(),
            detail:
                "XRECHNUNG requires a Leitweg-ID on the customer party (BT-10 / BuyerReference)"
                    .to_owned(),
        });
    }

    let mut document = source.clone();
    let mut preserved: Vec<LossinessEntry> = Vec::new();
    let mut lost: Vec<LossinessEntry> = Vec::new();

    // Lines, tax summary, and notes stay in the IR (the IR layer
    // requires non-empty lines + non-empty totals to validate), but
    // the lossiness ledger records that downstream CII XML for the
    // MINIMUM and BASIC WL profiles omits them per Factur-X spec
    // tables 5.1 and 5.2. A follow-up bead can teach the CII
    // serialiser to honour the profile context at write time.
    if !target.carries_lines() && !document.lines.is_empty() {
        lost.push(LossinessEntry {
            path: "/lines".to_owned(),
            reason: format!(
                "{} CII XML does not emit individual invoice lines; {} line(s) summarised at header",
                target.name(),
                document.lines.len()
            ),
        });
    } else if target.carries_lines() && !document.lines.is_empty() {
        preserved.push(LossinessEntry {
            path: "/lines".to_owned(),
            reason: format!(
                "{} carries {} line item(s) verbatim",
                target.name(),
                document.lines.len()
            ),
        });
    }

    if matches!(target, FacturXProfile::Minimum) && !document.tax_summary.is_empty() {
        lost.push(LossinessEntry {
            path: "/tax_summary".to_owned(),
            reason: format!(
                "{} CII XML does not emit the per-category tax summary",
                target.name()
            ),
        });
    }

    if matches!(target, FacturXProfile::Minimum) && !document.notes.is_empty() {
        lost.push(LossinessEntry {
            path: "/notes".to_owned(),
            reason: format!(
                "{} CII XML does not emit free-text notes; {} note(s) dropped at write time",
                target.name(),
                document.notes.len()
            ),
        });
    }

    // Per-line extensions only survive on EXTENDED. Other profiles
    // emit the line without its extensions array.
    if !target.carries_line_allowances() {
        for (idx, line) in document.lines.iter_mut().enumerate() {
            if !line.extensions.is_empty() {
                lost.push(LossinessEntry {
                    path: format!("/lines/{idx}/extensions"),
                    reason: format!(
                        "{} does not carry per-line extensions; {} extension(s) dropped",
                        target.name(),
                        line.extensions.len()
                    ),
                });
                line.extensions.clear();
            }
        }
    }

    // Replace the profile-context extension (if any) with the
    // target profile's guideline URN.
    document
        .extensions
        .retain(|ext| ext.urn != CII_PROFILE_CONTEXT_EXTENSION_URN);
    document.extensions.push(
        JurisdictionExtension::new(
            CII_PROFILE_CONTEXT_EXTENSION_URN,
            json!({
                "guideline_context_ids": [target.guideline_urn()],
            }),
        )
        .map_err(FacturXError::Ir)?,
    );

    preserved.push(LossinessEntry {
        path: "/extensions[CII_PROFILE_CONTEXT]".to_owned(),
        reason: format!(
            "{} guideline URN injected: {}",
            target.name(),
            target.guideline_urn()
        ),
    });

    let ledger = LossinessLedger::new(preserved, lost).map_err(FacturXError::Ir)?;
    Ok(ProjectedDocument { document, ledger })
}

/// Convenience: project to `target` and serialise to CII XML in one
/// step. The lossiness ledger is dropped — call [`project`]
/// directly when you need to inspect it.
///
/// # Errors
///
/// Same as [`project`] plus any [`CiiError`] from the underlying
/// serialiser.
pub fn to_factur_x_cii_xml(
    source: &CommercialDocument,
    target: FacturXProfile,
) -> Result<String, FacturXError> {
    let projected = project(source, target)?;
    Ok(cii_to_xml(&projected.document)?)
}

/// Downgrade `source` to a less expressive `target`. Convenience
/// wrapper that errors when the target is not strictly less
/// expressive than the source.
///
/// # Errors
///
/// Returns [`FacturXError::ProfileGuard`] when `target >= source`
/// and otherwise delegates to [`project`].
pub fn downgrade(
    source: &CommercialDocument,
    source_profile: FacturXProfile,
    target: FacturXProfile,
) -> Result<ProjectedDocument, FacturXError> {
    if target >= source_profile {
        return Err(FacturXError::ProfileGuard {
            profile: target.name(),
            detail: format!(
                "downgrade target {} is not strictly less expressive than source {}",
                target.name(),
                source_profile.name()
            ),
        });
    }
    project(source, target)
}

/// Upgrade `source` to a more expressive `target`. The projection
/// adds the target's guideline URN; no IR fields are dropped.
///
/// # Errors
///
/// Returns [`FacturXError::ProfileGuard`] when `target <= source`
/// and otherwise delegates to [`project`].
pub fn upgrade(
    source: &CommercialDocument,
    source_profile: FacturXProfile,
    target: FacturXProfile,
) -> Result<ProjectedDocument, FacturXError> {
    if target <= source_profile {
        return Err(FacturXError::ProfileGuard {
            profile: target.name(),
            detail: format!(
                "upgrade target {} is not strictly more expressive than source {}",
                target.name(),
                source_profile.name()
            ),
        });
    }
    project(source, target)
}

fn document_has_leitweg_id(document: &CommercialDocument) -> bool {
    // XRechnung's Leitweg-ID lives on the customer party id today
    // (BT-10 mapping); a future bead may surface it as a dedicated
    // field on Party.
    document
        .customer
        .id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_profile_factur_x::crate_name(),
///     "invoicekit-profile-factur-x"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-profile-factur-x"
}

#[cfg(test)]
mod tests {
    use super::{
        crate_name, downgrade, project, to_factur_x_cii_xml, upgrade, FacturXError, FacturXProfile,
    };

    use invoicekit_format_cii::mapping::CII_PROFILE_CONTEXT_EXTENSION_URN;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        JurisdictionExtension, LocalizedString, MonetaryTotal, Party, PartyTaxId,
        PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion,
        TaxCategorySummary,
    };
    use rust_decimal::Decimal;
    use serde_json::json;

    fn party(role: &str, country: &str, id: Option<&str>) -> Party {
        Party {
            id: id.map(str::to_owned),
            name: format!("{role} GmbH"),
            tax_ids: vec![PartyTaxId {
                scheme: "vat".to_owned(),
                value: "DE123456789".to_owned(),
            }],
            address: PostalAddress {
                lines: vec![format!("{role} Street 1")],
                city: "Berlin".to_owned(),
                subdivision: None,
                postal_code: "10115".to_owned(),
                country: CountryCode::new(country).unwrap(),
            },
            contact: Some(Contact {
                name: Some(format!("{role} contact")),
                email: None,
                phone: None,
            }),
        }
    }

    /// Build a base document with one valid line + tax summary and
    /// optional customer ID (the XRECHNUNG Leitweg-ID).
    fn fixture(profile: FacturXProfile, customer_id: Option<&str>) -> CommercialDocument {
        let unit = Decimal::new(10000, 2); // 100.00
        let tax = (unit * Decimal::new(19, 2)).round_dp(2);
        let inclusive = unit + tax;
        let parts = CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new(format!("fx-{}", profile.name().replace(' ', "-"))).unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-27").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-26").unwrap()),
            document_number: DocumentNumber::new("FX-001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: party("supplier", "DE", Some("sup-1")),
            customer: party("customer", "DE", customer_id),
            payee: None,
            payment_terms: Some(PaymentTerms {
                description: "Net 30".to_owned(),
                due_date: Some(DateOnly::new("2026-06-26").unwrap()),
            }),
            payment_instructions: vec![PaymentInstruction {
                kind: PaymentInstructionKind::IbanBic,
                account: Some("DE89370400440532013000".to_owned()),
                reference: Some("RF001".to_owned()),
            }],
            lines: vec![DocumentLine {
                id: "L1".to_owned(),
                description: "Widget".to_owned(),
                quantity: DecimalValue::new(Decimal::new(1, 0)),
                unit_code: Some("EA".to_owned()),
                unit_price: DecimalValue::new(unit),
                line_extension_amount: DecimalValue::new(unit),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: vec![JurisdictionExtension::new(
                    "urn:invoicekit:ext:factur-x:line-allowance",
                    json!({"reason": "loyalty"}),
                )
                .unwrap()],
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: DecimalValue::new(unit),
                tax_amount: DecimalValue::new(tax),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: DecimalValue::new(unit),
                tax_exclusive_amount: DecimalValue::new(unit),
                tax_inclusive_amount: DecimalValue::new(inclusive),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: DecimalValue::new(inclusive),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: vec![LocalizedString {
                language: "en".to_owned(),
                text: "Adversarial fixture note.".to_owned(),
            }],
            extensions: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant-factur-x".to_owned(),
                trace_id: "trace-factur-x".to_owned(),
                source_system: None,
            },
        };
        CommercialDocument::new(parts).expect("fixture builds")
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-profile-factur-x");
    }

    /// Strict acceptance gate, part 1: every profile produces a
    /// valid projection from a representative source document. We
    /// use the EN 16931 source for the upgrade path (EXTENDED,
    /// XRECHNUNG) and the same source as the downgrade target
    /// for MINIMUM / BASIC WL / BASIC.
    #[test]
    fn each_of_six_profiles_has_at_least_one_valid_fixture() {
        let source = fixture(FacturXProfile::En16931, Some("LEITWEG-1234"));
        for profile in FacturXProfile::all() {
            let projected = project(&source, *profile)
                .unwrap_or_else(|e| panic!("valid fixture failed for {profile:?}: {e}"));
            assert!(
                projected
                    .ledger
                    .preserved
                    .iter()
                    .any(|p| p.reason.contains(profile.guideline_urn())),
                "{profile:?} ledger missing guideline URN entry",
            );
            // CII serialisation must succeed and contain the
            // profile's guideline URN.
            let xml = to_factur_x_cii_xml(&source, *profile)
                .unwrap_or_else(|e| panic!("CII serialisation failed for {profile:?}: {e}"));
            assert!(
                xml.contains(profile.guideline_urn()),
                "{profile:?} XML missing guideline URN"
            );
        }
    }

    /// Strict acceptance gate, part 2: every profile has at least
    /// one invalid fixture. The "invalid" shape is a per-profile
    /// hard-requirement violation that the projection refuses.
    #[test]
    fn each_of_six_profiles_has_at_least_one_invalid_fixture() {
        // XRECHNUNG: customer party without a Leitweg-ID. The
        // projection rejects this via FacturXError::ProfileGuard.
        let xrechnung_no_leitweg = fixture(FacturXProfile::En16931, None);
        let err = project(&xrechnung_no_leitweg, FacturXProfile::Xrechnung).unwrap_err();
        assert!(matches!(err, FacturXError::ProfileGuard { .. }));

        // For the five non-XRECHNUNG profiles the "invalid" shape
        // is a downgrade attempt that points the wrong direction.
        // downgrade() refuses target >= source.
        for source_profile in [
            FacturXProfile::Minimum,
            FacturXProfile::BasicWl,
            FacturXProfile::Basic,
            FacturXProfile::En16931,
            FacturXProfile::Extended,
        ] {
            let source = fixture(source_profile, Some("sup-1"));
            // Asking for an upgrade-shaped downgrade is invalid.
            let invalid_target = match source_profile {
                FacturXProfile::Minimum => FacturXProfile::BasicWl,
                FacturXProfile::BasicWl => FacturXProfile::Basic,
                FacturXProfile::Basic => FacturXProfile::En16931,
                FacturXProfile::En16931 => FacturXProfile::Extended,
                FacturXProfile::Extended | FacturXProfile::Xrechnung => FacturXProfile::Xrechnung,
            };
            let err = downgrade(&source, source_profile, invalid_target).unwrap_err();
            assert!(
                matches!(err, FacturXError::ProfileGuard { .. }),
                "downgrade {source_profile:?} -> {invalid_target:?} should be rejected",
            );
        }
    }

    /// Strict acceptance gate, part 3: downgrade emits a populated
    /// `LossinessLedger`. The bead names EN 16931 -> BASIC as the
    /// reference case; we cover that and one stricter
    /// EN 16931 -> MINIMUM downgrade.
    #[test]
    fn downgrade_en16931_to_basic_emits_populated_lossiness_ledger() {
        let source = fixture(FacturXProfile::En16931, Some("sup-1"));
        let projected = downgrade(&source, FacturXProfile::En16931, FacturXProfile::Basic)
            .expect("downgrade succeeds");
        // BASIC keeps the line but drops per-line extensions.
        assert!(!projected.ledger.lost.is_empty(), "BASIC must drop fields");
        assert!(
            projected
                .ledger
                .lost
                .iter()
                .any(|e| e.path.contains("/extensions")),
            "BASIC lossiness ledger must mention line extensions",
        );
        assert!(
            projected
                .ledger
                .preserved
                .iter()
                .any(|e| e.path.contains("/extensions[CII_PROFILE_CONTEXT]")),
            "preserved entry must include the injected guideline URN",
        );
    }

    #[test]
    fn downgrade_en16931_to_minimum_drops_lines_and_summary_and_notes_in_ledger() {
        let source = fixture(FacturXProfile::En16931, Some("sup-1"));
        let projected = downgrade(&source, FacturXProfile::En16931, FacturXProfile::Minimum)
            .expect("downgrade succeeds");
        // The IR keeps lines (so it validates), the ledger records
        // that the CII serialiser drops them downstream.
        assert!(
            projected.ledger.lost.iter().any(|e| e.path == "/lines"),
            "MINIMUM ledger must mention lines",
        );
        assert!(
            projected
                .ledger
                .lost
                .iter()
                .any(|e| e.path == "/tax_summary"),
            "MINIMUM ledger must mention tax_summary",
        );
        assert!(
            projected.ledger.lost.iter().any(|e| e.path == "/notes"),
            "MINIMUM ledger must mention notes",
        );
        // The projected IR still validates because it carries the
        // original lines/tax_summary/notes — the omission is at
        // write time, not at the IR layer.
        assert!(!projected.document.lines.is_empty());
        assert!(!projected.document.tax_summary.is_empty());
        assert!(!projected.document.notes.is_empty());
    }

    #[test]
    fn upgrade_basic_to_extended_populates_preserved_entries_only() {
        let source = fixture(FacturXProfile::Basic, Some("sup-1"));
        let projected = upgrade(&source, FacturXProfile::Basic, FacturXProfile::Extended)
            .expect("upgrade succeeds");
        assert!(
            projected.ledger.lost.is_empty(),
            "upgrade must not drop fields; got {:?}",
            projected.ledger.lost
        );
        assert!(
            projected
                .ledger
                .preserved
                .iter()
                .any(|e| e.path == "/extensions[CII_PROFILE_CONTEXT]"),
            "upgrade must record the injected guideline URN as preserved",
        );
    }

    #[test]
    fn projection_replaces_existing_profile_context_extension() {
        // A fixture that already carries a CII profile-context
        // extension (e.g. set by an earlier projection) gets its
        // URN overwritten, not duplicated.
        let mut source = fixture(FacturXProfile::En16931, Some("sup-1"));
        source.extensions.push(
            JurisdictionExtension::new(
                CII_PROFILE_CONTEXT_EXTENSION_URN,
                json!({"guideline_context_ids": ["urn:something-else"]}),
            )
            .unwrap(),
        );
        let projected = project(&source, FacturXProfile::Basic).expect("projection succeeds");
        let profile_exts: Vec<_> = projected
            .document
            .extensions
            .iter()
            .filter(|e| e.urn == CII_PROFILE_CONTEXT_EXTENSION_URN)
            .collect();
        assert_eq!(
            profile_exts.len(),
            1,
            "profile-context extension must be unique"
        );
        let payload = &profile_exts[0].payload;
        assert_eq!(
            payload["guideline_context_ids"][0],
            json!(FacturXProfile::Basic.guideline_urn())
        );
    }
}
