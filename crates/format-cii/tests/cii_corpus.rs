// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Regression tests for the committed synthetic CII D16B corpus.

use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_canonical::canonicalize_xml;
use invoicekit_format_cii::{from_xml, to_xml};
use invoicekit_ir::LossinessLedger;

const EXPECTED_FIXTURES: usize = 50;
const EXPECTED_PRESERVED_PATHS: [&str; 21] = [
    "/attachments",
    "/currency",
    "/customer",
    "/document_number",
    "/document_type",
    "/due_date",
    "/extensions",
    "/id",
    "/issue_date",
    "/lines",
    "/meta",
    "/monetary_total",
    "/notes",
    "/payee",
    "/payment_instructions",
    "/payment_terms",
    "/references",
    "/schema_version",
    "/supplier",
    "/tax_point_date",
    "/tax_summary",
];

#[test]
fn committed_cii_corpus_round_trips_and_is_byte_stable() {
    let fixtures = fixture_paths();
    assert_eq!(fixtures.len(), EXPECTED_FIXTURES);

    for fixture in fixtures {
        let xml = fs::read_to_string(&fixture).unwrap();
        let (parsed, ledger) = from_xml(&xml).unwrap();
        assert_zero_loss_ledger(&ledger, &fixture);
        let first = to_xml(&parsed).unwrap();
        let second = to_xml(&parsed).unwrap();
        assert_eq!(
            first, second,
            "non-deterministic serializer for {fixture:?}"
        );
        assert_eq!(
            canonicalize_xml(&first).unwrap(),
            first,
            "non-canonical serializer output for {fixture:?}"
        );
        let (reparsed, reparse_ledger) = from_xml(&first).unwrap();
        assert_zero_loss_ledger(&reparse_ledger, &fixture);
        assert_eq!(
            parsed, reparsed,
            "parse -> serialize -> parse drift for {fixture:?}"
        );
    }
}

#[test]
fn committed_cii_corpus_covers_required_scenarios() {
    let joined = fixture_paths()
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    for required in [
        ">380</ram:TypeCode>",
        ">381</ram:TypeCode>",
        "IncludedSupplyChainTradeLineItem",
        ">S</ram:CategoryCode>",
        ">AA</ram:CategoryCode>",
        ">Z</ram:CategoryCode>",
        ">E</ram:CategoryCode>",
        ">AE</ram:CategoryCode>",
        "AllowanceTotalAmount",
        "ChargeTotalAmount",
        "TotalPrepaidAmount",
        "SpecifiedTradeSettlementPaymentMeans",
        "SpecifiedTradePaymentTerms",
        "ActualDeliverySupplyChainEvent",
        "PayeeTradeParty",
        "BuyerReference",
        "BusinessProcessSpecifiedDocumentContextParameter",
        "IncludedNote",
    ] {
        assert!(
            joined.contains(required),
            "missing required CII pattern {required}"
        );
    }
}

/// T-021a strict-acceptance gate, CII flavour. Mirrors the UBL
/// assertion in `format-ubl/tests/ubl_corpus.rs`:
///
/// 1. >= 20 fixtures covered (bead lower bound).
/// 2. Zero lossiness per fixture, asserted directly by the
///    inline parser ledger and again by IR equality across the
///    parse -> serialize -> parse cycle.
#[test]
fn committed_cii_corpus_satisfies_t_021a_zero_loss() {
    let fixtures = fixture_paths();
    assert!(
        fixtures.len() >= 20,
        "T-021a strict gate: expected >= 20 CII fixtures, got {}",
        fixtures.len()
    );
    for fixture in &fixtures {
        let xml = fs::read_to_string(fixture).unwrap();
        let (parsed, ledger) = from_xml(&xml).unwrap();
        assert_zero_loss_ledger(&ledger, fixture);
        let serialized = to_xml(&parsed).unwrap();
        let (reparsed, reparse_ledger) = from_xml(&serialized).unwrap();
        assert_zero_loss_ledger(&reparse_ledger, fixture);
        assert_eq!(
            parsed, reparsed,
            "T-021a: non-empty lossiness implied for {fixture:?}",
        );
    }
}

fn fixture_paths() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance-corpus/synthetic/cii-d16b-profiled");
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("fixture.xml"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_zero_loss_ledger(ledger: &LossinessLedger, fixture: &Path) {
    let fixture = fixture.display();
    assert!(
        ledger.lost.is_empty(),
        "CII inline parse ledger should have no lost fields for {fixture}: {:?}",
        ledger.lost
    );
    assert!(
        ledger.warnings.is_empty(),
        "CII inline parse ledger should have no warnings for {fixture}: {:?}",
        ledger.warnings
    );

    let mut actual = ledger
        .preserved
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = EXPECTED_PRESERVED_PATHS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        actual, expected,
        "CII inline parse ledger preserved paths drifted for {fixture}"
    );
}
