// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-045 integration test: project every committed UBL 2.1
//! conformance fixture through `to_xrechnung_3_x_xml` and assert
//! the XRechnung-specific header trio (`CustomizationID`,
//! `ProfileID`, `BuyerReference` for B2G) appears in the output.

use std::fs;
use std::path::PathBuf;

use invoicekit_format_ubl::from_xml;
use invoicekit_profile_xrechnung::{
    to_xrechnung_3_x_xml, XRechnungOptions, XRECHNUNG_3_CUSTOMIZATION_ID, XRECHNUNG_PROFILE_ID,
};

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

#[test]
fn projects_thirty_plus_fixtures_with_xrechnung_headers() -> Result<(), String> {
    let fixtures = fixture_paths();
    assert!(
        fixtures.len() >= 30,
        "T-045 strict gate: need >=30 fixtures; have {}",
        fixtures.len()
    );

    // Take the first 30 fixtures (deterministic) and project each
    // through XRechnung. The gate is: every projection emits the
    // CIUS-DE CustomizationID + ProfileID + Leitweg-ID-derived
    // BuyerReference, and the projection itself does not panic
    // for any fixture.
    let leitweg_id = "04011000-1234512345-06";
    let options = XRechnungOptions {
        leitweg_id: Some(leitweg_id.to_owned()),
    };
    for (i, path) in fixtures.iter().take(30).enumerate() {
        let xml = fs::read_to_string(path)
            .map_err(|err| format!("fixture {i} ({path:?}) could not be read: {err}"))?;
        let (document, _) = from_xml(&xml)
            .map_err(|err| format!("fixture {i} ({path:?}) failed to parse: {err}"))?;
        let projected = to_xrechnung_3_x_xml(&document, &options)
            .map_err(|err| format!("projection {i} ({path:?}) failed: {err}"))?;

        assert!(
            projected.contains(XRECHNUNG_3_CUSTOMIZATION_ID),
            "fixture {i}: projected XML must carry the XRechnung 3.x CustomizationID"
        );
        assert!(
            projected.contains(XRECHNUNG_PROFILE_ID),
            "fixture {i}: projected XML must carry the Peppol BIS Billing 3.0 ProfileID"
        );
        assert!(
            projected.contains(leitweg_id),
            "fixture {i}: projected XML must carry the Leitweg-ID for B2G;\nXML excerpt:\n{}",
            projected.chars().take(2000).collect::<String>()
        );
    }
    Ok(())
}

#[test]
fn b2b_projection_omits_buyer_reference_override() {
    // Without a Leitweg-ID the projection should still emit the
    // CIUS-DE customization but leave BuyerReference to whatever
    // the upstream document carried.
    let xml = fs::read_to_string(&fixture_paths()[0]).unwrap();
    let (document, _) = from_xml(&xml).unwrap();
    let projected = to_xrechnung_3_x_xml(&document, &XRechnungOptions::default()).unwrap();

    assert!(projected.contains(XRECHNUNG_3_CUSTOMIZATION_ID));
    assert!(projected.contains(XRECHNUNG_PROFILE_ID));
    // The upstream fixtures all carry an extension-driven
    // BuyerReference value of the form BUYER-REF-UBL-{nnnn}
    // (from the bbqm corpus generator). When no Leitweg-ID is
    // supplied the projection passes that through unchanged.
    assert!(
        projected.contains("BUYER-REF-UBL-"),
        "B2B projection should preserve the upstream BuyerReference"
    );
}
