// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-046: compute a populated [`LossinessLedger`] for every
//! supported cross-format projection.
//!
//! The bead's strict gate calls for "every projection produces a
//! populated [`LossinessLedger`] listing preserved and lost
//! fields". This crate is the producer.
//!
//! Two projection styles are supported:
//!
//! 1. **Profile projections** (Factur-X / ZUGFeRD) — delegate to
//!    [`invoicekit_profile_factur_x::project`], which already
//!    emits a ledger.
//! 2. **Format projections** (UBL, CII) — serialize the source IR
//!    through the target's adapter, reparse the emitted bytes
//!    back into IR, and compare the two trees via
//!    [`LossinessLedger::from_roundtrip_comparison`]. This is not
//!    an exhaustive per-field guarantee: the diff compares a fixed
//!    set of top-level IR fields (identity fields such as `/id`,
//!    `/document_number`, `/currency`, and dates; payload fields
//!    such as `/lines`, `/tax_summary`, `/notes`, `/extensions`)
//!    by whole-field equality. A top-level field whose value
//!    survived the round-trip lands in `preserved`; one that
//!    drifted (or vanished) lands in `lost`, identified by its
//!    top-level path rather than the specific element that changed.
//!
//! Both paths surface the result as a [`LossinessLedger`] so the
//! evidence bundle (T-080) can attach the ledger verbatim.

use invoicekit_format_cii::{from_xml as cii_from_xml, to_xml as cii_to_xml, CiiError};
use invoicekit_format_ubl::{from_xml as ubl_from_xml, to_xml as ubl_to_xml, UblError};
use invoicekit_ir::{CommercialDocument, IrError, LossinessLedger};
use invoicekit_profile_factur_x::{
    project as factur_x_project, FacturXError, FacturXProfile, ProjectedDocument,
};
use thiserror::Error;

/// Target format / profile for a ledger computation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetFormat {
    /// UBL 2.1 (`Invoice` + `CreditNote`) via `invoicekit-format-ubl`.
    Ubl,
    /// CII D16B via `invoicekit-format-cii`.
    Cii,
    /// One of the six Factur-X / ZUGFeRD profiles via
    /// `invoicekit-profile-factur-x`.
    FacturX(FacturXProfile),
}

impl TargetFormat {
    /// Operator-readable identifier used in tracing.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ubl => "format-ubl",
            Self::Cii => "format-cii",
            Self::FacturX(p) => match p {
                FacturXProfile::Minimum => "factur-x-minimum",
                FacturXProfile::BasicWl => "factur-x-basic-wl",
                FacturXProfile::Basic => "factur-x-basic",
                FacturXProfile::En16931 => "factur-x-en16931",
                FacturXProfile::Extended => "factur-x-extended",
                FacturXProfile::Xrechnung => "factur-x-xrechnung",
            },
        }
    }
}

/// Errors raised by [`compute_ledger`].
#[derive(Debug, Error)]
pub enum LossinessGeneratorError {
    /// The source document did not validate.
    #[error("source IR validation failed: {0}")]
    SourceIr(IrError),
    /// The UBL adapter failed during serialise or reparse.
    #[error("UBL adapter failed: {0}")]
    Ubl(#[from] UblError),
    /// The CII adapter failed during serialise or reparse.
    #[error("CII adapter failed: {0}")]
    Cii(#[from] CiiError),
    /// The Factur-X projection refused the input.
    #[error("Factur-X projection failed: {0}")]
    FacturX(#[from] FacturXError),
    /// The ledger itself failed its own envelope checks.
    #[error("ledger envelope check failed: {0}")]
    Ledger(IrError),
}

/// Compute the lossiness ledger for projecting `source` to `target`.
///
/// # Errors
///
/// Returns [`LossinessGeneratorError`] if the source IR is invalid,
/// the adapter for the target format fails, or the projected ledger
/// itself rejects its entries.
pub fn compute_ledger(
    source: &CommercialDocument,
    target: TargetFormat,
) -> Result<LossinessLedger, LossinessGeneratorError> {
    source
        .validate()
        .map_err(LossinessGeneratorError::SourceIr)?;
    match target {
        TargetFormat::Ubl => compute_ubl_ledger(source),
        TargetFormat::Cii => compute_cii_ledger(source),
        TargetFormat::FacturX(profile) => {
            let ProjectedDocument { ledger, .. } = factur_x_project(source, profile)?;
            Ok(ledger)
        }
    }
}

fn compute_ubl_ledger(
    source: &CommercialDocument,
) -> Result<LossinessLedger, LossinessGeneratorError> {
    let xml = ubl_to_xml(source)?;
    let (reparsed, _) = ubl_from_xml(&xml)?;
    LossinessLedger::from_roundtrip_comparison(source, &reparsed, "format-ubl")
        .map_err(LossinessGeneratorError::Ledger)
}

fn compute_cii_ledger(
    source: &CommercialDocument,
) -> Result<LossinessLedger, LossinessGeneratorError> {
    let xml = cii_to_xml(source)?;
    let (reparsed, _) = cii_from_xml(&xml)?;
    LossinessLedger::from_roundtrip_comparison(source, &reparsed, "format-cii")
        .map_err(LossinessGeneratorError::Ledger)
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_lossiness_ledger_generator::crate_name(),
///     "invoicekit-lossiness-ledger-generator"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-lossiness-ledger-generator"
}

#[cfg(test)]
mod tests {
    use super::{compute_ledger, crate_name, TargetFormat};

    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, InvoicePeriod,
        Iso4217Code, JurisdictionExtension, LocalizedString, LossinessLedger, MonetaryTotal, Party,
        PartyTaxId,
        PaymentInstruction, PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion,
        TaxCategorySummary,
    };
    use invoicekit_profile_factur_x::FacturXProfile;
    use rust_decimal::Decimal;

    fn party(role: &str, id: Option<&str>) -> Party {
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
                country: CountryCode::new("DE").unwrap(),
            },
            contact: Some(Contact {
                name: Some(format!("{role} contact")),
                email: None,
                phone: None,
            }),
        }
    }

    fn fixture() -> CommercialDocument {
        let unit = Decimal::new(10000, 2);
        let tax = (unit * Decimal::new(19, 2)).round_dp(2);
        let inclusive = unit + tax;
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::V1_0,
            id: DocumentId::new("ledger-fixture").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-27").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-26").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("LEDGER-001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: party("supplier", Some("sup-1")),
            customer: party("customer", Some("cus-1")),
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
                extensions: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: DecimalValue::new(unit),
                tax_amount: DecimalValue::new(tax),
                tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
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
                text: "Ledger fixture note.".to_owned(),
            }],
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            meta: DocumentMeta {
                tenant_id: "tenant-ledger".to_owned(),
                trace_id: "trace-ledger".to_owned(),
                source_system: None,
            },
        })
        .unwrap()
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-lossiness-ledger-generator");
    }

    /// Strict-gate part 1: UBL projection emits a populated
    /// ledger (preserved entries cover id, currency, lines, etc.).
    #[test]
    fn ubl_projection_populates_preserved_entries() {
        let source = fixture();
        let ledger = compute_ledger(&source, TargetFormat::Ubl).expect("UBL ledger computes");
        assert!(
            !ledger.preserved.is_empty(),
            "UBL preserved entries must be populated"
        );
        for path in [
            "/id",
            "/document_number",
            "/currency",
            "/issue_date",
            "/lines",
            "/tax_summary",
        ] {
            assert!(
                ledger.preserved.iter().any(|e| e.path == path),
                "UBL ledger missing preserved entry for {path}"
            );
        }
    }

    /// Regression guard for the comparator coverage of the EN 16931 BG-14
    /// invoice period and BT-72 delivery date. Both are round-trippable typed
    /// fields the serializers emit but the parsers keep as preserved raw XML
    /// (IR field reset to None), so a source carrying them differs from its
    /// reparse — the ledger MUST surface each on its own typed path (here as
    /// `lost`, since the value relocates into the preserved-XML extension)
    /// rather than only generically under `/extensions`. Without the
    /// comparator entries these paths would be silently absent.
    #[test]
    fn invoice_period_and_delivery_date_are_tracked_per_field_in_the_ledger() {
        let mut source = fixture();
        source.invoice_period = Some(InvoicePeriod {
            start_date: Some(DateOnly::new("2026-05-01").unwrap()),
            end_date: Some(DateOnly::new("2026-05-31").unwrap()),
        });
        source.delivery_date = Some(DateOnly::new("2026-05-15").unwrap());

        for format in [TargetFormat::Ubl, TargetFormat::Cii] {
            let ledger = compute_ledger(&source, format).expect("ledger computes");
            for path in ["/invoice_period", "/delivery_date"] {
                let surfaced = ledger.lost.iter().chain(ledger.preserved.iter()).any(|e| e.path == path);
                assert!(
                    surfaced,
                    "{format:?} ledger must surface {path} on its own typed path"
                );
                // The round-trip relocates the value into preserved raw XML, so
                // the typed field does not survive equal — it is reported lost.
                assert!(
                    ledger.lost.iter().any(|e| e.path == path),
                    "{format:?} ledger must report {path} as lost (relocated to preserved XML)"
                );
            }
        }
    }

    /// Strict-gate part 1, CII flavour. CII collapses `id` and
    /// `document_number` to a single field, so the round-trip
    /// drops one of them — the ledger records that as a `lost`
    /// entry and keeps the other as `preserved`.
    #[test]
    fn cii_projection_populates_ledger_with_both_preserved_and_lost() {
        let source = fixture();
        let ledger = compute_ledger(&source, TargetFormat::Cii).expect("CII ledger computes");
        assert!(!ledger.preserved.is_empty());
        for path in ["/document_number", "/currency", "/issue_date", "/lines"] {
            assert!(
                ledger.preserved.iter().any(|e| e.path == path),
                "CII ledger missing preserved entry for {path}"
            );
        }
        // The CII adapter folds `id` and `document_number` into a
        // single XML field; the reparse round-trips both to the
        // same value, so /id surfaces as a lost-entry the ledger
        // can flag for the evidence bundle to display.
        assert!(
            ledger.lost.iter().any(|e| e.path == "/id"),
            "CII ledger must record /id drift since CII has no separate ID column",
        );
    }

    /// Strict-gate part 2: at least one expected-loss case per
    /// cross-format pair. EN 16931 -> MINIMUM Factur-X loses
    /// `lines`, `tax_summary`, and `notes` at the CII write layer.
    #[test]
    fn en16931_to_factur_x_minimum_populates_lost_entries() {
        let source = fixture();
        let ledger =
            compute_ledger(&source, TargetFormat::FacturX(FacturXProfile::Minimum)).unwrap();
        assert!(
            ledger.lost.iter().any(|e| e.path == "/lines"),
            "MINIMUM ledger must mention /lines as lost"
        );
        assert!(
            ledger.lost.iter().any(|e| e.path == "/tax_summary"),
            "MINIMUM ledger must mention /tax_summary as lost"
        );
        assert!(
            ledger.lost.iter().any(|e| e.path == "/notes"),
            "MINIMUM ledger must mention /notes as lost"
        );
    }

    /// The diff must compare collection contents, not just counts.
    #[test]
    fn ledger_records_value_drift_when_collection_counts_match() -> Result<(), String> {
        let source = fixture();
        let mut tampered = source.clone();
        tampered
            .lines
            .first_mut()
            .ok_or_else(|| "fixture missing line".to_owned())?
            .description = "Different widget".to_owned();
        tampered
            .tax_summary
            .first_mut()
            .ok_or_else(|| "fixture missing tax summary".to_owned())?
            .category_code = "AA".to_owned();
        tampered
            .notes
            .first_mut()
            .ok_or_else(|| "fixture missing note".to_owned())?
            .text = "Different note".to_owned();
        tampered.extensions.push(
            JurisdictionExtension::new(
                "urn:invoicekit:test:extension".to_owned(),
                serde_json::json!({"value": "preserved"}),
            )
            .map_err(|error| error.to_string())?,
        );
        let mut source_with_extension = source;
        source_with_extension.extensions.push(
            JurisdictionExtension::new(
                "urn:invoicekit:test:extension".to_owned(),
                serde_json::json!({"value": "source"}),
            )
            .map_err(|error| error.to_string())?,
        );

        let ledger = LossinessLedger::from_roundtrip_comparison(
            &source_with_extension,
            &tampered,
            "test-adapter",
        )
        .map_err(|error| error.to_string())?;
        for path in ["/lines", "/tax_summary", "/notes", "/extensions"] {
            assert!(
                ledger.lost.iter().any(|entry| entry.path == path),
                "ledger must record value drift at {path}"
            );
        }
        Ok(())
    }
}
