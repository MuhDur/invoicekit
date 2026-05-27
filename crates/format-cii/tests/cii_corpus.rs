// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Regression tests for the committed synthetic CII D16B corpus.

use std::fs;
use std::path::PathBuf;

use invoicekit_canonical::canonicalize_xml;
use invoicekit_format_cii::{from_xml, to_xml};

const EXPECTED_FIXTURES: usize = 50;

#[test]
fn committed_cii_corpus_round_trips_and_is_byte_stable() {
    let fixtures = fixture_paths();
    assert_eq!(fixtures.len(), EXPECTED_FIXTURES);

    for fixture in fixtures {
        let xml = fs::read_to_string(&fixture).unwrap();
        let parsed = from_xml(&xml).unwrap();
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
        let reparsed = from_xml(&first).unwrap();
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
/// 2. Zero lossiness per fixture, asserted indirectly by IR
///    equality across the parse -> serialize -> parse cycle.
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
        let parsed = from_xml(&xml).unwrap();
        let serialized = to_xml(&parsed).unwrap();
        let reparsed = from_xml(&serialized).unwrap();
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
