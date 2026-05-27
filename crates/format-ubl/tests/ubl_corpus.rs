// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! bbqm regression tests for the committed synthetic UBL 2.1 corpus.

use std::fs;
use std::path::PathBuf;

use invoicekit_canonical::canonicalize_xml;
use invoicekit_format_ubl::{from_xml, to_xml};

const EXPECTED_FIXTURES: usize = 50;

#[test]
fn committed_ubl_corpus_round_trips_and_is_byte_stable() {
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
fn committed_ubl_corpus_covers_required_scenarios() {
    let joined = fixture_paths()
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    for required in [
        // Document type codes.
        ">380<",
        ">381<",
        // Lines.
        "InvoiceLine",
        "CreditNoteLine",
        // Tax category codes. The serializer emits the cbc:ID
        // with inline namespace declarations, so we match on the
        // closing-tag suffix.
        ">S</cbc:ID>",
        ">AA</cbc:ID>",
        ">Z</cbc:ID>",
        ">E</cbc:ID>",
        ">AE</cbc:ID>",
        // Header allowance / charge / prepaid totals.
        "AllowanceTotalAmount",
        "ChargeTotalAmount",
        "PrepaidAmount",
        // Payment + party blocks.
        "PaymentMeans",
        "PaymentTerms",
        "PayeeParty",
        "BuyerReference",
        "AccountingCost",
        "CustomizationID",
        "ProfileID",
        "cbc:Note",
    ] {
        assert!(
            joined.contains(required),
            "missing required UBL pattern {required}"
        );
    }
}

fn fixture_paths() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance-corpus/synthetic/ubl-2-1");
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("fixture.xml"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
